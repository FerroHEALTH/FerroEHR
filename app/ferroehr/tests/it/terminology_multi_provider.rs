// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Simultaneous multi-terminology-server operation + terminology-server
//! authentication, driven by `wiremock` — several hermetic FHIR R4B servers in
//! one process, no network.
//!
//! BASE `docs/architecture_overview/master12-terminology.adoc` §Overview names
//! the ecosystem archetypes bind to — "LOINC, `ICDx`, ICPC, SNOMED CT and the
//! many other terminologies and vocabularies used in healthcare" — so a
//! deployment binds several at the same time and the CDR must operate against
//! several terminology servers simultaneously. These tests are the N ≥ 2
//! proof: two servers serving different terminologies inside one
//! `FerroEhrService`, selected per call by the configured routing.
//!
//! The routing-map mechanics themselves are spec-silent — our own
//! design/extension.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use ferroehr::service::FerroEhrService;
use ferroehr::service::terminology::config::{
    ExternalTerminologyConfig, FhirOperation, FhirProviderConfig, Oauth2AuthMethod, ProviderKind,
    TerminologyOauth2Config,
};
use ferroehr::service::terminology::router::TerminologyRouter;
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SNOMED: &str = "http://snomed.info/sct";
const LOINC: &str = "http://loinc.org";

/// A provider config pointing at `base`, cache off so every test assertion
/// counts real server interactions.
fn provider_cfg(base: &str) -> FhirProviderConfig {
    FhirProviderConfig {
        kind: ProviderKind::Fhir,
        url: base.to_owned(),
        operation: FhirOperation::ValidateCode,
        connect_timeout_ms: 500,
        request_timeout_ms: 800,
        oauth2_client: None,
        client_cert_path: None,
        client_key_path: None,
        ca_bundle_path: None,
        cache_ttl_secs: 0,
        cache_capacity: 0,
    }
}

fn external(
    providers: &[(&str, &FhirProviderConfig)],
    routes: &[(&str, &str)],
) -> ExternalTerminologyConfig {
    ExternalTerminologyConfig {
        enabled: true,
        providers: providers
            .iter()
            .map(|(name, cfg)| ((*name).to_owned(), (*cfg).clone()))
            .collect(),
        routes: routes
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        ..ExternalTerminologyConfig::default()
    }
}

/// A `CodeSystem/$lookup` server that answers only for `system`, with
/// `display` — so a call landing on the wrong server is an unmistakable miss.
async fn lookup_server(system: &str, code: &str, display: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/CodeSystem/$lookup"))
        .and(query_param("system", system))
        .and(query_param("code", code))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "display", "valueString": display}]
        })))
        .mount(&server)
        .await;
    // Anything else this server is asked is a routing defect, reported as a
    // distinguishable 404 rather than wiremock's "no match" panic.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    server
}

/// Two servers, two terminologies, one running service: a SNOMED CT lookup and
/// a LOINC lookup each reach their own server (BASE master12 §Overview — the
/// N ≥ 2 requirement).
#[tokio::test]
async fn two_servers_serve_two_terminologies_at_once() {
    let snomed = lookup_server(SNOMED, "38341003", "Hypertensive disorder").await;
    let loinc = lookup_server(LOINC, "8480-6", "Systolic blood pressure").await;

    let snomed_cfg = provider_cfg(&snomed.uri());
    let loinc_cfg = provider_cfg(&loinc.uri());
    let cfg = external(
        &[("snomed", &snomed_cfg), ("loinc", &loinc_cfg)],
        &[(SNOMED, "snomed"), (LOINC, "loinc")],
    );
    let router = TerminologyRouter::build(&cfg)
        .expect("build router")
        .expect("router materialised");
    assert_eq!(
        router.provider_names().collect::<Vec<_>>(),
        ["loinc", "snomed"],
        "every configured provider is materialised, not just `default`"
    );

    let service = service_with(router).await;
    let snomed_term = service
        .get_term(SNOMED, "38341003", None, None)
        .await
        .expect("SNOMED lookup routes to the SNOMED server");
    assert_eq!(snomed_term.terminology_id, SNOMED);
    let loinc_term = service
        .get_term(LOINC, "8480-6", None, None)
        .await
        .expect("LOINC lookup routes to the LOINC server");
    assert_eq!(loinc_term.terminology_id, LOINC);

    // Each server saw exactly its own terminology's lookup — proof the calls
    // did not both land on one provider.
    assert_eq!(
        snomed.received_requests().await.unwrap().len(),
        1,
        "the SNOMED server answered exactly the SNOMED lookup"
    );
    assert_eq!(
        loinc.received_requests().await.unwrap().len(),
        1,
        "the LOINC server answered exactly the LOINC lookup"
    );
}

/// A terminology with no explicit route falls back to the `default` provider;
/// with a route it does not. Both servers stay live throughout, so the
/// assertion is about selection, not availability.
#[tokio::test]
async fn unrouted_terminologies_fall_back_to_the_default_provider() {
    const OTHER: &str = "http://example.org/icd";
    let fallback = lookup_server(OTHER, "I10", "Essential hypertension").await;
    let snomed = lookup_server(SNOMED, "38341003", "Hypertensive disorder").await;

    let fallback_cfg = provider_cfg(&fallback.uri());
    let snomed_cfg = provider_cfg(&snomed.uri());
    let cfg = external(
        &[("default", &fallback_cfg), ("snomed", &snomed_cfg)],
        &[(SNOMED, "snomed")],
    );
    let service = service_with(
        TerminologyRouter::build(&cfg)
            .expect("build router")
            .expect("router"),
    )
    .await;

    service
        .get_term(OTHER, "I10", None, None)
        .await
        .expect("an unrouted terminology reaches the default provider");
    assert_eq!(fallback.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        snomed.received_requests().await.unwrap().len(),
        0,
        "the routed provider must not see an unrouted terminology"
    );
}

/// Route keys match case-insensitively (`SNOMED-CT` ≡ `snomed-ct`), so an
/// archetype's terminology id spelling does not silently change the server.
#[tokio::test]
async fn route_keys_are_case_insensitive() {
    let snomed = lookup_server("SNOMED-CT", "38341003", "Hypertensive disorder").await;
    let snomed_cfg = provider_cfg(&snomed.uri());
    let cfg = external(
        &[
            ("default", &provider_cfg("http://unused.invalid/fhir")),
            ("snomed", &snomed_cfg),
        ],
        &[("snomed-ct", "snomed")],
    );
    let service = service_with(
        TerminologyRouter::build(&cfg)
            .expect("build router")
            .expect("router"),
    )
    .await;
    service
        .get_term("SNOMED-CT", "38341003", None, None)
        .await
        .expect("an upper-case terminology id matches a lower-case route key");
    assert_eq!(snomed.received_requests().await.unwrap().len(), 1);
}

/// The AQL `TERMINOLOGY('expand', …)` seam routes by the value-set URL, so two
/// value sets on two servers resolve in one instance (QUERY master03
/// §TERMINOLOGY).
#[tokio::test]
async fn the_aql_terminology_seam_routes_per_value_set() {
    let snomed = expand_server("http://vs.example/snomed-set", &["38341003"]).await;
    let loinc = expand_server("http://vs.example/loinc-set", &["8480-6"]).await;

    let snomed_cfg = provider_cfg(&snomed.uri());
    let loinc_cfg = provider_cfg(&loinc.uri());
    let cfg = external(
        &[("snomed", &snomed_cfg), ("loinc", &loinc_cfg)],
        &[
            ("http://vs.example/snomed-set", "snomed"),
            ("http://vs.example/loinc-set", "loinc"),
        ],
    );
    let service = service_with(
        TerminologyRouter::build(&cfg)
            .expect("build router")
            .expect("router"),
    )
    .await;

    let codes = ferroehr::aql::terminology::TerminologyExpander::expand(
        &service,
        "hl7.org/fhir/4.0",
        "http://vs.example/snomed-set",
    )
    .await
    .expect("expand the SNOMED value set");
    assert_eq!(codes, ["38341003"]);

    let codes = ferroehr::aql::terminology::TerminologyExpander::expand(
        &service,
        "hl7.org/fhir/4.0",
        "http://vs.example/loinc-set",
    )
    .await
    .expect("expand the LOINC value set");
    assert_eq!(codes, ["8480-6"]);

    assert_eq!(snomed.received_requests().await.unwrap().len(), 1);
    assert_eq!(loinc.received_requests().await.unwrap().len(), 1);
}

/// With no external terminology configured — the shipped default — no router
/// is built at all, so terminology stays on the in-process `openehr-term`
/// bundle and no remote call can happen.
#[tokio::test]
async fn the_disabled_default_builds_no_router_and_calls_nothing() {
    let unreachable = MockServer::start().await;
    let cfg = ExternalTerminologyConfig {
        // A provider IS configured — only `enabled` is off, which is the
        // switch that must decide.
        providers: BTreeMap::from([("default".to_owned(), provider_cfg(&unreachable.uri()))]),
        ..ExternalTerminologyConfig::default()
    };
    assert!(!cfg.enabled, "external terminology is off by default");
    assert!(
        TerminologyRouter::build(&cfg).expect("build").is_none(),
        "the disabled default materialises no terminology server"
    );

    // A bare service answers the bundle terminologies exactly as before and
    // never reaches out.
    let db = testkit::db().await.expect("testkit database");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    assert!(
        service
            .has_term("ISO_3166-1", "NL", None)
            .await
            .expect("bundle lookup"),
        "the openEHR bundle still answers with no external provider"
    );
    let unknown = service
        .get_term("http://snomed.info/sct", "38341003", None, None)
        .await;
    assert!(
        unknown.is_err(),
        "an unknown terminology stays a bundle precondition failure, not a remote call"
    );
    assert_eq!(
        unreachable.received_requests().await.unwrap().len(),
        0,
        "nothing was sent to the configured-but-disabled server"
    );
}

/// A provider with an `oauth2_client` obtains a client-credentials token
/// (RFC 6749 §4.4) and sends it as a bearer credential on every FHIR
/// operation; the token is cached, so a second operation costs no second
/// grant.
#[tokio::test]
async fn oauth2_client_credentials_token_is_obtained_and_reused() {
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        // client_secret_basic (RFC 6749 §2.3.1) — credentials in the header,
        // never the body.
        .and(header_exists("authorization"))
        .and(body_string_contains("grant_type=client_credentials"))
        // The configured scope reaches the token request (RFC 6749 §4.4.2);
        // the matcher stops at the scope name because the form encoder's
        // treatment of `/` and `*` is its own business, not this test's.
        .and(body_string_contains("scope=system"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-abc",
            "token_type": "bearer",
            "expires_in": 3600
        })))
        .mount(&idp)
        .await;

    let ts = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/CodeSystem/$lookup"))
        .and(header("authorization", "Bearer tok-abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "display", "valueString": "Hypertensive disorder"}]
        })))
        .mount(&ts)
        .await;

    let mut ts_cfg = provider_cfg(&ts.uri());
    ts_cfg.oauth2_client = Some("ts-client".to_owned());
    let cfg = ExternalTerminologyConfig {
        enabled: true,
        providers: BTreeMap::from([("default".to_owned(), ts_cfg)]),
        oauth2_clients: BTreeMap::from([(
            "ts-client".to_owned(),
            TerminologyOauth2Config {
                token_url: format!("{}/token", idp.uri()),
                client_id: "ferroehr-cdr".to_owned(),
                client_secret: Some(ferroehr::config::secret::Secret::new("s3cret")),
                client_secret_file: None,
                scopes: vec!["system/*.read".to_owned()],
                refresh_leeway_secs: 30,
                auth_method: Oauth2AuthMethod::ClientSecretBasic,
            },
        )]),
        ..ExternalTerminologyConfig::default()
    };
    let service = service_with(
        TerminologyRouter::build(&cfg)
            .expect("build router")
            .expect("router"),
    )
    .await;

    for _ in 0..2 {
        service
            .get_term(SNOMED, "38341003", None, None)
            .await
            .expect("an authenticated lookup succeeds");
    }
    assert_eq!(
        ts.received_requests().await.unwrap().len(),
        2,
        "both lookups carried the bearer credential (an unauthenticated one would not match)"
    );
    assert_eq!(
        idp.received_requests().await.unwrap().len(),
        1,
        "the token is cached — a second operation must not re-run the grant"
    );
}

/// A token endpoint that refuses the grant surfaces as a typed provider error
/// — never a silent unauthenticated request.
#[tokio::test]
async fn a_refused_grant_is_a_typed_error_not_an_anonymous_request() {
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "invalid_client"
        })))
        .mount(&idp)
        .await;

    let ts = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "display", "valueString": "should never be reached"}]
        })))
        .mount(&ts)
        .await;

    let mut ts_cfg = provider_cfg(&ts.uri());
    ts_cfg.oauth2_client = Some("ts-client".to_owned());
    let cfg = ExternalTerminologyConfig {
        enabled: true,
        providers: BTreeMap::from([("default".to_owned(), ts_cfg)]),
        oauth2_clients: BTreeMap::from([(
            "ts-client".to_owned(),
            TerminologyOauth2Config {
                token_url: format!("{}/token", idp.uri()),
                client_id: "ferroehr-cdr".to_owned(),
                client_secret: Some(ferroehr::config::secret::Secret::new("wrong")),
                client_secret_file: None,
                scopes: Vec::new(),
                refresh_leeway_secs: 30,
                auth_method: Oauth2AuthMethod::ClientSecretBasic,
            },
        )]),
        ..ExternalTerminologyConfig::default()
    };
    let service = service_with(
        TerminologyRouter::build(&cfg)
            .expect("build router")
            .expect("router"),
    )
    .await;

    let err = service
        .get_term(SNOMED, "38341003", None, None)
        .await
        .expect_err("a refused grant must fail the call");
    // The refused grant is a TYPED failure — never an anonymous retry — while
    // the operator detail (which client, and the authorization server's own
    // error) stays on the trace record, off the wire body.
    assert_eq!(
        err.status,
        ferroehr::service::status::CallStatusType::Exception,
        "got {err:?}"
    );
    assert!(
        !err.message.contains("client-credentials grant failed")
            && !err.message.contains("ts-client"),
        "the deployment's credential configuration must not reach the wire body, got: {}",
        err.message
    );
    assert_eq!(
        ts.received_requests().await.unwrap().len(),
        0,
        "no request reaches the terminology server without a token"
    );
}

/// `client_secret_post` puts the credentials in the token-request body
/// instead of the `Authorization` header (RFC 6749 §2.3.1).
#[tokio::test]
async fn client_secret_post_sends_credentials_in_the_body() {
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("client_id=ferroehr-cdr"))
        .and(body_string_contains("client_secret=s3cret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-post",
            "token_type": "bearer",
            "expires_in": 3600
        })))
        .mount(&idp)
        .await;

    let ts = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer tok-post"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "display", "valueString": "ok"}]
        })))
        .mount(&ts)
        .await;

    let mut ts_cfg = provider_cfg(&ts.uri());
    ts_cfg.oauth2_client = Some("ts-client".to_owned());
    let cfg = ExternalTerminologyConfig {
        enabled: true,
        providers: BTreeMap::from([("default".to_owned(), ts_cfg)]),
        oauth2_clients: BTreeMap::from([(
            "ts-client".to_owned(),
            TerminologyOauth2Config {
                token_url: format!("{}/token", idp.uri()),
                client_id: "ferroehr-cdr".to_owned(),
                client_secret: Some(ferroehr::config::secret::Secret::new("s3cret")),
                client_secret_file: None,
                scopes: Vec::new(),
                refresh_leeway_secs: 30,
                auth_method: Oauth2AuthMethod::ClientSecretPost,
            },
        )]),
        ..ExternalTerminologyConfig::default()
    };
    let service = service_with(
        TerminologyRouter::build(&cfg)
            .expect("build router")
            .expect("router"),
    )
    .await;
    service
        .get_term(SNOMED, "38341003", None, None)
        .await
        .expect("the post-form grant authenticates the lookup");
    assert_eq!(idp.received_requests().await.unwrap().len(), 1);
}

/// A `ValueSet/$expand` server answering only for `value_set_url`.
async fn expand_server(value_set_url: &str, codes: &[&str]) -> MockServer {
    let server = MockServer::start().await;
    let contains: Vec<_> = codes
        .iter()
        .map(|c| json!({ "system": "s", "code": c, "display": c }))
        .collect();
    Mock::given(method("GET"))
        .and(path("/ValueSet/$expand"))
        .and(query_param("url", value_set_url))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "ValueSet",
            "status": "active",
            "expansion": { "timestamp": "2026-08-04T00:00:00Z", "contains": contains }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    server
}

/// A service over a throwaway database with `router` installed. The database
/// comes from the shared harness — never a per-test container.
async fn service_with(router: TerminologyRouter) -> FerroEhrService {
    let db = testkit::db().await.expect("testkit database");
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_terminology_router(Arc::new(router))
}
