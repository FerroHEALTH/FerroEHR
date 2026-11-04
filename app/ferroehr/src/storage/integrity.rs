// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads of the storage-integrity sweep and the node rebuild: a keyset
//! page of version identifiers, a chunk of stored bodies, and one body under
//! its row lock.
//!
//! No openEHR spec governs storage mechanics: our own design/extension.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// Which stored versions a sweep covers.
///
/// A sweep reads every byte of the versions it covers, so an operator
/// verifying one record, one object, or everything committed since an
/// incident, should not have to read the whole repository to do it. Every
/// bound is optional and they compose; the default covers everything.
///
/// No openEHR spec governs storage mechanics — our own design/extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageParityScope {
    /// Cover only versions belonging to this EHR.
    pub ehr_id: Option<Uuid>,
    /// Cover only versions whose validity begins at or after this instant.
    pub committed_since: Option<jiff::Timestamp>,
    /// Cover only versions of this versioned object.
    pub vo_id: Option<Uuid>,
    /// Cover only this version of it. Ignored without [`Self::vo_id`], which
    /// is what makes a `sys_version` alone meaningless: the ordinal is
    /// per-object, so on its own it names one version of every object.
    pub sys_version: Option<i32>,
}

/// One page of at most `limit` stored versions of `scope` after `cursor`.
///
/// The page is in `(vo_id, sys_version)` order, each row as `(vo_id,
/// sys_version, kind, has_body)`: a keyset walk, so a long sweep never pays a
/// growing `OFFSET`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn parity_page(
    pool: &PgPool,
    cursor: Option<(Uuid, i32)>,
    limit: i64,
    scope: StorageParityScope,
) -> Result<Vec<(Uuid, i32, String, bool)>, StorageError> {
    let (after_vo, after_version) = match cursor {
        Some((vo_id, sys_version)) => (Some(vo_id), Some(sys_version)),
        None => (None, None),
    };
    Ok(sqlx::query_as(
        "SELECT vo_id, sys_version, kind, body IS NOT NULL \
         FROM version \
         WHERE ($1::uuid IS NULL OR (vo_id, sys_version) > ($1::uuid, $2::int)) \
           AND ($4::uuid IS NULL OR ehr_id = $4::uuid) \
           AND ($5::timestamptz IS NULL OR committed_at >= $5::timestamptz) \
           AND ($6::uuid IS NULL OR vo_id = $6::uuid) \
           AND ($6::uuid IS NULL OR $7::int IS NULL OR sys_version = $7::int) \
         ORDER BY vo_id, sys_version \
         LIMIT $3",
    )
    .bind(after_vo)
    .bind(after_version)
    .bind(limit)
    .bind(scope.ehr_id)
    .bind(scope.committed_since.map(jiff_sqlx::Timestamp::from))
    .bind(scope.vo_id)
    .bind(scope.sys_version)
    .fetch_all(&mut *acquire(pool).await?)
    .await?)
}

/// The stored bodies of the `(vo_ids[i], sys_versions[i])` versions in one
/// statement, as `(vo_id, sys_version, body)`; a version with no body, or one
/// gone since its page was read, is absent.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn stored_bodies(
    pool: &PgPool,
    vo_ids: &[Uuid],
    sys_versions: &[i32],
) -> Result<Vec<(Uuid, i32, String)>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT v.vo_id, v.sys_version, v.body \
         FROM unnest($1::uuid[], $2::int[]) AS k(vo_id, sys_version) \
         JOIN version v \
           ON v.vo_id = k.vo_id AND v.sys_version = k.sys_version \
         WHERE v.body IS NOT NULL",
    )
    .bind(vo_ids)
    .bind(sys_versions)
    .fetch_all(&mut *acquire(pool).await?)
    .await?)
}

/// One primary-tier version as the node rebuild reads it under its row lock:
/// the owning EHR, the stored body, and the identity the root row's fragment
/// is slimmed against.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct LockedVersion {
    /// The owning EHR, `None` for a demographic party.
    pub ehr_id: Option<EhrId>,
    /// The stored canonical body, `None` for a logical delete (data Void, RM
    /// common `master06-change_control_package.adoc` §Logical Deletion).
    pub body: Option<String>,
    /// The version's `OBJECT_VERSION_ID` text (`ext.version_uid`).
    pub version_uid: String,
    /// `version.template_id`.
    pub template_id: Option<String>,
}

/// One primary-tier version, read under its row lock.
///
/// The lock keeps a concurrent commit on the same version from interleaving
/// with the replacement of its rows. `None` when no such version exists.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn locked_body(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
) -> Result<Option<LockedVersion>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT ehr_id, body, \
                ext.version_uid(vo_id, creating_system_id, trunk_version, branch_number, \
                                branch_version) AS version_uid, \
                template_id \
         FROM version WHERE vo_id = $1 AND sys_version = $2 FOR UPDATE",
    )
    .bind(vo_id)
    .bind(sys_version)
    .fetch_optional(tx)
    .await?)
}
