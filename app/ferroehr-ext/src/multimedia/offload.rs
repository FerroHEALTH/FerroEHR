// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The canonical-JSON transforms: externalizing large inline
//! `DV_MULTIMEDIA.data` on the commit path and re-inlining externalized blobs
//! on the read path.
//!
//! Synchronous functions over `serde_json::Value`; the blob I/O is driven by
//! [`MultimediaEngine`](super::MultimediaEngine). No openEHR spec governs the
//! blob-storage mechanism (our own design), but the shape being rewritten is
//! RM data types, so the rewrite honours their invariants.
//!
//! Per RM 1.2.0 `DV_MULTIMEDIA`
//! (`RM/docs/UML/classes/org.openehr.rm.data_types.dv_multimedia.adoc`), an
//! externalized value drops inline `data`, gains a `uri` (`is_external`),
//! carries the mandatory unencoded `size`, and, setting `integrity_check`,
//! must also set `integrity_check_algorithm` from the openEHR `Integrity check
//! algorithms` code set (`Not_empty`, `Integrity_check_validity`,
//! `Integrity_check_algorithm_validity`, `Size_valid`). The algorithm code is
//! `SHA-256`.
//!
//! The inline `data` is base64 in the request body. It is decoded in fixed
//! windows, never into one buffer the size of the blob: once to hash it and
//! once more as the upload streams.

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

use std::collections::BTreeMap;

use base64::Engine as _;
use bytes::Bytes;
use openehr_base::prelude::TerminologyId;
use openehr_rm::prelude::{CodePhrase, DvMultimedia, DvUri, DvUriData};
use openehr_rm::v1_2::access::RmObject;
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use super::MultimediaError;
use super::blob::{BlobKey, BlobStore};

/// The `_type` of the node we externalize.
pub(super) const DV_MULTIMEDIA: &str = "DV_MULTIMEDIA";
/// openEHR `Integrity check algorithms` code-set id (TERM 3.1.0).
const INTEGRITY_ALGORITHM_TERMINOLOGY_ID: &str = "openehr_integrity_check_algorithms";
/// The integrity-check algorithm we compute (openEHR code-set entry).
pub(super) const SHA256_CODE: &str = "SHA-256";
/// How many base64 characters one decode window holds: a multiple of 4, so
/// every window but the last decodes on its own.
const DECODE_WINDOW: usize = 64 * 1024;

/// The base64 (standard, padded) engine openEHR canonical JSON uses for
/// `Array<Octet>` fields (`data`, `integrity_check`).
fn b64() -> &'static base64::engine::general_purpose::GeneralPurpose {
    &base64::engine::general_purpose::STANDARD
}

/// The `integrity_check_algorithm` CODE_PHRASE for SHA-256, in canonical JSON.
fn integrity_algorithm_code_phrase() -> Value {
    openehr_its::json::to_canonical_value(&integrity_algorithm())
}

/// The `integrity_check_algorithm` CODE_PHRASE for SHA-256.
fn integrity_algorithm() -> CodePhrase {
    CodePhrase {
        terminology_id: TerminologyId {
            value: INTEGRITY_ALGORITHM_TERMINOLOGY_ID.to_owned(),
        },
        code_string: SHA256_CODE.to_owned(),
        preferred_term: None,
    }
}

/// The decoded length of a padded base64 text, read off its length and
/// padding alone; `None` when the length is not a multiple of 4.
fn decoded_len(data: &str) -> Option<usize> {
    if !data.len().is_multiple_of(4) {
        return None;
    }
    let padding = data.bytes().rev().take(2).filter(|&b| b == b'=').count();
    // Every 4 characters carry 3 bytes, less one per padding character.
    data.len()
        .div_euclid(4)
        .checked_mul(3)?
        .checked_sub(padding)
}

/// The bytes of a base64 text, decoded one window at a time.
pub(super) fn decoded_chunks(
    data: &str,
) -> impl Iterator<Item = Result<Bytes, MultimediaError>> + '_ {
    data.as_bytes().chunks(DECODE_WINDOW).map(|window| {
        b64().decode(window).map(Bytes::from).map_err(|e| {
            MultimediaError::Malformed(format!("DV_MULTIMEDIA.data is not base64: {e}"))
        })
    })
}

/// A blob the commit has to upload: its key, its unencoded length, and the
/// base64 text it was inline as, moved out of the record.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Pending {
    pub(super) key: BlobKey,
    pub(super) size: u64,
    pub(super) data: String,
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

/// The read-only twin of [`walk_multimedia`].
fn visit_multimedia<F: FnMut(&Map<String, Value>)>(v: &Value, f: &mut F) {
    match v {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some(DV_MULTIMEDIA) {
                f(map);
            }
            for child in map.values() {
                visit_multimedia(child, f);
            }
        }
        Value::Array(items) => {
            for item in items {
                visit_multimedia(item, f);
            }
        }
        _ => {}
    }
}

/// The URI a `DV_MULTIMEDIA` map references, if any.
fn uri_of(map: &Map<String, Value>) -> Option<&str> {
    map.get("uri")
        .and_then(|u| u.get("value"))
        .and_then(Value::as_str)
}

/// The keys of *our* externalized blobs referenced anywhere in the tree, sorted
/// and each once. Used by GC and dump/load.
#[must_use]
pub(super) fn referenced_keys(root: &Value, store: &BlobStore) -> Vec<BlobKey> {
    let mut out = Vec::new();
    visit_multimedia(root, &mut |map| {
        if let Some(key) = uri_of(map).and_then(|uri| store.key_from_uri(uri)) {
            out.push(key);
        }
    });
    out.sort_unstable();
    out.dedup();
    out
}

/// Whether a `DV_MULTIMEDIA` map holds inline `data` larger than `threshold`
/// and no `uri`: the values a commit externalizes.
fn qualifies(map: &Map<String, Value>, threshold: usize) -> Result<bool, MultimediaError> {
    if map.get("uri").is_some_and(|u| !u.is_null()) {
        return Ok(false);
    }
    let Some(data) = map.get("data").and_then(Value::as_str) else {
        return Ok(false);
    };
    let length = decoded_len(data).ok_or_else(|| {
        MultimediaError::Malformed(
            "DV_MULTIMEDIA.data is not base64: its length is not a multiple of 4".to_owned(),
        )
    })?;
    Ok(length > threshold)
}

/// Whether any `DV_MULTIMEDIA` in the tree still qualifies for
/// externalization: the guard that refuses a commit whose externalization was
/// skipped.
///
/// # Errors
/// [`MultimediaError::Malformed`] when an inline `data` is not padded base64.
pub(super) fn needs_offload(root: &Value, threshold: usize) -> Result<bool, MultimediaError> {
    let mut found = Ok(false);
    visit_multimedia(root, &mut |map| {
        if matches!(found, Ok(false)) {
            found = qualifies(map, threshold);
        }
    });
    found
}

/// Rewrite one `DV_MULTIMEDIA` map for externalization, returning the blob to
/// upload, or `None` if it does not qualify.
///
/// Qualifies only when the value is purely inline (`data` present, `uri`
/// absent) and its **decoded** length strictly exceeds `threshold`. A value
/// that already carries a `uri` (client-managed external media) is stored
/// verbatim; a small inline value is left untouched (the zero-drift gate).
fn offload_one(
    map: &mut Map<String, Value>,
    threshold: usize,
    store: &BlobStore,
) -> Result<Option<Pending>, MultimediaError> {
    if !qualifies(map, threshold)? {
        return Ok(None);
    }
    let Some(Value::String(data)) = map.get("data") else {
        return Ok(None);
    };
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    for chunk in decoded_chunks(data) {
        let chunk = chunk?;
        hasher.update(&chunk);
        size = size.saturating_add(super::blob::len64(chunk.len()));
    }
    let digest: [u8; 32] = hasher.finalize().into();
    let key = BlobKey::from_digest(digest);
    let declared = i64::try_from(size).map_err(|e| {
        MultimediaError::Malformed(format!(
            "DV_MULTIMEDIA.data exceeds the i64 byte range: {e}"
        ))
    })?;

    // Drop inline data, become external, carry integrity and size: `Not_empty`
    // via `uri`, `Integrity_check_*`, `Size_valid`.
    let Some(Value::String(data)) = map.remove("data") else {
        return Ok(None);
    };
    map.insert(
        "uri".to_owned(),
        openehr_its::json::to_canonical_value(&DvUri::DvUri(DvUriData {
            value: store.uri_for(&key),
        })),
    );
    map.insert(
        "integrity_check".to_owned(),
        Value::String(b64().encode(digest)),
    );
    map.insert(
        "integrity_check_algorithm".to_owned(),
        integrity_algorithm_code_phrase(),
    );
    map.insert("size".to_owned(), Value::Number(declared.into()));
    Ok(Some(Pending { key, size, data }))
}

/// Walk `root`, externalizing every qualifying inline `DV_MULTIMEDIA` in place,
/// and return the blobs the caller must upload.
///
/// The tree is fully rewritten synchronously; the uploads are the caller's
/// second phase, so a failed upload aborts the commit before anything is
/// stored.
pub(super) fn plan_offload(
    root: &mut Value,
    threshold: usize,
    store: &BlobStore,
) -> Result<Vec<Pending>, MultimediaError> {
    let mut pending: Vec<Pending> = Vec::new();
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

/// Every `DV_MULTIMEDIA` of a typed RM value, its thumbnails included, in
/// attribute order.
fn walk_multimedia_typed(
    node: &mut dyn RmObject,
    f: &mut dyn FnMut(&mut DvMultimedia) -> Result<(), MultimediaError>,
) -> Result<(), MultimediaError> {
    if let Some(multimedia) = node.as_any_mut().downcast_mut::<DvMultimedia>() {
        f(multimedia)?;
    }
    for (_, child) in node.children_mut() {
        walk_multimedia_typed(child, f)?;
    }
    Ok(())
}

/// The read-only twin of [`walk_multimedia_typed`].
fn visit_multimedia_typed(
    node: &dyn RmObject,
    f: &mut dyn FnMut(&DvMultimedia) -> Result<(), MultimediaError>,
) -> Result<(), MultimediaError> {
    if let Some(multimedia) = node.as_any().downcast_ref::<DvMultimedia>() {
        f(multimedia)?;
    }
    for (_, _, child) in node.children() {
        visit_multimedia_typed(child, f)?;
    }
    Ok(())
}

/// [`qualifies`] over a typed `DV_MULTIMEDIA`.
fn qualifies_typed(value: &DvMultimedia, threshold: usize) -> Result<bool, MultimediaError> {
    if value.uri.is_some() {
        return Ok(false);
    }
    let Some(data) = value.data.as_deref() else {
        return Ok(false);
    };
    let length = decoded_len(data).ok_or_else(|| {
        MultimediaError::Malformed(
            "DV_MULTIMEDIA.data is not base64: its length is not a multiple of 4".to_owned(),
        )
    })?;
    Ok(length > threshold)
}

/// [`needs_offload`] over a typed RM value.
///
/// # Errors
/// [`MultimediaError::Malformed`] when an inline `data` is not padded base64.
pub(super) fn needs_offload_typed(
    root: &dyn RmObject,
    threshold: usize,
) -> Result<bool, MultimediaError> {
    let mut found = false;
    visit_multimedia_typed(root, &mut |value| {
        found = found || qualifies_typed(value, threshold)?;
        Ok(())
    })?;
    Ok(found)
}

/// [`plan_offload`] over a typed RM value: every qualifying inline
/// `DV_MULTIMEDIA` becomes external in place, with the same `uri`,
/// `integrity_check`, `integrity_check_algorithm` and `size` the canonical
/// rewrite writes.
///
/// # Errors
/// [`MultimediaError::Malformed`] when an inline `data` is not base64, or
/// decodes past the `Integer` range RM `DV_MULTIMEDIA.size` holds.
pub(super) fn plan_offload_typed(
    root: &mut dyn RmObject,
    threshold: usize,
    store: &BlobStore,
) -> Result<Vec<Pending>, MultimediaError> {
    let mut pending: Vec<Pending> = Vec::new();
    walk_multimedia_typed(root, &mut |value| {
        if !qualifies_typed(value, threshold)? {
            return Ok(());
        }
        let Some(data) = value.data.take() else {
            return Ok(());
        };
        let mut hasher = Sha256::new();
        let mut size: u64 = 0;
        for chunk in decoded_chunks(&data) {
            let chunk = chunk?;
            hasher.update(&chunk);
            size = size.saturating_add(super::blob::len64(chunk.len()));
        }
        let digest: [u8; 32] = hasher.finalize().into();
        let key = BlobKey::from_digest(digest);
        value.size = i32::try_from(size).map_err(|e| {
            MultimediaError::Malformed(format!(
                "DV_MULTIMEDIA.data exceeds the Integer range of DV_MULTIMEDIA.size: {e}"
            ))
        })?;
        value.uri = Some(DvUri::DvUri(DvUriData {
            value: store.uri_for(&key),
        }));
        value.integrity_check = Some(b64().encode(digest));
        value.integrity_check_algorithm = Some(Box::new(integrity_algorithm()));
        pending.push(Pending { key, size, data });
        Ok(())
    })?;
    Ok(pending)
}

/// The keys the read-path expansion must fetch, sorted and each once: every
/// `DV_MULTIMEDIA` that references one of *our* blobs and is not already
/// inline.
pub(super) fn collect_expand_keys(root: &Value, store: &BlobStore) -> Vec<BlobKey> {
    let mut out = Vec::new();
    visit_multimedia(root, &mut |map| {
        if map.get("data").and_then(Value::as_str).is_none()
            && let Some(key) = uri_of(map).and_then(|uri| store.key_from_uri(uri))
        {
            out.push(key);
        }
    });
    out.sort_unstable();
    out.dedup();
    out
}

/// A fetched, verified blob: its base64 text and its unencoded length.
#[derive(Debug)]
pub(super) struct Fetched {
    pub(super) data: String,
    pub(super) size: u64,
}

/// Re-inline externalized `DV_MULTIMEDIA` values in place from the verified
/// blobs the caller fetched. The `uri` and integrity fields are kept (the value
/// is now *expanded*: `is_inline` and `is_external` both true, which the RM
/// allows), so a later commit of the same body stores it verbatim.
///
/// # Errors
/// [`MultimediaError::Size`] when a value's declared `size` is not the length
/// of the blob it references (`Size_valid`); nothing is inlined then.
pub(super) fn apply_expand(
    root: &mut Value,
    fetched: &BTreeMap<BlobKey, Fetched>,
    store: &BlobStore,
) -> Result<(), MultimediaError> {
    let mut error: Option<MultimediaError> = None;
    visit_multimedia(root, &mut |map| {
        if error.is_some() || map.get("data").and_then(Value::as_str).is_some() {
            return;
        }
        let Some(key) = uri_of(map).and_then(|uri| store.key_from_uri(uri)) else {
            return;
        };
        let Some(blob) = fetched.get(&key) else {
            return;
        };
        if let Some(declared) = map.get("size").and_then(Value::as_u64)
            && declared != blob.size
        {
            error = Some(MultimediaError::Size {
                key,
                declared,
                actual: blob.size,
            });
        }
    });
    if let Some(e) = error {
        return Err(e);
    }
    walk_multimedia(root, &mut |map| {
        if map.get("data").and_then(Value::as_str).is_some() {
            return;
        }
        if let Some(blob) = uri_of(map)
            .and_then(|uri| store.key_from_uri(uri))
            .and_then(|key| fetched.get(&key))
        {
            map.insert("data".to_owned(), Value::String(blob.data.clone()));
        }
    });
    Ok(())
}

/// A base64 encoder fed in chunks of any length: it encodes every whole
/// 3-byte group as it arrives and carries the remainder to the next chunk.
#[derive(Debug, Default)]
pub(super) struct Base64Text {
    out: String,
    carry: Vec<u8>,
}

impl Base64Text {
    /// Appends the encoding of `chunk`.
    pub(super) fn push(&mut self, mut chunk: &[u8]) {
        if !self.carry.is_empty() {
            let wanted = 3 - self.carry.len();
            let (head, rest) = chunk.split_at(wanted.min(chunk.len()));
            self.carry.extend_from_slice(head);
            chunk = rest;
            if self.carry.len() < 3 {
                return;
            }
            b64().encode_string(&self.carry, &mut self.out);
            self.carry.clear();
        }
        let whole = chunk.len() - chunk.len() % 3;
        let (groups, tail) = chunk.split_at(whole);
        b64().encode_string(groups, &mut self.out);
        self.carry.extend_from_slice(tail);
    }

    /// The finished, padded text.
    pub(super) fn finish(mut self) -> String {
        b64().encode_string(&self.carry, &mut self.out);
        self.out
    }
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

    fn store() -> BlobStore {
        BlobStore::in_memory("media")
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
        assert_eq!(pending, Vec::<Pending>::new());
        assert_eq!(comp, before, "small inline media must be byte-identical");
        assert!(!needs_offload(&comp, 256).unwrap());
    }

    #[test]
    fn above_threshold_offloads_and_honours_invariants() {
        let s = store();
        let raw = vec![0x42u8; 1000];
        let want = BlobKey::of(&raw);
        let want_ic = b64().encode(want.digest());
        let mut comp = wrap(multimedia(1000));
        assert!(needs_offload(&comp, 256).unwrap());
        let pending = plan_offload(&mut comp, 256, &s).unwrap();

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].key, want);
        assert_eq!(pending[0].size, 1000);
        assert_eq!(pending[0].data, b64().encode(&raw));
        assert!(
            !needs_offload(&comp, 256).unwrap(),
            "nothing left to offload"
        );

        let node = comp.pointer("/content/0/value").unwrap();
        // data removed; uri present (is_external) → Not_empty holds.
        assert!(node.get("data").is_none());
        assert_eq!(
            node.pointer("/uri/value").unwrap().as_str().unwrap(),
            format!("s3://media/{want}")
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
    fn a_large_value_decodes_window_by_window_to_the_same_bytes() {
        let raw: Vec<u8> = (0..200_000u32)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        let text = b64().encode(&raw);
        let decoded: Vec<u8> = decoded_chunks(&text)
            .map(Result::unwrap)
            .flat_map(|chunk| chunk.to_vec())
            .collect();
        assert_eq!(decoded, raw);
        assert_eq!(decoded_len(&text), Some(raw.len()));
    }

    #[test]
    fn data_that_is_not_base64_is_malformed() {
        let s = store();
        let mut node = multimedia(1000);
        node.as_object_mut()
            .unwrap()
            .insert("data".to_owned(), json!("not base64!"));
        let err = plan_offload(&mut wrap(node), 4, &s).unwrap_err();
        assert!(matches!(err, MultimediaError::Malformed(_)), "{err:?}");
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
        assert_eq!(pending, Vec::<Pending>::new());
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
        let key = BlobKey::of(&raw);
        let mut comp = wrap(multimedia(1000));
        plan_offload(&mut comp, 256, &s).unwrap();

        let keys = collect_expand_keys(&comp, &s);
        assert_eq!(keys, vec![key]);

        // The re-inlined data must decode back to the *original* blob bytes.
        let mut text = Base64Text::default();
        for chunk in raw.chunks(7) {
            text.push(chunk);
        }
        let data = text.finish();
        assert_eq!(data, b64().encode(&raw));
        let fetched = BTreeMap::from([(
            key,
            Fetched {
                data: data.clone(),
                size: 1000,
            },
        )]);
        apply_expand(&mut comp, &fetched, &s).unwrap();

        let node = comp.pointer("/content/0/value").unwrap();
        assert_eq!(node.get("data").unwrap(), &json!(data));
        // uri kept → expanded (both is_inline and is_external), spec-legal.
        assert!(node.get("uri").is_some());
    }

    #[test]
    fn a_declared_size_other_than_the_blob_length_refuses_the_expansion() {
        let s = store();
        let raw = vec![0x42u8; 1000];
        let key = BlobKey::of(&raw);
        let mut comp = wrap(multimedia(1000));
        plan_offload(&mut comp, 256, &s).unwrap();
        let before = comp.clone();
        let fetched = BTreeMap::from([(
            key,
            Fetched {
                data: b64().encode(&raw[..999]),
                size: 999,
            },
        )]);
        let err = apply_expand(&mut comp, &fetched, &s).unwrap_err();
        assert!(
            matches!(
                err,
                MultimediaError::Size {
                    declared: 1000,
                    actual: 999,
                    ..
                }
            ),
            "{err:?}"
        );
        assert_eq!(comp, before, "nothing inlined");
    }

    #[test]
    fn the_chunked_encoder_matches_the_one_shot_encoding_for_every_split() {
        let raw: Vec<u8> = (0..50u8).collect();
        for split in 1..8 {
            let mut text = Base64Text::default();
            for chunk in raw.chunks(split) {
                text.push(chunk);
            }
            assert_eq!(text.finish(), b64().encode(&raw), "split {split}");
        }
    }

    #[test]
    fn referenced_keys_finds_our_blobs_only() {
        let s = store();
        let mut comp = wrap(multimedia(1000));
        plan_offload(&mut comp, 256, &s).unwrap();
        let keys = referenced_keys(&comp, &s);
        assert_eq!(keys.len(), 1);
    }

    /// The typed offload rewrites a COMPOSITION to the canonical value the
    /// canonical rewrite gives, a thumbnail and a below-threshold value
    /// included, and plans the same uploads.
    #[test]
    fn the_typed_offload_writes_what_the_canonical_one_writes() {
        use openehr_rm::v1_2::access::AsRm as _;
        let mut large = multimedia(64);
        large["thumbnail"] = multimedia(40);
        let element = |value: Value| {
            json!({ "_type": "ELEMENT", "archetype_node_id": "at0001",
                "name": { "_type": "DV_TEXT", "value": "image" }, "value": value })
        };
        let canonical = json!({
            "_type": "ITEM_TREE", "archetype_node_id": "at0000",
            "name": { "_type": "DV_TEXT", "value": "tree" },
            "items": [element(large), element(multimedia(8))]
        });
        let mut typed: openehr_rm::prelude::ItemTree =
            openehr_its::json::from_canonical_value(&canonical).unwrap();
        let mut expected = canonical;
        let store = store();
        let planned = plan_offload(&mut expected, 16, &store).unwrap();
        let typed_planned = plan_offload_typed(typed.as_rm_mut(), 16, &store).unwrap();
        assert_eq!(openehr_its::json::to_canonical_value(&typed), expected);
        assert_eq!(typed_planned, planned);
        assert_eq!(planned.len(), 2);
        assert!(!needs_offload_typed(typed.as_rm(), 16).unwrap());
    }
}
