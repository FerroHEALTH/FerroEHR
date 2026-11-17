// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit sender: each record written durably before the response that
//! reports its access, and the background tasks that chain and deliver the
//! stored records (#5747).
//!
//! The request path [`AuditSender::record`]s every record of a request in
//! one transaction of the local Audit Record Repository: the subject of the
//! record's EHR is resolved (memoized), the record is classified, the
//! access-log entry of an access to EHR data is built, and all of it goes
//! into the intake (`crate::system_log::store`). A record that cannot be
//! written is an error the caller answers with `503`, so no access is
//! reported without its record and nothing waits in memory.
//!
//! Behind the request path:
//!
//! - **the chainer** (`sender::chainer`) links the intake into the chains, any
//!   replica under the head's lock, and owes each chained record to the
//!   sinks that deliver from the store;
//! - **syslog** (`[audit.syslog]`, ITI-20) and the **FHIR Feed**
//!   (`[audit.fhir_feed]`, ITI-20 ATX: FHIR Feed) are projections of the
//!   stored records, delivered by `ihe-atna`'s forwarders from the store's
//!   outboxes, so a down repository loses nothing.
//!
//! No openEHR spec governs the audit trail's delivery: our own design.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ihe_atna::atna::forwarder::Status;
#[cfg(feature = "fhir")]
use ihe_atna::atna::forwarder::{Forwarder, Outbox};
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::event::AuditEvent;

use crate::system_log::AuditError;
use crate::system_log::categories::CategoryMapConfig;
use crate::system_log::chain_check::ChainCheck;
use crate::system_log::config::{AuditConfig, StoreConfig};
use crate::system_log::render::{CompiledPolicy, RenderPolicy};
use crate::system_log::store::AuditStore;

mod background;
mod categories;
mod chainer;
mod live;
pub mod reconciler;
mod records;

use background::Background;
use categories::Classifiers;
use chainer::Chainer;
use live::LiveContext;
use records::{Subjects, render_records};

/// The patient subject of an EHR: `EHR_STATUS.subject.external_ref`'s id and
/// namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSubject {
    /// The subject's id value (`ehr.subject_id`).
    pub id: String,
    /// The namespace that issued it (`ehr.subject_namespace`), when stored.
    pub namespace: Option<String>,
}

impl crate::memory::Weigh for ResolvedSubject {
    fn heap(&self) -> usize {
        self.id.heap().saturating_add(self.namespace.heap())
    }
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

/// An indexed lookup of the EHR's patient subject.
///
/// Reads `ehr.subject_id` and `ehr.subject_namespace`. Supplied by the binary;
/// given an `ehr_id`, says whether it found a subject, found none, or could
/// not ask. Its answers are memoized, so a request seldom asks.
pub type SubjectResolver =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = SubjectLookup> + Send>> + Send + Sync>;

/// The cheaply-cloneable handle the REST layer records through.
#[derive(Debug, Clone)]
pub struct AuditSender {
    inner: Arc<SenderInner>,
}

struct SenderInner {
    enabled: bool,
    suppress_login_events: bool,
    /// The access-logging declaration surface: the header a caller declares a
    /// purpose of use in, the codes this deployment accepts (empty accepts
    /// any), and the legal basis it processes under.
    purpose_header: String,
    /// The header a caller declares the data permit it acts under in.
    permit_header: String,
    purpose_codes: Vec<String>,
    /// The purposes of use that mark an access as an emergency access, held
    /// by `ehds-logging`.
    emergency: ehds_logging::emergency::EmergencyPurposes,
    legal_basis: Option<String>,
    /// The compiled category register every record is classified through.
    categories: Arc<Classifiers>,
    /// The render policy and context records are written under.
    context: Arc<LiveContext>,
    /// The local Audit Record Repository every record is written to.
    store: AuditStore,
    /// The subject lookup, when subjects are resolved.
    resolver: Option<SubjectResolver>,
    /// The `ehr_id` to subject memo.
    subjects: SubjectCache,
    /// Wakes the chainer once a request wrote its records.
    chain: Arc<Notify>,
}

impl std::fmt::Debug for SenderInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SenderInner")
            .field("enabled", &self.enabled)
            .field("store", &self.store)
            .field("resolves_subjects", &self.resolver.is_some())
            .finish_non_exhaustive()
    }
}

impl AuditSender {
    /// How many EHRs' subjects the memo holds, and their bytes (#5926).
    #[must_use]
    pub fn subjects_held(&self) -> crate::memory::Held {
        crate::memory::held(&self.inner.subjects)
    }

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

    /// The request header a caller declares its data permit in, lowercased.
    #[must_use]
    pub fn permit_header(&self) -> &str {
        &self.inner.permit_header
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

    /// The local Audit Record Repository the records are written to.
    #[must_use]
    pub fn store(&self) -> &AuditStore {
        &self.inner.store
    }

    /// Hand `reservations`, whose writes committed, to the chainer and wake
    /// it: its next round releases their records inside its own transaction
    /// (#5762).
    pub fn release_committed(&self, reservations: &[uuid::Uuid]) {
        if reservations.is_empty() {
            return;
        }
        self.inner.store.queue_release(reservations);
        self.inner.chain.notify_one();
    }

    /// Reads on `conn`, the open transaction of the write `events` record,
    /// the subject of each EHR they name that the memo does not hold, and
    /// memoizes it, so [`Self::record`] looks none up: a lookup of its own
    /// would ask the clinical pool for a second connection while the write
    /// holds one, which a burst of writes waits out to the request timeout
    /// (#5947), and could not see an EHR the write itself creates. Without a
    /// resolver ([`AuditConfig::resolve_subject`] off) it reads nothing. No
    /// specification governs this: our own design.
    ///
    /// # Errors
    /// The database error of the read, which leaves `conn`'s transaction
    /// failed.
    pub async fn learn_subjects(
        &self,
        conn: &mut sqlx::PgConnection,
        events: &[AuditEvent],
    ) -> Result<(), sqlx::Error> {
        if self.inner.resolver.is_none() {
            return Ok(());
        }
        let mut named: Vec<(&str, uuid::Uuid)> = Vec::new();
        for ehr_id in events.iter().filter_map(|event| event.ehr_id.as_deref()) {
            // NOTE: an id that is no UUID names no stored EHR, so the parse failure is the answer.
            let Ok(id) = uuid::Uuid::parse_str(ehr_id) else {
                continue;
            };
            if !self.inner.subjects.contains_key(ehr_id) && !named.iter().any(|(n, _)| *n == ehr_id)
            {
                named.push((ehr_id, id));
            }
        }
        if named.is_empty() {
            return Ok(());
        }
        let ids: Vec<uuid::Uuid> = named.iter().map(|(_, id)| *id).collect();
        let rows: Vec<(uuid::Uuid, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT id, subject_id, subject_namespace FROM ehr WHERE id = ANY($1)")
                .bind(&ids)
                .fetch_all(conn)
                .await?;
        for (ehr_id, id) in named {
            let subject =
                rows.iter()
                    .find(|(row, _, _)| *row == id)
                    .and_then(|(_, subject, namespace)| {
                        subject.clone().map(|id| ResolvedSubject {
                            id,
                            namespace: namespace.clone(),
                        })
                    });
            self.inner.subjects.insert(ehr_id.to_owned(), subject).await;
        }
        Ok(())
    }

    /// Write `events`, the records of one act, into the local store in one
    /// transaction; they are durable when this returns, and the chainer links
    /// them after, unless `reservation` holds them until the write they
    /// record is released ([`Self::release_committed`], #5762).
    ///
    /// Every record is stamped with the digest of the category map in force,
    /// with the emergency mark when a purpose of use it names is one of
    /// `[audit] emergency_purpose_codes`, and with the subject of its EHR.
    ///
    /// # Errors
    /// The errors of [`AuditStore::write`]: the caller refuses the access it
    /// was to report (`503`), since no access is reported without its
    /// record.
    pub async fn record(
        &self,
        events: Vec<AuditEvent>,
        reservation: Option<uuid::Uuid>,
    ) -> Result<(), AuditError> {
        let Some((records, ends, count)) = self.rendered(events).await else {
            return Ok(());
        };
        let store = &self.inner.store;
        match reservation {
            Some(reservation) => {
                self.wrote(count, store.write_held(records, &ends, reservation))
                    .await
            }
            None => self.wrote(count, store.write(records, &ends)).await,
        }
    }

    /// [`Self::record`] held under `reservation`, written on `conn`, a
    /// connection of the audit pool the write took before its own (#5964).
    ///
    /// # Errors
    /// As [`Self::record`].
    pub(crate) async fn record_on(
        &self,
        conn: &mut sqlx::PgConnection,
        events: Vec<AuditEvent>,
        reservation: uuid::Uuid,
    ) -> Result<(), AuditError> {
        let Some((records, ends, count)) = self.rendered(events).await else {
            return Ok(());
        };
        let write = self
            .inner
            .store
            .write_held_on(conn, records, &ends, reservation);
        self.wrote(count, write).await
    }

    /// `events` stamped and rendered as the store writes them, with their
    /// retention ends and their count; `None` when there are none.
    async fn rendered(
        &self,
        mut events: Vec<AuditEvent>,
    ) -> Option<(
        Vec<crate::system_log::store::AuditRecord>,
        Vec<crate::system_log::store::RecordEnd>,
        u64,
    )> {
        if events.is_empty() {
            return None;
        }
        let count = u64::try_from(events.len()).unwrap_or(u64::MAX);
        let digest = self.categories().digest();
        for event in &mut events {
            event.category_map_digest = Some(digest);
            event.emergency_access = event.names_purpose(|c| self.is_emergency_purpose(Some(c)));
        }
        let ctx = self.inner.context.current();
        let subjects = match &self.inner.resolver {
            Some(resolve) => Some(memoize_subjects(&events, resolve, &self.inner.subjects).await),
            None => None,
        };
        let records = render_records(events, &ctx, subjects.as_ref());
        let ends = crate::system_log::retention::record_ends(&records, &ctx);
        Some((records, ends, count))
    }

    /// Runs `write`, the write of `count` records, metering it, and wakes the
    /// chainer once it succeeded.
    async fn wrote(
        &self,
        count: u64,
        write: impl Future<Output = Result<(), AuditError>>,
    ) -> Result<(), AuditError> {
        let started = std::time::Instant::now();
        let written = write.await;
        let path = crate::telemetry::access_path::metrics();
        let outcome = crate::telemetry::access_path::outcome(&written);
        path.log_append_duration.record(
            crate::telemetry::access_path::seconds_since(started),
            std::slice::from_ref(&outcome),
        );
        path.log_entries.add(count, &[outcome]);
        let metrics = crate::telemetry::metrics::metrics();
        match written {
            Ok(()) => {
                metrics.atna_audit_emitted.add(count, &[]);
                self.inner.chain.notify_one();
                Ok(())
            }
            Err(error) => {
                metrics.atna_audit_rejected.add(count, &[]);
                tracing::error!(
                    records = count,
                    error = %crate::error::ErrorChain::new(&error),
                    "the audit records could not be written; the access is refused"
                );
                Err(error)
            }
        }
    }
}

/// Owns the background tasks; the binary keeps it and stops it on shutdown.
#[derive(Debug)]
pub struct AuditHandle {
    /// The chainer, stopped first so its last round finishes.
    chainer: Chainer,
    /// The retention workers (reaper, chain verification), aborted on
    /// shutdown.
    workers: Vec<JoinHandle<()>>,
    /// The forwarders, stopped after the chainer so their connections close
    /// cleanly; what they owe is delivered by the next run.
    deliveries: Vec<Delivery>,
    /// The runtime the chainer, the forwarders and the workers run on.
    background: Background,
    /// The store the background work writes through, over the audit
    /// runtime's pool.
    behind: AuditStore,
    /// The result of the scheduled hash-chain verification, when it runs.
    chain_check: Option<ChainCheck>,
}

impl AuditHandle {
    /// Returns the audit runtime's own pool, for the pool gauges.
    #[must_use]
    pub fn runtime_pool(&self) -> PgPool {
        self.behind.pool.clone()
    }

    /// Returns the shared result of the scheduled hash-chain verification;
    /// `None` when `verify_interval` is `0`.
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

    /// Run the reconciler of held audit records over the clinical and party
    /// domains of `pools` on the audit runtime (#5762): it resolves the
    /// reservations of writes whose process stopped between their commit and
    /// their release.
    pub fn reconcile_reservations(&mut self, pools: &crate::db::domain::DomainPools) {
        // NOTE: our own design (#5845), the domain pools inside the connection budget serve the
        // reconciler's few statements, so it opens no pool of its own.
        let domains = reconciler::Domains {
            clinical: pools.clinical.clone(),
            party: pools.party.clone(),
        };
        self.workers.push(
            self.background
                .spawn(reconciler::run(self.behind.clone(), domains)),
        );
    }

    /// Stop the chainer, bounded by `timeout`, then the workers and the
    /// forwarders. A record the chainer did not link waits in the intake for
    /// the next chainer, of this replica or another.
    pub async fn shutdown(self, timeout: Duration) {
        self.chainer.stop(timeout).await;
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
        self.background.stop();
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

/// Spawns `forwarder` onto the audit runtime as the delivery of `sink`, with
/// the sampler that moves its delivered and retried counts onto the metrics.
#[cfg(feature = "fhir")]
fn deliver<O: Outbox + 'static>(
    background: &Background,
    sink: &'static str,
    forwarder: &Arc<Forwarder<O>>,
) -> Delivery {
    let join = background.spawn(Arc::clone(forwarder).run());
    let reader = Arc::clone(forwarder);
    let stopper = Arc::clone(forwarder);
    let sampled = Arc::clone(forwarder);
    drop(background.spawn(sample(sink, move || sampled.status())));
    Delivery {
        sink,
        status: Arc::new(move || reader.status()),
        stop: Arc::new(move || stopper.stop()),
        join,
    }
}

/// How often a forwarder's counts move onto the metrics.
#[cfg(feature = "fhir")]
const SAMPLE_INTERVAL: Duration = Duration::from_secs(15);

/// Moves the delivered and retried counts of `status` onto
/// `atna_audit_sent` and `atna_audit_send_failed` as they grow, until the
/// runtime ends the task.
#[cfg(feature = "fhir")]
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

/// The most EHRs whose subject the audit sender memoizes.
pub const SUBJECTS: u64 = 100_000;

/// The `ehr_id` → subject memo (an EHR's subject is immutable for audit
/// purposes, so one lookup serves every later record).
type SubjectCache = moka::future::Cache<String, Option<ResolvedSubject>>;

/// Start the audit subsystem over the local store on `pool`: the sender, the
/// chainer, the forwarders of the configured sinks and the store's upkeep.
///
/// # Errors
/// [`AuditError`] if a sink's transport cannot be established (a TLS
/// config or CA problem, a refused feed endpoint), or the render policy does
/// not compile.
#[cfg_attr(
    not(feature = "fhir"),
    expect(
        clippy::unused_async,
        reason = "a slim build starts no forwarder, which is what the full build awaits"
    )
)]
pub async fn start(
    config: AuditConfig,
    resolver: Option<SubjectResolver>,
    pool: PgPool,
) -> Result<(AuditSender, AuditHandle), AuditError> {
    let policy = RenderPolicy::of(&config, &CategoryMapConfig::default());
    let background = Background::start()?;
    let behind = background.pool_over(&pool);
    let store = AuditStore::new(pool)
        .with_fhir_feed(config.fhir_feed.is_some())
        .with_syslog(config.syslog.is_some())
        .with_render_policy(policy.clone());
    // NOTE: our own design (#5770), the background work runs on the audit runtime over its own pool.
    let behind = store.with_pool(behind);
    #[cfg_attr(
        not(feature = "fhir"),
        expect(unused_mut, reason = "a slim build has no sink")
    )]
    let mut deliveries = Vec::new();
    #[cfg_attr(
        not(feature = "fhir"),
        expect(unused_mut, reason = "a slim build has no sink")
    )]
    let mut woken = Vec::new();
    #[cfg(feature = "fhir")]
    if let Some(syslog) = &config.syslog {
        let notify = Arc::new(Notify::new());
        let forwarder = crate::system_log::syslog::forwarder(
            syslog,
            config.server_host.as_deref().unwrap_or_default(),
            &config.source_id,
            behind.clone(),
            Arc::clone(&notify),
        )
        .await?;
        deliveries.push(deliver(&background, "syslog", &forwarder));
        woken.push(notify);
    }
    #[cfg(feature = "fhir")]
    if let Some(fhir_feed) = &config.fhir_feed {
        use crate::system_log::feed::{StoreOutbox, repository};
        use crate::system_log::store::outbox::Sink;
        let arr = repository(fhir_feed)?;
        let notify = Arc::new(Notify::new());
        let idle = Duration::max(fhir_feed.poll_interval.get(), Duration::from_millis(100));
        let outbox = StoreOutbox::new(behind.clone(), Sink::FhirFeed, Arc::clone(&notify), idle);
        deliveries.push(deliver(
            &background,
            "fhir_feed",
            &Forwarder::fhir_feed_from(outbox, arr),
        ));
        woken.push(notify);
    }

    let CompiledPolicy {
        context: ctx,
        categories,
        emergency,
    } = policy.compile()?;
    let live = Arc::new(LiveContext::new(policy, ctx));
    let mut workers = Vec::new();
    let chain_check = spawn_store_upkeep(&background, &behind, &config.store, &live, &mut workers);
    let chain = Arc::new(Notify::new());
    let chainer = Chainer::spawn(&background, behind.clone(), Arc::clone(&chain), woken);

    let sender = AuditSender {
        inner: Arc::new(SenderInner {
            enabled: config.enabled,
            suppress_login_events: config.suppress_login_events,
            purpose_header: config.purpose_header.to_ascii_lowercase(),
            permit_header: config.permit_header.to_ascii_lowercase(),
            purpose_codes: config.purpose_codes.clone(),
            emergency,
            legal_basis: config.legal_basis.clone(),
            categories: Arc::new(Classifiers::new(categories)),
            context: live,
            store,
            resolver: resolver.filter(|_| config.resolve_subject),
            subjects: moka::future::Cache::builder()
                .max_capacity(SUBJECTS)
                .time_to_live(Duration::from_hours(1))
                .build(),
            chain,
        }),
    };
    Ok((
        sender,
        AuditHandle {
            chainer,
            workers,
            deliveries,
            background,
            behind,
            chain_check,
        },
    ))
}

/// Spawns the local store's upkeep workers onto `workers`: the retention
/// reaper, which deletes each record at its own end, and the scheduled hash-chain
/// verification unless `verify_interval` is `"0s"`, whose shared result is
/// returned.
fn spawn_store_upkeep(
    background: &Background,
    store: &AuditStore,
    config: &StoreConfig,
    live: &Arc<LiveContext>,
    workers: &mut Vec<JoinHandle<()>>,
) -> Option<ChainCheck> {
    workers.push(background.spawn(live::reaper(store.clone(), Arc::clone(live))));
    if config.verify_interval.is_zero() {
        return None;
    }
    let check =
        ChainCheck::new((!config.max_verify_lag.is_zero()).then(|| config.max_verify_lag.get()));
    workers.push(background.spawn(crate::system_log::chain_check::scheduled(
        store.clone(),
        check.clone(),
        config.verify_interval.get(),
        (!config.full_verify_interval.is_zero()).then(|| config.full_verify_interval.get()),
    )));
    Some(check)
}

/// The subject a read of the request found for `ehr_id` in the store, as
/// the resolver would name it: `ehr.subject_id` under `ehr.subject_namespace`.
fn judged_subject(events: &[AuditEvent], ehr_id: &str) -> Option<ResolvedSubject> {
    events
        .iter()
        .filter(|event| event.ehr_id.as_deref() == Some(ehr_id))
        .find_map(|event| event.judged_subject.as_ref())
        .map(|subject| ResolvedSubject {
            id: subject.id.clone(),
            namespace: Some(subject.namespace.clone()),
        })
}

/// The subject of every EHR the records reference: the one a read of the
/// request found, from the memo, or resolved concurrently and memoized.
///
/// A failed lookup is not memoized, so the next request asks again, and its
/// EHR is returned among the failed for these records.
async fn memoize_subjects(
    events: &[AuditEvent],
    resolve: &SubjectResolver,
    cache: &SubjectCache,
) -> Subjects {
    let mut referenced: Vec<String> = events
        .iter()
        .filter_map(|event| event.ehr_id.clone())
        .collect();
    referenced.sort_unstable();
    referenced.dedup();
    let mut subjects = Subjects::default();
    let mut pending = Vec::new();
    for ehr_id in referenced {
        if let Some(judged) = judged_subject(events, &ehr_id) {
            subjects.resolved.insert(ehr_id, Some(judged));
            continue;
        }
        match cache.get(&ehr_id).await {
            Some(held) => {
                subjects.resolved.insert(ehr_id, held);
            }
            None => pending.push(ehr_id),
        }
    }
    if pending.is_empty() {
        return subjects;
    }
    // NOTE: our own design (#5947), this task waits for the lookups, so it keeps no connection.
    crate::db::acquire::release_held();
    let mut lookups = tokio::task::JoinSet::new();
    for ehr_id in pending {
        subjects.unavailable.insert(ehr_id.clone());
        let lookup = resolve(ehr_id.clone());
        let cache = cache.clone();
        // NOTE: our own design (#5985), concurrent lookups of one EHR share one read, and a
        // lookup that could not ask the database is never memoized.
        lookups.spawn(crate::db::acquire::inheriting(async move {
            let found = cache
                .optionally_get_with(ehr_id.clone(), async move {
                    match lookup.await {
                        SubjectLookup::Found(subject) => Some(Some(subject)),
                        SubjectLookup::NoSubject => Some(None),
                        SubjectLookup::Unavailable => None,
                    }
                })
                .await;
            (ehr_id, found)
        }));
    }
    // A lookup task that panicked never answers, so its EHR stays unavailable.
    while let Some(joined) = lookups.join_next().await {
        let Ok((ehr_id, Some(subject))) = joined else {
            continue;
        };
        subjects.unavailable.remove(&ehr_id);
        subjects.resolved.insert(ehr_id, subject);
    }
    subjects
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::system_log::event::{EventActionCode, EventOutcome, ObjectClass};

    fn event() -> AuditEvent {
        AuditEvent::new(
            EventActionCode::Create,
            ObjectClass::Ehr,
            EventOutcome::Success,
        )
    }

    /// A failed lookup is reported for its EHR and never memoized, so the next
    /// request asks again; a subject or its absence is memoized.
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
        let subjects = memoize_subjects(&batch, &resolve, &cache).await;
        assert_eq!(subjects.unavailable, BTreeSet::from(["failing".to_owned()]));
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

    /// The memo holds at most its capacity, and a record names the subject
    /// its act read, whatever the cap evicted meanwhile (#5926).
    #[tokio::test]
    async fn the_memo_stays_under_its_cap_and_an_eviction_changes_no_record() {
        let cap = 4;
        let batch: Vec<AuditEvent> = (0..32)
            .map(|n| {
                let mut e = event();
                e.ehr_id = Some(format!("ehr-{n}"));
                e
            })
            .collect();
        let resolve: SubjectResolver = Arc::new(|ehr_id: String| {
            Box::pin(async move {
                SubjectLookup::Found(ResolvedSubject {
                    id: format!("patient-of-{ehr_id}"),
                    namespace: Some("demographic".to_owned()),
                })
            })
        });
        let cache: SubjectCache = moka::future::Cache::builder().max_capacity(cap).build();
        for round in 0..2 {
            let subjects = memoize_subjects(&batch, &resolve, &cache).await;
            cache.run_pending_tasks().await;
            let memo = crate::memory::held(&cache);
            assert!(memo.entries <= cap, "{memo:?} within {cap}");
            let records = render_records(
                batch.clone(),
                &crate::system_log::message::AuditContext::default(),
                Some(&subjects),
            );
            for record in records {
                let ehr_id = record.event.ehr_id.clone().expect("an EHR");
                assert_eq!(
                    record.subject,
                    Some(format!("patient-of-{ehr_id}")),
                    "round {round}: the record names its EHR's subject"
                );
                assert!(!record.event.subject_lookup_failed);
            }
        }
    }

    /// Concurrent records of one EHR the memo does not hold share one lookup
    /// of its subject, and each names it.
    #[tokio::test]
    async fn concurrent_lookups_of_one_ehr_share_one_read() {
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&asked);
        let resolve: SubjectResolver = Arc::new(move |_ehr_id: String| {
            let counted = Arc::clone(&counted);
            Box::pin(async move {
                counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(50)).await;
                SubjectLookup::Found(ResolvedSubject {
                    id: "patient-1".to_owned(),
                    namespace: Some("demographic".to_owned()),
                })
            })
        });
        let cache: SubjectCache = moka::future::Cache::builder().max_capacity(8).build();
        let mut e = event();
        e.ehr_id = Some("one".to_owned());
        let batch = vec![e];
        let answers =
            futures::future::join_all((0..8).map(|_| memoize_subjects(&batch, &resolve, &cache)))
                .await;
        assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 1);
        for subjects in answers {
            assert_eq!(
                subjects
                    .resolved
                    .get("one")
                    .cloned()
                    .flatten()
                    .map(|s| s.id),
                Some("patient-1".to_owned())
            );
        }
    }

    /// A record whose request read its EHR's subject in the store names that
    /// subject, and the resolver is not asked.
    #[tokio::test]
    async fn a_subject_the_request_read_is_named_without_a_lookup() {
        let resolve: SubjectResolver =
            Arc::new(|_ehr_id: String| Box::pin(async move { SubjectLookup::Unavailable }));
        let cache: SubjectCache = moka::future::Cache::builder().max_capacity(8).build();
        let mut e = event();
        e.ehr_id = Some("one".to_owned());
        e.judged_subject = Some(crate::storage::access_projection::EhrSubject::new(
            "demographic",
            "patient-2",
        ));
        let subjects = memoize_subjects(&[e], &resolve, &cache).await;
        assert!(subjects.unavailable.is_empty());
        let named = subjects.resolved.get("one").cloned().flatten();
        assert_eq!(
            named
                .as_ref()
                .map(|s| (s.id.as_str(), s.namespace.as_deref())),
            Some(("patient-2", Some("demographic")))
        );
    }
}
