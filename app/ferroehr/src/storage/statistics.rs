// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The counts and listings behind the admin statistics, over `contribution`,
//! `version` and `commit_audit`.
//!
//! A time bound is an ISO 8601 date-time string `PostgreSQL` casts to
//! `timestamptz`, matched against the audit's `time_committed`; `None` leaves
//! that side open. Each scope is a static statement chosen in Rust: a
//! `($3 AND …) OR (NOT $3 AND …)` scoping predicate resists index planning at
//! the measured corpus scale. No openEHR spec governs the SQL: our own design
//! over `clinical/0003_change_control.sql`.

use sqlx::PgPool;

use crate::db::acquire::{acquire, lease};
use crate::storage::error::StorageError;

/// Which contributions a statistic covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContributionScope {
    /// The EHR-scoped contributions (`ehr_id IS NOT NULL`).
    Ehr,
    /// The ehr-less, demographic contributions (`ehr_id IS NULL`).
    Demographic,
}

/// The distinct versioned COMPOSITIONs, unbounded.
const VO_COUNT: &str = "SELECT count(DISTINCT vo_id) FROM version WHERE kind = 'COMPOSITION'";

/// The distinct versioned COMPOSITIONs committed within `$1..=$2`.
const VO_COUNT_IN_RANGE: &str = "SELECT count(DISTINCT v.vo_id) FROM version v \
     JOIN commit_audit a ON a.id = v.commit_audit_id \
     WHERE v.kind = 'COMPOSITION' \
       AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
       AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz)";

/// The COMPOSITION version rows, unbounded.
const VERSION_COUNT: &str = "SELECT count(*) FROM version WHERE kind = 'COMPOSITION'";

/// The COMPOSITION version rows committed within `$1..=$2`.
const VERSION_COUNT_IN_RANGE: &str = "SELECT count(*) FROM version v \
     JOIN commit_audit a ON a.id = v.commit_audit_id \
     WHERE v.kind = 'COMPOSITION' \
       AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
       AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz)";

/// The ids of the contributions of `scope` committed within `lo..=hi`, in
/// commit order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn contribution_ids(
    pool: &PgPool,
    scope: ContributionScope,
    lo: Option<&str>,
    hi: Option<&str>,
) -> Result<Vec<String>, StorageError> {
    let sql = match scope {
        ContributionScope::Ehr => {
            "SELECT c.id::text FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
             WHERE c.ehr_id IS NOT NULL \
               AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
               AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz) \
             ORDER BY a.time_committed, c.id"
        }
        ContributionScope::Demographic => {
            "SELECT c.id::text FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
             WHERE c.ehr_id IS NULL \
               AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
               AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz) \
             ORDER BY a.time_committed, c.id"
        }
    };
    Ok(sqlx::query_scalar(sql)
        .bind(lo)
        .bind(hi)
        .fetch_all(&mut *acquire(pool).await?)
        .await?)
}

/// How many contributions of `scope` were committed within `lo..=hi`.
///
/// Unbounded, the count is index-only over `idx_contribution_ehr_id`; bounded,
/// it joins the audit behind `idx_commit_audit_time_committed`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn contribution_count(
    pool: &PgPool,
    scope: ContributionScope,
    lo: Option<&str>,
    hi: Option<&str>,
) -> Result<i64, StorageError> {
    if lo.is_none() && hi.is_none() {
        let sql = match scope {
            ContributionScope::Ehr => "SELECT count(*) FROM contribution WHERE ehr_id IS NOT NULL",
            ContributionScope::Demographic => {
                "SELECT count(*) FROM contribution WHERE ehr_id IS NULL"
            }
        };
        return Ok(sqlx::query_scalar(sql).fetch_one(pool).await?);
    }
    let sql = match scope {
        ContributionScope::Ehr => {
            "SELECT count(*) FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
             WHERE c.ehr_id IS NOT NULL \
               AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
               AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz)"
        }
        ContributionScope::Demographic => {
            "SELECT count(*) FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
             WHERE c.ehr_id IS NULL \
               AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
               AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz)"
        }
    };
    Ok(sqlx::query_scalar(sql)
        .bind(lo)
        .bind(hi)
        .fetch_one(pool)
        .await?)
}

/// What a COMPOSITION count counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionCount {
    /// The distinct versioned COMPOSITIONs with a version in the range.
    VersionedObjects,
    /// The COMPOSITION version rows in the range.
    Versions,
}

/// Count the COMPOSITIONs `what` names committed within `lo..=hi`.
///
/// The count runs on a leased connection, so a caller that gives up on a long
/// count frees its connection; unbounded, it touches no audit row.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn composition_count(
    pool: &PgPool,
    what: CompositionCount,
    lo: Option<&str>,
    hi: Option<&str>,
) -> Result<i64, StorageError> {
    let (unbounded, in_range) = match what {
        CompositionCount::VersionedObjects => (VO_COUNT, VO_COUNT_IN_RANGE),
        CompositionCount::Versions => (VERSION_COUNT, VERSION_COUNT_IN_RANGE),
    };
    let mut lease = lease(pool).await?;
    let count = if lo.is_none() && hi.is_none() {
        lease
            .run(|conn| sqlx::query_scalar(unbounded).fetch_one(conn))
            .await
    } else {
        lease
            .run(|conn| {
                sqlx::query_scalar(in_range)
                    .bind(lo)
                    .bind(hi)
                    .fetch_one(conn)
            })
            .await
    }?;
    Ok(count)
}
