// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Parse gate over the ADL 1.4 archetype resources of the shared corpus
//! (`corpus/fixtures/service/knowledge/archetypes/`).
//!
//! These are real-world CKM-lineage 1.4 sources (Ocean Informatics, 2007) —
//! the only artefacts in the tree that carry `ac`-code constraints with a
//! `constraint_definitions` ontology section and the inline dADL
//! `C_DV_QUANTITY` domain blocks of
//! `docs/specs/openehr/AM/docs/ADL1.4/master09-customising_adl.adoc`. Most were
//! reachable only through the database-backed `service_definition` suite, so a
//! parser regression on the constructs unique to them went unseen; this gate is
//! DB-free and pins each file's outcome by name.
//!
//! The outcome is the SPEC's verdict on the artefact, not an assumption that a
//! published artefact must be valid: two of them state an assumed value their
//! own constraint excludes, which `ADL1.4/master05-cadl.adoc` §Assumed Values
//! L1012 forbids ("a value of the same type as that implied by the preceding
//! part of the constraint"), and the parser refuses them loudly rather than
//! binding the assumed instance to an arbitrary alternative. Both the accepted
//! and the refused shapes are pinned, so neither a newly-lenient nor a
//! newly-strict parser passes silently.

use std::path::{Path, PathBuf};

use openehr_adl::assemble::parse_artefact;
use openehr_adl::error::SyntaxErrorCode;
use openehr_adl::parse::Dialect;

/// What the ADL 1.4 parser must do with a knowledge-resource archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Parses in the 1.4 dialect.
    Parses,
    /// Refused at parse with this syntax code.
    Refuse(SyntaxErrorCode),
}

/// Every `.adl` resource in the knowledge tree, with its expected outcome.
const ARCHETYPES: &[(&str, Expect)] = &[
    // A COMPOSITION with an archetype slot; the `service_definition` suite's
    // upload fixture.
    (
        "openEHR-EHR-COMPOSITION.prescription.v1.adl",
        Expect::Parses,
    ),
    // An INSTRUCTION with an activity slot and an ISO8601 duration constraint.
    ("openEHR-EHR-INSTRUCTION.medication.v1.adl", Expect::Parses),
    // The two `ac`-code + `constraint_definitions` artefacts, each with 18
    // inline `C_DV_QUANTITY` domain blocks. Both state
    // `assumed_value.magnitude = 0.0` against a `magnitude` constrained to
    // `|>0.0|` — an assumed value outside the constraint's own value space,
    // which `ADL1.4/master05-cadl.adoc` §Assumed Values L1012 does not admit.
    (
        "openEHR-EHR-ITEM_TREE.medication.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sdinv),
    ),
    (
        "openEHR-EHR-ITEM_TREE.medication_mod.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sdinv),
    ),
    // The master08 §Revision History Section example (shared with the
    // `adl14-dadl` breadth tree).
    (
        "openEHR-EHR-OBSERVATION.revision_history.v1.adl",
        Expect::Parses,
    ),
    // Two SECTIONs whose slots carry `include`/`exclude` assertion pairs.
    ("openEHR-EHR-SECTION.medication.v1.adl", Expect::Parses),
    ("openEHR-EHR-SECTION.medications.v1.adl", Expect::Parses),
];

fn tree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fixtures/service/knowledge/archetypes")
}

#[test]
fn every_knowledge_archetype_meets_its_declared_parse_outcome() {
    for (name, expect) in ARCHETYPES {
        let path = tree().join(name);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        match expect {
            Expect::Parses => {
                parse_artefact(&src, Dialect::Adl14)
                    .unwrap_or_else(|e| panic!("{name} must parse as ADL 1.4, got {e:?}"));
            }
            Expect::Refuse(code) => {
                let errs = parse_artefact(&src, Dialect::Adl14)
                    .err()
                    .unwrap_or_else(|| panic!("{name} must be refused at parse"));
                assert!(
                    errs.iter().any(|e| e.code == *code),
                    "{name}: expected {code}, got {:?}",
                    errs.iter().map(|e| e.code).collect::<Vec<_>>()
                );
            }
        }
    }
}

/// No `.adl` resource may sit unexercised: a new or renamed knowledge archetype
/// breaks this gate rather than silently escaping the parser.
#[test]
fn every_knowledge_archetype_file_is_in_the_table() {
    let mut on_disk: Vec<String> = std::fs::read_dir(tree())
        .expect("read the knowledge archetype tree")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| Path::new(n).extension().is_some_and(|e| e == "adl"))
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = ARCHETYPES.iter().map(|(n, _)| (*n).to_owned()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "the archetype table and the tree disagree");
}
