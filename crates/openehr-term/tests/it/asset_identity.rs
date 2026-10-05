// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The vendored terminology assets are byte-identical to the definitive
//! computable expression.
//!
//! TERM `SupportTerminology/master02-overview.adoc` names the computable form
//! in the openEHR `specifications-TERM` repository as "the definitive
//! expression" of the Support Terminology. That repository is vendored twice
//! in this workspace at the same pinned commit (`assets/PROVENANCE.md` and
//! `docs/specs/openehr/TERM/PROVENANCE.md`): the crate embeds the XML under
//! `assets/`, and the spec-text vendoring carries the same files under
//! `docs/specs/openehr/TERM/computable/XML/`. Each pair must stay
//! byte-identical — an edit to either side (a "cleanup", a re-vendor of one
//! copy without the other) is a defect this test makes loud.

/// One (crate asset, vendored computable expression) pair, both embedded at
/// compile time so the test needs no runtime filesystem layout assumptions.
const PAIRS: [(&str, &str, &str); 7] = [
    (
        "en/openehr_terminology.xml",
        include_str!("../../assets/en/openehr_terminology.xml"),
        include_str!(
            "../../../../docs/specs/openehr/TERM/computable/XML/en/openehr_terminology.xml"
        ),
    ),
    (
        "es/openehr_terminology.xml",
        include_str!("../../assets/es/openehr_terminology.xml"),
        include_str!(
            "../../../../docs/specs/openehr/TERM/computable/XML/es/openehr_terminology.xml"
        ),
    ),
    (
        "ja/openehr_terminology.xml",
        include_str!("../../assets/ja/openehr_terminology.xml"),
        include_str!(
            "../../../../docs/specs/openehr/TERM/computable/XML/ja/openehr_terminology.xml"
        ),
    ),
    (
        "pt/openehr_terminology.xml",
        include_str!("../../assets/pt/openehr_terminology.xml"),
        include_str!(
            "../../../../docs/specs/openehr/TERM/computable/XML/pt/openehr_terminology.xml"
        ),
    ),
    (
        "zh/openehr_terminology.xml",
        include_str!("../../assets/zh/openehr_terminology.xml"),
        include_str!(
            "../../../../docs/specs/openehr/TERM/computable/XML/zh/openehr_terminology.xml"
        ),
    ),
    (
        "openehr_external_terminologies.xml",
        include_str!("../../assets/openehr_external_terminologies.xml"),
        include_str!(
            "../../../../docs/specs/openehr/TERM/computable/XML/openehr_external_terminologies.xml"
        ),
    ),
    (
        "PropertyUnitData.xml",
        include_str!("../../assets/PropertyUnitData.xml"),
        include_str!("../../../../docs/specs/openehr/TERM/computable/XML/PropertyUnitData.xml"),
    ),
];

#[test]
fn assets_are_byte_identical_to_the_vendored_computable_expression() {
    for (name, asset, definitive) in PAIRS {
        assert_eq!(
            asset, definitive,
            "crate asset {name} diverged from the vendored definitive \
             computable expression (docs/specs/openehr/TERM/computable/XML) — \
             re-vendor both sides together at the same pinned commit",
        );
    }
}
