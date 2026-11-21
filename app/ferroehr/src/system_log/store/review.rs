// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The level a stored record of the system chain is recorded for review at
//! (`audit.audit_event.review`): a care relationship written to a record
//! marked above `N` (draft access control, The Decision Procedure, Writing
//! Relationships). Our own design (#6141).

use openehr_rm::draft::access_control::terminology::Confidentiality;
use sqlx::Row;
use sqlx::postgres::PgRow;

use crate::system_log::AuditError;

/// The level in the `review` column of `row`, the record at `chain_seq`.
///
/// # Errors
/// [`AuditError::Unreadable`] for a level this build does not write, and the
/// driver error of the read.
pub(super) fn level(row: &PgRow, chain_seq: i64) -> Result<Option<Confidentiality>, AuditError> {
    let Some(code) = row.try_get::<Option<&str>, _>("review")? else {
        return Ok(None);
    };
    let reason = "its review level is none this build writes";
    Confidentiality::ALL
        .into_iter()
        .find(|level| level.code() == code)
        .map(Some)
        .ok_or(AuditError::Unreadable { chain_seq, reason })
}
