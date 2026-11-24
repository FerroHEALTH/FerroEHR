// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Directives, controller rules, classifications and delegations that name
//! an episode's data (`ACCESS_DATA_SCOPE.episodes`, draft access control,
//! Episodes of Care, #6416). A restriction of one episode is a restriction of
//! "parts" of the record (EHDS Art 8); every scope compares the episodes the
//! datum's current version is placed in by issuer and id.

use openehr_rm::draft::access_control::terminology::{CallerKind, OperationClass, Refusal};
use serde_json::json;

use super::*;

/// The confidentiality code system.
const LEVELS: &str = "HL7_Confidentiality";

/// The items each test reads: the EHR, an unplaced object, one in each
/// episode and one in both.
fn items(both: &[EpisodeKey]) -> [(&'static str, Item<'_>); 5] {
    [
        ("the EHR", Item::Ehr),
        ("unplaced", object(OBJECT, Some("vitals.v1"))),
        (
            "cardiology",
            placed(OBJECT, Some("vitals.v1"), &CARDIOLOGY_EPISODE[..]),
        ),
        (
            "dermatology",
            placed(OTHER, Some("vitals.v1"), &DERMATOLOGY_EPISODE[..]),
        ),
        ("both", placed(OTHER, Some("vitals.v1"), both)),
    ]
}

/// Both episodes, sorted as the head row keeps them.
fn both() -> Vec<EpisodeKey> {
    let mut both = vec![CARDIOLOGY_EPISODE[0], DERMATOLOGY_EPISODE[0]];
    both.sort_unstable();
    both
}

/// A patient's restriction of the dermatology episode hides that episode's
/// data, a datum in it and another episode included, and nothing else.
#[test]
fn a_restriction_of_one_episode_hides_its_data_alone() {
    let restriction = directive(
        "restriction",
        "deny",
        &json!({ "data": [episodes(&["D-4"])] }),
    );
    let case = Case::of(settings("open", &[restriction], &[], &[]));
    let verdict = case.verdict(&professional(NURSE), read(None));
    let both = both();
    let expected = [
        Outcome::Admit,
        Outcome::Admit,
        Outcome::Admit,
        ABSENT_RESTRICTED,
        ABSENT_RESTRICTED,
    ];
    for ((name, item), outcome) in items(&both).iter().zip(expected) {
        assert_outcome(&verdict, item, outcome, name);
    }
}

/// An episode named with no issuer names none: a restriction reads it as
/// covering every object, a permit as covering none.
#[test]
fn an_episode_without_an_issuer_is_read_by_polarity() {
    let unnamed = json!({ "episodes": [{ "_type": "DV_IDENTIFIER", "id": "D-4" }] });
    let restriction = directive("restriction", "deny", &json!({ "data": [unnamed] }));
    let case = Case::of(settings("open", &[restriction], &[], &[]));
    let verdict = case.verdict(&professional(NURSE), read(None));
    let unplaced = object(OBJECT, Some("vitals.v1"));
    assert_outcome(&verdict, &unplaced, ABSENT_RESTRICTED, "a deny covers all");
    let consent = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", NURSE) }], "data": [unnamed] }),
    );
    let case = Case::of(settings("consent_required", &[consent], &[], &[]));
    let verdict = case.verdict(&professional(NURSE), read(None));
    let derm = placed(OTHER, Some("vitals.v1"), &DERMATOLOGY_EPISODE[..]);
    assert_outcome(
        &verdict,
        &derm,
        Outcome::Refused(Refusal::ConsentRequired),
        "a permit covers none",
    );
}

/// A delegation naming the cardiology episode limits the representative to
/// that episode's data; data outside it are refused as the delegation's
/// scope refuses them.
#[test]
fn a_delegation_limits_the_representative_to_its_episode() {
    let delegation = directive(
        "delegation",
        "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", GUARDIAN) }],
                 "data": [episodes(&["C-17"])] }),
    );
    let case = Case::of(settings("open", &[delegation], &[], &[]));
    let guardian = caller(CallerKind::Representative, GUARDIAN);
    let verdict = case.verdict(&guardian, read(None));
    let both = both();
    let denied = Outcome::Refused(Refusal::ConsentDenied);
    let expected = [denied, denied, Outcome::Admit, denied, Outcome::Admit];
    for ((name, item), outcome) in items(&both).iter().zip(expected) {
        assert_outcome(&verdict, item, outcome, name);
    }
}

/// A controller rule naming the dermatology episode binds only that
/// episode's data.
#[test]
fn a_controller_rule_naming_an_episode_applies_to_its_data_alone() {
    let ruling = rule(
        "deny",
        &json!({ "applies_to": [role_scope("NURSE")], "operations": [term("write")],
                 "data": [episodes(&["D-4"])] }),
    );
    let case = Case {
        policy: Some(policy(&json!({ "rules": [ruling] }))),
        ..Case::default()
    };
    let nurse = performing(CallerKind::Professional, NURSE, "NURSE");
    let verdict = case.verdict(&nurse, asked(OperationClass::Write, None));
    let both = both();
    let denied = Outcome::Refused(Refusal::PolicyDenied);
    let expected = [
        Outcome::Admit,
        Outcome::Admit,
        Outcome::Admit,
        denied,
        denied,
    ];
    for ((name, item), outcome) in items(&both).iter().zip(expected) {
        assert_outcome(&verdict, item, outcome, name);
    }
}

/// A classification naming the dermatology episode marks that episode's data
/// restricted, and nothing else.
#[test]
fn a_classification_naming_an_episode_marks_its_data_alone() {
    let classified = policy(&json!({ "classifications": [{
        "_type": "ACCESS_CLASSIFICATION",
        "name": { "_type": "DV_TEXT", "value": "the dermatology episode" },
        "archetype_node_id": "at0002",
        "data": { "_type": "ACCESS_DATA_SCOPE", "episodes": [{ "_type": "DV_IDENTIFIER",
            "issuer": HOSPITAL, "id": "D-4" }] },
        "confidentiality": coded(LEVELS, "R") }] }));
    let case = Case {
        policy: Some(classified),
        ..Case::of(settings("open", &[], &[], &[]))
    };
    let verdict = case.verdict(&professional(NURSE), read(None));
    let both = both();
    let refused = Outcome::Refused(Refusal::Confidentiality);
    let expected = [
        Outcome::Admit,
        Outcome::Admit,
        Outcome::Admit,
        refused,
        refused,
    ];
    for ((name, item), outcome) in items(&both).iter().zip(expected) {
        assert_outcome(&verdict, item, outcome, name);
    }
}
