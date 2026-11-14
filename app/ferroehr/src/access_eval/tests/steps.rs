// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One table per step of the decision procedure and per refusal code.

use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, OperationClass, Refusal,
};
use serde_json::json;

use super::*;
use crate::access_eval::procedure::{Emergency, Lifted};

const ABSENT_OPT_OUT: Outcome = Outcome::Absent(Refusal::OptOut);
const ABSENT_RESTRICTED: Outcome = Outcome::Absent(Refusal::Restricted);
const DENIED: Outcome = Outcome::Refused(Refusal::ConsentDenied);

/// The break-glass role declaring an emergency.
fn break_glass(asked: Asked<'_>) -> Asked<'_> {
    Asked {
        emergency: Emergency {
            declared: true,
            break_glass: true,
            opt_out_override: false,
        },
        ..asked
    }
}

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
fn step_2_restriction() {
    let template = |extra: serde_json::Value| {
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
    let rows: Vec<(&str, serde_json::Value, Caller, Asked<'_>, Outcome, Outcome)> = vec![
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
    let restriction =
        |data: serde_json::Value| directive("restriction", "deny", &json!({ "data": [data] }));
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
    let deny = |extra: serde_json::Value| directive("consent", "deny", &extra);
    let rows: Vec<(&str, serde_json::Value, Caller, Asked<'_>, Outcome)> = vec![
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

/// Step 4: data whose label or classification the caller is not cleared for
/// are refused `refused-sensitivity`, unless a permit names the sensitivity.
#[test]
fn step_4_sensitivity() {
    let hiv = code(CONFIDENTIALITY, "HIV");
    let label = json!({ "_type": "ACCESS_LABEL", "target": object_ref(OBJECT),
                        "sensitivities": [coded(CONFIDENTIALITY, "HIV")] });
    let classified = policy(&json!({ "classifications": [{
        "_type": "ACCESS_CLASSIFICATION",
        "name": { "_type": "DV_TEXT", "value": "HIV results" },
        "archetype_node_id": "at0002",
        "data": { "_type": "ACCESS_DATA_SCOPE", "templates": ["lab_hiv.v1"] },
        "sensitivities": [coded(CONFIDENTIALITY, "HIV")] }] }));
    let naming = directive(
        "consent",
        "permit",
        &json!({
        "applies_to": [{ "party": party("PERSON", NURSE) }],
        "data": [{ "sensitivities": [coded(CONFIDENTIALITY, "HIV")] }] }),
    );
    let sensitive = Outcome::Refused(Refusal::Sensitivity);
    let labelled = object(OBJECT, None);
    let lab = object(OTHER, Some("lab_hiv.v1"));
    let plain = object(OTHER, Some("vitals.v1"));
    let rows: Vec<(&str, Case, Caller, [Outcome; 3])> = vec![
        (
            "a label",
            Case::of(settings("open", &[], &[label.clone()], &[])),
            professional(NURSE),
            [sensitive, Outcome::Admit, Outcome::Admit],
        ),
        (
            "a classification",
            Case {
                policy: Some(classified.clone()),
                ..Case::of(settings("open", &[], &[], &[]))
            },
            professional(NURSE),
            [Outcome::Admit, sensitive, Outcome::Admit],
        ),
        (
            "a classification without settings",
            Case {
                policy: Some(classified.clone()),
                ..Case::default()
            },
            professional(NURSE),
            [Outcome::Admit, sensitive, Outcome::Admit],
        ),
        (
            "a clearance",
            Case {
                policy: Some(classified),
                ..Case::of(settings("open", &[], &[label.clone()], &[]))
            },
            with_facts(professional(NURSE), &[hiv], &[], &[]),
            [Outcome::Admit; 3],
        ),
        (
            "a permit naming it",
            Case::of(settings("open", &[naming], &[label], &[])),
            professional(NURSE),
            [Outcome::Admit; 3],
        ),
    ];
    for (name, case, who, [on_labelled, on_lab, on_plain]) in rows {
        let verdict = case.verdict(&who, read(None));
        assert_outcome(&verdict, &labelled, on_labelled, name);
        assert_outcome(&verdict, &lab, on_lab, name);
        assert_outcome(&verdict, &plain, on_plain, name);
    }
}

/// Step 5: where the policy requires one, a professional needs a live care
/// relationship, a permit naming them or a gate-keeper entry.
#[test]
fn step_5_care_relationship() {
    let required = policy(&json!({ "care_relationship_required": true }));
    let refused = Outcome::Refused(Refusal::NoCareRelationship);
    let naming = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", NURSE) }] }),
    );
    let keeper = json!({ "_type": "PARTY_IDENTIFIED", "external_ref": party("PERSON", NURSE) });
    let rows: Vec<(&str, Option<EhrAccessSettings>, bool, Caller, Outcome)> = vec![
        ("no relationship", None, false, professional(NURSE), refused),
        (
            "a live relationship",
            None,
            true,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "a permit naming the caller",
            Some(settings("open", &[naming], &[], &[])),
            false,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "a gate-keeper",
            Some(settings("open", &[], &[], &[keeper])),
            false,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "an administrator",
            None,
            false,
            Caller {
                administrator: true,
                ..professional(NURSE)
            },
            Outcome::Admit,
        ),
        (
            "a system",
            None,
            false,
            caller(CallerKind::System, NURSE),
            Outcome::Admit,
        ),
        (
            "an emergency",
            None,
            false,
            professional(NURSE),
            Outcome::Admit,
        ),
    ];
    for (name, settings, cared, who, expected) in rows {
        let case = Case {
            settings,
            policy: Some(required.clone()),
            cared,
            ..Case::default()
        };
        let asked = if name == "an emergency" {
            break_glass(read(Some("ETREAT")))
        } else {
            read(None)
        };
        assert_outcome(&case.verdict(&who, asked), &Item::Ehr, expected, name);
    }
}

/// Step 6: where the policy requires one, a declared purpose the caller's
/// capabilities permit.
#[test]
fn step_6_purpose() {
    let required = Case {
        policy: Some(policy(&json!({ "purpose_required": true }))),
        ..Case::default()
    };
    let permitted = with_facts(professional(NURSE), &[], &[code(PURPOSE, "TREAT")], &[]);
    let rows = [
        (
            "no purpose",
            read(None),
            Outcome::Refused(Refusal::PurposeMissing),
        ),
        (
            "a purpose not permitted",
            read(Some("HOPERAT")),
            Outcome::Refused(Refusal::PurposeNotPermitted),
        ),
        ("a permitted purpose", read(Some("TREAT")), Outcome::Admit),
    ];
    for (name, asked, expected) in rows {
        assert_outcome(
            &required.verdict(&permitted, asked),
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

/// Step 7: the default disposition, and the server's for an EHR with no
/// settings.
#[test]
fn step_7_default_disposition() {
    let required = Outcome::Refused(Refusal::ConsentRequired);
    let permit = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", NURSE) }] }),
    );
    let scoped = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "party": party("PERSON", NURSE) }],
        "data": [{ "templates": ["vitals.v1"] }] }),
    );
    let keeper = json!({ "_type": "PARTY_IDENTIFIED", "external_ref": party("PERSON", NURSE) });
    let admin = Caller {
        administrator: true,
        ..professional(NURSE)
    };
    let rows: Vec<(&str, Case, bool, Caller, Outcome)> = vec![
        (
            "open",
            Case::of(settings("open", &[], &[], &[])),
            false,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "restricted",
            Case::of(settings("restricted", &[], &[], &[])),
            false,
            professional(NURSE),
            required,
        ),
        (
            "restricted, a permit",
            Case::of(settings("restricted", &[permit.clone()], &[], &[])),
            false,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "restricted, a care relationship",
            Case::of(settings("restricted", &[], &[], &[])),
            true,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "restricted, a gate-keeper",
            Case::of(settings("restricted", &[], &[], &[keeper.clone()])),
            false,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "restricted, an administrator",
            Case::of(settings("restricted", &[], &[], &[])),
            false,
            admin.clone(),
            Outcome::Admit,
        ),
        (
            "consent required",
            Case::of(settings("consent_required", &[], &[], &[])),
            true,
            professional(NURSE),
            required,
        ),
        (
            "consent required, a permit",
            Case::of(settings("consent_required", &[permit], &[], &[])),
            false,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "consent required, a gate-keeper",
            Case::of(settings("consent_required", &[], &[], &[keeper])),
            false,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "held closed",
            Case::of(EhrAccessSettings::HeldClosed),
            true,
            professional(NURSE),
            required,
        ),
        (
            "held closed, an administrator",
            Case::of(EhrAccessSettings::HeldClosed),
            false,
            admin.clone(),
            Outcome::Admit,
        ),
        (
            "no settings, open server",
            Case::default(),
            false,
            professional(NURSE),
            Outcome::Admit,
        ),
        (
            "no settings, restricted server",
            Case {
                server_default: Some(AccessDefault::Restricted),
                ..Case::default()
            },
            false,
            professional(NURSE),
            required,
        ),
        (
            "no settings, restricted server, an administrator",
            Case {
                server_default: Some(AccessDefault::Restricted),
                ..Case::default()
            },
            false,
            admin,
            Outcome::Admit,
        ),
    ];
    for (name, mut case, cared, who, expected) in rows {
        case.cared = cared;
        assert_outcome(&case.verdict(&who, read(None)), &Item::Ehr, expected, name);
    }
    let case = Case::of(settings("consent_required", &[scoped], &[], &[]));
    let verdict = case.verdict(&professional(NURSE), read(None));
    assert_outcome(
        &verdict,
        &object(OBJECT, Some("vitals.v1")),
        Outcome::Admit,
        "a scoped permit admits its data",
    );
    assert_outcome(
        &verdict,
        &object(OTHER, Some("lab_hiv.v1")),
        required,
        "and nothing else",
    );
    assert_outcome(&verdict, &Item::Ehr, required, "nor the EHR itself");
    assert!(verdict.item_dependent() && verdict.admits_any());
}

/// Deny overrides permit in step order: a restriction answers absent before a
/// consent deny refuses, and a consent deny before a sensitivity.
#[test]
fn the_first_step_that_refuses_decides() {
    let label = json!({ "_type": "ACCESS_LABEL", "target": object_ref(OBJECT),
                        "sensitivities": [coded(CONFIDENTIALITY, "HIV")] });
    let deny = directive(
        "consent",
        "deny",
        &json!({ "data": [{ "objects": [object_ref(OBJECT)] }] }),
    );
    let restriction = directive(
        "restriction",
        "deny",
        &json!({ "data": [{ "objects": [object_ref(OBJECT)] }] }),
    );
    let permit = directive("consent", "permit", &json!({}));
    let item = object(OBJECT, None);
    let cases = [
        (
            "restriction before consent",
            vec![deny.clone(), restriction],
            ABSENT_RESTRICTED,
        ),
        (
            "consent before sensitivity",
            vec![deny, permit.clone()],
            DENIED,
        ),
        (
            "a permit does not lift a deny",
            vec![permit],
            Outcome::Refused(Refusal::Sensitivity),
        ),
    ];
    for (name, directives, expected) in cases {
        let case = Case::of(settings("restricted", &directives, &[label.clone()], &[]));
        assert_outcome(
            &case.verdict(&professional(NURSE), read(None)),
            &item,
            expected,
            name,
        );
    }
}
