// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the kind of the party containers a demographic write names.
//!
//! No openEHR spec governs the read: our own design.

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::storage::error::StorageError;

/// The stored kind of each of `vo_ids` this store holds, read on `conn` (a
/// commit's transaction sees the objects it has written itself).
///
/// One probe of `vo_head` by its key.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn kinds(
    conn: &mut PgConnection,
    vo_ids: &[Uuid],
) -> Result<Vec<(Uuid, String)>, StorageError> {
    if vo_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query("SELECT vo_id, kind FROM vo_head WHERE vo_id = ANY($1)")
        .bind(vo_ids)
        .fetch_all(&mut *conn)
        .await?;
    rows.iter()
        .map(|r| Ok((r.try_get("vo_id")?, r.try_get("kind")?)))
        .collect::<Result<_, StorageError>>()
}
