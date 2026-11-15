// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Step 4 of the decision procedure: the controller's rules
//! (`ACCESS_POLICY.rules`, EHDS Art. 11(3)), which bind every caller but the
//! subject, by the roles it performs in the demographic service, and which
//! no permit of the subject lifts.

use openehr_rm::draft::access_control::terminology::{CallerKind, OperationClass, Refusal};
use serde_json::json;

use super::*;

/// A deny rule refuses.
const POLICY_DENIED: Outcome = Outcome::Refused(Refusal::PolicyDenied);

/// The nurse rule: the nurse role may not write the diagnosis template.
fn nurse_rule() -> Value {
    rule(
        "deny",
        &json!({ "applies_to": [role_scope("NURSE")], "operations": [term("write")],
                 "data": [{ "templates": ["diagnosis.v1"] }] }),
    )
}

/// A case under a policy of `rules`, over `settings`.
fn ruled(rules: &[Value], settings: Option<EhrAccessSettings>) -> Case {
    Case {
        settings,
        policy: Some(policy(&json!({ "rules": rules }))),
        ..Case::default()
    }
}

/// A nurse may not commit the diagnosis template on any EHR, whatever the
/// subject's directives permit, and every other role and operation is
/// untouched.
#[test]
fn step_4_a_deny_rule_refuses_by_the_role_the_caller_performs() {
    let patient_permit = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", NURSE) }] }),
    );
    let diagnosis = object(OBJECT, Some("diagnosis.v1"));
    let vitals = object(OTHER, Some("vitals.v1"));
    let write = asked(OperationClass::Write, None);
    let nurse = performing(CallerKind::Professional, NURSE, "NURSE");
    let rows: Vec<(&str, Case, Caller, Asked<'_>, Outcome, Outcome)> = vec![
        (
            "the nurse writing",
            ruled(&[nurse_rule()], None),
            nurse.clone(),
            write,
            POLICY_DENIED,
            Outcome::Admit,
        ),
        (
            "a patient permit does not lift it",
            ruled(
                &[nurse_rule()],
                Some(settings("open", &[patient_permit], &[], &[])),
            ),
            nurse.clone(),
            write,
            POLICY_DENIED,
            Outcome::Admit,
        ),
        (
            "the nurse reading",
            ruled(&[nurse_rule()], None),
            nurse.clone(),
            read(None),
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "a physician writing",
            ruled(&[nurse_rule()], None),
            performing(CallerKind::Professional, DR_C, "PHYSICIAN"),
            write,
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "a nurse role of another terminology",
            ruled(&[nurse_rule()], None),
            with_facts(professional(NURSE), &[code("local", "NURSE")], &[]),
            write,
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "the subject writing their own record",
            ruled(&[nurse_rule()], None),
            with_facts(
                caller(CallerKind::Subject, NURSE),
                &[code(ROLES, "NURSE")],
                &[],
            ),
            write,
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "a representative performing the role",
            ruled(&[nurse_rule()], None),
            performing(CallerKind::Representative, GUARDIAN, "NURSE"),
            write,
            POLICY_DENIED,
            Outcome::Admit,
        ),
    ];
    for (name, case, who, ask, on_diagnosis, on_vitals) in rows {
        let verdict = case.verdict(&who, ask);
        assert_outcome(&verdict, &diagnosis, on_diagnosis, name);
        assert_outcome(&verdict, &vitals, on_vitals, name);
    }
}

/// A deny rule with a permit exception states "only": a system client's
/// `AGENT` writes no template but the ones its exception lists, the
/// equivalent of a template list on its grant (#5624).
#[test]
fn step_4_a_permit_exception_states_only() {
    let only_lab = rule(
        "deny",
        &json!({ "applies_to": [{ "party": party("AGENT", DR_B) }],
                 "operations": [term("write")],
                 "exceptions": [rule("permit", &json!({ "data": [{ "templates": ["lab_hiv.v1"] }] }))] }),
    );
    let case = ruled(&[only_lab], None);
    let client = caller(CallerKind::System, DR_B);
    let write = asked(OperationClass::Write, None);
    let verdict = case.verdict(&client, write);
    assert_outcome(
        &verdict,
        &object(OBJECT, Some("lab_hiv.v1")),
        Outcome::Admit,
        "a listed template",
    );
    assert_outcome(
        &verdict,
        &object(OTHER, Some("psych_note.v1")),
        POLICY_DENIED,
        "any other template",
    );
    let other = case.verdict(&caller(CallerKind::System, DR_C), write);
    assert_outcome(
        &other,
        &object(OTHER, Some("psych_note.v1")),
        Outcome::Admit,
        "another client",
    );
}

/// "Professionals may declare only treatment or emergency treatment": a deny
/// rule over every professional with a permit exception naming the purposes.
#[test]
fn step_4_a_purpose_limit_is_a_rule() {
    let treatment_only = rule(
        "deny",
        &json!({ "applies_to": [{ "caller_kind": term("professional") }],
                 "exceptions": [rule("permit", &json!({
                     "purposes": [coded(PURPOSE, "TREAT"), coded(PURPOSE, "ETREAT")] }))] }),
    );
    let case = ruled(&[treatment_only], None);
    let nurse = professional(NURSE);
    for (name, purpose, expected) in [
        ("treatment", Some("TREAT"), Outcome::Admit),
        ("emergency treatment", Some("ETREAT"), Outcome::Admit),
        ("research", Some("HRESCH"), POLICY_DENIED),
        ("no purpose", None, POLICY_DENIED),
    ] {
        assert_outcome(
            &case.verdict(&nurse, read(purpose)),
            &Item::Ehr,
            expected,
            name,
        );
    }
    assert_outcome(
        &case.verdict(&caller(CallerKind::System, DR_B), read(Some("HRESCH"))),
        &Item::Ehr,
        Outcome::Admit,
        "a system client is no professional",
    );
}

/// A declared emergency by the break-glass role lifts a deny rule only where
/// its `emergency_override` holds, and the verdict marks it.
#[test]
fn step_4_an_emergency_lifts_only_an_overridable_rule() {
    let diagnosis = object(OBJECT, Some("diagnosis.v1"));
    let nurse = performing(CallerKind::Professional, NURSE, "NURSE");
    let emergency = break_glass(asked(OperationClass::Write, Some("ETREAT")));
    let firm = ruled(&[nurse_rule()], None).verdict(&nurse, emergency);
    assert_outcome(&firm, &diagnosis, POLICY_DENIED, "a rule without override");
    let mut overridable = nurse_rule();
    overridable["emergency_override"] = json!(true);
    let lifted = ruled(&[overridable], None).verdict(&nurse, emergency);
    assert_outcome(&lifted, &diagnosis, Outcome::Admit, "an overridable rule");
    assert!(lifted.lifted.contains(&Lifted::Rule), "{lifted:?}");
    assert_eq!(lifted.grounds.lifted_rules, vec![0]);
}

/// A rule's step and its position name the refusal in the verdict.
#[test]
fn step_4_the_refusal_names_the_rule() {
    let verdict = ruled(&[rule("permit", &json!({})), nurse_rule()], None).verdict(
        &performing(CallerKind::Professional, NURSE, "NURSE"),
        asked(OperationClass::Write, None),
    );
    let diagnosis = object(OBJECT, Some("diagnosis.v1"));
    let deciding = verdict.rule_for(&diagnosis).expect("a rule decides");
    assert_eq!(deciding.then, POLICY_DENIED);
    assert_eq!(deciding.by, vec![1], "the position in the policy's rules");
}
