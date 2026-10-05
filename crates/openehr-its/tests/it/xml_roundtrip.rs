// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! XML round-trip fidelity gate: for every composition in the openEHR
//! corpus, RM → XML → RM → XML must be stable, proving the generated `ToXml` and
//! `FromXml` impls are mutually consistent on real data.
use openehr_its::xml::runtime::from_xml;
use openehr_its::xml::to_canonical_xml;
use openehr_rm::prelude::{Composition, FeederAuditDetails};
use std::path::Path;

fn corpus_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vendor/openehr_sdk/composition/canonical_json")
}

#[test]
fn composition_xml_round_trips() {
    // Exclusions come from the SINGLE registry (`common::excluded`) — never a
    // second by-name list here, which is how the three mechanisms drifted apart.
    // Where the adjudication produced a repo-authored VALID TWIN,
    // `common::twinned` substitutes it so the exclusion costs no coverage.
    let (mut ok, mut skipped) = (0, 0);
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(corpus_dir()).expect("corpus dir") {
        let path = entry.unwrap().path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("json") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
        let path = if crate::common::excluded(&crate::common::corpus_rel(&path)).is_some() {
            let twin = crate::common::twinned(&path);
            if twin == path {
                skipped += 1;
                continue;
            }
            twin
        } else {
            path
        };
        let json = std::fs::read_to_string(&path).unwrap();
        let Ok(compo) = openehr_its::json::from_canonical_json::<Composition>(&json) else {
            skipped += 1; // not a canonical composition
            continue;
        };
        let xml1 = to_canonical_xml(&compo, "composition").expect("serialize 1");
        match from_xml::<Composition>(&xml1) {
            Ok(compo2) => {
                let xml2 = to_canonical_xml(&compo2, "composition").expect("serialize 2");
                if xml1 == xml2 {
                    ok += 1;
                } else {
                    failures.push(format!("{stem}: round-trip not stable"));
                }
            }
            Err(e) => failures.push(format!("{stem}: parse failed: {e}")),
        }
    }
    eprintln!(
        "xml round-trip: {ok} ok, {skipped} skipped, {} failed",
        failures.len()
    );
    assert!(failures.is_empty(), "failures:\n{}", failures.join("\n"));
    assert!(
        ok > 10,
        "expected many compositions to round-trip, got {ok}"
    );
}

/// `FEEDER_AUDIT_DETAILS.other_details` — the RM 1.2.0 attribute the vendored
/// ITS-XML **v1** bundle (the served-by-default lineage) does not declare.
///
/// RM common `docs/UML/classes/org.openehr.rm.common.feeder_audit_details.adoc`
/// §Attributes types it `0..1 ITEM_STRUCTURE` ("Optional attribute to carry any
/// custom meta-data. May be archetyped."). The v1 XSD's `FEEDER_AUDIT_DETAILS`
/// sequence ends at `version_id`, so the generated codec emits the element from
/// the RM model under the completeness rule, and the lineage split is reported
/// rather than suppressed. This gate pins BOTH halves of that
/// handling: the element is on the wire, and it survives a parse back unchanged.
#[test]
fn feeder_audit_details_other_details_round_trips() {
    let json = r#"{
        "_type": "FEEDER_AUDIT_DETAILS",
        "system_id": "lab.example.org",
        "version_id": "final",
        "other_details": {
            "_type": "ITEM_TREE",
            "name": { "_type": "DV_TEXT", "value": "custom meta-data" },
            "archetype_node_id": "at0001",
            "items": [{
                "_type": "ELEMENT",
                "name": { "_type": "DV_TEXT", "value": "placer id" },
                "archetype_node_id": "at0002",
                "value": { "_type": "DV_TEXT", "value": "PLC-77421" }
            }]
        }
    }"#;
    let details: FeederAuditDetails =
        openehr_its::json::from_canonical_json(json).expect("deserialize FEEDER_AUDIT_DETAILS");
    assert!(
        details.other_details.is_some(),
        "the canonical-JSON reader must carry other_details"
    );

    let xml1 = to_canonical_xml(&details, "originating_system_audit").expect("serialize 1");
    assert!(
        xml1.contains("<other_details"),
        "the emitted XML must carry the other_details element: {xml1}"
    );
    assert!(
        xml1.contains("PLC-77421"),
        "the other_details leaf must reach the wire: {xml1}"
    );

    let parsed: FeederAuditDetails = from_xml(&xml1).expect("parse back");
    assert_eq!(parsed, details, "other_details must survive the round trip");
    let xml2 = to_canonical_xml(&parsed, "originating_system_audit").expect("serialize 2");
    assert_eq!(xml1, xml2, "the round trip must be byte-stable");
}
