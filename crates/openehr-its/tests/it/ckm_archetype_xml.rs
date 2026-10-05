// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! Breadth gate over the AM 1.4 **archetype XML** twin of the CKM archetype
//! pack (`corpus/archetypes/ckm/xml/`, vendored by
//! `scripts/vendor/ckm-archetypes.sh --with-xml`).
//!
//! Two serializations of the same 944 archetypes are vendored: CKM's ADL 1.4
//! text (gated in `openehr-adl` by `ckm_archetype_packs`) and CKM's AM 1.4
//! `<archetype>` XML, gated here. The XML half exercises a DIFFERENT code path
//! — the generated canonical-XML `FromXml` codec over the `Archetype.xsd`
//! closure (`openehr-codegen -- emit-opt`) — so a defect in one is invisible to
//! the other.
//!
//! The root element is `<archetype>` in `http://schemas.openehr.org/v1`, the
//! AM 1.4 ARCHETYPE serialization (`crates/openehr-its/schemas/xml/components/AM/Release-1.4`,
//! merged into the OPT emission closure alongside `Template.xsd`).
//!
//! Corpus discipline: 100% exercised, adjudicated failures only
//! (`.claude/rules/vendored-corpora.md`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use openehr_its::opt14::types::Archetype;

/// Archetype XML documents the codec cannot read, with the reason. A refusal
/// here is a NEGATIVE test: the gate asserts the read still fails, so a
/// loosened codec cannot hide behind the entry.
///
/// All four entries are the SAME defect in CKM's XML exporter, and the evidence
/// is CKM contradicting itself: the export names `xsi:type="C_DV_SCALE"` on a
/// `C_OBJECT`, but
///
/// * no vendored openEHR schema declares that type — it is absent from every
///   `OpenehrProfile.xsd` we vendor (ITS-XML 1.0.2 `ALL/`, ITS-XML 2.0.0
///   `AM/Release-1.4/`, and `components/AM/Release-1.4/`), all of which DO
///   declare the sibling constrainers `C_DV_ORDINAL`/`C_DV_QUANTITY`; and
/// * CKM's own ADL 1.4 export of the same archetype contains no `DV_SCALE` or
///   `C_DV_SCALE` anywhere, so the constraint is expressed differently in the
///   text serialization — the XML exporter introduces the type on its own.
///
/// An `xsi:type` naming a type the schema does not declare makes the document
/// schema-invalid, so refusing it is correct: accepting an undeclared
/// constrainer would silently fork the AM model. Recorded as a defect in the
/// vendored data, NOT as a codec gap — nothing here asks for a codegen change.
const ADJUDICATED: &[(&str, &str)] = &[
    (
        "openEHR-EHR-CLUSTER.lab_microscopy_culture.v0.xml",
        "xsi:type=\"C_DV_SCALE\" is declared by no vendored openEHR schema",
    ),
    (
        "openEHR-EHR-OBSERVATION.g8_screening_tool.v0.xml",
        "xsi:type=\"C_DV_SCALE\" is declared by no vendored openEHR schema",
    ),
    (
        "openEHR-EHR-OBSERVATION.harris_hip.v0.xml",
        "xsi:type=\"C_DV_SCALE\" is declared by no vendored openEHR schema",
    ),
    (
        "openEHR-EHR-OBSERVATION.visual_acuity.v0.xml",
        "xsi:type=\"C_DV_SCALE\" is declared by no vendored openEHR schema",
    ),
];

fn pack_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/archetypes/ckm/xml")
}

fn xml_files(dir: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("read pack dir {}: {e}", dir.display()),
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "xml"))
        .collect();
    out.sort();
    out
}

/// Every AM 1.4 archetype XML document CKM publishes is read by the generated
/// canonical-XML codec.
#[test]
fn ckm_archetype_xml_pack_reads() {
    let dir = pack_dir();
    let files = xml_files(&dir);
    assert!(
        files.len() >= 900,
        "the CKM archetype XML pack is missing: found {} files in {} — re-run \
         scripts/vendor/ckm-archetypes.sh --with-xml",
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
        let xml = match std::fs::read_to_string(path) {
            Ok(xml) => xml,
            Err(e) => {
                let _ = writeln!(findings, "{name}: read failed: {e}");
                continue;
            }
        };
        match (
            openehr_its::xml::runtime::from_xml::<Archetype>(&xml),
            expected,
        ) {
            (Ok(_), None) => read += 1,
            (Ok(_), Some((_, reason))) => stale.push(format!(
                "{name}: adjudicated as unreadable ({reason}) but the codec now reads \
                 it — remove the entry"
            )),
            (Err(e), None) => {
                let _ = writeln!(findings, "{name}: archetype XML read failed: {e}");
            }
            (Err(_), Some(_)) => refused += 1,
        }
    }

    println!(
        "CKM archetype XML pack: {} files, {read} read, {refused} adjudicated",
        files.len()
    );

    assert!(
        stale.is_empty(),
        "adjudications that no longer describe reality:\n{}",
        stale.join("\n")
    );
    assert!(
        findings.is_empty(),
        "unadjudicated failures across the CKM archetype XML pack:\n{findings}"
    );
    assert_eq!(
        read + refused,
        files.len(),
        "accounting mismatch over the archetype XML pack"
    );
}
