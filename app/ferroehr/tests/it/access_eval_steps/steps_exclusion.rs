// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's exclusion (step 0), a restriction that names a
//! representative (step 2) and the conflict step (step 5) of the draft
//! decision procedure (#6077).

use openehr_rm::draft::access_control::terminology::{CallerKind, Conflict, Refusal};
use serde_json::{Value, json};

use super::*;

/// The refusal of an exclusion.
const EXCLUDED: Outcome = Outcome::Refused(Refusal::Excluded);
/// The refusal of a conflict.
const CONFLICT: Outcome = Outcome::Refused(Refusal::Conflict);

/// A `PARTY_IDENTIFIED` naming the `PERSON` `id`, as a gate-keeper.
fn keeper(id: &str) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "external_ref": party("PERSON", id) })
}

/// Every probe item of the EHR answers `expected`.
fn assert_every(verdict: &Verdict, expected: Outcome, case: &str) {
    for item in [Item::Ehr, object(OBJECT, Some("vital_signs.v1"))] {
        assert_outcome(verdict, &item, expected, case);
    }
}

/// An exclusion refuses the party it names before every other step, a
/// representative and a gate-keeper included, and a permit of the subject
/// does not lift it; it never binds the subject or another caller.
#[test]
fn an_exclusion_binds_the_named_caller_before_step_one() {
    let named = exclusion(&json!({ "party": party("PERSON", DR_B) }), &json!({}));
    let permit = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "party": party("PERSON", DR_B) }] }),
    );
    let case = Case::of(excluding(
        std::slice::from_ref(&named),
        &[permit],
        &[keeper(DR_B)],
    ));
    for (who, caller) in [
        ("a professional", professional(DR_B)),
        ("a representative", caller(CallerKind::Representative, DR_B)),
        ("a system", caller(CallerKind::System, DR_B)),
    ] {
        let verdict = case.verdict(&caller, read(None));
        assert_every(&verdict, EXCLUDED, who);
        assert_eq!(
            verdict.rule_for(&Item::Ehr).map(|rule| rule.by.clone()),
            Some(vec![0]),
            "the refusal rests on the exclusion: {who}"
        );
    }
    assert_every(
        &case.verdict(&professional(NURSE), read(None)),
        Outcome::Admit,
        "another professional",
    );
    assert_every(
        &case.verdict(&caller(CallerKind::Subject, DR_B), read(None)),
        Outcome::Admit,
        "the subject",
    );
}

/// An exclusion that names the representative relationship binds every
/// representative of the subject.
#[test]
fn an_exclusion_binds_a_representative_by_its_relationship() {
    let named = exclusion(
        &json!({ "relationship": term("representative") }),
        &json!({}),
    );
    let case = Case::of(excluding(&[named], &[], &[]));
    assert_every(
        &case.verdict(&caller(CallerKind::Representative, GUARDIAN), read(None)),
        EXCLUDED,
        "a representative",
    );
    assert_every(
        &case.verdict(&professional(DR_B), read(None)),
        Outcome::Admit,
        "a professional",
    );
}

/// An emergency lifts an exclusion only where its `emergency_override`
/// holds and the caller is no representative, and marks the access.
#[test]
fn an_emergency_lifts_an_exclusion_only_where_it_allows_and_never_for_a_representative() {
    let overridable = exclusion(
        &json!({ "party": party("PERSON", DR_B) }),
        &json!({ "emergency_override": true }),
    );
    let case = Case::of(excluding(&[overridable], &[], &[]));
    let lifted = case.verdict(&professional(DR_B), break_glass(read(None)));
    assert_every(&lifted, Outcome::Admit, "a professional in an emergency");
    assert!(lifted.lifted.contains(&Lifted::Exclusion), "{lifted:?}");
    assert_eq!(lifted.grounds.lifted_exclusions.len(), 1);
    let representative = caller(CallerKind::Representative, DR_B);
    assert_every(
        &case.verdict(&representative, break_glass(read(None))),
        EXCLUDED,
        "a representative in an emergency",
    );
    let fixed = Case::of(excluding(
        &[exclusion(
            &json!({ "party": party("PERSON", DR_B) }),
            &json!({}),
        )],
        &[],
        &[],
    ));
    assert_every(
        &fixed.verdict(&professional(DR_B), break_glass(read(None))),
        EXCLUDED,
        "an exclusion with no override",
    );
}

/// A restriction binds a representative its scope names by party or by the
/// representative relationship, answered as absent; one that names no
/// representative leaves a representative to the other steps.
#[test]
fn a_restriction_binds_a_representative_it_names() {
    let representative = caller(CallerKind::Representative, GUARDIAN);
    for scope in [
        json!({ "_type": "ACCESS_ACTOR_SCOPE", "party": party("PERSON", GUARDIAN) }),
        json!({ "_type": "ACCESS_ACTOR_SCOPE", "relationship": term("representative") }),
    ] {
        let restriction = directive("restriction", "deny", &json!({ "applies_to": [scope] }));
        let case = Case::of(settings("open", &[restriction], &[], &[]));
        assert_every(
            &case.verdict(&representative, read(None)),
            ABSENT_RESTRICTED,
            "a named representative",
        );
    }
    let everyone = directive("restriction", "deny", &json!({}));
    let professionals = directive(
        "restriction",
        "deny",
        &json!({ "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                                 "caller_kind": term("representative") }] }),
    );
    for restriction in [everyone, professionals] {
        let case = Case::of(settings("open", &[restriction], &[], &[]));
        assert_every(
            &case.verdict(&representative, read(None)),
            Outcome::Admit,
            "a representative the restriction does not name",
        );
    }
}

/// A professional performing a `ROLE` that reads its own record is refused
/// `refused-conflict` under the default rule; bound with no `ROLE`, or as
/// the subject, it reads it as the record's person.
#[test]
fn staff_reading_their_own_record_as_a_professional_are_in_conflict() {
    let case = Case {
        subject: true,
        ..Case::of(settings("open", &[], &[], &[]))
    };
    let staff = performing(CallerKind::Professional, NURSE, "NURSE");
    assert_every(
        &case.verdict(&staff, read(None)),
        CONFLICT,
        "staff on their own record",
    );
    let person = case.verdict(&professional(NURSE), read(None));
    assert_every(&person, Outcome::Admit, "a bound person with no ROLE");
    assert!(person.grounds.subject);
    let subject = case.verdict(&caller(CallerKind::Subject, NURSE), read(None));
    assert_every(&subject, Outcome::Admit, "the subject path");
    assert!(subject.grounds.subject);
}

/// A professional related to the subject is refused under the default rule,
/// and a conflict rule naming that conflict, the caller and the kinship lets
/// it through, marked; kinship never grants.
#[test]
fn a_professional_related_to_the_subject_is_refused_unless_a_conflict_rule_allows_it() {
    let parent = code("openehr", "254");
    let staff = performing(CallerKind::Professional, NURSE, "NURSE");
    let related = Case {
        kinships: vec![parent.clone()],
        ..Case::of(settings("open", &[], &[], &[]))
    };
    assert_every(&related.verdict(&staff, read(None)), CONFLICT, "a relative");
    let allowing = |kinships: Value, applies_to: Value| Case {
        policy: Some(policy(&json!({ "conflict_rules": [{
            "_type": "ACCESS_CONFLICT_RULE", "conflict": term("related"),
            "kinships": kinships, "applies_to": applies_to }] }))),
        kinships: vec![code("openehr", "254")],
        ..Case::of(settings("open", &[], &[], &[]))
    };
    let nurse = json!([{ "_type": "ACCESS_ACTOR_SCOPE", "party": party("PERSON", NURSE) }]);
    let allowed =
        allowing(json!([coded("openehr", "254")]), nurse.clone()).verdict(&staff, read(None));
    assert_every(&allowed, Outcome::Admit, "an allowed relative");
    assert_eq!(allowed.grounds.conflicts, vec![Conflict::Related]);
    assert_every(
        &allowing(json!([coded("openehr", "26")]), nurse).verdict(&staff, read(None)),
        CONFLICT,
        "a rule of another kinship",
    );
    let doctor = json!([{ "_type": "ACCESS_ACTOR_SCOPE", "party": party("PERSON", DR_B) }]);
    assert_every(
        &allowing(json!([coded("openehr", "254")]), doctor).verdict(&staff, read(None)),
        CONFLICT,
        "a rule naming another caller",
    );
    let restricted = Case {
        kinships: vec![parent],
        policy: Some(policy(&json!({ "conflict_rules": [{
            "_type": "ACCESS_CONFLICT_RULE", "conflict": term("related") }] }))),
        ..Case::of(settings("consent_required", &[], &[], &[]))
    };
    assert_every(
        &restricted.verdict(&staff, read(None)),
        Outcome::Refused(Refusal::ConsentRequired),
        "an allowed relative is admitted by nothing kinship grants",
    );
}

/// A conflict refuses only a professional: a representative, a system and
/// the subject are never in conflict.
#[test]
fn only_a_professional_is_in_conflict() {
    let case = Case {
        kinships: vec![code("openehr", "254")],
        ..Case::of(settings("open", &[], &[], &[]))
    };
    for kind in [CallerKind::System, CallerKind::AccessService] {
        assert_every(
            &case.verdict(&caller(kind, NURSE), read(None)),
            Outcome::Admit,
            "no professional",
        );
    }
}
