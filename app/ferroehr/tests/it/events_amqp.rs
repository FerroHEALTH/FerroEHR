// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end contribution-outbox eventing against a **real broker**
//! (testcontainers `RabbitMQ`) + a real `PostgreSQL` 18 — the broker half of
//! Our own extension (task 3/5b).
//!
//! Proves: (1) a committed contribution is published to the topic exchange and
//! is consumable from a bound queue, with the expected routing key and a
//! PHI-free envelope; (2) at-least-once with no loss under a broker outage —
//! while the broker is unreachable the outbox rows stay pending, and once a
//! publisher can reach the broker every row is delivered.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::time::Duration;

use futures::StreamExt as _;
use lapin::options::{
    BasicAckOptions, BasicConsumeOptions, ExchangeDeclareOptions, QueueBindOptions,
    QueueDeclareOptions,
};
use lapin::types::FieldTable;
use lapin::{Connection, ConnectionProperties, ExchangeKind};
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::rabbitmq::RabbitMq;

use ferroehr::extensions::events::config::EventsConfig;
use ferroehr::extensions::events::publisher::{start, subscription_queue_name};
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
use openehr_rm::prelude::PartyProxy;

const EXCHANGE: &str = "ferroehr.events";

// ── containers ───────────────────────────────────────────────────────────────

async fn amqp_url(rmq: &ContainerAsync<RabbitMq>) -> String {
    let host = rmq.get_host().await.expect("rmq host").to_string();
    let port = rmq.get_host_port_ipv4(5672).await.expect("rmq amqp port");
    format!("amqp://guest:guest@{host}:{port}/%2f")
}

// ── fixtures ─────────────────────────────────────────────────────────────────

fn uv<T: serde::de::DeserializeOwned>(data: &Value, change_code: &str) -> UpdateVersion<T> {
    UpdateVersion {
        preceding_version_uid: None,
        lifecycle_state: lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the fixture commit body decodes as its RM type"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded(change_code),
            description: None,
            committer: openehr_its::json::from_canonical_value::<PartyProxy>(
                &json!({ "_type": "PARTY_IDENTIFIED", "name": "event tester" }),
            )
            .expect("committer"),
            additional_properties: std::collections::BTreeMap::new(),
        }),
        signature: None,
        additional_properties: std::collections::BTreeMap::new(),
    }
}

fn composition(name: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": name },
        "language": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "en"
        },
        "territory": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
            "code_string": "NL"
        },
        "category": {
            "_type": "DV_CODED_TEXT", "value": "event",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "433" }
        },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "secret clinician name" }
    })
}

async fn pending_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM clinical.event_outbox WHERE published_at IS NULL")
        .fetch_one(pool)
        .await
        .expect("pending count")
}

/// Wait for the outbox to drain, bounded by PROGRESS rather than by total
/// elapsed time (#3119).
///
/// A loaded runner makes the whole drain slower without making it stuck, so a
/// fixed budget fails on the machine rather than on the code — this suite's
/// wildcard test did exactly that at 63 s against a 60 s deadline, and passed
/// in 10 s alone minutes later. A stall budget cannot: it fires only when the
/// pending count has not moved at all, which is what "stuck" actually means,
/// and it fires just as fast on a real hang.
async fn drain_outbox(pool: &PgPool) {
    const STALL: Duration = Duration::from_secs(30);
    let mut pending = pending_count(pool).await;
    let mut last_progress = tokio::time::Instant::now();
    loop {
        let now = pending_count(pool).await;
        if now == 0 {
            return;
        }
        if now != pending {
            pending = now;
            last_progress = tokio::time::Instant::now();
        }
        assert!(
            last_progress.elapsed() < STALL,
            "the outbox stopped draining with {pending} row(s) pending and no progress for {STALL:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The total number of version entries across all outbox rows — the number of
/// per-version messages the publisher emits.
async fn version_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(jsonb_array_length(envelope -> 'versions')), 0)::bigint \
         FROM clinical.event_outbox",
    )
    .fetch_one(pool)
    .await
    .expect("version count")
}

/// Decode a raw-JSON subscription fixture into the typed definition (the
/// client-simulation input shape, exercised as submitted bytes).
fn subscription_definition(
    body: Value,
) -> ferroehr::extensions::events::subscription::SubscriptionDefinition {
    serde_json::from_value(body).expect("subscription fixture decodes")
}

fn events_config(url: String) -> EventsConfig {
    EventsConfig {
        enabled: true,
        url: ferroehr::config::secret::SecretUrl::new(url),
        exchange: EXCHANGE.to_owned(),
        poll_interval_ms: 50,
        publish_max_retries: 1,
        prune_interval_secs: 3_600,
        ..EventsConfig::default()
    }
}

/// Declare the durable topic exchange + a queue bound to `binding_key`, and open
/// a consumer on it. Returns the live connection (kept alive by the caller) and
/// the consumer.
async fn bound_consumer(url: &str, binding_key: &str) -> (Connection, lapin::Consumer) {
    let conn = Connection::connect(url, ConnectionProperties::default())
        .await
        .expect("consumer connect");
    let channel = conn.create_channel().await.expect("consumer channel");
    channel
        .exchange_declare(
            EXCHANGE.into(),
            ExchangeKind::Topic,
            ExchangeDeclareOptions {
                durable: true,
                ..ExchangeDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("declare exchange");
    let queue = channel
        .queue_declare(
            "test-events".into(),
            QueueDeclareOptions::default(),
            FieldTable::default(),
        )
        .await
        .expect("declare queue");
    channel
        .queue_bind(
            queue.name().as_str().into(),
            EXCHANGE.into(),
            binding_key.into(),
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await
        .expect("bind queue");
    let consumer = channel
        .basic_consume(
            queue.name().as_str().into(),
            "test-consumer".into(),
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await
        .expect("consume");
    (conn, consumer)
}

/// Open a consumer on an already-declared durable queue (the publisher declared
/// and bound it for the subscription; here we only re-declare it idempotently
/// and consume). No binding — the subscription's binding key is the publisher's.
async fn queue_consumer(url: &str, queue: &str) -> (Connection, lapin::Consumer) {
    let conn = Connection::connect(url, ConnectionProperties::default())
        .await
        .expect("consumer connect");
    let channel = conn.create_channel().await.expect("consumer channel");
    channel
        .queue_declare(
            queue.into(),
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("declare queue");
    let consumer = channel
        .basic_consume(
            queue.into(),
            "sub-consumer".into(),
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await
        .expect("consume");
    (conn, consumer)
}

/// The next delivery within a short timeout, or `None` — used to assert a queue
/// has *no more* messages (selectivity).
async fn maybe_next(consumer: &mut lapin::Consumer, dur: Duration) -> Option<(String, Value)> {
    match tokio::time::timeout(dur, consumer.next()).await {
        Ok(Some(Ok(delivery))) => {
            let rk = delivery.routing_key.as_str().to_owned();
            let body: Value = serde_json::from_slice(&delivery.data).expect("json body");
            delivery.ack(BasicAckOptions::default()).await.expect("ack");
            Some((rk, body))
        }
        _ => None,
    }
}

/// Consume the next delivery within a timeout, returning (`routing_key`, body).
async fn next_delivery(consumer: &mut lapin::Consumer) -> (String, Value) {
    let delivery = tokio::time::timeout(Duration::from_secs(90), consumer.next())
        .await
        .expect("a delivery within the timeout")
        .expect("consumer stream not closed")
        .expect("delivery ok");
    let routing_key = delivery.routing_key.as_str().to_owned();
    let body: Value = serde_json::from_slice(&delivery.data).expect("json body");
    delivery.ack(BasicAckOptions::default()).await.expect("ack");
    (routing_key, body)
}

fn assert_phi_free(env: &Value) {
    let text = serde_json::to_string(env).expect("serialize");
    for forbidden in [
        "composer",
        "secret clinician name",
        "archetype_node_id",
        "DV_TEXT",
    ] {
        assert!(!text.contains(forbidden), "leaked clinical content: {text}");
    }
    assert!(env.get("versions").is_some(), "envelope carries versions");
    assert!(env.get("seq").is_some(), "published payload carries seq");
}

// ── tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn end_to_end_publish_and_consume() {
    let db = testkit::db().await.expect("testkit database");
    let rmq = RabbitMq::default()
        .start()
        .await
        .expect("start rabbitmq (is Docker running?)");
    let pool = db.pool();
    let url = amqp_url(&rmq).await;

    // Bind a consumer for COMPOSITION events BEFORE publishing (a topic exchange
    // drops unroutable messages), so the composition event lands in the queue.
    let (_conn, mut consumer) = bound_consumer(&url, "COMPOSITION.#").await;

    // Commit an EHR (→ EHR_STATUS event, not COMPOSITION) then a composition.
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr = svc.create_ehr(None).await.expect("create_ehr");
    svc.create_composition(ehr, uv(&composition("v1"), "249"))
        .await
        .expect("create_composition");

    // Start the real publisher; it drains + publishes to the broker.
    let handle = start(
        events_config(url),
        pool.clone(),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    );

    // The bound queue receives exactly the composition event.
    let (routing_key, body) = next_delivery(&mut consumer).await;
    assert_eq!(
        routing_key, "COMPOSITION.249.-",
        "routing key is <kind>.<change_type>.<template|->"
    );
    assert_phi_free(&body);
    assert_eq!(body["versions"][0]["kind"], json!("COMPOSITION"));
    assert_eq!(body["ehr_id"], json!(ehr.to_string()));

    // Everything drained.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(pending_count(&pool).await, 0, "all rows drained");

    handle.shutdown(Duration::from_secs(3)).await;
}

#[tokio::test]
async fn broker_down_then_up_delivers_without_loss() {
    let db = testkit::db().await.expect("testkit database");
    let rmq = RabbitMq::default()
        .start()
        .await
        .expect("start rabbitmq (is Docker running?)");
    let pool = db.pool();
    let url = amqp_url(&rmq).await;

    // Bind a catch-all consumer before any publish.
    let (_conn, mut consumer) = bound_consumer(&url, "#").await;

    // Commit an EHR + a composition ⇒ two pending outbox rows.
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr = svc.create_ehr(None).await.expect("create_ehr");
    svc.create_composition(ehr, uv(&composition("v1"), "249"))
        .await
        .expect("create_composition");
    let committed = pending_count(&pool).await;
    assert_eq!(committed, 2, "EHR creation + composition ⇒ two rows");
    // Per-version fan-out: EHR creation commits
    // EHR_STATUS + EHR_ACCESS (2 versions) + the composition (1) ⇒ 3 messages.
    let expected_messages = version_count(&pool).await;
    assert_eq!(
        expected_messages, 3,
        "two rows fan out to three per-version messages"
    );

    // Broker "down": a publisher pointed at a dead port cannot deliver; rows
    // stay pending (the outbox buffers).
    let bad_url = "amqp://guest:guest@127.0.0.1:1/%2f".to_owned();
    let down = start(
        events_config(bad_url),
        pool.clone(),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    );
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        pending_count(&pool).await,
        committed,
        "no row drains while the broker is unreachable"
    );
    down.shutdown(Duration::from_secs(3)).await;

    // Broker up: a correctly-configured publisher drains every row (no loss).
    let up = start(
        events_config(url),
        pool.clone(),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    );
    drain_outbox(&pool).await;

    // Every per-version message arrives on the catch-all queue (no loss).
    for _ in 0..expected_messages {
        let (routing_key, body) = next_delivery(&mut consumer).await;
        assert!(routing_key.contains('.'), "topic-shaped key: {routing_key}");
        assert_phi_free(&body);
    }

    up.shutdown(Duration::from_secs(3)).await;
}

#[tokio::test]
async fn subscriptions_route_by_predicate_and_wildcard_receives_all() {
    let db = testkit::db().await.expect("testkit database");
    let rmq = RabbitMq::default()
        .start()
        .await
        .expect("start rabbitmq (is Docker running?)");
    let pool = db.pool();
    let url = amqp_url(&rmq).await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // Two subscriptions: a wildcard (all predicates NULL → binding
    // key *.*.* → every event) and a kind filter (kind=COMPOSITION → binding key
    // COMPOSITION.*.* → composition events only). The publisher declares + binds
    // a durable per-subscription queue `ferroehr.events.<name>` for each.
    svc.event_subscription_create(subscription_definition(json!({ "name": "everything" })))
        .await
        .expect("wildcard subscription");
    svc.event_subscription_create(subscription_definition(
        json!({ "name": "compositions", "kind": "COMPOSITION" }),
    ))
    .await
    .expect("kind-filtered subscription");

    // Commit an EHR (EHR_STATUS + EHR_ACCESS ⇒ 2 versions) + a composition (1).
    let ehr = svc.create_ehr(None).await.expect("create_ehr");
    svc.create_composition(ehr, uv(&composition("v1"), "249"))
        .await
        .expect("create_composition");
    let expected_all = version_count(&pool).await; // 3 per-version messages

    // Start the publisher: each cycle re-syncs (declares/binds) the subscription
    // queues *before* it publishes, so the durable queues capture the messages.
    let handle = start(
        events_config(url.clone()),
        pool.clone(),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    );
    drain_outbox(&pool).await;

    // The wildcard queue received every per-version message (per-version fan-out).
    let (_c_all, mut all) =
        queue_consumer(&url, &subscription_queue_name(EXCHANGE, "everything")).await;
    for _ in 0..expected_all {
        let (rk, body) = next_delivery(&mut all).await;
        assert!(rk.contains('.'), "topic-shaped key: {rk}");
        assert_phi_free(&body);
    }
    assert!(
        maybe_next(&mut all, Duration::from_millis(500))
            .await
            .is_none(),
        "wildcard queue must have exactly {expected_all} messages"
    );

    // The kind-filtered queue received ONLY the composition event — the
    // EHR_STATUS/EHR_ACCESS events (keys EHR_STATUS.* / EHR_ACCESS.*) did not
    // match COMPOSITION.*.*.
    let (_c_comp, mut comps) =
        queue_consumer(&url, &subscription_queue_name(EXCHANGE, "compositions")).await;
    let (rk, body) = next_delivery(&mut comps).await;
    assert_eq!(
        rk, "COMPOSITION.249.-",
        "only the composition version routes to the kind=COMPOSITION queue"
    );
    let vi = usize::try_from(body["version_index"].as_u64().expect("version_index")).unwrap();
    assert_eq!(body["versions"][vi]["kind"], "COMPOSITION");
    assert_phi_free(&body);
    assert!(
        maybe_next(&mut comps, Duration::from_millis(500))
            .await
            .is_none(),
        "the kind-filtered queue must receive only the matching event"
    );

    handle.shutdown(Duration::from_secs(3)).await;
}
