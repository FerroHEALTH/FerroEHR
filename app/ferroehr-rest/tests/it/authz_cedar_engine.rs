// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `CedarEngine` tests: the shipped
//! example policies produce the expected golden decisions, and the Cedar engine
//! is **behaviourally identical** to the remote PDP over a corpus of requests
//! (the differential test — same `AuthzRequest`, same `Decision`).
#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ferroehr::config::authz::{AbacConfig, AbacEngineKind, AbacParam, PolicyRule, RemoteConfig};
use ferroehr_rest::extensions::access::authz::cedar::CedarEngine;
use ferroehr_rest::extensions::access::authz::engine::PolicyEngine;
use ferroehr_rest::extensions::access::authz::remote::RemotePdp;
use ferroehr_rest::extensions::access::authz::request::{
    AccessMode, Attr, AuthzRequest, Decision, ResourceKind,
};
use serde_json::Value;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/policies")
}

fn composition<'a>(
    org: Option<&str>,
    patient: Option<Attr>,
    template: Option<Attr>,
) -> AuthzRequest<'a> {
    AuthzRequest {
        operation_id: "composition_create",
        kind: ResourceKind::Composition,
        access: AccessMode::Create,
        subject: "test-subject",
        roles: &[],
        scopes: &[],
        organization: org.map(str::to_owned),
        patient,
        template,
    }
}

#[tokio::test]
async fn shipped_example_policies_golden_decisions() {
    let engine = CedarEngine::new(&examples_dir(), None).expect("load example policies");

    // Permit: known org, patient present, template on the allow-list.
    assert_eq!(
        engine
            .decide(&composition(
                Some("org1"),
                Some(Attr::One("p1".to_owned())),
                Some(Attr::One("org.openehr::vital_signs.v1".to_owned())),
            ))
            .await
            .unwrap(),
        Decision::Permit
    );

    // Deny: template not on the allow-list.
    assert_eq!(
        engine
            .decide(&composition(
                Some("org1"),
                Some(Attr::One("p1".to_owned())),
                Some(Attr::One("org.openehr::secret.v1".to_owned())),
            ))
            .await
            .unwrap(),
        Decision::Deny
    );

    // Deny: no organization on the caller.
    assert_eq!(
        engine
            .decide(&composition(
                None,
                Some(Attr::One("p1".to_owned())),
                Some(Attr::One("org.openehr::vital_signs.v1".to_owned())),
            ))
            .await
            .unwrap(),
        Decision::Deny
    );

    // Deny: an explicitly revoked template (forbid overrides permit).
    assert_eq!(
        engine
            .decide(&composition(
                Some("org1"),
                Some(Attr::One("p1".to_owned())),
                Some(Attr::One("org.openehr::revoked.v1".to_owned())),
            ))
            .await
            .unwrap(),
        Decision::Deny
    );
}

#[tokio::test]
async fn bad_policy_dir_refuses_to_load() {
    let dir = std::env::temp_dir().join(format!("authz-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.cedar"), "this is not a cedar policy").unwrap();
    let e = CedarEngine::new(&dir, None).expect_err("must reject");
    let _cleanup = std::fs::remove_dir_all(&dir);
    assert!(format!("{e}").contains("policy load failed"), "got {e}");
}

/// Build a remote PDP whose mock permits iff the request body's `patient == "ok"`
/// — the same rule the Cedar policy below encodes.
async fn remote_permit_when_patient_ok() -> (MockServer, RemotePdp) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|req: &Request| {
            let ok = serde_json::from_slice::<Value>(&req.body)
                .ok()
                .and_then(|v| v.get("patient").and_then(Value::as_str).map(|p| p == "ok"))
                .unwrap_or(false);
            ResponseTemplate::new(if ok { 200 } else { 403 })
        })
        .mount(&server)
        .await;
    let mut policy = BTreeMap::new();
    policy.insert(
        "composition".to_owned(),
        PolicyRule {
            name: "p".to_owned(),
            parameters: vec![AbacParam::Patient],
        },
    );
    let config = AbacConfig {
        enabled: true,
        engine: AbacEngineKind::Remote,
        remote: RemoteConfig {
            server: Some(ferroehr::config::secret::SecretUrl::new(format!(
                "{}/",
                server.uri()
            ))),
            connect_timeout_ms: 500,
            request_timeout_ms: 1000,
        },
        policy,
        ..AbacConfig::default()
    };
    let pdp = RemotePdp::new(&config).expect("build remote");
    (server, pdp)
}

/// Differential test: the same `AuthzRequest` corpus must yield identical
/// decisions from the embedded Cedar engine and the v1-compatible remote PDP,
/// proving the fan-out semantics (cartesian product, all-must-permit,
/// short-circuit, empty→permit) are engine-independent.
#[tokio::test]
async fn cedar_and_remote_agree_over_corpus() {
    // Cedar policy encoding the identical rule: permit iff resource.patient == "ok".
    let cedar = CedarEngine::from_policy_src(
        r#"permit(principal, action, resource) when { resource has patient && resource.patient == "ok" };"#,
    )
    .expect("build cedar");
    let (_server, remote) = remote_permit_when_patient_ok().await;

    let corpus: Vec<AuthzRequest<'static>> = vec![
        composition(Some("org1"), Some(Attr::One("ok".to_owned())), None),
        composition(Some("org1"), Some(Attr::One("bad".to_owned())), None),
        composition(None, None, None),
        composition(
            Some("org1"),
            Some(Attr::Set(vec!["ok".to_owned(), "ok".to_owned()])),
            None,
        ),
        composition(
            Some("org1"),
            Some(Attr::Set(vec!["ok".to_owned(), "bad".to_owned()])),
            None,
        ),
        composition(Some("org1"), Some(Attr::Set(vec![])), None),
        // Template variation the [Patient]-only remote policy (and the
        // patient-only Cedar rule) both ignore.
        composition(
            Some("org1"),
            Some(Attr::One("ok".to_owned())),
            Some(Attr::Set(vec!["t1".to_owned(), "t2".to_owned()])),
        ),
    ];

    for req in &corpus {
        let c = cedar.decide(req).await.unwrap();
        let r = remote.decide(req).await.unwrap();
        assert_eq!(
            c, r,
            "engines diverged for {:?}/{:?}: cedar={c:?} remote={r:?}",
            req.patient, req.template
        );
    }
}
