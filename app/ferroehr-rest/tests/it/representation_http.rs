// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a bound caller reaches beyond its grants (#5086), driven through the
//! assembled router over a real `FerroEhrService` on a real `PostgreSQL` 18.
//!
//! A representative reaches only the EHR whose `EHR_STATUS.subject` names the
//! subject it represents, and is refused with `403`
//! (`refused-representation-unrecorded`) when the party source records no live
//! representation of the subject its token asserts. A `persons` token whose
//! only clinical grants are `patient/` is a professional confined to its
//! launch EHR, and its commit never records `PARTY_SELF`. No openEHR spec
//! governs resolving a login to a party: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::party_binding::source::BindingSource;
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::state::AppState;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::party::source::{Lookup, PartyRecord, PartySource, Query, SourceError};
use ferrohealth_smart::party::{Markers, PartyRef, PartyState, Represented};
use ferrohealth_smart::profile::Representation;
use http::StatusCode;
use openehr_rm::prelude::EhrStatus;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{AUDIENCE, ISSUER, LOGIN, refused, service, token};

/// The namespace the party sources below name every party in.
const NAMESPACE: &str = "demographic";
/// The claim naming the subject a representative acts for.
const REPRESENTS: &str = "represents";
/// The subject the representative's relationship is recorded to.
const CHILD: &str = "7a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
/// A subject no relationship of the representative names.
const STRANGER: &str = "8b2c3d4e-5f6a-4b7c-9d8e-0f1a2b3c4d5e";

/// The configuration trusting `ISSUER` for persons, whose representation
/// claim is `represents`, with its `patient/` grants honoured.
fn config() -> AppConfig {
    let mut auth = common::auth(AuthSection {
        issuer: vec![IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            representation: Some(Representation {
                claim: REPRESENTS.to_owned(),
                ground_claim: None,
            }),
            ..common::entry(ISSUER, AUDIENCE)
        }],
        party: vec![PartySection {
            issuer: ISSUER.to_owned(),
            identifier_issuer: LOGIN.to_owned(),
            identifier_type: "guid".to_owned(),
            ..PartySection::default()
        }],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let mut config = AppConfig {
        server: common::server(),
        auth,
        ..AppConfig::default()
    };
    config.auth.smart.patient_issuers = vec![ISSUER.to_owned()];
    config
}

/// The router over `svc`, looking every caller up in `source`.
async fn router(svc: Arc<FerroEhrService>, source: Arc<dyn BindingSource>) -> Router {
    let config = config();
    let authenticator = Authenticator::new(
        &config,
        None,
        Some(source),
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
    )
    .await
    .expect("the binding builds");
    let observability = Observability::default();
    observability.lifecycle.booted();
    let state = AppState::with_parts(config, svc, None, observability);
    ferroehr_rest::router::router(state, authenticator)
}

/// A party source that binds every login to one representative `PERSON`
/// whose live representation relationships name `subjects`: the local source
/// reports a relationship that has ended as none, so an empty list is the
/// ended one.
struct Recorded {
    subjects: Vec<&'static str>,
}

impl BindingSource for Recorded {}

impl PartySource for Recorded {
    fn lookup<'a>(
        &'a self,
        _query: &'a Query<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Lookup, SourceError>> + Send + 'a>> {
        let record = PartyRecord::new(
            PartyRef::new(NAMESPACE, "PERSON", "9c3d4e5f-6a7b-4c8d-8e9f-1a2b3c4d5e6f"),
            PartyState::Active,
            Markers {
                professional: false,
                representative: true,
            },
        )
        .with_represents(
            self.subjects
                .iter()
                .map(|id| Represented::new(PartyRef::new(NAMESPACE, "PERSON", *id)))
                .collect(),
        );
        Box::pin(async move { Ok(Lookup::Parties(vec![record])) })
    }
}

/// Creates an EHR whose `EHR_STATUS.subject` names the `PERSON` `subject`,
/// or no subject, returning its id.
async fn ehr(svc: &FerroEhrService, subject: Option<&str>) -> String {
    let status = subject.map(|id| {
        let status = json!({
            "_type": "EHR_STATUS",
            "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
            "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
                "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" } },
            "name": { "_type": "DV_TEXT", "value": "EHR Status" },
            "subject": { "_type": "PARTY_SELF", "external_ref": {
                "_type": "PARTY_REF", "namespace": NAMESPACE, "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": id } } },
            "is_queryable": true,
            "is_modifiable": true
        });
        openehr_its::json::from_canonical_value::<EhrStatus>(&status).expect("an EHR_STATUS")
    });
    let (ehr_id, _) = Box::pin(svc.create_ehr_meta(WriteRequest::new(status)))
        .await
        .expect("the EHR commits");
    ehr_id.to_string()
}

/// The status of `method` on `path` under the API base as `credential`.
async fn status(app: &Router, method: &str, path: &str, credential: &str) -> (StatusCode, String) {
    common::send_body(
        app,
        common::req(method, format!("{BASE}{path}"))
            .auth(credential)
            .accept("application/json")
            .build(),
    )
    .await
}

/// A representative's token asserting `subject`, with the full `user/` grants.
fn representative(subject: &str) -> String {
    token("rep-login", &json!({ REPRESENTS: subject }))
}

#[tokio::test]
async fn a_representative_reaches_only_the_represented_subjects_ehr() {
    let (_db, svc) = service().await;
    let child = ehr(&svc, Some(CHILD)).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = router(
        Arc::clone(&svc),
        Arc::new(Recorded {
            subjects: vec![CHILD],
        }),
    )
    .await;
    let credential = representative(CHILD);

    let (code, body) = status(&app, "GET", &format!("/ehr/{child}"), &credential).await;
    assert_eq!(
        code,
        StatusCode::OK,
        "the represented subject's EHR: {body}"
    );
    let (code, body) = status(&app, "GET", &format!("/ehr/{stranger}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "another subject's EHR: {body}");
    let (code, body) = status(
        &app,
        "GET",
        &format!("/ehr/{}/ehr_status", child.to_ascii_uppercase()),
        &credential,
    )
    .await;
    assert_ne!(
        code,
        StatusCode::FORBIDDEN,
        "the EHR id is matched without regard to case: {body}"
    );
    // A query that names no EHR reaches every EHR, so it is refused.
    let (code, body) = status(
        &app,
        "GET",
        "/query/aql?q=SELECT%20e%2Fehr_id%2Fvalue%20FROM%20EHR%20e",
        &credential,
    )
    .await;
    assert_eq!(
        code,
        StatusCode::FORBIDDEN,
        "a query over every EHR: {body}"
    );
    // A template is no patient's data.
    let (code, body) = status(&app, "GET", "/definition/template/adl1.4", &credential).await;
    assert_eq!(code, StatusCode::OK, "the template list: {body}");
}

#[tokio::test]
async fn a_representation_the_source_does_not_record_is_refused() {
    let (_db, svc) = service().await;
    ehr(&svc, Some(CHILD)).await;
    let app = router(
        Arc::clone(&svc),
        Arc::new(Recorded {
            subjects: vec![CHILD],
        }),
    )
    .await;
    refused(
        &app,
        &representative(STRANGER),
        "refused-representation-unrecorded",
    )
    .await;
    // A relationship that has ended is no longer reported.
    let ended = router(Arc::clone(&svc), Arc::new(Recorded { subjects: vec![] })).await;
    refused(
        &ended,
        &representative(CHILD),
        "refused-representation-unrecorded",
    )
    .await;
}

/// #5011: a person login, one asserting no represented subject, on a row
/// that carries only the representative marker is refused with `403`
/// (`refused-wrong-marker`): a representative's `PERSON` never binds a
/// professional.
#[tokio::test]
async fn a_person_login_on_a_representative_row_is_refused_as_the_wrong_marker() {
    let (_db, svc) = service().await;
    let app = router(
        Arc::clone(&svc),
        Arc::new(Recorded {
            subjects: vec![CHILD],
        }),
    )
    .await;
    refused(
        &app,
        &token("rep-login", &json!({})),
        "refused-wrong-marker",
    )
    .await;
}

#[tokio::test]
async fn a_persons_token_with_patient_grants_reaches_its_launch_ehr_and_never_commits_as_party_self()
 {
    let (_db, svc) = service().await;
    let key = "0c1d2e3f-4a5b-4c6d-8e7f-9a0b1c2d3e4f";
    crate::party_binding_http::professional(&svc, key, "532").await;
    let launched = ehr(&svc, None).await;
    let other = ehr(&svc, None).await;
    let source = Arc::new(svc.party_source(NAMESPACE));
    let app = router(Arc::clone(&svc), source).await;
    let credential = token(
        key,
        &json!({ "scope": "patient/composition-*.cru", "ehrId": launched }),
    );

    let (code, body) = status(&app, "GET", &format!("/ehr/{launched}"), &credential).await;
    assert_eq!(code, StatusCode::OK, "the launch EHR: {body}");
    let (code, body) = status(&app, "GET", &format!("/ehr/{other}"), &credential).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "another EHR: {body}");

    let (code, headers, body) = common::send(
        &app,
        common::req("GET", format!("{BASE}/ehr/{launched}/ehr_status"))
            .auth(&credential)
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "the launch EHR's status: {body}");
    let current = common::etag_uid(&headers);
    let (code, _, body) = common::send(
        &app,
        common::req("PUT", format!("{BASE}/ehr/{launched}/ehr_status"))
            .auth(&credential)
            .accept("application/json")
            .header("prefer", "return=minimal")
            .if_match(format!("\"{current}\""))
            .json(body.clone())
            .build(),
    )
    .await;
    assert!(code.is_success(), "the status update: {code} {body}");
    let (code, body) = status(
        &app,
        "GET",
        &format!("/ehr/{launched}/versioned_ehr_status/version"),
        &credential,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "the latest status version: {body}");
    let version: Value = serde_json::from_str(&body).expect("a version");
    let committer = &version["commit_audit"]["committer"];
    assert_ne!(committer["_type"], "PARTY_SELF", "{committer}");
    assert_eq!(committer["_type"], "PARTY_IDENTIFIED", "{committer}");
}
