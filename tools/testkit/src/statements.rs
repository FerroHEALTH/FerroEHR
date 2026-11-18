// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The test server's `pg_stat_statements` table: a test's count of its own
//! statements, and the statements of dropped clones removed from the table.
//! No specification governs this: our own design.

use sqlx::{PgConnection, PgPool};

/// The oid of the database `name`, `None` when the server has none of that
/// name or the read fails.
async fn database_oid(admin: &mut PgConnection, name: &str) -> Option<sqlx::postgres::types::Oid> {
    // NOTE: our own design, the oid only lets a dropped database's statements go; without it they
    // stay counted, as before.
    sqlx::query_scalar("SELECT oid FROM pg_database WHERE datname = $1")
        .bind(name)
        .fetch_optional(admin)
        .await
        .ok()
        .flatten()
}

/// Whether a drop ends the database's live sessions first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Force {
    /// `WITH (FORCE)`: the clone's owner is done with it.
    Yes,
    /// A clone with live sessions belongs to a running test process.
    No,
}

/// Drops the database `name` and removes the statements
/// `pg_stat_statements` counted in it.
///
/// # Errors
///
/// The [`sqlx::Error`] of the drop. A failure to remove the statements is
/// not one: the statements then stay counted.
pub(crate) async fn drop_database(
    admin: &mut PgConnection,
    name: &str,
    force: Force,
) -> Result<(), sqlx::Error> {
    let oid = database_oid(admin, name).await;
    let sql = match force {
        Force::Yes => format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"),
        Force::No => format!("DROP DATABASE IF EXISTS {name}"),
    };
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(&mut *admin)
        .await?;
    if let Some(oid) = oid {
        drop(forget_statements(admin, oid).await);
    }
    Ok(())
}

/// Removes the statements `pg_stat_statements` counted in the dropped
/// database `oid` from the server's statement table, so a dropped clone's
/// statements do not fill it (`pg_stat_statements.max`) and push out the
/// statements a running test counts.
///
/// # Errors
///
/// The [`sqlx::Error`] of the statement, when the admin database has no
/// `pg_stat_statements` (a server without the module).
async fn forget_statements(
    admin: &mut PgConnection,
    oid: sqlx::postgres::types::Oid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_stat_statements_reset(0, $1, 0)")
        .bind(oid)
        .execute(admin)
        .await
        .map(drop)
}

/// Removes from the server's statement table the statements of every
/// database the server no longer holds: those of clones dropped before their
/// statements were removed with them, or by a process that ended first.
///
/// # Errors
///
/// The [`sqlx::Error`] of the statement, when the admin database has no
/// `pg_stat_statements` (a server without the module).
pub(crate) async fn forget_dropped_statements(admin: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(
        "SELECT pg_stat_statements_reset(0, s.dbid, 0) \
           FROM (SELECT DISTINCT dbid FROM pg_stat_statements \
                  WHERE dbid NOT IN (SELECT oid FROM pg_database)) s",
    )
    .execute(admin)
    .await
    .map(drop)
}

/// The evictions of the server's statement table since its last full reset.
///
/// It is `pg_stat_statements_info.dealloc`, read on `pool`, whose database
/// has the extension. A test that reads the statement counts reads this
/// before and after its window: an eviction in between may have dropped a
/// statement it counts.
///
/// # Errors
///
/// The [`sqlx::Error`] of the read.
pub async fn statement_evictions(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT dealloc FROM pg_stat_statements_info")
        .fetch_one(pool)
        .await
}

/// Starts a test's count of its statements.
///
/// It resets what `pg_stat_statements` counted in `pool`'s database and
/// returns the evictions of the server's statement table so far
/// ([`statement_evictions`]), which [`assert_statements_kept`] compares at
/// the end of the count.
///
/// # Errors
///
/// The [`sqlx::Error`] of the reset or the read.
pub async fn statement_window(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query(
        "SELECT pg_stat_statements_reset(0, \
         (SELECT oid FROM pg_database WHERE datname = current_database()), 0)",
    )
    .execute(pool)
    .await?;
    statement_evictions(pool).await
}

/// Holds that the server's statement table evicted nothing in a count.
///
/// The count began with `since` evictions ([`statement_window`]); with none
/// since, it holds every statement the test ran. A count read across an
/// eviction may have lost statements, so a budget it checks would pass a
/// regression.
///
/// # Errors
///
/// The [`sqlx::Error`] of the read, and [`StatementsEvicted`] with both
/// counts when the table evicted statements in the window.
pub async fn assert_statements_kept(pool: &PgPool, since: i64) -> Result<(), StatementsEvicted> {
    let now = statement_evictions(pool)
        .await
        .map_err(StatementsEvicted::Read)?;
    if now == since {
        Ok(())
    } else {
        Err(StatementsEvicted::Evicted { since, now })
    }
}

/// Why a count of statements cannot be trusted.
#[derive(Debug, thiserror::Error)]
pub enum StatementsEvicted {
    /// The evictions could not be read.
    #[error("the evictions of pg_stat_statements could not be read")]
    Read(#[source] sqlx::Error),
    /// The table evicted statements while the test counted them.
    #[error(
        "pg_stat_statements evicted statements while the test counted them ({since} evictions \
         before, {now} after), so the count may have lost some: the server's \
         pg_stat_statements.max is too small for the run (tools/testkit/src/server_settings.rs, \
         scripts/lib/test-postgres.sh)"
    )]
    Evicted {
        /// The evictions when the count began.
        since: i64,
        /// The evictions when it ended.
        now: i64,
    },
}
