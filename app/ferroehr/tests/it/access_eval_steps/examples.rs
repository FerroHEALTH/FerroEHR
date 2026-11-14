// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The worked examples of the ABAC design, over the evaluator itself.

use std::sync::Arc;

use openehr_rm::draft::access_control::terminology::{AccessDefault, OperationClass, Refusal};
use serde_json::json;

use super::*;
use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess};
use ferroehr::access_eval::procedure::{Emergency, Lifted};

/// The ex-partner: a consent deny naming Dr B refuses him, and lifts for his
/// declared emergency with the break-glass role, which marks the access.
#[test]
fn the_ex_partner() {
    let deny = directive(
        "consent",
        "deny",
        &json!({ "applies_to": [{ "party": party("PERSON", DR_B) }], "emergency_override": true }),
    );
    let case = Case::of(settings("open", &[deny], &[], &[]));
    let dr_b = professional(DR_B);
    assert_outcome(
        &case.verdict(&dr_b, read(Some("TREAT"))),
        &Item::Ehr,
        Outcome::Refused(Refusal::ConsentDenied),
        "Dr B",
    );
    let emergency = Asked {
        emergency: Emergency {
            declared: true,
            break_glass: true,
            opt_out_override: false,
        },
        ..read(Some("ETREAT"))
    };
    let lifted = case.verdict(&dr_b, emergency);
    assert_outcome(&lifted, &Item::Ehr, Outcome::Admit, "Dr B in an emergency");
    assert!(
        lifted.lifted.contains(&Lifted::Consent),
        "the override is marked"
    );
    assert_outcome(
        &case.verdict(&professional(NURSE), read(None)),
        &Item::Ehr,
        Outcome::Admit,
        "anyone else",
    );
}

/// Mental health, except the psychiatrist: a restriction of the mental
/// health category with an exception for Dr C leaves the notes out for a
/// nurse, with no trace, and Dr C reads them.
#[test]
fn mental_health_except_the_psychiatrist() {
    let restriction = directive(
        "restriction",
        "deny",
        &json!({
            "data": [{ "categories": [coded(CATEGORIES, "mental-health")] }],
            "exceptions": [directive("consent", "permit",
                &json!({ "applies_to": [{ "party": party("PERSON", DR_C) }] }))]
        }),
    );
    let case = Case::of(settings("open", &[restriction], &[], &[]));
    let notes = object(OBJECT, Some("psych_note.v1"));
    let vitals = object(OTHER, Some("vitals.v1"));
    let nurse = case.verdict(&professional(NURSE), read(None));
    assert_outcome(
        &nurse,
        &notes,
        Outcome::Absent(Refusal::Restricted),
        "the nurse",
    );
    assert_outcome(&nurse, &vitals, Outcome::Admit, "the nurse, the rest");
    assert_outcome(&nurse, &Item::Ehr, Outcome::Admit, "the nurse, the EHR");
    let dr_c = case.verdict(&professional(DR_C), read(None));
    assert_outcome(&dr_c, &notes, Outcome::Admit, "Dr C");
}

/// The ward: the policy requires a care relationship and a 14-day grace; Mr
/// E is under the care of cardiology from 3 to 10 November. Dr D of
/// cardiology reads his record until 24 November and is refused on the 25th;
/// Dr F of nephrology never reads it.
#[test]
fn the_ward_with_its_grace_period() {
    let ward = policy(&json!({ "care_relationship_required": true,
        "relationship_grace": { "_type": "DV_DURATION", "value": "P14D" } }));
    let mr_e = [care_team(CARDIOLOGY, "2026-11-03", Some("2026-11-11"))];
    let judge = |caller: Caller, day: &str| {
        let access = RequestAccess::new(
            caller,
            Asking {
                operation: OperationClass::Read,
                purpose: Some("TREAT".to_owned()),
                emergency: Emergency::default(),
            },
            InForce {
                policy: Some(Arc::new(ward.clone())),
                categories: categories(),
                server_default: AccessDefault::Open,
                now: format!("{day}T12:00:00Z").parse().expect("an instant"),
                grants: None,
            },
        );
        access.judge(None, access.cared(&mr_e))
    };
    let dr_d = || with_facts(professional(NURSE), &[], &[], &[CARDIOLOGY]);
    let dr_f = with_facts(professional(DR_B), &[], &[], &[NEPHROLOGY]);
    let refused = Outcome::Refused(Refusal::NoCareRelationship);
    for (name, verdict, expected) in [
        (
            "Dr D during the stay",
            judge(dr_d(), "2026-11-05"),
            Outcome::Admit,
        ),
        (
            "Dr D on 24 November",
            judge(dr_d(), "2026-11-24"),
            Outcome::Admit,
        ),
        ("Dr D on 25 November", judge(dr_d(), "2026-11-25"), refused),
        ("Dr F", judge(dr_f, "2026-11-05"), refused),
    ] {
        assert_outcome(&verdict, &Item::Ehr, expected, name);
    }
}

/// HIV results: the policy classifies the HIV laboratory template; the
/// infectious-disease specialist cleared for it reads the results, and the
/// dermatologist reads the rest of the record without them.
#[test]
fn hiv_results() {
    let classified = policy(&json!({ "classifications": [{
        "_type": "ACCESS_CLASSIFICATION",
        "name": { "_type": "DV_TEXT", "value": "HIV results" },
        "archetype_node_id": "at0002",
        "data": { "_type": "ACCESS_DATA_SCOPE", "templates": ["lab_hiv.v1"] },
        "sensitivities": [coded(CONFIDENTIALITY, "HIV")] }] }));
    let case = Case {
        policy: Some(classified),
        ..Case::default()
    };
    let results = object(OBJECT, Some("lab_hiv.v1"));
    let vitals = object(OTHER, Some("vitals.v1"));
    let specialist = with_facts(
        professional(DR_C),
        &[code(CONFIDENTIALITY, "HIV")],
        &[],
        &[],
    );
    let specialist = case.verdict(&specialist, read(None));
    assert_outcome(&specialist, &results, Outcome::Admit, "the specialist");
    let dermatologist = case.verdict(&professional(NURSE), read(None));
    assert_outcome(
        &dermatologist,
        &results,
        Outcome::Refused(Refusal::Sensitivity),
        "the dermatologist, the results",
    );
    assert_outcome(
        &dermatologist,
        &vitals,
        Outcome::Admit,
        "the dermatologist, the rest",
    );
}

/// Regional sharing by BPPC: a permit copied from the policy catalogue,
/// identified by the region's opt-in policy, admits callers acting for the
/// regional group; `consent_required` holds for everyone else.
#[test]
fn regional_sharing_by_bppc() {
    let copied = directive(
        "consent",
        "permit",
        &json!({
            "applies_to": [{ "party": party("GROUP", REGION) }],
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "urn:oid:1.2.3.4",
                              "id": "regional-opt-in", "type": "BPPC" }],
            "evidence": [{ "_type": "DV_EHR_URI", "value": "ehr:/compositions/1" }]
        }),
    );
    let case = Case::of(settings("consent_required", &[copied], &[], &[]));
    let regional = Caller {
        acts_for: vec![REGION.to_owned()],
        ..professional(DR_C)
    };
    assert_outcome(
        &case.verdict(&regional, read(None)),
        &Item::Ehr,
        Outcome::Admit,
        "a caller acting for the region",
    );
    assert_outcome(
        &case.verdict(&professional(NURSE), read(None)),
        &Item::Ehr,
        Outcome::Refused(Refusal::ConsentRequired),
        "anyone else",
    );
}
