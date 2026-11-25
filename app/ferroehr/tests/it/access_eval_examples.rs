// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Two worked examples of the ABAC design end to end through the service and
//! the store, against a real `PostgreSQL` 18 (shared testkit harness): the
//! care relationships the ADT system writes and the role a `ROLE` names are
//! read from the replica's copy of the caller facts, and the decision holds
//! for a named composition and for a query alike.
//!
//! BASE Architecture Overview `master07-security.adoc` §Security Policy,
//! "Time-limitation of access", bounds access by the episode of care "plus
//! some further time"; the decision procedure is our own design (#5513).

use std::future::Future;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::fixtures::{change_type, composition, create_ehr_for_party, if_match, object, uv};
use crate::policy_fixtures::{
    CARE_PROVIDER, capability, care_provider, care_writer, create, granted, granting, of_client,
    provider_team,
};
use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess, with_request_access};
use ferroehr::access_eval::procedure::{Caller, Outcome};
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
const ACT_CODE: &str = "HL7_ActCode";

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

/// A professional's membership of a care team: `openehr_access::member_of`.
pub(crate) const MEMBER: (&str, &str) = ("openehr_access", "member_of");
/// The provider the examples' care relationships are in the care of.
pub(crate) fn provider() -> ferroehr::party_binding::grants::caller::care_of::ProviderKey {
    ferroehr::party_binding::grants::caller::care_of::ProviderKey::new(
        "urn:example:providers",
        "hospital",
    )
}

/// A care relationship of its own from `source` to `team` in the role
/// `PART` of `HL7_ParticipationType`, in the care of [`provider`], over the
/// dates `from` to `until`, each included (draft access control package,
/// Care Relationships).
pub(crate) fn care_of(source: VoId, team: VoId, from: &str, until: Option<&str>) -> Value {
    let mut body = care((TERMINOLOGY, "under_care_of"), source, team, from, until);
    body["name"] = coded(TERMINOLOGY, "under_care_of");
    body["details"]["items"] = json!([
        { "_type": "ELEMENT", "archetype_node_id": "at0003",
          "name": coded(TERMINOLOGY, "care_role"),
          "value": coded("HL7_ParticipationType", "PART") },
        { "_type": "ELEMENT", "archetype_node_id": "at0004",
          "name": coded(TERMINOLOGY, "provider"),
          "value": { "_type": "DV_IDENTIFIER", "issuer": "urn:example:providers",
                     "id": "hospital", "type": "ORGANISATION" } }]);
    body
}

/// A relationship of `code` from `source` to `team`, over the dates `from`
/// to `until`, each included.
pub(crate) fn care(
    (terminology, code): (&str, &str),
    source: VoId,
    team: VoId,
    from: &str,
    until: Option<&str>,
) -> Value {
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
                        "value": coded(terminology, code) }] },
        "time_validity": validity,
        "source": party_ref("PERSON", source),
        "target": party_ref("GROUP", team)
    })
}

/// A party of `archetype` named `name` holding `relationships`.
pub(crate) fn party(rm: &str, archetype: &str, name: &str, relationships: &[Value]) -> Value {
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
pub(crate) fn person(name: &str, relationships: &[Value]) -> Value {
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
        local_issuer: ferroehr::service::DEFAULT_SYSTEM_ID.into(),
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
/// ADT client; the new version's uid.
pub(crate) async fn update_as_adt(
    svc: &FerroEhrService,
    (vo, uid): (VoId, &str),
    body: &Value,
) -> String {
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
    written
        .expect("the ADT system writes the care relationship")
        .meta
        .expect("a version")
        .uid
}

/// Creates the care relationship `body` as the ADT system; its container
/// and version uid.
pub(crate) async fn relate_as_adt(svc: &FerroEhrService, body: &Value) -> (VoId, String) {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    let written: Result<_, ServiceError> = as_client(svc, "adt", async {
        Box::pin(svc.party_relationship_create(write)).await
    })
    .await;
    let written = written.expect("the ADT system writes the care relationship");
    let uid = written.meta.as_ref().expect("a version").uid.clone();
    (crate::fixtures::written_vo(&written), uid)
}

/// Commits `body` as the next version of the care relationship `vo` after
/// `uid` as the ADT system; the new version's uid.
pub(crate) async fn rerelate_as_adt(
    svc: &FerroEhrService,
    (vo, uid): (VoId, &str),
    body: &Value,
) -> String {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(uid.parse().expect("an OBJECT_VERSION_ID"));
    let written: Result<_, ServiceError> = as_client(svc, "adt", async {
        Box::pin(svc.party_relationship_update(vo, if_match(uid), write)).await
    })
    .await;
    written
        .expect("the ADT system changes the care relationship")
        .meta
        .expect("a version")
        .uid
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
            ehr_access: ferroehr::service::query::request::QueryAccess::Given(
                svc.request_access_condition().await.expect("the condition"),
            ),
            ..AqlQueryRequest::default()
        };
        let rows = Box::pin(svc.execute_aql(
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
            None,
            &request,
        ))
        .await
        .expect("the query runs")
        .result_set
        .to_value()
        .unwrap()["rows"]
            .as_array()
            .map_or(0, Vec::len);
        (verdict.outcome(&Item::Ehr), read, rows)
    };
    Box::pin(with_request_access(access, run)).await
}

/// The request access of the professional bound to `party` holding `facts`,
/// under `policy`, at `now`.
pub(crate) fn access(
    svc: &FerroEhrService,
    party: VoId,
    policy: AccessPolicy,
    now: jiff::Timestamp,
) -> Arc<RequestAccess> {
    let facts = svc
        .grants()
        .facts_of(party.0, now)
        .with_providers([provider()]);
    let acts_for = facts
        .care_teams()
        .iter()
        .map(|team| team.team().to_string())
        .collect();
    Arc::new(RequestAccess::new(
        Caller {
            kind: CallerKind::Professional,
            party: Some(party.to_string()),
            own_ehrs: Vec::new(),
            patient_grants_only: false,
            opt_out_override: false,
            acts_for,
            facts,
            administrator: false,
            settings: SettingsCaller::new(false, false, Some(party.to_string())),
            lacks_consent_reference: false,
        },
        Asking {
            operation: OperationClass::Read,
            purpose: Some("TREAT".to_owned()),
            reason: None,
            episodes: None,
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
pub(crate) fn policy(members: &Value) -> AccessPolicy {
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
    granted(&svc, "adt", &[care_writer()]).await;
    let group = |name: &str| party("GROUP", "openEHR-DEMOGRAPHIC-GROUP.ward.v1", name, &[]);
    let provider = care_provider(&svc, CARE_PROVIDER).await;
    let (cardiology, _) = provider_team(&svc, provider, &group("Cardiology")).await;
    let (nephrology, _) = provider_team(&svc, provider, &group("Nephrology")).await;
    let mut doctors = Vec::new();
    for (name, team) in [("Dr D", cardiology), ("Dr F", nephrology)] {
        let (doctor, uid) = create(&svc, PartyKind::Person, &person(name, &[]))
            .await
            .expect("the professional commits");
        let roster = person(name, &[care(MEMBER, doctor, team, "2026-10-01", None)]);
        update_as_adt(&svc, (doctor, &uid), &roster).await;
        doctors.push(doctor);
    }
    let (patient, _) = create(&svc, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let stay = care_of(patient, cardiology, "2026-11-03", Some("2026-11-10"));
    relate_as_adt(&svc, &stay).await;
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

/// HIV results: the policy classifies the results and a rule clears the
/// infectious-disease role for them; the specialist performing that role
/// reads them, and a professional who does not reads the rest of the record
/// without them.
#[tokio::test]
async fn hiv_results() {
    let db = testkit::db().await.expect("testkit database");
    let svc = granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
    ))
    .await;
    let (specialist, _) = create(&svc, PartyKind::Person, &person("Dr ID", &[]))
        .await
        .expect("the specialist commits");
    let mut role =
        crate::policy_fixtures::role(specialist, &[capability("access_demographic", &[])]);
    role["name"] = coded("urn:example:roles", "INFECTIOUS_DISEASE");
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
        "sensitivities": [coded(ACT_CODE, "HIV")] }],
        "rules": [{ "_type": "ACCESS_DIRECTIVE", "name": text("Infectious disease reads HIV"),
            "archetype_node_id": "at0003",
            "kind": coded(TERMINOLOGY, "controller_rule"), "effect": coded(openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, "permit"),
            "applies_to": [{ "role": coded("urn:example:roles", "INFECTIOUS_DISEASE") }],
            "data": [{ "sensitivities": [coded(ACT_CODE, "HIV")] }],
            "emergency_override": false }] }));
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

/// The pseudonymisation domain an identity source issues the clinical
/// domain's subject pseudonyms in, as `[pixm] assigning_authority` names it.
const PIXM: &str = "urn:oid:2.999.7.1";

/// `body` declaring `subjects`, each `(namespace, id)`, as record subject
/// elements of its details (#5634).
fn declaring(mut body: Value, subjects: &[(&str, &str)]) -> Value {
    let items: Vec<Value> = subjects
        .iter()
        .map(|(namespace, id)| {
            json!({ "_type": "ELEMENT", "archetype_node_id": "at0010",
                "name": coded(TERMINOLOGY, "record_subject"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": namespace, "id": id, "type": "PI" } })
        })
        .collect();
    body["details"] = json!({ "_type": "ITEM_TREE", "archetype_node_id": "at0009",
        "name": text("details"), "items": items });
    body
}

/// Cardiology and nephrology with Dr D a member of the first and Dr F of the
/// second, written by the ADT system: the two teams and the two doctors.
async fn two_wards(svc: &FerroEhrService) -> ([VoId; 2], [VoId; 2]) {
    granted(svc, "adt", &[care_writer()]).await;
    let group = |name: &str| party("GROUP", "openEHR-DEMOGRAPHIC-GROUP.ward.v1", name, &[]);
    let provider = care_provider(svc, CARE_PROVIDER).await;
    let mut teams = Vec::new();
    let mut doctors = Vec::new();
    for (team, doctor) in [("Cardiology", "Dr D"), ("Nephrology", "Dr F")] {
        let (team, _) = provider_team(svc, provider, &group(team)).await;
        let (person_id, uid) = create(svc, PartyKind::Person, &person(doctor, &[]))
            .await
            .expect("the professional commits");
        let roster = person(doctor, &[care(MEMBER, person_id, team, "2026-10-01", None)]);
        update_as_adt(svc, (person_id, &uid), &roster).await;
        teams.push(team);
        doctors.push(person_id);
    }
    ([teams[0], teams[1]], [doctors[0], doctors[1]])
}

/// An EHR whose subject is `id` under `namespace`, with one composition.
async fn ehr_of(svc: &FerroEhrService, id: &str, namespace: &str) -> (EhrId, VoId) {
    let subject = ferroehr::service::ehr_index::types::SubjectRef::person(id, namespace);
    let ehr_id = crate::fixtures::create_ehr_subject(svc, &subject).await;
    let vo = svc
        .create_composition(ehr_id, uv(&composition("ward note"), "249", None))
        .await
        .expect("the composition commits")
        .vo_id;
    (ehr_id, vo)
}

/// The ward under both subject indexes (#5634): Mr E's `PERSON` declares his
/// pseudonym, so the EHR whose `EHR_STATUS` names the `PERSON` (`local`) and
/// the EHR whose `EHR_STATUS` names the pseudonym (`pixm`) both inherit his
/// care relationship, and the representation reaches both. A pseudonym no
/// `PERSON` declares, and the `PERSON`'s container id under another
/// namespace, name no `PERSON`, so care fails closed on them.
#[tokio::test]
async fn care_and_representation_resolve_under_both_subject_indexes() {
    let db = testkit::db().await.expect("testkit database");
    let svc = granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
    ))
    .await;
    let ([cardiology, _], [dr_d, dr_f]) = two_wards(&svc).await;
    let pseudonym = uuid::Uuid::now_v7().to_string();
    let (patient, uid) = create(&svc, PartyKind::Person, &person("Mr E", &[]))
        .await
        .expect("the patient commits");
    let admitted = declaring(person("Mr E", &[]), &[(PIXM, &pseudonym)]);
    update_as_adt(&svc, (patient, &uid), &admitted).await;
    relate_as_adt(
        &svc,
        &care_of(patient, cardiology, "2026-11-03", Some("2027-11-03")),
    )
    .await;
    let local = create_ehr_for_party(&svc, patient).await;
    let local_vo = svc
        .create_composition(local, uv(&composition("ward note"), "249", None))
        .await
        .expect("the composition commits")
        .vo_id;
    let (pixm, pixm_vo) = ehr_of(&svc, &pseudonym, PIXM).await;
    let (orphan, orphan_vo) = ehr_of(&svc, &uuid::Uuid::now_v7().to_string(), PIXM).await;
    let (foreign, foreign_vo) = ehr_of(&svc, &patient.to_string(), "local").await;
    svc.grants().refresh().await.expect("the copy reads");

    assert_eq!(
        svc.ehr_subject_person(local).await.expect("read"),
        Some(patient.0)
    );
    assert_eq!(
        svc.ehr_subject_person(pixm).await.expect("read"),
        Some(patient.0)
    );
    assert_eq!(svc.ehr_subject_person(orphan).await.expect("read"), None);
    assert_eq!(
        svc.ehr_subject_person(foreign).await.expect("read"),
        None,
        "a container id under another namespace names no PERSON of this store"
    );
    let mut represented = svc
        .ehr_ids_for_person(&patient.to_string(), "demographic")
        .await
        .expect("the EHRs of the represented PERSON");
    represented.sort_unstable();
    let mut both = vec![local, pixm];
    both.sort_unstable();
    assert_eq!(represented, both, "a representative reaches both EHRs");

    let ward = policy(&json!({ "care_relationship_required": true }));
    let refused = Outcome::Refused(Refusal::NoCareRelationship);
    for (ehr_id, vo, expected) in [
        (local, local_vo, Outcome::Admit),
        (pixm, pixm_vo, Outcome::Admit),
        (orphan, orphan_vo, refused),
        (foreign, foreign_vo, refused),
    ] {
        let judged = access(&svc, dr_d, ward.clone(), noon("2026-11-05"));
        let (outcome, read, rows) = decided(&svc, judged, ehr_id, vo).await;
        assert_eq!(outcome, expected, "Dr D on {ehr_id}");
        assert_eq!(read, expected == Outcome::Admit, "Dr D reads {ehr_id}");
        assert_eq!(rows, 2, "Dr D's query reads the local and the pixm EHR");
    }
    let judged = access(&svc, dr_f, ward, noon("2026-11-05"));
    let (outcome, _, rows) = decided(&svc, judged, pixm, pixm_vo).await;
    assert_eq!(outcome, refused, "Dr F is in no care relationship");
    assert_eq!(rows, 0);
}

/// An identity merge is one demographic contribution (#5634): it moves the
/// pseudonym of Mr A's EHR onto Mr B's `PERSON`, in either order of its
/// versions, so the EHR inherits Mr B's care from the next request and a
/// representative of Mr B reaches both EHRs. A contribution that leaves the
/// pseudonym on both `PERSON`s is refused.
#[tokio::test]
async fn an_identity_merge_moves_the_record_subject_in_one_contribution() {
    let db = testkit::db().await.expect("testkit database");
    let svc = granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
    ))
    .await;
    let ([cardiology, nephrology], [dr_d, dr_f]) = two_wards(&svc).await;
    let (p_a, p_b) = (
        uuid::Uuid::now_v7().to_string(),
        uuid::Uuid::now_v7().to_string(),
    );
    let mut persons = Vec::new();
    for (name, team, pseudonym) in [("Mr A", cardiology, &p_a), ("Mr B", nephrology, &p_b)] {
        let (vo, uid) = create(&svc, PartyKind::Person, &person(name, &[]))
            .await
            .expect("the patient commits");
        let body = declaring(person(name, &[]), &[(PIXM, pseudonym.as_str())]);
        update_as_adt(&svc, (vo, &uid), &body).await;
        relate_as_adt(&svc, &care_of(vo, team, "2026-11-01", Some("2027-11-01"))).await;
        persons.push(vo);
    }
    let (ehr_a, vo_a) = ehr_of(&svc, &p_a, PIXM).await;
    let (ehr_b, _) = ehr_of(&svc, &p_b, PIXM).await;
    svc.grants().refresh().await.expect("the copy reads");
    let ward = policy(&json!({ "care_relationship_required": true }));
    let on_a = |doctor| {
        let svc = &svc;
        let ward = ward.clone();
        async move {
            let judged = access(svc, doctor, ward, noon("2026-11-05"));
            decided(svc, judged, ehr_a, vo_a).await.0
        }
    };
    assert_eq!(on_a(dr_d).await, Outcome::Admit, "Mr A is in cardiology");
    assert_eq!(
        on_a(dr_f).await,
        Outcome::Refused(Refusal::NoCareRelationship)
    );

    let (a, b) = (persons[0], persons[1]);
    let version = |vo: VoId, body: Value| {
        json!({ "_type": "ORIGINAL_VERSION",
            "commit_audit": { "change_type": change_type("251", "modification"),
                "committer": adt_committer() },
            "lifecycle_state": change_type("532", "complete"),
            "preceding_version_uid": { "value": format!("{vo}::{}::2", ferroehr::service::DEFAULT_SYSTEM_ID) },
            "data": body })
    };
    let contribution = |versions: Vec<Value>| {
        json!({ "_type": "CONTRIBUTION", "versions": versions,
            "audit": { "change_type": change_type("251", "modification"),
                "committer": adt_committer() } })
    };
    let survivor = declaring(
        person("Mr B", &[]),
        &[(PIXM, p_b.as_str()), (PIXM, p_a.as_str())],
    );
    let doubled = contribution(vec![version(b, survivor.clone())]);
    let refused = Box::pin(as_client(
        &svc,
        "adt",
        svc.demographic_contribution_create(doubled),
    ))
    .await
    .expect_err("one pseudonym on two PERSONs is refused");
    assert!(matches!(refused, ServiceError::Conflict(_)), "{refused:?}");

    let merged = contribution(vec![version(b, survivor), version(a, person("Mr A", &[]))]);
    Box::pin(as_client(
        &svc,
        "adt",
        svc.demographic_contribution_create(merged),
    ))
    .await
    .expect("the merge commits as one contribution");
    svc.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    assert_eq!(
        svc.ehr_subject_person(ehr_a).await.expect("read"),
        Some(b.0)
    );
    assert_eq!(
        on_a(dr_f).await,
        Outcome::Admit,
        "EHR A now has Mr B's care"
    );
    assert_eq!(
        on_a(dr_d).await,
        Outcome::Refused(Refusal::NoCareRelationship)
    );
    let mut represented = svc
        .ehr_ids_for_person(&b.to_string(), "demographic")
        .await
        .expect("the EHRs of the survivor");
    represented.sort_unstable();
    let mut both = vec![ehr_a, ehr_b];
    both.sort_unstable();
    assert_eq!(
        represented, both,
        "a representative of Mr B reaches both EHRs"
    );
}

/// The committer placeholder naming the ADT client, the authenticated caller.
fn adt_committer() -> Value {
    json!({ "_type": "PARTY_IDENTIFIED",
        "identifiers": [{ "_type": "DV_IDENTIFIER",
            "issuer": crate::policy_fixtures::ISSUER, "id": "adt" }] })
}
