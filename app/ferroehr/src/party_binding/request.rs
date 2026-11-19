// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party store's revisions as one request reads them (#6048).
//!
//! Every fact of the party store a decision rests on is held in a copy this
//! replica keeps by notification: the binding lookups, the grants and the
//! caller facts. A notification arrives after the commit it reports, so each
//! copy is taken only once the request has read the store's revisions and
//! found the copy at them. A request reads them once, in one indexed read on
//! a connection of its own, at the first decision that needs them, and its
//! later decisions take the same read; outside a request every caller reads
//! them itself. No openEHR spec governs the copies: our own design.

use std::future::Future;

use sqlx::PgPool;
use tokio::sync::OnceCell;

use crate::db::acquire::acquire;
use crate::party_binding::grants::live::revisions::Revisions;
use crate::party_binding::grants::store;
use crate::storage::error::StorageError;

tokio::task_local! {
    /// The revisions the running request read; empty until its first read.
    static READ: OnceCell<Revisions>;
}

/// Runs `fut` as one request: the first read of the store's revisions inside
/// it serves every later one.
pub async fn scope<F: Future>(fut: F) -> F::Output {
    READ.scope(OnceCell::new(), fut).await
}

/// The revisions in force in the party store: the ones the running request
/// read, or one indexed read on a connection of `pool` that is given back
/// before this returns.
///
/// # Errors
/// [`StorageError`] when the party store does not answer; a failed read is
/// not kept, and the next caller reads again.
pub(crate) async fn revisions(pool: &PgPool) -> Result<Revisions, StorageError> {
    let read = || async {
        let mut conn = acquire(pool).await?;
        store::revisions(&mut conn).await
    };
    // NOTE: tokio LocalKey::try_with, outside a request (a background task, a
    // test of a service) there is nothing to share the read with.
    let Ok(held) = READ.try_with(|held| held.get().copied()) else {
        return read().await;
    };
    if let Some(held) = held {
        return Ok(held);
    }
    let fresh = read().await?;
    // NOTE: tokio OnceCell::set, a read a sibling future of the request
    // finished first stands, and both are revisions the store held.
    Ok(READ
        .try_with(|held| {
            match held.set(fresh) {
                Ok(()) | Err(_) => {}
            }
            held.get().copied().unwrap_or(fresh)
        })
        .unwrap_or(fresh))
}
