// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O of the physical erasure of an EHR or a party: the rows the
//! cascade leaves behind, the rows it must read before they go, and the
//! erasure tombstone the change-event outbox carries.
//!
//! Every function but the pool reads runs on the erasure's own transaction.
//! No openEHR spec governs the cascade or the foreign-key graph: our own
//! design over the greenfield schema.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// Every EHR id in the repository.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn all_ehr_ids(pool: &PgPool) -> Result<Vec<EhrId>, StorageError> {
    Ok(sqlx::query_scalar("SELECT id FROM ehr")
        .fetch_all(&mut *acquire(pool).await?)
        .await?)
}

/// The commit-audit ids the versions and contributions of `ehr_ids`
/// reference, over both storage tiers, read before the cascade removes the
/// rows that name them (nothing references `ehr` from `commit_audit`).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn ehr_commit_audit_ids(
    tx: &mut PgConnection,
    ehr_ids: &[EhrId],
) -> Result<Vec<Uuid>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT commit_audit_id FROM version WHERE ehr_id = ANY($1) \
         UNION \
         SELECT commit_audit_id FROM contribution WHERE ehr_id = ANY($1)",
    )
    .bind(ehr_ids)
    .fetch_all(tx)
    .await?)
}

/// Delete `ehr_ids`, cascading through the foreign-key graph across both tier
/// partitions, and return the ids that existed.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete_ehrs(
    tx: &mut PgConnection,
    ehr_ids: &[EhrId],
) -> Result<Vec<EhrId>, StorageError> {
    Ok(
        sqlx::query_scalar("DELETE FROM ehr WHERE id = ANY($1) RETURNING id")
            .bind(ehr_ids)
            .fetch_all(tx)
            .await?,
    )
}

/// Delete the commit-audit rows `ids`, which nothing references any more.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete_commit_audits(tx: &mut PgConnection, ids: &[Uuid]) -> Result<(), StorageError> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query("DELETE FROM commit_audit WHERE id = ANY($1)")
        .bind(ids)
        .execute(tx)
        .await?;
    Ok(())
}

/// The externalized blob URIs the versions of `ehr_ids` reference, from the
/// `blob_ref` index the write path maintains.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn ehr_blob_uris(pool: &PgPool, ehr_ids: &[EhrId]) -> Result<Vec<String>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT b.uri FROM blob_ref b \
         JOIN version v ON v.tier = b.tier AND v.vo_id = b.vo_id \
             AND v.sys_version = b.sys_version \
         WHERE v.ehr_id = ANY($1)",
    )
    .bind(ehr_ids)
    .fetch_all(&mut *acquire(pool).await?)
    .await?)
}

/// The externalized blob URIs the versions of `vo_ids` reference.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn object_blob_uris(
    tx: &mut PgConnection,
    vo_ids: &[VoId],
) -> Result<Vec<String>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT DISTINCT uri FROM blob_ref WHERE vo_id = ANY($1)")
            .bind(vo_ids)
            .fetch_all(tx)
            .await?,
    )
}

/// The URIs of `uris` that a stored version of this pool's domain still
/// references.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn referenced_blob_uris(
    pool: &PgPool,
    uris: &[String],
) -> Result<Vec<String>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT DISTINCT uri FROM blob_ref WHERE uri = ANY($1)")
            .bind(uris)
            .fetch_all(&mut *acquire(pool).await?)
            .await?,
    )
}

/// Every `PARTY_RELATIONSHIP` versioned object whose `source` or `target`
/// `PARTY_REF` names `party_id`, in any version.
///
/// The references are data attributes of the relationship's root node, so
/// their `id.value` is matched by a jsonb path extraction.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn relationships_naming(
    tx: &mut PgConnection,
    party_id: VoId,
) -> Result<Vec<VoId>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT n.vo_id FROM node n \
         JOIN version v ON v.vo_id = n.vo_id AND v.sys_version = n.sys_version \
         WHERE v.kind = 'PARTY_RELATIONSHIP' \
           AND (n.data #>> '{source,id,value}' = $1 OR n.data #>> '{target,id,value}' = $1)",
    )
    .bind(party_id.to_string())
    .fetch_all(tx)
    .await?)
}

/// The contribution ids the versions and attestations of `vo_ids` reference.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn object_contribution_ids(
    tx: &mut PgConnection,
    vo_ids: &[VoId],
) -> Result<Vec<Uuid>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT contribution_id FROM version WHERE vo_id = ANY($1) \
         UNION \
         SELECT contribution_id FROM vo_attestation WHERE vo_id = ANY($1)",
    )
    .bind(vo_ids)
    .fetch_all(tx)
    .await?)
}

/// The commit-audit ids the versions of `vo_ids` and the contributions
/// `contribution_ids` reference.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn object_commit_audit_ids(
    tx: &mut PgConnection,
    vo_ids: &[VoId],
    contribution_ids: &[Uuid],
) -> Result<Vec<Uuid>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT commit_audit_id FROM version WHERE vo_id = ANY($1) \
         UNION \
         SELECT commit_audit_id FROM contribution WHERE id = ANY($2)",
    )
    .bind(vo_ids)
    .bind(contribution_ids)
    .fetch_all(tx)
    .await?)
}

/// Delete every version of `vo_ids`, cascading their `node` and
/// `vo_attestation` rows, over both tier partitions.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete_object_versions(
    tx: &mut PgConnection,
    vo_ids: &[VoId],
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM version WHERE vo_id = ANY($1)")
        .bind(vo_ids)
        .execute(tx)
        .await?;
    Ok(())
}

/// Delete the contributions of `ids` that no surviving version or attestation
/// references.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete_orphan_contributions(
    tx: &mut PgConnection,
    ids: &[Uuid],
) -> Result<(), StorageError> {
    sqlx::query(
        "DELETE FROM contribution c WHERE c.id = ANY($1) \
           AND NOT EXISTS (SELECT 1 FROM version v WHERE v.contribution_id = c.id) \
           AND NOT EXISTS (SELECT 1 FROM vo_attestation a WHERE a.contribution_id = c.id)",
    )
    .bind(ids)
    .execute(tx)
    .await?;
    Ok(())
}

/// Delete the commit-audit rows of `ids` that no surviving version or
/// contribution references.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete_orphan_commit_audits(
    tx: &mut PgConnection,
    ids: &[Uuid],
) -> Result<(), StorageError> {
    sqlx::query(
        "DELETE FROM commit_audit a WHERE a.id = ANY($1) \
           AND NOT EXISTS (SELECT 1 FROM version v WHERE v.commit_audit_id = a.id) \
           AND NOT EXISTS (SELECT 1 FROM contribution c WHERE c.commit_audit_id = a.id)",
    )
    .bind(ids)
    .execute(tx)
    .await?;
    Ok(())
}

/// Delete the reverse-relationship index rows naming any of `vo_ids` at
/// either end, and the head rows of `vo_ids`, which carry their archive
/// marker among the rest of their state.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete_object_index_rows(
    tx: &mut PgConnection,
    vo_ids: &[VoId],
) -> Result<(), StorageError> {
    sqlx::query(
        "DELETE FROM party_relationship_target \
         WHERE source_vo_id = ANY($1) OR target_party_id = ANY($1)",
    )
    .bind(vo_ids)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM vo_head WHERE vo_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Append one erasure tombstone to the change-event outbox, holding the outbox
/// lock shared to the transaction's end as every outbox insert does.
///
/// `ehr_id` is the erased EHR, or `None` for a party erasure.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
#[expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the PHI-free event envelope is open \
              operational JSON stored as jsonb (family 6, event rows)"
)]
pub async fn write_erase_tombstone(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    envelope: &serde_json::Value,
) -> Result<(), StorageError> {
    sqlx::query(
        "WITH gate AS (SELECT pg_advisory_xact_lock_shared( \
             hashtext(current_schema() || '.event_outbox'), 0)) \
         INSERT INTO event_outbox (ehr_id, envelope, committed_at) \
         SELECT $1::uuid, $2, now() FROM gate",
    )
    .bind(ehr_id)
    .bind(envelope)
    .execute(tx)
    .await?;
    Ok(())
}

/// Every version of `vo_id` in `sys_version` order with its stored body
/// (`None` on a logical delete), each row locked for the erasure of part of
/// its content.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn locked_version_bodies(
    tx: &mut PgConnection,
    vo_id: VoId,
) -> Result<Vec<(i32, Option<String>)>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT sys_version, body FROM version WHERE vo_id = $1 ORDER BY sys_version FOR UPDATE",
    )
    .bind(vo_id)
    .fetch_all(tx)
    .await?)
}

/// Replaces the stored body of one version of `vo_id` with `body`, its
/// content digest with `digest`, and voids its signature, which was taken
/// over the content the erasure removed part of.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn replace_version_body(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
    body: &str,
    digest: &[u8],
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE version SET body = $3, content_digest = $4, signature = NULL, \
                signature_client_supplied = false \
         WHERE vo_id = $1 AND sys_version = $2",
    )
    .bind(vo_id)
    .bind(sys_version)
    .bind(body)
    .bind(digest)
    .execute(tx)
    .await?;
    Ok(())
}
