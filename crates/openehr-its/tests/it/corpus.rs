// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Sanity + wiring for the vendored ITS material: the EHRbase canonical-JSON
//! corpus (`tests/vendor/`) and the ITS-JSON schema (`schemas/`).
//!
//! The full interop fidelity gates live in their own suites: `fidelity.rs`
//! (corpus → generated `openehr-rm` types → re-serialize → normalized
//! value-equality + ITS-JSON schema validation, with documented exclusions)
//! and `xml_roundtrip.rs`/`xml_c14n.rs` (the canonical-XML side). This file
//! only sanity-checks the vendored material itself.

use std::fs;
use std::path::Path;

fn corpus_files() -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vendor");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "json") {
                    out.push(p);
                }
            }
        }
    }
    out
}

#[test]
fn corpus_is_present_and_valid_json() {
    let files = corpus_files();
    assert!(
        files.len() >= 50,
        "expected the vendored EHRbase corpus (>=50 files), found {}",
        files.len()
    );
    for f in &files {
        let txt = fs::read_to_string(f).unwrap();
        let _parsed: serde_json::Value = serde_json::from_str(&txt)
            .unwrap_or_else(|e| panic!("corpus file {} is not valid JSON: {e}", f.display()));
    }
}

#[test]
fn its_json_schema_is_valid_json() {
    // The vendored ITS-JSON RM schema parses (wiring of `json::RM_SCHEMA_JSON`).
    let _schema: serde_json::Value = serde_json::from_str(openehr_its::json::RM_SCHEMA_JSON)
        .expect("vendored ITS-JSON schema must be valid JSON");
}
