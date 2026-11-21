// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A rule's care roles and care path decide a read and a population query
//! alike, and the access record names the relationship the access rested
//! on, through the service and the store against a real `PostgreSQL` 18
//! (shared testkit harness), with the caller's care read from the census and
//! from the clinical care projection (#6075).
//!
//! The rule is the draft package's "only the responsible practitioner": a
//! deny rule whose permit exception names the care role `RESP` and the path
//! `direct` (draft access control decision procedure, Holding a Care
//! Relationship; Appendix, Who Sees What).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::access_eval_examples::{
    MEMBER, access, care, care_of, party, person, policy, relate_as_adt, update_as_adt,
};
use crate::fixtures::{composition, create_ehr_for_party, uv};
use crate::policy_fixtures::{
    CARE_PROVIDER, care_provider, care_writer, create, granted, granting, provider_team,
};
use ferroehr::access_eval::condition::{
    Asking, InForce, RecordFacts, RequestAccess, current, with_request_access,
};
use ferroehr::access_eval::procedure::{Emergency, Outcome};
use ferroehr::access_eval::scope::{CategoryIndex, Item};
use ferroehr::ids::{EhrId, VoId};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::message::AuditContext;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{AccessDefault, OperationClass, Refusal};

/// The version of the policy the access record names.
const POLICY_VERSION: &str = "4b1f2c3d-5e6f-4a7b-8c9d-0e1f2a3b4c5d::cdr.example.org::1";

/// The rows a query over every EHR reads.
const ROWS: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";
/// The count of the same rows.
const COUNT: &str = "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c";

/// A coded value of `terminology`.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// A care relationship of its own from `source` to the `kind` `carer` in the
/// role `role`, in the care of the hospital, from 1 November to 31 December
/// 2026.
fn under_care(source: VoId, (kind, carer): (&str, VoId), role: &str) -> Value {
    let mut body = care_of(source, carer, "2026-11-01", Some("2026-12-31"));
    body["target"]["type"] = json!(kind);
    body["details"]["items"][0]["value"] = coded("HL7_ParticipationType", role);
    body
}

/// The hospital's policy: a deny rule that only the responsible practitioner,
/// directly, passes.
fn only_the_responsible() -> AccessPolicy {
    let rule = |effect: &str, extra: Value| {
        let mut d = json!({ "_type": "ACCESS_DIRECTIVE",
            "name": { "_type": "DV_TEXT", "value": "rule" }, "archetype_node_id": "at0001",
            "kind": coded(TERMINOLOGY, "controller_rule"),
            "effect": coded(
                openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_CONSENT_PROVISION_TYPE,
                effect),
            "emergency_override": false });
        if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
            d.extend(extra.clone());
        }
        d
    };
    let responsible = rule(
        "permit",
        json!({ "applies_to": [{ "relationship": coded(TERMINOLOGY, "care"),
            "care_roles": [coded("HL7_ParticipationType", "RESP")],
            "care_path": coded(TERMINOLOGY, "direct") }] }),
    );
    policy(&json!({ "rules": [rule("deny", json!({ "exceptions": [responsible] }))] }))
}

/// Dr A, a member of cardiology, and three patients: P0 under Dr A as the
/// responsible practitioner and under cardiology, P1 under cardiology alone,
/// P2 under Dr A as a consultant; each record holds one composition.
struct Ward {
    _db: testkit::TestDb,
    svc: FerroEhrService,
    doctor: VoId,
    /// Each patient's EHR and composition.
    records: Vec<(EhrId, VoId)>,
    /// The version of P0's relationship to Dr A.
    responsible: String,
    /// The `PERSON` P0's relationship to Dr A covers for, when it covers.
    covered: Option<VoId>,
}

impl Ward {
    async fn new() -> Self {
        Self::build(false).await
    }

    /// The ward, with P0's relationship to Dr A covering for Dr B when
    /// `covering`.
    async fn build(covering: bool) -> Self {
        let db = testkit::db().await.expect("testkit database");
        let svc = granting(FerroEhrService::new(
            &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
        ))
        .await;
        granted(&svc, "adt", &[care_writer()]).await;
        let provider = care_provider(&svc, CARE_PROVIDER).await;
        let cardiology = party(
            "GROUP",
            "openEHR-DEMOGRAPHIC-GROUP.ward.v1",
            "Cardiology",
            &[],
        );
        let (cardiology, _) = provider_team(&svc, provider, &cardiology).await;
        let (doctor, uid) = create(&svc, PartyKind::Person, &person("Dr A", &[]))
            .await
            .expect("the professional commits");
        let roster = person(
            "Dr A",
            &[care(MEMBER, doctor, cardiology, "2026-10-01", None)],
        );
        update_as_adt(&svc, (doctor, &uid), &roster).await;
        let covered = if covering {
            let (covered, _) = create(&svc, PartyKind::Person, &person("Dr B", &[]))
                .await
                .expect("the covered professional commits");
            Some(covered)
        } else {
            None
        };
        let mut records = Vec::new();
        let mut responsible = String::new();
        let carers: [&[((&str, VoId), &str)]; 3] = [
            &[
                (("PERSON", doctor), "RESP"),
                (("GROUP", cardiology), "PART"),
            ],
            &[(("GROUP", cardiology), "PART")],
            &[(("PERSON", doctor), "CON")],
        ];
        for (index, carers) in carers.into_iter().enumerate() {
            let (patient, _) = create(&svc, PartyKind::Person, &person("Patient", &[]))
                .await
                .expect("the patient commits");
            for (carer, role) in carers {
                let mut body = under_care(patient, *carer, role);
                if let (0, "RESP", Some(covered)) = (index, *role, covered) {
                    body["details"]["items"]
                        .as_array_mut()
                        .expect("items")
                        .push(json!({ "_type": "ELEMENT", "archetype_node_id": "at0005",
                            "name": coded(TERMINOLOGY, "covers_for"),
                            "value": { "_type": "DV_IDENTIFIER", "issuer": "demographic",
                                       "id": covered.to_string(), "type": "PERSON" } }));
                }
                let (_, version) = relate_as_adt(&svc, &body).await;
                if index == 0 && *role == "RESP" {
                    responsible = version;
                }
            }
            let ehr_id = create_ehr_for_party(&svc, patient).await;
            let vo = svc
                .create_composition(ehr_id, uv(&composition("note"), "249", None))
                .await
                .expect("the composition commits")
                .vo_id;
            records.push((ehr_id, vo));
        }
        svc.grants().refresh().await.expect("the copy reads");
        Self {
            _db: db,
            svc,
            doctor,
            records,
            responsible,
            covered,
        }
    }

    /// Dr A's request under the policy on 20 November 2026, its access
    /// record naming the policy's version.
    fn request(&self) -> Arc<RequestAccess> {
        let built = access(
            &self.svc,
            self.doctor,
            only_the_responsible(),
            "2026-11-20T12:00:00Z".parse().expect("an instant"),
        );
        let facts = RecordFacts {
            policy_version: Some(POLICY_VERSION.to_owned()),
            capabilities: Vec::new(),
        };
        Arc::new(
            RequestAccess::new(
                built.caller().clone(),
                Asking {
                    operation: OperationClass::Read,
                    purpose: Some("TREAT".to_owned()),
                    emergency: Emergency::default(),
                },
                InForce {
                    policy: Some(Arc::new(only_the_responsible())),
                    categories: CategoryIndex::default(),
                    server_default: AccessDefault::Open,
                    now: built.now(),
                    grants: Some(Arc::clone(self.svc.grants())),
                },
            )
            .with_record_facts(facts),
        )
    }

    /// What Dr A's request reads: each record's outcome and whether its
    /// composition reads, then the rows and the count of a query over every
    /// EHR, all in one request.
    async fn reads(&self, access: Arc<RequestAccess>) -> (Vec<(Outcome, bool)>, usize, Value) {
        let svc = &self.svc;
        let records = &self.records;
        let run = async {
            let current = current().expect("the request access");
            let mut each = Vec::new();
            for (ehr_id, vo) in records {
                let verdict = svc
                    .ehr_verdict(&current, *ehr_id)
                    .await
                    .expect("the verdict");
                let read = Box::pin(svc.read_composition_raw(*ehr_id, *vo, None))
                    .await
                    .is_ok();
                each.push((current.note(*ehr_id, &verdict, &Item::Ehr), read));
            }
            let query = |aql: &'static str| async move {
                let request = AqlQueryRequest {
                    ehr_access: ferroehr::service::query::request::QueryAccess::Given(
                        svc.request_access_condition().await.expect("the condition"),
                    ),
                    ..AqlQueryRequest::default()
                };
                Box::pin(svc.execute_aql(aql, None, &request))
                    .await
                    .expect("the query runs")
                    .result_set
                    .to_value()
                    .expect("the result set")["rows"]
                    .clone()
            };
            let rows = query(ROWS).await.as_array().map_or(0, Vec::len);
            let count = query(COUNT).await[0][0].clone();
            (each, rows, count)
        };
        Box::pin(with_request_access(access, run)).await
    }
}

/// A rule naming `RESP` with `care_path: direct` refuses a team member and a
/// consultant in a read, and a population query reads and counts only the
/// record the rule admits, from the census and from the projection alike.
#[tokio::test]
async fn a_responsible_direct_rule_refuses_the_team_and_the_consultant_in_a_read_and_a_query() {
    let ward = Box::pin(Ward::new()).await;
    let denied = Outcome::Refused(Refusal::PolicyDenied);
    let expected = vec![(Outcome::Admit, true), (denied, false), (denied, false)];
    for form in ["census", "projection"] {
        if form == "projection" {
            ward.svc
                .project_care()
                .await
                .expect("the projection is written");
            assert!(
                ward.svc.care_projection_current(),
                "the query reads the projection"
            );
        } else {
            assert!(
                !ward.svc.care_projection_current(),
                "the query reads the census"
            );
        }
        let (each, rows, count) = ward.reads(ward.request()).await;
        assert_eq!(
            each, expected,
            "{form}: P0 responsible, P1 team, P2 consultant"
        );
        assert_eq!(rows, 1, "{form}: the query reads P0's composition alone");
        assert_eq!(count, json!(1), "{form}: the count agrees with the reads");
    }
}

/// The access record names the carer a relationship covers for in full:
/// P0's relationship to Dr A covers for Dr B, and the care ground's
/// `covers_for` carries Dr B's issuer, container and kind as its `type`,
/// read back from the stored caller fact (#6171).
#[tokio::test]
async fn the_access_record_names_the_kind_of_the_carer_a_relationship_covers_for() {
    let ward = Box::pin(Ward::build(true)).await;
    let covered = ward.covered.expect("the relationship covers for Dr B");
    let access = ward.request();
    let (each, _, _) = ward.reads(Arc::clone(&access)).await;
    assert_eq!(each[0], (Outcome::Admit, true));
    let decision = access.decision(Some(ward.records[0].0));
    let care = decision
        .grounds
        .iter()
        .find_map(|ground| ground.care.as_ref())
        .expect("the treatment relationship ground names the relationship");
    let cover = care.covers_for.as_ref().expect("the relationship covers");
    assert_eq!(
        (
            cover.issuer.as_deref(),
            cover.id.as_str(),
            cover.r#type.as_deref()
        ),
        (
            Some("demographic"),
            covered.to_string().as_str(),
            Some("PERSON")
        ),
        "the issuer, the container and the kind the covers_for carries"
    );
}

/// The access record of an access the rule admitted names the relationship
/// it rested on: P0 is under Dr A directly as `RESP` and under cardiology,
/// and the record names the direct `RESP` relationship by its version, with
/// its provider, not in grace.
#[tokio::test]
async fn the_access_record_names_the_relationship_the_rule_admitted() {
    let ward = Box::pin(Ward::new()).await;
    let access = ward.request();
    let (each, _, _) = ward.reads(Arc::clone(&access)).await;
    assert_eq!(each[0], (Outcome::Admit, true));
    let decision = access.decision(Some(ward.records[0].0));
    let care = decision
        .grounds
        .iter()
        .find_map(|ground| {
            (ground.kind.value == "treatment relationship")
                .then_some(ground.care.as_ref())
                .flatten()
        })
        .expect("the treatment relationship ground names the relationship");
    assert_eq!(care.role.defining_code.code_string, "RESP");
    assert_eq!(
        care.role.defining_code.terminology_id.value,
        "HL7_ParticipationType"
    );
    assert_eq!(care.path.defining_code.code_string, "direct");
    assert_eq!(care.carer.r#type, "PERSON");
    assert_eq!(
        care.relationship
            .as_ref()
            .map(|version| version.value().to_owned()),
        Some(ward.responsible.clone()),
        "the relationship's uid and version"
    );
    assert!(!care.in_grace);
    assert_eq!(
        (care.provider.issuer.as_deref(), care.provider.id.as_str()),
        (Some(CARE_PROVIDER.0), CARE_PROVIDER.1)
    );
    assert_eq!(care.covers_for, None);
    let refused = access.decision(Some(ward.records[1].0));
    assert!(
        refused.grounds.iter().all(|ground| ground.care.is_none()),
        "a refused access names no relationship"
    );
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    event.user_id = "dr-a".to_owned();
    event.decided = Some(Box::new(decision));
    let context = AuditContext {
        source_id: "ferroehr".to_owned(),
        server_ip: "10.42.23.77".to_owned(),
        value_if_missing: "UNKNOWN".to_owned(),
        ..AuditContext::default()
    };
    let rendered = ferroehr::system_log::fhir::to_fhir(&event, &context, Some("patient-0"))
        .expect("the AuditEvent renders");
    let user = rendered["agent"]
        .as_array()
        .expect("agents")
        .iter()
        .find(|agent| agent["who"]["identifier"]["value"] == "dr-a")
        .expect("the user agent");
    assert!(
        user["role"]
            .as_array()
            .expect("roles")
            .iter()
            .any(|role| role["coding"][0]
                == json!({ "system": "http://terminology.hl7.org/CodeSystem/v3-ParticipationType",
                       "code": "RESP" })),
        "the care role is an agent.role coding (FHIR R4 AuditEvent.agent.role): {rendered}"
    );
    assert!(
        user["policy"]
            .as_array()
            .expect("policies")
            .contains(&json!(POLICY_VERSION)),
        "the policy that named the relationship is an agent.policy: {rendered}"
    );
}
