// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Read-fidelity gate against real EHRbase XML fixtures.

use openehr_its::xml::runtime::to_xml;
use openehr_its::xml::runtime::{FromXml, ToXml, from_xml};
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
    let out = to_xml(&value, tag, openehr_its::xml::runtime::Namespace::V2)
        .map_err(|e| format!("serialize {file}: {e}"))?;
    let reparsed: T = from_xml(&out).map_err(|e| format!("reparse {file}: {e}"))?;
    let out2 = to_xml(&reparsed, tag, openehr_its::xml::runtime::Namespace::V2)
        .map_err(|e| format!("serialize2 {file}: {e}"))?;
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

    // `RIPPLE_conformanceTesting_RAW.xml` is a raw-DB export, not canonical
    // ITS-XML; asserting it exists keeps the exclusion from going stale.
    assert!(
        samples_dir()
            .join("RIPPLE_conformanceTesting_RAW.xml")
            .exists(),
        "raw-DB fixture missing; revisit the exclusion note"
    );
}
