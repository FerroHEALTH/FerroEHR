// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Deprovisioning a professional who holds care relationships (#5628),
//! through the real callers: the SCIM client deactivates, reactivates and
//! deletes the User, the ADT system rosters it through the Demographic API,
//! and the professional's own login is read on a second replica.
//!
//! Revoking access is never blocked: a deactivation and a deletion need no
//! `write_care_relationship` grant, since neither changes what the care
//! relationships say. A deactivation is a lifecycle state, and a deletion is
//! a logical `deleted` version whose earlier versions keep them (RM common
//! `master06-change_control_package.adoc` §Version Lifecycle, §Logical
//! Deletion; RFC 7644 §3.6 answers `404` from then on). Every request reads
//! the binding with no cache, so the next request of the login is refused on
//! every replica. No openEHR spec governs SCIM or who writes a care
//! relationship: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::versioning::object_version_id::parse_uid_based_id;
use http::StatusCode;
use serde_json::{Value, json};

use super::care_teams::{
    ADT, carries_member_of, group, id_of, member_of, read_person, relationships, write_person,
};
use super::default_definitions::{seeded, zero_config};
use super::{
    AUDIENCE, CLIENT, LOGIN, MACHINES, OTHER, OTHER_LOGIN, OTHER_PERSONS, PERSONS, ROLE_CODES,
    ROLES, SCOPE, call, create_body, patch_op,
};
use crate::common;
use crate::policy_fixture::{capability, grant, scope};

/// The router over a fresh database holding the default definitions, where
/// [`CLIENT`] provisions under [`LOGIN`] and holds `write_care_relationship`,
/// [`OTHER`] provisions under [`OTHER_LOGIN`] without it, both may give a
/// User a role of [`ROLES`], and [`ADT`] reaches the Demographic API and
/// holds `write_care_relationship`.
async fn hospital() -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let config = zero_config();
    let (db, pool) = common::migrated_pool().await;
    let service = Arc::new(seeded(&config, &pool).await);
    let provisioning = |binding_issuer: &str| {
        let mut items = vec![
            scope("binding_issuer", binding_issuer),
            scope("token_scope", SCOPE),
            scope("role_terminology", ROLES),
        ];
        items.extend(ROLE_CODES.iter().map(|role| scope("role", role)));
        capability("scim_provision", &items)
    };
    let care = capability("write_care_relationship", &[]);
    grant(
        &service,
        MACHINES,
        CLIENT,
        &[provisioning(LOGIN), care.clone()],
    )
    .await;
    grant(&service, MACHINES, OTHER, &[provisioning(OTHER_LOGIN)]).await;
    let demographic = capability("access_demographic", &[]);
    grant(&service, MACHINES, ADT, &[demographic, care]).await;
    let app = common::router_with(config, Arc::clone(&service)).await;
    (db, service, app)
}

/// A second replica of the deployment over the database of `db`: its own
/// service and router, sharing nothing with the first but the database.
async fn replica(db: &testkit::TestDb) -> Router {
    let config = zero_config();
    let service = Arc::new(seeded(&config, &db.pool()).await);
    common::router_with(config, service).await
}

/// A nurse `user_name` of `client` whose login key is `key`, so the login
/// binds to a professional; the User's id.
async fn nurse(app: &Router, client: &str, user_name: &str, key: &str) -> String {
    let mut body = create_body(user_name, user_name, key);
    body["roles"] = json!([{ "value": "NURSE" }]);
    let (status, _, created) = call(app, "POST", "/Users", client, Some(&body), &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["id"].as_str().expect("an id").to_owned()
}

/// A bearer of the professional whose login key is `key` at the login of
/// `client`'s Users.
fn login(client: &str, key: &str) -> String {
    let issuer = if client == OTHER {
        OTHER_PERSONS
    } else {
        PERSONS
    };
    let exp = jiff::Timestamp::now().as_second() + 3600;
    common::bearer(&json!({ "iss": issuer, "aud": AUDIENCE, "exp": exp, "sub": key }))
}

/// `PATCH /Users/{user}` setting `active`, as `client`; the status and body.
async fn set_active(app: &Router, client: &str, user: &str, active: bool) -> (StatusCode, Value) {
    let change = patch_op(json!([{ "op": "replace", "path": "active", "value": active }]));
    let (status, _, body) = call(
        app,
        "PATCH",
        &format!("/Users/{user}"),
        client,
        Some(&change),
        &[],
    )
    .await;
    (status, body)
}

/// The ADT system rosters the `PERSON` `user` on `team`.
async fn roster(app: &Router, user: &str, team: &str) {
    let (status, version, mut person) = read_person(app, ADT, user).await;
    assert_eq!(status, StatusCode::OK, "the ADT system reads the PERSON");
    let mut held = person["relationships"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    held.push(member_of(user, team));
    person["relationships"] = Value::Array(held);
    let (status, answer) = write_person(app, ADT, (user, &version), &person).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{answer}");
}

/// The login `key` of a User of `client` is admitted, or refused with `code`.
async fn admitted(app: &Router, (client, key): (&str, &str), code: Option<&str>) {
    let (status, body) = crate::party_binding_http::list(app, &login(client, key)).await;
    match code {
        None => assert_eq!(status, StatusCode::OK, "{body}"),
        Some(code) => {
            assert_eq!(status, StatusCode::FORBIDDEN, "{code}: {body}");
            assert!(body.contains(code), "{code}: {body}");
        }
    }
}

/// A SCIM client without `write_care_relationship` deactivates, reactivates
/// and deletes a professional the ADT system rostered on a ward. Each
/// succeeds and leaves the relationships as they were; the login is refused
/// on the next request on another replica while inactive and after the
/// deletion, and admitted again once reactivated. The deleted User answers
/// `404` and its earlier versions keep the roster.
#[tokio::test]
async fn deprovisioning_a_rostered_professional_needs_no_grant_and_reaches_every_replica() {
    let (db, service, app) = hospital().await;
    let other = replica(&db).await;
    let key = (OTHER, "bram-login");
    let user = nurse(&app, OTHER, "bram", key.1).await;
    let (_, ward) = group(&app, OTHER, "Ward 3", &[], false).await;
    roster(&app, &user, &id_of(&ward)).await;
    let rostered = relationships(&app, &user).await;
    assert_eq!(rostered.iter().filter(|r| carries_member_of(r)).count(), 1);
    admitted(&other, key, None).await;

    let (status, body) = set_active(&app, OTHER, &user, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["active"], json!(false), "{body}");
    assert_eq!(relationships(&app, &user).await, rostered);
    admitted(&other, key, Some("refused-inactive")).await;

    let (status, body) = set_active(&app, OTHER, &user, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(relationships(&app, &user).await, rostered);
    admitted(&other, key, None).await;

    let (_, last, _) = read_person(&app, ADT, &user).await;
    let path = format!("/Users/{user}");
    let (status, _, deleted) = call(&app, "DELETE", &path, OTHER, None, &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{deleted}");
    let (status, _, gone) = call(&app, "GET", &path, OTHER, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");
    admitted(&other, key, Some("refused-deprovisioned")).await;
    admitted(&app, key, Some("refused-deprovisioned")).await;

    let earlier = service
        .party_get(
            PartyKind::Person,
            parse_uid_based_id(&last).expect("a version uid"),
            None,
        )
        .await
        .expect("the version before the deletion is kept");
    let kept = earlier.body["relationships"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(kept, rostered, "the history keeps the roster");
}

/// SCIM never changes a care relationship: deactivating a User drops its
/// plain Group memberships, keeps its care-team membership, and the
/// reactivated User holds that membership as before.
#[tokio::test]
async fn a_deactivation_keeps_a_care_team_membership_and_drops_a_plain_one() {
    let (_db, _service, app) = hospital().await;
    let user = nurse(&app, CLIENT, "carla", "carla-login").await;
    let (status, team) = group(&app, CLIENT, "Psychiatry", &[&user], true).await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let (status, shift) = group(&app, CLIENT, "Night shift", &[&user], false).await;
    assert_eq!(status, StatusCode::CREATED, "{shift}");
    let held = relationships(&app, &user).await;
    assert_eq!(held.len(), 2, "both memberships are held");
    let care: Vec<Value> = held.into_iter().filter(carries_member_of).collect();
    assert_eq!(care.len(), 1, "one membership is a care relationship");

    let (status, body) = set_active(&app, CLIENT, &user, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(relationships(&app, &user).await, care);
    let (status, body) = set_active(&app, CLIENT, &user, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(relationships(&app, &user).await, care);
}

/// The ADT system cannot undo a revocation: a roster written over a
/// deactivated User would reactivate its `PERSON`, so it is refused and the
/// login stays refused.
#[tokio::test]
async fn the_adt_system_does_not_reactivate_a_deactivated_professional() {
    let (_db, _service, app) = hospital().await;
    let key = (OTHER, "dirk-login");
    let user = nurse(&app, OTHER, "dirk", key.1).await;
    let (_, ward) = group(&app, OTHER, "Ward 4", &[], false).await;
    let (status, body) = set_active(&app, OTHER, &user, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, version, mut person) = read_person(&app, ADT, &user).await;
    assert_eq!(status, StatusCode::OK, "the ADT system reads the PERSON");
    person["relationships"] = json!([member_of(&user, &id_of(&ward))]);
    let (status, refused) = write_person(&app, ADT, (&user, &version), &person).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(relationships(&app, &user).await, Vec::<Value>::new());
    admitted(&app, key, Some("refused-inactive")).await;
}
