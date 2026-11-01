// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The served `OpenAPI` document, held byte for byte against a committed
//! snapshot, so a refactor of the `#[utoipa::path]` declarations cannot change
//! one byte of what the server publishes.
//!
//! The snapshot is the served document pretty-printed with `info.version` (the
//! product version, which every release moves) replaced by [`VERSION_SLOT`].
//! [`snapshot_text`] first proves that the compact re-serialization of the
//! parsed document reproduces the served bytes exactly, so the pretty form
//! loses nothing and the comparison is byte for byte. After a deliberate change
//! to the surface, rewrite the snapshot with the ignored writer
//! (`cargo nextest run -p ferroehr-rest --test it --run-ignored only
//! write_the_served_openapi_snapshot`) and review its diff. No specification
//! governs this: our own design.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helper; a failed fixture step is a failed test here (the Rust \
              Book ch11)"
)]

use std::path::PathBuf;

use serde_json::Value;

use crate::served_openapi::served_bytes;

/// What stands in the snapshot where the served document carries the
/// product version.
const VERSION_SLOT: &str = "@CARGO_PKG_VERSION@";

/// The committed snapshot.
fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/it/snapshots/served-openapi.json")
}

/// The served document in its snapshot form.
async fn snapshot_text() -> String {
    let served = served_bytes().await;
    let mut doc: Value = serde_json::from_slice(&served).expect("served openapi json");
    let compact = serde_json::to_vec(&doc).expect("compact re-serialization");
    assert!(
        compact == served,
        "the served document does not survive a parse and re-serialization unchanged, \
         so its pretty-printed snapshot would not be lossless"
    );
    let version = doc
        .pointer_mut("/info/version")
        .expect("the served document carries info.version");
    assert_eq!(
        version.as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "info.version is the product version"
    );
    *version = Value::String(VERSION_SLOT.to_owned());
    let mut pretty = serde_json::to_string_pretty(&doc).expect("pretty serialization");
    pretty.push('\n');
    pretty
}

/// The served document is byte-identical to the committed snapshot.
#[tokio::test]
async fn the_served_document_matches_its_snapshot() {
    let served = snapshot_text().await;
    let path = snapshot_path();
    let committed = std::fs::read_to_string(&path).expect("read the committed snapshot");
    let line = committed
        .lines()
        .zip(served.lines())
        .position(|(want, got)| want != got)
        .unwrap_or_else(|| committed.lines().count().min(served.lines().count()));
    assert!(
        committed == served,
        "the served OpenAPI document differs from {} from line {}:\n  snapshot: {}\n  served:   {}\n\
         rewrite it with write_the_served_openapi_snapshot only for a deliberate surface change",
        path.display(),
        line + 1,
        committed.lines().nth(line).unwrap_or("<end of file>"),
        served.lines().nth(line).unwrap_or("<end of file>"),
    );
}

/// Rewrites the committed snapshot from the served document.
#[tokio::test]
#[ignore = "the snapshot writer: run it after a deliberate change to the served surface"]
async fn write_the_served_openapi_snapshot() {
    std::fs::write(snapshot_path(), snapshot_text().await).expect("write the snapshot");
}
