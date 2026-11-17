// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reconciler of held audit records (#5762): any replica, one at a time
//! under a lease, resolves the reservations whose records have been held
//! longer than a grace period.
//!
//! A reservation the write's domain holds belongs to a write that committed
//! while its process died before the release: its records are released,
//! exactly as the request wrote them, once. A reservation no domain holds
//! after the grace belongs to a write that never committed: its records are
//! discarded. A domain's reservation whose records were already released is
//! removed. The grace outlasts the statement timeout, so no commit still in
//! flight is taken for one that never happened. No specification governs
//! the reconciliation: our own design.

use std::time::Duration;

use opentelemetry::KeyValue;
use sqlx::{Connection, PgPool};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::store::AuditStore;

/// How long a reservation's records may be held before the reconciler
/// resolves it: past the default statement timeout of one minute.
pub const GRACE: Duration = Duration::from_secs(120);

/// How often the reconciler looks.
const EVERY: Duration = Duration::from_secs(10);

/// The lease every replica's reconciler takes, so one resolves at a time.
const LEASE: &str = "SELECT pg_try_advisory_xact_lock(hashtext('audit.reservation_reconciler'), 0)";

/// The pools of the domains a write commits in.
#[derive(Debug, Clone)]
pub struct Domains {
    /// The clinical domain.
    pub clinical: PgPool,
    /// The party domain.
    pub party: PgPool,
}

/// What one reconciliation did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reconciled {
    /// Reservations whose records were released: their writes committed.
    pub released: u64,
    /// Reservations whose records were discarded: their writes never did.
    pub discarded: u64,
    /// Domain reservations removed whose records were released already.
    pub removed: u64,
}

impl Reconciled {
    /// Whether a process stopped between a write and its release: records
    /// were released or discarded. Removing spent reservations is the
    /// reconciler's routine work, since a committed write's release leaves
    /// its reservation to it (#5762).
    #[must_use]
    pub fn found_a_stop(&self) -> bool {
        self.released > 0 || self.discarded > 0
    }
}

/// The reconciler's loop, until the runtime ends the task.
#[expect(
    clippy::infinite_loop,
    reason = "a detached reconciler ended by the audit runtime, as the reaper is"
)]
pub(super) async fn run(store: AuditStore, domains: Domains) {
    loop {
        tokio::time::sleep(EVERY).await;
        if let Err(error) = reconcile(&store, &domains, GRACE).await {
            tracing::error!(
                error = %crate::error::ErrorChain::new(&error),
                "the audit reservation reconciliation failed; it runs again"
            );
        }
    }
}

/// Resolve every reservation held longer than `grace`, under the lease; a
/// replica that does not get the lease does nothing this round.
///
/// # Errors
/// [`AuditError::Store`] when a statement fails; what was resolved stays.
pub async fn reconcile(
    store: &AuditStore,
    domains: &Domains,
    grace: Duration,
) -> Result<Reconciled, AuditError> {
    let mut lease = acquire(&store.pool).await?;
    let mut tx = lease.begin().await?;
    let leased: bool = sqlx::query_scalar(LEASE).fetch_one(&mut *tx).await?;
    if !leased {
        return Ok(Reconciled::default());
    }
    let held = store.held_longer_than(grace).await?;
    let metrics = crate::telemetry::access_path::metrics();
    metrics
        .reservations_held
        .record(u64::try_from(held.len()).unwrap_or(u64::MAX), &[]);
    metrics
        .reservation_age
        .record(held.first().map_or(0.0, |(_, age)| age.as_secs_f64()), &[]);
    let mut done = Reconciled::default();
    let mut committed = Vec::new();
    let mut never = Vec::new();
    for (id, _age) in &held {
        if reserved_in(&domains.clinical, *id).await? || reserved_in(&domains.party, *id).await? {
            committed.push(*id);
        } else {
            never.push(*id);
        }
    }
    if !committed.is_empty() {
        store.release(&committed).await?;
        done.released = u64::try_from(committed.len()).unwrap_or(u64::MAX);
    }
    if !never.is_empty() {
        store.discard(&never).await?;
        done.discarded = u64::try_from(never.len()).unwrap_or(u64::MAX);
    }
    for pool in [&domains.clinical, &domains.party] {
        done.removed = done
            .removed
            .saturating_add(remove_released(store, pool, grace).await?);
    }
    // NOTE: our own design (#5762), the lease's transaction, which no request makes.
    tx.commit().await?;
    for (outcome, count) in [
        ("released", done.released),
        ("discarded", done.discarded),
        ("removed", done.removed),
    ] {
        metrics
            .reservations_reconciled
            .add(count, &[KeyValue::new("outcome", outcome)]);
    }
    if done.found_a_stop() {
        tracing::warn!(
            released = done.released,
            discarded = done.discarded,
            removed = done.removed,
            "audit reservations reconciled after a process stopped between a write and its release"
        );
    } else if done.removed > 0 {
        tracing::debug!(removed = done.removed, "spent audit reservations removed");
    }
    Ok(done)
}

/// Whether the domain of `pool` holds the reservation `id`.
async fn reserved_in(pool: &PgPool, id: Uuid) -> Result<bool, AuditError> {
    let held: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM audit_reservation WHERE id = $1)")
            .bind(id)
            .fetch_one(&mut *acquire(pool).await?)
            .await?;
    Ok(held)
}

/// Remove the reservations of `pool`'s domain older than `grace` whose
/// records no longer wait held: released, so the reservation is spent.
async fn remove_released(
    store: &AuditStore,
    pool: &PgPool,
    grace: Duration,
) -> Result<u64, AuditError> {
    let old: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM audit_reservation WHERE taken_at < now() - make_interval(secs => $1)",
    )
    .bind(grace.as_secs_f64())
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    if old.is_empty() {
        return Ok(0);
    }
    let still_held: Vec<Uuid> = store.held_among(&old).await?;
    let spent: Vec<Uuid> = old
        .into_iter()
        .filter(|id| !still_held.contains(id))
        .collect();
    if spent.is_empty() {
        return Ok(0);
    }
    let removed = sqlx::query("DELETE FROM audit_reservation WHERE id = ANY($1)")
        .bind(&spent)
        .execute(&mut *acquire(pool).await?)
        .await?
        .rows_affected();
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::Reconciled;

    #[test]
    fn removing_spent_reservations_is_no_stop() {
        let routine = Reconciled {
            removed: 107,
            ..Reconciled::default()
        };
        assert!(!routine.found_a_stop());
        for stopped in [
            Reconciled {
                released: 1,
                ..routine
            },
            Reconciled {
                discarded: 1,
                ..routine
            },
        ] {
            assert!(stopped.found_a_stop(), "{stopped:?}");
        }
    }
}
