// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end test of the RESTful-ATNA feed (IHE ITI-20 **ATX: FHIR Feed**):
//! events emitted through the sender land in the local store, and the outbox
//! worker POSTs each stored FHIR R4 `AuditEvent` to the ARR
//! (`{url}/AuditEvent`, `application/fhir+json`), removes it from
//! `audit.fhir_feed_outbox`, whose row is the record's delivery state — a real `PostgreSQL` 18
//! (shared testkit harness) plus a `wiremock` ARR.

use std::time::Duration;

/// The linked system records the FHIR Feed owes none of: every record once
/// the outbox is empty, since a record leaves the outbox only when the
/// repository accepted it (#6203).
const DELIVERED: &str = "SELECT CASE WHEN EXISTS (SELECT 1 FROM audit.fhir_feed_outbox) THEN 0 \
     ELSE (SELECT count(*) FROM audit.audit_event_link) END";

use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ferroehr::system_log::config::{AuditConfig, FhirFeedConfig, StoreConfig};
use ferroehr::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::sender::{self, AuditHandle, AuditSender};
use ferroehr::system_log::store::intake::Chains;

fn config(arr_url: &str) -> AuditConfig {
    AuditConfig {
        enabled: true,
        store: StoreConfig {
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            max_verify_lag: ferrohealth_config::units::Duration::from_secs(0),
            ..StoreConfig::default()
        },
        fhir_feed: Some(FhirFeedConfig {
            endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
                url: ferrohealth_secret::secret_url::SecretUrl::new(arr_url),
                retry: ferrohealth_smart::config::endpoint::RetrySection {
                    max_backoff: ferrohealth_config::units::Duration::from_millis(500),
                    ..ferrohealth_smart::config::endpoint::RetrySection::default()
                },
                ..ferrohealth_smart::config::endpoint::EndpointSection::default()
            },
            poll_interval: ferrohealth_config::units::Duration::from_millis(100),
        }),
        ..AuditConfig::default()
    }
}

/// A party read: an access outside the access log, which the store keeps as
/// an `audit_event` record.
fn event() -> AuditEvent {
    let mut e = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Demographic,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut e.user_id);
    e.object_id = Some("8fa1::ferroehr::1".to_owned());
    e.event_type = Some(EventType::rest("person_get"));
    e
}

/// A composition read: an access the access log covers, stored as its entry.
fn access() -> AuditEvent {
    let mut e = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut e.user_id);
    e.object_id = Some("8fa1::ferroehr::1".to_owned());
    e.event_type = Some(EventType::rest("composition_get"));
    e
}

/// Poll until `condition` returns true or the deadline passes.
async fn eventually<F, Fut>(mut condition: F, deadline: Duration) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if condition().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
async fn stored_records_are_fed_to_the_fhir_arr_and_stamped() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let arr = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fhir/AuditEvent"))
        .and(header("content-type", "application/fhir+json"))
        .respond_with(ResponseTemplate::new(201))
        .expect(2..)
        .mount(&arr)
        .await;

    let cfg = config(&format!("{}/fhir", arr.uri()));
    let (sender, handle) = sender::start(cfg, None, pool.clone()).await.expect("start");

    for _ in 0..2 {
        sender
            .record(vec![event()], None)
            .await
            .expect("the record is written");
    }

    // Both rows are stored and, via the outbox worker, delivered.
    let delivered = eventually(
        || async {
            sqlx::query_scalar::<_, i64>(DELIVERED)
                .fetch_one(&pool)
                .await
                .unwrap_or(0)
                == 2
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(delivered, "both records delivered to the ARR");

    drop(sender);
    handle.shutdown(Duration::from_secs(2)).await;
}

/// An access the access log covers is stored as its entry alone, and the
/// feed delivers the entry's projection: the BALP `AuditEvent` the access
/// renders, owed until the ARR accepts it (#5127).
#[tokio::test]
async fn an_entry_is_fed_as_its_projection() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let arr = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fhir/AuditEvent"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&arr)
        .await;

    let cfg = config(&format!("{}/fhir", arr.uri()));
    let (sender, handle) = sender::start(cfg, None, pool.clone()).await.expect("start");
    sender
        .record(vec![access()], None)
        .await
        .expect("the record is written");

    let delivered = eventually(
        || async {
            count(&pool, "SELECT count(*) FROM audit.access_log_record").await == 1
                && count(&pool, "SELECT count(*) FROM audit.fhir_feed_outbox").await == 0
                && !arr.received_requests().await.unwrap_or_default().is_empty()
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(
        delivered,
        "the entry was stored and its projection delivered"
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM audit.audit_event").await,
        0,
        "the access is stored as its entry alone"
    );
    let requests = arr.received_requests().await.expect("the ARR's requests");
    let posted: serde_json::Value =
        serde_json::from_slice(&requests.first().expect("one delivery").body).expect("JSON");
    assert_eq!(posted["resourceType"], "AuditEvent");
    assert!(
        posted["subtype"]
            .as_array()
            .expect("subtypes")
            .iter()
            .any(|subtype| subtype["code"] == "composition_get"),
        "{posted}"
    );

    drop(sender);
    handle.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn arr_outage_leaves_rows_pending_then_delivers_on_recovery() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let arr = MockServer::start().await;
    // Outage first: the ARR answers 503; the row must stay pending (durable).
    Mock::given(method("POST"))
        .and(path("/fhir/AuditEvent"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(4)
        .mount(&arr)
        .await;
    Mock::given(method("POST"))
        .and(path("/fhir/AuditEvent"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&arr)
        .await;

    let cfg = config(&format!("{}/fhir", arr.uri()));
    let (sender, handle) = sender::start(cfg, None, pool.clone()).await.expect("start");
    sender
        .record(vec![event()], None)
        .await
        .expect("the record is written");

    // Stored immediately (durability anchor) even while the ARR is down…
    let stored = eventually(
        || async {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit.audit_event")
                .fetch_one(&pool)
                .await
                .unwrap_or(0)
                == 1
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(stored, "the record is stored while the ARR is down");

    // …and delivered once the ARR recovers (the 503 mock expires).
    let delivered = eventually(
        || async {
            sqlx::query_scalar::<_, i64>(DELIVERED)
                .fetch_one(&pool)
                .await
                .unwrap_or(0)
                == 1
        },
        Duration::from_secs(15),
    )
    .await;
    assert!(delivered, "the pending record ships after ARR recovery");

    drop(sender);
    handle.shutdown(Duration::from_secs(2)).await;
}

/// Under `[audit.fhir_feed.oauth]` the feed fetches a token with the client
/// credentials grant (RFC 6749 §4.4) and POSTs each `AuditEvent` with it as a
/// bearer credential, the IHE IUA authorized client; when the ARR refuses the
/// held token with `401`, a new one is fetched and the record sent once more.
#[tokio::test]
async fn the_feed_authenticates_with_a_client_credentials_token() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let auth = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(wiremock::matchers::body_string_contains(
            "grant_type=client_credentials",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "feed-token", "token_type": "Bearer", "expires_in": 300
        })))
        .expect(1..)
        .mount(&auth)
        .await;
    let arr = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fhir/AuditEvent"))
        .and(header("authorization", "Bearer feed-token"))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&arr)
        .await;
    Mock::given(method("POST"))
        .and(path("/fhir/AuditEvent"))
        .and(header("authorization", "Bearer feed-token"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1..)
        .mount(&arr)
        .await;

    let mut cfg = config(&format!("{}/fhir", arr.uri()));
    cfg.fhir_feed
        .as_mut()
        .expect("the feed")
        .endpoint
        .credentials
        .oauth = Some(ferrohealth_smart::config::credentials::OauthSection {
        token_endpoint: ferrohealth_secret::secret_url::SecretUrl::new(format!(
            "{}/token",
            auth.uri()
        )),
        client_id: "ferroehr-feed".to_owned(),
        client_secret: Some(secrecy::SecretString::from("s3cret")),
        ..Default::default()
    });
    let (sender, handle) = sender::start(cfg, None, pool.clone()).await.expect("start");
    sender
        .record(vec![event()], None)
        .await
        .expect("the record is written");

    let delivered = eventually(
        || async {
            sqlx::query_scalar::<_, i64>(DELIVERED)
                .fetch_one(&pool)
                .await
                .unwrap_or(0)
                == 1
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(delivered, "the record is delivered under the renewed token");

    drop(sender);
    handle.shutdown(Duration::from_secs(2)).await;
}

/// The store with no FHIR Feed configured: the records it writes.
fn store_only() -> AuditConfig {
    let mut config = config("http://127.0.0.1:9/fhir");
    config.fhir_feed = None;
    config
}

/// Counts the rows `sql` selects.
#[expect(
    clippy::expect_used,
    reason = "clippy.toml `allow-expect-in-tests` reaches only `#[test]` functions, not this \
              helper; a failed count fails the test that asked for it"
)]
async fn count(pool: &sqlx::PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(pool)
        .await
        .expect("count")
}

/// Chains every record the intake holds, then stops the sender, so a count
/// read afterwards never depends on the shutdown's bounded drain (#6128).
#[expect(
    clippy::expect_used,
    reason = "clippy.toml `allow-expect-in-tests` reaches only `#[test]` functions, not this \
              helper; a failed settle fails the test that asked for it"
)]
async fn settle_and_stop(sender: AuditSender, handle: AuditHandle) {
    sender
        .store()
        .settle(Chains::BOTH)
        .await
        .expect("the intake is chained");
    drop(sender);
    handle.shutdown(Duration::from_secs(2)).await;
}

/// With no feed configured no sink is owed a record, so the outbox stays
/// empty however many records the store holds, and no index covers the
/// records by their delivery.
#[tokio::test]
async fn with_no_feed_configured_no_record_is_owed() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (sender, handle) = sender::start(store_only(), None, pool.clone())
        .await
        .expect("start");
    for _ in 0..3 {
        sender
            .record(vec![event()], None)
            .await
            .expect("the record is written");
    }
    settle_and_stop(sender, handle).await;

    assert_eq!(
        count(&pool, "SELECT count(*) FROM audit.audit_event").await,
        3
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM audit.fhir_feed_outbox").await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM pg_indexes WHERE schemaname = 'audit' \
             AND indexdef ILIKE '%delivered_%_at IS NULL%'"
        )
        .await,
        0,
        "no index grows with records no sink is owed"
    );
}
