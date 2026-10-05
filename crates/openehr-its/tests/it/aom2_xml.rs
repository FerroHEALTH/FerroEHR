// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! AOM2 archetype XML codec gate — the ADL 2 counterpart of `opt14`.
//!
//! Corpus: the **8 example documents openEHR ships inside the vendored ITS-XML
//! bundle** (`schemas/xml/its-xml-1.0.2-nsv1/AOM2/examples/`). They are the only
//! official AOM2 XML instance documents we have: the upstream
//! `openEHR/adl-archetypes` library publishes ADL text only (`.adl`/`.adls`), and
//! `openEHR/specifications-ITS-XML` has just three branches — `Release-1.0.2`,
//! `Release-2.0.0` and `master`, the last two identical and both already pinned
//! here — so there is no further upstream corpus to vendor. That ceiling is
//! stated rather than implied.
//!
//! Every document declares `xsi:schemaLocation="… ../P_Archetype.xsd"` with root
//! element `<archetype>` of type `P_AUTHORED_ARCHETYPE` — the PERSISTENT AOM2
//! form (`P_C_COMPLEX_OBJECT`, `P_C_ATTRIBUTE`, …), which is what
//! `openehr-codegen -- emit-aom2` generates a codec for.
//!
//! Corpus discipline: 100% exercised, adjudicated refusals only, asserted rather
//! than tolerated (`.claude/rules/vendored-corpora.md`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use openehr_its::aom2::types::PAuthoredArchetype;

/// Example documents the codec cannot read, with the reason. A refusal is a
/// NEGATIVE test — the gate fails if the document starts reading, so the entry
/// cannot mask a loosened codec.
const ADJUDICATED: &[(&str, &str)] = &[];

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/xml/its-xml-1.0.2-nsv1/AOM2/examples")
}

fn xml_files(dir: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("read examples dir {}: {e}", dir.display()),
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "xml"))
        .collect();
    out.sort();
    out
}

/// Every vendored AOM2 example document is read by the generated codec.
#[test]
fn aom2_example_documents_read() {
    let dir = examples_dir();
    let files = xml_files(&dir);
    assert_eq!(
        files.len(),
        8,
        "the vendored AOM2 example set changed ({} documents in {}) — re-check the \
         bundle provenance before adjusting this count",
        files.len(),
        dir.display()
    );

    let mut findings = String::new();
    let mut stale = Vec::new();
    let mut read = 0_usize;
    let mut refused = 0_usize;

    for path in &files {
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("?")
            .to_owned();
        let expected = ADJUDICATED.iter().find(|(file, _)| *file == name);
        let xml = std::fs::read_to_string(path).expect("read example document");
        match (
            openehr_its::xml::runtime::from_xml::<PAuthoredArchetype>(&xml),
            expected,
        ) {
            (Ok(_), None) => read += 1,
            (Ok(_), Some((_, reason))) => stale.push(format!(
                "{name}: adjudicated as unreadable ({reason}) but the codec now reads \
                 it — remove the entry"
            )),
            (Err(e), None) => {
                let _ = writeln!(findings, "{name}: AOM2 XML read failed: {e}");
            }
            (Err(_), Some(_)) => refused += 1,
        }
    }

    println!(
        "AOM2 examples: {} documents, {read} read, {refused} adjudicated",
        files.len()
    );

    assert!(
        stale.is_empty(),
        "adjudications that no longer describe reality:\n{}",
        stale.join("\n")
    );
    assert!(
        findings.is_empty(),
        "unadjudicated failures across the vendored AOM2 example set:\n{findings}"
    );
    assert_eq!(
        read + refused,
        files.len(),
        "accounting mismatch over the AOM2 example set"
    );
}

/// A read AOM2 archetype round-trips back to XML and re-reads equal — the codec
/// is symmetric, not merely lenient on input.
#[test]
fn aom2_examples_round_trip() {
    let files = xml_files(&examples_dir());
    let mut checked = 0_usize;
    let mut findings = String::new();
    for path in &files {
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("?")
            .to_owned();
        if ADJUDICATED.iter().any(|(file, _)| *file == name) {
            continue;
        }
        let xml = std::fs::read_to_string(path).expect("read example document");
        let Ok(first) = openehr_its::xml::runtime::from_xml::<PAuthoredArchetype>(&xml) else {
            continue; // the read gate above owns the failure
        };
        match openehr_its::aom2::to_xml(&first) {
            Ok(printed) => {
                match openehr_its::xml::runtime::from_xml::<PAuthoredArchetype>(&printed) {
                    Ok(second) if second == first => checked += 1,
                    Ok(_) => {
                        let _ = writeln!(findings, "{name}: re-read differs from the first read");
                    }
                    Err(e) => {
                        let _ =
                            writeln!(findings, "{name}: serialized output does not re-read: {e}");
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(findings, "{name}: serialization failed: {e}");
            }
        }
    }
    assert!(
        findings.is_empty(),
        "AOM2 XML round-trip failures:\n{findings}"
    );
    assert!(checked > 0, "no AOM2 example was round-tripped");
}

/// Every example carries a NON-EMPTY archetype body — identifier, definition tree
/// and terminology — not just the envelope.
///
/// `P_Archetype.xsd` declares the whole body behind
/// `<xs:group ref="pArchetypeElements"/>` / `<xs:group ref="P_AUTHORED_RESOURCE"/>`,
/// so a codec that ignored `xs:group` references would read and round-trip all 8
/// documents VACUOUSLY, over a struct holding only `other_metadata`. Asserting the
/// body pins that the group expansion is real.
#[test]
fn aom2_examples_carry_the_group_body() {
    let files = xml_files(&examples_dir());
    let mut findings = String::new();
    let mut checked = 0_usize;
    for path in &files {
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("?")
            .to_owned();
        if ADJUDICATED.iter().any(|(file, _)| *file == name) {
            continue;
        }
        let xml = std::fs::read_to_string(path).expect("read example document");
        let Ok(archetype) = openehr_its::xml::runtime::from_xml::<PAuthoredArchetype>(&xml) else {
            continue; // the read gate above owns the failure
        };
        if archetype.archetype_id.concept_id.is_empty() {
            let _ = writeln!(findings, "{name}: empty archetype_id/concept_id");
        }
        if archetype.original_language.is_empty() {
            let _ = writeln!(findings, "{name}: empty original_language");
        }
        if archetype.definition.attributes.is_empty() {
            let _ = writeln!(findings, "{name}: definition has no attributes");
        }
        if archetype.terminology.term_definitions.is_empty() {
            let _ = writeln!(findings, "{name}: terminology has no term_definitions");
        }
        checked += 1;
    }
    assert!(
        findings.is_empty(),
        "AOM2 examples read as an empty envelope — the archetype body was dropped:\n{findings}"
    );
    assert!(checked > 0, "no AOM2 example body was inspected");
}
