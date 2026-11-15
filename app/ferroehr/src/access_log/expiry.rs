// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scheduled end of an access-log entry's retention (#5128) and of its
//! tombstone's (#5619).
//!
//! Every entry whose `retain_until` has passed is replaced by its tombstone,
//! in place, so the chain still verifies (`docs/plans/access-log.md`
//! §Retention). A tombstone is kept as long again, and then each run of
//! tombstones past their end folds into one gap of the chain
//! ([`crate::access_log::gap`]).
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

/// Replace every entry whose retention ended before the UTC date of `now` by
/// its tombstone, removed at `now`, and return how many were replaced.
///
/// # Errors
/// The errors of [`AccessLogStore::tombstone_expired`]; the batches replaced
/// before the failing one stay replaced.
pub async fn expire(store: &AccessLogStore, now: jiff::Timestamp) -> Result<u64, AccessLogError> {
    let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
    let removed = dv_date_time(&now);
    let mut replaced = 0_u64;
    loop {
        let batch = store.tombstone_expired(today, &removed, BATCH).await?;
        replaced = replaced.saturating_add(u64::try_from(batch.len()).unwrap_or(u64::MAX));
        if batch.len() < usize::from(BATCH) {
            return Ok(replaced);
        }
    }
}

/// Replace every imported entry whose retention ended before the UTC date of
/// `now` by its tombstone, removed at `now`, and return how many were
/// replaced.
///
/// # Errors
/// The errors of [`AccessLogStore::tombstone_expired_imported`]; the batches
/// replaced before the failing one stay replaced.
pub async fn expire_imported(
    store: &AccessLogStore,
    now: jiff::Timestamp,
) -> Result<u64, AccessLogError> {
    let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
    let removed = dv_date_time(&now);
    let mut replaced = 0_u64;
    loop {
        let batch = store
            .tombstone_expired_imported(today, &removed, BATCH)
            .await?;
        replaced = replaced.saturating_add(batch);
        if batch < u64::from(BATCH) {
            return Ok(replaced);
        }
    }
}

/// Fold every tombstone whose own retention ended by `now` into the gaps of
/// its runs, and return what was removed and written.
///
/// # Errors
/// The errors of [`AccessLogStore::fold_expired`]; the batches folded before
/// the failing one stay folded.
pub async fn fold(store: &AccessLogStore, now: jiff::Timestamp) -> Result<Folded, AccessLogError> {
    let mut total = Folded::default();
    loop {
        let batch = store.fold_expired(now, FOLD_BATCH).await?;
        total.tombstones = total.tombstones.saturating_add(batch.tombstones);
        total.gaps = total.gaps.saturating_add(batch.gaps);
        if batch.tombstones < u64::from(FOLD_BATCH) {
            return Ok(total);
        }
    }
}

/// Fold the imported positions that no kept imported record needs any more
/// into gaps, and return what was removed and written.
///
/// # Errors
/// The errors of [`AccessLogStore::fold_expired_imported`]; the batches
/// folded before the failing one stay folded.
pub async fn fold_imported(
    store: &AccessLogStore,
    now: jiff::Timestamp,
) -> Result<Folded, AccessLogError> {
    let mut total = Folded::default();
    loop {
        let batch = store.fold_expired_imported(now, FOLD_BATCH).await?;
        total.tombstones = total.tombstones.saturating_add(batch.tombstones);
        total.gaps = total.gaps.saturating_add(batch.gaps);
        if batch.gaps == 0 {
            return Ok(total);
        }
    }
}

/// What one retention run did to one chain: the entries replaced by their
/// tombstones, and the tombstones and gaps of the fold.
type Ran = (u64, Folded);

/// [`expire`] then [`fold`], and [`expire_imported`] then [`fold_imported`],
/// as the retention worker runs them: the run is timed under
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
    let local = (expire(store, now).await?, fold(store, now).await?);
    let imported = (
        expire_imported(store, now).await?,
        fold_imported(store, now).await?,
    );
    Ok((local, imported))
}
