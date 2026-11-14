// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scheduled end of an access-log entry's retention (#5128).
//!
//! Every entry whose `retain_until` has passed is replaced by its tombstone,
//! in place, so the chain still verifies (`docs/plans/access-log.md`
//! §Retention).
//!
//! The audit store's hourly retention worker runs it
//! (`crate::system_log::sender`), beside the reaper of the `audit_event`
//! records written before the access log. No openEHR spec governs audit
//! retention: our own design.

use crate::access_log::store::{AccessLogError, AccessLogStore};
use crate::versioning::audit::dv_date_time;

/// The most entries one transaction replaces.
const BATCH: u16 = 500;

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

/// [`expire`] as the retention worker runs it: the run is timed under
/// `access_log_retention_duration`, the entries replaced go to the
/// `access_log_tombstones` and `atna_audit_reaped` counters, and a failure is
/// logged and retried on the next run.
pub(crate) async fn scheduled(store: &AccessLogStore, now: jiff::Timestamp) {
    let started = std::time::Instant::now();
    let expired = expire(store, now).await;
    let metrics = crate::telemetry::access_path::metrics();
    metrics.log_retention_duration.record(
        crate::telemetry::access_path::seconds_since(started),
        &[crate::telemetry::access_path::outcome(&expired)],
    );
    match expired {
        Ok(0) => {}
        Ok(n) => {
            metrics.log_tombstones.add(n, &[]);
            crate::telemetry::metrics::metrics()
                .atna_audit_reaped
                .add(n, &[]);
            tracing::debug!("access-log retention replaced {n} entries by their tombstones");
        }
        Err(e) => tracing::warn!(error = %e, "access-log retention failed"),
    }
}
