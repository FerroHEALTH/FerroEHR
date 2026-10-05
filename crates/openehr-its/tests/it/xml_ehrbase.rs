// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Read-fidelity gate against real EHRbase XML fixtures.
//!
//! `FromXml` must parse stock EHRbase-authored XML into the RM types, and the
//! parsed value must round-trip (RM → XML → RM → XML stable). These fixtures are
//! real EHRbase test data — full compositions with `xsi:type` polymorphism,
//! namespace prefixes, indentation, and archie-omitted `Interval` flags — so
//! this proves real-world read robustness end to end.
//!
//! They are hand-authored test *inputs* (varying namespace conventions,
//! whitespace), not archie-canonical *output*, so a byte-for-byte C14N compare
//! against the fixture is not the bar here; that awaits archie-canonical vectors
//! / the live parity harness (Stage-1 acceptance). Round-trip stability of our
//! own canonical output is the invariant this gate enforces.

use openehr_its::xml::runtime::{FromXml, ToXml, from_xml};
use openehr_its::xml::to_canonical_xml;
use openehr_rm::prelude::{Composition, ItemTree};

/// The EHRbase-derived XML samples in the shared corpus.
fn samples_dir() -> std::path::PathBuf {
    crate::common::service_corpus_dir().join("samples")
}

/// Parse `xml` into `T`, re-serialize, and confirm the canonical output is
/// stable across a second parse (`FromXml`/`ToXml` are mutually consistent on
/// this real input).
fn read_and_round_trip<T: FromXml + ToXml>(file: &str, tag: &str) -> Result<usize, String> {
    let xml = std::fs::read_to_string(samples_dir().join(file))
        .map_err(|e| format!("read {file}: {e}"))?;
    let value: T = from_xml(&xml).map_err(|e| format!("parse {file}: {e}"))?;
    let out = to_canonical_xml(&value, tag).map_err(|e| format!("serialize {file}: {e}"))?;
    let reparsed: T = from_xml(&out).map_err(|e| format!("reparse {file}: {e}"))?;
    let out2 = to_canonical_xml(&reparsed, tag).map_err(|e| format!("serialize2 {file}: {e}"))?;
    if out == out2 {
        Ok(out.len())
    } else {
        Err(format!("{file}: round-trip not stable"))
    }
}

#[test]
fn ferroehr_xml_fixtures_read_and_round_trip() {
    // Canonical ITS-XML fixtures: a full composition (~1120 lines, xsi:type
    // polymorphism, namespace prefixes, archie-omitted Interval flags) and an
    // ITEM_TREE whose `xsi:type` values carry a `v1:` prefix.
    let results = [
        read_and_round_trip::<Composition>("RIPPLE-ConformanceTest.xml", "composition"),
        read_and_round_trip::<ItemTree>("other_details.xml", "items"),
    ];
    let failures: Vec<String> = results
        .iter()
        .filter_map(|r| r.as_ref().err().cloned())
        .collect();
    let ok = results.iter().filter(|r| r.is_ok()).count();
    eprintln!(
        "EHRbase XML fixtures: {ok}/{} read + round-trip OK",
        results.len()
    );
    assert!(
        failures.is_empty(),
        "EHRbase fixture failures:\n{}",
        failures.join("\n")
    );

    // EXCLUDED: `RIPPLE_conformanceTesting_RAW.xml` is a raw-DB export shape
    // (decomposed row-per-locatable form) whose LOCATABLEs omit the canonical
    // `name` element — the same non-canonical form the JSON gate excludes as
    // `rawdb_*`, not ITS-XML canonical input. Assert it exists so a rename does
    // not silently drop the exclusion.
    assert!(
        samples_dir()
            .join("RIPPLE_conformanceTesting_RAW.xml")
            .exists(),
        "raw-DB fixture missing; revisit the exclusion note"
    );
}
