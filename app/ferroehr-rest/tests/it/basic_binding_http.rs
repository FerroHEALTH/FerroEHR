// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A Basic user is a person (#5009): `[[authz.party_binding.basic_user]]`
//! maps it by the binding key its `PERSON` carries, never by a party id, so
//! it binds whatever `HIER_OBJECT_ID` FerroEHR issued, commits under that
//! party's committer, is refused while no `PERSON` carries its key, and stays
//! below every `[authz.assurance] minimum`. Driven through the assembled
//! router over a real `FerroEhrService` on a real `PostgreSQL` 18.
//!
//! RM common `org.openehr.rm.common.audit_details.adoc` lets the server write
//! who committed; how a credential binds a party is governed by no openEHR
//! spec: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::config::binding::BasicUserKey;
use ferroehr::party_binding::source::BindingSource;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferrohealth_smart::assurance::{AssuranceLevel, AssuranceSection};
use ferrohealth_smart::config::{BasicSection, BasicUser};
use http::{StatusCode, header};
use serde_json::Value;

use crate::binding_commits_http::{answered, ehr, request, with_body};
use crate::common;
use crate::party_binding_http::{LOGIN, config, professional, router_with, service, vo};

/// The Basic user the deployment maps.
const USER: &str = "nurse";
/// The binding key the Basic user's `PERSON` carries.
const KEY: &str = "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d";

/// The binding configuration of `party_binding_http` with the Basic user
/// [`USER`] mapped by [`KEY`] when `mapped`.
fn deployment(mapped: bool) -> AppConfig {
    let mut config = config(None);
    config.auth.shared.basic = Some(BasicSection {
        users: vec![BasicUser {
            user: USER.to_owned(),
            password_hash: Some(secrecy::SecretString::from(common::hash_password("pw"))),
            password_hash_file: None,
            scopes: common::FULL_SCOPES
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            roles: Vec::new(),
        }],
        cache_ttl: ferrohealth_config::units::Duration::from_secs(60),
    });
    if mapped {
        config.party_binding.basic_user = vec![BasicUserKey {
            user: USER.to_owned(),
            key_issuer: LOGIN.to_owned(),
            key: KEY.to_owned(),
        }];
    }
    config
}

/// The router over `svc` under `config`.
async fn app(svc: &Arc<FerroEhrService>, config: AppConfig) -> Router {
    let source: Arc<dyn BindingSource> = Arc::new(svc.party_source("demographic"));
    router_with(Arc::clone(svc), config, source).await
}

#[tokio::test]
async fn a_basic_user_with_no_mapping_builds_no_authenticator() {
    let (_db, svc) = service().await;
    let source: Arc<dyn BindingSource> = Arc::new(svc.party_source("demographic"));
    let built = Authenticator::new(
        &deployment(false),
        None,
        Some(source),
        Arc::new(ferrohealth_smart::sender::replay::MemoryReplayStore::new()),
    )
    .await;
    let Err(refused) = built else {
        panic!("an unmapped Basic user is refused");
    };
    assert!(refused.contains(USER), "{refused}");
}

#[tokio::test]
async fn a_mapped_basic_user_commits_as_the_person_carrying_its_key() {
    let (_db, svc) = service().await;
    let app = app(&svc, deployment(true)).await;
    let credential = common::basic_credential(USER);

    // No PERSON carries the key yet, so the Basic user binds nothing.
    let (status, body) = crate::party_binding_http::list(&app, &credential).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("refused-unbound"), "{body}");

    let (person_uid, _) = professional(&svc, KEY, "532").await;
    let ehr = ehr(&app, &credential).await;
    let (_, body) = answered(
        &app,
        with_body(
            "POST",
            &format!("/ehr/{ehr}/composition"),
            &credential,
            &crate::abac_e2e::composition(),
        ),
        StatusCode::CREATED,
        "the composition",
    )
    .await;
    let created: Value = serde_json::from_str(&body).expect("a COMPOSITION");
    let uid = created["uid"]["value"]
        .as_str()
        .expect("the version uid")
        .to_owned();
    let container = uid.split("::").next().expect("a container id");
    let (_, body) = answered(
        &app,
        request(
            "GET",
            &format!("/ehr/{ehr}/versioned_composition/{container}/version/{uid}"),
            &credential,
        ),
        StatusCode::OK,
        "the version",
    )
    .await;
    let version: Value = serde_json::from_str(&body).expect("an ORIGINAL_VERSION");
    let committer = &version["commit_audit"]["committer"];
    assert_eq!(committer["_type"], "PARTY_IDENTIFIED", "{committer}");
    assert_eq!(
        committer["external_ref"]["id"]["value"],
        vo(&person_uid).0.to_string(),
        "the version names the PERSON FerroEHR issued for the key: {committer}"
    );
}

#[tokio::test]
async fn a_mapped_basic_user_stays_below_an_assurance_minimum() {
    let (_db, svc) = service().await;
    professional(&svc, KEY, "532").await;
    let mut config = deployment(true);
    config.assurance = AssuranceSection {
        levels: [("loa-low".to_owned(), AssuranceLevel::Low)]
            .into_iter()
            .collect(),
        minimum: Some(AssuranceLevel::Low),
        ..AssuranceSection::default()
    };
    let app = app(&svc, config).await;
    let credential = common::basic_credential(USER);

    // A definition route carries no patient data, so the bound person passes.
    let (status, body) = crate::party_binding_http::list(&app, &credential).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // RFC 9470 §3: a Basic credential carries no level, so patient data
    // answers the step-up challenge, as for an unmapped Basic credential.
    let (status, headers, body) =
        common::send(&app, request("POST", "/ehr", &credential).build()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let challenge = headers
        .get(header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        challenge.contains("insufficient_user_authentication"),
        "{challenge}"
    );
}
