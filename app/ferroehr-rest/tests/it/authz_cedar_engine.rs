// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Cedar engine tests over this CDR's model: the shipped example policies
//! produce the expected golden decisions, and the fan-out over a set of
//! attribute values decides as the request model says.
use std::path::PathBuf;

use ferroehr_rest::extensions::access::authz::request::{
    AccessMode, AuthzRequest, ResourceKind, model,
};
use ferrohealth_authz::cedar::CedarEngine;
use ferrohealth_authz::engine::{AuthzError, PolicyEngine};
use ferrohealth_authz::request::{Attr, Decision};

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
        category: None,
        origin: None,
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

/// The fan-out semantics over one rule, permit iff `resource.patient ==
/// "ok"`: a set is the cartesian product of its values, every combination must
/// permit, and an empty set is refused before the engine is asked.
#[tokio::test]
async fn cedar_fans_out_over_a_corpus() {
    let cedar = CedarEngine::from_source(
        model(),
        r#"permit(principal, action, resource) when { resource has patient && resource.patient == "ok" };"#,
    )
    .expect("build cedar");

    let corpus: Vec<(AuthzRequest<'static>, Option<bool>)> = vec![
        (
            composition(Some("org1"), Some(Attr::One("ok".to_owned())), None),
            Some(true),
        ),
        (
            composition(Some("org1"), Some(Attr::One("bad".to_owned())), None),
            Some(false),
        ),
        (composition(None, None, None), Some(false)),
        (
            composition(
                Some("org1"),
                Some(Attr::Set(vec!["ok".to_owned(), "ok".to_owned()])),
                None,
            ),
            Some(true),
        ),
        (
            composition(
                Some("org1"),
                Some(Attr::Set(vec!["ok".to_owned(), "bad".to_owned()])),
                None,
            ),
            Some(false),
        ),
        (
            composition(Some("org1"), Some(Attr::Set(vec![])), None),
            None,
        ),
        // A template variation the patient-only rule ignores.
        (
            composition(
                Some("org1"),
                Some(Attr::One("ok".to_owned())),
                Some(Attr::Set(vec!["t1".to_owned(), "t2".to_owned()])),
            ),
            Some(true),
        ),
    ];

    for (req, expected) in &corpus {
        let decided = permits(&cedar, req).await;
        assert_eq!(
            decided.as_ref().ok().copied(),
            *expected,
            "{:?}/{:?}: {decided:?}",
            req.patient,
            req.template
        );
    }
}
