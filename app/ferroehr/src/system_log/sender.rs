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

use std::collections::BTreeSet;
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
mod records;

use background::Background;
use categories::Classifiers;
use chainer::Chainer;
use live::LiveContext;
use records::render_records;

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

    /// Write `events`, the records of one act, into the local store in one
    /// transaction; they are durable when this returns, and the chainer links
    /// them after.
    ///
    /// Every record is stamped with the digest of the category map in force,
    /// with the emergency mark when a purpose of use it names is one of
    /// `[audit] emergency_purpose_codes`, and with the subject of its EHR.
    ///
    /// # Errors
    /// The errors of [`AuditStore::write`]: the caller refuses the access it
    /// was to report (`503`), since no access is reported without its
    /// record.
    pub async fn record(&self, events: Vec<AuditEvent>) -> Result<(), AuditError> {
        if events.is_empty() {
            return Ok(());
        }
        let count = u64::try_from(events.len()).unwrap_or(u64::MAX);
        let mut events = events;
        let digest = self.categories().digest();
        for event in &mut events {
            event.category_map_digest = Some(digest);
            event.emergency_access = event.names_purpose(|c| self.is_emergency_purpose(Some(c)));
        }
        let ctx = self.inner.context.current();
        let unavailable = match &self.inner.resolver {
            Some(resolve) => Some(memoize_subjects(&events, resolve, &self.inner.subjects).await),
            None => None,
        };
        let records =
            render_records(events, &ctx, &self.inner.subjects, unavailable.as_ref()).await;
        let ends = crate::system_log::retention::record_ends(&records, &ctx);
        let started = std::time::Instant::now();
        let written = self.inner.store.write(records, &ends).await;
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
    /// The result of the scheduled hash-chain verification, when it runs.
    chain_check: Option<ChainCheck>,
}

impl AuditHandle {
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
    let chainer = Chainer::spawn(&background, behind, Arc::clone(&chain), woken);

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
                .max_capacity(100_000)
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

/// Resolves the subject id of every EHR the records reference that the memo
/// does not already hold, concurrently, and memoizes each answer.
///
/// A failed lookup is not memoized, so the next request asks again; the EHRs
/// whose lookup failed are returned for these records.
async fn memoize_subjects(
    events: &[AuditEvent],
    resolve: &SubjectResolver,
    cache: &SubjectCache,
) -> BTreeSet<String> {
    let mut pending: Vec<String> = events
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

#[cfg(test)]
mod tests {
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
}
