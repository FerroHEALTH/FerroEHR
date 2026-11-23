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
/// One probe of `vo_head` by its key per object. The ids are read by their
/// subscripts, which give a custom plan no array length to be cheaper by, so
/// the plan cache settles on the generic plan (#6392).
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
    let rows = sqlx::query(
        "SELECT h.vo_id, h.kind FROM generate_subscripts($1::uuid[], 1) AS g (i) \
         JOIN vo_head h ON h.vo_id = ($1::uuid[])[g.i]",
    )
    .bind(vo_ids)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|r| Ok((r.try_get("vo_id")?, r.try_get("kind")?)))
        .collect::<Result<_, StorageError>>()
}
