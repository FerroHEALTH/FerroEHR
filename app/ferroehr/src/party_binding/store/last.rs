// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The last version with a body of a deleted `PERSON`, whose keys its
//! tombstone keeps: read from the versions, or taken from what a commit's
//! gate read found (#6257). No openEHR spec governs the side table: our own
//! design.

use sqlx::{PgConnection, Row};

use super::{DELETED, Head, parsed};
use crate::party_binding::BindingMapping;
use crate::storage::error::StorageError;
use crate::versioning::Kind;

/// `head` with what its last version with a body holds, when it is a deleted
/// `PERSON`: the key its tombstone keeps.
pub(super) async fn with_last(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    mut head: Head,
) -> Result<Head, StorageError> {
    if kind != Kind::Person || head.lifecycle_state != DELETED {
        return Ok(head);
    }
    let row = sqlx::query(
        "SELECT v.body, v.lifecycle_state FROM version v \
         WHERE v.vo_id = $1 AND v.body IS NOT NULL ORDER BY v.sys_version DESC LIMIT 1",
    )
    .bind(head.vo_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = row {
        let body: String = row.try_get("body")?;
        head.last = Some((
            row.try_get("lifecycle_state")?,
            parsed(kind, &body, mapping)?,
        ));
    }
    Ok(head)
}

/// [`with_last`] off what the gate read found of the versions before the
/// commit, `prior`, in place of a read: a delete writes no body, so the last
/// body before it is the last one.
pub(super) fn with_prior(
    mapping: &BindingMapping,
    kind: Kind,
    mut head: Head,
    prior: &crate::storage::version_repo::gate::Prior,
) -> Result<Head, StorageError> {
    if kind != Kind::Person || head.lifecycle_state != DELETED {
        return Ok(head);
    }
    if let Some((_, lifecycle_state, body)) = &prior.last_body {
        head.last = Some((lifecycle_state.clone(), parsed(kind, body, mapping)?));
    }
    Ok(head)
}
