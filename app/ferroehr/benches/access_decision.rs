// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The evaluator micro-benchmark of the access-path record (#5590): what one
//! decision of the fixed evaluator costs for a professional under a hospital
//! policy, over an EHR with no settings, a realistic directive set and a heavy
//! one.
//!
//! `cargo bench -p ferroehr --bench access_decision` prints one JSON object
//! per directive set on stdout, the `evaluator` rows of
//! `docs/benchmarks/access-path/record.json`. The policy requires a care
//! relationship with a 14-day grace and a declared purpose, classifies one
//! archetype as HIV results, and holds a controller rule letting one role read
//! HIV data; the caller is a professional of one care team in that role who
//! declares treatment. No openEHR spec governs this measurement: our own
//! design.

#![expect(
    clippy::expect_used,
    reason = "bench fixture setup: a fixture that does not read must abort the bench loudly \
              (there is no caller to return an error to)"
)]
#![expect(
    clippy::print_stdout,
    reason = "the bench is a dev binary whose output is the record's rows"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the fixture settings are raw canonical JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

use std::sync::Arc;
use std::time::Instant;

use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess};
use ferroehr::access_eval::procedure::{Caller, Emergency};
use ferroehr::access_eval::scope::CategoryIndex;
use ferroehr::party_binding::grants::caller::CareTeam;
use ferroehr::party_binding::grants::caller::facts::CallerFacts;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::ehr::access_settings::EhrAccessSettings;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_CONSENT_PROVISION_TYPE;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, ForeignCode, OperationClass,
};
use serde_json::{Value, json};

/// The professional the decisions are made for.
const NURSE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f83";
/// The care team the professional is a member of.
const TEAM: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f84";
/// The professional a restriction excepts.
const NAMED: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f85";
/// The subject's representative.
const REPRESENTATIVE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f86";
/// The HL7 v3 `ActCode` sensitivity terminology id.
const ACT_CODE: &str = "2.16.840.1.113883.5.25";
/// The HL7 v3 `ActReason` purpose of use terminology id.
const ACT_REASON: &str = "2.16.840.1.113883.5.8";
/// The code system of the professional's functional role.
const ROLES: &str = "urn:example:roles";
/// The professional's functional role, which a controller rule lets read HIV
/// data.
const INFECTIOUS_DISEASE: &str = "INFECTIOUS_DISEASE";
/// The category terminology of the fixture.
const CATEGORIES: &str = "urn:example:categories";
/// The decisions timed per directive set and care state.
const DECISIONS: u32 = 200_000;
/// The decisions of one timed batch, whose per-decision mean is one sample.
const BATCH: u32 = 1_000;

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code, "defining_code": { "_type": "CODE_PHRASE",
        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology }, "code_string": code } })
}

fn term(code: &str) -> Value {
    coded(TERMINOLOGY, code)
}

fn party_ref(id: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
        "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// A directive of `kind` and `effect` holding `members`.
fn directive(kind: &str, effect: &str, members: &Value) -> Value {
    let mut d = json!({ "_type": "ACCESS_DIRECTIVE", "name": text(kind),
        "archetype_node_id": "at0001", "kind": term(kind), "effect": coded(TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, effect),
        "emergency_override": true });
    if let (Some(d), Some(members)) = (d.as_object_mut(), members.as_object()) {
        d.extend(members.clone());
    }
    d
}

/// A data scope over the category `code`.
fn category(code: &str) -> Value {
    json!([{ "_type": "ACCESS_DATA_SCOPE", "categories": [coded(CATEGORIES, code)] }])
}

/// The realistic directive set: treatment consent for the care team, a
/// mental-health restriction excepting one professional, an HIV restriction,
/// an opt-out of research export and query, and a representative.
fn realistic() -> Vec<Value> {
    vec![
        directive(
            "consent",
            "permit",
            &json!({ "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "relationship": term("care") }],
                "operations": [term("read"), term("query")],
                "purposes": [coded(ACT_REASON, "TREAT")] }),
        ),
        directive(
            "restriction",
            "deny",
            &json!({ "data": category("mental-health"),
                "exceptions": [directive("consent", "permit", &json!({ "applies_to": [{
                    "_type": "ACCESS_ACTOR_SCOPE", "party": party_ref(NAMED) }] }))] }),
        ),
        directive(
            "restriction",
            "deny",
            &json!({ "data": [{ "_type": "ACCESS_DATA_SCOPE",
                "sensitivities": [coded(ACT_CODE, "HIV")] }] }),
        ),
        directive(
            "opt_out",
            "deny",
            &json!({ "operations": [term("export"), term("query")],
                "purposes": [coded(ACT_REASON, "HRESCH")] }),
        ),
        directive(
            "delegation",
            "permit",
            &json!({ "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                "relationship": term("representative"), "party": party_ref(REPRESENTATIVE) }] }),
        ),
    ]
}

/// The heavy set: the realistic one with eleven more category restrictions.
fn heavy() -> Vec<Value> {
    let mut set = realistic();
    for at in 0..11 {
        set.push(directive(
            "restriction",
            "deny",
            &json!({ "data": category(&format!("category-{at}")) }),
        ));
    }
    set
}

/// The settings holding `directives`, kept by one gate-keeper, with one
/// label.
fn settings(directives: &[Value]) -> EhrAccessSettings {
    let settings = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term("open"),
        "directives": directives,
        "gate_keepers": [{ "_type": "PARTY_IDENTIFIED", "external_ref": party_ref(NAMED) }],
        "labels": [{ "_type": "ACCESS_LABEL",
            "target": { "_type": "OBJECT_REF", "namespace": "local", "type": "VERSIONED_COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f87" } },
            "sensitivities": [coded(ACT_CODE, "R")] }] });
    ferroehr::service::ehr::access_settings::check_written_settings(&settings)
        .expect("the fixture holds every invariant");
    EhrAccessSettings::from_projected(&settings.to_string()).expect("the fixture reads")
}

/// The hospital policy: a care relationship with a 14-day grace, a declared
/// purpose, and HIV results classified.
fn policy() -> AccessPolicy {
    let p = json!({ "_type": "ACCESS_POLICY", "name": text("Hospital access policy"),
        "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
        "care_relationship_required": true,
        "relationship_grace": { "_type": "DV_DURATION", "value": "P14D" },
        "purpose_required": true,
        "classifications": [{ "_type": "ACCESS_CLASSIFICATION", "name": text("HIV results"),
            "archetype_node_id": "at0002",
            "data": { "_type": "ACCESS_DATA_SCOPE", "archetypes": [{ "_type": "ARCHETYPE_ID",
                "value": "openEHR-EHR-OBSERVATION.laboratory_test_result.v1" }] },
            "sensitivities": [coded(ACT_CODE, "HIV")] }],
        "rules": [directive("controller_rule", "permit", &json!({
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "role": coded(ROLES, INFECTIOUS_DISEASE) }],
            "data": [{ "_type": "ACCESS_DATA_SCOPE", "sensitivities": [coded(ACT_CODE, "HIV")] }],
            "emergency_override": false }))] });
    openehr_its::json::from_canonical_value(&p).expect("the policy reads")
}

/// The professional's request to read, declaring treatment.
fn request() -> RequestAccess {
    let code = |terminology: &str, code: &str| ForeignCode {
        terminology: terminology.to_owned(),
        code: code.to_owned(),
    };
    let team = TEAM.parse().expect("a UUID");
    let facts = CallerFacts::new(
        vec![code(ROLES, INFECTIOUS_DISEASE)],
        vec![CareTeam::spanning(team, None, None)],
    );
    RequestAccess::new(
        Caller {
            kind: CallerKind::Professional,
            party: Some(NURSE.to_owned()),
            own_ehrs: Vec::new(),
            patient_grants_only: false,
            acts_for: vec![TEAM.to_owned()],
            facts,
            administrator: false,
            settings: SettingsCaller::new(false, false, Some(NURSE.to_owned())),
            lacks_consent_reference: false,
        },
        Asking {
            operation: OperationClass::Read,
            purpose: Some("TREAT".to_owned()),
            emergency: Emergency::default(),
        },
        InForce {
            policy: Some(Arc::new(policy())),
            categories: CategoryIndex::default(),
            server_default: AccessDefault::Open,
            now: jiff::Timestamp::now(),
            grants: None,
        },
    )
}

/// The median and the 99th percentile of `samples`, sorted in place.
fn quantiles(samples: &mut [u128]) -> (u128, u128) {
    samples.sort_unstable();
    let at = |q: usize| {
        samples
            .get((samples.len().saturating_sub(1) * q).div_euclid(100))
            .copied()
            .unwrap_or_default()
    };
    (at(50), at(99))
}

fn main() {
    let access = request();
    let sets = [
        ("none", None, 0usize),
        ("realistic", Some(settings(&realistic())), realistic().len()),
        ("heavy", Some(settings(&heavy())), heavy().len()),
    ];
    for (name, read, directives) in &sets {
        for cared in [true, false] {
            let mut admitted = 0usize;
            let mut samples = Vec::new();
            for _ in 0..DECISIONS.div_euclid(BATCH) {
                let started = Instant::now();
                for _ in 0..BATCH {
                    admitted += usize::from(access.judge(None, read.as_ref(), cared).admits_any());
                }
                samples.push(started.elapsed().as_nanos().div_euclid(u128::from(BATCH)));
            }
            let (p50, p99) = quantiles(&mut samples);
            let row = json!({
                "set": name,
                "directives": directives,
                "cared": cared,
                "decisions": DECISIONS,
                "decision_ns_p50": u64::try_from(p50).unwrap_or(u64::MAX),
                "decision_ns_p99": u64::try_from(p99).unwrap_or(u64::MAX),
                "admits": admitted > 0,
            });
            println!("{row}");
        }
    }
}
