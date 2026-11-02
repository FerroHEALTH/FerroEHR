// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the SMART on openEHR surface: the discovery
//! document built from the issuer's metadata, with its absolute `baseUrl`s
//! (master04 §Services: "Absolute URL to the root of the API `*(required)*`")
//! and the capabilities the server enforces, and the route requirements of the
//! template, AQL and composition families (master08 §Resource Scopes), with the
//! `ehrId` launch context a `patient/` grant is confined to (master07 §Context
//! Selection).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ferroehr::config::rest::RestConfig;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::{AuthSection, IssuerKey};

use crate::common;
use crate::common::send_body;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const EXT: &str = "/ferroehr/rest/extensions/v1";
const AUDIENCE: &str = "ferroehr";
const DISCOVERY: &str = "/ferroehr/rest/.well-known/smart-configuration";

/// An authorization server serving its RFC 8414 metadata and the committed test
/// key set, advertising flows and methods the discovery document must filter.
async fn issuer() -> MockServer {
    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": uri,
            "jwks_uri": format!("{uri}/jwks"),
            "authorization_endpoint": format!("{uri}/authorize"),
            "token_endpoint": format!("{uri}/token"),
            // master06 §Deprecated Flows: the implicit and password grants
            // must never reach the document.
            "grant_types_supported": ["authorization_code", "client_credentials", "implicit", "password"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "private_key_jwt"],
            "response_types_supported": ["code"],
            "code_challenge_methods_supported": ["plain", "S256"],
            "scopes_supported": ["openid", "patient/composition-*.r"],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_string(include_str!(
            "../../../../docs/conformance/party/smart/jwks.json"
        )))
        .mount(&server)
        .await;
    server
}

/// The configuration trusting `issuer`, its discovery document served when
/// `smart_enabled`, its `patient/` grants bound when `patient_bound`.
fn config(issuer: &str, smart_enabled: bool, patient_bound: bool) -> AppConfig {
    let mut cfg = AppConfig {
        server: common::server(),
        rest: RestConfig {
            base_path: BASE.to_owned(),
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            cors_permissive: false,
            ..Default::default()
        },
        auth: AuthSection {
            issuer: IssuerKey::Flat(issuer.to_owned()),
            audience: vec![AUDIENCE.to_owned()],
            ..AuthSection::default()
        },
        ..Default::default()
    };
    cfg.smart.enabled = smart_enabled;
    cfg.smart.public_base_url = Some("https://cdr.example.com".to_owned());
    if patient_bound {
        cfg.smart.patient_issuers = vec![issuer.to_owned()];
    }
    cfg
}

async fn app(cfg: AppConfig) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(cfg, service).await)
}

/// A server with authentication off.
async fn open_app() -> (testkit::TestDb, Router) {
    let mut cfg = common::api_config(false);
    BASE.clone_into(&mut cfg.rest.base_path);
    app(cfg).await
}

fn get(uri: &str, credential: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::ACCEPT, "application/json");
    if let Some(credential) = credential {
        builder = builder.header(header::AUTHORIZATION, credential);
    }
    builder.body(Body::empty()).unwrap()
}

/// A bearer token of `issuer` granting `scope`, with `extra` claims.
fn token(issuer: &str, scope: &str, extra: &Value) -> String {
    let mut claims = json!({
        "sub": "alice",
        "iss": issuer,
        "aud": AUDIENCE,
        "exp": jiff::Timestamp::now().as_second() + 3600,
        "scope": scope,
    });
    for (key, value) in extra.as_object().expect("object") {
        claims[key] = value.clone();
    }
    common::bearer(&claims)
}

fn capabilities(doc: &Value) -> Vec<&str> {
    doc["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

// ── discovery ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn discovery_serves_absolute_base_urls_and_the_enforced_capabilities() {
    let issuer = issuer().await;
    let (_pg, app) = app(config(&issuer.uri(), true, false)).await;
    let (status, body) = send_body(&app, get(DISCOVERY, None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let doc: Value = serde_json::from_str(&body).unwrap();
    let base = doc["services"]["org.openehr.rest"]["baseUrl"]
        .as_str()
        .expect("baseUrl");
    assert!(
        base.starts_with("https://cdr.example.com/"),
        "master04 §Services: baseUrl is an Absolute URL — got {base}"
    );
    let caps = capabilities(&doc);
    assert!(caps.contains(&"openehr-permission-v1"), "{caps:?}");
    assert!(caps.contains(&"context-openehr-ehr"), "{caps:?}");
    assert!(caps.contains(&"permission-user"), "{caps:?}");
    assert!(
        !caps.contains(&"permission-patient"),
        "no issuer is bound, so no patient/ grant counts: {caps:?}"
    );
    assert_eq!(doc["token_endpoint"], format!("{}/token", issuer.uri()));
}

/// SMART App Launch 2.2.0, Conformance, §Metadata: the document names only
/// the `authorization_code` and `client_credentials` grants (master06
/// §Deprecated Flows refuses the implicit and password grants), carries `S256`
/// and never `plain`, and lists the scopes the server honours, never the
/// issuer's own list.
#[tokio::test]
async fn discovery_drops_deprecated_flows_plain_pkce_and_the_issuer_s_scopes() {
    let issuer = issuer().await;
    let (_pg, app) = app(config(&issuer.uri(), true, false)).await;
    let (_status, body) = send_body(&app, get(DISCOVERY, None)).await;
    let doc: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        doc["grant_types_supported"],
        json!(["authorization_code", "client_credentials"])
    );
    assert_eq!(doc["code_challenge_methods_supported"], json!(["S256"]));
    let scopes: Vec<&str> = doc["scopes_supported"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        !scopes.contains(&"patient/composition-*.r"),
        "the issuer's patient scope is not honoured without a bound issuer: {scopes:?}"
    );
}

#[tokio::test]
async fn a_bound_issuer_declares_patient_permission() {
    let issuer = issuer().await;
    let (_pg, app) = app(config(&issuer.uri(), true, true)).await;
    let (_status, body) = send_body(&app, get(DISCOVERY, None)).await;
    let doc: Value = serde_json::from_str(&body).unwrap();
    assert!(capabilities(&doc).contains(&"permission-patient"));
}

#[tokio::test]
async fn discovery_absent_when_disabled() {
    let issuer = issuer().await;
    let (_pg, app) = app(config(&issuer.uri(), false, false)).await;
    let (status, _b) = send_body(&app, get(DISCOVERY, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "zero wire drift when off");
}

// ── the route requirements ───────────────────────────────────────────────────

/// master08 §Resource Scopes: a token granting no `template-` scope is refused
/// the template family, one granting it is served.
#[tokio::test]
async fn the_template_family_needs_a_template_grant() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (_pg, app) = app(config(&uri, false, false)).await;
    let list = format!("{BASE}/definition/template/adl1.4");
    let (status, body) =
        send_body(&app, get(&list, Some(&token(&uri, "openid", &json!({}))))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send_body(
        &app,
        get(&list, Some(&token(&uri, "user/template-*.r", &json!({})))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// master08 §Resource Scopes: a token granting no `aql-` search is refused an
/// ad hoc query.
#[tokio::test]
async fn the_query_family_needs_an_aql_grant() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (_pg, app) = app(config(&uri, false, false)).await;
    let query = |credential: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/query/aql"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, credential)
            .body(Body::from(r#"{"q": "SELECT e/ehr_id/value FROM EHR e"}"#))
            .unwrap()
    };
    let (status, body) =
        send_body(&app, query(&token(&uri, "user/template-*.r", &json!({})))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send_body(&app, query(&token(&uri, "user/aql-*.s", &json!({})))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// master07 §Context Selection and master08 §Resource Scopes: a `patient/`
/// query grant of a bound issuer runs only within the launch context's EHR,
/// named by the single-EHR `ehr_id`; a query naming no EHR is refused.
#[tokio::test]
async fn a_patient_query_grant_runs_within_the_launch_context_ehr_alone() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (_pg, app) = app(config(&uri, false, true)).await;
    let launched = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let credential = token(&uri, "patient/aql-*.s", &json!({ "ehrId": launched }));
    let query = |ehr: Option<&str>| {
        let q = "SELECT%20e/ehr_id/value%20FROM%20EHR%20e";
        let uri = ehr.map_or_else(
            || format!("{BASE}/query/aql?q={q}"),
            |ehr| format!("{BASE}/query/aql?q={q}&ehr_id={ehr}"),
        );
        get(&uri, Some(&credential))
    };
    let (status, body) = send_body(&app, query(None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send_body(&app, query(Some("0b8f7c5e-0000-4000-8000-000000000001"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "another EHR: {body}");
    let (status, _body) = send_body(&app, query(Some(launched))).await;
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "the launch context's EHR is reached"
    );
}

/// master07 §Context Selection and master08 §Resource Scopes: a `patient/`
/// grant of a bound issuer reaches the launch context's EHR on a route that
/// names it by `{ehr_id}`, a released operation and an extension alike, and is
/// refused another EHR. The API is nested under the base path, so this pins
/// that the `{ehr_id}` segment is read from the full request path.
#[tokio::test]
async fn a_patient_grant_reaches_the_launch_context_ehr_on_its_routes() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (_pg, app) = app(config(&uri, false, true)).await;
    let launched = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let other = "0b8f7c5e-0000-4000-8000-000000000001";
    let credential = token(
        &uri,
        "patient/composition-*.r",
        &json!({ "ehrId": launched }),
    );
    for (base, route) in [(BASE, ""), (BASE, "/ehr_status"), (EXT, "/contribution")] {
        let (status, body) = send_body(
            &app,
            get(&format!("{base}/ehr/{launched}{route}"), Some(&credential)),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "GET /ehr/{{ehr_id}}{route} of the launch context's EHR: {body}"
        );
        let (status, body) = send_body(
            &app,
            get(&format!("{base}/ehr/{other}{route}"), Some(&credential)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "GET /ehr/{{ehr_id}}{route} of another EHR: {body}"
        );
    }
}

/// A `user/` grant reaches the EHR-scoped extension routes, the contribution
/// list among them, which name their EHR only by the `{ehr_id}` segment.
#[tokio::test]
async fn a_user_grant_reaches_the_contribution_list() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (_pg, app) = app(config(&uri, false, false)).await;
    let ehr = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let (status, body) = send_body(
        &app,
        get(
            &format!("{EXT}/ehr/{ehr}/contribution"),
            Some(&token(&uri, "user/composition-*.r", &json!({}))),
        ),
    )
    .await;
    assert_ne!(status, StatusCode::FORBIDDEN, "{body}");
    assert_ne!(status, StatusCode::UNAUTHORIZED, "{body}");
}

/// With authentication off there is no credential, so there is no scope to
/// check: the operator's explicit choice, which a gate that refused every
/// request would make unusable.
#[tokio::test]
async fn authentication_off_leaves_the_families_ungated() {
    let (_pg, app) = open_app().await;
    let (status, body) = send_body(
        &app,
        get(&format!("{BASE}/definition/template/adl1.4"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

// ── an unserved method ───────────────────────────────────────────────────────

/// ITS-REST `overview/Requests_and_responses` §HTTP Methods and RFC 9110
/// §15.5.6: `DELETE /ehr` is a method the resource does not serve, so it is
/// `405` with `Allow` whatever the token grants (Veredictum case
/// `I_EHR_SERVICE.create_ehr-wrong_method`), and the served `POST /ehr` keeps
/// its scope check.
#[tokio::test]
async fn an_unserved_method_is_405_with_allow_whatever_the_grants() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (_pg, app) = app(config(&uri, true, false)).await;
    let request = |method: &str, credential: &str| {
        Request::builder()
            .method(method)
            .uri(format!("{BASE}/ehr"))
            .header(header::ACCEPT, "application/json")
            .header(header::AUTHORIZATION, credential)
            .body(Body::empty())
            .unwrap()
    };
    let authorized = token(&uri, "user/composition-*.cruds", &json!({}));
    let unauthorized = token(&uri, "openid", &json!({}));
    for credential in [&authorized, &unauthorized] {
        let resp = app
            .clone()
            .oneshot(request("DELETE", credential))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let allow = resp
            .headers()
            .get(header::ALLOW)
            .and_then(|value| value.to_str().ok())
            .expect("RFC 9110 §15.5.6: a 405 carries Allow")
            .to_owned();
        assert!(allow.contains("POST"), "{allow}");
        assert!(!allow.contains("DELETE"), "{allow}");
    }
    let (status, body) = send_body(&app, request("POST", &unauthorized)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send_body(&app, request("POST", &authorized)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// A request without a credential is still `401` on an unserved method: the
/// method check follows authentication and opens nothing to an anonymous
/// caller.
#[tokio::test]
async fn an_unserved_method_without_a_credential_is_401() {
    let issuer = issuer().await;
    let (_pg, app) = app(config(&issuer.uri(), true, false)).await;
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/ehr"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send_body(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

/// A server trusting `issuer` with the coarse role gate switched on
/// (`[authz.rbac] enabled`).
async fn rbac_app(issuer: &str) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    let mut cfg = config(issuer, false, false);
    cfg.rbac.enabled = true;
    let mut authz = ferroehr::config::authz::AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = ferroehr_rest::extensions::access::authz::AuthzHandle::build(
        &authz,
        &cfg.rest.base_path,
        None,
        common::null_resolvers(),
    )
    .map(std::sync::Arc::new);
    let app = ferroehr_rest::build_full(
        cfg,
        service,
        authz,
        ferroehr_rest::extensions::management::Observability::default(),
    )
    .await
    .expect("build app");
    (pg, app)
}

/// A token whose SMART on openEHR scopes are `system/` scopes is a backend
/// client: it is authenticated without a role and its scopes decide access
/// (owner ruling on #4467; master08 §Resource Scopes, `system` access is
/// "based on pre-configured client-specific policy"). A role-less token whose
/// scope does not cover the operation, and one holding no SMART scope, are
/// still refused.
#[tokio::test]
async fn a_system_scoped_token_without_a_role_reaches_what_its_scopes_cover() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (_pg, app) = rbac_app(&uri).await;
    let list = format!("{BASE}/definition/template/adl1.4");
    for scope in ["system/template-*.r", "system/template-*.crud"] {
        let (status, body) =
            send_body(&app, get(&list, Some(&token(&uri, scope, &json!({}))))).await;
        assert_eq!(status, StatusCode::OK, "{scope}: {body}");
    }
    let (status, body) = send_body(
        &app,
        get(&list, Some(&token(&uri, "system/aql-*.s", &json!({})))),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an uncovering scope: {body}");
    let (status, body) =
        send_body(&app, get(&list, Some(&token(&uri, "openid", &json!({}))))).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "no SMART scope and no role: {body}"
    );
}

/// A `user/` or `patient/` token belongs to a person, so it still needs a role
/// with the role gate on (owner ruling on #4467, #2160): without one it is
/// refused `403` even when its scopes cover the operation, and a token mixing
/// a `system/` scope with a person's scope is refused alike.
#[tokio::test]
async fn a_person_scoped_token_without_a_role_is_refused() {
    let issuer = issuer().await;
    let uri = issuer.uri();
    let (_pg, app) = rbac_app(&uri).await;
    let list = format!("{BASE}/definition/template/adl1.4");
    for scope in [
        "user/template-*.r",
        "patient/template-*.r",
        "system/template-*.r user/template-*.r",
    ] {
        let (status, body) =
            send_body(&app, get(&list, Some(&token(&uri, scope, &json!({}))))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{scope}: {body}");
    }
    let (status, body) = send_body(
        &app,
        get(
            &list,
            Some(&token(
                &uri,
                "user/template-*.r",
                &json!({ "roles": ["USER"] }),
            )),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a user/ token with a role: {body}");
}
