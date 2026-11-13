// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Representatives in the patient portal (#5446), driven through the assembled
//! router over a real `FerroEhrService` on a real `PostgreSQL` 18.
//!
//! A `patients` issuer's token names its patient by the EHR of its launch
//! context. A parent signs in as the actor of the child's token (RFC 8693 §4.1
//! `act`) or under their own login naming the child's EHR; FerroEHR binds them
//! to the `PERSON` whose binding key is that login under the token's issuer
//! and admits them only while that `PERSON` holds a live representation of
//! the EHR's subject, a `PARTY_RELATIONSHIP` carrying
//! `ferroehr-operations::represents`. The patient authorises a proxy by
//! committing such a relationship of its own, and revoking it ends the access
//! on the next request. No openEHR spec governs resolving a login to a party:
//! our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::precondition::Precondition;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::kind::IssuerKind;
use http::StatusCode;
use openehr_rm::prelude::EhrStatus;
use serde_json::{Value, json};

use crate::common::{self, BASE, EXT};
use crate::party_binding_http::{
    AUDIENCE, audited_service, person, router_with, service, uid, vo, write,
};

/// The patient portal's issuer, under which a representative's login is
/// their binding key.
const PORTAL: &str = "https://portal.example";
/// The patient's login.
const CHILD_LOGIN: &str = "child-login";
/// The parent's login, the binding key of their `PERSON`.
const PARENT: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
/// The login of a parent whose representation has ended.
const FORMER: &str = "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e";
/// The login of the proxy the patient authorises.
const PROXY: &str = "3c4d5e6f-7a8b-4c9d-8e0f-2a3b4c5d6e7f";
/// The scopes of every portal token.
const SCOPES: &str = "patient/composition-*.cruds";

/// The configuration trusting `PORTAL` as a `patients` issuer that reads an
/// `act` as the patient's representative, with its `patient/` grants
/// honoured.
fn config() -> AppConfig {
    let mut auth = common::auth(AuthSection {
        issuer: vec![IssuerEntry {
            kind: Some(IssuerKind::Patients),
            jwks_file: common::jwks_file().display().to_string(),
            act_represents_patient: Some(true),
            ..common::entry(PORTAL, AUDIENCE)
        }],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let mut config = AppConfig {
        server: common::server(),
        auth,
        ..AppConfig::default()
    };
    config.auth.smart.patient_issuers =
        vec![ferrohealth_secret::secret_url::SecretUrl::new(PORTAL)];
    config
}

/// The router over `svc`.
async fn router(svc: &Arc<FerroEhrService>) -> Router {
    let source = Arc::new(svc.party_source("demographic"));
    router_with(Arc::clone(svc), config(), source).await
}

/// A portal token naming `ehr` for `sub`, with `extra` claims.
fn token(sub: &str, ehr: &str, extra: &Value) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let mut claims = json!({ "sub": sub, "iss": PORTAL, "aud": AUDIENCE, "exp": exp,
        "ehrId": ehr, "scope": SCOPES });
    if let (Some(claims), Some(extra)) = (claims.as_object_mut(), extra.as_object()) {
        claims.extend(extra.clone());
    }
    common::bearer(&claims)
}

/// The child's token, its actor `actor` when one signs in for the child.
fn acting(ehr: &str, actor: Option<&str>) -> String {
    let extra = actor.map_or_else(|| json!({}), |sub| json!({ "act": { "sub": sub } }));
    token(CHILD_LOGIN, ehr, &extra)
}

/// The status and body of `GET /ehr/{ehr}` as `credential`.
async fn read(app: &Router, ehr: &str, credential: &str) -> (StatusCode, String) {
    common::send_body(
        app,
        common::req("GET", format!("{BASE}/ehr/{ehr}"))
            .auth(credential)
            .accept("application/json")
            .build(),
    )
    .await
}

/// A `PARTY_REF` to the `PERSON` `id`.
fn person_ref(id: VoId) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() } })
}

/// A representation from `source` to `target` of the openEHR
/// `subject_relationship` code `code`, valid over `validity`: its details
/// carry `ferroehr-operations::represents`, in an archetype of the
/// deployment's own.
fn representation(source: VoId, target: VoId, code: &str, validity: Option<Value>) -> Value {
    let coded = |terminology: &str, code: &str| {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
    };
    let mut body = json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.custodian.v2",
        "name": coded("openehr", code),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002",
            "name": { "_type": "DV_TEXT", "value": "details" },
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003",
                "name": { "_type": "DV_TEXT", "value": "Representation code" },
                "value": coded("ferroehr-operations", "represents") }] },
        "source": person_ref(source),
        "target": person_ref(target)
    });
    if let Some(validity) = validity {
        body["time_validity"] = validity;
    }
    body
}

/// A `DV_INTERVAL<DV_DATE>` from `lower` to `upper`.
fn interval(lower: &str, upper: &str) -> Value {
    json!({ "_type": "DV_INTERVAL",
        "lower": { "_type": "DV_DATE", "value": lower },
        "upper": { "_type": "DV_DATE", "value": upper },
        "lower_unbounded": false, "upper_unbounded": false,
        "lower_included": true, "upper_included": true })
}

/// A `PERSON` of the mapped template carrying the binding key `key` under
/// the portal, holding `relationships`; its container.
async fn representative(svc: &FerroEhrService, key: &str, relationships: Value) -> VoId {
    let created =
        Box::pin(svc.party_create(PartyKind::Person, write(&person(PORTAL, key), "532", None)))
            .await
            .expect("the PERSON commits");
    let id = vo(&uid(&created));
    if relationships
        .as_array()
        .is_some_and(|list| !list.is_empty())
    {
        let mut body = person(PORTAL, key);
        let held: Vec<Value> = relationships
            .as_array()
            .into_iter()
            .flatten()
            .map(|relationship| {
                let mut relationship = relationship.clone();
                relationship["source"] = person_ref(id);
                relationship
            })
            .collect();
        body["relationships"] = Value::Array(held);
        let preceding = uid(&created);
        Box::pin(svc.party_update(
            PartyKind::Person,
            id,
            Precondition::parse(Some(&preceding), None).expect("an If-Match"),
            write(&body, "532", Some(&preceding)),
        ))
        .await
        .expect("the representation commits");
    }
    id
}

/// The child's `PERSON` and the EHR whose `EHR_STATUS` names it.
async fn child(svc: &FerroEhrService) -> (VoId, String) {
    let body = json!({
        "_type": "PERSON", "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" } },
        "name": { "_type": "DV_TEXT", "value": "child" },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "name" },
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "name" },
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "value": { "_type": "DV_TEXT", "value": "A child" } }] } }]
    });
    let created = Box::pin(svc.party_create(PartyKind::Person, write(&body, "532", None)))
        .await
        .expect("the child commits");
    let person = vo(&uid(&created));
    let status = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" } },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": { "_type": "PARTY_SELF", "external_ref": person_ref(person) },
        "is_queryable": true,
        "is_modifiable": true
    });
    let status = openehr_its::json::from_canonical_value::<EhrStatus>(&status).expect("a status");
    let (ehr, _) = Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(status))))
        .await
        .expect("the EHR commits");
    (person, ehr.to_string())
}

/// An EHR with no subject, which no representation reaches.
async fn other_ehr(svc: &FerroEhrService) -> String {
    let (ehr, _) = Box::pin(svc.create_ehr_meta(WriteRequest::new(None)))
        .await
        .expect("the EHR commits");
    ehr.to_string()
}

/// A parent reads the child's EHR as the actor of the child's token and
/// under their own login naming it, and reaches no other EHR either way.
#[tokio::test]
async fn a_parent_reads_the_childs_ehr_on_either_portal_path() {
    let (_db, svc) = service().await;
    let (subject, ehr) = child(&svc).await;
    representative(
        &svc,
        PARENT,
        json!([representation(subject, subject, "254", None)]),
    )
    .await;
    let elsewhere = other_ehr(&svc).await;
    let app = router(&svc).await;
    for (path, credential) in [
        ("act", acting(&ehr, Some(PARENT))),
        ("own login", token(PARENT, &ehr, &json!({}))),
    ] {
        let (status, body) = read(&app, &ehr, &credential).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        let (status, body) = read(&app, &elsewhere, &credential).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{path} reaches no other EHR: {body}"
        );
    }
    let (status, body) = read(&app, &ehr, &acting(&ehr, None)).await;
    assert_eq!(status, StatusCode::OK, "the child themself: {body}");
}

/// A representation is refused, `refused-representation-unrecorded`, when
/// no relationship records one or the one recorded has ended.
#[tokio::test]
async fn a_representation_not_recorded_or_ended_is_refused() {
    let (_db, svc) = service().await;
    let (subject, ehr) = child(&svc).await;
    let ended = interval("2010-01-01", "2020-12-31");
    representative(
        &svc,
        FORMER,
        json!([representation(subject, subject, "254", Some(ended))]),
    )
    .await;
    let app = router(&svc).await;
    for (case, credential) in [
        ("no relationship", acting(&ehr, Some(PARENT))),
        ("ended, act", acting(&ehr, Some(FORMER))),
        ("ended, own login", token(FORMER, &ehr, &json!({}))),
    ] {
        let (status, body) = read(&app, &ehr, &credential).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{case}: {body}");
        assert!(
            body.contains("refused-representation-unrecorded"),
            "{case}: {body}"
        );
    }
}

/// The patient authorises a proxy through the API: the proxy then reads the
/// patient's EHR, and the patient's revocation ends it on the next request.
/// The patient writes no representation of anyone else.
#[tokio::test]
async fn the_patient_authorises_a_proxy_and_revokes_it() {
    let (_db, svc) = service().await;
    let (subject, ehr) = child(&svc).await;
    let proxy = representative(&svc, PROXY, json!([])).await;
    let (stranger, _) = child(&svc).await;
    let app = router(&svc).await;
    let patient = acting(&ehr, None);
    let as_proxy = acting(&ehr, Some(PROXY));
    assert_eq!(read(&app, &ehr, &as_proxy).await.0, StatusCode::FORBIDDEN);

    let foreign = representation(proxy, stranger, "253", None);
    let (status, _, body) = common::send(&app, post(&patient, &foreign)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "nobody else's: {body}");

    let (status, headers, body) = common::send(
        &app,
        post(&patient, &representation(proxy, subject, "253", None)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let location = headers
        .get(http::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("a Location")
        .to_owned();
    let (status, body) = read(&app, &ehr, &as_proxy).await;
    assert_eq!(status, StatusCode::OK, "authorised: {body}");

    let uid = location.rsplit('/').next().expect("a uid").to_owned();
    let (status, _, body) = common::send(
        &app,
        common::req(
            "DELETE",
            format!("{EXT}/demographic/party_relationship/{uid}"),
        )
        .auth(&patient)
        .build(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = read(&app, &ehr, &as_proxy).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "revoked: {body}");
}

/// A representation of the patient an administrator recorded, a guardian's,
/// is not the patient's to revoke: they alter only what they inserted
/// themself (EHDS Art. 5).
#[tokio::test]
async fn the_patient_does_not_revoke_a_representation_they_did_not_insert() {
    let (_db, svc) = service().await;
    let (subject, ehr) = child(&svc).await;
    let guardian = representative(&svc, PARENT, json!([])).await;
    let mut recorded = representation(guardian, subject, "264", None);
    recorded["archetype_details"] = json!({ "_type": "ARCHETYPED", "rm_version": "1.1.0",
        "archetype_id": { "_type": "ARCHETYPE_ID",
            "value": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.custodian.v2" } });
    let created = Box::pin(svc.party_relationship_create(write::<
        openehr_rm::prelude::PartyRelationship,
    >(&recorded, "532", None)))
    .await
    .expect("the administrator records the guardian");
    let app = router(&svc).await;
    let as_guardian = acting(&ehr, Some(PARENT));
    assert_eq!(read(&app, &ehr, &as_guardian).await.0, StatusCode::OK);
    let (status, _, body) = common::send(
        &app,
        common::req(
            "DELETE",
            format!("{EXT}/demographic/party_relationship/{}", uid(&created)),
        )
        .auth(&acting(&ehr, None))
        .build(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(read(&app, &ehr, &as_guardian).await.0, StatusCode::OK);
}

/// The patient's `POST` of `relationship`, a relationship of its own.
fn post(credential: &str, relationship: &Value) -> http::Request<axum::body::Body> {
    let mut relationship = relationship.clone();
    relationship["archetype_details"] = json!({ "_type": "ARCHETYPED", "rm_version": "1.1.0",
        "archetype_id": { "_type": "ARCHETYPE_ID",
            "value": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.custodian.v2" } });
    common::req("POST", format!("{EXT}/demographic/party_relationship"))
        .auth(credential)
        .accept("application/json")
        .json(relationship.to_string())
        .build()
}

/// Every access of a portal representative is recorded bound to their
/// `PERSON`, naming the subject they represent and the relationship.
#[tokio::test]
async fn the_access_log_names_the_representative_and_the_subject() {
    let (db, _audit, svc) = audited_service().await;
    let (subject, ehr) = child(&svc).await;
    let parent = representative(
        &svc,
        PARENT,
        json!([representation(subject, subject, "254", None)]),
    )
    .await;
    let app = router(&svc).await;
    for credential in [acting(&ehr, Some(PARENT)), token(PARENT, &ehr, &json!({}))] {
        assert_eq!(read(&app, &ehr, &credential).await.0, StatusCode::OK);
    }
    let rows = recorded(&db.pool(), &parent.to_string(), 2).await;
    for (party, represented, relationship) in rows {
        assert_eq!(party.as_deref(), Some(parent.to_string().as_str()));
        assert_eq!(represented.as_deref(), Some(subject.to_string().as_str()));
        assert_eq!(relationship.as_deref(), Some("254"));
    }
}

/// The access records bound to `party`, as `(party, represented,
/// relationship)`, waiting for the drain to write `count` of them.
async fn recorded(
    pool: &sqlx::PgPool,
    party: &str,
    count: usize,
) -> Vec<(Option<String>, Option<String>, Option<String>)> {
    for _ in 0..50 {
        let rows: Vec<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT party ->> 'id', representation ->> 'represented', \
                    representation ->> 'relationship' \
             FROM audit.audit_event WHERE binding = 'bound' AND party ->> 'id' = $1 \
             ORDER BY recorded_at",
        )
        .bind(party)
        .fetch_all(pool)
        .await
        .expect("query");
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("fewer than {count} records bound to {party} were stored");
}
