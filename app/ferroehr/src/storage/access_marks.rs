// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The marks of the EHDS Art. 8 restriction and Art. 10 opt-out directives.
//!
//! `ehr.access_restricted_at`, `vo_head.access_restricted_at` and
//! `ehr.opted_out_at` are the projection of the `restriction` and `opt_out`
//! directives in force in an EHR's current `EHR_ACCESS`, written by [`write()`]
//! in the transaction of every `EHR_ACCESS` commit; the event outbox, which
//! has no caller for the access evaluator to judge, withholds what they mark
//! (`docs/law/eu/ehds/text.html` Art. 8 and Art. 10). No openEHR spec governs
//! either right or the projection: our own design/extension.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::ids::EhrId;
use crate::storage::error::StorageError;

/// The marks one EHR's directives in force imply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    /// An `opt_out` directive is in force.
    pub opted_out: bool,
    /// A `restriction` directive in force covers the whole EHR.
    pub restricted_ehr: bool,
    /// The versioned objects a `restriction` directive in force names, sorted
    /// and without duplicates.
    pub restricted_objects: Vec<Uuid>,
}

/// Write the marks of `ehr_id`: each mark that holds keeps the instant it was
/// first projected, and each that no longer holds is cleared.
///
/// Only the rows whose marks change are written.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn write(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    marks: &Marks,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE ehr SET \
             access_restricted_at = CASE WHEN $2 THEN coalesce(access_restricted_at, now()) END, \
             opted_out_at = CASE WHEN $3 THEN coalesce(opted_out_at, now()) END \
         WHERE id = $1 \
           AND ((access_restricted_at IS NOT NULL) <> $2 OR (opted_out_at IS NOT NULL) <> $3)",
    )
    .bind(ehr_id)
    .bind(marks.restricted_ehr)
    .bind(marks.opted_out)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE vo_head SET access_restricted_at = \
             CASE WHEN $2 OR vo_id = ANY($3) THEN coalesce(access_restricted_at, now()) END \
         WHERE ehr_id = $1 \
           AND (access_restricted_at IS NOT NULL) <> ($2 OR vo_id = ANY($3))",
    )
    .bind(ehr_id)
    .bind(marks.restricted_ehr)
    .bind(&marks.restricted_objects)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
