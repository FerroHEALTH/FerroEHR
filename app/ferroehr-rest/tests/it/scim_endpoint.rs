// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SCIM endpoint at `/scim/v2/` (#5014), driven through the assembled
//! router: `/ServiceProviderConfig` without a token, the other discovery
//! documents only for a declared provisioning client holding its scope
//! (RFC 7644 §4, RFC 6750 §3), and every refusal as an RFC 7644 §3.12
//! `Error` message. Who may provision is our own design.

use std::num::NonZeroU32;

use axum::Router;
use ferroehr::config::binding::PartyBindingConfig;
use ferroehr::config::binding::person::{PersonSection, PersonTemplate, RoleSection, Selector};
use ferroehr::config::scim::{ScimClient, ScimConfig};
use ferroehr_rest::config::AppConfig;
use ferrohealth_server::section::CallerRate;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::profile::Preset;
use http::{StatusCode, header};
use serde_json::{Value, json};

use crate::common;

/// The deployment's own authorization server, which issues the client's tokens.
const MACHINES: &str = "https://as.example";
/// The issuer of the professionals' logins.
const PERSONS: &str = "https://login.example";
/// The audience of both issuers.
const AUDIENCE: &str = "ferroehr";
/// The identifier issuer of the binding keys the client provisions.
const LOGIN: &str = "urn:example:login";
/// The client and its scope.
const CLIENT: &str = "provisioning";
const SCOPE: &str = "ferroehr/scim";

/// The party mapping a User is written with: the default person template
/// with its key and `userName` selectors, and the professional role.
fn binding() -> PartyBindingConfig {
    let selector = |kind: &str| Selector {
        path: "/details[at0001]/items[at0002]".to_owned(),
        identifier_type: kind.to_owned(),
    };
    PartyBindingConfig {
        person: PersonSection {
            template: vec![PersonTemplate {
                template_id: "FerroEHR professional".to_owned(),
                key: selector("guid"),
                user_name: Some(selector("scim_user_name")),
                default: true,
                ..PersonTemplate::default()
            }],
            ..PersonSection::default()
        },
        professional_role: Some(RoleSection {
            template_id: "FerroEHR professional role".to_owned(),
        }),
        ..PartyBindingConfig::default()
    }
}

fn config(rate: Option<CallerRate>) -> AppConfig {
    let machines = IssuerEntry {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(MACHINES),
        kind: Some(IssuerKind::Machines),
        profile: Preset::Rfc9068,
        audience: vec![AUDIENCE.to_owned()],
        jwks_file: common::jwks_file().display().to_string(),
        // NOTE: the test key signs RFC 9068 `at+jwt` tokens.
        token_types: Some(vec!["at+jwt".to_owned()]),
        ..IssuerEntry::default()
    };
    let persons = IssuerEntry {
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(PERSONS, AUDIENCE)
    };
    let mut auth = common::auth(AuthSection {
        issuer: vec![machines, persons],
        party: vec![PartySection {
            issuer: ferrohealth_secret::secret_url::SecretUrl::new(PERSONS),
            identifier_issuer: LOGIN.to_owned(),
            identifier_type: "guid".to_owned(),
            ..PartySection::default()
        }],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let mut server = common::server();
    server.caller_rate = rate;
    {
        let mut c = AppConfig {
            server,
            auth,
            party_binding: binding(),
            ..AppConfig::default()
        };
        c.public_root = Some(
            ferroehr::config::server::PublicRoot::new("https://cdr.example.com", "")
                .expect("a public root"),
        );
        c.auth.scim = Some(ScimConfig {
            client: vec![ScimClient {
                issuer: ferrohealth_secret::secret_url::SecretUrl::new(MACHINES),
                client_id: CLIENT.to_owned(),
                scope: SCOPE.to_owned(),
                binding_issuer: LOGIN.to_owned(),
            }],
            ..ScimConfig::default()
        });
        c
    }
}

async fn app(rate: Option<CallerRate>) -> (testkit::TestDb, Router) {
    let (db, service) = common::test_service().await;
    let config = config(rate);
    assert!(
        config
            .auth
            .scim
            .as_ref()
            .is_some_and(|scim| scim.errors(None).is_empty()),
        "the test configuration is valid"
    );
    (db, common::router_with(config, service).await)
}

/// An app-only token of `client_id` at the authorization server holding `scope`.
fn client_token(client_id: &str, scope: &str) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    common::bearer(&json!({ "iss": MACHINES, "aud": AUDIENCE, "exp": exp,
                            "sub": client_id, "client_id": client_id, "scope": scope }))
}

/// A professional's token at the persons issuer.
fn person_token() -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    common::bearer(&json!({ "iss": PERSONS, "aud": AUDIENCE, "exp": exp,
                            "sub": "someone", "scope": SCOPE }))
}

/// `GET path` under `/scim/v2` as `credential`; the status, the headers and
/// the JSON body.
async fn get(
    app: &Router,
    path: &str,
    credential: Option<&str>,
) -> (StatusCode, http::HeaderMap, Value) {
    let mut request = common::req("GET", format!("/scim/v2{path}"));
    if let Some(credential) = credential {
        request = request.auth(credential);
    }
    let (status, headers, body) = common::send(app, request.build()).await;
    let body = serde_json::from_str(&body).unwrap_or(Value::Null);
    (status, headers, body)
}

/// Asserts a SCIM `Error` message with `status`, in the SCIM media type.
fn assert_error(status: StatusCode, headers: &http::HeaderMap, body: &Value, expected: StatusCode) {
    assert_eq!(status, expected, "{body}");
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/scim+json")
    );
    assert_eq!(
        body["schemas"],
        json!(["urn:ietf:params:scim:api:messages:2.0:Error"])
    );
    assert_eq!(body["status"], json!(expected.as_str()));
}

#[tokio::test]
async fn the_service_provider_config_is_readable_without_a_token() {
    let (_db, app) = app(None).await;
    let (status, headers, body) = get(&app, "/ServiceProviderConfig", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/scim+json")
    );
    assert_eq!(body["authenticationSchemes"][0]["type"], "oauthbearertoken");
    assert_eq!(body["bulk"]["supported"], false);
    assert_eq!(body["filter"]["supported"], true);
    assert_eq!(body["filter"]["maxResults"], 100);
    assert_eq!(
        body["meta"]["location"],
        "https://cdr.example.com/scim/v2/ServiceProviderConfig"
    );
}

/// Every `meta.location` hangs off the one public URL of `[server]`, after the
/// mount path the server serves every route under (#5465).
#[tokio::test]
async fn the_locations_hang_off_the_server_s_public_url_and_mount_path() {
    let (_db, service) = common::test_service().await;
    let mut config = config(None);
    let server = ferroehr::config::server::ServerConfig {
        shared: ferrohealth_server::section::ServerSection {
            base_path: ferrohealth_server::section::BasePath::new("/ehr").expect("a mount path"),
            ..config.server.clone()
        },
        public_base_url: Some(ferrohealth_secret::secret_url::SecretUrl::new(
            "https://gw.example/cdr",
        )),
    };
    config.public_root = server.public_root();
    config.server = server.shared;
    let app = common::router_with(config, service).await;
    let request = common::req("GET", "/ehr/scim/v2/ServiceProviderConfig").build();
    let (status, _headers, body) = common::send(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let body: Value = serde_json::from_str(&body).expect("a JSON body");
    assert_eq!(
        body["meta"]["location"],
        "https://gw.example/cdr/ehr/scim/v2/ServiceProviderConfig"
    );
}

#[tokio::test]
async fn a_declared_client_holding_its_scope_reads_the_discovery_documents() {
    let (_db, app) = app(None).await;
    let token = client_token(CLIENT, SCOPE);
    let (status, _, body) = get(&app, "/ResourceTypes", Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["Resources"][0]["endpoint"], "/Users");
    let (status, _, body) = get(&app, "/ResourceTypes/User", Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, body) = get(&app, "/Schemas", Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["Resources"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|schema| schema["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "urn:ietf:params:scim:schemas:core:2.0:User",
            "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User"
        ]
    );
    let (status, _, body) = get(
        &app,
        "/Schemas/urn:ietf:params:scim:schemas:core:2.0:User",
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, headers, body) = get(&app, "/Schemas?filter=id%20pr", Some(&token)).await;
    assert_error(status, &headers, &body, StatusCode::FORBIDDEN);
    let (status, headers, body) = get(&app, "/Schemas/urn:example:none", Some(&token)).await;
    assert_error(status, &headers, &body, StatusCode::NOT_FOUND);
    let (status, headers, body) = get(&app, "/Nowhere", Some(&token)).await;
    assert_error(status, &headers, &body, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn no_token_is_401_with_the_bearer_challenge() {
    let (_db, app) = app(None).await;
    let (status, headers, body) = get(&app, "/Schemas", None).await;
    assert_error(status, &headers, &body, StatusCode::UNAUTHORIZED);
    assert_eq!(
        headers
            .get(header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok()),
        Some("Bearer")
    );
}

#[tokio::test]
async fn a_missing_scope_an_undeclared_client_and_a_person_are_refused() {
    let (_db, app) = app(None).await;
    let (status, headers, body) = get(&app, "/Schemas", Some(&client_token(CLIENT, "other"))).await;
    assert_error(status, &headers, &body, StatusCode::FORBIDDEN);
    assert_eq!(
        headers
            .get(header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok()),
        Some("Bearer error=\"insufficient_scope\"")
    );
    let (status, headers, body) =
        get(&app, "/Schemas", Some(&client_token("stranger", SCOPE))).await;
    assert_error(status, &headers, &body, StatusCode::FORBIDDEN);
    let (status, headers, body) = get(&app, "/Schemas", Some(&person_token())).await;
    assert_error(status, &headers, &body, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn past_its_rate_a_client_is_429() {
    let rate = CallerRate {
        requests_per_second: NonZeroU32::MIN,
        burst: NonZeroU32::MIN,
    };
    let (_db, app) = app(Some(rate)).await;
    let token = client_token(CLIENT, SCOPE);
    let mut statuses = Vec::new();
    for _ in 0..4 {
        statuses.push(get(&app, "/Schemas", Some(&token)).await.0);
    }
    assert!(
        statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
        "{statuses:?}"
    );
}

#[tokio::test]
async fn off_by_default_nothing_is_served() {
    let (db, service) = common::test_service().await;
    let config = {
        let mut c = AppConfig { ..config(None) };
        c.auth.scim = Some(ScimConfig::default());
        c
    };
    let app = common::router_with(config, service).await;
    let _keep = db;
    let (status, _, _) = get(&app, "/ServiceProviderConfig", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
