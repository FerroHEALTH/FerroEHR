// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hospital scenario on two replicas of one deployment: a change made
//! through one replica's public routes decides the next request on the
//! other once its notification arrives, with no restart. Each replica holds
//! copies of the grants, the caller facts, the policy and the `EHR_ACCESS`
//! settings, kept fresh by `LISTEN`/`NOTIFY` (#5459, #5509). No openEHR spec
//! governs the copies: our own design.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use ferroehr::system_log::sender::{AuditHandle, start};
use http::StatusCode;
use serde_json::json;

use crate::hospital::abac::{answered, read};
use crate::hospital::build::{audit, directive, eva_directives, settle, world};
use crate::hospital::world::{
    Answer, Doc, SCIM, World, admit, day, patch_op, replica, router, scim,
};

/// The second replica over the world's database, auditing to the same
/// store, its listeners connected; with its audit drain.
async fn second(world: &World) -> (Router, AuditHandle) {
    let (sender, drain) = start(audit(), None, Some(world.pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(replica(&world.config, &world.pool, Some(sender)).await);
    let connect = (*world.pool.connect_options()).clone();
    svc.ehr_access_listened().spawn_listener(connect.clone());
    svc.grants().listened().spawn_listener(connect.clone());
    svc.access_policy().listened().spawn_listener(connect);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !(svc.ehr_access_listened().is_listening()
        && svc.grants().listened().is_listening()
        && svc.access_policy().listened().is_listening())
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the listeners connect"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (router(&world.config, &svc).await, drain)
}

/// The read of `doc` by `caller` on `app`, polled until it answers `status`
/// or ten seconds pass: the other replica's notification is asynchronous.
async fn eventually(
    world: &World,
    app: &Router,
    (caller, doc): (&str, &Doc),
    status: StatusCode,
) -> Answer {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let answer = crate::hospital::world::call(
            app,
            caller,
            "GET",
            &format!(
                "{}/ehr/{}/composition/{}",
                crate::common::BASE,
                world.ehr,
                doc.vo
            ),
            Some("TREAT"),
            None,
        )
        .await;
        if answer.status == status || std::time::Instant::now() >= deadline {
            return answer;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Eva's opt-out, committed through the first replica, hides her record
/// from the access service on the second.
#[tokio::test]
async fn an_opt_out_committed_on_one_replica_decides_the_other() {
    let mut world = Box::pin(world()).await;
    let (other, _drain) = second(&world).await;
    let answer = eventually(
        &world,
        &other,
        ("client:access-service", &world.vitals),
        StatusCode::OK,
    )
    .await;
    answered(&answer, StatusCode::OK, None);
    let opt_out = directive("opt_out", "deny", &json!({ "emergency_override": true }));
    let opted = eva_directives(&world, None, &[opt_out]);
    settle(&mut world, &opted).await;
    let answer = eventually(
        &world,
        &other,
        ("client:access-service", &world.vitals),
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
}

/// The ADT system's discharge of Eva from Cardiology, written through the
/// first replica, refuses Dr. Jansen on the second once the grace has
/// passed.
#[tokio::test]
async fn a_discharge_written_on_one_replica_decides_the_other() {
    let mut world = Box::pin(world()).await;
    let (other, _drain) = second(&world).await;
    let answer = eventually(&world, &other, ("jansen", &world.encounter), StatusCode::OK).await;
    answered(&answer, StatusCode::OK, None);
    let (ward, old_from, old_until) = (day(-2), day(-40), day(-20));
    world.eva_version = admit(
        &world,
        &world.eva_version,
        &[
            ("Cardiology", &old_from, Some(&old_until)),
            ("Ward 3", &ward, None),
            ("Psychiatry", &ward, None),
        ],
    )
    .await;
    let answer = eventually(
        &world,
        &other,
        ("jansen", &world.encounter),
        StatusCode::FORBIDDEN,
    )
    .await;
    answered(
        &answer,
        StatusCode::FORBIDDEN,
        Some("refused-no-care-relationship"),
    );
}

/// SCIM's deactivation of the nurse through the first replica refuses her on
/// the second.
#[tokio::test]
async fn a_scim_deactivation_on_one_replica_refuses_on_the_other() {
    let world = Box::pin(world()).await;
    let (other, _drain) = second(&world).await;
    let answer = eventually(&world, &other, ("devries", &world.vitals), StatusCode::OK).await;
    answered(&answer, StatusCode::OK, None);
    let path = format!("/Users/{}", world.people["devries"].person);
    let body = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let (status, scimmed) = scim(&world.app, SCIM, "PATCH", &path, Some(&body)).await;
    assert_eq!(status, StatusCode::OK, "the deactivation: {scimmed}");
    let answer = eventually(
        &world,
        &other,
        ("devries", &world.vitals),
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN, "{}", answer.body);
    let first = read(&world, "devries", &world.vitals, Some("TREAT")).await;
    assert_eq!(first.status, StatusCode::FORBIDDEN, "{}", first.body);
}
