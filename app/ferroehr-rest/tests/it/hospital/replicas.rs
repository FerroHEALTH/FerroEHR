// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hospital scenario on two replicas of one deployment: a change made
//! through one replica's public routes decides the next request on the
//! other once its notification arrives, with no restart. Each replica holds
//! copies of the grants, the caller facts, the policy and the `EHR_ACCESS`
//! settings, kept fresh by `LISTEN`/`NOTIFY` (#5459, #5509). No openEHR spec
//! governs the copies: our own design.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use ferroehr::policy_register::listen::Listeners;
use ferroehr::system_log::sender::{AuditHandle, start};
use http::StatusCode;
use serde_json::json;
use sqlx::postgres::PgConnectOptions;

use crate::hospital::abac::{answered, read};
use crate::hospital::build::{audit, directive, eva_directives, settle, world};
use crate::hospital::ledger::{Expect, Ledger};
use crate::hospital::world::{
    Answer, Doc, ENCOUNTER, SCIM, World, admit, day, patch_op, replica, router, scim,
};

/// The second replica over the world's database, auditing to the same
/// store, its listeners connected; with its audit drain.
async fn second(world: &World) -> (Router, AuditHandle) {
    let (sender, drain) = start(audit(), None, world.pool.clone())
        .await
        .expect("the audit sender");
    let svc = Arc::new(replica(&world.config, &world.pool, Some(sender)).await);
    let connect = (*world.pool.connect_options()).clone();
    Listeners::on(
        connect,
        &[
            Arc::clone(svc.ehr_access_listened()),
            Arc::clone(svc.grants().listened()),
            Arc::clone(svc.access_policy().listened()),
        ],
    )
    .spawn();
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

/// A second replica whose caller facts copy hears no notification: its
/// grants listener connects to a database of its own, the longest lag a
/// notification can have, so its next care-decided request reads the
/// store's revisions before its decision is taken (#5806); its other copies
/// listen on the world's database. With its audit drain, and the database
/// its grants listener connects to.
async fn lagging(world: &World) -> (Router, AuditHandle, testkit::TestDb) {
    let (sender, drain) = start(audit(), None, world.pool.clone())
        .await
        .expect("the audit sender");
    let svc = Arc::new(replica(&world.config, &world.pool, Some(sender)).await);
    let elsewhere = testkit::db().await.expect("a second database");
    let lagging = PgConnectOptions::from_str(elsewhere.url()).expect("the test DSN parses");
    Listeners::on(lagging, &[Arc::clone(svc.grants().listened())]).spawn();
    let connect = (*world.pool.connect_options()).clone();
    Listeners::on(
        connect,
        &[
            Arc::clone(svc.ehr_access_listened()),
            Arc::clone(svc.access_policy().listened()),
        ],
    )
    .spawn();
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
    svc.grants().refresh().await.expect("the copy reads");
    (router(&world.config, &svc).await, drain, elsewhere)
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
    admit(
        &mut world,
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

/// A second replica none of whose copies hears a notification: every
/// listener connects to a database of its own, so what its requests decide
/// rests on what they read (#6048). With its audit drain, and that database.
async fn deaf(world: &World) -> (Router, AuditHandle, testkit::TestDb) {
    let (sender, drain) = start(audit(), None, world.pool.clone())
        .await
        .expect("the audit sender");
    let svc = Arc::new(replica(&world.config, &world.pool, Some(sender)).await);
    let elsewhere = testkit::db().await.expect("a second database");
    let listened: Vec<_> = svc
        .policy_listeners()
        .into_iter()
        .map(|(_, listened)| listened)
        .collect();
    Listeners::on(
        PgConnectOptions::from_str(elsewhere.url()).expect("the test DSN parses"),
        &listened,
    )
    .spawn();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !listened.iter().all(|listened| listened.is_listening()) {
        assert!(
            std::time::Instant::now() < deadline,
            "the listeners connect"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (router(&world.config, &svc).await, drain, elsewhere)
}

/// SCIM's deactivation of the nurse through the first replica refuses her
/// very next request on a replica that hears no notification: the request
/// reads the party store's revisions before it resolves her login (#6048).
#[tokio::test]
async fn a_scim_deactivation_refuses_the_next_request_of_a_replica_that_hears_nothing() {
    let world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let path = format!(
        "{}/ehr/{}/composition/{}",
        crate::common::BASE,
        world.ehr,
        world.vitals.vo
    );
    let read =
        || crate::hospital::world::call(&other, "devries", "GET", &path, Some("TREAT"), None);
    for _ in 0..2 {
        let answer = read().await;
        answered(&answer, StatusCode::OK, None);
    }
    let user = format!("/Users/{}", world.people["devries"].person);
    let body = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let (status, scimmed) = scim(&world.app, SCIM, "PATCH", &user, Some(&body)).await;
    assert_eq!(status, StatusCode::OK, "the deactivation: {scimmed}");
    let answer = read().await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN, "{}", answer.body);
}

/// The system log's records of the request `request_id`, as `(operation,
/// outcome)`, once at least one has been chained, or none after ten seconds.
async fn audited(world: &World, request_id: &str) -> Vec<(Option<String>, i16)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let rows: Vec<(Option<String>, i16)> = sqlx::query_as(
            "SELECT operation, outcome FROM audit.audit_event WHERE request_id = $1 \
             ORDER BY recorded_at",
        )
        .bind(request_id)
        .fetch_all(&world.pool)
        .await
        .expect("read the system log");
        if !rows.is_empty() || std::time::Instant::now() >= deadline {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Dr. Jansen's discharge from Cardiology, written through the first
/// replica, refuses his next read on a replica that has not heard of it. The
/// composition is read beside the decision's read of the store's revisions
/// (#5959), and the refusal is still the whole answer: the body carries
/// nothing of the composition, and the access log and the system log each
/// hold the one refusal of the request, as a refusal decided before any read
/// leaves them.
#[tokio::test]
async fn a_read_made_beside_a_refusing_decision_leaves_only_the_refusal() {
    let mut world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let (other, _drain, _elsewhere) = lagging(&world).await;
    let uri = format!(
        "{}/ehr/{}/composition/{}",
        crate::common::BASE,
        world.ehr,
        world.encounter.version
    );
    let read_on_other =
        || crate::hospital::world::call(&other, "jansen", "GET", &uri, Some("TREAT"), None);
    let answer = read_on_other().await;
    answered(&answer, StatusCode::OK, None);
    assert!(
        answer.body.contains("Cardiology encounter"),
        "the admitted read serves the composition: {}",
        answer.body
    );
    ledger.note(
        &world,
        "before the discharge",
        &answer,
        Expect::granted(&["treatment relationship"]),
    );

    let (ward, old_from, old_until) = (day(-2), day(-40), day(-20));
    admit(
        &mut world,
        &[
            ("Cardiology", &old_from, Some(&old_until)),
            ("Ward 3", &ward, None),
            ("Psychiatry", &ward, None),
        ],
    )
    .await;
    let refused = read_on_other().await;
    answered(
        &refused,
        StatusCode::FORBIDDEN,
        Some("refused-no-care-relationship"),
    );
    for content in ["Cardiology encounter", "Dr. Anna Jansen", ENCOUNTER] {
        assert!(
            !refused.body.contains(content),
            "the refusal carries nothing of the composition ({content}): {}",
            refused.body
        );
    }
    ledger.note(
        &world,
        "after the discharge",
        &refused,
        Expect::refused("refused-no-care-relationship", 7),
    );
    ledger.verify(&world).await;

    let records = audited(&world, &refused.request_id()).await;
    assert_eq!(
        records.len(),
        1,
        "one system log record of the refused request: {records:?}"
    );
    let (_, outcome) = &records[0];
    assert_ne!(*outcome, 0, "the record is the refusal: {records:?}");
}
