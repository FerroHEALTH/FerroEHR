// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outbox feed end to end over the assembled router (#4786): the events
//! after a reader's cursor in `seq` order, the page size, the acknowledgement
//! that moves the cursor, the refusal of a reader no `read_change_feed` grant
//! in the CDR names, of a caller whose own grant does not name it and of a
//! caller without the `__ferroehr.outbox` scope, and one access record per
//! read (#5439). No openEHR spec governs eventing: our own design/extension.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::extensions::outbox::{OutboxGate, reconcile};
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::caller_kinds_http::{audience, bearer, machines};
use crate::common;
use crate::common::BASE;
use crate::policy_fixture::{capability, grant, scope};

/// The reader the `read_change_feed` grant of the `bridge` client names.
const READER: &str = "ferrobridge";
/// A reader once registered that no grant names.
const FORMER: &str = "former";
/// The machines issuer of the feed's clients.
const MACHINES: &str = "https://outbox.machines.example";

/// An audited app whose `bridge` client holds the outbox scope alone and the
/// `read_change_feed` grant of `ferrobridge`, whose `other` client holds that
/// of another reader, and an administrator holding every other scope;
/// `former` is a registered reader no grant names. The outbox is written
/// while a reader is granted, as the binary writes it.
async fn app() -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let party = ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party);
    for outbox in [&pool, &party] {
        reconcile(outbox, &[READER.to_owned(), FORMER.to_owned()])
            .await
            .expect("the first boot");
        reconcile(outbox, &[READER.to_owned()])
            .await
            .expect("the boot that drops `former`");
    }
    let (sender, _handle): (_, AuditHandle) =
        start(common::local_audit(), None, Some(pool.clone()))
            .await
            .expect("the audit sender");
    let svc = crate::policy_fixture::granting(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone()))
            .with_outbox_gate(OutboxGate::WhileGranted),
    )
    .await;
    for (client, reader) in [("bridge", READER), ("other", "other")] {
        let reads = capability("read_change_feed", &[scope("reader", reader)]);
        grant(&svc, MACHINES, client, &[reads]).await;
    }
    // The replica's refresher reads the grants on the commits' notification.
    svc.grants().refresh().await.expect("the grants read");
    let svc = Arc::new(svc);
    let mut config = common::api_config(true);
    common::with_basic_users(&mut config, &svc, &pool, &[("root", &[])]).await;
    config.auth.shared.issuer.push(machines(
        MACHINES,
        ferrohealth_smart::profile::Preset::Rfc9068,
    ));
    let app = common::build_full(config, svc, None, Observability::default()).await;
    (db, pool, app)
}

/// The credential of `user`: the Basic administrator `root`, or an app-only
/// token of the client `user` holding the outbox scope alone.
fn credential(user: &str) -> String {
    if user == "root" {
        return common::basic_credential(user);
    }
    bearer(
        MACHINES,
        &audience(MACHINES),
        &json!({ "sub": user, "client_id": user, "scope": "__ferroehr.outbox" }),
    )
}

/// Sends `method uri` as `user`, with an optional JSON body.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    user: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, credential(user))
        .header(header::ACCEPT, "application/json");
    let req = match body {
        Some(body) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    common::send_body(app, req).await
}

/// The feed path of `reader`, with `query` appended.
fn feed(reader: &str, query: &str) -> String {
    common::full(&format!("/admin/outbox/{reader}{query}"))
}

/// Reads a page as the feed reader; returns the events.
async fn page(app: &Router, query: &str) -> Vec<Value> {
    let (status, body) = send(app, "GET", &feed(READER, query), "bridge", None).await;
    assert_eq!(status, StatusCode::OK, "feed read: {body}");
    let page: Value = serde_json::from_str(&body).expect("page json");
    assert_eq!(page["reader"], json!(READER), "the page names its reader");
    page["events"].as_array().expect("events").clone()
}

/// Acknowledges `last_seq` as the feed reader; returns the status.
async fn ack(app: &Router, reader: &str, last_seq: i64) -> StatusCode {
    send(
        app,
        "POST",
        &feed(reader, "/ack"),
        "bridge",
        Some(json!({ "last_seq": last_seq })),
    )
    .await
    .0
}

/// Creates `n` EHRs as the administrator, one outbox row each.
async fn create_ehrs(app: &Router, n: usize) {
    for _ in 0..n {
        let (status, body) = send(app, "POST", &format!("{BASE}/ehr"), "root", None).await;
        assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    }
}

fn seq(event: &Value) -> i64 {
    event["seq"].as_i64().expect("seq")
}

#[tokio::test]
async fn an_empty_feed_is_an_empty_list() {
    let (_db, _pool, app) = app().await;
    assert_eq!(
        page(&app, "").await,
        Vec::<Value>::new(),
        "no commit, no event"
    );
    assert_eq!(
        page(&app, "?domain=party").await,
        Vec::<Value>::new(),
        "no party commit, no party event"
    );
}

#[tokio::test]
async fn a_page_is_in_seq_order_and_bounded_by_its_limit() {
    let (_db, _pool, app) = app().await;
    create_ehrs(&app, 3).await;

    let events = page(&app, "").await;
    assert_eq!(events.len(), 3, "{events:?}");
    let seqs: Vec<i64> = events.iter().map(seq).collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "in seq order: {seqs:?}"
    );
    for event in &events {
        for member in ["contribution_id", "ehr_id", "committed_at", "versions"] {
            assert!(event.get(member).is_some(), "{member} in {event}");
        }
    }

    let two = page(&app, "?limit=2").await;
    assert_eq!(two.iter().map(seq).collect::<Vec<_>>(), seqs[..2]);
    assert_eq!(page(&app, "?limit=1000").await.len(), 3);
    for bad in [
        "?limit=0",
        "?limit=1001",
        "?limit=-1",
        "?limit=many",
        "?domain=audit",
    ] {
        let (status, body) = send(&app, "GET", &feed(READER, bad), "bridge", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
}

#[tokio::test]
async fn an_acknowledgement_moves_the_cursor_forward_only() {
    let (_db, _pool, app) = app().await;
    create_ehrs(&app, 3).await;
    let seqs: Vec<i64> = page(&app, "").await.iter().map(seq).collect();

    assert_eq!(ack(&app, READER, seqs[1]).await, StatusCode::NO_CONTENT);
    let rest = page(&app, "").await;
    assert_eq!(rest.iter().map(seq).collect::<Vec<_>>(), seqs[2..]);
    assert_eq!(
        ack(&app, READER, seqs[1]).await,
        StatusCode::NO_CONTENT,
        "acknowledging the cursor again"
    );
    assert_eq!(
        ack(&app, READER, seqs[0]).await,
        StatusCode::CONFLICT,
        "a cursor never moves back"
    );
    assert_eq!(
        ack(&app, READER, seqs[2] + 1).await,
        StatusCode::CONFLICT,
        "a cursor never passes the highest row"
    );
    assert_eq!(page(&app, "").await.len(), 1, "a refused ack moves nothing");

    let (status, body) = send(
        &app,
        "POST",
        &feed(READER, "/ack"),
        "bridge",
        Some(json!({ "last_seq": "many" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn an_unknown_or_inactive_reader_is_not_found() {
    let (_db, _pool, app) = app().await;
    create_ehrs(&app, 1).await;
    for reader in ["nobody", FORMER] {
        let (status, body) = send(&app, "GET", &feed(reader, ""), "bridge", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{reader}: {body}");
        assert_eq!(
            ack(&app, reader, 0).await,
            StatusCode::NOT_FOUND,
            "{reader}"
        );
    }
}

#[tokio::test]
async fn another_readers_client_is_refused() {
    let (_db, _pool, app) = app().await;
    create_ehrs(&app, 1).await;
    let (status, body) = send(&app, "GET", &feed(READER, ""), "other", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("read_change_feed"), "{body}");
    let (status, body) = send(&app, "GET", &feed("other", ""), "other", None).await;
    assert_eq!(status, StatusCode::OK, "its own reader: {body}");
}

#[tokio::test]
async fn the_feed_needs_the_outbox_scope() {
    let (_db, _pool, app) = app().await;
    let (status, body) = send(&app, "GET", &feed(READER, ""), "root", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send(
        &app,
        "POST",
        &feed(READER, "/ack"),
        "root",
        Some(json!({ "last_seq": 0 })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send(
        &app,
        "GET",
        &common::full("/admin/report/contribution/count"),
        "bridge",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the outbox scope reaches no other admin route: {body}"
    );
}

#[tokio::test]
async fn every_read_is_one_access_record() {
    let (_db, pool, app) = app().await;
    create_ehrs(&app, 1).await;
    page(&app, "").await;
    page(&app, "?limit=1").await;

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let reads: Vec<(Option<String>, String)> = sqlx::query_as(
            "SELECT principal, action FROM audit.audit_event \
             WHERE operation = 'admin_outbox_read' ORDER BY chain_seq",
        )
        .fetch_all(&pool)
        .await
        .expect("read the trail");
        if reads.len() >= 2 {
            assert_eq!(reads.len(), 2, "one record per read: {reads:?}");
            for (principal, action) in &reads {
                assert_eq!(principal.as_deref(), Some("bridge"));
                assert_eq!(action, "R");
            }
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {} of 2 read records landed",
            reads.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
