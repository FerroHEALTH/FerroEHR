// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one way the crate obtains a pooled connection, with the connect phase of
//! a failure kept in the error type.
//!
//! A [`sqlx::Error`] does not record whether it arose while a connection was
//! being obtained or on an open session, and a statement executed on a
//! `&PgPool` acquires implicitly, so its failure carries no phase either.
//! [`Pool::acquire`] fails only before any statement of the caller runs: the
//! pool timed out or was closed, or opening a new connection failed (sqlx 0.9,
//! <https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.Pool.html#method.acquire>).
//! [`acquire`] wraps every such failure in an [`AcquireError`], which the
//! storage and service bridges answer with `503` plus `Retry-After`, while a
//! failure on the acquired session stays a plain [`sqlx::Error`] and is
//! classified by identity. Executing on the connection [`acquire`] returns,
//! instead of on the pool, is what makes the phase known; a transaction is
//! begun on that connection (`conn.begin()`), so a failing `BEGIN` is a
//! session failure.
//!
//! No openEHR spec governs this — our own design.
//!
//! [`Pool::acquire`]: sqlx::Pool::acquire

use sqlx::PgPool;
use sqlx::Postgres;
use sqlx::pool::PoolConnection;

/// A failure to obtain a connection from a pool: no statement of the caller
/// reached the database, so the request can be repeated once it answers.
///
/// The driver error is the source and stays out of this error's own message.
#[derive(Debug, thiserror::Error)]
#[error("acquiring a database connection failed")]
pub struct AcquireError(#[source] sqlx::Error);

impl AcquireError {
    /// Returns the driver error the pool reported.
    #[must_use]
    pub fn driver(&self) -> &sqlx::Error {
        &self.0
    }

    /// Returns whether the pool gave up waiting for a free connection, as
    /// opposed to failing to open one.
    #[must_use]
    pub fn is_pool_timeout(&self) -> bool {
        matches!(self.0, sqlx::Error::PoolTimedOut)
    }
}

/// Obtains a connection from `pool`, marking any failure as acquire-time.
///
/// The wait, successful or not, is recorded on
/// [`DB_POOL_ACQUIRE_DURATION`](crate::telemetry::metrics::DB_POOL_ACQUIRE_DURATION).
///
/// The connection returns to the pool when the returned guard drops; a caller
/// that executes one statement passes `&mut *acquire(pool).await?` so the guard
/// drops at the end of that statement, as an implicit acquire would.
///
/// # Errors
///
/// [`AcquireError`] when the pool timed out, was closed, or could not open a
/// new connection.
pub async fn acquire(pool: &PgPool) -> Result<PoolConnection<Postgres>, AcquireError> {
    let started = std::time::Instant::now();
    // Boxed as sqlx's own `&Pool` executor boxes it: the connect future is
    // large, and every caller's state machine would otherwise carry it inline.
    let result = Box::pin(pool.acquire()).await;
    crate::telemetry::metrics::metrics()
        .db_pool_acquire_duration
        .record(started.elapsed().as_secs_f64(), &[]);
    result.map_err(AcquireError)
}
