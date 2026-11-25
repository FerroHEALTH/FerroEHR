// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The worked examples of the ABAC design, over the evaluator itself.

use std::sync::Arc;

use openehr_rm::draft::access_control::terminology::{AccessDefault, OperationClass, Refusal};
use serde_json::json;

use super::*;
use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess};
use ferroehr::access_eval::procedure::Lifted;

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
    let emergency = break_glass(read(Some("ETREAT")));
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
    let mr_e = [care_of(CARDIOLOGY, "2026-11-03", Some("2026-11-11"))];
    let judge = |caller: Caller, day: &str| {
        let access = RequestAccess::new(
            caller,
            Asking {
                operation: OperationClass::Read,
                purpose: Some("TREAT".to_owned()),
                reason: None,
                opt_out_override: false,
                episodes: None,
            },
            InForce {
                policy: Some(Arc::new(ward.clone())),
                categories: categories(),
                server_default: AccessDefault::Open,
                now: format!("{day}T12:00:00Z").parse().expect("an instant"),
                grants: None,
            },
        );
        access.judge(None, None, &access.cared(&mr_e))
    };
    let dr_d = || with_facts(professional(NURSE), &[], &[CARDIOLOGY]);
    let dr_f = with_facts(professional(DR_B), &[], &[NEPHROLOGY]);
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

/// A care relationship reaches a caller directly, by its `PERSON` or a
/// `ROLE` it performs, or through a team or unit it is a member of, and only
/// while the caller acts for the relationship's provider (draft access
/// control decision procedure, Terms and Holding a Care Relationship;
/// #6055).
#[test]
fn a_care_relationship_admits_its_carer_only_under_its_provider() {
    const PERSON: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f92";
    const ROLE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f93";
    const UNIT: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f94";
    let ward = policy(&json!({ "care_relationship_required": true }));
    let judge = |caller: &Caller, carer: &str| {
        let access = RequestAccess::new(
            caller.clone(),
            Asking {
                operation: OperationClass::Read,
                purpose: Some("TREAT".to_owned()),
                reason: None,
                opt_out_override: false,
                episodes: None,
            },
            InForce {
                policy: Some(Arc::new(ward.clone())),
                categories: categories(),
                server_default: AccessDefault::Open,
                now: "2026-11-05T12:00:00Z".parse().expect("an instant"),
                grants: None,
            },
        );
        let relations = [care_of(carer, "2026-11-03", None)];
        access.judge(None, None, &access.cared(&relations))
    };
    let mut direct = professional(NURSE);
    direct.facts = CallerFacts::default()
        .with_party(uuid(PERSON))
        .with_providers([provider()]);
    let mut performing = professional(NURSE);
    performing.facts = CallerFacts::default()
        .with_performed(vec![uuid(ROLE)])
        .with_providers([provider()]);
    let team = with_facts(professional(NURSE), &[], &[CARDIOLOGY]);
    let unit = with_facts(professional(NURSE), &[], &[UNIT]);
    let refused = Outcome::Refused(Refusal::NoCareRelationship);
    for (name, caller, carer) in [
        ("the PERSON", &direct, PERSON),
        ("a performed ROLE", &performing, ROLE),
        ("a team", &team, CARDIOLOGY),
        ("a unit", &unit, UNIT),
    ] {
        assert_outcome(&judge(caller, carer), &Item::Ehr, Outcome::Admit, name);
        let mut alone = caller.clone();
        alone.facts = CallerFacts::new(
            caller.facts.roles().to_vec(),
            caller.facts.care_teams().to_vec(),
        )
        .with_performed(caller.facts.performed().to_vec());
        if let Some(party) = caller.facts.party() {
            alone.facts = alone.facts.with_party(party);
        }
        let mut elsewhere = alone.clone();
        elsewhere.facts = elsewhere
            .facts
            .with_providers([ProviderKey::new("urn:example:providers", "clinic")]);
        for (acting, caller) in [("no provider", &alone), ("another provider", &elsewhere)] {
            assert_outcome(
                &judge(caller, carer),
                &Item::Ehr,
                refused,
                &format!("{name}, acting for {acting}"),
            );
        }
    }
    assert_outcome(
        &judge(&team, NEPHROLOGY),
        &Item::Ehr,
        refused,
        "a team the caller is no member of",
    );
}

/// HIV results: the policy classifies the HIV laboratory template and clears
/// the infectious-disease role for it; the specialist reads the results, and
/// the dermatologist reads the rest of the record without them.
#[test]
fn hiv_results() {
    let classified = policy(&json!({ "classifications": [{
        "_type": "ACCESS_CLASSIFICATION",
        "name": { "_type": "DV_TEXT", "value": "HIV results" },
        "archetype_node_id": "at0002",
        "data": { "_type": "ACCESS_DATA_SCOPE", "templates": ["lab_hiv.v1"] },
        "sensitivities": [coded(CONFIDENTIALITY, "HIV")] }],
        "rules": [rule("permit", &json!({ "applies_to": [role_scope("INFECTIOUS_DISEASE")],
            "data": [{ "sensitivities": [coded(CONFIDENTIALITY, "HIV")] }] }))] }));
    let case = Case {
        policy: Some(classified),
        ..Case::default()
    };
    let results = object(OBJECT, Some("lab_hiv.v1"));
    let vitals = object(OTHER, Some("vitals.v1"));
    let specialist = performing(CallerKind::Professional, DR_C, "INFECTIOUS_DISEASE");
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
