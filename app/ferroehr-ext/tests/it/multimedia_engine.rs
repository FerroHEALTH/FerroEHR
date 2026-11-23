// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The multimedia engine over an in-memory blob store: a `DV_MULTIMEDIA` above
//! the threshold is offloaded once, keeps the RM invariants of an external
//! value, and expands back to its inline bytes.
//!
//! No openEHR spec governs blob externalisation: our own design. The value
//! shape is RM `DV_MULTIMEDIA`
//! (`docs/specs/openehr/RM/docs/data_types/master09-encapsulated_package.adoc`).

#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule (.claude/rules/testing.md §Test-fixture construction): a canonical composition is bytes a client sends, and building it through a typed model would test the model instead of the code under test"
)]

use base64::Engine;
use ferroehr_ext::multimedia::blob::{BlobKey, BlobStore};
use ferroehr_ext::multimedia::{MultimediaEngine, MultimediaError, references_external_blob};
use serde_json::{Value, json};

const THRESHOLD: usize = 256;

fn engine() -> MultimediaEngine {
    MultimediaEngine::from_parts(BlobStore::in_memory("media"), THRESHOLD)
}

/// The bytes the store holds under `key`, read through the verified stream.
async fn stored_bytes(store: &BlobStore, key: &BlobKey) -> Vec<u8> {
    let mut out = Vec::new();
    store
        .read_verified(key, |chunk| {
            out.extend_from_slice(chunk);
            Ok::<(), MultimediaError>(())
        })
        .await
        .expect("stored bytes");
    out
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A composition carrying one `DV_MULTIMEDIA` of `n` bytes inline.
fn composition_with_media(n: usize) -> (Value, Vec<u8>) {
    let payload: Vec<u8> = (0..n)
        .map(|i| u8::try_from(i % 251).expect("< 256"))
        .collect();
    let composition = json!({
        "_type": "COMPOSITION",
        "content": [{
            "_type": "ELEMENT",
            "value": {
                "_type": "DV_MULTIMEDIA",
                "media_type": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_media-types" },
                    "code_string": "application/octet-stream"
                },
                "size": n,
                "data": b64(&payload)
            }
        }]
    });
    (composition, payload)
}

#[tokio::test]
async fn a_large_value_round_trips_through_the_store() {
    let engine = engine();
    let (mut composition, payload) = composition_with_media(4 * THRESHOLD);
    let inline = composition.clone();

    engine.offload(&mut composition).await.expect("offload");
    let node = composition
        .pointer("/content/0/value")
        .expect("the multimedia node");
    assert!(node.get("data").is_none(), "the bytes left the document");
    let uri = node
        .pointer("/uri/value")
        .and_then(Value::as_str)
        .expect("an external uri")
        .to_owned();
    assert!(uri.starts_with("s3://media/"), "{uri}");
    assert_eq!(
        node.get("size"),
        Some(&json!(4 * THRESHOLD)),
        "size stays the decoded length"
    );
    assert!(
        node.get("integrity_check").is_some(),
        "the digest travels with the reference"
    );
    assert!(references_external_blob(&composition));

    // The bytes are in the store under the key the uri names, verbatim.
    let keys = engine.referenced_keys(&composition);
    assert_eq!(keys.len(), 1);
    let stored = stored_bytes(engine.store(), &keys[0]).await;
    assert_eq!(stored, payload);
    assert_eq!(engine.store().key_from_uri(&uri), Some(keys[0]));

    // Expanding restores the inline document exactly.
    engine.expand(&mut composition).await.expect("expand");
    let restored = composition
        .pointer("/content/0/value/data")
        .and_then(Value::as_str)
        .expect("inline data again");
    assert_eq!(restored, b64(&payload));
    // The reference stays beside the restored bytes: a value that is both
    // inline and external is spec-legal, and a later offload finds the same key.
    assert_eq!(
        composition
            .pointer("/content/0/value/uri/value")
            .and_then(Value::as_str),
        Some(uri.as_str())
    );
    assert_eq!(
        composition.pointer("/content/0/value/size"),
        inline.pointer("/content/0/value/size")
    );
}

#[tokio::test]
async fn a_small_value_and_a_disabled_engine_leave_the_document_untouched() {
    let engine = engine();
    let (mut small, _) = composition_with_media(THRESHOLD - 1);
    let before = small.clone();
    engine.offload(&mut small).await.expect("offload");
    assert_eq!(small, before, "below the threshold nothing moves");
    assert_eq!(engine.referenced_keys(&small), Vec::<BlobKey>::new());

    let disabled = engine.with_offload_enabled(false);
    assert!(!disabled.offload_enabled());
    let (mut large, _) = composition_with_media(4 * THRESHOLD);
    let before = large.clone();
    disabled.offload(&mut large).await.expect("offload");
    assert_eq!(
        large, before,
        "a disabled engine offloads nothing, whatever the size"
    );
}

#[tokio::test]
async fn the_same_bytes_are_stored_once_and_expand_is_idempotent() {
    let engine = engine();
    let (mut first, payload) = composition_with_media(3 * THRESHOLD);
    let (mut second, _) = composition_with_media(3 * THRESHOLD);
    engine.offload(&mut first).await.expect("offload first");
    engine.offload(&mut second).await.expect("offload second");
    let k1 = engine.referenced_keys(&first);
    let k2 = engine.referenced_keys(&second);
    assert_eq!(k1, k2, "content addressing: identical bytes share one key");
    assert_eq!(stored_bytes(engine.store(), &k1[0]).await, payload);

    engine.expand(&mut first).await.expect("expand");
    let once = first.clone();
    engine.expand(&mut first).await.expect("expand again");
    assert_eq!(
        first, once,
        "expanding an already inline document changes nothing"
    );
}

#[tokio::test]
async fn a_reference_whose_bytes_are_missing_is_a_typed_refusal() {
    let engine = engine();
    let mut composition = json!({
        "_type": "COMPOSITION",
        "content": [{
            "_type": "ELEMENT",
            "value": {
                "_type": "DV_MULTIMEDIA",
                "media_type": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_media-types" },
                    "code_string": "application/octet-stream"
                },
                "size": 3,
                "uri": { "_type": "DV_URI", "value": engine.store().uri_for(&"00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff".parse().expect("a digest")) }
            }
        }]
    });
    let err = engine
        .expand(&mut composition)
        .await
        .expect_err("a reference into our store with no object behind it cannot be expanded");
    assert!(
        !matches!(err, MultimediaError::Malformed(_)),
        "a missing object is a store failure, got {err:?}"
    );
    assert!(
        composition.pointer("/content/0/value/data").is_none(),
        "nothing was invented for the missing bytes"
    );
}
