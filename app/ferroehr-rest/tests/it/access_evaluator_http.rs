// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access evaluator at the wire (#5513): worked examples of the ABAC
//! design over the assembled router, with the refusal code in the problem
//! body of a `403` and nothing in the `404` of a restriction.
//!
//! `EHR_ACCESS` is the access-decision authority of an EHR (RM ehr
//! `org.openehr.rm.ehr.ehr_access.adoc` §`EHR_ACCESS` Class); EHDS Art. 8
//! keeps a restriction invisible to a provider and Art. 11(5) lets the
//! person's vital interests lift it (`docs/law/eu/ehds/text.html`). The
//! decision procedure and its codes are our own design.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::party_binding::grants::operation::Operation;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;
use crate::common::{BASE, EXT};

/// The header the default configuration reads the declared purpose from.
const PURPOSE: &str = "x-purpose-of-use";
/// The `persons` issuer of Dr B, whose role claims grant the emergency role.
const MEDICS: &str = "https://medics.example";

/// The `machines` issuer of the regional providers' system clients.
const MACHINES: &str = "https://machines.example";
/// The SMART scopes of a system client that reads compositions and queries
/// across EHRs.
const SYSTEM_SCOPES: &str = "system/composition-*.rs system/aql-*.s";

/// The credential of `name`: Dr B's token of [`MEDICS`], holding the
/// emergency role, an app-only token of [`MACHINES`] for `client:<id>`, or a
/// Basic user's.
fn credential(name: &str) -> String {
    if let Some(client) = name.strip_prefix("client:") {
        return crate::caller_kinds_http::bearer(
            MACHINES,
            &crate::caller_kinds_http::audience(MACHINES),
            &json!({ "sub": client, "client_id": client, "scope": SYSTEM_SCOPES }),
        );
    }
    if name == "drb" {
        let exp = jiff::Timestamp::now().as_second() + 3600;
        return common::bearer(&json!({
            "sub": "drb", "iss": MEDICS, "aud": "ferroehr", "exp": exp,
            "roles": ["USER", "EMERGENCY"],
        }));
    }
    common::basic_credential(name)
}

/// An audited app with `ETREAT` an emergency purpose and `EMERGENCY` the
/// emergency access role; Basic users for the administrator, a nurse and a
/// psychiatrist, each bound to a `PERSON`; Dr B of [`MEDICS`]; the system
/// clients of [`MACHINES`]; and the IPS template uploaded.
async fn app() -> (
    testkit::TestDb,
    PgPool,
    Arc<ferroehr::service::FerroEhrService>,
    Router,
) {
    let (db, pool) = common::migrated_pool().await;
    let audit = ferroehr::system_log::config::AuditConfig {
        purpose_codes: vec!["TREAT".to_owned(), "ETREAT".to_owned()],
        emergency_purpose_codes: vec!["ETREAT".to_owned()],
        ..common::local_audit()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        common::binding_service(&pool)
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone())),
    );
    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[
            ("root", &["ADMIN"]),
            ("nurse", &["USER"]),
            ("psychiatrist", &["USER"]),
        ],
    )
    .await;
    common::grant_to(&svc, &pool, "root", Operation::WriteAccessPolicy).await;
    config
        .auth
        .shared
        .issuer
        .push(ferrohealth_smart::config::IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            ..common::entry(MEDICS, "ferroehr")
        });
    config
        .auth
        .shared
        .issuer
        .push(crate::caller_kinds_http::machines(
            MACHINES,
            ferrohealth_smart::profile::Preset::Rfc9068,
        ));
    common::bind_issuer(&mut config, MEDICS);
    common::bind_subjects(&pool, &[(MEDICS, "drb")]).await;
    config.rbac.role_issuers = vec![ferrohealth_secret::secret_url::SecretUrl::new(MEDICS)];
    config.rbac.emergency_access_role = Some("EMERGENCY".to_owned());
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = AuthzHandle::build(&authz, &config.api.base_path).map(Arc::new);
    let app = common::build_full(config, Arc::clone(&svc), authz, Observability::default()).await;
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/definition/template/adl1.4"),
        "root",
        None,
        Some(("application/xml", common::ips_opt_xml())),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    (db, pool, svc, app)
}

/// Sends `method uri` as `user`, with an optional declared purpose and body,
/// asking for the full problem details.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    user: &str,
    purpose: Option<&str>,
    body: Option<(&str, String)>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, credential(user))
        .header(header::ACCEPT, "application/json")
        .header("Prefer", "return=representation");
    if let Some(purpose) = purpose {
        req = req.header(PURPOSE, purpose);
    }
    let req = match body {
        Some((content_type, body)) => req
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body)),
        None => req.body(Body::empty()),
    }
    .unwrap();
    common::send_body(app, req).await
}

/// An EHR with one IPS composition, as the administrator; the EHR and the
/// composition's versioned-object id.
async fn ehr_with_composition(app: &Router) -> (String, String) {
    let (status, body) = send(app, "POST", &format!("{BASE}/ehr"), "root", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let ehr = serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let (status, headers, body) = common::send(
        app,
        Request::post(format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::AUTHORIZATION, credential("root"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(common::ips_canonical_composition().to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    let vo = headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .split("::")
        .next()
        .expect("object id")
        .to_owned();
    (ehr, vo)
}

/// A term of the access control groups.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID",
                    "value": ferroehr::party_binding::policy_mark::TERMINOLOGY },
                "code_string": code } })
}

/// A directive of `kind` and `effect` with `extra` members.
fn directive(kind: &str, effect: &str, extra: &Value) -> Value {
    let mut d = json!({
        "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": kind },
        "archetype_node_id": "at0001",
        "kind": term(kind),
        "effect": term(effect),
        "emergency_override": false
    });
    if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
        d.extend(extra.clone());
    }
    d
}

/// An actor scope naming the `PERSON` `person`.
fn naming(person: uuid::Uuid) -> Value {
    json!([{ "party": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                        "id": { "_type": "HIER_OBJECT_ID", "value": person.to_string() } } }])
}

/// The `refusal` member and the `detail` of a problem body.
fn refusal(body: &str) -> (Value, String) {
    let problem: Value = serde_json::from_str(body).expect("a problem body");
    (
        problem["refusal"].clone(),
        problem["detail"].as_str().unwrap_or_default().to_owned(),
    )
}

/// The composition ids an AQL population query returns to `user`.
async fn queried(app: &Router, user: &str, purpose: Option<&str>) -> Vec<String> {
    let (status, body) = send(
        app,
        "POST",
        &format!("{BASE}/query/aql"),
        user,
        purpose,
        Some((
            "application/json",
            json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" }).to_string(),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "query: {body}");
    serde_json::from_str::<Value>(&body).expect("RESULT_SET")["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|row| row[0].as_str())
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect()
}

/// The ex-partner: Ms A's consent deny names Dr B, who is refused with the
/// consent code everywhere he reaches her record, until he declares an
/// emergency with the break-glass role; everyone else reads it.
#[tokio::test]
async fn the_ex_partner_is_refused_until_his_emergency() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let drb = common::issuer_person(&pool, MEDICS, "drb").await;
    let deny = directive(
        "consent",
        "deny",
        &json!({ "applies_to": naming(drb), "emergency_override": true }),
    );
    common::commit_access_settings(
        &svc,
        ehr.parse().expect("an EHR id"),
        common::directive_settings(&[deny]),
    )
    .await;
    let read = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "drb", Some("TREAT"), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (code, detail) = refusal(&body);
    assert_eq!(code, json!("refused-consent-denied"), "{body}");
    assert!(detail.contains("refused-consent-denied"), "{detail}");
    let (status, _) = send(&app, "GET", &format!("{BASE}/ehr/{ehr}"), "drb", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the EHR itself");
    assert!(
        queried(&app, "drb", Some("TREAT")).await.is_empty(),
        "the query"
    );
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::OK, "anyone else: {body}");
    let (status, body) = send(&app, "GET", &read, "drb", Some("ETREAT"), None).await;
    assert_eq!(status, StatusCode::OK, "the emergency lifts it: {body}");
    assert_eq!(queried(&app, "drb", Some("ETREAT")).await, vec![vo]);
}

/// Mental health, except the psychiatrist: a restriction of the category the
/// patient summaries are mapped to, with an exception for the psychiatrist,
/// hides the summary from the nurse as though it did not exist, in a read and
/// a query, while she reads the EHR; the psychiatrist reads it.
#[tokio::test]
async fn a_category_restriction_with_an_exception() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let psychiatrist = common::basic_person(&pool, "psychiatrist").await;
    let restriction = directive(
        "restriction",
        "deny",
        &json!({
            "data": [{ "categories": [{ "_type": "DV_CODED_TEXT", "value": "Patient summaries",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "EEHRxF" },
                    "code_string": "Patient-Summaries" } }] }],
            "exceptions": [directive("consent", "permit", &json!({ "applies_to": naming(psychiatrist) }))]
        }),
    );
    common::commit_access_settings(
        &svc,
        ehr.parse().expect("an EHR id"),
        common::directive_settings(&[restriction]),
    )
    .await;
    let read = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(!body.to_lowercase().contains("restrict"), "{body}");
    assert!(!body.contains("refused-"), "{body}");
    let (status, _) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{ehr}"),
        "nurse",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the EHR stays readable");
    assert!(
        queried(&app, "nurse", None).await.is_empty(),
        "the query leaves it out"
    );
    let (status, body) = send(&app, "GET", &read, "psychiatrist", None, None).await;
    assert_eq!(status, StatusCode::OK, "the exception reads it: {body}");
    assert_eq!(queried(&app, "psychiatrist", None).await, vec![vo]);
}

/// HIV results: the controller's policy classifies the summary template as
/// `HIV`; a professional without the clearance is refused it with the
/// sensitivity code, and the query leaves it out.
#[tokio::test]
async fn a_classification_refuses_the_uncleared() {
    let (_db, _pool, _svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let archetype = "openEHR-EHR-ACCESS_POLICY.controller.v1";
    let policy = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Hospital access policy" },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0" },
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
        "classifications": [{
            "_type": "ACCESS_CLASSIFICATION",
            "name": { "_type": "DV_TEXT", "value": "HIV results" },
            "archetype_node_id": "at0002",
            "data": { "_type": "ACCESS_DATA_SCOPE", "templates": ["International Patient Summary"] },
            "sensitivities": [{ "_type": "DV_CODED_TEXT", "value": "HIV",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "2.16.840.1.113883.5.25" },
                    "code_string": "HIV" } }]
        }],
        "care_relationship_required": false,
        "purpose_required": false
    });
    let (status, _, body) = common::send(
        &app,
        Request::put(format!("{EXT}/admin/access_policy"))
            .header(header::AUTHORIZATION, credential("root"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_NONE_MATCH, "*")
            .body(Body::from(policy.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the policy: {body}");
    let read = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body).0, json!("refused-sensitivity"), "{body}");
    let (status, _) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{ehr}"),
        "nurse",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the EHR stays readable");
    assert!(
        queried(&app, "nurse", None).await.is_empty(),
        "the query leaves it out"
    );
}

/// A restriction no emergency may lift is accepted, and holds against the
/// break-glass role declaring an emergency.
#[tokio::test]
async fn an_absolute_restriction_holds_in_an_emergency() {
    let (_db, _pool, svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let mut absolute = common::access_directive("restriction", &[&vo]);
    absolute["emergency_override"] = json!(false);
    common::commit_access_settings(
        &svc,
        ehr.parse().expect("an EHR id"),
        common::directive_settings(&[absolute]),
    )
    .await;
    let read = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "drb", Some("ETREAT"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// Regional sharing by BPPC: the subject's permit, copied from the region's
/// opt-in policy, names the regional `GROUP` under `consent_required`
/// settings. A system client acting for a member `ORGANISATION` reads the
/// record and queries it; a client of a provider outside the region is
/// refused with the default's code and queries nothing (IHE BPPC; RM
/// Demographic IM `PARTY_RELATIONSHIP`, coded `member_of`; our own design,
/// #5575).
#[tokio::test]
async fn regional_sharing_admits_a_client_of_a_member_organisation() {
    use crate::system_grants::{acting_client, group, provider_organisation};
    let (_db, _pool, svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let region = group(&svc, "Regional health network").await;
    let member = provider_organisation(&svc, ("urn:oid:2.999.7", "member"), &[region]).await;
    let outside = provider_organisation(&svc, ("urn:oid:2.999.7", "outside"), &[]).await;
    common::allowing(&svc, &[Operation::QuerySystemWide]);
    let queries = [crate::system_grants::capability("query_system_wide", &[])];
    acting_client(&svc, &[(MACHINES, "member-lab")], Some(member), &queries).await;
    acting_client(&svc, &[(MACHINES, "outside-lab")], Some(outside), &queries).await;
    let permit = directive(
        "consent",
        "permit",
        &json!({
            "applies_to": [{ "party": { "_type": "PARTY_REF", "namespace": "demographic",
                "type": "GROUP", "id": { "_type": "HIER_OBJECT_ID", "value": region.to_string() } } }],
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "urn:oid:2.999.7.1",
                              "id": "regional-opt-in", "type": "BPPC" }]
        }),
    );
    let mut settings = common::access_settings("consent_required", &[]);
    settings["directives"] = json!([permit]);
    common::commit_access_settings(&svc, ehr.parse().expect("an EHR id"), settings).await;
    svc.grants().refresh().await.expect("the copy reads");
    let read = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "client:member-lab", None, None).await;
    assert_eq!(status, StatusCode::OK, "a member's client reads: {body}");
    assert_eq!(queried(&app, "client:member-lab", None).await, vec![vo]);
    let (status, body) = send(&app, "GET", &read, "client:outside-lab", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        refusal(&body).0,
        json!("refused-consent-required"),
        "{body}"
    );
    assert!(
        queried(&app, "client:outside-lab", None).await.is_empty(),
        "the query"
    );
}
