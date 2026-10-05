// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::panic,
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "test assertions/diagnostics/fixtures"
)]
//! Upstream maintains the `ehr` and `demographic` `UPDATE_VERSION` schemas as a
//! HAND-COPIED PAIR, and says so in-band:
//!
//! > copy of same schema from `ehr`, as we need to redefine to use only
//! > demographic types / reminder to keep them in sync!
//! > — `docs/specs/openehr/ITS-REST/specifications/schemas/demographic/UpdateVersion.yaml`
//!
//! A hand-maintained duplicate with a keep-in-sync reminder is a drift hazard
//! this workspace inherits SILENTLY: `emit-rest` reads the bundled OAS, so if
//! upstream lets the two diverge on the next pin bump, the generated `ehr` and
//! `demographic` contracts diverge with them and nothing here notices. These
//! gates make that loud, on both layers:
//!
//! 1. the SOURCE pair under the vendored spec text — identical modulo the one
//!    difference the note itself declares, the `data` `$ref` (the only reason
//!    the copy exists);
//! 2. the BUNDLED components the generator actually consumes
//!    (`{ehr,demographic}-codegen.openapi.yaml`) — where `$ref` resolution
//!    collapses even that one difference, so they must be byte-identical.

use std::path::{Path, PathBuf};

/// The vendored spec-text schema source for `<api>/UpdateVersion.yaml`.
fn source_schema(api: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/specs/openehr/ITS-REST/specifications/schemas")
        .join(api)
        .join("UpdateVersion.yaml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The vendored bundled OAS the `emit-rest` generator reads.
fn bundle(api: &str) -> (PathBuf, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("vendor/rest-oas")
        .join(format!("{api}-codegen.openapi.yaml"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    (path, text)
}

/// The `components.schemas.<name>` block of a bundled OAS document, verbatim.
///
/// Extracted by indentation: the block runs from its `    <name>:` key to the
/// next line at that same indent, which is the next sibling schema.
fn component_block(text: &str, name: &str) -> String {
    let key = format!("    {name}:");
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|l| *l == key)
        .unwrap_or_else(|| panic!("component {name} not found"));
    let mut end = lines.len();
    for (i, l) in lines.iter().enumerate().skip(start + 1) {
        if !l.trim().is_empty() && !l.starts_with("     ") {
            end = i;
            break;
        }
    }
    lines[start..end].join("\n")
}

/// Drop upstream's leading `#` note block and normalize the ONE line the note
/// declares as the intended difference, so what remains is the structure the
/// two copies must share.
fn normalized_source(text: &str) -> String {
    let mut out = Vec::new();
    let mut in_header = true;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if in_header && line.trim_start().starts_with('#') {
            continue;
        }
        in_header = false;
        // `data: { $ref: ../<api>/UVersionable.yaml }` — the api-scoped
        // reference the copy exists for.
        if line.trim() == "data:" {
            out.push(line.to_owned());
            if let Some(next) = lines.peek()
                && next.trim().starts_with("$ref:")
            {
                let indent = &next[..next.len() - next.trim_start().len()];
                out.push(format!("{indent}$ref: <api>/UVersionable.yaml"));
                lines.next();
            }
            continue;
        }
        out.push(line.to_owned());
    }
    out.join("\n")
}

#[test]
fn the_vendored_ehr_and_demographic_update_version_sources_stay_in_sync() {
    let ehr = normalized_source(&source_schema("ehr"));
    let demographic = normalized_source(&source_schema("demographic"));
    assert_eq!(
        ehr, demographic,
        "upstream's hand-copied UPDATE_VERSION pair has DIVERGED beyond the \
         `data` $ref the in-band note declares — re-adjudicate the divergence \
         against the ITS-REST docs text before regenerating"
    );
    // The normalization must actually have found the api-scoped reference; a
    // silently-unmatched marker would make the comparison vacuous.
    assert!(
        ehr.contains("$ref: <api>/UVersionable.yaml"),
        "the `data` $ref line was not recognised — the schema shape changed"
    );
}

#[test]
fn the_bundled_update_version_components_are_identical() {
    let (ehr_path, ehr_text) = bundle("ehr");
    let (dem_path, dem_text) = bundle("demographic");
    let ehr = component_block(&ehr_text, "UpdateVersion");
    let dem = component_block(&dem_text, "UpdateVersion");
    assert!(
        ehr.contains("title: UPDATE_VERSION"),
        "extraction missed the component in {}",
        ehr_path.display()
    );
    assert_eq!(
        ehr,
        dem,
        "the bundled UPDATE_VERSION components differ between {} and {} — \
         `emit-rest` would generate two divergent DTOs from what upstream \
         maintains as one schema",
        ehr_path.display(),
        dem_path.display()
    );
}
