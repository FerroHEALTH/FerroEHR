// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `RemotePdp` wire-contract tests,
//! driven by `wiremock`: URL = base + policy name; a flat JSON body with exactly
//! the configured keys; 200 → permit, non-200 → deny; connect/timeout →
//! `AuthzError` (→ 500 at the PEP); cartesian fan-out order + short-circuit;
//! all-must-permit.

use std::collections::BTreeMap;

use ferroehr::config::authz::{AbacConfig, AbacEngineKind, AbacParam, PolicyRule, RemoteConfig};
use ferroehr_rest::extensions::access::authz::engine::{AuthzError, PolicyEngine};
use ferroehr_rest::extensions::access::authz::remote::RemotePdp;
use ferroehr_rest::extensions::access::authz::request::{
    AccessMode, Attr, AuthzRequest, Decision, ResourceKind,
};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Build a config whose remote PDP points at `server` (already slash-terminated)
/// with a single `composition` policy taking the given parameters.
fn config(server: &str, name: &str, parameters: Vec<AbacParam>) -> AbacConfig {
    let mut policy = BTreeMap::new();
    policy.insert(
        "composition".to_owned(),
        PolicyRule {
            name: name.to_owned(),
            parameters,
        },
    );
    AbacConfig {
        enabled: true,
        engine: AbacEngineKind::Remote,
        remote: RemoteConfig {
            server: Some(ferroehr::config::secret::SecretUrl::new(server)),
            connect_timeout_ms: 500,
            request_timeout_ms: 1000,
        },
        policy,
        ..AbacConfig::default()
    }
}

fn composition_req<'a>(patient: Option<Attr>, template: Option<Attr>) -> AuthzRequest<'a> {
    AuthzRequest {
        operation_id: "composition_create",
        kind: ResourceKind::Composition,
        access: AccessMode::Create,
        subject: "test-subject",
        roles: &[],
        scopes: &[],
        organization: Some("org-7".to_owned()),
        patient,
        template,
    }
}

#[tokio::test]
async fn url_is_base_plus_policy_name_and_body_has_exactly_configured_keys() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/v1/policy/execute/name/has_consent_template"))
        .and(body_json(json!({
            "organization": "org-7",
            "template": "t.en.v1",
        })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let base = format!("{}/rest/v1/policy/execute/name/", server.uri());
    let pdp = RemotePdp::new(&config(
        &base,
        "has_consent_template",
        vec![AbacParam::Organization, AbacParam::Template],
    ))
    .expect("build");

    // Patient is resolved but NOT a configured parameter → must not appear.
    let req = composition_req(
        Some(Attr::One("p-1".to_owned())),
        Some(Attr::One("t.en.v1".to_owned())),
    );
    assert_eq!(pdp.decide(&req).await.unwrap(), Decision::Permit);
}

/// A 4xx is the policy server DECIDING to refuse, so it denies.
#[tokio::test]
async fn client_error_denies() {
    for status in [401u16, 403, 404, 422] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let base = format!("{}/", server.uri());
        let pdp = RemotePdp::new(&config(&base, "p", vec![AbacParam::Patient])).expect("build");
        let req = composition_req(Some(Attr::One("p-1".to_owned())), None);
        assert_eq!(
            pdp.decide(&req).await.unwrap(),
            Decision::Deny,
            "status {status} must deny"
        );
    }
}

/// A 5xx is NOT a decision — the policy server says it failed. Reading it as a
/// deny would let a broken PDP silently refuse clinical access while looking
/// like policy, so it is a fail-closed error the PEP renders 500
/// (RFC 9110 §15.6: a 5xx means the server "is aware that it has erred").
///
/// A 3xx is equally not a decision: an authorization endpoint that redirects is
/// misconfigured, and following it would send the attribute body somewhere the
/// deployment never named.
#[tokio::test]
async fn a_server_error_is_not_a_decision() {
    for status in [500u16, 502, 503, 504, 301, 302] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let base = format!("{}/", server.uri());
        let pdp = RemotePdp::new(&config(&base, "p", vec![AbacParam::Patient])).expect("build");
        let req = composition_req(Some(Attr::One("p-1".to_owned())), None);
        let outcome = pdp.decide(&req).await;
        assert!(
            matches!(outcome, Err(AuthzError::Unreachable(_))),
            "status {status} must be a fail-closed error, not a deny: {outcome:?}"
        );
    }
}

#[tokio::test]
async fn connection_failure_is_fail_closed_error() {
    // A port nothing listens on → connect error → AuthzError (→ 500 at the PEP).
    let base = "http://127.0.0.1:1/"; // port 1: reliably refused
    let pdp = RemotePdp::new(&config(base, "p", vec![AbacParam::Patient])).expect("build");
    let req = composition_req(Some(Attr::One("p-1".to_owned())), None);
    let err = pdp.decide(&req).await.expect_err("must error");
    assert!(matches!(err, AuthzError::Unreachable(_)), "got {err:?}");
}

/// A resource kind with no configured policy DENIES: there is no policy to ask,
/// so the only safe answer is refusal. Boot validation requires a policy per
/// kind under `engine = remote`, making this branch unreachable in a booted
/// server — it is the fail-closed floor, not a routine path. No openEHR spec
/// governs authorization — our own design/extension.
#[tokio::test]
async fn unconfigured_kind_denies() {
    // Port 1 is reliably refused, so a deny here also proves no HTTP call is made.
    let base = "http://127.0.0.1:1/";
    let pdp = RemotePdp::new(&config(base, "p", vec![AbacParam::Patient])).expect("build");
    let req = AuthzRequest {
        operation_id: "query_execute_adhoc_query",
        kind: ResourceKind::Query,
        access: AccessMode::Execute,
        subject: "test-subject",
        roles: &[],
        scopes: &[],
        organization: None,
        patient: Some(Attr::One("p-1".to_owned())),
        template: None,
    };
    assert_eq!(pdp.decide(&req).await.unwrap(), Decision::Deny);
}

#[tokio::test]
async fn cartesian_fan_out_all_must_permit() {
    let server = MockServer::start().await;
    // Every combination permits → overall permit; 2 patients × 2 templates = 4.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(4)
        .mount(&server)
        .await;
    let base = format!("{}/", server.uri());
    let pdp = RemotePdp::new(&config(
        &base,
        "p",
        vec![AbacParam::Patient, AbacParam::Template],
    ))
    .expect("build");
    let req = composition_req(
        Some(Attr::Set(vec!["p1".to_owned(), "p2".to_owned()])),
        Some(Attr::Set(vec!["t1".to_owned(), "t2".to_owned()])),
    );
    assert_eq!(pdp.decide(&req).await.unwrap(), Decision::Permit);
}

#[tokio::test]
async fn fan_out_short_circuits_on_first_deny() {
    let server = MockServer::start().await;
    // The first combination (p1) is denied; the PDP must stop before p2.
    Mock::given(method("POST"))
        .and(deny_for_patient("p1"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(deny_for_patient("p2"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0) // must never be reached (short-circuit)
        .mount(&server)
        .await;
    let base = format!("{}/", server.uri());
    let pdp = RemotePdp::new(&config(&base, "p", vec![AbacParam::Patient])).expect("build");
    let req = composition_req(
        Some(Attr::Set(vec!["p1".to_owned(), "p2".to_owned()])),
        None,
    );
    assert_eq!(pdp.decide(&req).await.unwrap(), Decision::Deny);
}

#[tokio::test]
async fn empty_set_permits_without_any_call() {
    let base = "http://127.0.0.1:1/";
    let pdp = RemotePdp::new(&config(base, "p", vec![AbacParam::Patient])).expect("build");
    // An empty patient set (empty query result) → no combinations → permit.
    let req = composition_req(Some(Attr::Set(vec![])), None);
    assert_eq!(pdp.decide(&req).await.unwrap(), Decision::Permit);
}

/// A `wiremock` matcher: the request body's `patient` equals `who`.
fn deny_for_patient(who: &'static str) -> impl Fn(&Request) -> bool {
    move |req: &Request| {
        serde_json::from_slice::<Value>(&req.body)
            .ok()
            .and_then(|v| v.get("patient").and_then(Value::as_str).map(|p| p == who))
            .unwrap_or(false)
    }
}
