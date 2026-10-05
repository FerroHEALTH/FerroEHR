// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM resource-package meta-data pass over the vendored OPT corpora.
//!
//! Enforcement of the RM common ch.8 invariants at the template seam
//! (`docs/specs/openehr/RM/docs/UML/classes/
//! org.openehr.rm.common.authored_resource.adoc` and siblings §Invariants)
//! was audited clean against the whole vendored corpus first, so it newly
//! rejects no previously-accepted artefact — this test IS that record, and
//! keeps it true as the corpora grow.

use ferroehr::service::error::ServiceError;
use ferroehr::validation::validate_opt_artefact;

/// The invariant-named codes the resource-meta pass can report.
const RESOURCE_CODES: &[&str] = &[
    "AUTHORED_RESOURCE.Original_language_valid",
    "AUTHORED_RESOURCE.Revision_history_valid",
    "AUTHORED_RESOURCE.Translations_valid",
    "AUTHORED_RESOURCE.Description_valid",
    "TRANSLATION_DETAILS.Language_valid",
    "RESOURCE_DESCRIPTION.Original_author_valid",
    "RESOURCE_DESCRIPTION.Lifecycle_state_valid",
    "RESOURCE_DESCRIPTION.Details_valid",
    "RESOURCE_DESCRIPTION_ITEM.Language_valid",
    "RESOURCE_DESCRIPTION_ITEM.Purpose_valid",
    "RESOURCE_DESCRIPTION_ITEM.Use_valid",
    "RESOURCE_DESCRIPTION_ITEM.misuse_valid",
    "RESOURCE_DESCRIPTION_ITEM.copyright_valid",
];

/// Every parseable vendored OPT passes the resource-meta pass.
///
/// A fixture the AOM2 catalogue already refuses on another rule is out of
/// scope here (first-violation ordering); a fixture refused on a RESOURCE
/// code is a sweep failure to adjudicate, never to silence.
#[test]
fn vendored_opt_corpora_pass_the_resource_meta_checks() {
    let corpora = [
        "../../corpus/templates",
        "../../crates/openehr-its/tests/fixtures/sdk",
    ];
    let mut swept = 0usize;
    let mut offenders = Vec::new();
    for dir in corpora {
        for entry in std::fs::read_dir(dir).expect("corpus directory is readable") {
            let path = entry.expect("corpus directory entry is readable").path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("opt") {
                continue;
            }
            let xml = std::fs::read_to_string(&path).expect("corpus OPT is readable");
            let Ok(opt) = openehr_its::opt14::from_xml(&xml) else {
                // A deliberately-unparseable fixture is the reader gates'
                // territory, not this pass's.
                continue;
            };
            swept += 1;
            if let Err(ServiceError::ValidationFailed(violations)) = validate_opt_artefact(&opt)
                && let Some(v) = violations
                    .first()
                    .filter(|v| RESOURCE_CODES.contains(&v.path.as_str()))
            {
                offenders.push(format!("{}: {} — {}", path.display(), v.path, v.message));
            }
        }
    }
    assert!(
        swept > 50,
        "the sweep walked only {swept} OPTs — corpus path drift?"
    );
    assert!(
        offenders.is_empty(),
        "vendored OPTs refused by the resource-meta pass (adjudicate, never silence):\n{}",
        offenders.join("\n")
    );
}

/// Each ch.8 refusal fixture refuses with its invariant-named code.
///
/// The corpus manifest's `defect`/`spec_ref` rows are the adjudication; this
/// gate pins that the refusal actually happens and names the right invariant
/// (a lenient server, or one refusing under a different rule, fails here).
#[test]
fn ch8_refusal_fixtures_refuse_with_their_invariant() {
    let dir = "../../corpus/fixtures/opt/invalid";
    let expect = [
        (
            "description_empty_lifecycle_state.opt",
            "RESOURCE_DESCRIPTION.Lifecycle_state_valid",
        ),
        (
            "description_empty_purpose.opt",
            "RESOURCE_DESCRIPTION_ITEM.Purpose_valid",
        ),
        (
            "description_item_language_not_in_code_set.opt",
            "RESOURCE_DESCRIPTION_ITEM.Language_valid",
        ),
        (
            "template_language_not_in_code_set.opt",
            "AUTHORED_RESOURCE.Original_language_valid",
        ),
        (
            "controlled_without_revision_history.opt",
            "AUTHORED_RESOURCE.Revision_history_valid",
        ),
        (
            "description_duplicate_detail_language.opt",
            "RESOURCE_DESCRIPTION.Details_valid",
        ),
        (
            "description_without_original_author.opt",
            "RESOURCE_DESCRIPTION.Original_author_valid",
        ),
        (
            "description_without_details.opt",
            "RESOURCE_DESCRIPTION.Details_valid",
        ),
    ];
    for (file, invariant) in expect {
        let xml = std::fs::read_to_string(format!("{dir}/{file}")).expect("fixture is readable");
        let opt = openehr_its::opt14::from_xml(&xml).expect("fixture parses");
        let Err(ServiceError::ValidationFailed(violations)) = validate_opt_artefact(&opt) else {
            panic!("{file}: expected a validation refusal");
        };
        assert_eq!(
            violations.first().map(|v| v.path.as_str()),
            Some(invariant),
            "{file}: wrong refusal code"
        );
    }
}
