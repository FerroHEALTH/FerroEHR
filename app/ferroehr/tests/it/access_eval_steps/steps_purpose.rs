// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Step 4 of the decision procedure, a purpose the access policy requires,
//! checked before the controller's rules of step 5 are read, and the purpose
//! scope of a rule, which matches only a purpose the request declares
//! (#5737).

use openehr_rm::draft::access_control::terminology::{CallerKind, Refusal};
use serde_json::json;

use super::*;

/// A request without a purpose under a policy that requires one.
const PURPOSE_MISSING: Outcome = Outcome::Refused(Refusal::PurposeMissing);
/// A deny rule refuses.
const POLICY_DENIED: Outcome = Outcome::Refused(Refusal::PolicyDenied);

/// A policy requiring a purpose or not, with `rules`.
fn under(purpose_required: bool, rules: &[Value]) -> Case {
    let mut members = json!({ "purpose_required": purpose_required });
    if !rules.is_empty() {
        members["rules"] = json!(rules);
    }
    Case {
        policy: Some(policy(&members)),
        ..Case::default()
    }
}

/// "No medical examination": a deny rule over one purpose.
fn no_examination() -> Value {
    rule(
        "deny",
        &json!({ "purposes": [coded(PURPOSE, "EXAMINATION")] }),
    )
}

/// "Emergency treatment only by the emergency role": a deny rule over the
/// emergency purpose that a declared emergency by the break-glass role lifts.
fn emergency_by_the_role() -> Value {
    rule(
        "deny",
        &json!({ "purposes": [coded(PURPOSE, "ETREAT")], "emergency_override": true }),
    )
}

/// Where the policy requires one, a declared purpose; a representative is
/// held to steps 1 to 3 and 5 and declares none.
#[test]
fn step_4_purpose() {
    let required = under(true, &[]);
    let nurse = professional(NURSE);
    for (name, purpose, expected) in [
        ("no purpose", None, PURPOSE_MISSING),
        ("a purpose", Some("HOPERAT"), Outcome::Admit),
    ] {
        assert_outcome(
            &required.verdict(&nurse, read(purpose)),
            &Item::Ehr,
            expected,
            name,
        );
    }
    let guardian = caller(CallerKind::Representative, GUARDIAN);
    assert_outcome(
        &required.verdict(&guardian, read(None)),
        &Item::Ehr,
        Outcome::Admit,
        "a representative",
    );
}

/// Every combination of the requirement, the declared purpose and a deny
/// rule over one purpose: a missing purpose is refused by the requirement
/// before any rule, whatever rules exist, and never by a purpose rule; a
/// declared purpose inside the rule's scope is refused by the rule.
#[test]
fn step_4_comes_before_the_rules_and_a_purpose_scope_needs_a_declared_purpose() {
    let nurse = professional(NURSE);
    let rows = [
        (
            "required, no rule, none",
            true,
            false,
            None,
            PURPOSE_MISSING,
        ),
        (
            "required, no rule, inside",
            true,
            false,
            Some("EXAMINATION"),
            Outcome::Admit,
        ),
        (
            "required, no rule, outside",
            true,
            false,
            Some("TREAT"),
            Outcome::Admit,
        ),
        ("required, a rule, none", true, true, None, PURPOSE_MISSING),
        (
            "required, a rule, inside",
            true,
            true,
            Some("EXAMINATION"),
            POLICY_DENIED,
        ),
        (
            "required, a rule, outside",
            true,
            true,
            Some("TREAT"),
            Outcome::Admit,
        ),
        (
            "optional, no rule, none",
            false,
            false,
            None,
            Outcome::Admit,
        ),
        (
            "optional, no rule, inside",
            false,
            false,
            Some("EXAMINATION"),
            Outcome::Admit,
        ),
        (
            "optional, no rule, outside",
            false,
            false,
            Some("TREAT"),
            Outcome::Admit,
        ),
        ("optional, a rule, none", false, true, None, Outcome::Admit),
        (
            "optional, a rule, inside",
            false,
            true,
            Some("EXAMINATION"),
            POLICY_DENIED,
        ),
        (
            "optional, a rule, outside",
            false,
            true,
            Some("TREAT"),
            Outcome::Admit,
        ),
    ];
    for (name, required, ruled, purpose, expected) in rows {
        let rules = if ruled {
            vec![no_examination()]
        } else {
            Vec::new()
        };
        assert_outcome(
            &under(required, &rules).verdict(&nurse, read(purpose)),
            &Item::Ehr,
            expected,
            name,
        );
    }
}

/// The refusal of a missing purpose rests on no rule, even where a rule over
/// the caller's role would also refuse; with a purpose, the rule refuses.
#[test]
fn step_4_refuses_before_a_rule_over_the_role() {
    let case = under(
        true,
        &[rule(
            "deny",
            &json!({ "applies_to": [role_scope("NURSE")] }),
        )],
    );
    let nurse = performing(CallerKind::Professional, NURSE, "NURSE");
    let verdict = case.verdict(&nurse, read(None));
    assert_outcome(&verdict, &Item::Ehr, PURPOSE_MISSING, "no purpose");
    let deciding = verdict.rule_for(&Item::Ehr).expect("a rule decides");
    assert!(deciding.by.is_empty(), "{deciding:?}");
    assert_outcome(
        &case.verdict(&nurse, read(Some("TREAT"))),
        &Item::Ehr,
        POLICY_DENIED,
        "a purpose, then the role rule",
    );
}

/// Every scope but a purpose keeps its reading: a caller the system does not
/// establish in the role a permit exception names stays under the deny.
#[test]
fn a_role_scope_still_covers_a_caller_without_the_fact() {
    let unless_physician = rule(
        "deny",
        &json!({ "exceptions": [rule("permit", &json!({
            "applies_to": [role_scope("PHYSICIAN")] }))] }),
    );
    assert_outcome(
        &under(false, &[unless_physician]).verdict(&professional(NURSE), read(Some("TREAT"))),
        &Item::Ehr,
        POLICY_DENIED,
        "no role performed",
    );
}

/// The break-glass role declaring an emergency lifts the deny rule over the
/// emergency purpose; any other caller declaring it is refused, and one
/// declaring nothing under a required purpose is refused for that.
#[test]
fn break_glass_still_lifts_the_emergency_purpose_rule() {
    let case = under(true, &[emergency_by_the_role()]);
    let doctor = professional(DR_B);
    let lifted = case.verdict(&doctor, break_glass(read(Some("ETREAT"))));
    assert_outcome(&lifted, &Item::Ehr, Outcome::Admit, "the break-glass role");
    assert!(lifted.lifted.contains(&Lifted::Rule), "{lifted:?}");
    for (name, purpose, expected) in [
        (
            "an emergency without the role",
            Some("ETREAT"),
            POLICY_DENIED,
        ),
        ("treatment", Some("TREAT"), Outcome::Admit),
        ("no purpose", None, PURPOSE_MISSING),
    ] {
        assert_outcome(
            &case.verdict(&doctor, read(purpose)),
            &Item::Ehr,
            expected,
            name,
        );
    }
}

/// A subject's deny over a purpose still covers a request that declares
/// none: the subject has no `purpose_required` to set, so absence cannot
/// lift the subject's deny.
#[test]
fn a_subject_purpose_deny_still_covers_a_missing_purpose() {
    let no_research = directive(
        "consent",
        "deny",
        &json!({ "purposes": [coded(PURPOSE, "HRESCH")] }),
    );
    let case = Case::of(settings("open", &[no_research], &[], &[]));
    assert_outcome(
        &case.verdict(&professional(NURSE), read(None)),
        &Item::Ehr,
        DENIED,
        "no declared purpose",
    );
}
