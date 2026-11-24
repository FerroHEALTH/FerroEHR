// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login of a record's person (#6106): a `persons` caller whose one
//! `PERSON` carries no professional and no representative marker, which the
//! shared binding answers `wrong_marker`. FerroEHR admits it as the person
//! themself (EHDS Art. 3): it finds and reads the EHRs whose
//! `EHR_STATUS.subject` names that `PERSON`, as their subject, and nothing
//! else, with no role and no professional right. No openEHR spec governs
//! resolving a login to a party: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::party_binding::source::BindingSource;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::state::AppState;
use ferrohealth_smart::party::source::{Lookup, PartyRecord, PartySource, Query, SourceError};
use ferrohealth_smart::party::{Markers, PartyRef, PartyState};
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{LOGIN, person, service, token, uid, vo, write};
use crate::representation_http::{STRANGER, config, ehr, router};

/// The `PERSON` every login of [`Patient`] binds.
const PERSON: &str = "4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f7a";

/// A party source that binds every login to one `PERSON` holding neither
/// marker: a patient's.
struct Patient;

impl BindingSource for Patient {}

impl PartySource for Patient {
    fn lookup<'a>(
        &'a self,
        _query: &'a Query<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Lookup, SourceError>> + Send + 'a>> {
        let record = PartyRecord::new(
            PartyRef::new("demographic", "PERSON", PERSON),
            PartyState::Active,
            Markers::default(),
        );
        Box::pin(async move { Ok(Lookup::Parties(vec![record])) })
    }
}

/// `method` on `path` under the API base as `credential`, with `body`.
async fn send(
    app: &Router,
    (method, path): (&str, &str),
    credential: &str,
    body: Option<&Value>,
) -> (StatusCode, String) {
    let req = common::req(method, format!("{BASE}{path}"))
        .auth(credential)
        .accept("application/json");
    let req = match body {
        Some(body) => req.json(body.to_string()).build(),
        None => req.build(),
    };
    common::send_body(app, req).await
}

/// The lookup of the EHR whose subject is the `PERSON` `person`.
fn lookup(person: &str) -> String {
    format!("/ehr?subject_id={person}&subject_namespace=demographic")
}

#[tokio::test]
async fn a_patients_own_login_finds_and_reads_their_own_ehr() {
    let (_db, svc) = service().await;
    let own = ehr(&svc, Some(PERSON)).await;
    let app = router(Arc::clone(&svc), Arc::new(Patient)).await;
    let credential = token("eva-login", &json!({}));

    let (code, body) = send(&app, ("GET", &lookup(PERSON)), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "the lookup of her own EHR: {body}");
    let found: Value = serde_json::from_str(&body).expect("an EHR");
    assert_eq!(found["ehr_id"]["value"], json!(own), "{body}");
    let (code, body) = send(&app, ("GET", &format!("/ehr/{own}")), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "her own EHR: {body}");
}

#[tokio::test]
async fn a_patients_own_login_holds_no_professional_right() {
    let (_db, svc) = service().await;
    ehr(&svc, Some(PERSON)).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = router(Arc::clone(&svc), Arc::new(Patient)).await;
    let credential = token("eva-login", &json!({ "roles": ["CLINICIAN", "ADMIN"] }));

    let (code, body) = send(&app, ("GET", &lookup(STRANGER)), &credential, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND, "another person's EHR: {body}");
    let (code, body) = send(
        &app,
        ("GET", &format!("/ehr/{stranger}")),
        &credential,
        None,
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "another EHR by id: {body}");
    assert!(body.contains("refused-not-subject"), "{body}");
    let aql = json!({ "q": "SELECT e/ehr_id/value FROM EHR e" });
    for (method, path, body) in [("POST", "/ehr", None), ("POST", "/query/aql", Some(&aql))] {
        let (code, answer) = send(&app, (method, path), &credential, body).await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{method} {path}: {answer}");
        assert!(
            answer.contains("refused-not-subject"),
            "{method} {path}: {answer}"
        );
    }
}

/// Commits a `PERSON` carrying the login `key` and no professional `ROLE`
/// through the service; its id.
async fn plain_person(svc: &FerroEhrService, key: &str) -> String {
    let created =
        Box::pin(svc.party_create(PartyKind::Person, write(&person(LOGIN, key), "532", None)))
            .await
            .expect("the PERSON commits");
    vo(&uid(&created)).to_string()
}

/// A `PERSON` with no professional `ROLE` binds through the local party
/// source, which holds its key, and reads its own EHR as its subject.
#[tokio::test]
async fn a_person_with_no_role_binds_through_the_local_source_to_their_own_ehr() {
    let (_db, svc) = service().await;
    let key = "5e6f7a8b-9c0d-4e1f-8a2b-3c4d5e6f7a8b";
    let person = plain_person(&svc, key).await;
    let own = ehr(&svc, Some(&person)).await;
    let app = router(Arc::clone(&svc), Arc::new(svc.party_source("demographic"))).await;
    let credential = token(key, &json!({}));
    let (code, body) = send(&app, ("GET", &lookup(&person)), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "the lookup of their own EHR: {body}");
    let (code, body) = send(&app, ("GET", &format!("/ehr/{own}")), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "their own EHR: {body}");
}

/// A `PERSON` with no professional `ROLE` and no EHR reaches nothing: every
/// request is still decided, and refused.
#[tokio::test]
async fn a_person_of_no_ehr_reads_no_ehr() {
    let (_db, svc) = service().await;
    let key = "6f7a8b9c-0d1e-4f2a-9b3c-4d5e6f7a8b9c";
    plain_person(&svc, key).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = router(Arc::clone(&svc), Arc::new(svc.party_source("demographic"))).await;
    let credential = token(key, &json!({}));
    let (code, body) = send(
        &app,
        ("GET", &format!("/ehr/{stranger}")),
        &credential,
        None,
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("refused-not-subject"), "{body}");
    let (code, body) = send(&app, ("GET", &lookup(STRANGER)), &credential, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND, "{body}");
}

/// The router of [`router`] with the RBAC gate on, as every deployment that
/// grants by role runs it.
async fn rbac_router(svc: Arc<FerroEhrService>, source: Arc<dyn BindingSource>) -> Router {
    let config = config();
    common::open_configured_key_spaces(&config).await;
    let authenticator = Authenticator::new(
        &config,
        Some(source),
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
    )
    .await
    .expect("the binding builds");
    let observability = Observability::default();
    observability.lifecycle.booted();
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = AuthzHandle::build(&authz, &config.api.base_path).map(Arc::new);
    let state = AppState::with_parts(config, svc, authz, observability);
    ferroehr_rest::router::router(state, authenticator)
}

/// Under the RBAC gate the person themself, who holds no role, still finds
/// and reads their own EHR: their right is who they are (EHDS Art. 3), and
/// the gate admits them as it admits the patient's token; another EHR stays
/// refused.
#[tokio::test]
async fn the_rbac_gate_admits_the_person_themself_to_their_own_ehr() {
    let (_db, svc) = service().await;
    let own = ehr(&svc, Some(PERSON)).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = rbac_router(Arc::clone(&svc), Arc::new(Patient)).await;
    let credential = token("eva-login", &json!({}));

    let (code, body) = send(&app, ("GET", &lookup(PERSON)), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "the lookup of her own EHR: {body}");
    let (code, body) = send(&app, ("GET", &format!("/ehr/{own}")), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "her own EHR: {body}");
    let (code, body) = send(
        &app,
        ("GET", &format!("/ehr/{stranger}")),
        &credential,
        None,
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "another EHR: {body}");
    assert!(body.contains("refused-not-subject"), "{body}");
}

/// The SMART app an openEHR launch reaches the CDR through.
const APP: &str = "hospital-app";

/// A patient's login through a SMART app of their issuer is the person
/// themself: an openEHR launch reaches the CDR through such an app (#6080). They read their own EHR and
/// write its settings, and any other EHR stays refused.
#[tokio::test]
async fn a_patients_login_through_an_app_reads_their_own_ehr() {
    let (_db, svc) = service().await;
    let own = ehr(&svc, Some(PERSON)).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = router(Arc::clone(&svc), Arc::new(Patient)).await;
    let credential = token("eva-login", &json!({ "client_id": APP }));

    let (code, body) = send(&app, ("GET", &format!("/ehr/{own}")), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "her own EHR through the app: {body}");
    let (code, body) = send(&app, ("GET", &lookup(PERSON)), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "the lookup of her own EHR: {body}");
    let (code, body) = send(
        &app,
        ("GET", &format!("/ehr/{stranger}")),
        &credential,
        None,
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "another EHR: {body}");
    assert!(body.contains("refused-not-subject"), "{body}");
}
