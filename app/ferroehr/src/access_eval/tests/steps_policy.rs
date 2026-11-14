// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One table per step of the decision procedure and per refusal code:
//! steps 4 to 7, which read the caller's facts and the access policy, and
//! the order of the steps.

use openehr_rm::draft::access_control::terminology::{AccessDefault, CallerKind, Refusal};
use serde_json::json;

use super::*;

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
            Case::of(settings("open", &[], std::slice::from_ref(&label), &[])),
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
                ..Case::of(settings("open", &[], std::slice::from_ref(&label), &[]))
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
#[expect(
    clippy::too_many_lines,
    reason = "one row per case of the step's table"
)]
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
            Case::of(settings(
                "restricted",
                std::slice::from_ref(&permit),
                &[],
                &[],
            )),
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
            Case::of(settings(
                "restricted",
                &[],
                &[],
                std::slice::from_ref(&keeper),
            )),
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
        let case = Case::of(settings(
            "restricted",
            &directives,
            std::slice::from_ref(&label),
            &[],
        ));
        assert_outcome(
            &case.verdict(&professional(NURSE), read(None)),
            &item,
            expected,
            name,
        );
    }
}
