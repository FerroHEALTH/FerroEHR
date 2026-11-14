// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Two worked examples of the ABAC design end to end through the service and
//! the store, against a real `PostgreSQL` 18 (shared testkit harness): the
//! care relationships the ADT system writes and the clearance a capability
//! carries are read from the replica's copy of the caller facts, and the
//! decision holds for a named composition and for a query alike.
//!
//! BASE Architecture Overview `master07-security.adoc` §Security Policy,
//! "Time-limitation of access", bounds access by the episode of care "plus
//! some further time"; the decision procedure is our own design (#5513).

use std::future::Future;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::fixtures::{composition, create_ehr_for_party, if_match, object, uv};
use crate::policy_fixtures::{capability, create, granted, granting, of_client};
use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess, with_request_access};
use ferroehr::access_eval::procedure::{Caller, Emergency, Outcome};
use ferroehr::access_eval::scope::{CategoryIndex, Item};
use ferroehr::error::ServiceError;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::party_binding::grants::capabilities::with_capabilities;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, OperationClass, Refusal,
};

/// The HL7 v3 `ActCode` sensitivity terminology id.
const ACT_CODE: &str = "2.16.840.1.113883.5.25";

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

fn party_ref(kind: &str, id: VoId) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": kind,
            "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() } })
}

/// A care relationship of `code` from `source` to `team`, over the dates
/// `from` to `until`, each included.
fn care(code: &str, source: VoId, team: VoId, from: &str, until: Option<&str>) -> Value {
    let mut validity = json!({ "_type": "DV_INTERVAL",
        "lower": { "_type": "DV_DATE", "value": from },
        "lower_unbounded": false, "upper_unbounded": until.is_none(),
        "lower_included": true, "upper_included": until.is_some() });
    if let Some(until) = until {
        validity["upper"] = json!({ "_type": "DV_DATE", "value": until });
    }
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.ward_stay.v3",
        "name": text("Ward stay"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("kind"),
                        "value": coded(TERMINOLOGY, code) }] },
        "time_validity": validity,
        "source": party_ref("PERSON", source),
        "target": party_ref("GROUP", team)
    })
}

/// A party of `archetype` named `name` holding `relationships`.
fn party(rm: &str, archetype: &str, name: &str, relationships: &[Value]) -> Value {
    let mut body = json!({
        "_type": rm, "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "name": text(name),
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": text("name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("name"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                            "value": text(name) }] } }]
    });
    if !relationships.is_empty() {
        body["relationships"] = Value::Array(relationships.to_vec());
    }
    body
}

/// A `PERSON` named `name`.
fn person(name: &str, relationships: &[Value]) -> Value {
    party(
        "PERSON",
        "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        name,
        relationships,
    )
}

/// Runs `fut` as the system client `client_id` inside its grants on `svc`.
async fn as_client<F: Future>(svc: &FerroEhrService, client_id: &str, fut: F) -> F::Output {
    let caller = CommitterIdentity {
        subject: client_id.to_owned(),
        id_type: "oauth2",
        issuer: Some(crate::policy_fixtures::ISSUER.to_owned()),
        client: Some(client_id.to_owned()),
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: true,
        agent: None,
        external: None,
    };
    let granted = of_client(svc, client_id).await;
    with_capabilities(granted, with_committer(Some(caller), Box::pin(fut))).await
}

/// Commits `body` as the next version of the `PERSON` `vo` after `uid` as the
/// ADT client.
async fn update_as_adt(svc: &FerroEhrService, (vo, uid): (VoId, &str), body: &Value) {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(uid.parse().expect("an OBJECT_VERSION_ID"));
    let written: Result<_, ServiceError> = as_client(svc, "adt", async {
        Box::pin(svc.party_update(
            PartyKind::Person,
            object(vo.to_string()),
            if_match(uid),
            write,
        ))
        .await
    })
    .await;
    written.expect("the ADT system writes the care relationship");
}

/// The answer `access` gives on `ehr_id`, the composition `vo` read, and the
/// compositions a query reads, all as one request.
async fn decided(
    svc: &FerroEhrService,
    access: Arc<RequestAccess>,
    ehr_id: EhrId,
    vo: VoId,
) -> (Outcome, bool, usize) {
    let run = async {
        let current = ferroehr::access_eval::condition::current().expect("the request access");
        let verdict = svc
            .ehr_verdict(&current, ehr_id)
            .await
            .expect("the verdict");
        let read = Box::pin(svc.read_composition_raw(ehr_id, vo, None))
            .await
            .is_ok();
        let request = AqlQueryRequest {
            ehr_access: svc.request_access_condition().await.expect("the condition"),
            ..AqlQueryRequest::default()
        };
        let rows = Box::pin(svc.execute_aql(
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
            None,
            &request,
        ))
        .await
        .expect("the query runs")
        .result_set["rows"]
            .as_array()
            .map_or(0, Vec::len);
        (verdict.outcome(&Item::Ehr), read, rows)
    };
    Box::pin(with_request_access(access, run)).await
}

/// The request access of the professional bound to `party` holding `facts`,
/// under `policy`, at `now`.
fn access(
    svc: &FerroEhrService,
    party: VoId,
    policy: AccessPolicy,
    now: jiff::Timestamp,
) -> Arc<RequestAccess> {
    let facts = svc.grants().facts_of(party.0, now);
    let acts_for = facts
        .care_teams()
        .iter()
        .map(|team| team.team().to_string())
        .collect();
    Arc::new(RequestAccess::new(
        Caller {
            kind: CallerKind::Professional,
            party: Some(party.to_string()),
            own_ehr: None,
            patient_grants_only: false,
            acts_for,
            roles: Vec::new(),
            facts,
            administrator: false,
            settings: SettingsCaller::new(false, false, Some(party.to_string())),
        },
        Asking {
            operation: OperationClass::Read,
            purpose: Some("TREAT".to_owned()),
            emergency: Emergency::default(),
        },
        InForce {
            policy: Some(Arc::new(policy)),
            categories: CategoryIndex::default(),
            server_default: AccessDefault::Open,
            now,
            grants: Some(Arc::clone(svc.grants())),
        },
    ))
}

/// An `ACCESS_POLICY` with `members`.
fn policy(members: &Value) -> AccessPolicy {
    let mut p = json!({
        "_type": "ACCESS_POLICY",
        "name": text("Hospital access policy"),
        "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
        "care_relationship_required": false,
        "purpose_required": false
    });
    if let (Some(p), Some(members)) = (p.as_object_mut(), members.as_object()) {
        p.extend(members.clone());
    }
    openehr_its::json::from_canonical_value(&p).expect("the policy reads")
}

/// `day` at noon UTC.
fn noon(day: &str) -> jiff::Timestamp {
    format!("{day}T12:00:00Z").parse().expect("an instant")
}

/// The ward: the hospital requires a care relationship with a 14-day grace;
/// Mr E is under the care of cardiology from 3 to 10 November. Dr D, a member
/// of cardiology, reads his record until 24 November and is refused on the
/// 25th; Dr F, a member of nephrology, never reads it.
#[tokio::test]
async fn the_ward_with_its_grace_period() {
    let db = testkit::db().await.expect("testkit database");
    let svc = granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
    ))
    .await;
    granted(&svc, "adt", &[capability("write_care_relationship", &[])]).await;
    let group = |name: &str| party("GROUP", "openEHR-DEMOGRAPHIC-GROUP.ward.v1", name, &[]);
    let (cardiology, _) = create(&svc, PartyKind::Group, &group("Cardiology"))
        .await
        .expect("the GROUP commits");
    let (nephrology, _) = create(&svc, PartyKind::Group, &group("Nephrology"))
        .await
        .expect("the GROUP commits");
    let mut doctors = Vec::new();
    for (name, team) in [("Dr D", cardiology), ("Dr F", nephrology)] {
        let (doctor, uid) = create(&svc, PartyKind::Person, &person(name, &[]))
            .await
            .expect("the professional commits");
        let roster = person(name, &[care("member_of", doctor, team, "2026-10-01", None)]);
        update_as_adt(&svc, (doctor, &uid), &roster).await;
        doctors.push(doctor);
    }
    let (patient, uid) = create(&svc, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let stay = care(
        "under_care_of",
        patient,
        cardiology,
        "2026-11-03",
        Some("2026-11-10"),
    );
    update_as_adt(&svc, (patient, &uid), &person("Mr E", &[stay])).await;
    let ehr_id = create_ehr_for_party(&svc, patient).await;
    let vo = svc
        .create_composition(ehr_id, uv(&composition("cardiology note"), "249", None))
        .await
        .expect("the composition commits")
        .vo_id;
    svc.grants().refresh().await.expect("the copy reads");
    let ward = policy(&json!({ "care_relationship_required": true,
        "relationship_grace": { "_type": "DV_DURATION", "value": "P14D" } }));
    let refused = Outcome::Refused(Refusal::NoCareRelationship);
    for (name, doctor, day, expected) in [
        (
            "Dr D on 24 November",
            doctors[0],
            "2026-11-24",
            Outcome::Admit,
        ),
        ("Dr D on 25 November", doctors[0], "2026-11-25", refused),
        ("Dr F", doctors[1], "2026-11-05", refused),
    ] {
        let judged = access(&svc, doctor, ward.clone(), noon(day));
        let (outcome, read, rows) = decided(&svc, Arc::clone(&judged), ehr_id, vo).await;
        let admitted = expected == Outcome::Admit;
        assert_eq!(outcome, expected, "{name}");
        assert_eq!(read, admitted, "{name}: the composition read");
        assert_eq!(rows, usize::from(admitted), "{name}: the query");
        // NOTE: EHDS Annex II 3.2, the access record names the care relationship it rests on.
        let cared = judged
            .decision(Some(ehr_id))
            .grounds
            .iter()
            .any(|ground| ground.kind.value == "treatment relationship");
        assert_eq!(cared, admitted, "{name}: the treatment relationship ground");
    }
}

/// HIV results: the policy classifies the results; the specialist whose
/// capability clears `HIV` reads them, and a professional without the
/// clearance reads the rest of the record without them.
#[tokio::test]
async fn hiv_results() {
    let db = testkit::db().await.expect("testkit database");
    let svc = granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
    ))
    .await;
    let qualifier = |name: &str, value: &Value| {
        json!({ "_type": "ELEMENT", "archetype_node_id": "at0040",
                "name": coded(TERMINOLOGY, name), "value": value })
    };
    let (specialist, _) = create(&svc, PartyKind::Person, &person("Dr ID", &[]))
        .await
        .expect("the specialist commits");
    let role = crate::policy_fixtures::role(
        specialist,
        &[capability(
            "access_demographic",
            &[qualifier("sensitivity_clearance", &coded(ACT_CODE, "HIV"))],
        )],
    );
    create(&svc, PartyKind::Role, &role)
        .await
        .expect("the ROLE commits");
    let (dermatologist, _) = create(&svc, PartyKind::Person, &person("Dr Skin", &[]))
        .await
        .expect("the dermatologist commits");
    let (patient, _) = create(&svc, PartyKind::Person, &person("Ms H", &[]))
        .await
        .expect("the patient commits");
    let ehr_id = create_ehr_for_party(&svc, patient).await;
    let results = svc
        .create_composition(ehr_id, uv(&composition("HIV results"), "249", None))
        .await
        .expect("the composition commits")
        .vo_id;
    svc.grants().refresh().await.expect("the copy reads");
    let classified = policy(&json!({ "classifications": [{
        "_type": "ACCESS_CLASSIFICATION",
        "name": text("HIV results"),
        "archetype_node_id": "at0002",
        "data": { "_type": "ACCESS_DATA_SCOPE",
                  "archetypes": [{ "_type": "ARCHETYPE_ID",
                                   "value": "openEHR-EHR-COMPOSITION.encounter.v1" }] },
        "sensitivities": [coded(ACT_CODE, "HIV")] }] }));
    let now = jiff::Timestamp::now();
    let (_, read, rows) = decided(
        &svc,
        access(&svc, specialist, classified.clone(), now),
        ehr_id,
        results,
    )
    .await;
    assert!(read && rows == 1, "the specialist reads the results");
    let (outcome, read, rows) = decided(
        &svc,
        access(&svc, dermatologist, classified, now),
        ehr_id,
        results,
    )
    .await;
    assert_eq!(outcome, Outcome::Admit, "the dermatologist reaches the EHR");
    assert!(!read && rows == 0, "and not the results");
}
