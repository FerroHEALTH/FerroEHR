// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR outbound emitter's background drainer + [`FhirOutboundHandle`].
//!
//! No openEHR spec governs this — our own design/extension. The gate is
//! [`FhirOutboundConfig::enabled`] (`fhir.outbound.enabled`, default off), a
//! separate switch from the REST FHIR connector because this stream carries PHI.
//!
//! It is wired like the contribution-outbox publisher but reads committed
//! `event_outbox` rows through its own persistent cursor
//! (`event_outbox_reader`, reader `fhir-outbound`), never touching the events drainer's
//! `published_at` watermark. One tokio task polls the outbox for rows past the
//! cursor in `seq` order; for each COMPOSITION version whose template matches an
//! enabled `fhir_mapping` it loads the committed version through the versioned
//! read seam, reverse-maps it to a FHIR resource and publishes it with broker
//! confirms to the configured PHI exchange. The cursor advances only over the
//! fully-published prefix, so a crash and retry re-emit at least once and
//! downstream FHIR systems upsert by resource id.
//!
//! A row whose reverse-mapping fails deterministically, from a defective stored
//! mapping or template, is retried `PARK_AFTER_FAILED_PASSES` times, then
//! dead-lettered to the log as an `error` record naming the row, and the cursor
//! advances past it so one bad row cannot head-of-line-block later commits.
//! Broker and database failures are transient and never park a row.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6): FHIR resources are an external standard \
              with no RM type (typed-FHIR evaluation tracked separately)"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::extensions::outbox;
use crate::extensions::outbox::OutboxReader;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::ids::VoId;

use crate::service::FerroEhrService;
use ferroehr_ext::events::amqp::AmqpPublisher;
use ferroehr_ext::events::{EventError, EventPublisher};

use super::config::FhirOutboundConfig;

/// How many consecutive failed passes a deterministically-unmappable (poison)
/// row gets before it is parked: dead-lettered to the log and skipped by
/// advancing the cursor past it. In-memory — a restart grants a candidate a
/// fresh budget, which only delays parking (at-least-once is preserved; a
/// parked row is the one case a message is deliberately dropped, and it is
/// always logged). No openEHR spec governs this — our own design.
const PARK_AFTER_FAILED_PASSES: u32 = 5;

/// Owns the outbound-emitter task; the binary keeps it and shuts it down on exit.
#[derive(Debug)]
pub struct FhirOutboundHandle {
    shutdown: watch::Sender<bool>,
    join: JoinHandle<()>,
    /// Liveness of broker delivery: `true` while emitting succeeds, `false`
    /// after a publish/transport failure. Surfaced by the `fhir_outbound` health
    /// indicator (never blocks readiness — degraded-tolerable).
    healthy: Arc<AtomicBool>,
}

impl FhirOutboundHandle {
    /// A shared flag reporting whether outbound delivery is currently healthy,
    /// for the `fhir_outbound` health indicator.
    #[must_use]
    pub fn healthy(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.healthy)
    }

    /// Signal the emitter to stop and await it, bounded by `timeout`. Unprocessed
    /// outbox rows stay past the cursor and are emitted on next start
    /// (at-least-once).
    pub async fn shutdown(self, timeout: Duration) {
        let _ = self.shutdown.send(true);
        match tokio::time::timeout(timeout, self.join).await {
            Ok(Ok(())) => tracing::debug!("fhir outbound emitter stopped"),
            Ok(Err(e)) => tracing::warn!("fhir outbound emitter task panicked: {e}"),
            Err(_) => tracing::warn!("fhir outbound emitter did not stop within {timeout:?}"),
        }
    }
}

/// Start the outbound emitter over the real AMQP broker.
///
/// Constructs the lazily-connecting [`AmqpPublisher`] on the PHI exchange and
/// spawns the drainer (a broker that is down at start is tolerated — rows
/// stay past the cursor until it returns).
#[must_use]
pub fn start(
    config: FhirOutboundConfig,
    pool: PgPool,
    service: Arc<FerroEhrService>,
) -> FhirOutboundHandle {
    let publisher = Arc::new(AmqpPublisher::new(
        config.effective_url(),
        config.exchange.clone(),
    ));
    start_with_publisher(config, pool, service, publisher)
}

/// Start the emitter over an arbitrary [`EventPublisher`] (the seam the tests use
/// to drive the emitter without a real broker).
#[must_use]
pub fn start_with_publisher(
    config: FhirOutboundConfig,
    pool: PgPool,
    service: Arc<FerroEhrService>,
    publisher: Arc<dyn EventPublisher>,
) -> FhirOutboundHandle {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let healthy = Arc::new(AtomicBool::new(true));
    let join = tokio::spawn(run(
        config,
        pool,
        service,
        publisher,
        shutdown_rx,
        Arc::clone(&healthy),
    ));
    FhirOutboundHandle {
        shutdown: shutdown_tx,
        join,
        healthy,
    }
}

/// A failure during one process pass.
enum ProcessError {
    /// A DB error reading the outbox / cursor.
    Db(sqlx::Error),
    /// Reverse-mapping a COMPOSITION failed (a stored mapping/template
    /// problem); the row stays past the cursor for retry until the park
    /// budget is exhausted.
    Map(String),
    /// A broker publish failure; back off before retrying.
    Publish(EventError),
}

impl From<sqlx::Error> for ProcessError {
    fn from(e: sqlx::Error) -> Self {
        ProcessError::Db(e)
    }
}

/// The poison-row retry budget: the head row currently failing to map, with
/// its consecutive-failed-pass count. Only ever the head row — the emitter is
/// strictly sequential, so a mapping failure always blocks at the front.
type PoisonBudget = Option<(i64, u32)>;

/// The poll loop.
async fn run(
    config: FhirOutboundConfig,
    pool: PgPool,
    service: Arc<FerroEhrService>,
    publisher: Arc<dyn EventPublisher>,
    mut shutdown: watch::Receiver<bool>,
    healthy: Arc<AtomicBool>,
) {
    let poll_interval = Duration::from_millis(config.poll_interval_ms.max(1));
    let mut poison: PoisonBudget = None;
    tracing::info!(
        exchange = %config.exchange,
        batch_size = config.batch_size,
        "fhir outbound emitter started"
    );

    loop {
        if *shutdown.borrow() {
            break;
        }
        // A running reader holds the prune floor; register before the first
        // batch so a lagging start never loses rows.
        if let Err(e) = outbox::reconcile(&pool, OutboxReader::FHIR_OUTBOUND, true).await {
            tracing::warn!(
                "fhir outbound reader registration failed, retried by the first advance: {e}"
            );
        }
        // Drain until the outbox is caught up or the broker/DB stalls.
        loop {
            if *shutdown.borrow() {
                break;
            }
            match process_batch(&pool, &service, publisher.as_ref(), &config, &mut poison).await {
                Ok(n) => {
                    healthy.store(true, Ordering::Relaxed);
                    // A short batch means the outbox is caught up.
                    if usize::try_from(config.batch_size).unwrap_or(usize::MAX) > n {
                        break;
                    }
                }
                Err(ProcessError::Publish(e)) => {
                    healthy.store(false, Ordering::Relaxed);
                    tracing::warn!("fhir outbound publish stalled (broker unavailable?): {e}");
                    break;
                }
                Err(ProcessError::Db(e)) => {
                    tracing::warn!("fhir outbound DB error: {e}");
                    break;
                }
                Err(ProcessError::Map(e)) => {
                    tracing::warn!(
                        "fhir outbound reverse-mapping error (row retried, then parked): {e}"
                    );
                    break;
                }
            }
        }

        tokio::select! {
            _ = shutdown.changed() => {}
            () = tokio::time::sleep(poll_interval) => {}
        }
    }
    tracing::debug!("fhir outbound emitter loop exited");
}

/// Process one batch of outbox rows past the cursor in `seq` order, publishing a
/// FHIR resource for every matching COMPOSITION version and advancing the cursor
/// over the fully-published prefix. Returns the number of rows fully processed
/// this pass, or a [`ProcessError`] (the committed prefix's cursor advance is
/// persisted first, preserving order).
///
/// `poison` is the dead-letter budget of the head row currently failing to
/// reverse-map: each failed pass increments it, and once it reaches
/// [`PARK_AFTER_FAILED_PASSES`] the row is **parked** — logged at `error` and
/// skipped by advancing the cursor past it — so a poison row never permanently
/// blocks later rows. Publish/DB failures leave the budget untouched (they are
/// transient and not the row's fault).
async fn process_batch(
    pool: &PgPool,
    service: &FerroEhrService,
    publisher: &dyn EventPublisher,
    config: &FhirOutboundConfig,
    poison: &mut PoisonBudget,
) -> Result<usize, ProcessError> {
    let last_seq = read_cursor(pool).await?;
    let rows =
        sqlx::query("SELECT seq, envelope FROM event_outbox WHERE seq > $1 ORDER BY seq LIMIT $2")
            .bind(last_seq)
            .bind(config.batch_size)
            .fetch_all(pool)
            .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let mut processed = 0usize;
    let mut advanced = last_seq;
    let mut outcome: Result<usize, ProcessError> = Ok(0);
    for row in &rows {
        let seq: i64 = row.try_get("seq")?;
        let envelope: Value = row.try_get("envelope")?;
        match process_row(service, publisher, config, poison, seq, &envelope).await {
            RowOutcome::Stop(e) => {
                outcome = Err(e);
                break;
            }
            RowOutcome::Published { parked } => {
                // The whole row published (or parked): it is safe to advance
                // the cursor past it.
                advanced = seq;
                processed += 1;
                if !parked {
                    // The head row completed cleanly — any earlier failures on
                    // it were transient; clear its budget.
                    clear_poison_for(poison, seq);
                }
            }
        }
    }

    // Persist the fully-published prefix's cursor even on a mid-batch failure, so
    // a re-run resumes from the last completed row (at-least-once).
    if advanced > last_seq {
        write_cursor(pool, advanced).await?;
    }
    match outcome {
        Err(e) => Err(e),
        Ok(_) => Ok(processed),
    }
}

/// What one outbox row's processing means for the rest of the batch.
enum RowOutcome {
    /// Every mappable version of the row published, or the row was parked —
    /// the cursor may advance past it. `parked` says which.
    Published { parked: bool },
    /// The batch stops here; the already-published prefix keeps its cursor
    /// advance, so a re-run resumes from this row.
    Stop(ProcessError),
}

/// Publishes every mappable version of one outbox row.
///
/// Only COMPOSITION versions can map to a FHIR resource (`EHR_STATUS`/`FOLDER`
/// carry no mappable template); the template is read from the COMPOSITION body
/// by the service. A mapping failure is deterministic for the row, so it
/// charges the row's park budget; once the budget is exhausted the row is
/// dead-lettered to the log and skipped. Publish and payload failures are
/// transient and stop the batch instead.
async fn process_row(
    service: &FerroEhrService,
    publisher: &dyn EventPublisher,
    config: &FhirOutboundConfig,
    poison: &mut PoisonBudget,
    seq: i64,
    envelope: &Value,
) -> RowOutcome {
    let ehr_id = envelope
        .get("ehr_id")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok());
    for version in envelope
        .get("versions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some((vo_id, sys_version)) = mappable_version(version) else {
            continue;
        };
        let messages = match service
            .fhir_outbound_messages(ehr_id, vo_id, sys_version)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                let failed_passes = charge_poison(poison, seq);
                if failed_passes < PARK_AFTER_FAILED_PASSES {
                    return RowOutcome::Stop(ProcessError::Map(e.to_string()));
                }
                tracing::error!(
                    seq,
                    vo_id = %vo_id,
                    sys_version,
                    error = %e,
                    "fhir outbound: parking poison outbox row after \
                     {PARK_AFTER_FAILED_PASSES} failed passes — row skipped \
                     (dead-lettered to the log; fix the stored mapping/template \
                     and re-commit to re-emit)"
                );
                *poison = None;
                return RowOutcome::Published { parked: true };
            }
        };
        for (resource_type, template_id, resource) in &messages {
            let routing_key = routing_key(resource_type, template_id);
            let payload = match resource_payload(resource_type, resource) {
                Ok(bytes) => bytes,
                Err(e) => return RowOutcome::Stop(e),
            };
            if let Err(e) = publish_with_retry(publisher, &routing_key, &payload, config).await {
                return RowOutcome::Stop(ProcessError::Publish(e));
            }
        }
    }
    RowOutcome::Published { parked: false }
}

/// The versioned-object id and version ordinal of one announced version, or
/// `None` when it is not a mappable COMPOSITION version.
fn mappable_version(version: &Value) -> Option<(VoId, i32)> {
    if version.get("kind").and_then(Value::as_str) != Some("COMPOSITION") {
        return None;
    }
    let vo_id = version
        .get("vo_id")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(VoId)?;
    let sys_version = version
        .get("sys_version")
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())?;
    Some((vo_id, sys_version))
}

/// Charge one failed pass against `seq`'s park budget, returning its
/// consecutive-failed-pass count. A different failing seq resets the budget
/// (the previous blocker was resolved or parked).
fn charge_poison(poison: &mut PoisonBudget, seq: i64) -> u32 {
    match poison {
        Some((s, n)) if *s == seq => {
            *n += 1;
            *n
        }
        _ => {
            *poison = Some((seq, 1));
            1
        }
    }
}

/// Clear the park budget when `seq` completed cleanly (its earlier failures
/// were transient).
fn clear_poison_for(poison: &mut PoisonBudget, seq: i64) {
    if matches!(poison, Some((s, _)) if *s == seq) {
        *poison = None;
    }
}

/// Read the emitter's delivery cursor (`0` when unset).
async fn read_cursor(pool: &PgPool) -> Result<i64, sqlx::Error> {
    outbox::cursor(pool, OutboxReader::FHIR_OUTBOUND).await
}

/// Serialize one mapped FHIR resource for publishing. A serialization failure
/// is a mapping fault for the row (surfaced, and poison-parked after its retry
/// budget) — never published as an empty message.
fn resource_payload(resource_type: &str, resource: &Value) -> Result<Vec<u8>, ProcessError> {
    serde_json::to_vec(resource)
        .map_err(|e| ProcessError::Map(format!("serialize {resource_type}: {e}")))
}

/// Advance the emitter's delivery cursor to `seq`; the registry keeps it
/// monotonic, so a concurrent or delayed pass never moves it back.
async fn write_cursor(pool: &PgPool, seq: i64) -> Result<(), sqlx::Error> {
    outbox::advance(pool, OutboxReader::FHIR_OUTBOUND, seq).await
}

/// The topic routing key for one emitted FHIR resource:
/// `<resource_type>.<template_id>` on the PHI exchange, with both segments
/// sanitised to single topic words (a dotted OPT id must not split the key).
fn routing_key(resource_type: &str, template_id: &str) -> String {
    format!("{}.{}", sanitize(resource_type), sanitize(template_id))
}

/// Collapse any AMQP-topic-hostile character (dots, spaces, …) to `_`.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Publish with exponential backoff, up to `publish_max_retries` extra attempts.
async fn publish_with_retry(
    publisher: &dyn EventPublisher,
    routing_key: &str,
    payload: &[u8],
    config: &FhirOutboundConfig,
) -> Result<(), EventError> {
    (|| async { publisher.publish(routing_key, payload).await })
        .retry(ExponentialBuilder::default().with_max_times(config.publish_max_retries))
        .notify(|e, d| tracing::warn!("fhir outbound publish retry in {d:?}: {e}"))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_key_sanitises_both_segments() {
        assert_eq!(
            routing_key("Observation", "minimal_evaluation.en.v1"),
            "Observation.minimal_evaluation_en_v1"
        );
    }

    #[test]
    fn poison_budget_charges_per_seq_and_resets_on_new_seq() {
        let mut poison: PoisonBudget = None;
        assert_eq!(charge_poison(&mut poison, 7), 1);
        assert_eq!(charge_poison(&mut poison, 7), 2);
        // A different failing row resets the budget (the old blocker is gone).
        assert_eq!(charge_poison(&mut poison, 9), 1);
        // A clean completion clears it.
        clear_poison_for(&mut poison, 9);
        assert!(poison.is_none());
        // Clearing a non-matching seq is a no-op.
        assert_eq!(charge_poison(&mut poison, 11), 1);
        clear_poison_for(&mut poison, 12);
        assert_eq!(poison, Some((11, 1)));
    }
}
