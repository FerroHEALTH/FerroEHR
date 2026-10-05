// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The canonical-JSON transforms: externalizing large inline
//! `DV_MULTIMEDIA.data` on the commit path and re-inlining externalized blobs
//! with integrity verification on the read path.
//!
//! Pure functions over `serde_json::Value`; the async blob I/O is driven by
//! [`MultimediaEngine`](super::MultimediaEngine). No openEHR spec governs the
//! blob-storage mechanism — our own design/extension — but the shape being
//! rewritten is RM data types, so the rewrite honours their invariants.
//!
//! Per RM 1.2.0 `DV_MULTIMEDIA`
//! (`RM/docs/UML/classes/org.openehr.rm.data_types.dv_multimedia.adoc`), an
//! externalized value drops inline `data`, gains a `uri` (`is_external`),
//! carries the mandatory unencoded `size`, and — setting `integrity_check` —
//! must also set `integrity_check_algorithm` from the openEHR `Integrity check
//! algorithms` code set (`Not_empty`, `Integrity_check_validity`,
//! `Integrity_check_algorithm_validity`, `Size_valid`). The algorithm code is
//! `SHA-256`.

#![expect(
    clippy::doc_markdown,
    reason = "openEHR identifiers (CODE_PHRASE, DV_URI, …) read as prose in this \
              module's docs"
)]
#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): external FHIR resources, tenancy/event CRUD rows, \
              multimedia offload over stored fragments (families 3/6/8)"
)]

use std::collections::HashMap;

use base64::Engine as _;
use openehr_base::prelude::TerminologyId;
use openehr_rm::prelude::{CodePhrase, DvUri, DvUriData};
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use super::MultimediaError;
use super::store::BlobStore;

/// The `_type` of the node we externalize.
pub(super) const DV_MULTIMEDIA: &str = "DV_MULTIMEDIA";
/// openEHR `Integrity check algorithms` code-set id (TERM 3.1.0).
const INTEGRITY_ALGORITHM_TERMINOLOGY_ID: &str = "openehr_integrity_check_algorithms";
/// The integrity-check algorithm we compute (openEHR code-set entry).
pub(super) const SHA256_CODE: &str = "SHA-256";

/// The base64 (standard, padded) engine openEHR canonical JSON uses for
/// `Array<Octet>` fields (`data`, `integrity_check`).
fn b64() -> &'static base64::engine::general_purpose::GeneralPurpose {
    &base64::engine::general_purpose::STANDARD
}

/// SHA-256 of `bytes` as `(lowercase-hex, standard-base64)`.
fn sha256(bytes: &[u8]) -> (String, String) {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in &digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    (hex, b64().encode(digest))
}

/// The `integrity_check_algorithm` CODE_PHRASE for SHA-256, in canonical JSON.
fn integrity_algorithm_code_phrase() -> Value {
    openehr_its::json::to_canonical_value(&CodePhrase {
        terminology_id: TerminologyId {
            value: INTEGRITY_ALGORITHM_TERMINOLOGY_ID.to_owned(),
        },
        code_string: SHA256_CODE.to_owned(),
        preferred_term: None,
    })
}

/// Recursively visit every `DV_MULTIMEDIA` object in the tree, calling `f` on
/// its map. Recurses through the whole tree, including a node's own `thumbnail`
/// (a nested `DV_MULTIMEDIA`), so thumbnails are handled too.
fn walk_multimedia<F: FnMut(&mut Map<String, Value>)>(v: &mut Value, f: &mut F) {
    match v {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some(DV_MULTIMEDIA) {
                f(map);
            }
            for child in map.values_mut() {
                walk_multimedia(child, f);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_multimedia(item, f);
            }
        }
        _ => {}
    }
}

/// Read-only walk collecting keys of *our* externalized blobs referenced in the
/// tree (`s3://<our-bucket>/<hex>` URIs). Used by GC and dump/load.
#[must_use]
pub(super) fn referenced_keys(root: &Value, store: &BlobStore) -> Vec<String> {
    fn recurse(v: &Value, store: &BlobStore, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                if map.get("_type").and_then(Value::as_str) == Some(DV_MULTIMEDIA)
                    && let Some(uri) = map
                        .get("uri")
                        .and_then(|u| u.get("value"))
                        .and_then(Value::as_str)
                    && let Some(key) = store.key_from_uri(uri)
                {
                    out.push(key.to_owned());
                }
                for child in map.values() {
                    recurse(child, store, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    recurse(item, store, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    recurse(root, store, &mut out);
    out
}

/// Rewrite one `DV_MULTIMEDIA` map for externalization, returning the
/// `(hex, bytes)` to upload, or `None` if it does not qualify.
///
/// Qualifies only when the value is purely inline (`data` present, `uri`
/// absent) and its **decoded** length strictly exceeds `threshold`. A value
/// that already carries a `uri` (client-managed external media) is stored
/// verbatim; a small inline value is left untouched (the zero-drift gate).
fn offload_one(
    map: &mut Map<String, Value>,
    threshold: usize,
    store: &BlobStore,
) -> Result<Option<(String, Vec<u8>)>, MultimediaError> {
    // A value that already references external storage is stored verbatim.
    if map.get("uri").is_some_and(|u| !u.is_null()) {
        return Ok(None);
    }
    let Some(data) = map.get("data").and_then(Value::as_str) else {
        return Ok(None);
    };
    let bytes = b64().decode(data).map_err(|e| {
        MultimediaError::Malformed(format!("DV_MULTIMEDIA.data is not base64: {e}"))
    })?;
    if bytes.len() <= threshold {
        return Ok(None);
    }
    let (hex, integrity_b64) = sha256(&bytes);
    #[expect(
        clippy::map_err_ignore,
        reason = "TryFromIntError carries no payload beyond out-of-range, which \
                  the mapped message already states"
    )]
    let size = i64::try_from(bytes.len()).map_err(|_| {
        MultimediaError::Malformed("DV_MULTIMEDIA.data exceeds i64 byte range".to_owned())
    })?;

    // Drop inline data, become external, carry integrity and size: `Not_empty`
    // via `uri`, `Integrity_check_*`, `Size_valid`.
    map.remove("data");
    map.insert(
        "uri".to_owned(),
        openehr_its::json::to_canonical_value(&DvUri::DvUri(DvUriData {
            value: store.uri_for(&hex),
        })),
    );
    map.insert("integrity_check".to_owned(), Value::String(integrity_b64));
    map.insert(
        "integrity_check_algorithm".to_owned(),
        integrity_algorithm_code_phrase(),
    );
    map.insert("size".to_owned(), Value::Number(size.into()));
    Ok(Some((hex, bytes)))
}

/// Walk `root`, externalizing every qualifying inline `DV_MULTIMEDIA` in place,
/// and return the `(hex, bytes)` blobs the caller must upload.
///
/// The tree is fully rewritten synchronously; the async uploads are the
/// caller's second phase (so no upload result is needed to finish the rewrite,
/// and a failed upload aborts the commit before anything is stored).
pub(super) fn plan_offload(
    root: &mut Value,
    threshold: usize,
    store: &BlobStore,
) -> Result<Vec<(String, Vec<u8>)>, MultimediaError> {
    let mut pending: Vec<(String, Vec<u8>)> = Vec::new();
    let mut error: Option<MultimediaError> = None;
    walk_multimedia(root, &mut |map| {
        if error.is_some() {
            return;
        }
        match offload_one(map, threshold, store) {
            Ok(Some(blob)) => pending.push(blob),
            Ok(None) => {}
            Err(e) => error = Some(e),
        }
    });
    match error {
        Some(e) => Err(e),
        None => Ok(pending),
    }
}

/// Collect the blob keys the read-path expansion must fetch: every
/// `DV_MULTIMEDIA` that references one of *our* blobs and is not already inline.
pub(super) fn collect_expand_keys(root: &Value, store: &BlobStore) -> Vec<String> {
    fn recurse(v: &Value, store: &BlobStore, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                if map.get("_type").and_then(Value::as_str) == Some(DV_MULTIMEDIA)
                    && map.get("data").and_then(Value::as_str).is_none()
                    && let Some(uri) = map
                        .get("uri")
                        .and_then(|u| u.get("value"))
                        .and_then(Value::as_str)
                    && let Some(key) = store.key_from_uri(uri)
                {
                    out.push(key.to_owned());
                }
                for child in map.values() {
                    recurse(child, store, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    recurse(item, store, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    recurse(root, store, &mut out);
    out
}

/// Re-inline externalized `DV_MULTIMEDIA` values in place, using the verified
/// `key → base64(data)` map the caller fetched. The `uri` + integrity fields are
/// kept (the value is now *expanded*: `is_inline` and `is_external` both true —
/// spec-legal), so a subsequent commit of the same body re-offloads cleanly.
pub(super) fn apply_expand(root: &mut Value, fetched: &HashMap<String, String>, store: &BlobStore) {
    walk_multimedia(root, &mut |map| {
        if map.get("data").and_then(Value::as_str).is_some() {
            return;
        }
        let Some(key) = map
            .get("uri")
            .and_then(|u| u.get("value"))
            .and_then(Value::as_str)
            .and_then(|uri| store.key_from_uri(uri))
            .map(str::to_owned)
        else {
            return;
        };
        if let Some(b64_data) = fetched.get(&key) {
            map.insert("data".to_owned(), Value::String(b64_data.clone()));
        }
    });
}

/// Verify fetched blob `bytes` hash to the expected `hex` key, returning the
/// canonical base64 encoding **of the blob bytes** for re-inlining as
/// `DV_MULTIMEDIA.data`. A mismatch is a hard error — never silent corruption.
pub(super) fn verify_and_encode(hex: &str, bytes: &[u8]) -> Result<String, MultimediaError> {
    let (actual, _) = sha256(bytes);
    if actual != hex {
        return Err(MultimediaError::Integrity {
            expected: hex.to_owned(),
            actual,
        });
    }
    Ok(b64().encode(bytes))
}

#[cfg(test)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the test helpers take an owned Value so call sites can pass a \
              json! literal directly"
)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn store() -> BlobStore {
        BlobStore::from_parts(
            Arc::new(object_store::memory::InMemory::new()),
            "media".to_owned(),
        )
    }

    /// A canonical DV_MULTIMEDIA node with `n` bytes of inline data.
    fn multimedia(n: usize) -> Value {
        let payload = vec![0x42u8; n];
        json!({
            "_type": "DV_MULTIMEDIA",
            "media_type": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_media-types" },
                "code_string": "application/octet-stream"
            },
            "size": n,
            "data": b64().encode(&payload),
        })
    }

    fn wrap(node: Value) -> Value {
        json!({
            "_type": "COMPOSITION",
            "content": [ { "_type": "ELEMENT", "value": node } ]
        })
    }

    #[test]
    fn below_threshold_is_untouched() {
        let s = store();
        let mut comp = wrap(multimedia(100));
        let before = comp.clone();
        let pending = plan_offload(&mut comp, 256, &s).unwrap();
        assert!(pending.is_empty());
        assert_eq!(comp, before, "small inline media must be byte-identical");
    }

    #[test]
    fn above_threshold_offloads_and_honours_invariants() {
        let s = store();
        let raw = vec![0x42u8; 1000];
        let (want_hex, want_ic) = sha256(&raw);
        let mut comp = wrap(multimedia(1000));
        let pending = plan_offload(&mut comp, 256, &s).unwrap();

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, want_hex);
        assert_eq!(pending[0].1, raw);

        let node = comp.pointer("/content/0/value").unwrap();
        // data removed; uri present (is_external) → Not_empty holds.
        assert!(node.get("data").is_none());
        assert_eq!(
            node.pointer("/uri/value").unwrap().as_str().unwrap(),
            format!("s3://media/{want_hex}")
        );
        assert_eq!(node.pointer("/uri/_type").unwrap(), "DV_URI");
        // integrity_check = base64(sha256 octets); algorithm coded SHA-256.
        assert_eq!(node.get("integrity_check").unwrap(), &json!(want_ic));
        assert_eq!(
            node.pointer("/integrity_check_algorithm/code_string")
                .unwrap(),
            SHA256_CODE
        );
        assert_eq!(
            node.pointer("/integrity_check_algorithm/terminology_id/value")
                .unwrap(),
            INTEGRITY_ALGORITHM_TERMINOLOGY_ID
        );
        // mandatory unencoded size = decoded length.
        assert_eq!(node.get("size").unwrap(), &json!(1000));
    }

    #[test]
    fn client_supplied_uri_is_passthrough() {
        let s = store();
        let mut node = multimedia(1000);
        node.as_object_mut().unwrap().insert(
            "uri".to_owned(),
            json!({ "_type": "DV_URI", "value": "https://pacs.example/img" }),
        );
        // Also drop inline data to model a pure external reference.
        node.as_object_mut().unwrap().remove("data");
        let mut comp = wrap(node);
        let before = comp.clone();
        let pending = plan_offload(&mut comp, 256, &s).unwrap();
        assert!(pending.is_empty());
        assert_eq!(
            comp, before,
            "client-managed external media stored verbatim"
        );
    }

    #[test]
    fn thumbnail_is_offloaded_too() {
        let s = store();
        let mut node = multimedia(1000);
        node.as_object_mut()
            .unwrap()
            .insert("thumbnail".to_owned(), multimedia(500));
        let mut comp = wrap(node);
        let pending = plan_offload(&mut comp, 256, &s).unwrap();
        assert_eq!(pending.len(), 2, "both the media and its thumbnail offload");
    }

    #[test]
    fn expand_round_trips_offloaded_value() {
        let s = store();
        let raw = vec![0x42u8; 1000];
        let (hex, _) = sha256(&raw);
        let mut comp = wrap(multimedia(1000));
        plan_offload(&mut comp, 256, &s).unwrap();

        let keys = collect_expand_keys(&comp, &s);
        assert_eq!(keys, vec![hex.clone()]);

        // Simulate a verified fetch: the re-inlined data must decode back to the
        // *original* blob bytes (not the digest).
        let b64_data = verify_and_encode(&hex, &raw).unwrap();
        assert_eq!(b64().decode(&b64_data).unwrap(), raw);
        let fetched = HashMap::from([(hex.clone(), b64_data.clone())]);
        apply_expand(&mut comp, &fetched, &s);

        let node = comp.pointer("/content/0/value").unwrap();
        assert_eq!(node.get("data").unwrap(), &json!(b64_data));
        // uri kept → expanded (both is_inline and is_external), spec-legal.
        assert!(node.get("uri").is_some());
    }

    #[test]
    fn integrity_mismatch_is_error() {
        let raw = vec![0x42u8; 100];
        let (hex, _) = sha256(&raw);
        let err = verify_and_encode(&hex, b"tampered").unwrap_err();
        assert!(matches!(err, MultimediaError::Integrity { .. }));
    }

    #[test]
    fn referenced_keys_finds_our_blobs_only() {
        let s = store();
        let mut comp = wrap(multimedia(1000));
        plan_offload(&mut comp, 256, &s).unwrap();
        let keys = referenced_keys(&comp, &s);
        assert_eq!(keys.len(), 1);
    }
}
