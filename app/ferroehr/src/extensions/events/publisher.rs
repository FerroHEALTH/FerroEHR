// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The background drainer + retention pruner + [`EventsHandle`].
//!
//! **No openEHR spec governs this — our own design/extension.** Active only
//! when the eventing extension is enabled.
//!
//! A **single** tokio task polls the outboxes, publishes pending rows in `seq`
//! order (a global order that trivially preserves per-EHR order), and marks
//! each published only after the broker confirms. There are two outboxes, one
//! per pseudonymisation domain (`clinical.event_outbox` and
//! `party.event_outbox` — a demographic contribution's foreign key must
//! stay inside its own schema), drained by this one task; per-EHR ordering is
//! unaffected, because a demographic event has no EHR. On a publish failure it stops
//! the batch — never skipping ahead — so an EHR's events keep their order, and
//! backs off before retrying (the outbox buffers while the broker is down). A
//! periodic pass prunes published rows older than the retention window and
//! behind every active cursor reader (`crate::extensions::outbox`).
//!
//! **Subscription topology is declared on connect/change only:** each cycle
//! reads the enabled subscriptions (a cheap local query) and touches the broker
//! only when the desired queue/binding set differs from what was last declared,
//! or the publisher's topology epoch advanced (a fresh broker connection, which
//! may mean a replaced broker without our durable queues). Declaration is
//! idempotent, and it runs before the cycle's publishes so a just-created
//! subscription's queue is bound before any matching event is routed (a topic
//! exchange drops unroutable messages).
//!
//! The outbox row itself is written **inside the commit transaction** by
//! `crate::storage::version_repo::commit::write_outbox`, from the PHI-free per-version
//! envelope built by `crate::versioning` (`Committed::envelope_entry`); this
//! module only drains what storage recorded.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): external FHIR resources, tenancy/event CRUD rows, \
              multimedia offload over stored fragments (families 3/6/8)"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use sqlx::{PgPool, Row};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::config::EventsConfig;
use crate::extensions::outbox;
use ferroehr_ext::events::amqp::AmqpPublisher;
use ferroehr_ext::events::{EventError, EventPublisher};

/// Owns the drainer task; the binary keeps it and shuts it down on exit.
#[derive(Debug)]
pub struct EventsHandle {
    shutdown: watch::Sender<bool>,
    join: JoinHandle<()>,
    /// Liveness of broker delivery: `true` while draining succeeds, `false`
    /// after a publish/transport failure. Surfaced by the `events` health
    /// indicator (never blocks readiness — degraded-tolerable).
    healthy: Arc<AtomicBool>,
}

impl EventsHandle {
    /// A shared flag reporting whether broker delivery is currently healthy,
    /// for the `events` health indicator.
    #[must_use]
    pub fn healthy(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.healthy)
    }

    /// Signal the drainer to stop and await it, bounded by `timeout`. Pending
    /// (unpublished) rows are left in the outbox and drained on next start
    /// (at-least-once).
    pub async fn shutdown(self, timeout: Duration) {
        let _ = self.shutdown.send(true);
        match tokio::time::timeout(timeout, self.join).await {
            Ok(Ok(())) => tracing::debug!("event publisher stopped"),
            Ok(Err(e)) => tracing::warn!("event publisher task panicked: {e}"),
            Err(_) => tracing::warn!("event publisher did not stop within {timeout:?}"),
        }
    }
}

/// Start the publisher over the real AMQP broker.
///
/// Constructs the lazily-connecting [`AmqpPublisher`] and spawns the drainer
/// — a broker that is down at start is tolerated (rows stay pending until it
/// returns).
#[must_use]
pub fn start(config: EventsConfig, pool: PgPool, demographic_pool: PgPool) -> EventsHandle {
    let publisher = Arc::new(AmqpPublisher::new(
        config.effective_url(),
        config.exchange.clone(),
    ));
    start_with_publisher(config, pool, demographic_pool, publisher)
}

/// Start the publisher over an arbitrary [`EventPublisher`] (the seam the tests
/// use to drive the drainer without a real broker).
#[must_use]
pub fn start_with_publisher(
    config: EventsConfig,
    pool: PgPool,
    demographic_pool: PgPool,
    publisher: Arc<dyn EventPublisher>,
) -> EventsHandle {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let healthy = Arc::new(AtomicBool::new(true));
    let join = tokio::spawn(run(
        config,
        pool,
        demographic_pool,
        publisher,
        shutdown_rx,
        Arc::clone(&healthy),
    ));
    EventsHandle {
        shutdown: shutdown_tx,
        join,
        healthy,
    }
}

/// The broker queue name for a subscription: `<exchange>.<name>`
/// (`ferroehr.events.<name>` for the default exchange) — the configured
/// exchange prefix + the subscription name.
///
/// Exposed so a consumer knows the queue to consume from.
#[must_use]
pub fn subscription_queue_name(exchange: &str, name: &str) -> String {
    format!("{exchange}.{name}")
}

/// A failure during one drain pass.
enum DrainError {
    /// A DB error reading/marking the outbox.
    Db(sqlx::Error),
    /// A broker publish failure; the drainer should back off before retrying.
    Publish(EventError),
}

/// A failure while syncing subscription queues.
enum SyncError {
    /// Reading the subscription rows failed.
    Db(sqlx::Error),
    /// Declaring/binding a queue on the broker failed.
    Broker(EventError),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Db(e) => write!(f, "subscription store: {e}"),
            SyncError::Broker(e) => write!(f, "broker declare: {e}"),
        }
    }
}

/// The last successfully declared broker topology: the publisher's topology
/// epoch at declaration time + the sorted `(queue, binding_key)` set. Broker
/// declares are skipped while both still match (declared on connect/change
/// only — never per cycle).
type DeclaredTopology = Option<(u64, Vec<(String, String)>)>;

/// The drain + prune loop.
async fn run(
    config: EventsConfig,
    pool: PgPool,
    demographic_pool: PgPool,
    publisher: Arc<dyn EventPublisher>,
    mut shutdown: watch::Receiver<bool>,
    healthy: Arc<AtomicBool>,
) {
    // Both outboxes are drained; only the clinical pool reads the subscription
    // registry, because subscriptions are deployment configuration rather than
    // domain content and `event_subscription` lives only in the clinical
    // schema. The domain travels with the pool because the legal marks live on
    // `ehr`, which the party domain does not have.
    let pools = [
        (&pool, OutboxDomain::Clinical),
        (&demographic_pool, OutboxDomain::Party),
    ];
    let poll_interval = Duration::from_millis(config.poll_interval_ms.max(1));
    let prune_every = Duration::from_secs(config.prune_interval_secs.max(1));
    let mut last_prune = tokio::time::Instant::now();
    let mut declared: DeclaredTopology = None;
    tracing::info!(
        exchange = %config.exchange,
        batch_size = config.batch_size,
        retention_days = config.retention_days,
        "event publisher started"
    );

    loop {
        if *shutdown.borrow() {
            break;
        }
        // Sync the subscription topology (queues + bindings) before this
        // cycle's publishes; broker calls happen only on connect/change (see
        // the module docs). Best-effort: a failure (broker down) is logged;
        // rows stay pending and the sync retries next cycle.
        if let Err(e) =
            sync_subscriptions(&pool, publisher.as_ref(), &config.exchange, &mut declared).await
        {
            tracing::debug!("event subscription sync deferred: {e}");
        }
        for (domain, kind) in pools {
            drain_until_caught_up(
                domain,
                kind,
                publisher.as_ref(),
                &config,
                &shutdown,
                &healthy,
            )
            .await;
        }

        // Retention prune (best-effort), on its own cadence.
        if last_prune.elapsed() >= prune_every {
            for (domain, _) in pools {
                if let Err(e) = outbox::prune(domain, config.retention_days).await {
                    tracing::warn!("event outbox retention prune failed: {e}");
                }
            }
            last_prune = tokio::time::Instant::now();
        }

        tokio::select! {
            _ = shutdown.changed() => {}
            () = tokio::time::sleep(poll_interval) => {}
        }
    }

    // Best-effort final drain so a clean shutdown flushes what the broker will
    // still take; anything left stays pending for next start (at-least-once).
    for (domain, kind) in pools {
        if let Ok(n) = drain_batch(domain, kind, publisher.as_ref(), &config).await
            && n > 0
        {
            tracing::debug!("event publisher flushed {n} events on shutdown");
        }
    }
    tracing::debug!("event publisher loop exited");
}

/// Drains the outbox until it is empty, the broker or DB stalls, or shutdown
/// is signalled, keeping the health flag in step with broker delivery.
///
/// A full batch means more may remain — keep draining; a short batch means the
/// outbox is now empty.
async fn drain_until_caught_up(
    pool: &PgPool,
    domain: OutboxDomain,
    publisher: &dyn EventPublisher,
    config: &EventsConfig,
    shutdown: &watch::Receiver<bool>,
    healthy: &AtomicBool,
) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        match drain_batch(pool, domain, publisher, config).await {
            Ok(n) => {
                healthy.store(true, Ordering::Relaxed);
                if usize::try_from(config.batch_size).unwrap_or(usize::MAX) > n {
                    return;
                }
            }
            Err(DrainError::Publish(e)) => {
                healthy.store(false, Ordering::Relaxed);
                tracing::warn!("event publish stalled (broker unavailable?): {e}");
                return;
            }
            Err(DrainError::Db(e)) => {
                tracing::warn!("event outbox drain DB error: {e}");
                return;
            }
        }
    }
}

/// Declare + bind the queue for every **enabled** subscription — but only when
/// the desired topology differs from `declared` (the subscription set changed)
/// or the publisher's topology epoch advanced (a fresh broker connection).
/// Reading the subscription rows is a cheap local query and runs every cycle
/// so CRUD is picked up within one poll; the broker round-trips happen on
/// change only. On success `declared` records the new state; on failure it is
/// left unchanged so the caller retries next cycle.
async fn sync_subscriptions(
    pool: &PgPool,
    publisher: &dyn EventPublisher,
    exchange: &str,
    declared: &mut DeclaredTopology,
) -> Result<(), SyncError> {
    let rows = sqlx::query(
        "SELECT name, kind, change_type, template_id FROM event_subscription WHERE enabled",
    )
    .fetch_all(pool)
    .await
    .map_err(SyncError::Db)?;
    let mut desired: Vec<(String, String)> = Vec::with_capacity(rows.len());
    for row in &rows {
        let name: String = row.try_get("name").map_err(SyncError::Db)?;
        let kind: Option<String> = row.try_get("kind").map_err(SyncError::Db)?;
        let change_type: Option<String> = row.try_get("change_type").map_err(SyncError::Db)?;
        let template_id: Option<String> = row.try_get("template_id").map_err(SyncError::Db)?;
        let binding_key = ferroehr_ext::events::subscription_binding_key(
            kind.as_deref(),
            change_type.as_deref(),
            template_id.as_deref(),
        );
        desired.push((subscription_queue_name(exchange, &name), binding_key));
    }
    desired.sort_unstable();

    if let Some((epoch, topology)) = declared.as_ref()
        && *epoch == publisher.topology_epoch()
        && *topology == desired
    {
        return Ok(()); // Unchanged: no broker round-trips this cycle.
    }
    for (queue, binding_key) in &desired {
        publisher
            .declare_subscription(queue, binding_key)
            .await
            .map_err(SyncError::Broker)?;
    }
    // Read the epoch AFTER declaring: the declares themselves may have opened
    // the connection (bumping the epoch); a later reconnect bumps it again and
    // triggers a re-declare next cycle.
    *declared = Some((publisher.topology_epoch(), desired));
    Ok(())
}

/// Which pseudonymisation domain an outbox drain is running in.
///
/// The two outboxes are the same relation in two schemas, but only the clinical
/// one sits beside an `ehr` table, and the legal marks that withhold an event
/// live there. Naming `ehr` from the party domain would fail at parse time, so
/// the domain decides which statement runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutboxDomain {
    /// The clinical schema, whose `ehr` rows carry the restriction and
    /// research-objection marks.
    Clinical,
    /// The party schema, which has no EHR and therefore no mark to honour.
    Party,
}

/// Publish one batch of pending rows in `seq` order, marking each published only
/// after the broker confirms. Returns the count published this pass, or a
/// [`DrainError`] (the published prefix is committed first, preserving per-EHR
/// order). `FOR UPDATE SKIP LOCKED` makes concurrent instances safe.
async fn drain_batch(
    pool: &PgPool,
    domain: OutboxDomain,
    publisher: &dyn EventPublisher,
    config: &EventsConfig,
) -> Result<usize, DrainError> {
    let mut tx = pool.begin().await.map_err(DrainError::Db)?;
    // A mark recorded after the commit withholds the row instead of dropping
    // it: it stays pending and publishes if the restriction is lifted (GDPR
    // Art. 18(3) contemplates exactly that) or if the controller records an
    // Art. 21(6) override, and it leaves with the EHR if the EHR is erased.
    // Only the clinical statement names `ehr`, which is the only domain that
    // has it. `docs/law/eu/gdpr/text.html`; no openEHR spec governs eventing —
    // our own extension.
    let rows = sqlx::query(match domain {
        OutboxDomain::Clinical => {
            "SELECT seq, envelope FROM event_outbox o \
             WHERE o.published_at IS NULL \
               AND NOT EXISTS (SELECT 1 FROM ehr e WHERE e.id = o.ehr_id \
                   AND (e.restricted_at IS NOT NULL \
                        OR (e.research_objected_at IS NOT NULL \
                            AND e.research_objection_ground IS NULL))) \
             ORDER BY o.seq LIMIT $1 FOR UPDATE SKIP LOCKED"
        }
        OutboxDomain::Party => {
            "SELECT seq, envelope FROM event_outbox \
             WHERE published_at IS NULL ORDER BY seq LIMIT $1 FOR UPDATE SKIP LOCKED"
        }
    })
    .bind(config.batch_size)
    .fetch_all(&mut *tx)
    .await
    .map_err(DrainError::Db)?;
    if rows.is_empty() {
        return Ok(0);
    }

    let mut sent = 0usize;
    let mut publish_err = None;
    'rows: for row in &rows {
        let seq: i64 = row.try_get("seq").map_err(DrainError::Db)?;
        let envelope: serde_json::Value = row.try_get("envelope").map_err(DrainError::Db)?;
        // Per-version fan-out: one message per version entry, each under its
        // own routing key, carrying the shared envelope + seq + version_index.
        // All of a row's messages must confirm before the row is marked
        // published; a failure part-way leaves the whole row pending, so the
        // retry re-publishes every message (at-least-once — consumers
        // deduplicate on (contribution_id, version_index)).
        for (version_index, routing_key) in version_routing_keys(&envelope) {
            let payload = build_payload(seq, version_index, &envelope);
            if let Err(e) = publish_with_retry(publisher, &routing_key, &payload, config).await {
                // Stop at the first failure: never publish a later event for an
                // EHR before an earlier one (per-EHR ordering by design).
                publish_err = Some(e);
                break 'rows;
            }
        }
        sqlx::query("UPDATE event_outbox SET published_at = now() WHERE seq = $1")
            .bind(seq)
            .execute(&mut *tx)
            .await
            .map_err(DrainError::Db)?;
        sent += 1;
    }
    tx.commit().await.map_err(DrainError::Db)?;
    if sent > 0 {
        #[expect(
            clippy::as_conversions,
            reason = "the published-event count widens exactly: usize is at most 64 bits \
                      on every supported target"
        )]
        crate::telemetry::metrics::metrics()
            .events_published
            .add(sent as u64, &[]);
    }
    match publish_err {
        Some(e) => Err(DrainError::Publish(e)),
        None => Ok(sent),
    }
}

/// The per-version routing keys for one envelope: `(version_index,
/// routing_key)` for each entry in `versions`. A well-formed outbox row always
/// carries ≥1 version; a defensively-empty envelope yields a single message at
/// `version_index` 0 with the fallback key so nothing is silently dropped.
fn version_routing_keys(envelope: &serde_json::Value) -> Vec<(usize, String)> {
    match envelope.get("versions").and_then(|v| v.as_array()) {
        Some(versions) if !versions.is_empty() => versions
            .iter()
            .enumerate()
            .map(|(i, v)| (i, ferroehr_ext::events::routing_key_of_version(v)))
            .collect(),
        _ => vec![(
            0,
            ferroehr_ext::events::routing_key("UNKNOWN", ferroehr_ext::events::ABSENT, None),
        )],
    }
}

/// The published payload for one version: the stored PHI-free envelope with the
/// delivery `seq` and the `version_index` injected. Consumers order by `seq`,
/// and deduplicate on `(contribution_id, version_index)` (at-least-once).
fn build_payload(seq: i64, version_index: usize, envelope: &serde_json::Value) -> Vec<u8> {
    let mut payload = envelope.clone();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("seq".to_owned(), serde_json::json!(seq));
        obj.insert("version_index".to_owned(), serde_json::json!(version_index));
    }
    // A `Value` serializes infallibly (no non-string keys, no NaN); if that
    // invariant ever breaks, fail loudly for this row rather than publishing
    // a zero-byte message a consumer cannot attribute.
    match serde_json::to_vec(&payload) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(seq, version_index, error = %e, "event payload serialization failed");
            serde_json::json!({ "seq": seq, "version_index": version_index, "error": "payload serialization failed" })
                .to_string()
                .into_bytes()
        }
    }
}

/// Publish with exponential backoff, up to `publish_max_retries`
/// extra attempts, before giving up for this pass.
async fn publish_with_retry(
    publisher: &dyn EventPublisher,
    routing_key: &str,
    payload: &[u8],
    config: &EventsConfig,
) -> Result<(), EventError> {
    (|| async { publisher.publish(routing_key, payload).await })
        .retry(ExponentialBuilder::default().with_max_times(config.publish_max_retries))
        .notify(|e, d| tracing::warn!("event publish retry in {d:?}: {e}"))
        .await
}
