// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! CONTRIBUTION reads: a contribution's own audit, the versions it affected,
//! and per-EHR listing/counting.
//!
//! No openEHR spec governs the SQL — our own design. The CONTRIBUTION semantics realized are RM common master06
//! §Contributions / §Committal and Audits.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// A CONTRIBUTION's own audit row (`contribution` ⋈ `audit`), flattened.
#[derive(Debug, Clone)]
pub struct ContributionAudit {
    /// `AUDIT_DETAILS.system_id`.
    pub system_id: String,
    /// The numeric `audit_change_type` group code of the change set.
    pub change_type: String,
    /// The canonical `DV_TEXT` fragment of `AUDIT_DETAILS.description`, when
    /// the committer supplied one.
    pub description: Option<Value>,
    /// The canonical `PARTY_PROXY` JSON of the committer.
    pub committer: Value,
    /// The canonical fragment of the `ATTESTATION`-declared attributes when the
    /// change-set audit is an `ATTESTATION` (RM common master06 §Attestation).
    pub attestation: Option<Value>,
    /// The server-computed commit instant.
    pub time_committed: jiff::Timestamp,
}

/// Read a CONTRIBUTION's audit, scoped to its owning EHR (`None` scope = the
/// demographic, ehr-less store). `None` when the contribution does not exist
/// in that scope.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn contribution_audit(
    pool: &PgPool,
    contribution_id: Uuid,
    ehr_id: Option<EhrId>,
) -> Result<Option<ContributionAudit>, StorageError> {
    let Some(row) = sqlx::query(
        "SELECT a.system_id, a.change_type, a.description, a.committer, a.attestation, \
         a.time_committed \
         FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
         WHERE c.id = $1 AND c.ehr_id IS NOT DISTINCT FROM $2",
    )
    .bind(contribution_id)
    .bind(ehr_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(ContributionAudit {
        system_id: row.try_get("system_id")?,
        change_type: row.try_get("change_type")?,
        description: row.try_get("description")?,
        committer: row.try_get("committer")?,
        attestation: row.try_get("attestation")?,
        time_committed: row
            .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
            .to_jiff(),
    }))
}

/// The versions a CONTRIBUTION affected: the rows it committed, unioned with
/// the rows its `666|attestation|` items attested (which add no new version)
/// — deduplicated.
///
/// Returned as `(vo_id, (trunk, branch_number, branch_version),
/// creating_system_id, kind_text)`.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn contribution_version_refs(
    pool: &PgPool,
    contribution_id: Uuid,
) -> Result<Vec<(VoId, (i32, i32, i32), String, String)>, StorageError> {
    let rows = sqlx::query(
        "SELECT vo_id, trunk_version, branch_number, branch_version, creating_system_id, \
         kind FROM version \
         WHERE contribution_id = $1 \
         UNION \
         SELECT v.vo_id, v.trunk_version, v.branch_number, v.branch_version, \
         v.creating_system_id, v.kind FROM version v \
         JOIN vo_attestation att ON att.vo_id = v.vo_id AND att.sys_version = v.sys_version \
         WHERE att.contribution_id = $1 \
         ORDER BY vo_id",
    )
    .bind(contribution_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get("vo_id")?,
                (
                    row.try_get("trunk_version")?,
                    row.try_get("branch_number")?,
                    row.try_get("branch_version")?,
                ),
                row.try_get("creating_system_id")?,
                row.try_get("kind")?,
            ))
        })
        .collect()
}

/// One row of the EHR CONTRIBUTION-list extension: the contribution's own audit,
/// flattened to the fields the list surface reports.
#[derive(Debug, Clone)]
pub struct ContributionSummary {
    /// The CONTRIBUTION uid.
    pub uid: Uuid,
    /// The server-computed commit instant.
    pub time_committed: jiff::Timestamp,
    /// The committer `PARTY_PROXY`'s `name` (`PARTY_IDENTIFIED` /
    /// `PARTY_RELATED`); `None` for a `PARTY_SELF` committer, which has no name.
    pub committer: Option<String>,
    /// The numeric `audit_change_type` group code of the change set.
    pub change_type: String,
}

/// An EHR's CONTRIBUTIONs, newest-first (audit `time_committed`, then id),
/// flattened to the list-surface fields, paged by `offset`/`limit`.
///
/// This is the storage half of the EHR contribution-list extension.
///
/// No openEHR spec governs the SQL — our own design; the ITS-REST contract
/// defines only the by-uid CONTRIBUTION GET, so the paged list is an extension.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn list_contribution_summaries(
    pool: &PgPool,
    ehr_id: EhrId,
    offset: i64,
    limit: i64,
) -> Result<Vec<ContributionSummary>, StorageError> {
    let rows = sqlx::query(
        "SELECT c.id, a.time_committed, a.change_type, a.committer #>> '{name}' AS committer_name \
         FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
         WHERE c.ehr_id = $1 \
         -- newest-first: this extension is an activity feed (the sibling SM
         -- list_contributions stays oldest-first; a deliberate divergence for
         -- a UI-facing summary — our own extension, no openEHR spec governs it)
         ORDER BY a.time_committed DESC, c.id DESC \
         OFFSET $2 LIMIT $3",
    )
    .bind(ehr_id)
    .bind(offset)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(ContributionSummary {
                uid: row.try_get("id")?,
                time_committed: row
                    .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
                    .to_jiff(),
                committer: row.try_get("committer_name")?,
                change_type: row.try_get("change_type")?,
            })
        })
        .collect()
}

/// The ids of an EHR's CONTRIBUTIONs, oldest-first (audit `time_committed`,
/// then id), within the optional inclusive commit-time window, paged.
///
/// A NULL bound disables that side; a NULL LIMIT returns all rows.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn list_contributions(
    pool: &PgPool,
    ehr_id: EhrId,
    lower: Option<jiff::Timestamp>,
    upper: Option<jiff::Timestamp>,
    offset: i64,
    limit: Option<i64>,
) -> Result<Vec<Uuid>, StorageError> {
    let rows = sqlx::query(
        "SELECT c.id FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
         WHERE c.ehr_id = $1 \
           AND ($2::timestamptz IS NULL OR a.time_committed >= $2::timestamptz) \
           AND ($3::timestamptz IS NULL OR a.time_committed <= $3::timestamptz) \
         ORDER BY a.time_committed, c.id \
         OFFSET $4 LIMIT $5",
    )
    .bind(ehr_id)
    .bind(lower.map(|t| t.to_string()))
    .bind(upper.map(|t| t.to_string()))
    .bind(offset)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(|row| Ok(row.try_get("id")?)).collect()
}

/// The number of an EHR's CONTRIBUTIONs within the optional inclusive
/// commit-time window.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn count_contributions(
    pool: &PgPool,
    ehr_id: EhrId,
    lower: Option<jiff::Timestamp>,
    upper: Option<jiff::Timestamp>,
) -> Result<i64, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
         WHERE c.ehr_id = $1 \
           AND ($2::timestamptz IS NULL OR a.time_committed >= $2::timestamptz) \
           AND ($3::timestamptz IS NULL OR a.time_committed <= $3::timestamptz)",
    )
    .bind(ehr_id)
    .bind(lower.map(|t| t.to_string()))
    .bind(upper.map(|t| t.to_string()))
    .fetch_one(pool)
    .await?)
}

/// The total number of an EHR's CONTRIBUTIONs (the
/// `EHR_SUMMARY.contribution_count` — SM `ehr_summary.adoc`), unwindowed.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn ehr_contribution_count(pool: &PgPool, ehr_id: EhrId) -> Result<i64, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM contribution WHERE ehr_id = $1")
            .bind(ehr_id)
            .fetch_one(pool)
            .await?,
    )
}
