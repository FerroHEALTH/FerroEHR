// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The setup every pooled physical connection runs before it serves a query.
//! No openEHR spec governs persistence: our own design.

use sqlx::PgConnection;

/// How often a backend running a statement checks that its client is still
/// connected, so a statement whose connection [`super::acquire::Lease`] closed stops
/// within this interval instead of running to its end.
const CLIENT_CONNECTION_CHECK: &str = "SET client_connection_check_interval = 1000";

/// Everything a freshly-opened physical connection needs before it serves a
/// query: the domain's search path, the client-connection check and the
/// statement-timeout backstop.
///
/// One implementation for every pool, so a domain cannot drop a setting.
/// Dropping the timeout in particular silently disarms the DB-side
/// runaway-query guard, and a broken control must never look like a policy
/// outcome.
pub(super) async fn open_session(
    conn: &mut PgConnection,
    search_path: &'static str,
    statement_timeout: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(search_path).execute(&mut *conn).await?;
    // NOTE: PostgreSQL docs §20.3.1, `client_connection_check_interval`:
    // a backend whose client closed the connection stops its running query at the next check.
    sqlx::query(CLIENT_CONNECTION_CHECK)
        .execute(&mut *conn)
        .await?;
    if let Some(statement_timeout) = statement_timeout {
        // A session-level SET on the physical connection, surviving every
        // checkout, where `SET LOCAL` would last one transaction.
        // `AssertSqlSafe` is audited: PostgreSQL's `SET` takes no bind
        // placeholder and the value is a `u64` from our own configuration,
        // never client input.
        sqlx::query(sqlx::AssertSqlSafe(statement_timeout.to_owned()))
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
