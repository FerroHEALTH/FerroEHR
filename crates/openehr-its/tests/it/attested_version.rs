// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::expect_used,
    reason = "test assertions and fixture plumbing outside #[test] fns, which the \
              clippy.toml allow-*-in-tests scoping does not reach"
)]
//! The `ORIGINAL_VERSION.attestations` serialization vector.
//!
//! `ORIGINAL_VERSION` carries `attestations: List<ATTESTATION> [0..1]` (RM
//! common `docs/UML/classes/org.openehr.rm.common.original_version.adoc`
//! §Attributes), and `ATTESTATION` is an `AUDIT_DETAILS` subtype adding
//! `attested_view` / `proof` / `items` / `reason` / `is_pending` (RM common
//! `docs/UML/classes/org.openehr.rm.common.attestation.adoc` §Attributes).
//! RM common `master04-generic_package.adoc` §Attestation gives the two-object
//! pattern the vector encodes: an attestation committed with `_is_pending_`
//! True, then a second one with `_is_pending_` False "and the appropriate
//! proof supplied".
//!
//! The vector is the CNF catalogue's committed corpus pair
//! (`cnf.version.attested` / `cnf.version.attested.xml`), so this gate also
//! keeps the two committed representations from drifting apart: the XML twin
//! must be exactly what the codec emits from the JSON twin.

use std::path::{Path, PathBuf};

use openehr_its::xml::runtime::Namespace;
use openehr_its::xml::{from_canonical_xml, to_canonical_xml, to_canonical_xml_declared};
use openehr_rm::v1_2::common::change_control::original_version::OriginalVersion;
use openehr_rm::v1_2::composition::composition::Composition;

/// The CNF corpus directory holding the committed vector pair.
fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fixtures/attestation")
}

/// The committed canonical-JSON vector, decoded into the generated RM type.
fn attested_version() -> OriginalVersion<Composition> {
    let json = std::fs::read_to_string(corpus_dir().join("attested_version.v1.json"))
        .expect("the committed canonical-JSON vector");
    openehr_its::json::from_canonical_json(&json).expect("typed ORIGINAL_VERSION<COMPOSITION>")
}

/// Canonical JSON → RM → canonical JSON carries both attestations, in order,
/// with every `ATTESTATION` attribute intact.
#[test]
fn the_attested_version_round_trips_through_canonical_json() {
    let json = std::fs::read_to_string(corpus_dir().join("attested_version.v1.json"))
        .expect("the committed canonical-JSON vector");
    let committed: serde_json::Value =
        serde_json::from_str(&json).expect("the vector is valid JSON");
    let typed = attested_version();
    let emitted = openehr_its::json::to_canonical_value(&typed);

    let attestations = emitted["attestations"]
        .as_array()
        .expect("attestations survive the decode");
    assert_eq!(attestations.len(), 2, "both attestations: {emitted}");
    // master04 §Attestation: the pending attestation first, then the completed
    // one carrying its proof.
    assert_eq!(attestations[0]["_type"], "ATTESTATION");
    assert_eq!(attestations[0]["is_pending"], serde_json::json!(true));
    assert!(
        attestations[0].get("proof").is_none(),
        "a pending attestation has no proof yet: {}",
        attestations[0]
    );
    assert_eq!(attestations[1]["is_pending"], serde_json::json!(false));
    assert!(attestations[1]["proof"].is_string());
    // Items_valid: present implies non-empty (attestation.adoc §Invariants).
    assert_eq!(
        attestations[1]["items"].as_array().map(Vec::len),
        Some(1),
        "the completed attestation names the item it attests"
    );
    assert_eq!(attestations[1]["items"][0]["_type"], "DV_EHR_URI");
    // Reason_valid: the coded reasons are members of the openEHR
    // `attestation reason` group (TERM SupportTerminology: 648 witnessed,
    // 240 signed).
    assert_eq!(
        attestations[0]["reason"]["defining_code"]["code_string"],
        "648"
    );
    assert_eq!(
        attestations[1]["reason"]["defining_code"]["code_string"],
        "240"
    );

    // The whole document is stable: the committed vector IS the codec's own
    // canonical output, so a reader/writer drift fails here.
    assert_eq!(
        emitted, committed,
        "the committed JSON vector must be the codec's canonical output"
    );
}

/// The same value survives the canonical-XML lineage: RM → XML → RM → XML is
/// stable, and the committed XML twin is byte-identical to the codec's output
/// under the published `<version>` root.
#[test]
fn the_attested_version_round_trips_through_canonical_xml() {
    let typed = attested_version();
    let xml = to_canonical_xml_declared(&typed, "version", "VERSION", Namespace::V1)
        .expect("serialize under the published <version> root");
    assert!(
        xml.contains("<attestations>"),
        "the attestations reach the XML wire: {xml}"
    );

    let back: OriginalVersion<Composition> = from_canonical_xml(&xml).expect("re-read");
    assert_eq!(
        to_canonical_xml(&back, "original_version").expect("re-serialize"),
        to_canonical_xml(&typed, "original_version").expect("serialize"),
        "the XML round trip must carry the whole ORIGINAL_VERSION, attestations included"
    );

    let committed = std::fs::read_to_string(corpus_dir().join("attested_version.v1.xml"))
        .expect("the committed canonical-XML twin");
    assert_eq!(
        xml, committed,
        "the committed XML twin must be exactly what the codec emits from the JSON twin \
         (regenerate with `cargo run -p openehr-its --example canonical_convert`)"
    );
}
