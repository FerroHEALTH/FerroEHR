// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! the node codec round-trips the entire canonical-JSON corpus
//! losslessly (in memory — the DB round-trip lives in `persistence.rs`).

use std::path::Path;

use ferroehr::storage::codec::{decompose, reassemble};
use serde_json::Value;

#[test]
fn corpus_round_trips_losslessly() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(dir).expect("corpus dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read corpus file");
        let Ok(composition) = serde_json::from_str::<Value>(&text) else {
            continue; // deliberately-invalid corpus files
        };
        if composition.get("_type").and_then(Value::as_str) != Some("COMPOSITION") {
            continue;
        }
        let rows = decompose(composition.clone())
            .unwrap_or_else(|e| panic!("decompose {}: {e}", path.display()));
        let reassembled =
            reassemble(&rows).unwrap_or_else(|e| panic!("reassemble {}: {e}", path.display()));
        assert_eq!(
            reassembled,
            composition,
            "lossless round-trip failed for {}",
            path.display()
        );
        checked += 1;
    }
    assert!(checked >= 50, "expected the full corpus, got {checked}");
}
