// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The patient's own token that FerroSMART's RFC 8693 token exchange issues
//! to FerroBRIDGE for the xShare Yellow Button (#6420), over the real axum
//! app under the `ferrosmart` issuer preset.
//!
//! The token carries the subject token's `sub`, `fhirUser`, `patient`, `acr`
//! and `auth_time`, the `ehrId` of the patient's EHR, the exchanging client
//! as `client_id`, no `act`, and `patient/composition-*.r patient/aql-*.s` at
//! most. `ferrohealth-smart` reads it as the patient through a client the
//! CDR grants `act_for_persons`. FerroEHR holds it to its subject's own EHR
//! (EHDS Art. 3), withholds what the Art. 3(3) access delay holds, refuses it
//! below the `[authz.assurance]` floor with the RFC 9470 §3 step-up
//! challenge, and records the person, the client and the level (Annex II
//! 3.2(b), `docs/law/eu/ehds/text.html`).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::party_binding::grants::operation::Operation;
use ferroehr::policy_register::Register;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr::system_log::store::chains::AuditChain;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use ferrohealth_smart::assurance::{AssuranceLevel, AssuranceSection};
use ferrohealth_smart::config::IssuerEntry;
use ferrohealth_smart::profile::Preset;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

use crate::common::{self, BASE, EXT};

/// FerroSMART, trusted in one `persons` table under its preset.
const ISSUER: &str = "https://smart.example";
/// The openEHR base FerroSMART pairs with FerroBRIDGE's FHIR base.
const AUDIENCE: &str = "ferroehr";
/// FerroBRIDGE's confidential client at FerroSMART.
const BRIDGE: &str = "ferrobridge";
/// The professional who records the patient summary.
const PROFESSIONAL: &str = "dr-jansen";
/// The patient's pairwise subject at FerroSMART.
const PATIENT: &str = "pairwise-eva";
/// The eIDAS levels of assurance FerroSMART writes into `acr`.
const LOW: &str = "http://eidas.europa.eu/LoA/low";
const SUBSTANTIAL: &str = "http://eidas.europa.eu/LoA/substantial";
const HIGH: &str = "http://eidas.europa.eu/LoA/high";
/// The code system of the EHDS Art. 14(1) priority categories, HL7 Europe's
/// `EEHRxFDocumentPriorityCategoryCS`.
const PRIORITY: &str =
    "http://hl7.eu/fhir/health-data-api/CodeSystem/eehrxf-document-priority-category-cs";

/// `[authz.assurance]` at the floor the xShare Bronze declaration names.
fn assurance() -> AssuranceSection {
    AssuranceSection {
        levels: [
            (LOW.to_owned(), AssuranceLevel::Low),
            (SUBSTANTIAL.to_owned(), AssuranceLevel::Substantial),
            (HIGH.to_owned(), AssuranceLevel::High),
        ]
        .into_iter()
        .collect(),
        minimum: Some(AssuranceLevel::Substantial),
        ..AssuranceSection::default()
    }
}

/// What a test reaches: the router, the audit pool, the patient's `PERSON`,
/// their EHR with the committed summary's object id, and another person's
/// EHR.
struct World {
    _db: testkit::TestDb,
    pool: PgPool,
    app: Router,
    person: uuid::Uuid,
    own: String,
    summary: String,
    other: String,
}

/// A deployment behind FerroSMART with the floor at substantial, the access
/// delay on for patient summaries when `delayed`, FerroBRIDGE granted
/// `act_for_persons`, the patient's and the professional's logins bound, and
/// a patient summary committed into the patient's EHR.
async fn world(delayed: bool) -> World {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle): (_, AuditHandle) = start(common::local_audit(), None, pool.clone())
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone())),
    );
    if delayed {
        svc.put_policy_register(
            Register::AccessDelay,
            0,
            r#"{"enabled": true, "categories": ["Patient-Summaries"], "max_delay": "72h"}"#,
        )
        .await
        .expect("turn the access delay on");
    }
    svc.mirror_category_map()
        .await
        .expect("mirror the category map");
    let mut config = common::api_config(true);
    config.auth = common::trusting(vec![IssuerEntry {
        preset: Preset::Ferrosmart,
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(ISSUER, AUDIENCE)
    }]);
    config.assurance = assurance();
    common::bind_issuer(&mut config, ISSUER);
    config.rbac.role_issuers = vec![common::name_of(ISSUER)];
    common::bind_subjects(&pool, &[(ISSUER, PROFESSIONAL)]).await;
    let person = patient_person(&pool).await;
    common::grant_standing(&svc, ISSUER, BRIDGE, Operation::ActForPersons).await;
    let mut rbac = AuthzConfig::default();
    rbac.rbac.enabled = true;
    let authz = AuthzHandle::build(&rbac, &config.api.base_path).map(Arc::new);
    let app = common::build_full(config, Arc::clone(&svc), authz, Observability::default()).await;
    let (status, body) = send(
        &app,
        ("POST", &format!("{BASE}/definition/template/adl1.4")),
        &professional(),
        Some(("application/xml", common::ips_opt_xml())),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    let own = ehr(&app, Some(&common::person_status(person))).await;
    let summary = commit(&app, &own).await;
    let other = ehr(&app, None).await;
    World {
        _db: db,
        pool,
        app,
        person,
        own,
        summary,
        other,
    }
}

/// Commits the patient's `PERSON`, carrying their FerroSMART login and no
/// professional `ROLE`; its container id.
async fn patient_person(pool: &PgPool) -> uuid::Uuid {
    let logins = common::logins_of(ISSUER);
    common::open_key_spaces(pool, &[(logins.as_str(), false)]).await;
    let created = Box::pin(common::binding_service(pool).party_create(
        PartyKind::Person,
        crate::party_binding_http::write(
            &crate::party_binding_http::person(&logins, PATIENT),
            "532",
            None,
        ),
    ))
    .await
    .expect("the patient's PERSON commits");
    crate::party_binding_http::vo(&crate::party_binding_http::uid(&created)).0
}

/// The professional's FerroSMART token, through the first-party client.
fn professional() -> String {
    common::bearer(
        &json!({ "sub": PROFESSIONAL, "iss": ISSUER, "aud": AUDIENCE,
        "exp": jiff::Timestamp::now().as_second() + 3600,
        "roles": ["ADMIN", "USER"], "acr": SUBSTANTIAL }),
    )
}

/// The token FerroSMART's exchange issues FerroBRIDGE for the patient
/// launched for `ehr`, authenticated at `acr`.
fn exchanged(ehr: &str, acr: &str) -> String {
    let now = jiff::Timestamp::now().as_second();
    common::bearer(&json!({
        "sub": PATIENT, "iss": ISSUER, "aud": AUDIENCE, "exp": now + 300,
        "client_id": BRIDGE,
        "scope": "patient/composition-*.r patient/aql-*.s",
        "fhirUser": "https://bridge.example/fhir/Patient/p-eva",
        "patient": "p-eva",
        "ehrId": ehr,
        "acr": acr,
        "auth_time": now - 60,
    }))
}

/// Sends `method uri` with `credential` and an optional body.
async fn send(
    app: &Router,
    (method, uri): (&str, &str),
    credential: &str,
    body: Option<(&str, String)>,
) -> (StatusCode, String) {
    let (status, _headers, body) = send_full(app, (method, uri), credential, body).await;
    (status, body)
}

/// [`send`], with the response headers.
async fn send_full(
    app: &Router,
    (method, uri): (&str, &str),
    credential: &str,
    body: Option<(&str, String)>,
) -> (StatusCode, header::HeaderMap, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, credential)
        .header(header::ACCEPT, "application/json")
        .header("Prefer", "return=representation");
    let req = match body {
        Some((content_type, body)) => req
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body)),
        None => req.body(Body::empty()),
    }
    .expect("request");
    common::send(app, req).await
}

/// Creates an EHR as the professional, with the `EHR_STATUS` `status` when
/// one is given; its id.
async fn ehr(app: &Router, status: Option<&Value>) -> String {
    let body = status.map(|status| ("application/json", status.to_string()));
    let (code, body) = send(app, ("POST", &format!("{BASE}/ehr")), &professional(), body).await;
    assert_eq!(code, StatusCode::CREATED, "EHR create: {body}");
    serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned()
}

/// Commits the IPS composition into `ehr` as the professional; its object id.
async fn commit(app: &Router, ehr: &str) -> String {
    let mut composition = common::ips_canonical_composition();
    composition["composer"] = json!({ "_type": "PARTY_IDENTIFIED",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": PROFESSIONAL }] });
    let (status, headers, body) = send_full(
        app,
        ("POST", &format!("{BASE}/ehr/{ehr}/composition")),
        &professional(),
        Some(("application/json", composition.to_string())),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .split("::")
        .next()
        .expect("object id")
        .to_owned()
}

/// The status and body of a read of the composition `vo` of `ehr`.
async fn read(app: &Router, ehr: &str, vo: &str, credential: &str) -> (StatusCode, String) {
    send(
        app,
        ("GET", &format!("{BASE}/ehr/{ehr}/composition/{vo}")),
        credential,
        None,
    )
    .await
}

/// The composition uids an AQL query scoped to `ehr` returns.
async fn queried(app: &Router, ehr: &str, credential: &str) -> Vec<String> {
    let q = urlencoding::encode("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c");
    let (status, body) = send(
        app,
        ("GET", &format!("{BASE}/query/aql?q={q}&ehr_id={ehr}")),
        credential,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "query: {body}");
    serde_json::from_str::<Value>(&body).expect("RESULT_SET JSON")["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row[0].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The exchanged token reads its own EHR, its composition and an AQL query
/// over it, and is refused `refused-not-subject` on another person's EHR.
#[tokio::test]
async fn the_exchanged_token_reads_its_own_ehr_and_no_other() {
    let w = world(false).await;
    let token = exchanged(&w.own, SUBSTANTIAL);

    let (status, body) = send(
        &w.app,
        ("GET", &format!("{BASE}/ehr/{}", w.own)),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "their own EHR: {body}");
    let (status, body) = read(&w.app, &w.own, &w.summary, &token).await;
    assert_eq!(status, StatusCode::OK, "their own summary: {body}");
    let uids = queried(&w.app, &w.own, &token).await;
    assert!(
        uids.iter().any(|uid| uid.starts_with(&w.summary)),
        "{uids:?}"
    );

    let (status, body) = send(
        &w.app,
        ("GET", &format!("{BASE}/ehr/{}", w.other)),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "another EHR: {body}");
    assert!(body.contains("refused-not-subject"), "{body}");
}

/// RFC 9470 §3: the same token authenticated at a level below the floor is
/// refused `401` with the step-up challenge naming the accepted values.
#[tokio::test]
async fn the_exchanged_token_below_the_floor_gets_the_step_up_challenge() {
    let w = world(false).await;
    let (status, headers, body) = send_full(
        &w.app,
        (
            "GET",
            &format!("{BASE}/ehr/{}/composition/{}", w.own, w.summary),
        ),
        &exchanged(&w.own, LOW),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(
        headers
            .get(header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok()),
        Some(
            format!(
                r#"Bearer realm="ferroehr", error="insufficient_user_authentication", acr_values="{HIGH} {SUBSTANTIAL}""#
            )
            .as_str()
        )
    );
    let (status, body) = read(&w.app, &w.own, &w.summary, &exchanged(&w.own, HIGH)).await;
    assert_eq!(status, StatusCode::OK, "at high: {body}");
}

/// EHDS Art. 3(3): a summary under an active access delay reads as absent to
/// the exchanged token, by id and in a query, until a professional releases
/// it.
#[tokio::test]
async fn the_access_delay_withholds_from_the_exchanged_token() {
    let w = world(true).await;
    let token = exchanged(&w.own, SUBSTANTIAL);

    let (status, body) = read(&w.app, &w.own, &w.summary, &token).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "held: {body}");
    let uids = queried(&w.app, &w.own, &token).await;
    assert!(
        !uids.iter().any(|uid| uid.starts_with(&w.summary)),
        "{uids:?}"
    );
    let (status, body) = read(&w.app, &w.own, &w.summary, &professional()).await;
    assert_eq!(status, StatusCode::OK, "the professional reads it: {body}");

    let (status, body) = send(
        &w.app,
        ("POST", &format!("{EXT}/admin/access-delay/release")),
        &professional(),
        Some((
            "application/json",
            json!({ "ehr_id": w.own, "vo_id": w.summary }).to_string(),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "release: {body}");
    let (status, body) = read(&w.app, &w.own, &w.summary, &token).await;
    assert_eq!(status, StatusCode::OK, "released: {body}");
}

/// EHDS Annex II 3.2(b): the access record of the read names the patient's
/// login and `PERSON`, the exchanging client, the issuer and the assurance
/// level, and classifies the category read.
#[tokio::test]
async fn the_access_record_names_the_patient_the_client_and_the_level() {
    let w = world(false).await;
    let (status, body) = read(&w.app, &w.own, &w.summary, &exchanged(&w.own, SUBSTANTIAL)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let row = loop {
        let row = sqlx::query(
            "SELECT principal, client_id, issuer, assurance_level, assurance_value, \
             ehr_id, binding, party, chain, chain_seq FROM audit.access_trail \
             WHERE principal = $1 AND operation = 'composition_get' \
             ORDER BY chain_seq DESC LIMIT 1",
        )
        .bind(PATIENT)
        .fetch_optional(&w.pool)
        .await
        .expect("read the trail");
        if let Some(row) = row {
            break row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no access record of the read"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        row.get::<Option<String>, _>("client_id").as_deref(),
        Some(BRIDGE)
    );
    assert_eq!(
        row.get::<Option<String>, _>("issuer").as_deref(),
        Some(ISSUER)
    );
    assert_eq!(
        row.get::<Option<String>, _>("assurance_level").as_deref(),
        Some("substantial")
    );
    assert_eq!(
        row.get::<Option<String>, _>("assurance_value").as_deref(),
        Some(SUBSTANTIAL)
    );
    assert_eq!(
        row.get::<Option<String>, _>("ehr_id").as_deref(),
        Some(w.own.as_str())
    );
    let chain: String = row.get("chain");
    let (event, _) = AuditStore::new(w.pool.clone())
        .record(
            AuditChain::named(&chain).expect("a chain the trail names"),
            row.get("chain_seq"),
        )
        .await
        .expect("read the record")
        .expect("the record is stored");
    assert_eq!(
        event
            .category
            .as_ref()
            .map(|category| json!(category.stored_categories())),
        Some(json!([format!("{PRIORITY}|Patient-Summaries")]))
    );
    assert_eq!(
        row.get::<Option<String>, _>("binding").as_deref(),
        Some("self")
    );
    let party: Option<Value> = row.get("party");
    assert_eq!(
        party.as_ref().map(|party| &party["id"]),
        Some(&json!(w.person.to_string())),
        "the record names the patient's PERSON"
    );
}
