// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lock that serializes changes to the shared test server's
//! cluster-global roles across test processes.

use sqlx::{Connection, PgConnection};

use crate::TestkitError;

/// Advisory-lock key serializing template builds and every other change to
/// the cluster-global roles across test processes ([`RolesLock`]).
pub(crate) const TEMPLATE_LOCK_KEY: i64 = 0x0EB2_7E57_0001;

/// Holds the lock that serializes changes to the server's cluster-global
/// roles, the same lock a template build holds, until it is released or
/// dropped.
///
/// Roles are shared by every database of a cluster (`PostgreSQL` docs
/// § 22.1 "Database Roles"), and the migrations create a role only when
/// `pg_roles` lacks it, so two first migrations of fresh databases racing on
/// one server both see it missing and the second `CREATE ROLE` fails with
/// `23505`. A test that migrates a database [`empty_db`](super::empty_db) returned, or that
/// drops a role the migrations create, holds this lock around that step,
/// taken after its databases exist, since [`db()`](super::db()) may build the template under
/// the same lock.
#[derive(Debug)]
pub struct RolesLock {
    connection: PgConnection,
}

impl RolesLock {
    /// Releases the lock and closes its connection; dropping the guard
    /// releases it too, when the server ends the session.
    pub async fn release(mut self) {
        drop(
            sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(TEMPLATE_LOCK_KEY)
                .execute(&mut self.connection)
                .await,
        );
        drop(self.connection.close().await);
    }
}

/// Takes the [`RolesLock`] on the shared test server, waiting for a holder
/// in any other test process.
///
/// # Errors
///
/// Returns [`TestkitError`] when the server cannot be acquired or the lock
/// cannot be taken.
pub async fn lock() -> Result<RolesLock, TestkitError> {
    let server = super::server().await?;
    let mut connection = super::connect_ready(server).await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(TEMPLATE_LOCK_KEY)
        .execute(&mut connection)
        .await?;
    Ok(RolesLock { connection })
}
