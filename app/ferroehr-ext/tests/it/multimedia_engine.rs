// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The multimedia engine (`ferroehr_ext::multimedia`) end to end over a real
//! object store: a `DV_MULTIMEDIA` above the threshold is offloaded, its bytes
//! land in the store once, the canonical value keeps the invariants RM data
//! types demand of an externalised value, and expanding it restores the inline
//! bytes. The in-memory `object_store` backend stands in for S3; the S3-backed
//! journey is `app/ferroehr/tests/it/multimedia_s3.rs`. The planner's own
//! arithmetic (thresholds, thumbnails, integrity) is unit-tested beside it in
//! `src/multimedia/offload.rs`; this module drives the engine's public seam.
//!
//! No openEHR spec governs blob externalisation — our own design/extension;
//! the value shape is RM data types `DV_MULTIMEDIA`
//! (`docs/specs/openehr/RM/docs/data_types/master09-encapsulated_package.adoc`).

use std::sync::Arc;

use base64::Engine;
use ferroehr_ext::multimedia::store::BlobStore;
use ferroehr_ext::multimedia::{MultimediaEngine, MultimediaError, references_external_blob};
use serde_json::{Value, json};

const THRESHOLD: usize = 256;

fn engine() -> MultimediaEngine {
    MultimediaEngine::from_parts(
        BlobStore::from_parts(
            Arc::new(object_store::memory::InMemory::new()),
            "media".to_owned(),
        ),
        THRESHOLD,
    )
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
    let stored = engine.store().get(&keys[0]).await.expect("stored bytes");
    assert_eq!(stored.as_ref(), payload.as_slice());
    assert_eq!(engine.store().key_from_uri(&uri), Some(keys[0].as_str()));

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
    assert!(engine.referenced_keys(&small).is_empty());

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
    assert_eq!(
        engine.store().get(&k1[0]).await.expect("bytes").as_ref(),
        payload.as_slice()
    );

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
                "uri": { "_type": "DV_URI", "value": engine.store().uri_for("00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff") }
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
