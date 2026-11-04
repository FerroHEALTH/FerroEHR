// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The load writes: every archived row back, verbatim, on the load's transaction.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 3): EHR-Extract/dump-load compose over \
              verbatim stored content (RM common master06 §Copying)"
)]

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::ids::EhrId;
use crate::storage::dump_repo::{
    ArchiveRow, AttestationRow, AuditRow, ContributionRow, EhrRow, FolderRankRow, ItemTagRow,
};
use crate::storage::error::StorageError;
use crate::storage::version_repo;

/// The commit-audit insert both waves share, completed by its conflict rule.
const INSERT_AUDITS: &str = "INSERT INTO commit_audit (id, time_committed, system_id, change_type, \
     description, committer, attestation) \
     SELECT t.id, t.time_committed::timestamptz, t.system_id, t.change_type, \
     t.description, t.committer, t.attestation \
     FROM unnest($1::uuid[], $2::text[], $3::text[], $4::text[], $5::jsonb[], \
     $6::jsonb[], $7::jsonb[]) \
     AS t(id, time_committed, system_id, change_type, description, committer, attestation)";

/// [`INSERT_AUDITS`] leaving an id already stored as it is.
const INSERT_AUDITS_KEEPING_STORED: &str = "INSERT INTO commit_audit (id, time_committed, system_id, \
     change_type, description, committer, attestation) \
     SELECT t.id, t.time_committed::timestamptz, t.system_id, t.change_type, \
     t.description, t.committer, t.attestation \
     FROM unnest($1::uuid[], $2::text[], $3::text[], $4::text[], $5::jsonb[], \
     $6::jsonb[], $7::jsonb[]) \
     AS t(id, time_committed, system_id, change_type, description, committer, attestation) \
     ON CONFLICT (id) DO NOTHING";

/// What a commit-audit insert does with an id already stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredAudit {
    /// The insert fails: the EHR wave's audits belong to that EHR alone, so a
    /// duplicate means a corrupt archive.
    Refuse,
    /// The stored row stays: one demographic contribution can commit several
    /// containers, so a partial load may already have written it.
    Keep,
}

/// Insert a record's commit audits in one `unnest` statement, `stored`
/// deciding what an id already stored does.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, a refused duplicate id
/// included.
pub async fn insert_audits(
    tx: &mut PgConnection,
    audits: &[AuditRow],
    stored: StoredAudit,
) -> Result<(), StorageError> {
    if audits.is_empty() {
        return Ok(());
    }
    let sql = match stored {
        StoredAudit::Refuse => INSERT_AUDITS,
        StoredAudit::Keep => INSERT_AUDITS_KEEPING_STORED,
    };
    let ids: Vec<Uuid> = audits.iter().map(|a| a.id).collect();
    let times: Vec<&str> = audits.iter().map(|a| a.time_committed.as_str()).collect();
    let systems: Vec<&str> = audits.iter().map(|a| a.system_id.as_str()).collect();
    let changes: Vec<&str> = audits.iter().map(|a| a.change_type.as_str()).collect();
    let descriptions: Vec<Option<&Value>> = audits.iter().map(|a| a.description.as_ref()).collect();
    let committers: Vec<&Value> = audits.iter().map(|a| &a.committer).collect();
    let attestations: Vec<Option<&Value>> = audits.iter().map(|a| a.attestation.as_ref()).collect();
    sqlx::query(sql)
        .bind(ids)
        .bind(times)
        .bind(systems)
        .bind(changes)
        .bind(descriptions)
        .bind(committers)
        .bind(attestations)
        .execute(tx)
        .await?;
    Ok(())
}

/// Insert contributions in one `unnest` statement: an EHR record's under
/// `Some(ehr_id)`, each new, or the ehr-less ones under `None`, leaving an id
/// already stored as it is.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, a stored id under
/// `Some(ehr_id)` included.
pub async fn insert_contributions(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    contributions: &[ContributionRow],
) -> Result<(), StorageError> {
    if contributions.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = contributions.iter().map(|c| c.id).collect();
    let audits: Vec<Uuid> = contributions.iter().map(|c| c.commit_audit_id).collect();
    let sql = match ehr_id {
        Some(_) => {
            "INSERT INTO contribution (id, ehr_id, commit_audit_id) \
             SELECT t.id, $2, t.commit_audit_id FROM unnest($1::uuid[], $3::uuid[]) \
             AS t(id, commit_audit_id)"
        }
        None => {
            "INSERT INTO contribution (id, ehr_id, commit_audit_id) \
             SELECT t.id, $2, t.commit_audit_id FROM unnest($1::uuid[], $3::uuid[]) \
             AS t(id, commit_audit_id) ON CONFLICT (id) DO NOTHING"
        }
    };
    sqlx::query(sql)
        .bind(ids)
        .bind(ehr_id)
        .bind(audits)
        .execute(tx)
        .await?;
    Ok(())
}

/// Insert the archived `ehr` root row verbatim.
///
/// # Errors
/// [`StorageError::SubjectInUse`] when another EHR already holds the row's
/// subject (`uq_ehr_subject`); [`StorageError::Database`] on any other
/// failure.
pub async fn insert_ehr(tx: &mut PgConnection, ehr: &EhrRow) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO ehr (id, system_id, time_created, subject_id, subject_namespace) \
         VALUES ($1, $2, $3::timestamptz, $4, $5)",
    )
    .bind(ehr.id)
    .bind(&ehr.system_id)
    .bind(&ehr.time_created)
    .bind(&ehr.subject_id)
    .bind(&ehr.subject_namespace)
    .execute(tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.constraint() == Some("uq_ehr_subject") => {
            StorageError::SubjectInUse(
                ehr.subject_id.clone().unwrap_or_else(|| "?".to_owned()),
                ehr.subject_namespace
                    .clone()
                    .unwrap_or_else(|| "?".to_owned()),
            )
        }
        _ => StorageError::Database(e),
    })?;
    Ok(())
}

/// Insert an EHR's `ehr_folder` membership rows verbatim.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn insert_folder_ranks(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    ranks: &[FolderRankRow],
) -> Result<(), StorageError> {
    if ranks.is_empty() {
        return Ok(());
    }
    let rank: Vec<i32> = ranks.iter().map(|f| f.rank).collect();
    let vo_ids: Vec<Uuid> = ranks.iter().map(|f| f.vo_id.0).collect();
    sqlx::query(
        "INSERT INTO ehr_folder (ehr_id, rank, vo_id) \
         SELECT $1, t.rank, t.vo_id FROM unnest($2::int[], $3::uuid[]) AS t(rank, vo_id)",
    )
    .bind(ehr_id)
    .bind(rank)
    .bind(vo_ids)
    .execute(tx)
    .await?;
    Ok(())
}

/// Insert a record's `item_tag` rows verbatim in one `unnest` statement
/// (`ehr_id = None` for the demographic wave's ehr-less tags).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn insert_item_tags(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    tags: &[ItemTagRow],
) -> Result<(), StorageError> {
    if tags.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = tags.iter().map(|t| t.id).collect();
    let targets: Vec<Uuid> = tags.iter().map(|t| t.target_vo_id.0).collect();
    let target_types: Vec<&str> = tags.iter().map(|t| t.target_type.as_str()).collect();
    let keys: Vec<&str> = tags.iter().map(|t| t.key.as_str()).collect();
    let values: Vec<Option<&str>> = tags.iter().map(|t| t.value.as_deref()).collect();
    let paths: Vec<Option<&str>> = tags.iter().map(|t| t.target_path.as_deref()).collect();
    let created: Vec<&str> = tags.iter().map(|t| t.created_at.as_str()).collect();
    sqlx::query(
        "INSERT INTO item_tag (id, ehr_id, target_vo_id, target_type, key, value, \
         target_path, created_at) \
         SELECT t.id, $2, t.target_vo_id, t.target_type, t.key, t.value, t.target_path, \
         t.created_at::timestamptz \
         FROM unnest($1::uuid[], $3::uuid[], $4::text[], $5::text[], $6::text[], $7::text[], \
         $8::text[]) \
         AS t(id, target_vo_id, target_type, key, value, target_path, created_at)",
    )
    .bind(ids)
    .bind(ehr_id)
    .bind(targets)
    .bind(target_types)
    .bind(keys)
    .bind(values)
    .bind(paths)
    .bind(created)
    .execute(tx)
    .await?;
    Ok(())
}

/// Restore an archived object's placement and its marker: the rows move to
/// the cold tier, then the dumped `archived_at`/`archive_reason` are
/// re-stamped on the head rows in one `unnest` statement.
///
/// The order matters. `tier::freeze` stamps `archived_at = now()`, which is
/// the instant of the load rather than the instant the object was archived,
/// so the marker is restored after the move and not before it.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn restore_archives(
    tx: &mut PgConnection,
    archives: &[ArchiveRow],
) -> Result<(), StorageError> {
    if archives.is_empty() {
        return Ok(());
    }
    for archive in archives {
        version_repo::tier::freeze(&mut *tx, &[archive.vo_id], archive.reason.as_deref()).await?;
    }
    let vo_ids: Vec<Uuid> = archives.iter().map(|a| a.vo_id.0).collect();
    let times: Vec<&str> = archives.iter().map(|a| a.archived_at.as_str()).collect();
    let reasons: Vec<Option<&str>> = archives.iter().map(|a| a.reason.as_deref()).collect();
    sqlx::query(
        "UPDATE vo_head h SET archived_at = t.archived_at::timestamptz, archive_reason = t.reason \
         FROM unnest($1::uuid[], $2::text[], $3::text[]) AS t(vo_id, archived_at, reason) \
         WHERE h.vo_id = t.vo_id",
    )
    .bind(vo_ids)
    .bind(times)
    .bind(reasons)
    .execute(tx)
    .await?;
    Ok(())
}

/// Insert the archived `vo_attestation` rows verbatim in one `unnest`
/// statement, once their version and contribution rows exist.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn insert_attestations(
    tx: &mut PgConnection,
    attestations: &[AttestationRow],
) -> Result<(), StorageError> {
    if attestations.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = attestations.iter().map(|a| a.id).collect();
    let vo_ids: Vec<Uuid> = attestations.iter().map(|a| a.vo_id.0).collect();
    let sys_versions: Vec<i32> = attestations.iter().map(|a| a.sys_version).collect();
    let contributions: Vec<Uuid> = attestations.iter().map(|a| a.contribution_id).collect();
    let times: Vec<&str> = attestations
        .iter()
        .map(|a| a.time_committed.as_str())
        .collect();
    let at_committals: Vec<bool> = attestations.iter().map(|a| a.at_committal).collect();
    let datas: Vec<&Value> = attestations.iter().map(|a| &a.data).collect();
    sqlx::query(
        "INSERT INTO vo_attestation (id, vo_id, sys_version, contribution_id, \
         time_committed, at_committal, data) \
         SELECT t.id, t.vo_id, t.sys_version, t.contribution_id, \
         t.time_committed::timestamptz, t.at_committal, t.data \
         FROM unnest($1::uuid[], $2::uuid[], $3::int[], $4::uuid[], $5::text[], $6::bool[], \
         $7::jsonb[]) \
         AS t(id, vo_id, sys_version, contribution_id, time_committed, at_committal, data)",
    )
    .bind(ids)
    .bind(vo_ids)
    .bind(sys_versions)
    .bind(contributions)
    .bind(times)
    .bind(at_committals)
    .bind(datas)
    .execute(tx)
    .await?;
    Ok(())
}

/// Whether two versions of one lineage of `ehr_id` carry the same commit
/// instant, which leaves "the version in force then" unanswerable (RM common
/// master06: one valid version per lineage at any instant).
///
/// Trunk rows are one lineage per `vo_id`; branch rows one per versioned
/// object, creating system, fork point and branch number.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn ambiguous_commit_instant(
    tx: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM version a \
             JOIN version b ON a.vo_id = b.vo_id \
                 AND a.branch_number = b.branch_number \
                 AND (a.branch_number = 0 \
                      OR (a.creating_system_id = b.creating_system_id \
                          AND a.trunk_version = b.trunk_version)) \
                 AND a.sys_version < b.sys_version \
                 AND a.committed_at = b.committed_at \
             WHERE a.ehr_id = $1)",
    )
    .bind(ehr_id)
    .fetch_one(tx)
    .await?)
}
