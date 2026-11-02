// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The planner statistics of the partitioned parents, which autovacuum never
//! collects (#4453).
//!
//! The planner estimates a join over `node` or `version` from the parent's own
//! statistics, and "partitioned tables are not processed by autovacuum", so
//! they come only from a manual `ANALYZE` of the parent, run when it is
//! populated and again as its partitions change (PostgreSQL 18, §24.1.3
//! Updating Planner Statistics, and the `ANALYZE` reference page). Left
//! at the statistics of an empty or freshly seeded store, a revision history
//! walked the whole commit audit index (#4438) and the population `CONTAINS`
//! drove from every COMPOSITION root.
//!
//! [`refresh`] runs `ANALYZE ONLY` on every partitioned table of the stores,
//! which collects the parent's statistics and leaves its partitions to
//! autovacuum, and analyzes `ehr` while autovacuum has never analyzed it: a
//! table below the autoanalyze threshold (50 rows plus 10 %) would otherwise
//! stay without statistics. It runs after the migrations, after an archive
//! load and every [`REFRESH_INTERVAL`] in [`scheduled`].
//!
//! No openEHR spec governs planner statistics: our own design.

use std::time::Duration;

use sqlx::{PgConnection, PgPool};

use crate::db::DbError;
use crate::db::acquire::acquire;
use crate::db::domain::{Domain, DomainPools};

/// The stores whose partitioned parents the AQL and storage statements join.
pub const STORES: [Domain; 2] = [Domain::Clinical, Domain::Party];

/// How often [`scheduled`] refreshes the parents' statistics.
pub const REFRESH_INTERVAL: Duration = Duration::from_hours(1);

/// The tables one [`refresh`] analyzed, and the ones its role may not.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Refreshed {
    /// The schema-qualified tables analyzed.
    pub analyzed: Vec<String>,
    /// The schema-qualified tables the role holds no `MAINTAIN` privilege on.
    pub refused: Vec<String>,
}

impl Refreshed {
    fn absorb(&mut self, other: Self) {
        self.analyzed.extend(other.analyzed);
        self.refused.extend(other.refused);
    }
}

/// The tables of `schemas` whose statistics [`refresh`] keeps.
const TARGETS_SQL: &str = "SELECT format('%I.%I', n.nspname, c.relname), \
     has_table_privilege(c.oid, 'MAINTAIN') \
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     LEFT JOIN pg_stat_all_tables s ON s.relid = c.oid \
     WHERE n.nspname = ANY($1) \
       AND (c.relkind = 'p' \
            OR (c.relkind = 'r' AND c.relname = 'ehr' \
                AND s.last_analyze IS NULL AND s.last_autoanalyze IS NULL)) \
     ORDER BY 1";

/// Analyze the partitioned parents of `schemas`, and `ehr` while autovacuum
/// has never analyzed it.
///
/// A table the role may not maintain is listed in [`Refreshed::refused`]
/// rather than analyzed, since `ANALYZE` would skip it with only a warning.
///
/// # Errors
/// [`DbError::Sqlx`] when the catalog read or an `ANALYZE` fails.
pub async fn refresh(conn: &mut PgConnection, schemas: &[&str]) -> Result<Refreshed, DbError> {
    let targets: Vec<(String, bool)> = sqlx::query_as(TARGETS_SQL)
        .bind(schemas)
        .fetch_all(&mut *conn)
        .await?;
    let mut refreshed = Refreshed::default();
    for (table, allowed) in targets {
        if !allowed {
            refreshed.refused.push(table);
            continue;
        }
        // The name is `format('%I.%I')` over the catalog, quoted as an
        // identifier; ANALYZE takes no bind placeholder.
        sqlx::query(sqlx::AssertSqlSafe(format!("ANALYZE ONLY {table}")))
            .execute(&mut *conn)
            .await?;
        refreshed.analyzed.push(table);
    }
    Ok(refreshed)
}

/// [`refresh`] of one store on the pool that serves it.
///
/// # Errors
/// [`DbError::Acquire`] when the pool yields no connection, or whatever
/// [`refresh`] returns.
pub async fn refresh_store(pool: &PgPool, store: Domain) -> Result<Refreshed, DbError> {
    let mut conn = acquire(pool).await?;
    refresh(&mut conn, &[store.as_str()]).await
}

/// [`refresh`] of each store on its own pool, so each runs under the role that
/// serves the store.
///
/// # Errors
/// Whatever [`refresh_store`] returns for the first store that fails.
pub async fn refresh_pools(pools: &DomainPools) -> Result<Refreshed, DbError> {
    let mut refreshed = Refreshed::default();
    for store in STORES {
        refreshed.absorb(refresh_store(pools.get(store), store).await?);
    }
    Ok(refreshed)
}

/// Log one refresh's outcome: what was analyzed at debug, a refused table or
/// a failure as a warning.
pub fn report(outcome: &Result<Refreshed, DbError>) {
    match outcome {
        Ok(refreshed) => {
            tracing::debug!(
                analyzed = ?refreshed.analyzed,
                "refreshed the planner statistics of the partitioned parents"
            );
            if !refreshed.refused.is_empty() {
                tracing::warn!(
                    refused = ?refreshed.refused,
                    "the database role holds no MAINTAIN privilege on these tables, so \
                     their planner statistics stay as they are; grant MAINTAIN on them \
                     (PostgreSQL 18, \"Privileges\") or ANALYZE them out of band"
                );
            }
        }
        Err(error) => tracing::warn!(
            error = %error,
            "refreshing the planner statistics of the partitioned parents failed"
        ),
    }
}

/// Refresh the parents' statistics now and then every [`REFRESH_INTERVAL`].
#[expect(
    clippy::infinite_loop,
    reason = "a detached background task with no shutdown channel: it ends when \
              the runtime drops it, as `crate::support::watch` does"
)]
pub async fn scheduled(pools: DomainPools) {
    let mut ticks = tokio::time::interval(REFRESH_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        report(&refresh_pools(&pools).await);
    }
}
