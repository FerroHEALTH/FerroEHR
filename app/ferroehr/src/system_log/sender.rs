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
//! - **syslog** (`[audit.syslog]`): the classic ITI-20 feed. The drain stores
//!   each frame in `ihe-atna`'s spool and the crate's forwarder delivers it
//!   (ITI TF-2 §3.20.4.1.1); the local store keeps no syslog frame.
//! - **`fhir_feed`** (`[audit.fhir_feed]`, ITI-20 ATX:FHIR Feed): the crate's
//!   forwarder delivers from the local store (`delivered_fhir_feed_at IS
//!   NULL`), so a down ARR loses nothing, or with the store off from an
//!   in-memory spool.
//!
//! Failures are fail-open (drop + `tracing` + a `metrics` counter) or
//! fail-closed (the REST layer returns `503`); see [`EmitOutcome`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the drain carries an \
              already-rendered FHIR document, never a typed resource"
)]

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use backon::{ExponentialBuilder, Retryable};
use ihe_atna::atna::forwarder::{Forwarder, Outbox, Status};
#[cfg(feature = "fhir")]
use ihe_atna::atna::spool::Spool;
use sqlx::PgPool;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use super::event::{AuditEvent, EmitOutcome};

use crate::system_log::AuditError;
use crate::system_log::chain_check::ChainCheck;
use crate::system_log::config::{AuditConfig, FailMode, Retention, StoreConfig};
use crate::system_log::message::AuditContext;
use crate::system_log::store::AuditStore;
use crate::system_log::syslog::SyslogRoute;

/// The patient subject of an EHR: `EHR_STATUS.subject.external_ref`'s id and
/// namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSubject {
    /// The subject's id value (`ehr.subject_id`).
    pub id: String,
    /// The namespace that issued it (`ehr.subject_namespace`), when stored.
    pub namespace: Option<String>,
}

/// What a [`SubjectResolver`] learned about an EHR's patient subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectLookup {
    /// The EHR names this subject.
    Found(ResolvedSubject),
    /// The EHR names no subject, or there is no such EHR.
    NoSubject,
    /// The lookup failed: the database could not be asked, or did not answer.
    Unavailable,
}

/// A background-only, indexed lookup of the EHR's patient subject.
///
/// Reads `ehr.subject_id` and `ehr.subject_namespace`. Supplied by the binary
/// (keeps the request path DB-free); given an `ehr_id`, says whether it found
/// a subject, found none, or could not ask.
pub type SubjectResolver =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = SubjectLookup> + Send>> + Send + Sync>;

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
    /// The purposes of use that mark an access as an emergency access, held
    /// by `ehds-logging`.
    emergency: ehds_logging::emergency::EmergencyPurposes,
    legal_basis: Option<String>,
    /// The compiled `[audit.categories]` map every record is classified through.
    categories: crate::system_log::categories::Classifier,
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

    /// Whether `purpose` is one of `[audit] emergency_purpose_codes`, the
    /// codes that mark an access as declared in the vital interest of the data
    /// subject (`docs/law/eu/ehds/text.html Art. 11(5)`).
    ///
    /// `None`, an empty list, and a code off the list are all `false`: the
    /// mark is never inferred, only read off a declared, agreed code.
    #[must_use]
    pub fn is_emergency_purpose(&self, purpose: Option<&str>) -> bool {
        purpose.is_some_and(|code| {
            self.inner
                .emergency
                .mark(&[ehds_logging::record::Purpose {
                    system: None,
                    code: code.to_owned(),
                }])
                .is_some()
        })
    }

    /// The legal basis this deployment records on every access event.
    #[must_use]
    pub fn legal_basis(&self) -> Option<&str> {
        self.inner.legal_basis.as_deref()
    }

    /// The category map every access record is classified through.
    #[must_use]
    pub fn categories(&self) -> &crate::system_log::categories::Classifier {
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
    /// Every record is stamped with the digest of the category map in force,
    /// and with the emergency mark when its declared purpose is one of
    /// `[audit] emergency_purpose_codes`.
    pub fn emit(&self, mut event: AuditEvent) -> EmitOutcome {
        event.category_map_digest = Some(self.inner.categories.digest());
        event.emergency_access = self.is_emergency_purpose(event.purpose.as_deref());
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
    /// The retention workers (reaper, chain verification). Aborted on
    /// shutdown after the drain flushes.
    workers: Vec<JoinHandle<()>>,
    /// The forwarders, stopped after the drain flushes so their connections
    /// close cleanly; what they hold is delivered by the next run.
    deliveries: Vec<Delivery>,
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

    /// Returns what each forwarder reports, for the health indicator; empty
    /// when no repository is configured.
    #[must_use]
    pub fn delivery(&self) -> DeliveryStatus {
        DeliveryStatus {
            sinks: self
                .deliveries
                .iter()
                .map(|delivery| (delivery.sink, Arc::clone(&delivery.status)))
                .collect(),
        }
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
        for delivery in self.deliveries {
            (delivery.stop)();
            if tokio::time::timeout(timeout, delivery.join).await.is_err() {
                tracing::warn!(
                    sink = delivery.sink,
                    "ATNA audit forwarder did not stop within {timeout:?}"
                );
            }
        }
    }
}

/// A status source of one forwarder.
type StatusFn = Arc<dyn Fn() -> Status + Send + Sync>;

/// One running forwarder: its sink name, its status, how to stop it, and its
/// task.
struct Delivery {
    sink: &'static str,
    status: StatusFn,
    stop: Arc<dyn Fn() + Send + Sync>,
    join: JoinHandle<()>,
}

impl std::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Delivery")
            .field("sink", &self.sink)
            .field("status", &(self.status)())
            .finish_non_exhaustive()
    }
}

/// What each forwarder reports: its sink (`syslog`, `fhir_feed`) and its
/// `ihe-atna` [`Status`].
#[derive(Clone, Default)]
pub struct DeliveryStatus {
    sinks: Vec<(&'static str, StatusFn)>,
}

impl std::fmt::Debug for DeliveryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.statuses()).finish()
    }
}

impl DeliveryStatus {
    /// Each sink with its status now.
    #[must_use]
    pub fn statuses(&self) -> Vec<(&'static str, Status)> {
        self.sinks
            .iter()
            .map(|(sink, status)| (*sink, status()))
            .collect()
    }
}

/// Spawns `forwarder` onto the runtime as the delivery of `sink`, with the
/// sampler that moves its delivered and retried counts onto the metrics.
fn deliver<O: Outbox + 'static>(sink: &'static str, forwarder: &Arc<Forwarder<O>>) -> Delivery {
    let join = tokio::spawn(Arc::clone(forwarder).run());
    let reader = Arc::clone(forwarder);
    let stopper = Arc::clone(forwarder);
    let sampled = Arc::clone(forwarder);
    tokio::spawn(sample(sink, move || sampled.status()));
    Delivery {
        sink,
        status: Arc::new(move || reader.status()),
        stop: Arc::new(move || stopper.stop()),
        join,
    }
}

/// How often a forwarder's counts move onto the metrics.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(15);

/// Moves the delivered and retried counts of `status` onto
/// `atna_audit_sent` and `atna_audit_send_failed` as they grow, until the
/// runtime ends the task.
#[expect(
    clippy::infinite_loop,
    reason = "a detached sampler ended by the runtime, as the reaper is"
)]
async fn sample(sink: &'static str, status: impl Fn() -> Status + Send + 'static) {
    let mut seen = status();
    loop {
        tokio::time::sleep(SAMPLE_INTERVAL).await;
        let now = status();
        let metrics = crate::telemetry::metrics::metrics();
        let label = [opentelemetry::KeyValue::new("sink", sink)];
        metrics
            .atna_audit_sent
            .add(now.delivered.saturating_sub(seen.delivered), &label);
        metrics
            .atna_audit_send_failed
            .add(now.retries.saturating_sub(seen.retries), &label);
        seen = now;
    }
}

/// The resolved sink set the drain fans out to.
struct Sinks {
    store: Option<AuditStore>,
    syslog: Option<SyslogRoute>,
    /// The FHIR feed's spool (store OFF only; with the store on the forwarder
    /// delivers from the store).
    #[cfg(feature = "fhir")]
    feed_spool: Option<Arc<Forwarder<Spool>>>,
    /// Wakes the store's feed forwarder after a store insert.
    feed_notify: Option<Arc<Notify>>,
}

/// One drained event with its resolved subject id and rendered FHIR
/// `AuditEvent` document, the unit every sink consumes.
type DrainRecord = (AuditEvent, Option<String>, Option<serde_json::Value>);

/// The drain's `ehr_id` → subject-id memo (an EHR's subject is immutable for
/// audit purposes, so one lookup serves every later record).
type SubjectCache = moka::future::Cache<String, Option<ResolvedSubject>>;

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
    let mut deliveries = Vec::new();
    let syslog = if config.syslog.enabled {
        let route = SyslogRoute::new(
            &config.syslog,
            config.server_host.as_deref().unwrap_or_default(),
            &config.source_id,
        )
        .await?;
        deliveries.push(deliver("syslog", &route.forwarder()));
        Some(route)
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
    #[cfg(feature = "fhir")]
    let (feed_notify, feed_spool) = start_feed(&config, store.as_ref(), &mut deliveries)?;
    #[cfg(not(feature = "fhir"))]
    let feed_notify = None;
    let chain_check = store
        .as_ref()
        .and_then(|store| spawn_store_upkeep(store, &config.store, &mut workers));

    if store.is_none() && syslog.is_none() && !config.fhir_feed.enabled {
        tracing::warn!(
            "ATNA auditing is enabled with no active sink — records will be rendered and discarded"
        );
    }

    let categories = config
        .categories
        .compile()
        .map_err(AuditError::CategoryMap)?;
    let emergency = emergency_purposes(&config)?;

    let ctx = AuditContext {
        source_id: config.source_id.clone(),
        enterprise_site_id: config.enterprise_site_id.clone().unwrap_or_default(),
        server_ip: config.server_host.clone().unwrap_or_default(),
        value_if_missing: config.value_if_missing.clone(),
        #[cfg(feature = "fhir")]
        recording: recording(&config, &categories, &emergency)?,
    };

    let store_healthy = Arc::new(AtomicBool::new(true));
    let (tx, rx) = mpsc::channel(config.queue_capacity.max(1));
    let sinks = Sinks {
        store,
        syslog,
        #[cfg(feature = "fhir")]
        feed_spool,
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
            emergency,
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
            deliveries,
            chain_check,
        },
    ))
}

/// What starting the feed hands the drain: the notify that wakes the store's
/// forwarder, or the store-off spool's forwarder.
#[cfg(feature = "fhir")]
type FeedStart = (Option<Arc<Notify>>, Option<Arc<Forwarder<Spool>>>);

/// Starts the ITI-20 ATX:FHIR Feed when it is on: a forwarder delivering
/// from the store when the store is on, returning the notify that wakes it,
/// else a forwarder delivering from an in-memory spool the drain fills.
///
/// # Errors
/// [`AuditError::Transport`] when the feed repository cannot be built.
#[cfg(feature = "fhir")]
fn start_feed(
    config: &AuditConfig,
    store: Option<&AuditStore>,
    deliveries: &mut Vec<Delivery>,
) -> Result<FeedStart, AuditError> {
    use crate::system_log::feed::{StoreOutbox, repository};
    if !config.fhir_feed.enabled {
        return Ok((None, None));
    }
    let feed = repository(&config.fhir_feed)?;
    let retry_max = Duration::from_millis(config.fhir_feed.retry_max_ms);
    let Some(store) = store else {
        // NOTE: no specification governs the bounds of the store-off feed's
        // spool: our own design, about a day of records at a modest rate.
        let spool = Spool::in_memory(ihe_atna::atna::spool::Bounds {
            max_messages: 100_000,
            max_bytes: 256 * 1024 * 1024,
            write_timeout: Duration::from_secs(2),
        });
        let forwarder = Forwarder::fhir_feed(spool, feed, retry_max);
        deliveries.push(deliver("fhir_feed", &forwarder));
        return Ok((None, Some(forwarder)));
    };
    let notify = Arc::new(Notify::new());
    let outbox = StoreOutbox::new(
        store.clone(),
        Arc::clone(&notify),
        Duration::from_millis(config.fhir_feed.poll_interval_ms.max(100)),
    );
    deliveries.push(deliver(
        "fhir_feed",
        &Forwarder::fhir_feed_from(outbox, feed, retry_max),
    ));
    Ok((Some(notify), None))
}

/// The purposes of use `[audit] emergency_purpose_codes` declares, held by
/// `ehds-logging`.
///
/// # Errors
/// [`AuditError::Emergency`] for an empty code; boot validation refuses one
/// first.
fn emergency_purposes(
    config: &AuditConfig,
) -> Result<ehds_logging::emergency::EmergencyPurposes, AuditError> {
    let purposes: Vec<ehds_logging::record::Purpose> = config
        .emergency_purpose_codes
        .iter()
        .map(|code| ehds_logging::record::Purpose {
            system: None,
            code: code.clone(),
        })
        .collect();
    ehds_logging::emergency::EmergencyPurposes::declare(&purposes).map_err(AuditError::Emergency)
}

/// The policies the EHDS access record is written under: the store's horizon
/// as the record's retention, the categories' and origins' labels, and the
/// emergency purposes.
///
/// The record states the horizon as configured: years, days, or without end
/// for `retention_days = 0`. A horizon under the three years of EHDS Art. 9(2)
/// passes the boot validation only where no Union jurisdiction is in force,
/// so its record names the deployment's own retention as the ground of its
/// floor, never Art. 9(2).
///
/// # Errors
/// [`AuditError::Policy`] when `ehds-logging` refuses the retention or the
/// label policy, which a validated configuration does not cause.
#[cfg(feature = "fhir")]
fn recording(
    config: &AuditConfig,
    categories: &crate::system_log::categories::Classifier,
    emergency: &ehds_logging::emergency::EmergencyPurposes,
) -> Result<crate::system_log::fhir::Recording, AuditError> {
    use ehds_logging::retention::{Bounds, FLOOR_YEARS, Period, RetentionPolicy};
    let none = std::collections::BTreeMap::new();
    let period = match config.store.retention() {
        Retention::Forever => Period::Indefinite,
        Retention::Days(days) => Period::Days(days),
        Retention::Years(years) => Period::Years(u16::try_from(years).unwrap_or(u16::MAX)),
    };
    let bounds = if matches!(period, Period::Years(years) if years < FLOOR_YEARS)
        || matches!(period, Period::Days(days) if days < 1096)
    {
        // NOTE: EHDS Art. 9(2) binds the Member States; the boot validation refuses a horizon
        // under a Union floor, so one under three years runs under no Union jurisdiction.
        Bounds::outside_union(
            period,
            format!(
                "the audit.store horizon ({}), under no Union jurisdiction",
                config.store.retention().describe()
            ),
        )
        .map_err(|e| AuditError::Policy(Box::new(e)))?
    } else {
        Bounds::union()
    };
    let retention =
        RetentionPolicy::declare_within(&bounds, period, &none, &none, categories.map())
            .map_err(|e| AuditError::Policy(Box::new(e)))?;
    let unlabelled = std::collections::BTreeMap::new();
    let labels =
        ehds_logging::label::LabelPolicy::declare(None, &unlabelled, &unlabelled, categories.map())
            .map_err(|e| AuditError::Policy(Box::new(e)))?;
    Ok(crate::system_log::fhir::Recording {
        retention,
        labels,
        emergency: emergency.clone(),
    })
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
    sinks: Sinks,
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

        let unavailable = match (&resolver, resolve_subject) {
            (Some(resolve), true) => memoize_subjects(&batch, resolve, &subject_cache).await,
            _ => BTreeSet::new(),
        };
        let records = render_batch(
            &mut batch,
            &ctx,
            &subject_cache,
            resolve_subject.then_some(&unavailable),
            &sinks,
        )
        .await;

        // The store is the durability anchor and is written first.
        write_to_store(&sinks, &records, &store_healthy).await;
        if let Some(route) = &sinks.syslog {
            submit_to_syslog(route, &records, &ctx).await;
        }
        #[cfg(feature = "fhir")]
        if let Some(forwarder) = &sinks.feed_spool {
            submit_to_feed_spool(forwarder, &records).await;
        }
    }
    tracing::debug!("ATNA audit drain: channel closed, task exiting");
}

/// Resolves the subject id of every EHR the batch references that the memo
/// does not already hold, concurrently, and memoizes each answer.
///
/// A failed lookup is not memoized, so the next batch asks again; the EHRs
/// whose lookup failed are returned for this batch's records.
async fn memoize_subjects(
    batch: &[AuditEvent],
    resolve: &SubjectResolver,
    cache: &SubjectCache,
) -> BTreeSet<String> {
    let mut pending: Vec<String> = batch
        .iter()
        .filter_map(|event| event.ehr_id.clone())
        .filter(|ehr_id| !cache.contains_key(ehr_id))
        .collect();
    pending.sort_unstable();
    pending.dedup();
    let mut unavailable: BTreeSet<String> = pending.iter().cloned().collect();
    let mut lookups = tokio::task::JoinSet::new();
    for ehr_id in pending {
        let lookup = resolve(ehr_id.clone());
        lookups.spawn(async move { (ehr_id, lookup.await) });
    }
    // A lookup task that panicked never answers, so its EHR stays in `unavailable`.
    while let Some(joined) = lookups.join_next().await {
        let Ok((ehr_id, lookup)) = joined else {
            continue;
        };
        let subject = match lookup {
            SubjectLookup::Found(subject) => Some(subject),
            SubjectLookup::NoSubject => None,
            SubjectLookup::Unavailable => continue,
        };
        unavailable.remove(&ehr_id);
        cache.insert(ehr_id, subject).await;
    }
    unavailable
}

/// Drains the batch into records, attaching each event's memoized subject (or
/// marking its lookup failed when its EHR is in `unavailable`) and, when any
/// FHIR consumer is enabled, its rendered `AuditEvent` document. `unavailable`
/// is `None` when subjects are not resolved at all.
///
/// The document is rendered once here because both the store and the FHIR feed
/// consume it; with neither sink enabled nothing is rendered at all.
async fn render_batch(
    batch: &mut Vec<AuditEvent>,
    ctx: &AuditContext,
    subject_cache: &SubjectCache,
    unavailable: Option<&BTreeSet<String>>,
    sinks: &Sinks,
) -> Vec<DrainRecord> {
    #[cfg(feature = "fhir")]
    let render_fhir = sinks.store.is_some() || sinks.feed_spool.is_some();
    #[cfg(not(feature = "fhir"))]
    let render_fhir = sinks.store.is_some();
    let mut records: Vec<DrainRecord> = Vec::with_capacity(batch.len());
    for event in batch.drain(..) {
        let mut event = event;
        let resolved = match (unavailable, &event.ehr_id) {
            (Some(failed), Some(ehr_id)) if failed.contains(ehr_id) => {
                event.subject_lookup_failed = true;
                None
            }
            (Some(_), Some(ehr_id)) => subject_cache.get(ehr_id).await.flatten(),
            _ => None,
        };
        event
            .subject_namespace
            .clone_from(&resolved.as_ref().and_then(|s| s.namespace.clone()));
        let subject = resolved.map(|s| s.id);
        let rendered = render_fhir
            .then(|| render_audit_event(&event, ctx, subject.as_deref()))
            .flatten();
        records.push((event, subject, rendered));
    }
    records
}

/// Writes the batch to the local Audit Record Repository in one multi-row
/// INSERT.
async fn write_to_store(sinks: &Sinks, records: &[DrainRecord], store_healthy: &AtomicBool) {
    if let Some(store) = &sinks.store {
        store_whole_batch(store, records, store_healthy, sinks.feed_notify.as_ref()).await;
    }
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

/// Stores every record's DICOM PS3.15 §A.5 XML frame in the syslog spool,
/// which the forwarder delivers.
async fn submit_to_syslog(route: &SyslogRoute, records: &[DrainRecord], ctx: &AuditContext) {
    for (event, subject, _) in records {
        let xml = match crate::system_log::message::to_xml(event, ctx, subject.as_deref()) {
            Ok(xml) => xml,
            Err(e) => {
                crate::telemetry::metrics::metrics()
                    .atna_audit_serialize_failed
                    .add(1, &[]);
                tracing::warn!("ATNA audit message serialization failed: {e}");
                continue;
            }
        };
        if let Err(e) = route.submit(event.timestamp, &xml).await {
            crate::telemetry::metrics::metrics()
                .atna_audit_dropped
                .add(1, &[opentelemetry::KeyValue::new("sink", "syslog")]);
            tracing::warn!(
                "ATNA audit syslog spool refused a record, which syslog never receives: {e}"
            );
        }
    }
}

/// Stores every rendered document in the store-off FHIR feed's spool, which
/// the forwarder delivers.
#[cfg(feature = "fhir")]
async fn submit_to_feed_spool(forwarder: &Forwarder<Spool>, records: &[DrainRecord]) {
    for (_, _, rendered) in records {
        let Some(document) = rendered else {
            continue;
        };
        let bytes = match serde_json::to_vec(document) {
            Ok(bytes) => bytes,
            Err(e) => {
                crate::telemetry::metrics::metrics()
                    .atna_audit_serialize_failed
                    .add(1, &[]);
                tracing::warn!("ATNA audit FHIR document serialization failed: {e}");
                continue;
            }
        };
        if let Err(e) = forwarder.submit(secrecy::SecretSlice::from(bytes)).await {
            crate::telemetry::metrics::metrics()
                .atna_audit_dropped
                .add(1, &[opentelemetry::KeyValue::new("sink", "fhir_feed")]);
            tracing::warn!("ATNA audit FHIR feed spool refused a record: {e}");
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

    fn syslog_config() -> AuditConfig {
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
                // A port nothing listens on is fine: the sink connects on the
                // first send, which fails and is metered.
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

    /// A failed lookup is reported for its EHR and never memoized, so the next
    /// batch asks again; a subject or its absence is memoized.
    #[tokio::test]
    async fn a_failed_subject_lookup_is_reported_and_not_memoized() {
        let batch: Vec<AuditEvent> = ["failing", "empty", "named"]
            .into_iter()
            .map(|ehr_id| {
                let mut e = event();
                e.ehr_id = Some(ehr_id.to_owned());
                e
            })
            .collect();
        let resolve: SubjectResolver = Arc::new(|ehr_id: String| {
            Box::pin(async move {
                match ehr_id.as_str() {
                    "failing" => SubjectLookup::Unavailable,
                    "empty" => SubjectLookup::NoSubject,
                    _ => SubjectLookup::Found(ResolvedSubject {
                        id: "patient-1".to_owned(),
                        namespace: None,
                    }),
                }
            })
        });
        let cache: SubjectCache = moka::future::Cache::builder().max_capacity(8).build();
        let unavailable = memoize_subjects(&batch, &resolve, &cache).await;
        assert_eq!(unavailable, BTreeSet::from(["failing".to_owned()]));
        assert!(
            !cache.contains_key("failing"),
            "a failure is never memoized"
        );
        assert_eq!(cache.get("empty").await, Some(None));
        assert_eq!(
            cache.get("named").await.flatten().map(|subject| subject.id),
            Some("patient-1".to_owned())
        );
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
                emergency: ehds_logging::emergency::EmergencyPurposes::declare(&[
                    ehds_logging::record::Purpose {
                        system: None,
                        code: "ETREAT".to_owned(),
                    },
                ])
                .expect("one declared purpose"),
                legal_basis: None,
                categories: crate::system_log::categories::Classifier::default(),
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
        let (sender, handle) = start(syslog_config(), None, None).await.expect("start");
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

    /// Only a declared code on `emergency_purpose_codes` marks an access; an
    /// absent purpose and any other code never do (EHDS Art. 11(5)).
    #[tokio::test]
    async fn only_a_configured_emergency_code_marks_an_access() {
        let sender = bare_sender(FailMode::Open, 8, true);
        assert!(sender.is_emergency_purpose(Some("ETREAT")));
        assert!(!sender.is_emergency_purpose(Some("TREAT")));
        assert!(!sender.is_emergency_purpose(Some("etreat")));
        assert!(!sender.is_emergency_purpose(None));
    }

    #[tokio::test]
    async fn fail_open_ignores_store_health() {
        let sender = bare_sender(FailMode::Open, 8, false);
        assert_eq!(sender.emit(event()), EmitOutcome::Enqueued);
    }
}
