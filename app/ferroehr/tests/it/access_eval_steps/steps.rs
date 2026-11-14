// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One table per step of the decision procedure and per refusal code:
//! steps 1 to 3, which read the subject's directives.

use openehr_rm::draft::access_control::terminology::{CallerKind, OperationClass};
use serde_json::{Value, json};

use super::*;
use ferroehr::access_eval::procedure::{Emergency, Lifted};

/// The subject reads their own record whatever the directives say (EHDS Art. 3).
#[test]
fn the_subject_skips_every_step() {
    let deny_all = directive("consent", "deny", &json!({}));
    let case = Case {
        policy: Some(policy(
            &json!({ "care_relationship_required": true, "purpose_required": true }),
        )),
        ..Case::of(settings("consent_required", &[deny_all], &[], &[]))
    };
    let subject = caller(CallerKind::Subject, NURSE);
    let verdict = case.verdict(&subject, read(None));
    assert_outcome(&verdict, &Item::Ehr, Outcome::Admit, "subject");
}

/// A subject caller is the subject of the EHR its patient context names
/// alone: on any other EHR every item is refused, under every disposition,
/// whatever the EHR's directives permit (#5586).
#[test]
fn the_subject_is_refused_on_an_ehr_not_their_own() {
    let permit_all = directive("consent", "permit", &json!({}));
    let subject = caller(CallerKind::Subject, NURSE);
    for default in ["open", "restricted", "consent_required"] {
        for case in [
            Case {
                foreign: true,
                ..Case::of(settings(
                    default,
                    std::slice::from_ref(&permit_all),
                    &[],
                    &[],
                ))
            },
            Case {
                foreign: true,
                server_default: Some(AccessDefault::Open),
                ..Case::default()
            },
        ] {
            let verdict = case.verdict(&subject, read(None));
            for item in [Item::Ehr, object(OBJECT, Some("vital_signs.v1"))] {
                assert_outcome(
                    &verdict,
                    &item,
                    Outcome::Refused(Refusal::NotSubject),
                    &format!("a subject on another EHR ({default})"),
                );
            }
            assert!(!verdict.grounds.subject, "{default}: {verdict:?}");
        }
    }
}

/// Step 1: an opt-out binds the access services only, reads as absent, and
/// yields to a declared emergency only where its override holds.
#[test]
fn step_1_opt_out() {
    let opt_out = |overridable: bool| {
        directive(
            "opt_out",
            "deny",
            &json!({ "emergency_override": overridable }),
        )
    };
    let service = caller(CallerKind::AccessService, DR_B);
    let emergency = Asked {
        emergency: Emergency {
            declared: true,
            break_glass: false,
            opt_out_override: true,
        },
        ..read(Some("ETREAT"))
    };
    let cases: [(&str, bool, &Caller, Asked<'_>, Outcome); 5] = [
        (
            "an access service",
            true,
            &service,
            read(None),
            ABSENT_OPT_OUT,
        ),
        (
            "a professional",
            true,
            &professional(DR_B),
            read(None),
            Outcome::Admit,
        ),
        ("the override", true, &service, emergency, Outcome::Admit),
        (
            "an absolute opt-out",
            false,
            &service,
            emergency,
            ABSENT_OPT_OUT,
        ),
        (
            "an emergency without the override",
            true,
            &service,
            Asked {
                emergency: Emergency {
                    opt_out_override: false,
                    ..emergency.emergency
                },
                ..emergency
            },
            ABSENT_OPT_OUT,
        ),
    ];
    for (name, overridable, who, asked, expected) in cases {
        let case = Case::of(settings("open", &[opt_out(overridable)], &[], &[]));
        let verdict = case.verdict(who, asked);
        assert_outcome(&verdict, &Item::Ehr, expected, name);
        assert_outcome(&verdict, &object(OBJECT, None), expected, name);
        if name == "the override" {
            assert_eq!(verdict.lifted, vec![Lifted::OptOut], "{name}");
        }
    }
}

/// Step 2: a restriction reads as absent to professionals, systems and the
/// access services, over its data scope, less its exceptions, while in force,
/// and yields to the break-glass role only where its override holds.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one row per case of the step's table"
)]
fn step_2_restriction() {
    let template = |extra: Value| {
        let mut d = directive(
            "restriction",
            "deny",
            &json!({ "data": [{ "templates": ["psych_note.v1"] }] }),
        );
        if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
            d.extend(extra.clone());
        }
        d
    };
    let psych = object(OBJECT, Some("psych_note.v1"));
    let lab = object(OTHER, Some("lab_hiv.v1"));
    let except_c = json!({ "exceptions": [directive("consent", "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", DR_C) }] }))] });
    let rows: Vec<(&str, Value, Caller, Asked<'_>, Outcome, Outcome)> = vec![
        (
            "a professional",
            json!({}),
            professional(NURSE),
            read(None),
            ABSENT_RESTRICTED,
            Outcome::Admit,
        ),
        (
            "a system",
            json!({}),
            caller(CallerKind::System, NURSE),
            read(None),
            ABSENT_RESTRICTED,
            Outcome::Admit,
        ),
        (
            "an access service",
            json!({}),
            caller(CallerKind::AccessService, NURSE),
            read(None),
            ABSENT_RESTRICTED,
            Outcome::Admit,
        ),
        (
            "a representative",
            json!({}),
            caller(CallerKind::Representative, GUARDIAN),
            read(None),
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "the exception",
            except_c.clone(),
            professional(DR_C),
            read(None),
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "not the exception",
            except_c,
            professional(NURSE),
            read(None),
            ABSENT_RESTRICTED,
            Outcome::Admit,
        ),
        (
            "ended",
            json!({ "time_validity": interval(None, Some("2026-11-01T00:00:00Z")) }),
            professional(NURSE),
            read(None),
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "not begun",
            json!({ "time_validity": interval(Some("2026-12-01T00:00:00Z"), None) }),
            professional(NURSE),
            read(None),
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "an unreadable end",
            json!({ "time_validity": interval(None, Some("2026-11-01T00:00:00")) }),
            professional(NURSE),
            read(None),
            ABSENT_RESTRICTED,
            Outcome::Admit,
        ),
        (
            "another operation",
            json!({ "operations": [term("write")] }),
            professional(NURSE),
            read(None),
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "the operation",
            json!({ "operations": [term("read")] }),
            professional(NURSE),
            read(None),
            ABSENT_RESTRICTED,
            Outcome::Admit,
        ),
        (
            "lifted",
            json!({ "emergency_override": true }),
            professional(NURSE),
            break_glass(read(Some("ETREAT"))),
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "absolute",
            json!({ "emergency_override": false }),
            professional(NURSE),
            break_glass(read(Some("ETREAT"))),
            ABSENT_RESTRICTED,
            Outcome::Admit,
        ),
    ];
    for (name, extra, who, asked, on_psych, on_lab) in rows {
        let case = Case::of(settings("open", &[template(extra)], &[], &[]));
        let verdict = case.verdict(&who, asked);
        assert_outcome(&verdict, &psych, on_psych, name);
        assert_outcome(&verdict, &lab, on_lab, name);
        assert_outcome(&verdict, &Item::Ehr, Outcome::Admit, name);
    }
    let whole = Case::of(settings(
        "open",
        &[directive("restriction", "deny", &json!({}))],
        &[],
        &[],
    ));
    assert_outcome(
        &whole.verdict(&professional(NURSE), read(None)),
        &Item::Ehr,
        ABSENT_RESTRICTED,
        "a whole-EHR restriction",
    );
}

/// Step 2 over each attribute of a data scope: objects, archetypes,
/// categories, origins and commit time, and the polarity of a fact the
/// object lacks.
#[test]
fn step_2_data_scope_attributes() {
    let restriction = |data: Value| directive("restriction", "deny", &json!({ "data": [data] }));
    let origins = ["lab-system".to_owned()];
    let fed = Item::Object(Object {
        vo_id: uuid(OTHER),
        template: Some("vitals.v1"),
        archetype: Some("openEHR-EHR-COMPOSITION.report.v1"),
        origins: Some(&origins),
        recorded: Some("2026-11-12T08:00:00Z".parse().expect("an instant")),
    });
    let unstamped = Item::Object(Object {
        vo_id: uuid(OTHER),
        template: None,
        archetype: None,
        origins: None,
        recorded: None,
    });
    let rows = [
        (
            "objects",
            json!({ "objects": [object_ref(OTHER)] }),
            ABSENT_RESTRICTED,
            ABSENT_RESTRICTED,
        ),
        (
            "archetypes",
            json!({ "archetypes": [{ "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.report.v1" }] }),
            ABSENT_RESTRICTED,
            Outcome::Admit,
        ),
        (
            "categories",
            json!({ "categories": [coded(CATEGORIES, "mental-health")] }),
            Outcome::Admit,
            Outcome::Admit,
        ),
        (
            "origins",
            json!({ "origins": ["lab-system"] }),
            ABSENT_RESTRICTED,
            ABSENT_RESTRICTED,
        ),
        (
            "recorded",
            json!({ "recorded": interval(Some("2026-11-10T00:00:00Z"), None) }),
            ABSENT_RESTRICTED,
            ABSENT_RESTRICTED,
        ),
        (
            "recorded before",
            json!({ "recorded": interval(None, Some("2026-11-10T00:00:00Z")) }),
            Outcome::Admit,
            ABSENT_RESTRICTED,
        ),
    ];
    for (name, data, on_fed, on_unstamped) in rows {
        let case = Case::of(settings("open", &[restriction(data)], &[], &[]));
        let verdict = case.verdict(&professional(NURSE), read(None));
        assert_outcome(&verdict, &fed, on_fed, name);
        assert_outcome(
            &verdict,
            &unstamped,
            on_unstamped,
            &format!("{name}, unstamped"),
        );
    }
    let psych = Item::Object(Object {
        vo_id: uuid(OBJECT),
        template: Some("PSYCH_NOTE.v1"),
        archetype: None,
        origins: Some(&[]),
        recorded: None,
    });
    let case = Case::of(settings(
        "open",
        &[restriction(
            json!({ "categories": [coded(CATEGORIES, "mental-health")] }),
        )],
        &[],
        &[],
    ));
    assert_outcome(
        &case.verdict(&professional(NURSE), read(None)),
        &psych,
        ABSENT_RESTRICTED,
        "a category by its template, case-folded",
    );
}

/// Step 3: a deny directive that applies refuses `refused-consent-denied`,
/// by caller, operation and purpose, less an exception, and a purpose the
/// request does not declare is read as matching a deny.
#[test]
fn step_3_consent_deny() {
    let deny = |extra: Value| directive("consent", "deny", &extra);
    let rows: Vec<(&str, Value, Caller, Asked<'_>, Outcome)> = vec![
        (
            "the party",
            json!({ "applies_to": [{ "party": party("PERSON", DR_B) }] }),
            professional(DR_B),
            read(None),
            DENIED,
        ),
        (
            "another party",
            json!({ "applies_to": [{ "party": party("PERSON", DR_B) }] }),
            professional(NURSE),
            read(None),
            Outcome::Admit,
        ),
        (
            "a group the caller is a member of",
            json!({ "applies_to": [{ "party": party("GROUP", CARDIOLOGY) }] }),
            with_facts(professional(NURSE), &[], &[], &[CARDIOLOGY]),
            read(None),
            DENIED,
        ),
        (
            "a caller kind",
            json!({ "applies_to": [{ "caller_kind": term("access_service") }] }),
            caller(CallerKind::AccessService, NURSE),
            read(None),
            DENIED,
        ),
        (
            "the purpose",
            json!({ "purposes": [coded(PURPOSE, "HRESCH")] }),
            professional(NURSE),
            read(Some("HRESCH")),
            DENIED,
        ),
        (
            "another purpose",
            json!({ "purposes": [coded(PURPOSE, "HRESCH")] }),
            professional(NURSE),
            read(Some("TREAT")),
            Outcome::Admit,
        ),
        (
            "no declared purpose",
            json!({ "purposes": [coded(PURPOSE, "HRESCH")] }),
            professional(NURSE),
            read(None),
            DENIED,
        ),
        (
            "another operation",
            json!({ "operations": [term("export")] }),
            professional(NURSE),
            read(None),
            Outcome::Admit,
        ),
        (
            "an exception",
            json!({ "exceptions": [directive("consent", "permit", &json!({ "applies_to": [{ "role": coded("local", "PSYCHIATRIST") }] }))] }),
            Caller {
                roles: vec!["PSYCHIATRIST".to_owned()],
                ..professional(NURSE)
            },
            read(None),
            Outcome::Admit,
        ),
        (
            "a representative",
            json!({}),
            caller(CallerKind::Representative, GUARDIAN),
            read(None),
            DENIED,
        ),
        (
            "lifted",
            json!({ "emergency_override": true }),
            professional(NURSE),
            break_glass(read(Some("ETREAT"))),
            Outcome::Admit,
        ),
        (
            "absolute",
            json!({}),
            professional(NURSE),
            break_glass(read(Some("ETREAT"))),
            DENIED,
        ),
    ];
    for (name, extra, who, asked, expected) in rows {
        let case = Case::of(settings("open", &[deny(extra)], &[], &[]));
        assert_outcome(&case.verdict(&who, asked), &Item::Ehr, expected, name);
    }
}

/// Step 3 for a representative: a `delegation` permit naming them limits them
/// to its scope; one naming somebody else does not bind them.
#[test]
fn step_3_delegation_scope() {
    let delegation = |who: &str| {
        directive(
            "delegation",
            "permit",
            &json!({ "applies_to": [{ "party": party("PERSON", who) }],
                     "operations": [term("read")] }),
        )
    };
    let guardian = caller(CallerKind::Representative, GUARDIAN);
    let rows = [
        (
            "a read in scope",
            GUARDIAN,
            OperationClass::Read,
            Outcome::Admit,
        ),
        (
            "a write out of scope",
            GUARDIAN,
            OperationClass::Write,
            DENIED,
        ),
        (
            "another representative's delegation",
            NURSE,
            OperationClass::Write,
            Outcome::Admit,
        ),
    ];
    for (name, named, operation, expected) in rows {
        let case = Case::of(settings("consent_required", &[delegation(named)], &[], &[]));
        assert_outcome(
            &case.verdict(&guardian, asked(operation, None)),
            &Item::Ehr,
            expected,
            name,
        );
    }
    let deny = directive("delegation", "deny", &json!({}));
    let case = Case::of(settings("open", &[deny], &[], &[]));
    assert_outcome(
        &case.verdict(&guardian, read(None)),
        &Item::Ehr,
        DENIED,
        "a delegation deny",
    );
    assert_outcome(
        &case.verdict(&professional(NURSE), read(None)),
        &Item::Ehr,
        Outcome::Admit,
        "a delegation binds no professional",
    );
}
