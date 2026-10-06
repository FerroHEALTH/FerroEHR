// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The non-blocking audit sender: a bounded `tokio::mpsc` drained by a
//! background task that renders each record once and fans it out to the
//! enabled sinks.
//!
//! The request path only [`AuditSender::emit`]s (a `try_send`, never blocking
//! or awaiting). The drain task optionally enriches the patient subject
//! (background only — never on the request path), renders the record — the
//! FHIR R4 `AuditEvent` (IHE BALP, [`super::fhir`]) and/or the DICOM PS3.15
//! §A.5 XML ([`super::message`]) — and delivers it:
//!
//! - **store** (`[audit.store]`, the durability anchor): the record is
//!   `INSERTed` into the local Audit Record Repository first, with bounded
//!   retries; a persistent write failure marks the store unhealthy, which
//!   under `fail_mode = closed` makes every subsequent auditable request
//!   answer `503` until a write succeeds again (prospective "no un-audited
//!   PHI access" — the emit path is non-blocking, so the failing record's
//!   own request has already been answered).
//! - **syslog** (`[audit.syslog]`): the classic ITI-20 feed, sent in-drain;
//!   with the store on, a successful send stamps `delivered_syslog_at`.
//! - **`fhir_feed`** (`[audit.fhir_feed]`, ITI-20 ATX:FHIR Feed): with the
//!   store on, a dedicated outbox worker polls undelivered rows
//!   (`delivered_fhir_feed_at IS NULL`) and POSTs them — a down ARR loses
//!   nothing; with the store off it ships in-drain with bounded retries.
//!
//! Failures are fail-open (drop + `tracing` + a `metrics` counter) or
//! fail-closed (the REST layer returns `503`); see [`EmitOutcome`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the drain carries an \
              already-rendered FHIR document, never a typed resource"
)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use backon::{ExponentialBuilder, Retryable};
use sqlx::PgPool;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use super::event::{AuditEvent, EmitOutcome};

use crate::system_log::AuditError;
use crate::system_log::chain_check::ChainCheck;
use crate::system_log::config::{AuditConfig, FailMode, FhirFeedConfig, Retention, StoreConfig};
use crate::system_log::message::{AuditContext, AuditMessage};
use crate::system_log::store::AuditStore;
use crate::system_log::syslog::{Transport, assemble_syslog};

/// A background-only, indexed lookup of the EHR's patient subject id
/// (`ehr.subject_id`). Supplied by the binary (keeps the request path
/// DB-free); given an `ehr_id`, resolves the subject id or `None`.
pub type SubjectResolver =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;

/// `metrics` counter names.
pub const METRIC_EMITTED: &str = "atna_audit_emitted_total";
/// Records dropped because the queue was full or the drain stopped.
pub const METRIC_DROPPED: &str = "atna_audit_dropped_total";
/// Auditable operations rejected under `fail_mode = closed` (queue full, or
/// the local store is unhealthy). Each rejection is a `503` at the REST layer.
pub const METRIC_REJECTED: &str = "atna_audit_rejected_total";
/// Records dropped in the drain because a rendering failed.
///
/// SM master02 §Component table names the System Log as an "IHE
/// ATNA-compliant system log"; silent audit loss would undermine that, so
/// every serialize-drop is metered.
pub const METRIC_SERIALIZE_FAILED: &str = "atna_audit_serialize_failed_total";
/// Records successfully written to a sink (label `sink`:
/// `store`/`syslog`/`fhir_feed`).
pub const METRIC_SENT: &str = "atna_audit_sent_total";
/// Sink delivery failures (label `sink`, post-enqueue).
pub const METRIC_SEND_FAILED: &str = "atna_audit_send_failed_total";
/// Rows reaped by the retention job.
pub const METRIC_REAPED: &str = "atna_audit_reaped_total";

/// How often the retention reaper runs.
const REAP_INTERVAL: Duration = Duration::from_hours(1);

/// The drain's batch bound: how many queued events one `recv_many` takes at
/// once (the store sink writes the whole batch in one multi-row INSERT).
const DRAIN_BATCH: usize = 256;

/// Drop warnings are rate-limited to one per this interval, carrying the
/// count of records dropped since the previous warning (per-record WARNs at
/// a loaded write rate flood the log and perturb the very load being
/// measured; `atna_audit_dropped_total` still counts every drop exactly).
const DROP_WARN_INTERVAL: Duration = Duration::from_secs(5);

/// The cheaply-cloneable handle the REST layer emits through.
#[derive(Debug, Clone)]
pub struct AuditSender {
    inner: Arc<SenderInner>,
}

#[derive(Debug)]
struct SenderInner {
    tx: mpsc::Sender<AuditEvent>,
    enabled: bool,
    suppress_login_events: bool,
    fail_mode: FailMode,
    /// The access-logging declaration surface: the header a caller declares a
    /// purpose of use in, the codes this deployment accepts (empty accepts
    /// any), and the legal basis it processes under.
    purpose_header: String,
    purpose_codes: Vec<String>,
    legal_basis: Option<String>,
    /// The compiled `[audit.categories]` map every record is classified through.
    categories: crate::system_log::categories::CategoryMap,
    /// Whether the local store is currently accepting writes (`true` when the
    /// store is disabled — health then rides on the queue alone). Written by
    /// the drain, read by [`AuditSender::emit`] under `fail_mode = closed`.
    store_healthy: Arc<AtomicBool>,
    /// Drops since the last rate-limited warning (drained by the warner).
    dropped_since_warn: AtomicU64,
    /// Milliseconds since `warn_epoch` of the last emitted drop warning.
    last_drop_warn_ms: AtomicU64,
    /// The process-local time anchor for `last_drop_warn_ms`.
    warn_epoch: Instant,
}

impl SenderInner {
    /// Count one drop and emit at most one warning per
    /// [`DROP_WARN_INTERVAL`], carrying the accumulated count.
    fn warn_dropped(&self) {
        let pending = self.dropped_since_warn.fetch_add(1, Ordering::Relaxed) + 1;
        let now_ms = u64::try_from(self.warn_epoch.elapsed().as_millis()).unwrap_or(u64::MAX);
        let last = self.last_drop_warn_ms.load(Ordering::Relaxed);
        let interval_ms = u64::try_from(DROP_WARN_INTERVAL.as_millis()).unwrap_or(u64::MAX);
        let due = last == 0 || now_ms.saturating_sub(last) >= interval_ms;
        if due
            && self
                .last_drop_warn_ms
                .compare_exchange(last, now_ms.max(1), Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            let count = self.dropped_since_warn.swap(0, Ordering::Relaxed);
            tracing::warn!(
                dropped = count.max(pending.min(1)),
                "ATNA audit records dropped (queue full or drain stopped); fail_mode={:?} — \
                 count since the previous warning; atna_audit_dropped_total is exact",
                self.fail_mode
            );
        }
    }
}

impl AuditSender {
    /// Whether auditing is on (the master switch).
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.inner.enabled
    }

    /// Whether login / user-authentication success events are suppressed.
    #[must_use]
    pub fn suppress_login_events(&self) -> bool {
        self.inner.suppress_login_events
    }

    /// The request header a caller declares its purpose of use in, lowercased.
    #[must_use]
    pub fn purpose_header(&self) -> &str {
        &self.inner.purpose_header
    }

    /// Whether `code` is one this deployment records.
    ///
    /// An empty allow-list accepts any non-empty code, which is the default:
    /// a deployment that has agreed no vocabulary still benefits from
    /// recording what the caller said.
    #[must_use]
    pub fn accepts_purpose(&self, code: &str) -> bool {
        !code.is_empty()
            && (self.inner.purpose_codes.is_empty()
                || self.inner.purpose_codes.iter().any(|known| known == code))
    }

    /// The legal basis this deployment records on every access event.
    #[must_use]
    pub fn legal_basis(&self) -> Option<&str> {
        self.inner.legal_basis.as_deref()
    }

    /// The category map every access record is classified through.
    #[must_use]
    pub fn categories(&self) -> &crate::system_log::categories::CategoryMap {
        &self.inner.categories
    }

    /// Enqueue an event (non-blocking). Never awaits, never blocks the request.
    ///
    /// A full queue (or a stopped drain) is metered and mapped through the
    /// configured [`FailMode`]: `open` → [`EmitOutcome::Dropped`] (the request
    /// proceeds), `closed` → [`EmitOutcome::Rejected`] (the REST layer returns
    /// `503`). Under `fail_mode = closed` an unhealthy local store also
    /// rejects — the event is still enqueued (best-effort: it delivers when
    /// the store recovers), but the operation must not be reported as having
    /// been audited.
    ///
    /// Every record is stamped with the digest of the category map in force.
    pub fn emit(&self, mut event: AuditEvent) -> EmitOutcome {
        event.category_map_digest = Some(self.inner.categories.digest());
        crate::telemetry::metrics::metrics()
            .atna_audit_emitted
            .add(1, &[]);
        let enqueued = self.inner.tx.try_send(event).is_ok();
        if !enqueued {
            crate::telemetry::metrics::metrics()
                .atna_audit_dropped
                .add(1, &[]);
            self.inner.warn_dropped();
        }
        match self.inner.fail_mode {
            FailMode::Open => {
                if enqueued {
                    EmitOutcome::Enqueued
                } else {
                    EmitOutcome::Dropped
                }
            }
            FailMode::Closed => {
                if !enqueued {
                    crate::telemetry::metrics::metrics()
                        .atna_audit_rejected
                        .add(1, &[]);
                    return EmitOutcome::Rejected;
                }
                if !self.inner.store_healthy.load(Ordering::Relaxed) {
                    // No un-audited PHI access: the store stopped accepting
                    // writes, so auditable operations 503 until it recovers
                    // (the enqueued record delivers on recovery).
                    crate::telemetry::metrics::metrics()
                        .atna_audit_rejected
                        .add(1, &[]);
                    tracing::warn!(
                        "ATNA fail-closed: local audit store unhealthy — rejecting the operation"
                    );
                    return EmitOutcome::Rejected;
                }
                EmitOutcome::Enqueued
            }
        }
    }
}

/// Owns the background tasks; the binary keeps it and drains it on shutdown.
#[derive(Debug)]
pub struct AuditHandle {
    join: JoinHandle<()>,
    /// The forwarding/retention workers (outbox poller, reaper). Aborted on
    /// shutdown after the drain flushes: their state is durable (the store),
    /// so undelivered rows simply ship on the next boot.
    workers: Vec<JoinHandle<()>>,
    /// The result of the scheduled hash-chain verification, when it runs.
    chain_check: Option<ChainCheck>,
}

impl AuditHandle {
    /// Returns the shared result of the scheduled hash-chain verification;
    /// `None` when the local store is off or `verify_interval_seconds` is `0`.
    #[must_use]
    pub fn chain_check(&self) -> Option<ChainCheck> {
        self.chain_check.clone()
    }

    /// Await the drain task, bounded by `timeout`, then stop the background
    /// workers. All [`AuditSender`] clones must be dropped first so the
    /// channel closes and the drain flushes then exits.
    pub async fn shutdown(self, timeout: Duration) {
        match tokio::time::timeout(timeout, self.join).await {
            Ok(Ok(())) => tracing::debug!("ATNA audit drain flushed and exited"),
            Ok(Err(e)) => tracing::warn!("ATNA audit drain task panicked: {e}"),
            Err(_) => tracing::warn!("ATNA audit drain did not flush within {timeout:?}"),
        }
        for worker in self.workers {
            worker.abort();
        }
    }
}

/// The resolved sink set the drain fans out to.
struct Sinks {
    store: Option<AuditStore>,
    syslog: Option<Transport>,
    /// The in-drain FHIR feed (store OFF only; with the store on the outbox
    /// worker owns delivery).
    direct_feed: Option<FeedClient>,
    /// Wakes the outbox worker after a store insert.
    feed_notify: Option<Arc<Notify>>,
}

/// One drained event with its resolved subject id and rendered FHIR
/// `AuditEvent` document, the unit every sink consumes.
type DrainRecord = (AuditEvent, Option<String>, Option<serde_json::Value>);

/// The drain's `ehr_id` → subject-id memo (an EHR's subject is immutable for
/// audit purposes, so one lookup serves every later record).
type SubjectCache = moka::future::Cache<String, Option<String>>;

/// The ITI-20 ATX:FHIR Feed HTTP client.
#[derive(Clone)]
struct FeedClient {
    client: reqwest::Client,
    endpoint: String,
    max_retries: usize,
}

impl FeedClient {
    fn new(config: &FhirFeedConfig) -> Self {
        FeedClient {
            client: reqwest::Client::new(),
            endpoint: format!("{}/AuditEvent", config.url.expose().trim_end_matches('/')),
            max_retries: config.max_retries,
        }
    }

    /// POST one FHIR `AuditEvent` (ITI-20 ATX:FHIR Feed) with bounded jittered
    /// retries.
    async fn post(&self, body: &serde_json::Value) -> Result<(), AuditError> {
        let send = || async {
            let resp = self
                .client
                .post(&self.endpoint)
                .header(http::header::CONTENT_TYPE, "application/fhir+json")
                .json(body)
                .send()
                .await
                .map_err(|e| AuditError::Transport(Box::new(e)))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(AuditError::FeedRejected(resp.status()))
            }
        };
        send.retry(
            ExponentialBuilder::default()
                .with_jitter()
                .with_max_times(self.max_retries),
        )
        .await
    }
}

/// Start the audit subsystem: resolve the sinks and spawn the drain plus any
/// forwarding/retention workers. `pool` backs the local store (`None` in
/// store-less setups, e.g. transport-only tests).
///
/// # Errors
/// [`AuditError`] if the syslog transport cannot be established (UDP
/// bind/connect, or a TLS config/CA problem).
pub async fn start(
    config: AuditConfig,
    resolver: Option<SubjectResolver>,
    pool: Option<PgPool>,
) -> Result<(AuditSender, AuditHandle), AuditError> {
    let syslog = if config.syslog.enabled {
        Some(
            Transport::connect(&config.syslog)
                .await
                .map_err(|e| AuditError::Transport(Box::new(e)))?,
        )
    } else {
        None
    };

    let store = if config.store.enabled {
        if let Some(pool) = pool {
            Some(AuditStore::new(pool))
        } else {
            tracing::warn!(
                "ATNA audit store enabled but no database pool supplied — store sink off"
            );
            None
        }
    } else {
        None
    };

    let mut workers = Vec::new();
    let mut feed_notify = None;
    let mut direct_feed = None;
    if config.fhir_feed.enabled {
        let feed = FeedClient::new(&config.fhir_feed);
        if let Some(store) = store.clone() {
            // Outbox-durable delivery: the worker polls undelivered rows.
            let notify = Arc::new(Notify::new());
            feed_notify = Some(Arc::clone(&notify));
            workers.push(tokio::spawn(feed_outbox(
                store,
                feed,
                config.fhir_feed.clone(),
                notify,
            )));
        } else {
            direct_feed = Some(feed);
        }
    }
    let chain_check = store
        .as_ref()
        .and_then(|store| spawn_store_upkeep(store, &config.store, &mut workers));

    if store.is_none() && syslog.is_none() && !config.fhir_feed.enabled {
        tracing::warn!(
            "ATNA auditing is enabled with no active sink — records will be rendered and discarded"
        );
    }

    let ctx = AuditContext {
        source_id: config.source_id.clone(),
        enterprise_site_id: config.enterprise_site_id.clone().unwrap_or_default(),
        server_ip: config.server_host.clone().unwrap_or_default(),
        value_if_missing: config.value_if_missing.clone(),
    };

    let categories = config
        .categories
        .compile()
        .map_err(AuditError::CategoryMap)?;

    let store_healthy = Arc::new(AtomicBool::new(true));
    let (tx, rx) = mpsc::channel(config.queue_capacity.max(1));
    let sinks = Sinks {
        store,
        syslog,
        direct_feed,
        feed_notify,
    };
    let join = tokio::spawn(drain(
        rx,
        sinks,
        ctx,
        resolver,
        config.resolve_subject,
        Arc::clone(&store_healthy),
    ));

    let sender = AuditSender {
        inner: Arc::new(SenderInner {
            tx,
            enabled: config.enabled,
            suppress_login_events: config.suppress_login_events,
            purpose_header: config.purpose_header.to_ascii_lowercase(),
            purpose_codes: config.purpose_codes.clone(),
            legal_basis: config.legal_basis.clone(),
            categories,
            fail_mode: config.fail_mode,
            store_healthy,
            dropped_since_warn: AtomicU64::new(0),
            last_drop_warn_ms: AtomicU64::new(0),
            warn_epoch: Instant::now(),
        }),
    };
    Ok((
        sender,
        AuditHandle {
            join,
            workers,
            chain_check,
        },
    ))
}

/// Spawns the local store's upkeep workers onto `workers`: the retention
/// reaper unless records are kept forever, and the scheduled hash-chain
/// verification unless `verify_interval_seconds` is `0`, whose shared result is
/// returned.
fn spawn_store_upkeep(
    store: &AuditStore,
    config: &StoreConfig,
    workers: &mut Vec<JoinHandle<()>>,
) -> Option<ChainCheck> {
    if config.retention() != Retention::Forever {
        workers.push(tokio::spawn(reaper(store.clone(), config.retention())));
    }
    if config.verify_interval_seconds == 0 {
        return None;
    }
    let check = ChainCheck::new();
    workers.push(tokio::spawn(crate::system_log::chain_check::scheduled(
        store.clone(),
        check.clone(),
        Duration::from_secs(config.verify_interval_seconds),
    )));
    Some(check)
}

async fn drain(
    mut rx: mpsc::Receiver<AuditEvent>,
    mut sinks: Sinks,
    ctx: AuditContext,
    resolver: Option<SubjectResolver>,
    resolve_subject: bool,
    store_healthy: Arc<AtomicBool>,
) {
    // Batched receive: one loop turn takes up to DRAIN_BATCH queued events, so
    // the store sink persists them in one multi-row INSERT rather than per-event
    // round trips.
    let mut batch: Vec<AuditEvent> = Vec::with_capacity(DRAIN_BATCH);
    // The subject lookup memo: an EHR's subject id is immutable for audit
    // purposes at write-path rates, and one awaited per-event lookup is the
    // remaining drain bottleneck.
    let subject_cache: SubjectCache = moka::future::Cache::builder()
        .max_capacity(100_000)
        .time_to_live(Duration::from_hours(1))
        .build();
    loop {
        batch.clear();
        if rx.recv_many(&mut batch, DRAIN_BATCH).await == 0 {
            break; // channel closed and drained
        }

        if resolve_subject && let Some(resolve) = &resolver {
            memoize_subjects(&batch, resolve, &subject_cache).await;
        }
        let records = render_batch(&mut batch, &ctx, &subject_cache, resolve_subject, &sinks).await;

        // The sink order is the behaviour: the store is the durability
        // anchor and is written first, so a syslog send can stamp the row it
        // already persisted.
        let row_ids = write_to_store(&sinks, &records, &store_healthy).await;
        if let Some(transport) = &mut sinks.syslog {
            send_to_syslog(transport, &records, &row_ids, sinks.store.as_ref(), &ctx).await;
        }
        if let Some(feed) = &sinks.direct_feed {
            send_to_direct_feed(feed, &records).await;
        }
    }
    tracing::debug!("ATNA audit drain: channel closed, task exiting");
}

/// Resolves the subject id of every EHR the batch references that the memo
/// does not already hold, concurrently, and memoizes each answer.
async fn memoize_subjects(batch: &[AuditEvent], resolve: &SubjectResolver, cache: &SubjectCache) {
    let mut pending: Vec<String> = batch
        .iter()
        .filter_map(|event| event.ehr_id.clone())
        .filter(|ehr_id| !cache.contains_key(ehr_id))
        .collect();
    pending.sort_unstable();
    pending.dedup();
    let mut lookups = tokio::task::JoinSet::new();
    for ehr_id in pending {
        let lookup = resolve(ehr_id.clone());
        lookups.spawn(async move { (ehr_id, lookup.await) });
    }
    while let Some(joined) = lookups.join_next().await {
        if let Ok((ehr_id, subject)) = joined {
            cache.insert(ehr_id, subject).await;
        }
    }
}

/// Drains the batch into records, attaching each event's memoized subject and,
/// when any FHIR consumer is enabled, its rendered `AuditEvent` document.
///
/// The document is rendered once here because both the store and the FHIR feed
/// consume it; with neither sink enabled nothing is rendered at all.
async fn render_batch(
    batch: &mut Vec<AuditEvent>,
    ctx: &AuditContext,
    subject_cache: &SubjectCache,
    resolve_subject: bool,
    sinks: &Sinks,
) -> Vec<DrainRecord> {
    let render_fhir = sinks.store.is_some() || sinks.direct_feed.is_some();
    let mut records: Vec<DrainRecord> = Vec::with_capacity(batch.len());
    for event in batch.drain(..) {
        let subject = match (resolve_subject, &event.ehr_id) {
            (true, Some(ehr_id)) => subject_cache.get(ehr_id).await.flatten(),
            _ => None,
        };
        let rendered = render_fhir
            .then(|| render_audit_event(&event, ctx, subject.as_deref()))
            .flatten();
        records.push((event, subject, rendered));
    }
    records
}

/// Writes the batch to the local Audit Record Repository, returning the stored
/// row id of each record (`None` where nothing was stored).
///
/// Without the syslog sink no per-row delivery stamp is ever needed, so the
/// whole batch lands in ONE multi-row INSERT; with syslog on, the per-record
/// path keeps each row id for `delivered_syslog_at`.
async fn write_to_store(
    sinks: &Sinks,
    records: &[DrainRecord],
    store_healthy: &AtomicBool,
) -> Vec<Option<uuid::Uuid>> {
    let Some(store) = &sinks.store else {
        return vec![None; records.len()];
    };
    if sinks.syslog.is_none() {
        store_whole_batch(store, records, store_healthy, sinks.feed_notify.as_ref()).await;
        return vec![None; records.len()];
    }
    store_each_record(store, records, store_healthy, sinks.feed_notify.as_ref()).await
}

/// Persists the batch in one multi-row INSERT with bounded jittered retries,
/// updating the store health flag the fail-closed mode reads.
async fn store_whole_batch(
    store: &AuditStore,
    records: &[DrainRecord],
    store_healthy: &AtomicBool,
    feed_notify: Option<&Arc<Notify>>,
) {
    let insert = || async { store.insert_batch(records).await };
    #[expect(
        clippy::as_conversions,
        reason = "the batch record count widens exactly: usize is at most 64 bits on \
                  every supported target"
    )]
    let count = records.len() as u64;
    match insert
        .retry(
            ExponentialBuilder::default()
                .with_jitter()
                .with_max_times(2),
        )
        .await
    {
        Ok(()) => {
            store_healthy.store(true, Ordering::Relaxed);
            crate::telemetry::metrics::metrics()
                .atna_audit_sent
                .add(count, &[opentelemetry::KeyValue::new("sink", "store")]);
            if let Some(notify) = feed_notify {
                notify.notify_one();
            }
        }
        Err(e) => {
            store_healthy.store(false, Ordering::Relaxed);
            crate::telemetry::metrics::metrics()
                .atna_audit_send_failed
                .add(count, &[opentelemetry::KeyValue::new("sink", "store")]);
            tracing::warn!("ATNA audit store batch write failed: {e}");
        }
    }
}

/// Persists the batch one record at a time, returning the row id of each
/// record a syslog send may later stamp as delivered.
async fn store_each_record(
    store: &AuditStore,
    records: &[DrainRecord],
    store_healthy: &AtomicBool,
    feed_notify: Option<&Arc<Notify>>,
) -> Vec<Option<uuid::Uuid>> {
    let mut row_ids: Vec<Option<uuid::Uuid>> = vec![None; records.len()];
    for (index, (event, subject, rendered)) in records.iter().enumerate() {
        let Some(rendered) = rendered else {
            continue;
        };
        let insert = || async { store.insert(event, subject.as_deref(), rendered).await };
        match insert
            .retry(
                ExponentialBuilder::default()
                    .with_jitter()
                    .with_max_times(2),
            )
            .await
        {
            Ok(id) => {
                if let Some(slot) = row_ids.get_mut(index) {
                    *slot = Some(id);
                }
                store_healthy.store(true, Ordering::Relaxed);
                crate::telemetry::metrics::metrics()
                    .atna_audit_sent
                    .add(1, &[opentelemetry::KeyValue::new("sink", "store")]);
                if let Some(notify) = feed_notify {
                    notify.notify_one();
                }
            }
            Err(e) => {
                store_healthy.store(false, Ordering::Relaxed);
                crate::telemetry::metrics::metrics()
                    .atna_audit_send_failed
                    .add(1, &[opentelemetry::KeyValue::new("sink", "store")]);
                tracing::warn!("ATNA audit store write failed: {e}");
            }
        }
    }
    row_ids
}

/// Feeds every record to the classic ITI-20 syslog sink, which is inherently
/// sequential: one datagram or frame per record.
async fn send_to_syslog(
    transport: &mut Transport,
    records: &[DrainRecord],
    row_ids: &[Option<uuid::Uuid>],
    store: Option<&AuditStore>,
    ctx: &AuditContext,
) {
    for (index, (event, subject, _)) in records.iter().enumerate() {
        let row_id = row_ids.get(index).copied().flatten();
        send_one_to_syslog(transport, event, subject.as_deref(), row_id, store, ctx).await;
    }
}

/// Sends one record as DICOM PS3.15 §A.5 XML and, on success, stamps
/// `delivered_syslog_at` on the row the store already holds for it.
async fn send_one_to_syslog(
    transport: &mut Transport,
    event: &AuditEvent,
    subject: Option<&str>,
    row_id: Option<uuid::Uuid>,
    store: Option<&AuditStore>,
    ctx: &AuditContext,
) {
    let xml = match AuditMessage::build(event, ctx, subject).to_xml() {
        Ok(xml) => xml,
        Err(e) => {
            crate::telemetry::metrics::metrics()
                .atna_audit_serialize_failed
                .add(1, &[]);
            tracing::warn!("ATNA audit message serialization failed: {e}");
            return;
        }
    };
    let syslog = assemble_syslog(&ctx.server_ip, &ctx.source_id, &event.timestamp, &xml);
    match transport.send(&syslog).await {
        Ok(()) => {
            crate::telemetry::metrics::metrics()
                .atna_audit_sent
                .add(1, &[opentelemetry::KeyValue::new("sink", "syslog")]);
            if let (Some(store), Some(id)) = (store, row_id) {
                store.mark_syslog_delivered(id).await;
            }
        }
        Err(e) => {
            crate::telemetry::metrics::metrics()
                .atna_audit_send_failed
                .add(1, &[opentelemetry::KeyValue::new("sink", "syslog")]);
            tracing::warn!("ATNA audit syslog send failed: {e}");
        }
    }
}

/// Ships every rendered document to the ITI-20 ATX:FHIR Feed in-drain, the
/// store-off path (with the store on, the outbox worker owns delivery).
async fn send_to_direct_feed(feed: &FeedClient, records: &[DrainRecord]) {
    for (_, _, rendered) in records {
        let Some(body) = rendered else {
            continue;
        };
        match feed.post(body).await {
            Ok(()) => {
                crate::telemetry::metrics::metrics()
                    .atna_audit_sent
                    .add(1, &[opentelemetry::KeyValue::new("sink", "fhir_feed")]);
            }
            Err(e) => {
                crate::telemetry::metrics::metrics()
                    .atna_audit_send_failed
                    .add(1, &[opentelemetry::KeyValue::new("sink", "fhir_feed")]);
                tracing::warn!("ATNA audit FHIR feed send failed: {e}");
            }
        }
    }
}

/// The FHIR R4 `AuditEvent` document for one resolved record, or `None` when
/// the rendering failed (metered + logged, never silent).
#[cfg(feature = "fhir")]
fn render_audit_event(
    event: &AuditEvent,
    ctx: &AuditContext,
    subject: Option<&str>,
) -> Option<serde_json::Value> {
    match crate::system_log::fhir::to_fhir(event, ctx, subject) {
        Ok(document) => Some(document),
        Err(e) => {
            crate::telemetry::metrics::metrics()
                .atna_audit_serialize_failed
                .add(1, &[]);
            tracing::warn!("ATNA audit FHIR rendering failed: {e}");
            None
        }
    }
}

/// A slim build renders no FHIR document. Unreachable: [`start`] refuses a
/// configuration that enables the store or the FHIR feed, and nothing else
/// consumes the document.
#[cfg(not(feature = "fhir"))]
fn render_audit_event(
    _event: &AuditEvent,
    _ctx: &AuditContext,
    _subject: Option<&str>,
) -> Option<serde_json::Value> {
    None
}

/// The ITI-20 ATX:FHIR Feed outbox worker: ship undelivered rows oldest-first,
/// stamping `delivered_fhir_feed_at` per success. On a delivery failure the
/// batch stops (rows stay pending — durable) and the worker backs off one poll
/// interval.
async fn feed_outbox(
    store: AuditStore,
    feed: FeedClient,
    config: FhirFeedConfig,
    notify: Arc<Notify>,
) {
    let idle = Duration::from_millis(config.poll_interval_ms.max(100));
    loop {
        let batch = match store.pending_fhir_feed(config.batch_size).await {
            Ok(batch) => batch,
            Err(e) => {
                tracing::warn!("ATNA audit feed outbox poll failed: {e}");
                tokio::time::sleep(idle).await;
                continue;
            }
        };
        if batch.is_empty() {
            tokio::select! {
                () = notify.notified() => {}
                () = tokio::time::sleep(idle) => {}
            }
            continue;
        }
        for (id, body) in batch {
            match feed.post(&body).await {
                Ok(()) => {
                    store.mark_fhir_feed_delivered(id).await;
                    crate::telemetry::metrics::metrics()
                        .atna_audit_sent
                        .add(1, &[opentelemetry::KeyValue::new("sink", "fhir_feed")]);
                }
                Err(e) => {
                    crate::telemetry::metrics::metrics()
                        .atna_audit_send_failed
                        .add(1, &[opentelemetry::KeyValue::new("sink", "fhir_feed")]);
                    tracing::warn!("ATNA audit FHIR feed delivery failed (row stays pending): {e}");
                    tokio::time::sleep(idle).await;
                    break;
                }
            }
        }
    }
}

/// The hourly retention reaper, for a horizon in days or in calendar years.
#[expect(
    clippy::infinite_loop,
    reason = "the retention reaper is a detached background task with no \
              shutdown channel — it ends when the runtime drops the task; \
              declaring `-> !` is not an option because `tokio::spawn` would \
              then need the never type as a type argument, which is unstable"
)]
async fn reaper(store: AuditStore, retention: Retention) {
    loop {
        tokio::time::sleep(REAP_INTERVAL).await;
        let reaped = match retention {
            Retention::Forever => Ok(0),
            Retention::Days(days) => store.reap(days).await,
            Retention::Years(years) => store.reap_years(years).await,
        };
        match reaped {
            Ok(0) => {}
            Ok(n) => {
                crate::telemetry::metrics::metrics()
                    .atna_audit_reaped
                    .add(n, &[]);
                tracing::debug!("ATNA audit retention reaped {n} records");
            }
            Err(e) => tracing::warn!("ATNA audit retention reap failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Mutex};

    use super::*;
    use crate::system_log::config::{StoreConfig, SyslogConfig};
    use crate::system_log::event::{EventActionCode, EventOutcome, ObjectClass};

    fn udp_config() -> AuditConfig {
        AuditConfig {
            enabled: true,
            // No DB pool in these tests — exercise the transport path alone.
            store: StoreConfig {
                enabled: false,
                retention_days: 0,
                retention_years: None,
                sgb_v_309_controller: false,
                verify_interval_seconds: 0,
            },
            syslog: SyslogConfig {
                enabled: true,
                transport: crate::system_log::config::Transport::Udp,
                // A port nothing listens on is fine: connected UDP send does
                // not fail.
                host: "127.0.0.1".to_owned(),
                port: 1,
                ..SyslogConfig::default()
            },
            queue_capacity: 8,
            ..AuditConfig::default()
        }
    }

    fn event() -> AuditEvent {
        AuditEvent::new(
            EventActionCode::Create,
            ObjectClass::Ehr,
            EventOutcome::Success,
        )
    }

    /// Receivers of senders built by [`bare_sender`]: parked here (never read)
    /// so the channel stays open and the queue can fill, without leaking them.
    static PARKED_RECEIVERS: LazyLock<Mutex<Vec<mpsc::Receiver<AuditEvent>>>> =
        LazyLock::new(|| Mutex::new(Vec::new()));

    fn bare_sender(fail_mode: FailMode, capacity: usize, store_healthy: bool) -> AuditSender {
        let (tx, rx) = mpsc::channel(capacity);
        // Keep the receiver alive but unread so the queue can fill.
        PARKED_RECEIVERS
            .lock()
            .expect("parked-receiver registry")
            .push(rx);
        AuditSender {
            inner: Arc::new(SenderInner {
                tx,
                enabled: true,
                suppress_login_events: true,
                purpose_header: "x-purpose-of-use".to_owned(),
                purpose_codes: Vec::new(),
                legal_basis: None,
                categories: crate::system_log::categories::CategoryMap::default(),
                fail_mode,
                store_healthy: Arc::new(AtomicBool::new(store_healthy)),
                dropped_since_warn: AtomicU64::new(0),
                last_drop_warn_ms: AtomicU64::new(0),
                warn_epoch: Instant::now(),
            }),
        }
    }

    #[tokio::test]
    async fn emit_enqueues_and_drains_on_shutdown() {
        let (sender, handle) = start(udp_config(), None, None).await.expect("start");
        assert!(sender.enabled());
        assert_eq!(sender.emit(event()), EmitOutcome::Enqueued);
        drop(sender);
        handle.shutdown(Duration::from_secs(2)).await;
    }

    #[tokio::test]
    async fn fail_open_drops_when_full() {
        let sender = bare_sender(FailMode::Open, 1, true);
        assert_eq!(sender.emit(event()), EmitOutcome::Enqueued);
        assert_eq!(sender.emit(event()), EmitOutcome::Dropped);
    }

    #[tokio::test]
    async fn fail_closed_rejects_when_full() {
        let sender = bare_sender(FailMode::Closed, 1, true);
        assert_eq!(sender.emit(event()), EmitOutcome::Enqueued);
        assert_eq!(sender.emit(event()), EmitOutcome::Rejected);
    }

    #[tokio::test]
    async fn fail_closed_rejects_while_store_unhealthy() {
        // The event is still enqueued (delivers on recovery) but the
        // operation is rejected: no un-audited PHI access.
        let sender = bare_sender(FailMode::Closed, 8, false);
        assert_eq!(sender.emit(event()), EmitOutcome::Rejected);
    }

    #[tokio::test]
    async fn fail_open_ignores_store_health() {
        let sender = bare_sender(FailMode::Open, 8, false);
        assert_eq!(sender.emit(event()), EmitOutcome::Enqueued);
    }
}
