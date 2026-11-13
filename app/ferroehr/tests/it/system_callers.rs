// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a system caller commits and who attests (#5100), over a real
//! PostgreSQL 18.
//!
//! EHDS Art. 13(4) asks the record to identify the professional and the
//! provider, so a system commit is authorized by the policy parties (the
//! client's `AGENT` acting for its provider and performing a `ROLE` granting
//! `commit_composition`, #5436) and its facility is checked, never filled. RM `COMPOSITION.composer`
//! is "The person primarily responsible for the content of the Composition";
//! an `ATTESTATION` is "an explicit signing by one healthcare agent" (RM
//! common master04 §Attestation). What a caller kind may do is our own
//! design.

use serde_json::{Value, json};

use crate::committer_binding::{ISSUER, KEY, bound, placeholder, professional, service};
use crate::fixtures::{attestation_by, ckm_example, deleting};
use crate::system_grants::{
    DEVICES, FACILITIES, MACHINES, System, run, system, system_client, writes,
};
use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::change::Committed;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The issuer of the professionals' identifiers a relayed record names.
const PROFESSIONALS: &str = "urn:oid:2.999.5";

/// The system caller of `client`, acting for a provider, granted to commit
/// `template` from `origins`.
async fn granted(svc: &FerroEhrService, client: &str, template: &str, origins: &[&str]) -> System {
    system_client(svc, client, true, &[writes(template, origins)]).await;
    system(svc, client).await
}

/// A `PARTY_IDENTIFIED` carrying one identifier.
fn party(name: &str, issuer: &str, id: &str) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": name,
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id }] })
}

/// Uploads the vital-signs template and returns its example with the
/// template id.
async fn example(svc: &FerroEhrService) -> (Value, String) {
    let example = ckm_example(svc, "vital-signs").await;
    let template = example
        .pointer("/archetype_details/template_id/value")
        .and_then(Value::as_str)
        .expect("the example names its template")
        .to_owned();
    (example, template)
}

/// `example` composed by `composer` at the facility `facility`.
fn vital_signs(example: &Value, composer: Value, facility: &str) -> Value {
    let mut body = example.clone();
    body["composer"] = composer;
    body["context"]["health_care_facility"] = party("Ward 7", FACILITIES, facility);
    body
}

/// Commits `body` into `ehr` as `caller`.
async fn commit(
    svc: &FerroEhrService,
    ehr: EhrId,
    caller: Option<&System>,
    body: &Value,
) -> Result<Committed, ServiceError> {
    let request = WriteRequest::new(crate::typed_body::typed(body));
    run(caller, svc.create_composition(ehr, request))
        .await
        .map_err(ServiceError::flattened)
}

/// The stored committer of `committed` in `ehr`.
async fn stored_committer(svc: &FerroEhrService, ehr: EhrId, committed: &Committed) -> Value {
    let ovid = format!(
        "{}::{}::{}",
        committed.vo_id, committed.creating_system_id, committed.tree
    );
    svc.composition_version_envelope(ehr, ovid.parse().expect("an OBJECT_VERSION_ID"))
        .await
        .expect("the stored version")
        .body["commit_audit"]["committer"]
        .clone()
}

fn forbidden<T: std::fmt::Debug>(result: Result<T, ServiceError>) {
    match result {
        Err(ServiceError::Forbidden(_)) => {}
        other => panic!("expected 403, got {other:?}"),
    }
}

/// Asserts a `422` whose violation names `element`.
fn unprocessable<T: std::fmt::Debug>(result: Result<T, ServiceError>, element: &str) {
    match result {
        Err(ServiceError::Unprocessable { violation, .. }) => {
            assert_eq!(violation.path(), Some(element), "{violation:?}");
        }
        other => panic!("expected 422 naming {element}, got {other:?}"),
    }
}

#[tokio::test]
async fn an_ungranted_system_client_commits_nothing_into_an_ehr() {
    let (_db, svc, ehr) = service().await;
    let (example, template) = example(&svc).await;
    let body = vital_signs(&example, party("Dr A", PROFESSIONALS, "hp-1"), "ward-7");
    let unknown = system(&svc, "lab").await;
    assert!(unknown.granted.client().is_none());
    forbidden(commit(&svc, ehr, Some(&unknown), &body).await);
    let created = run(Some(&unknown), svc.create_ehr_meta(WriteRequest::new(None)))
        .await
        .map_err(ServiceError::flattened);
    forbidden(created);
    // An AGENT with no grant commits nothing either.
    system_client(&svc, "idle", true, &[]).await;
    let idle = system(&svc, "idle").await;
    assert!(idle.granted.client().is_some());
    forbidden(commit(&svc, ehr, Some(&idle), &body).await);
    // A grant whose client acts for no provider commits nothing (EHDS Art.
    // 13(4)).
    system_client(&svc, "orphan", false, &[writes(&template, &["relay"])]).await;
    let orphan = system(&svc, "orphan").await;
    assert!(
        orphan
            .granted
            .client()
            .is_some_and(|client| client.provider.is_none())
    );
    forbidden(commit(&svc, ehr, Some(&orphan), &body).await);
}

#[tokio::test]
async fn a_relay_client_commits_a_person_act_under_the_client_committer() {
    let (_db, svc, ehr) = service().await;
    let (example, template) = example(&svc).await;
    let body = vital_signs(&example, party("Dr A", PROFESSIONALS, "hp-1"), "ward-7");
    let lab = granted(&svc, "lab", &template, &["relay"]).await;
    let agent = lab.identity.agent.expect("the client names its AGENT");
    let committed = commit(&svc, ehr, Some(&lab), &body)
        .await
        .expect("a relayed person act commits");
    assert_eq!(
        stored_committer(&svc, ehr, &committed).await,
        json!({ "_type": "PARTY_IDENTIFIED", "name": "lab",
                "external_ref": { "_type": "PARTY_REF", "namespace": "demographic",
                    "type": "ACTOR", "id": { "_type": "HIER_OBJECT_ID", "value": agent.to_string() } },
                "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": MACHINES, "id": "lab",
                                  "type": "client" }] })
    );
    // The composer may be nameless when a performer participation names the
    // professional.
    let mut performed = body.clone();
    performed["composer"] = json!({ "_type": "PARTY_IDENTIFIED", "name": "Lab system" });
    performed["context"]["participations"] = json!([{ "_type": "PARTICIPATION",
        "function": { "_type": "DV_TEXT", "value": "performer" },
        "performer": party("Dr B", PROFESSIONALS, "hp-2") }]);
    commit(&svc, ehr, Some(&lab), &performed)
        .await
        .expect("a performer participation names the professional");
    // A person act naming nobody by an identifier is refused.
    let mut anonymous = body.clone();
    anonymous["composer"] = json!({ "_type": "PARTY_IDENTIFIED", "name": "A clinician" });
    unprocessable(
        commit(&svc, ehr, Some(&lab), &anonymous).await,
        "COMPOSITION.composer",
    );
    // An organisation composer is refused.
    let mut organisation = body.clone();
    organisation["composer"] = json!({ "_type": "PARTY_IDENTIFIED", "name": "Hospital",
        "external_ref": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": uuid::Uuid::now_v7().to_string() } } });
    unprocessable(
        commit(&svc, ehr, Some(&lab), &organisation).await,
        "COMPOSITION.composer",
    );
}

#[tokio::test]
async fn a_system_commit_is_held_to_its_writes_and_its_facilities() {
    let (_db, svc, ehr) = service().await;
    let (example, template) = example(&svc).await;
    let body = vital_signs(&example, party("Dr A", PROFESSIONALS, "hp-1"), "ward-7");
    let elsewhere = granted(&svc, "elsewhere", "Another template", &["relay"]).await;
    forbidden(commit(&svc, ehr, Some(&elsewhere), &body).await);
    let lab = granted(&svc, "lab", &template, &["relay"]).await;
    let other_ward = vital_signs(&example, party("Dr A", PROFESSIONALS, "hp-1"), "ward-8");
    unprocessable(
        commit(&svc, ehr, Some(&lab), &other_ward).await,
        "EVENT_CONTEXT.health_care_facility",
    );
    let mut unlocated = body.clone();
    unlocated
        .as_object_mut()
        .expect("an object")
        .shift_remove("context");
    unlocated["feeder_audit"] = json!({ "_type": "FEEDER_AUDIT",
        "originating_system_audit": { "_type": "FEEDER_AUDIT_DETAILS", "system_id": "lab" } });
    unprocessable(
        commit(&svc, ehr, Some(&lab), &unlocated).await,
        "COMPOSITION.feeder_audit.originating_system_audit.location",
    );
    // A deletion of a COMPOSITION outside the writes is refused too.
    let committed = commit(&svc, ehr, None, &body).await.expect("commits");
    let uid = format!(
        "{}::{}::{}",
        committed.vo_id, committed.creating_system_id, committed.tree
    )
    .parse()
    .expect("an OBJECT_VERSION_ID");
    let deleted = run(
        Some(&elsewhere),
        svc.delete_composition(ehr, &uid, deleting(Some(uid.clone()))),
    )
    .await
    .map_err(ServiceError::flattened);
    forbidden(deleted);
}

#[tokio::test]
async fn a_device_client_commits_a_device_act_and_no_person_act() {
    let (_db, svc, ehr) = service().await;
    let (example, template) = example(&svc).await;
    let mut device_act = vital_signs(&example, party("Pump", DEVICES, "pump-1"), "ward-7");
    device_act["feeder_audit"] = json!({ "_type": "FEEDER_AUDIT",
        "originating_system_audit": { "_type": "FEEDER_AUDIT_DETAILS", "system_id": "pump-1" } });
    let pump = granted(&svc, "pump", &template, &["device"]).await;
    commit(&svc, ehr, Some(&pump), &device_act)
        .await
        .expect("a device act commits with the facility alone");
    // A person act from a client without the relay origin is refused, and
    // so is one naming a professional.
    let person_act = vital_signs(&example, party("Dr A", PROFESSIONALS, "hp-1"), "ward-7");
    forbidden(commit(&svc, ehr, Some(&pump), &person_act).await);
    // A device act from a relay-only client is refused.
    let relay = granted(&svc, "lab", &template, &["relay"]).await;
    forbidden(commit(&svc, ehr, Some(&relay), &device_act).await);
}

#[tokio::test]
async fn a_person_act_naming_a_local_person_is_refused_from_a_client_without_relay() {
    let (_db, svc, ehr) = service().await;
    let vo = professional(&svc, "Dr Jane Roe", KEY).await;
    let local = json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe",
        "external_ref": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": vo.to_string() } },
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": PROFESSIONALS, "id": "hp-1" }] });
    let (example, template) = example(&svc).await;
    let body = vital_signs(&example, local, "ward-7");
    let pump = granted(&svc, "pump", &template, &["device"]).await;
    forbidden(commit(&svc, ehr, Some(&pump), &body).await);
}

/// The `UPDATE_ATTESTATION` committed by `committer`.
fn attestation(committer: &Value) -> openehr_its::rest::generated::common::UpdateAttestation {
    let mut wire = attestation_by("x", "signed", false);
    wire["committer"] = committer.clone();
    openehr_its::json::from_canonical_value(&wire).expect("an UPDATE_ATTESTATION")
}

/// Commits a COMPOSITION carrying one attestation by `committer`, as
/// `identity`.
async fn attest(
    svc: &FerroEhrService,
    ehr: EhrId,
    identity: CommitterIdentity,
    committer: &Value,
) -> Result<Committed, ServiceError> {
    let mut request =
        WriteRequest::new(crate::typed_body::typed(&crate::fixtures::composition("c")));
    request.attestations = vec![attestation(committer)];
    with_committer(
        Some(identity),
        Box::pin(svc.create_composition(ehr, request)),
    )
    .await
    .map_err(ServiceError::flattened)
}

#[tokio::test]
async fn only_a_bound_professional_attests() {
    let (_db, svc, ehr) = service().await;
    professional(&svc, "Dr Jane Roe", KEY).await;
    let clinician = bound(&svc, "sub-a", KEY).await;
    attest(&svc, ehr, clinician.clone(), &placeholder(ISSUER, "sub-a"))
        .await
        .expect("a bound professional attests");
    let anyone = placeholder(ISSUER, "sub-a");
    let patient = CommitterIdentity {
        person: true,
        party: None,
        bound: None,
        ..clinician.clone()
    };
    let representative = CommitterIdentity {
        representative: true,
        ..clinician.clone()
    };
    let unbound = CommitterIdentity {
        party: None,
        bound: None,
        ..clinician
    };
    let lab = system(&svc, "lab").await.identity;
    for identity in [patient, representative, unbound, lab] {
        forbidden(attest(&svc, ehr, identity, &anyone).await);
    }
}

/// Commits `body` into `ehr` as `caller`, stating `committer` in its audit.
async fn commit_stating(
    svc: &FerroEhrService,
    ehr: EhrId,
    caller: &System,
    body: &Value,
    committer: &Value,
) -> Result<Committed, ServiceError> {
    let mut request = WriteRequest::new(crate::typed_body::typed(body));
    request.audit = Some(
        openehr_its::json::from_canonical_value(&json!({
            "change_type": { "_type": "DV_CODED_TEXT", "value": "creation",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "249" } },
            "committer": committer
        }))
        .expect("an UPDATE_AUDIT"),
    );
    run(Some(caller), svc.create_composition(ehr, request))
        .await
        .map_err(ServiceError::flattened)
}

/// #5011: a system caller that states a bound person's committer is refused
/// with `403`, since a system commit records the client and never a person
/// (EHDS Art. 13(4)); the placeholder naming the client is accepted and
/// replaced by the client's committer.
#[tokio::test]
async fn a_persons_committer_stated_by_a_system_caller_is_refused() {
    let (_db, svc, ehr) = service().await;
    let (example, template) = example(&svc).await;
    let vo = professional(&svc, "Dr Jane Roe", KEY).await;
    let body = vital_signs(&example, party("Dr A", PROFESSIONALS, "hp-1"), "ward-7");
    let lab = granted(&svc, "lab", &template, &["relay"]).await;
    let person = json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jane Roe",
        "external_ref": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": vo.to_string() } },
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": KEY, "type": "oauth2" }] });
    forbidden(commit_stating(&svc, ehr, &lab, &body, &person).await);
    let committed = commit_stating(&svc, ehr, &lab, &body, &placeholder(MACHINES, "lab"))
        .await
        .expect("the placeholder naming the client");
    assert_eq!(
        stored_committer(&svc, ehr, &committed).await["identifiers"][0]["type"],
        json!("client")
    );
}
