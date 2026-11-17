// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scheduled end of an access-log entry's retention (#5128) and of its
//! tombstone's (#5619).
//!
//! Every entry whose `retain_until` has passed is replaced by its tombstone,
//! in place, so the chain still verifies (`docs/plans/access-log.md`
//! §Retention). A tombstone is kept as long again, and then each run of
//! tombstones past their end folds into one gap of the chain
//! ([`crate::access_log::gap`]). The shared parts past their own
//! `retain_until` that no entry names any more are collected after the fold
//! (#5968).
//!
//! The entries imported from another system end the same way, at the
//! `retain_until` they state, and their tombstones fold by the imported rule
//! ([`crate::access_log::imported`], #5664).
//!
//! The audit store's hourly retention worker runs both
//! (`crate::system_log::sender`), beside the reaper of the `audit_event`
//! records written before the access log. No openEHR spec governs audit
//! retention: our own design.

use crate::access_log::gap::Folded;
use crate::access_log::store::{AccessLogError, AccessLogStore};
use crate::versioning::audit::dv_date_time;

/// The most entries one transaction replaces.
const BATCH: u16 = 500;

/// The most tombstones one transaction folds.
// NOTE: our own design (#5619), at about 1.1 kB a tombstone one fold reads near 5 MB.
const FOLD_BATCH: u16 = 5000;

/// The most shared parts one statement collects.
const COLLECT_BATCH: u16 = 500;

/// The log a retention run works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Log {
    /// This system's own chain.
    Local,
    /// The logs imported from other systems (#5664).
    Imported,
}

/// Replace every entry of `log` whose retention ended before the UTC date of
/// `now` by its tombstone, removed at `now`, and return how many were
/// replaced.
///
/// # Errors
/// The errors of [`AccessLogStore::tombstone_expired`] or
/// [`AccessLogStore::tombstone_expired_imported`]; the batches replaced
/// before the failing one stay replaced.
pub async fn expire(
    store: &AccessLogStore,
    log: Log,
    now: jiff::Timestamp,
) -> Result<u64, AccessLogError> {
    let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
    let removed = dv_date_time(&now);
    let mut replaced = 0_u64;
    loop {
        let count = match log {
            Log::Local => {
                let batch = store.tombstone_expired(today, &removed, BATCH).await?;
                u64::try_from(batch.len()).unwrap_or(u64::MAX)
            }
            Log::Imported => {
                store
                    .tombstone_expired_imported(today, &removed, BATCH)
                    .await?
            }
        };
        replaced = replaced.saturating_add(count);
        if count < u64::from(BATCH) {
            return Ok(replaced);
        }
    }
}

/// Fold the tombstones of `log` whose own retention ended by `now` into the
/// gaps of their runs, and return what was removed and written.
///
/// On the imported logs only the positions no kept imported record needs
/// fold.
///
/// # Errors
/// The errors of [`AccessLogStore::fold_expired`]; the batches folded before
/// the failing one stay folded.
pub async fn fold(
    store: &AccessLogStore,
    log: Log,
    now: jiff::Timestamp,
) -> Result<Folded, AccessLogError> {
    let mut total = Folded::default();
    loop {
        let batch = store.fold_expired(log, now, FOLD_BATCH).await?;
        total.tombstones = total.tombstones.saturating_add(batch.tombstones);
        total.gaps = total.gaps.saturating_add(batch.gaps);
        let done = match log {
            Log::Local => batch.tombstones < u64::from(FOLD_BATCH),
            Log::Imported => batch.gaps == 0,
        };
        if done {
            return Ok(total);
        }
    }
}

/// Remove the shared parts past their retention that no entry names, in
/// batches of [`COLLECT_BATCH`], and return how many (#5968).
///
/// # Errors
/// The errors of [`AccessLogStore::collect_shared`]; the batches removed
/// before the failing one stay removed.
pub async fn collect(store: &AccessLogStore, now: jiff::Timestamp) -> Result<u64, AccessLogError> {
    let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
    let mut removed = 0_u64;
    loop {
        let batch = store.collect_shared(today, COLLECT_BATCH).await?;
        removed = removed.saturating_add(batch);
        if batch < u64::from(COLLECT_BATCH) {
            return Ok(removed);
        }
    }
}

/// What one retention run did to one chain: the entries replaced by their
/// tombstones, and the tombstones and gaps of the fold.
type Ran = (u64, Folded);

/// [`expire`] then [`fold`] on the local chain and then on the imported
/// logs, as the retention worker runs them: the run is timed under
/// `access_log_retention_duration`, the entries replaced go to the
/// `access_log_tombstones` and `atna_audit_reaped` counters, the tombstones
/// folded and the gaps written to `access_log_folded` and `access_log_gaps`,
/// each counter with `chain` = `local` or `imported`, and a failure is logged
/// and retried on the next run.
pub(crate) async fn scheduled(store: &AccessLogStore, now: jiff::Timestamp) {
    let started = std::time::Instant::now();
    let ran = run(store, now).await;
    let metrics = crate::telemetry::access_path::metrics();
    metrics.log_retention_duration.record(
        crate::telemetry::access_path::seconds_since(started),
        &[crate::telemetry::access_path::outcome(&ran)],
    );
    match ran {
        Ok((local, imported)) => {
            count("local", local);
            count("imported", imported);
        }
        Err(e) => tracing::warn!(error = %e, "access-log retention failed"),
    }
}

/// Count what one run did to the `chain` named.
fn count(chain: &'static str, (replaced, folded): Ran) {
    let metrics = crate::telemetry::access_path::metrics();
    let labels = [opentelemetry::KeyValue::new("chain", chain)];
    if replaced > 0 {
        metrics.log_tombstones.add(replaced, &labels);
        crate::telemetry::metrics::metrics()
            .atna_audit_reaped
            .add(replaced, &[]);
        tracing::debug!(
            chain,
            "access-log retention replaced {replaced} entries by tombstones"
        );
    }
    if folded.gaps > 0 {
        metrics.log_folded.add(folded.tombstones, &labels);
        metrics.log_gaps.add(folded.gaps, &labels);
        tracing::debug!(
            chain,
            tombstones = folded.tombstones,
            gaps = folded.gaps,
            "access-log retention folded tombstones into gaps"
        );
    }
}

/// One retention run: on the local chain and then on the imported logs, the
/// entries replaced, then the tombstones folded.
async fn run(store: &AccessLogStore, now: jiff::Timestamp) -> Result<(Ran, Ran), AccessLogError> {
    let local = (
        expire(store, Log::Local, now).await?,
        fold(store, Log::Local, now).await?,
    );
    collect(store, now).await?;
    let imported = (
        expire(store, Log::Imported, now).await?,
        fold(store, Log::Imported, now).await?,
    );
    Ok((local, imported))
}
