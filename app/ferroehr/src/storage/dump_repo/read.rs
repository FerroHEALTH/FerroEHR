// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The dump reads: every row an archive carries, per EHR or per container.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 3): EHR-Extract/dump-load compose over \
              verbatim stored content (RM common master06 §Copying)"
)]

use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row as _};

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::dump_repo::{
    ArchiveRow, AttestationRow, AuditRow, ContributionRow, EhrRow, FolderRankRow, ItemTagRow,
    Scope, VersionRecord,
};
use crate::storage::error::StorageError;

fn audit_row(r: &PgRow) -> Result<AuditRow, StorageError> {
    Ok(AuditRow {
        id: r.try_get("id")?,
        time_committed: r
            .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
            .to_jiff()
            .to_string(),
        system_id: r.try_get("system_id")?,
        change_type: r.try_get("change_type")?,
        description: r.try_get("description")?,
        committer: r.try_get("committer")?,
        attestation: r.try_get("attestation")?,
    })
}

fn contribution_row(r: &PgRow) -> Result<ContributionRow, StorageError> {
    Ok(ContributionRow {
        id: r.try_get("id")?,
        commit_audit_id: r.try_get("commit_audit_id")?,
    })
}

fn version_record(r: &PgRow) -> Result<VersionRecord, StorageError> {
    Ok(VersionRecord {
        vo_id: r.try_get("vo_id")?,
        kind: r.try_get("kind")?,
        sys_version: r.try_get("sys_version")?,
        trunk_version: r.try_get("trunk_version")?,
        branch_number: r.try_get("branch_number")?,
        branch_version: r.try_get("branch_version")?,
        preceding_version_uid: r.try_get("preceding_version_uid")?,
        other_input_version_uids: r.try_get("other_input_version_uids")?,
        committed_at: r.try_get("ca")?,
        lifecycle_state: r.try_get("lifecycle_state")?,
        contribution_id: r.try_get("contribution_id")?,
        commit_audit_id: r.try_get("commit_audit_id")?,
        template_id: r.try_get("template_id")?,
        signature: r.try_get("signature")?,
        signature_client_supplied: r.try_get("signature_client_supplied")?,
        creating_system_id: r.try_get("creating_system_id")?,
        wrapped_original: r.try_get("wrapped_original")?,
        body: Value::Null,
        body_entry: None,
    })
}

fn attestation_row(r: &PgRow) -> Result<AttestationRow, StorageError> {
    Ok(AttestationRow {
        id: r.try_get("id")?,
        vo_id: r.try_get("vo_id")?,
        sys_version: r.try_get("sys_version")?,
        contribution_id: r.try_get("contribution_id")?,
        time_committed: r
            .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
            .to_jiff()
            .to_string(),
        at_committal: r.try_get("at_committal")?,
        data: r.try_get("data")?,
    })
}

fn item_tag_row(r: &PgRow) -> Result<ItemTagRow, StorageError> {
    Ok(ItemTagRow {
        id: r.try_get("id")?,
        target_vo_id: r.try_get("target_vo_id")?,
        target_type: r.try_get("target_type")?,
        key: r.try_get("key")?,
        value: r.try_get("value")?,
        target_path: r.try_get("target_path")?,
        created_at: r.try_get("created_at")?,
    })
}

fn archive_row(r: &PgRow) -> Result<ArchiveRow, StorageError> {
    Ok(ArchiveRow {
        vo_id: r.try_get("vo_id")?,
        archived_at: r.try_get("archived_at")?,
        reason: r.try_get("reason")?,
    })
}

/// Decode every row with `decode`.
fn decode_all<T>(
    rows: &[PgRow],
    decode: fn(&PgRow) -> Result<T, StorageError>,
) -> Result<Vec<T>, StorageError> {
    rows.iter().map(decode).collect()
}

// ── reads ────────────────────────────────────────────────────────────────────

/// Every EHR id, in id order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn ehr_ids(pool: &PgPool) -> Result<Vec<EhrId>, StorageError> {
    Ok(sqlx::query_scalar("SELECT id FROM ehr ORDER BY id")
        .fetch_all(&mut *acquire(pool).await?)
        .await?)
}

/// The `ehr` row of `ehr_id`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, including no such row.
pub async fn ehr_row(pool: &PgPool, ehr_id: EhrId) -> Result<EhrRow, StorageError> {
    let row = sqlx::query(
        "SELECT system_id, time_created::text, subject_id, subject_namespace \
         FROM ehr WHERE id = $1",
    )
    .bind(ehr_id)
    .fetch_one(&mut *acquire(pool).await?)
    .await?;
    Ok(EhrRow {
        id: ehr_id,
        system_id: row.try_get("system_id")?,
        time_created: row.try_get("time_created")?,
        subject_id: row.try_get("subject_id")?,
        subject_namespace: row.try_get("subject_namespace")?,
    })
}

/// The ehr-less demographic versioned objects with their kind, in id order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn demographic_containers(pool: &PgPool) -> Result<Vec<(VoId, String)>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT DISTINCT vo_id, kind FROM version WHERE ehr_id IS NULL \
         ORDER BY vo_id",
    )
    .fetch_all(&mut *acquire(pool).await?)
    .await?)
}

/// Whether an ehr-less demographic container with `vo_id` already exists.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn demographic_container_exists(
    pool: &PgPool,
    vo_id: VoId,
) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM version WHERE vo_id = $1 AND ehr_id IS NULL)",
    )
    .bind(vo_id)
    .fetch_one(&mut *acquire(pool).await?)
    .await?)
}

/// Every commit audit `scope` references (an EHR's contributions and
/// versions, a container's versions), in id order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn audits(pool: &PgPool, scope: Scope) -> Result<Vec<AuditRow>, StorageError> {
    let query = match scope {
        Scope::Ehr(ehr_id) => sqlx::query(
            "SELECT id, time_committed, system_id, change_type, \
             description, committer, attestation FROM commit_audit \
             WHERE id IN (SELECT commit_audit_id FROM contribution WHERE ehr_id = $1 \
                          UNION SELECT commit_audit_id FROM version WHERE ehr_id = $1) \
             ORDER BY id",
        )
        .bind(ehr_id),
        Scope::Container(vo_id) => sqlx::query(
            "SELECT id, time_committed, system_id, change_type, \
             description, committer, attestation FROM commit_audit \
             WHERE id IN (SELECT commit_audit_id FROM version \
                          WHERE vo_id = $1 AND ehr_id IS NULL) \
             ORDER BY id",
        )
        .bind(vo_id),
    };
    let rows = query.fetch_all(&mut *acquire(pool).await?).await?;
    decode_all(&rows, audit_row)
}

/// Every commit audit an ehr-less contribution references, in id order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn demographic_contribution_audits(pool: &PgPool) -> Result<Vec<AuditRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, time_committed, system_id, change_type, \
         description, committer, attestation FROM commit_audit \
         WHERE id IN (SELECT commit_audit_id FROM contribution WHERE ehr_id IS NULL) \
         ORDER BY id",
    )
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    decode_all(&rows, audit_row)
}

/// The contributions of `ehr_id`, or every ehr-less contribution for `None`,
/// in id order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn contributions(
    pool: &PgPool,
    ehr_id: Option<EhrId>,
) -> Result<Vec<ContributionRow>, StorageError> {
    let query = match ehr_id {
        Some(ehr_id) => sqlx::query(
            "SELECT id, commit_audit_id FROM contribution WHERE ehr_id = $1 ORDER BY id",
        )
        .bind(ehr_id),
        None => sqlx::query(
            "SELECT id, commit_audit_id FROM contribution WHERE ehr_id IS NULL ORDER BY id",
        ),
    };
    let rows = query.fetch_all(&mut *acquire(pool).await?).await?;
    decode_all(&rows, contribution_row)
}

/// The version rows of `scope`, ordered by object and version, each with a
/// `Null` body.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn versions(pool: &PgPool, scope: Scope) -> Result<Vec<VersionRecord>, StorageError> {
    let query = match scope {
        Scope::Ehr(ehr_id) => sqlx::query(
            "SELECT vo_id, kind, sys_version, trunk_version, branch_number, branch_version, \
             preceding_version_uid, other_input_version_uids, committed_at::text AS ca, \
             lifecycle_state, contribution_id, commit_audit_id, \
             template_id, signature, signature_client_supplied, creating_system_id, \
             wrapped_original \
             FROM version WHERE ehr_id = $1 ORDER BY vo_id, sys_version",
        )
        .bind(ehr_id),
        Scope::Container(vo_id) => sqlx::query(
            "SELECT vo_id, kind, sys_version, trunk_version, branch_number, branch_version, \
             preceding_version_uid, other_input_version_uids, committed_at::text AS ca, \
             lifecycle_state, contribution_id, commit_audit_id, \
             template_id, signature, signature_client_supplied, creating_system_id, \
             wrapped_original \
             FROM version WHERE vo_id = $1 AND ehr_id IS NULL \
             ORDER BY sys_version",
        )
        .bind(vo_id),
    };
    let rows = query.fetch_all(&mut *acquire(pool).await?).await?;
    decode_all(&rows, version_record)
}

/// The `vo_attestation` rows of `scope`'s versioned objects, in commit order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn attestations(
    pool: &PgPool,
    scope: Scope,
) -> Result<Vec<AttestationRow>, StorageError> {
    let query = match scope {
        Scope::Ehr(ehr_id) => sqlx::query(
            "SELECT id, vo_id, sys_version, contribution_id, time_committed, at_committal, \
             data FROM vo_attestation \
             WHERE vo_id IN (SELECT DISTINCT vo_id FROM version WHERE ehr_id = $1) \
             ORDER BY vo_id, sys_version, time_committed, id",
        )
        .bind(ehr_id),
        Scope::Container(vo_id) => sqlx::query(
            "SELECT id, vo_id, sys_version, contribution_id, time_committed, at_committal, \
             data FROM vo_attestation WHERE vo_id = $1 \
             ORDER BY sys_version, time_committed, id",
        )
        .bind(vo_id),
    };
    let rows = query.fetch_all(&mut *acquire(pool).await?).await?;
    decode_all(&rows, attestation_row)
}

/// The `ehr_folder` membership rows of `ehr_id`, in rank order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn folder_ranks(
    pool: &PgPool,
    ehr_id: EhrId,
) -> Result<Vec<FolderRankRow>, StorageError> {
    let rows = sqlx::query("SELECT rank, vo_id FROM ehr_folder WHERE ehr_id = $1 ORDER BY rank")
        .bind(ehr_id)
        .fetch_all(&mut *acquire(pool).await?)
        .await?;
    rows.iter()
        .map(|r| {
            Ok(FolderRankRow {
                rank: r.try_get("rank")?,
                vo_id: r.try_get("vo_id")?,
            })
        })
        .collect()
}

/// The `item_tag` rows of `scope`, in id order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn item_tags(pool: &PgPool, scope: Scope) -> Result<Vec<ItemTagRow>, StorageError> {
    let query = match scope {
        Scope::Ehr(ehr_id) => sqlx::query(
            "SELECT id, target_vo_id, target_type, key, value, target_path, \
             created_at::text AS created_at FROM item_tag WHERE ehr_id = $1 ORDER BY id",
        )
        .bind(ehr_id),
        Scope::Container(vo_id) => sqlx::query(
            "SELECT id, target_vo_id, target_type, key, value, target_path, \
             created_at::text AS created_at FROM item_tag \
             WHERE ehr_id IS NULL AND target_vo_id = $1 ORDER BY id",
        )
        .bind(vo_id),
    };
    let rows = query.fetch_all(&mut *acquire(pool).await?).await?;
    decode_all(&rows, item_tag_row)
}

/// The archive markers of `scope`'s head rows, in object order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn archives(pool: &PgPool, scope: Scope) -> Result<Vec<ArchiveRow>, StorageError> {
    let query = match scope {
        Scope::Ehr(ehr_id) => sqlx::query(
            "SELECT vo_id, archived_at::text AS archived_at, archive_reason AS reason \
             FROM vo_head WHERE ehr_id = $1 AND archived_at IS NOT NULL ORDER BY vo_id",
        )
        .bind(ehr_id),
        Scope::Container(vo_id) => sqlx::query(
            "SELECT vo_id, archived_at::text AS archived_at, archive_reason AS reason \
             FROM vo_head WHERE vo_id = $1 AND archived_at IS NOT NULL ORDER BY vo_id",
        )
        .bind(vo_id),
    };
    let rows = query.fetch_all(&mut *acquire(pool).await?).await?;
    decode_all(&rows, archive_row)
}

/// The externalized blob URIs the `(vo_id, sys_version)` versions reference.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn version_blob_uris(
    pool: &PgPool,
    vo_ids: &[VoId],
    sys_versions: &[i32],
) -> Result<Vec<String>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT b.uri FROM blob_ref b \
         JOIN unnest($1::uuid[], $2::int[]) AS t(vo_id, sys_version) \
           ON t.vo_id = b.vo_id AND t.sys_version = b.sys_version",
    )
    .bind(vo_ids)
    .bind(sys_versions)
    .fetch_all(&mut *acquire(pool).await?)
    .await?)
}
