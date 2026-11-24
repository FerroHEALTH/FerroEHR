// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What each local caller kind may do (#5100), driven through the assembled
//! router over a real `FerroEhrService` on a real `PostgreSQL` 18: a system
//! caller holds `system/` scopes only and commits only as a client the policy
//! parties grant `commit_composition` (#5436), and a person a token names
//! beside an actor is bound to their own `PERSON`.
//!
//! RFC 9068 §2.2 and §5 name the person in `sub` and mark an app-only token;
//! RFC 8693 §4.1 names an actor in `act`; SMART on openEHR
//! `master08-scopes.adoc` §Resource Scopes gives the `patient`, `user` and
//! `system` contexts. What a caller kind may do here is our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::state::AppState;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::party::config::PartySection;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{
    AUDIENCE, ISSUER, LOGIN, audited_service, list, professional, refusal_rows, refused, service,
};

/// The standards-only machines issuer.
const MACHINES: &str = "https://machines.example";
/// The actor a delegated token names.
const GATEWAY: &str = "gateway";
/// The SMART scopes of a backend client that reads templates and writes EHRs.
const SYSTEM_SCOPES: &str = "system/composition-*.cruds system/template-*.r";

/// The audience of the machines issuer `issuer`: each issuer has its own.
pub(crate) fn audience(issuer: &str) -> String {
    format!("{AUDIENCE}-{}", issuer.trim_start_matches("https://"))
}

/// An issuer of client credentials tokens at `issuer`.
pub(crate) fn machines(issuer: &str) -> IssuerEntry {
    IssuerEntry {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer.to_owned()),
        audience: vec![audience(issuer)],
        jwks_file: common::jwks_file().display().to_string(),
        ..IssuerEntry::default()
    }
}

/// The configuration trusting the bound persons issuer and the machines
/// issuer.
fn config() -> AppConfig {
    let persons = IssuerEntry {
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(ISSUER, AUDIENCE)
    };
    let mut auth = common::auth(AuthSection {
        issuer: common::bound(
            vec![persons, machines(MACHINES)],
            vec![PartySection {
                issuer: ferrohealth_secret::secret_url::SecretUrl::new(ISSUER),
                identifier_issuer: LOGIN.to_owned(),
                identifier_type: "guid".to_owned(),
                ..PartySection::default()
            }],
        ),
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    AppConfig {
        server: common::server(),
        auth,
        ..AppConfig::default()
    }
}

/// The router over `svc` under `config`.
async fn router(svc: Arc<FerroEhrService>, config: AppConfig) -> Router {
    common::open_configured_key_spaces(&config).await;
    let source = Arc::new(svc.party_source("demographic"));
    let authenticator = Authenticator::new(
        &config,
        Some(source),
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
    )
    .await
    .expect("the authenticator builds");
    let observability = Observability::default();
    observability.lifecycle.booted();
    let state = AppState::with_parts(config, svc, None, observability);
    ferroehr_rest::router::router(state, authenticator)
}

/// A bearer of `issuer` for `audience` carrying `claims`.
pub(crate) fn bearer(issuer: &str, audience: &str, claims: &Value) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    let mut all = json!({ "iss": issuer, "aud": audience, "exp": exp });
    if let (Some(all), Some(claims)) = (all.as_object_mut(), claims.as_object()) {
        all.extend(claims.clone());
    }
    common::bearer(&all)
}

/// An app-only token of the machines issuer for the client `lab`, carrying
/// `scope`.
fn app_only(scope: &str) -> [(&'static str, String); 1] {
    [(
        "rfc9068",
        bearer(
            MACHINES,
            &audience(MACHINES),
            &json!({ "sub": "lab", "client_id": "lab", "scope": scope }),
        ),
    )]
}

/// Creates an EHR as `credential`; returns the status and body.
async fn create_ehr(app: &Router, credential: &str) -> (StatusCode, String) {
    common::send_body(
        app,
        common::req("POST", format!("{BASE}/ehr"))
            .auth(credential)
            .prefer_representation()
            .build(),
    )
    .await
}

#[tokio::test]
async fn a_system_caller_holds_system_scopes_only() {
    let (_db, svc) = service().await;
    let app = router(svc, config()).await;
    for (profile, credential) in app_only("user/template-*.r") {
        let (status, body) = list(&app, &credential).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{profile}: {body}");
        assert!(body.contains("system/ scopes only"), "{profile}: {body}");
    }
    for (profile, credential) in app_only("patient/template-*.r") {
        assert_eq!(
            list(&app, &credential).await.0,
            StatusCode::FORBIDDEN,
            "{profile}"
        );
    }
    for (profile, credential) in app_only(SYSTEM_SCOPES) {
        let (status, body) = list(&app, &credential).await;
        assert_eq!(status, StatusCode::OK, "{profile} reads: {body}");
    }
}

/// The client `lab` of the machines issuer, granted to commit and acting
/// for the provider `90000001`, as the policy parties name it (#5436); the
/// container of its `AGENT`.
async fn grant_lab(svc: &FerroEhrService) -> ferroehr::ids::VoId {
    crate::system_grants::system_client(
        svc,
        &[(MACHINES, "lab")],
        Some(("urn:oid:2.16.528.1.1007.3.3", "90000001")),
        &[crate::system_grants::relays(
            "Lab result",
            ("urn:oid:2.999.1", "ward-7"),
        )],
    )
    .await
}

#[tokio::test]
async fn only_a_granted_system_client_commits() {
    let (_db, svc) = service().await;
    let app = router(Arc::clone(&svc), config()).await;
    for (profile, credential) in app_only(SYSTEM_SCOPES) {
        let (status, body) = create_ehr(&app, &credential).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{profile}: {body}");
    }
    grant_lab(&svc).await;
    for (profile, credential) in app_only(SYSTEM_SCOPES) {
        let (status, body) = create_ehr(&app, &credential).await;
        assert_eq!(status, StatusCode::CREATED, "{profile}: {body}");
    }
}

#[tokio::test]
async fn a_delegated_person_commits_as_their_own_person() {
    let (_db, svc) = service().await;
    let key = "8d9e0f1a-2b3c-4d4e-9f5a-6b7c8d9e0f1a";
    professional(&svc, key, "532").await;
    let app = router(Arc::clone(&svc), config()).await;
    let delegated = bearer(
        ISSUER,
        AUDIENCE,
        &json!({ "sub": key, "client_id": "portal", "act": { "sub": GATEWAY } }),
    );
    let (status, body) = create_ehr(&app, &delegated).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ehr: Value = serde_json::from_str(&body).expect("an EHR");
    let ehr_id = ehr
        .pointer("/ehr_id/value")
        .and_then(Value::as_str)
        .expect("the EHR id")
        .to_owned();
    let (status, version) = common::send_json(
        &app,
        common::req(
            "GET",
            format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status/version"),
        )
        .auth(&delegated)
        .accept("application/json")
        .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{version}");
    let committer = &version["commit_audit"]["committer"];
    assert_eq!(committer["external_ref"]["type"], "PERSON", "{committer}");
    assert_eq!(committer["identifiers"][0]["id"], key, "{committer}");
}

/// A `RelatedPerson`, someone acting for the patient in their launch context
/// (EHDS Art. 4(2)), whose login no party binds is refused `403`
/// (`refused-unbound`): a representation is read from the store alone.
#[tokio::test]
async fn a_related_person_no_party_binds_is_refused() {
    let (_db, svc) = service().await;
    let app = router(Arc::clone(&svc), config()).await;
    let token = bearer(
        ISSUER,
        AUDIENCE,
        &json!({ "sub": "c3d4e5f6-a7b8-4c9d-8e0f-1a2b3c4d5e6f", "client_id": "portal",
                 "fhirUser": "RelatedPerson/carer-7", "patient": "patient-1",
                 "scope": "patient/template-*.r" }),
    );
    refused(&app, &token, "refused-unbound").await;
}

/// A system caller refused for a personal scope is recorded under
/// `refused-system-personal-scope`, once per refused request (#5140).
#[tokio::test]
async fn a_system_caller_with_a_personal_scope_is_recorded_with_its_code() {
    let (db, _audit, svc) = audited_service().await;
    let app = router(svc, config()).await;
    let tokens = app_only("user/template-*.r");
    for (profile, credential) in &tokens {
        assert_eq!(
            list(&app, credential).await.0,
            StatusCode::FORBIDDEN,
            "{profile}"
        );
    }
    let rows = refusal_rows(&db.pool(), "refused-system-personal-scope", tokens.len()).await;
    assert_eq!(rows.len(), tokens.len(), "one record per refused request");
}

/// The `(binding, acting_mode, client_id, provider id)` of a stored record.
type Stored = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The `(binding, acting_mode, client_id, provider id)` of the stored records
/// of `operation` whose principal is `principal`, waiting for `count`.
async fn stored_rows(
    pool: &sqlx::PgPool,
    operation: &str,
    principal: &str,
    count: usize,
) -> Vec<Stored> {
    for _ in 0..50 {
        let rows: Vec<Stored> = sqlx::query_as(
            "SELECT binding, acting_mode, client_id, provider ->> 'id' \
                 FROM audit.access_trail WHERE operation = $1 AND principal = $2 \
                 ORDER BY recorded_at",
        )
        .bind(operation)
        .bind(principal)
        .fetch_all(pool)
        .await
        .expect("query");
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("fewer than {count} {operation} records of {principal} were stored");
}

/// Every access record of a system caller carries `binding = system`, the
/// acting mode `system`, the client, its `AGENT` as the party and the
/// healthcare provider that `AGENT` acts for in the policy parties (EHDS
/// Annex II 3.2(a), "identification of the healthcare provider"; the fields
/// are our own design, #5008).
#[tokio::test]
async fn a_system_callers_records_carry_its_client_and_provider() {
    let (db, _audit, svc) = audited_service().await;
    let agent = grant_lab(&svc).await;
    let app = router(svc, config()).await;
    let tokens = app_only(SYSTEM_SCOPES);
    for (profile, credential) in &tokens {
        let (status, body) = create_ehr(&app, credential).await;
        assert_eq!(status, StatusCode::CREATED, "{profile}: {body}");
    }
    for principal in ["lab"] {
        let rows = stored_rows(&db.pool(), "ehr_create", principal, 1).await;
        assert_eq!(
            rows,
            vec![(
                Some("system".to_owned()),
                Some("system".to_owned()),
                Some("lab".to_owned()),
                Some("90000001".to_owned())
            )],
            "{principal}"
        );
        let party: Option<Value> = sqlx::query_scalar(
            "SELECT party FROM audit.access_trail WHERE operation = 'ehr_create' AND principal = $1",
        )
        .bind(principal)
        .fetch_one(&db.pool())
        .await
        .expect("the record's party");
        assert_eq!(
            party,
            Some(json!({ "namespace": "demographic", "type": "ACTOR", "id": agent.to_string() })),
            "the record names the client's AGENT: {principal}"
        );
    }
}

/// A client-credentials client's write and its read are recorded under one
/// client identity: every access record of the two carries the same binding,
/// acting mode, client, provider and `AGENT`, and the access-log entries of
/// the write and the read name the same accessor (EHDS Annex II 3.2; #6130).
#[tokio::test]
async fn a_system_clients_write_and_read_are_recorded_under_one_client_identity() {
    type Trail = (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<Value>,
    );
    let (db, _audit, svc) = audited_service().await;
    let agent = grant_lab(&svc).await;
    let app = router(svc, config()).await;
    let [(_, credential), ..] = app_only(SYSTEM_SCOPES);
    let (status, body) = create_ehr(&app, &credential).await;
    assert_eq!(status, StatusCode::CREATED, "the write: {body}");
    let created: Value = serde_json::from_str(&body).expect("the EHR");
    let ehr = created["ehr_id"]["value"].as_str().expect("an EHR id");
    let (status, body) = common::send_body(
        &app,
        common::req("GET", format!("{BASE}/ehr/{ehr}"))
            .auth(&credential)
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the read: {body}");
    let mut rows: Vec<Trail> = Vec::new();
    for _ in 0..50 {
        rows = sqlx::query_as(
            "SELECT operation, binding, acting_mode, client_id, provider ->> 'id', party \
             FROM audit.access_trail WHERE principal = 'lab' ORDER BY recorded_at",
        )
        .fetch_all(&db.pool())
        .await
        .expect("the access trail");
        if rows.len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        rows.len() >= 2,
        "the write and the read are recorded: {rows:?}"
    );
    let identity = |row: &Trail| {
        (
            row.1.clone(),
            row.2.clone(),
            row.3.clone(),
            row.4.clone(),
            row.5.clone(),
        )
    };
    let party = json!({ "namespace": "demographic", "type": "ACTOR", "id": agent.to_string() });
    for row in &rows {
        assert_eq!(
            identity(row),
            (
                Some("system".to_owned()),
                Some("system".to_owned()),
                Some("lab".to_owned()),
                Some("90000001".to_owned()),
                Some(party.clone())
            ),
            "{row:?}"
        );
    }
    let operations: std::collections::BTreeSet<&str> =
        rows.iter().map(|row| row.0.as_str()).collect();
    assert!(operations.len() >= 2, "a write and a read: {operations:?}");
    let entries = crate::access_log_http::entries(&db.pool(), 2, |stored| {
        stored.ehr_id.as_deref() == Some(ehr) && matches!(stored.action.as_deref(), Some("C" | "R"))
    })
    .await;
    let accessors: Vec<Value> = entries
        .iter()
        .map(|stored| {
            serde_json::from_str::<Value>(&stored.content).expect("an entry")["accessor"].clone()
        })
        .collect();
    let actions: std::collections::BTreeSet<Option<&str>> = entries
        .iter()
        .map(|stored| stored.action.as_deref())
        .collect();
    assert_eq!(
        actions.len(),
        2,
        "a write entry and a read entry: {accessors:?}"
    );
    assert!(
        accessors.windows(2).all(|pair| pair[0] == pair[1]),
        "one accessor: {accessors:#?}"
    );
}

/// An RFC 7523 JWT-bearer onward token names the person in `sub` and the
/// middle-tier client in its client claim, with no `act`: the person is
/// bound and commits as their own `PERSON`, and the record carries the
/// person's mode and the client (#5008).
#[tokio::test]
async fn a_jwt_bearer_onward_token_commits_as_the_person_and_records_its_client() {
    let (db, _audit, svc) = audited_service().await;
    let key = "b1c2d3e4-f5a6-4b7c-8d9e-0f1a2b3c4d5e";
    professional(&svc, key, "532").await;
    let app = router(Arc::clone(&svc), config()).await;
    let onward = bearer(
        ISSUER,
        AUDIENCE,
        &json!({ "sub": key, "client_id": "bridge" }),
    );
    let (status, body) = create_ehr(&app, &onward).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ehr: Value = serde_json::from_str(&body).expect("an EHR");
    let ehr_id = ehr
        .pointer("/ehr_id/value")
        .and_then(Value::as_str)
        .expect("the EHR id")
        .to_owned();
    let (status, version) = common::send_json(
        &app,
        common::req(
            "GET",
            format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status/version"),
        )
        .auth(&onward)
        .accept("application/json")
        .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{version}");
    let committer = &version["commit_audit"]["committer"];
    assert_eq!(committer["external_ref"]["type"], "PERSON", "{committer}");
    assert_eq!(committer["identifiers"][0]["id"], key, "{committer}");
    let rows = stored_rows(&db.pool(), "ehr_create", key, 1).await;
    assert_eq!(rows[0].1.as_deref(), Some("person"), "the person's mode");
    assert_eq!(rows[0].2.as_deref(), Some("bridge"), "the client");
}
