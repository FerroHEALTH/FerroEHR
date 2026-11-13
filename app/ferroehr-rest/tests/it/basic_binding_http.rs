// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A Basic user is a person (#5009): `[[authz.party_binding.basic_user]]`
//! maps it by the binding key its `PERSON` carries, never by a party id, so
//! it binds whatever `HIER_OBJECT_ID` FerroEHR issued, commits under that
//! party's committer, is refused while no `PERSON` carries its key, and stays
//! below every `[authz.assurance] minimum`, and holds the roles and scopes
//! the `hold_roles` and `hold_scopes` grants of its `PERSON`'s `ROLE`s name
//! (#5438), never a privileged role. Driven through the assembled router over
//! a real `FerroEhrService` on a real `PostgreSQL` 18.
//!
//! RM common `org.openehr.rm.common.audit_details.adoc` lets the server write
//! who committed; how a credential binds a party is governed by no openEHR
//! spec: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::config::binding::BasicUserKey;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::party_binding::source::BindingSource;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::{Authenticator, Principal};
use ferrohealth_smart::assurance::{AssuranceLevel, AssuranceSection};
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
    config.auth.shared.basic = Some(common::basic_section(&[(
        USER,
        common::hash_password("pw").as_str(),
    )]));
    if mapped {
        config.party_binding.basic_user = vec![BasicUserKey {
            user: USER.to_owned(),
            key_issuer: LOGIN.to_owned(),
            key: KEY.to_owned(),
        }];
    }
    config
}

/// Grants the `PERSON` at `person_uid` the [`common::FULL_SCOPES`] a Basic
/// user holds through `hold_scopes`.
async fn scoped(svc: &FerroEhrService, person_uid: &str) {
    common::allowing(svc, &[Operation::HoldScopes]);
    svc.grant_person(&ferroehr::party_binding::grants::bootstrap::PersonGrant {
        person: vo(person_uid).0,
        operation: Operation::HoldScopes,
        elements: common::FULL_SCOPES
            .split_whitespace()
            .map(|scope| ("scope".to_owned(), scope.to_owned()))
            .collect(),
    })
    .await
    .expect("the PERSON holds the scopes");
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
        common::standing(&svc),
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
    scoped(&svc, &person_uid).await;
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
    let (person_uid, _) = professional(&svc, KEY, "532").await;
    scoped(&svc, &person_uid).await;
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

/// A `DV_CODED_TEXT` of the operation terminology.
fn operation_code(code: &str) -> Value {
    serde_json::json!({ "_type": "DV_CODED_TEXT", "value": code,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
            "code_string": code } })
}

/// A credentials element named by the terminology's `code`, holding `text`.
fn named(code: &str, text: &str) -> Value {
    serde_json::json!({ "_type": "ELEMENT", "archetype_node_id": "at0030",
        "name": operation_code(code), "value": { "_type": "DV_TEXT", "value": text } })
}

/// A capability granting `operation` with the credentials `items`.
fn capability(operation: &str, items: Vec<Value>) -> Value {
    let mut all = vec![
        serde_json::json!({ "_type": "ELEMENT", "archetype_node_id": "at0021",
        "name": { "_type": "DV_TEXT", "value": "operation" }, "value": operation_code(operation) }),
    ];
    all.extend(items);
    serde_json::json!({ "_type": "CAPABILITY", "archetype_node_id": "at0020",
        "name": { "_type": "DV_TEXT", "value": "grant" },
        "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0022",
            "name": { "_type": "DV_TEXT", "value": "terms" }, "items": all } })
}

/// Commits, as the server, a `ROLE` of a deployment's own archetype the
/// `PERSON` `person` performs, holding `capabilities`.
async fn grant(svc: &FerroEhrService, person: &str, capabilities: Vec<Value>) {
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.ward_access.v1";
    let role = serde_json::json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype }, "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "Ward access" },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "name" },
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "details" },
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "value": { "_type": "DV_TEXT", "value": "ward access" } }] } }],
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                       "id": { "_type": "HIER_OBJECT_ID", "value": person } },
        "capabilities": capabilities
    });
    Box::pin(svc.party_create(
        PartyKind::Role,
        crate::party_binding_http::write(&role, "532", None),
    ))
    .await
    .expect("the ROLE commits");
}

/// The status of a template listing as `credential`, and the principal it
/// ran as.
async fn listed(app: &Router, credential: &str) -> (StatusCode, Option<Principal>) {
    let resp = tower::ServiceExt::oneshot(
        app.clone(),
        common::req(
            "GET",
            format!("{}/definition/template/adl1.4", common::BASE),
        )
        .auth(credential)
        .accept("application/json")
        .build(),
    )
    .await
    .expect("a response");
    (resp.status(), resp.extensions().get::<Principal>().cloned())
}

#[tokio::test]
async fn a_bound_basic_user_holds_the_roles_and_scopes_its_grants_name() {
    let (_db, pool) = common::migrated_pool().await;
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_binding_mapping(crate::party_binding_http::mapping())
            .with_grant_ceiling(GrantCeiling::new([
                Operation::HoldRoles,
                Operation::HoldScopes,
            ])),
    );
    let app = app(&svc, deployment(true)).await;
    let credential = common::basic_credential(USER);
    let (person_uid, _) = professional(&svc, KEY, "532").await;
    let person = vo(&person_uid).0.to_string();

    let (status, principal) = listed(&app, &credential).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no scope reads a template");
    let principal = principal.expect("the refusal names its caller");
    assert!(principal.scopes.is_empty() && principal.roles.is_empty());

    // NOTE: the grant ceiling reserves no role here, so ADMIN is stored; the
    // request still never holds it, since admin_role is a privileged role.
    grant(
        &svc,
        &person,
        vec![
            capability(
                "hold_scopes",
                common::FULL_SCOPES
                    .split_whitespace()
                    .map(|scope| named("scope", scope))
                    .collect(),
            ),
            capability(
                "hold_roles",
                vec![named("role", "ward_clerk"), named("role", "ADMIN")],
            ),
        ],
    )
    .await;
    let (status, principal) = listed(&app, &credential).await;
    assert_eq!(status, StatusCode::OK, "the granted scope reads a template");
    let principal = principal.expect("an authenticated request carries its principal");
    assert_eq!(principal.roles, ["WARD_CLERK"]);
    assert!(
        principal
            .scopes
            .iter()
            .any(|s| s == "user/template-*.cruds"),
        "{:?}",
        principal.scopes
    );
}

#[tokio::test]
async fn a_hold_roles_grant_of_a_privileged_role_is_refused_at_commit() {
    let (_db, pool) = common::migrated_pool().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_binding_mapping(crate::party_binding_http::mapping())
        .with_grant_ceiling(
            GrantCeiling::new([Operation::HoldRoles]).with_reserved_roles(
                ferroehr::config::FerroEhrConfig::default()
                    .authz
                    .rbac
                    .privileged_roles(),
            ),
        );
    let (person_uid, _) = professional(&svc, KEY, "532").await;
    let person = vo(&person_uid).0.to_string();
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.ward_access.v1";
    let role = serde_json::json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype }, "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "Ward access" },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "name" },
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "details" },
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "value": { "_type": "DV_TEXT", "value": "ward access" } }] } }],
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                       "id": { "_type": "HIER_OBJECT_ID", "value": person } },
        "capabilities": [capability("hold_roles", vec![named("role", "admin")])]
    });
    let refused = Box::pin(svc.party_create(
        PartyKind::Role,
        crate::party_binding_http::write(&role, "532", None),
    ))
    .await
    .expect_err("admin_role is reserved")
    .flattened();
    assert!(
        matches!(refused, ferroehr::error::ServiceError::Forbidden(_)),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("privileged role"), "{refused}");
}
