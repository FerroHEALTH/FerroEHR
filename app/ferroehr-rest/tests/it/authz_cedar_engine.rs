// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Cedar engine tests over this CDR's model: the shipped example policies
//! produce the expected golden decisions, and the Cedar engine is
//! **behaviourally identical** to the remote PDP over a corpus of requests
//! (the differential test — same `AuthzRequest`, same decision).
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
use ferroehr_rest::extensions::access::authz::remote_pdp;
use ferroehr_rest::extensions::access::authz::request::{
    AccessMode, AuthzRequest, ResourceKind, model,
};
use ferrohealth_authz::cedar::CedarEngine;
use ferrohealth_authz::engine::{AuthzError, PolicyEngine};
use ferrohealth_authz::remote::RemotePdp;
use ferrohealth_authz::request::{Attr, Decision};
use serde_json::Value;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/policies")
}

/// A composition create; a single patient is also the caller's patient claim,
/// as the enforcement point's subject gate guarantees before it asks.
fn composition<'a>(
    org: Option<&str>,
    patient: Option<Attr>,
    template: Option<Attr>,
) -> AuthzRequest<'a> {
    let subject_patient = match &patient {
        Some(Attr::One(p)) => Some(p.clone()),
        _ => None,
    };
    AuthzRequest {
        operation_id: "composition_create",
        kind: ResourceKind::Composition,
        access: AccessMode::Create,
        subject: "test-subject",
        roles: &[],
        scopes: &[],
        organization: org.map(str::to_owned),
        subject_patient,
        patient,
        template,
    }
}

/// Puts `req` to `engine`: `Ok(true)` permits; a request the policy request
/// refuses to carry (an empty set) is an `Err` like an engine failure.
async fn permits(engine: &dyn PolicyEngine, req: &AuthzRequest<'_>) -> Result<bool, String> {
    let subject = req.subject();
    let request = req.to_request(&subject).map_err(|e| e.to_string())?;
    engine
        .decide(&request)
        .await
        .map(|decision: Decision| decision.is_permit())
        .map_err(|e: AuthzError| e.to_string())
}

#[tokio::test]
async fn shipped_example_policies_golden_decisions() {
    let engine =
        CedarEngine::from_directory(model(), &examples_dir(), None).expect("load example policies");

    // Permit: known org, patient present, template on the allow-list.
    assert!(
        permits(
            &engine,
            &composition(
                Some("org1"),
                Some(Attr::One("p1".to_owned())),
                Some(Attr::One("org.openehr::vital_signs.v1".to_owned())),
            ),
        )
        .await
        .unwrap()
    );

    // Deny: template not on the allow-list.
    assert!(
        !permits(
            &engine,
            &composition(
                Some("org1"),
                Some(Attr::One("p1".to_owned())),
                Some(Attr::One("org.openehr::secret.v1".to_owned())),
            ),
        )
        .await
        .unwrap()
    );

    // Deny: no organization on the caller.
    assert!(
        !permits(
            &engine,
            &composition(
                None,
                Some(Attr::One("p1".to_owned())),
                Some(Attr::One("org.openehr::vital_signs.v1".to_owned())),
            ),
        )
        .await
        .unwrap()
    );

    // Deny: an explicitly revoked template (forbid overrides permit).
    assert!(
        !permits(
            &engine,
            &composition(
                Some("org1"),
                Some(Attr::One("p1".to_owned())),
                Some(Attr::One("org.openehr::revoked.v1".to_owned())),
            ),
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn bad_policy_dir_refuses_to_load() {
    let dir = std::env::temp_dir().join(format!("authz-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.cedar"), "this is not a cedar policy").unwrap();
    let e = CedarEngine::from_directory(model(), &dir, None).expect_err("must reject");
    let _cleanup = std::fs::remove_dir_all(&dir);
    assert!(matches!(e, AuthzError::Load(_)), "got {e}");
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
            oauth: None,
        },
        policy,
        ..AbacConfig::default()
    };
    let pdp = remote_pdp(&config).expect("build remote");
    (server, pdp)
}

/// Differential test: the same `AuthzRequest` corpus must yield identical
/// decisions from the embedded Cedar engine and the v1-compatible remote PDP,
/// proving the fan-out semantics (cartesian product, all-must-permit,
/// short-circuit, an empty set refused before either is asked) are
/// engine-independent.
#[tokio::test]
async fn cedar_and_remote_agree_over_corpus() {
    // Cedar policy encoding the identical rule: permit iff resource.patient == "ok".
    let cedar = CedarEngine::from_source(
        model(),
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
        let c = permits(&cedar, req).await;
        let r = permits(&remote, req).await;
        assert_eq!(
            c, r,
            "engines diverged for {:?}/{:?}: cedar={c:?} remote={r:?}",
            req.patient, req.template
        );
    }
}
