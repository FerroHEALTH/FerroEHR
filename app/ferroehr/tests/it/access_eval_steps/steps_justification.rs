// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Justified access (#6587): a justification of the controller's policy
//! stands in for a care relationship at steps 7 and 8 when the request states
//! a reason, and is refused `refused-reason-missing` without one; an
//! emergency justification lifts what an emergency lifts and needs a reason
//! before any directive is read; a patient's restriction still refuses (EHDS
//! Art. 8, GDPR Art. 18). Our own design over the draft decision procedure.

use openehr_rm::draft::access_control::terminology::{CallerKind, Refusal};
use serde_json::{Value, json};

use super::*;
use ferroehr::access_eval::procedure::Asked;

/// The refusal of a datum no care relationship covers.
const NO_CARE: Outcome = Outcome::Refused(Refusal::NoCareRelationship);
/// The refusal of a justified access that states no reason.
const NO_REASON: Outcome = Outcome::Refused(Refusal::ReasonMissing);

/// A justification for `purposes`, an emergency one when `emergency`,
/// invoked by the callers `applies_to` names, or by every professional.
fn justification(purposes: &[&str], emergency: bool, applies_to: Option<Value>) -> Value {
    let mut j = json!({ "_type": "ACCESS_JUSTIFICATION", "emergency": emergency,
        "purposes": purposes.iter().map(|code| coded(PURPOSE, code)).collect::<Vec<_>>() });
    if let Some(scopes) = applies_to {
        j["applies_to"] = scopes;
    }
    j
}

/// A case under a policy that requires a care relationship when `care` and
/// holds `justifications`, of `settings`.
fn under(care: bool, justifications: &[Value], settings: EhrAccessSettings) -> Case {
    Case {
        settings: Some(settings),
        policy: Some(policy(&json!({
            "care_relationship_required": care,
            "justifications": justifications,
        }))),
        ..Case::default()
    }
}

/// The policy of the hospital: a consultation or second opinion under
/// `TREAT`, and an emergency under `ETREAT`.
fn hospital() -> Vec<Value> {
    vec![
        justification(&["TREAT"], false, None),
        justification(&["ETREAT"], true, None),
    ]
}

/// `asked`, stating a reason.
fn reasoned(asked: Asked<'_>) -> Asked<'_> {
    Asked {
        reasoned: true,
        ..asked
    }
}

/// Outside its care relationships, a professional invoking a justification
/// is admitted with a reason and refused without one; one that invokes none
/// is refused for the relationship, reason or not.
#[test]
fn a_justification_stands_in_for_the_care_relationship_with_a_reason() {
    let case = under(true, &hospital(), settings("open", &[], &[], &[]));
    let nurse = professional(NURSE);
    for (name, asked, expected) in [
        ("no purpose", read(None), NO_CARE),
        ("no purpose, a reason", reasoned(read(None)), NO_CARE),
        ("a justification, no reason", read(Some("TREAT")), NO_REASON),
        (
            "a justification, a reason",
            reasoned(read(Some("TREAT"))),
            Outcome::Admit,
        ),
        (
            "a purpose no justification holds, a reason",
            reasoned(read(Some("HOPERAT"))),
            NO_CARE,
        ),
    ] {
        let verdict = case.verdict(&nurse, asked);
        assert_outcome(&verdict, &Item::Ehr, expected, name);
        assert_outcome(&verdict, &object(OBJECT, None), expected, name);
    }
    let verdict = case.verdict(&nurse, reasoned(read(Some("TREAT"))));
    let justified = verdict
        .grounds
        .justification
        .as_ref()
        .expect("the verdict rests on the justification");
    assert_eq!((justified.at, justified.emergency), (0, false));
    assert!(verdict.lifted.is_empty(), "a justification lifts nothing");
}

/// Inside its care relationships no reason is asked and no justification is
/// named: the access rests on the relationship.
#[test]
fn inside_the_care_relationships_no_reason_is_asked() {
    let case = Case {
        cared: true,
        ..under(true, &hospital(), settings("open", &[], &[], &[]))
    };
    let verdict = case.verdict(&professional(NURSE), read(Some("TREAT")));
    assert_outcome(&verdict, &object(OBJECT, None), Outcome::Admit, "cared");
    assert_eq!(verdict.grounds.justification, None);
    assert!(verdict.grounds.cared);
}

/// Under a `restricted` default, a justification stands in for the
/// relationship the default asks for, the same way.
#[test]
fn a_restricted_default_admits_a_justified_access_with_a_reason() {
    let case = under(false, &hospital(), settings("restricted", &[], &[], &[]));
    let nurse = professional(NURSE);
    assert_outcome(
        &case.verdict(&nurse, read(Some("TREAT"))),
        &object(OBJECT, None),
        NO_REASON,
        "no reason",
    );
    assert_outcome(
        &case.verdict(&nurse, reasoned(read(Some("TREAT")))),
        &object(OBJECT, None),
        Outcome::Admit,
        "a reason",
    );
    assert_outcome(
        &case.verdict(&nurse, read(None)),
        &object(OBJECT, None),
        Outcome::Refused(Refusal::ConsentRequired),
        "no justification",
    );
}

/// A justification lifts no restriction of the patient (EHDS Art. 8): the
/// data it covers stay absent, whatever the reason; the rest is admitted.
#[test]
fn a_patients_restriction_refuses_a_justified_access() {
    let restriction = directive(
        "restriction",
        "deny",
        &json!({ "emergency_override": true,
                 "data": [{ "_type": "ACCESS_DATA_SCOPE", "objects": [object_ref(OBJECT)] }] }),
    );
    let case = under(
        true,
        &hospital(),
        settings("open", &[restriction], &[], &[]),
    );
    let verdict = case.verdict(&professional(NURSE), reasoned(read(Some("TREAT"))));
    assert_outcome(
        &verdict,
        &object(OBJECT, None),
        ABSENT_RESTRICTED,
        "the restricted object",
    );
    assert_outcome(
        &verdict,
        &object(OTHER, None),
        Outcome::Admit,
        "another object",
    );
}

/// An emergency is a justification that needs a reason before any directive
/// is read, and with one lifts a restriction whose override holds and steps
/// 6 to 8; a restriction without the override still holds.
#[test]
fn an_emergency_needs_a_reason_and_lifts_an_overridable_restriction() {
    let restriction = |override_holds: bool| {
        directive(
            "restriction",
            "deny",
            &json!({ "emergency_override": override_holds,
                     "data": [{ "_type": "ACCESS_DATA_SCOPE", "objects": [object_ref(OBJECT)] }] }),
        )
    };
    let nurse = professional(NURSE);
    let overridable = under(
        true,
        &hospital(),
        settings("open", &[restriction(true)], &[], &[]),
    );
    let unreasoned = overridable.verdict(&nurse, read(Some("ETREAT")));
    assert_outcome(
        &unreasoned,
        &object(OBJECT, None),
        ABSENT_RESTRICTED,
        "restricted, no reason",
    );
    assert_outcome(
        &unreasoned,
        &object(OTHER, None),
        NO_REASON,
        "the rest, no reason",
    );
    let lifted = overridable.verdict(&nurse, reasoned(read(Some("ETREAT"))));
    assert_outcome(&lifted, &object(OBJECT, None), Outcome::Admit, "lifted");
    assert!(lifted.lifted.contains(&Lifted::Restriction), "{lifted:?}");
    assert!(lifted.lifted.contains(&Lifted::Policy), "{lifted:?}");
    assert_eq!(
        lifted
            .grounds
            .justification
            .as_ref()
            .map(|j| (j.at, j.emergency)),
        Some((1, true))
    );
    let absolute = under(
        true,
        &hospital(),
        settings("open", &[restriction(false)], &[], &[]),
    );
    assert_outcome(
        &absolute.verdict(&nurse, reasoned(read(Some("ETREAT")))),
        &object(OBJECT, None),
        ABSENT_RESTRICTED,
        "an absolute restriction",
    );
}

/// A justification is invoked only by the callers it names: a professional
/// that performs no role it names stays refused for the relationship, and a
/// representative never invokes one.
#[test]
fn a_justification_is_invoked_only_by_the_callers_it_names() {
    let consultants =
        json!([{ "_type": "ACCESS_ACTOR_SCOPE", "role": coded(ROLES, "CONSULTANT") }]);
    let case = under(
        true,
        &[justification(&["TREAT"], false, Some(consultants))],
        settings("open", &[], &[], &[]),
    );
    assert_outcome(
        &case.verdict(&professional(NURSE), reasoned(read(Some("TREAT")))),
        &Item::Ehr,
        NO_CARE,
        "a nurse",
    );
    assert_outcome(
        &case.verdict(
            &performing(CallerKind::Professional, DR_B, "CONSULTANT"),
            reasoned(read(Some("TREAT"))),
        ),
        &Item::Ehr,
        Outcome::Admit,
        "a consultant",
    );
    let representative = Case {
        foreign: false,
        ..under(true, &hospital(), settings("open", &[], &[], &[]))
    };
    let verdict = representative.verdict(
        &caller(CallerKind::Representative, DR_B),
        reasoned(read(Some("ETREAT"))),
    );
    assert_eq!(verdict.grounds.justification, None, "{verdict:?}");
    assert!(verdict.lifted.is_empty(), "{verdict:?}");
}
