// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The instruments of the access path: the policy copies and their
//! listeners, the access evaluator, the access log and the verification of
//! the audit chains.
//!
//! No openEHR spec governs metrics: our own design (#5591). The naming rules
//! are [`super::metrics`]'s: no Prometheus suffix, a declared unit. Every
//! label is drawn from a closed set written in this crate (a copy, a
//! listener, an outcome, a `refused-*` code, a chain), so no label carries a
//! patient, an EHR id, a party or clinical content, and none grows with the
//! data.

use std::sync::{Arc, OnceLock};

use opentelemetry::KeyValue;
use opentelemetry::metrics::{AsyncInstrument, Counter, Gauge, Histogram, Meter};

use crate::instance::{Agreement, Instances};
use crate::policy_register::listen::Listened;
use crate::service::FerroEhrService;

// ── Instrument names (no Prometheus suffix — the exporter adds it) ───────────

/// Lookups in the per-EHR access copy and the class copy (`copy`, `result`).
pub const ACCESS_COPY_LOOKUPS: &str = "access_copy_lookups";
/// Reads that refill the per-EHR access copy or the class copy (`copy`,
/// `outcome`).
pub const ACCESS_COPY_REFILLS: &str = "access_copy_refills";
/// Entries dropped from the per-EHR access copy or the class copy (`scope`).
pub const ACCESS_COPY_DROPS: &str = "access_copy_drops";
/// Entries the per-EHR access copy and the class copy hold (`copy`).
pub const ACCESS_COPY_ENTRIES: &str = "access_copy_entries";
/// Whether a policy copy's listener is connected, `1` or `0` (`listener`).
pub const POLICY_LISTENER_UP: &str = "policy_listener_up";
/// Listener connections lost (`listener`).
pub const POLICY_LISTENER_LOSSES: &str = "policy_listener_losses";
/// Notifications a listener received (`listener`).
pub const POLICY_NOTIFICATIONS: &str = "policy_notifications";
/// The time since a listener's newest notification (`listener`).
pub const POLICY_NOTIFICATION_AGE: &str = "policy_notification_age";
/// Reads of the store a policy copy made on a request (`listener`, `reason`).
pub const POLICY_COPY_READS: &str = "policy_copy_reads";
/// Refreshes of the grants and caller facts copy (`kind`, `outcome`).
pub const GRANTS_REFRESH_DURATION: &str = "grants_refresh_duration";
/// Access decisions, one per EHR a request reached (`outcome`, `code`).
pub const ACCESS_DECISIONS: &str = "access_decisions";
/// Directives and steps an emergency lifted (`lifted`).
pub const ACCESS_EMERGENCY_OVERRIDES: &str = "access_emergency_overrides";
/// Access-log entries appended (`outcome`).
pub const ACCESS_LOG_ENTRIES: &str = "access_log_entries";
/// Access-log append latency, one transaction's entries (`outcome`).
pub const ACCESS_LOG_APPEND_DURATION: &str = "access_log_append_duration";
/// Access-log link signing latency, one append's links (`outcome`).
pub const ACCESS_LOG_SIGN_DURATION: &str = "access_log_sign_duration";
/// Checkpoint and seal runs over a chain (`chain`, `outcome`).
pub const AUDIT_SEAL_DURATION: &str = "audit_seal_duration";
/// The Unix time of a chain's newest successful seal run (`chain`).
pub const AUDIT_SEAL_LAST_SUCCESS: &str = "audit_seal_last_success";
/// Access-log entries replaced by their tombstones.
pub const ACCESS_LOG_TOMBSTONES: &str = "access_log_tombstones";
/// Access-log retention runs (`outcome`).
pub const ACCESS_LOG_RETENTION_DURATION: &str = "access_log_retention_duration";
/// Scheduled verifications of the audit chains (`scope`, `result`).
pub const AUDIT_CHAIN_VERIFICATION_DURATION: &str = "audit_chain_verification_duration";
/// The Unix time of the newest finished chain verification.
pub const AUDIT_CHAIN_LAST_VERIFICATION: &str = "audit_chain_last_verification";
/// Whether the newest chain verification found the chains intact, `1` or `0`.
pub const AUDIT_CHAIN_INTACT: &str = "audit_chain_intact";
/// Records waiting in the audit drain's queue.
pub const AUDIT_DRAIN_QUEUE_DEPTH: &str = "audit_drain_queue_depth";
/// Live instances of the deployment that differ from this one, by what
/// differs (`differs`), at its last comparison (#5589).
pub const INSTANCE_MISMATCHES: &str = "instance_mismatches";

/// Every instrument name of this module, for the naming tests.
pub const NAMES: [&str; 24] = [
    ACCESS_COPY_LOOKUPS,
    ACCESS_COPY_REFILLS,
    ACCESS_COPY_DROPS,
    ACCESS_COPY_ENTRIES,
    POLICY_LISTENER_UP,
    POLICY_LISTENER_LOSSES,
    POLICY_NOTIFICATIONS,
    POLICY_NOTIFICATION_AGE,
    POLICY_COPY_READS,
    GRANTS_REFRESH_DURATION,
    ACCESS_DECISIONS,
    ACCESS_EMERGENCY_OVERRIDES,
    ACCESS_LOG_ENTRIES,
    ACCESS_LOG_APPEND_DURATION,
    ACCESS_LOG_SIGN_DURATION,
    AUDIT_SEAL_DURATION,
    AUDIT_SEAL_LAST_SUCCESS,
    ACCESS_LOG_TOMBSTONES,
    ACCESS_LOG_RETENTION_DURATION,
    AUDIT_CHAIN_VERIFICATION_DURATION,
    AUDIT_CHAIN_LAST_VERIFICATION,
    AUDIT_CHAIN_INTACT,
    AUDIT_DRAIN_QUEUE_DEPTH,
    INSTANCE_MISMATCHES,
];

// ── Bucket ladders ───────────────────────────────────────────────────────────

/// Work on a request's path (a refresh, an append, the signatures of one
/// append): 100 µs … 5 s.
pub const PATH_BUCKETS: &[f64] = &[
    0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0,
];
/// Scheduled work over a whole chain or the whole log (a seal, a retention
/// run, a verification): 1 ms … 1 h.
pub const SCHEDULED_BUCKETS: &[f64] = &[
    0.001, 0.01, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0, 900.0, 3600.0,
];

/// Every histogram of this module and its bucket ladder.
#[must_use]
pub fn histogram_views() -> Vec<(&'static str, &'static [f64])> {
    vec![
        (GRANTS_REFRESH_DURATION, PATH_BUCKETS),
        (ACCESS_LOG_APPEND_DURATION, PATH_BUCKETS),
        (ACCESS_LOG_SIGN_DURATION, PATH_BUCKETS),
        (AUDIT_SEAL_DURATION, SCHEDULED_BUCKETS),
        (ACCESS_LOG_RETENTION_DURATION, SCHEDULED_BUCKETS),
        (AUDIT_CHAIN_VERIFICATION_DURATION, SCHEDULED_BUCKETS),
    ]
}

/// The access path's instruments that record an event; the ones read at
/// collection time are registered by [`observe`].
#[derive(Debug)]
pub struct AccessPathMetrics {
    /// Lookups in the per-EHR access copy and the class copy.
    pub copy_lookups: Counter<u64>,
    /// Reads that refill the per-EHR access copy or the class copy.
    pub copy_refills: Counter<u64>,
    /// Entries dropped from the per-EHR access copy or the class copy.
    pub copy_drops: Counter<u64>,
    /// Refreshes of the grants and caller facts copy, seconds.
    pub grants_refresh_duration: Histogram<f64>,
    /// Access decisions by outcome and refusal code.
    pub decisions: Counter<u64>,
    /// Directives and steps an emergency lifted.
    pub emergency_overrides: Counter<u64>,
    /// Access-log entries appended.
    pub log_entries: Counter<u64>,
    /// Access-log append latency, seconds.
    pub log_append_duration: Histogram<f64>,
    /// Access-log link signing latency, seconds.
    pub log_sign_duration: Histogram<f64>,
    /// Checkpoint and seal runs, seconds.
    pub seal_duration: Histogram<f64>,
    /// The Unix time of each chain's newest successful seal run.
    pub seal_last_success: Gauge<f64>,
    /// Access-log entries replaced by their tombstones.
    pub log_tombstones: Counter<u64>,
    /// Access-log retention runs, seconds.
    pub log_retention_duration: Histogram<f64>,
    /// Chain verifications, seconds.
    pub chain_verification_duration: Histogram<f64>,
    /// The Unix time of the newest finished chain verification.
    pub chain_last_verification: Gauge<f64>,
    /// Whether the newest chain verification found the chains intact.
    pub chain_intact: Gauge<u64>,
}

impl AccessPathMetrics {
    /// Create every event instrument on `meter`.
    #[must_use]
    pub fn new(meter: &Meter) -> Self {
        let make = Make(meter);
        Self {
            copy_lookups: make.counter(
                ACCESS_COPY_LOOKUPS,
                "Lookups in the per-EHR access copy and the class copy \
                 (copy = ehr | classes, result = hit | miss)",
            ),
            copy_refills: make.counter(
                ACCESS_COPY_REFILLS,
                "Reads that refill the per-EHR access copy or the class copy \
                 (copy = ehr | classes, outcome = ok | failed)",
            ),
            copy_drops: make.counter(
                ACCESS_COPY_DROPS,
                "Drops from the per-EHR access copy and the class copy \
                 (scope = ehr | classes | every)",
            ),
            grants_refresh_duration: make.histogram(
                GRANTS_REFRESH_DURATION,
                "Refreshes of the grants and caller facts copy \
                 (kind = unchanged | grants | facts, outcome = ok | failed)",
                PATH_BUCKETS,
            ),
            decisions: make.counter(
                ACCESS_DECISIONS,
                "Access decisions, one per EHR a request reached \
                 (outcome = admitted | refused, code = the refused-* code or none)",
            ),
            emergency_overrides: make.counter(
                ACCESS_EMERGENCY_OVERRIDES,
                "Directives and steps an emergency lifted \
                 (lifted = opt_out | restriction | consent | policy)",
            ),
            log_entries: make.counter(
                ACCESS_LOG_ENTRIES,
                "Access-log entries appended (outcome = ok | failed)",
            ),
            log_append_duration: make.histogram(
                ACCESS_LOG_APPEND_DURATION,
                "Access-log append latency per transaction (outcome = ok | failed)",
                PATH_BUCKETS,
            ),
            log_sign_duration: make.histogram(
                ACCESS_LOG_SIGN_DURATION,
                "Access-log link signing latency per append (outcome = ok | failed)",
                PATH_BUCKETS,
            ),
            seal_duration: make.histogram(
                AUDIT_SEAL_DURATION,
                "Checkpoint and seal runs (chain = access_log | system, \
                 outcome = sealed | empty | failed)",
                SCHEDULED_BUCKETS,
            ),
            seal_last_success: make.timestamp(
                AUDIT_SEAL_LAST_SUCCESS,
                "Unix time of the newest successful seal run, by chain",
            ),
            log_tombstones: make.counter(
                ACCESS_LOG_TOMBSTONES,
                "Access-log entries replaced by their tombstones",
            ),
            log_retention_duration: make.histogram(
                ACCESS_LOG_RETENTION_DURATION,
                "Access-log retention runs (outcome = ok | failed)",
                SCHEDULED_BUCKETS,
            ),
            chain_verification_duration: make.histogram(
                AUDIT_CHAIN_VERIFICATION_DURATION,
                "Scheduled audit chain verifications (scope = full | since_checkpoint, \
                 result = intact | damaged | unverifiable)",
                SCHEDULED_BUCKETS,
            ),
            chain_last_verification: make.timestamp(
                AUDIT_CHAIN_LAST_VERIFICATION,
                "Unix time of the newest finished audit chain verification",
            ),
            chain_intact: meter
                .u64_gauge(AUDIT_CHAIN_INTACT)
                .with_description(
                    "Whether the newest audit chain verification found the chains intact (1) \
                     or damaged or unverifiable (0)",
                )
                .build(),
        }
    }
}

/// The instrument shapes of [`AccessPathMetrics::new`], over one meter.
struct Make<'a>(&'a Meter);

impl Make<'_> {
    /// A counter.
    fn counter(&self, name: &'static str, description: &'static str) -> Counter<u64> {
        self.0
            .u64_counter(name)
            .with_description(description)
            .build()
    }

    /// A latency histogram in seconds over `ladder`.
    fn histogram(
        &self,
        name: &'static str,
        description: &'static str,
        ladder: &[f64],
    ) -> Histogram<f64> {
        self.0
            .f64_histogram(name)
            .with_description(description)
            .with_unit("s")
            .with_boundaries(ladder.to_vec())
            .build()
    }

    /// A gauge of a Unix time in seconds.
    fn timestamp(&self, name: &'static str, description: &'static str) -> Gauge<f64> {
        self.0
            .f64_gauge(name)
            .with_description(description)
            .with_unit("s")
            .build()
    }
}

/// The process-wide instruments.
static METRICS: OnceLock<AccessPathMetrics> = OnceLock::new();

/// The instruments; on the global no-op meter until [`init`] runs, as
/// [`super::metrics::metrics`].
#[must_use]
pub fn metrics() -> &'static AccessPathMetrics {
    METRICS.get_or_init(|| {
        AccessPathMetrics::new(&opentelemetry::global::meter(super::metrics::SCOPE))
    })
}

/// Install the instruments for the process; a second call returns the
/// existing set.
pub fn init(meter: &Meter) -> &'static AccessPathMetrics {
    METRICS.get_or_init(|| AccessPathMetrics::new(meter))
}

/// The seconds since `started`, for a histogram.
#[must_use]
pub fn seconds_since(started: std::time::Instant) -> f64 {
    started.elapsed().as_secs_f64()
}

/// `ok` or `failed`, as the `outcome` label of `result`.
#[must_use]
pub fn outcome<T, E>(result: &Result<T, E>) -> KeyValue {
    KeyValue::new("outcome", if result.is_ok() { "ok" } else { "failed" })
}

/// The Unix time now, in seconds, for a timestamp gauge.
#[must_use]
pub fn unix_now() -> f64 {
    jiff::Timestamp::now().as_duration().as_secs_f64()
}

/// Register the instruments read at collection time over `service`: each
/// policy listener's state, the size of the per-EHR access copy and the
/// class copy, and the depth of the audit drain's queue.
pub fn observe(meter: &Meter, service: &Arc<FerroEhrService>) {
    let listeners = service.policy_listeners();
    observe_listeners(meter, &listeners);

    let held = Arc::clone(service);
    let _entries = meter
        .u64_observable_gauge(ACCESS_COPY_ENTRIES)
        .with_description("Entries the per-EHR access copy and the class copy hold (copy)")
        .with_callback(move |observer| {
            let (ehrs, classes) = held.ehr_access_held();
            observer.observe(ehrs, &[KeyValue::new("copy", "ehr")]);
            observer.observe(classes, &[KeyValue::new("copy", "classes")]);
        })
        .build();

    let drained = Arc::clone(service);
    let _queue = meter
        .u64_observable_gauge(AUDIT_DRAIN_QUEUE_DEPTH)
        .with_description("Records waiting in the audit drain's queue")
        .with_callback(move |observer| {
            if let Some(queued) = drained.audit_queued() {
                observer.observe(queued, &[]);
            }
        })
        .build();
}

/// Register how many live instances of the deployment differ from this one.
///
/// Counted at its last comparison, by what differs (`differs = signing_key |
/// configuration`); nothing before the first comparison or after one that
/// could not read the rows, which the `instances` health indicator names.
pub fn observe_instances(meter: &Meter, instances: &Arc<Instances>) {
    let held = Arc::clone(instances);
    let _mismatches = meter
        .u64_observable_gauge(INSTANCE_MISMATCHES)
        .with_description(
            "Live instances of the deployment that sign with another key or run another \
             configuration (differs = signing_key | configuration)",
        )
        .with_callback(move |observer| {
            let differences = match held.agreement() {
                Agreement::Agrees { .. } => Vec::new(),
                Agreement::Differs(differences) => differences,
                Agreement::Unread | Agreement::Unknown => return,
            };
            let count = |pick: fn(&crate::instance::Difference) -> bool| {
                u64::try_from(differences.iter().filter(|d| pick(d)).count()).unwrap_or(u64::MAX)
            };
            observer.observe(
                count(|d| d.signing_key),
                &[KeyValue::new("differs", "signing_key")],
            );
            observer.observe(
                count(|d| d.configuration),
                &[KeyValue::new("differs", "configuration")],
            );
        })
        .build();
}

/// Register each listener's state, by the name the `policy_listeners`
/// health indicator reports it under.
pub fn observe_listeners(meter: &Meter, listeners: &[(&'static str, Arc<Listened>)]) {
    let _up = meter
        .u64_observable_gauge(POLICY_LISTENER_UP)
        .with_description("Whether a policy copy's LISTEN connection is up (1) or lost (0)")
        .with_callback(per_listener(listeners, |l| u64::from(l.is_listening())))
        .build();
    let _losses = meter
        .u64_observable_counter(POLICY_LISTENER_LOSSES)
        .with_description("LISTEN connections lost by a policy copy's listener")
        .with_callback(per_listener(listeners, Listened::losses))
        .build();
    let _notified = meter
        .u64_observable_counter(POLICY_NOTIFICATIONS)
        .with_description("Notifications a policy copy's listener received")
        .with_callback(per_listener(listeners, Listened::notifications))
        .build();
    let read = listeners.to_vec();
    let _reads = meter
        .u64_observable_counter(POLICY_COPY_READS)
        .with_description(
            "Reads of the store a policy copy made on a request \
             (reason = notified | unlistened)",
        )
        .with_callback(move |observer| {
            for (name, listened) in &read {
                let (notified, unlistened) = listened.reads();
                for (count, reason) in [(notified, "notified"), (unlistened, "unlistened")] {
                    observer.observe(
                        count,
                        &[
                            KeyValue::new("listener", *name),
                            KeyValue::new("reason", reason),
                        ],
                    );
                }
            }
        })
        .build();

    let aged = listeners.to_vec();
    let _age = meter
        .f64_observable_gauge(POLICY_NOTIFICATION_AGE)
        .with_description("Time since a policy copy's listener received its newest notification")
        .with_unit("s")
        .with_callback(move |observer| {
            let now = jiff::Timestamp::now();
            for (name, listened) in &aged {
                if let Some(at) = listened.last_notified() {
                    let age = now.duration_since(at).as_secs_f64().max(0.0);
                    observer.observe(age, &[KeyValue::new("listener", *name)]);
                }
            }
        })
        .build();
}

/// The callback that observes `read` of each of `listeners`, labelled by
/// its name.
fn per_listener(
    listeners: &[(&'static str, Arc<Listened>)],
    read: fn(&Listened) -> u64,
) -> impl Fn(&dyn AsyncInstrument<u64>) + Send + Sync + 'static {
    let listeners = listeners.to_vec();
    move |observer| {
        for (name, listened) in &listeners {
            observer.observe(read(listened), &[KeyValue::new("listener", *name)]);
        }
    }
}

#[cfg(test)]
#[path = "access_path_tests.rs"]
mod tests;
