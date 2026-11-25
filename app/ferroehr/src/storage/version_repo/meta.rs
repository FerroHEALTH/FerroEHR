// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lean metadata-only reads over `version`.
//!
//! No openEHR spec governs the SQL — our own design. The version identity these
//! reads serve is RM common master06 §Version Identification and the commit
//! instant is §Committal and Audits.
//!
//! NOTE: no openEHR spec governs storage tiering — our own design; `version`
//! is partitioned by tier, so a lookup that names no tier reads both partitions
//! in one statement and one that pins `tier = 'hot'` prunes the cold partition
//! at plan time.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

// ── the rendered statements ──────────────────────────────────────────────────
// Two of the reads below expose the statement they run as a named constant, so
// the plan-shape suite (`tests/it/storage_plans.rs`) explains the statement the
// read path emits rather than a copy of it.

/// The statement [`all_version_meta`] runs: every version row of one object in
/// storage-ordinal order — the revision history.
pub const ALL_VERSION_META_SQL: &str = concat!(
    "SELECT v.ehr_id, v.kind, v.sys_version, v.trunk_version, v.branch_number, ",
    "v.branch_version, v.creating_system_id, v.lifecycle_state, ",
    "a.system_id, a.change_type, a.description, a.committer, a.attestation, ",
    "a.time_committed, att.attestations, ",
    "h.restricted_at IS NOT NULL AS restricted ",
    "FROM version v JOIN vo_head h ON h.vo_id = v.vo_id ",
    audit_of_version!(),
    "LEFT JOIN LATERAL ( ",
    "SELECT coalesce(jsonb_agg(x.data ORDER BY x.time_committed, x.id), ",
    "'[]'::jsonb) AS attestations ",
    "FROM vo_attestation x ",
    "WHERE x.vo_id = v.vo_id AND x.sys_version = v.sys_version ",
    ") att ON true ",
    "WHERE v.vo_id = $1 ORDER BY v.sys_version"
);

/// The statement [`current_version_meta_scoped`] runs: the `ETag`/`If-Match`
/// identity of one EHR-owned object's current trunk version.
pub const CURRENT_VERSION_META_SCOPED_SQL: &str = "SELECT v.vo_id, v.trunk_version, v.branch_number, v.branch_version, \
     v.creating_system_id, a.time_committed \
     FROM version v JOIN commit_audit_facts a ON a.id = v.commit_audit_id \
     JOIN vo_head h ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
     WHERE v.vo_id = $1 AND v.ehr_id = $2";

// ── revision-history enumeration ──────────────────────────────────────────────

/// One version's metadata row (`version` ⋈ `audit`).
#[derive(Debug, Clone)]
pub struct VersionMeta {
    /// The owning EHR, or `None` for a demographic versioned object.
    pub ehr_id: Option<EhrId>,
    /// The `version.kind` discriminator text.
    pub kind: String,
    /// The per-object storage commit ordinal — NOT the wire version number.
    pub sys_version: i32,
    /// `VERSION_TREE_ID` first part.
    pub trunk_version: i32,
    /// `VERSION_TREE_ID` second part; `0` on a trunk row.
    pub branch_number: i32,
    /// `VERSION_TREE_ID` third part; `0` on a trunk row.
    pub branch_version: i32,
    /// The version's immutable `creating_system_id`.
    pub creating_system_id: String,
    /// The `version_lifecycle_state` numeric code.
    pub lifecycle_state: String,
    /// The version audit's `system_id`.
    pub audit_system_id: String,
    /// The version audit's numeric `audit_change_type` group code.
    pub audit_change_type: String,
    /// The canonical `DV_TEXT` fragment of the version audit's description,
    /// when the committer supplied one.
    pub audit_description: Option<Value>,
    /// The canonical `PARTY_PROXY` JSON of the committer.
    pub audit_committer: Value,
    /// The canonical fragment of the `ATTESTATION`-declared attributes when the
    /// commit audit is an `ATTESTATION` (RM common master06 §Attestation).
    pub audit_attestation: Option<Value>,
    /// The audit's server-computed commit instant.
    pub time_committed: jiff::Timestamp,
    /// The version's `ATTESTATION` fragments in commit order (RM common
    /// master06 §Attestation), folded into the meta row so a revision history
    /// never issues a second attestation query; `[]` in the common case.
    pub attestations: Vec<Value>,
    /// Whether the versioned object carries a restriction-of-processing mark
    /// (`vo_head.restricted_at`). The same fact on every row of one object;
    /// carried here so the revision-history read needs no second statement to
    /// refuse a restricted object. Our own design/extension.
    pub restricted: bool,
}

/// All version metadata rows of an object, ordered by storage ordinal.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn all_version_meta(
    pool: &PgPool,
    vo_id: VoId,
) -> Result<Vec<VersionMeta>, StorageError> {
    let rows = sqlx::query(ALL_VERSION_META_SQL)
        .bind(vo_id)
        .fetch_all(&mut *acquire(pool).await?)
        .await?;
    rows.iter()
        .map(|row| {
            Ok(VersionMeta {
                ehr_id: row.try_get("ehr_id")?,
                kind: row.try_get("kind")?,
                sys_version: row.try_get("sys_version")?,
                trunk_version: row.try_get("trunk_version")?,
                branch_number: row.try_get("branch_number")?,
                branch_version: row.try_get("branch_version")?,
                creating_system_id: row.try_get("creating_system_id")?,
                lifecycle_state: row.try_get("lifecycle_state")?,
                audit_system_id: row.try_get("system_id")?,
                audit_change_type: row.try_get("change_type")?,
                audit_description: row.try_get("description")?,
                audit_committer: row.try_get("committer")?,
                audit_attestation: row.try_get("attestation")?,
                time_committed: row
                    .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
                    .to_jiff(),
                attestations: row
                    .try_get::<Value, _>("attestations")?
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
                restricted: row.try_get("restricted")?,
            })
        })
        .collect()
}

/// The commit time of the earliest **held** version of an object
/// (`VERSIONED_OBJECT.time_created`; a latest-only import clone legitimately
/// starts above version 1). `None` when the object does not exist.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn time_created(
    pool: &PgPool,
    vo_id: VoId,
) -> Result<Option<jiff::Timestamp>, StorageError> {
    const SQL: &str = "SELECT a.time_committed FROM version v JOIN commit_audit_facts a ON a.id = v.commit_audit_id \
         WHERE v.vo_id = $1 ORDER BY v.sys_version LIMIT 1";
    let stamp = sqlx::query_scalar::<_, jiff_sqlx::Timestamp>(SQL)
        .bind(vo_id)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?;
    Ok(stamp.map(jiff_sqlx::Timestamp::to_jiff))
}

/// The owning EHR and the commit instants bounding an object's held
/// versions, in ONE read.
///
/// The earliest instant is `VERSIONED_OBJECT.time_created` (RM common
/// master06 §Versioned Objects; earliest **held**, so a latest-only import
/// clone starts above version 1) and the latest is the container resource's
/// last-modified instant (ITS-REST overview `Requests_and_responses.md`
/// §"`ETag` and Last-Modified" derives `Last-Modified` from
/// `VERSION.commit_audit.time_committed.value`, and for a `VERSIONED_OBJECT`
/// response the newest held version carries that instant). The owner is
/// `None` for an ehr-less demographic object; the whole result is `None`
/// when the object does not exist.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn commit_bounds(
    pool: &PgPool,
    vo_id: VoId,
) -> Result<Option<(Option<EhrId>, jiff::Timestamp, jiff::Timestamp)>, StorageError> {
    // The owner rides the same aggregate row (constant across an object's
    // versions), so a container read never pays a separate ownership probe.
    const SQL: &str = "SELECT (array_agg(v.ehr_id))[1] AS ehr_id, \
                       min(a.time_committed) AS created, \
                       max(a.time_committed) AS modified \
                       FROM version v JOIN commit_audit_facts a ON a.id = v.commit_audit_id WHERE v.vo_id = $1";
    let row = sqlx::query(SQL)
        .bind(vo_id)
        .fetch_one(&mut *acquire(pool).await?)
        .await?;
    let owner: Option<EhrId> = row.try_get("ehr_id")?;
    let created: Option<jiff_sqlx::Timestamp> = row.try_get("created")?;
    let modified: Option<jiff_sqlx::Timestamp> = row.try_get("modified")?;
    Ok(created
        .zip(modified)
        .map(|(c, m)| (owner, c.to_jiff(), m.to_jiff())))
}

/// The stored `template_id` of one version of an object — the current open
/// trunk version when `tree` is `None`, else the addressed `VERSION_TREE_ID`.
///
/// Outer `None`: no such version; inner `None`: the version carries no template
/// (non-COMPOSITION).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn template_id_of(
    pool: &PgPool,
    vo_id: VoId,
    tree: Option<(i32, i32, i32)>,
) -> Result<Option<Option<String>>, StorageError> {
    const BY_TREE_SQL: &str = "SELECT template_id FROM version WHERE vo_id = $1 \
                               AND trunk_version = $2 AND branch_number = $3 \
                               AND branch_version = $4";
    const CURRENT_SQL: &str = "SELECT version.template_id FROM version \
                               JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
                               WHERE version.vo_id = $1";
    if let Some((trunk, branch, branch_version)) = tree {
        return Ok(sqlx::query_scalar(BY_TREE_SQL)
            .bind(vo_id)
            .bind(trunk)
            .bind(branch)
            .bind(branch_version)
            .fetch_optional(&mut *acquire(pool).await?)
            .await?);
    }
    Ok(sqlx::query_scalar(CURRENT_SQL)
        .bind(vo_id)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?)
}

// ── existence / kind / ownership ──────────────────────────────────────────────

/// The kind text of the current version of an object, or `None` if it does not
/// exist. Mapping to a versioned-object kind is the caller's.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn object_kind(pool: &PgPool, vo_id: VoId) -> Result<Option<String>, StorageError> {
    const SQL: &str = "SELECT version.kind FROM version \
                       JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
                       WHERE version.vo_id = $1";
    Ok(sqlx::query_scalar(SQL)
        .bind(vo_id)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?)
}

/// The kind text of the current version of EVERY object in `vo_ids`.
///
/// One round trip (the CONTRIBUTION-commit target pre-check batches its
/// per-version lookups — a K-change commit reads its targets with one
/// statement). Absent objects are simply missing from the result; the caller
/// maps absence to its 400. No openEHR spec governs read batching — our own
/// design.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn object_kinds(
    pool: &PgPool,
    vo_ids: &[VoId],
) -> Result<Vec<(VoId, String)>, StorageError> {
    const SQL: &str = "SELECT version.vo_id, version.kind FROM version \
                       JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
                       WHERE version.vo_id = ANY($1)";
    if vo_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(SQL)
        .bind(vo_ids)
        .fetch_all(&mut *acquire(pool).await?)
        .await?;
    rows.iter()
        .map(|r| Ok((r.try_get("vo_id")?, r.try_get("kind")?)))
        .collect::<Result<_, StorageError>>()
}

/// The owning EHR of a versioned object (`None` when the object does not
/// exist; `Some(None)` for an ehr-less demographic object).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn vo_owner(pool: &PgPool, vo_id: VoId) -> Result<Option<Option<EhrId>>, StorageError> {
    const SQL: &str = "SELECT ehr_id FROM version WHERE vo_id = $1 LIMIT 1";
    Ok(sqlx::query_scalar(SQL)
        .bind(vo_id)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?)
}

/// A versioned object's owning EHR **and its RM kind**.
///
/// The owning EHR is `None` for a demographic object; `None` when no such
/// versioned object exists.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn vo_owner_kind(
    pool: &PgPool,
    vo_id: VoId,
) -> Result<Option<(Option<EhrId>, String)>, StorageError> {
    const SQL: &str = "SELECT ehr_id, kind FROM version WHERE vo_id = $1 LIMIT 1";
    Ok(sqlx::query_as(SQL)
        .bind(vo_id)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?)
}

/// Whether a specific VERSION of a versioned object exists, addressed by its
/// `VERSION_TREE_ID` parts (trunk / branch pair, branch 0 0 = trunk row).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn version_exists(
    pool: &PgPool,
    vo_id: VoId,
    trunk: i32,
    branch_number: i32,
    branch_version: i32,
) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(EXISTS_SQL)
        .bind(vo_id)
        .bind(trunk)
        .bind(branch_number)
        .bind(branch_version)
        .fetch_one(&mut *acquire(pool).await?)
        .await?)
}

/// The version-addressed existence probe (both tiers: the statement names none,
/// so neither partition is pruned).
const EXISTS_SQL: &str = "SELECT EXISTS(SELECT 1 FROM version WHERE vo_id = $1 \
                          AND trunk_version = $2 AND branch_number = $3 \
                          AND branch_version = $4)";

// ── current-version identity reads ────────────────────────────────────────────

/// The current version of an EHR-owned object of one kind: its `vo_id` and
/// `VERSION_TREE_ID` column ints, from the current open trunk row
/// (the head row's `trunk_head_sys_version`).
#[derive(Debug, Clone)]
pub struct CurrentVoRow {
    /// The versioned object's id.
    pub vo_id: VoId,
    /// `VERSION_TREE_ID` first part.
    pub trunk_version: i32,
    /// `VERSION_TREE_ID` second part; `0` on a trunk row.
    pub branch_number: i32,
    /// `VERSION_TREE_ID` third part; `0` on a trunk row.
    pub branch_version: i32,
}

/// Read the current-version `(vo_id, tree)` of an EHR's object of `kind` on
/// `conn`.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn current_vo(
    conn: &mut sqlx::PgConnection,
    ehr_id: EhrId,
    kind: &str,
) -> Result<Option<CurrentVoRow>, StorageError> {
    const SQL: &str = "SELECT version.vo_id, version.trunk_version, version.branch_number, \
                       version.branch_version \
                       FROM version \
                       JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
                       WHERE version.ehr_id = $1 AND version.kind = $2";
    let found = sqlx::query(SQL)
        .bind(ehr_id)
        .bind(kind)
        .fetch_optional(conn)
        .await?;
    let Some(row) = found else {
        return Ok(None);
    };
    Ok(Some(CurrentVoRow {
        vo_id: row.try_get("vo_id")?,
        trunk_version: row.try_get("trunk_version")?,
        branch_number: row.try_get("branch_number")?,
        branch_version: row.try_get("branch_version")?,
    }))
}

/// The current version's metadata only.
///
/// The `version`⋈`audit` columns the `ETag`/`If-Match`
/// full-`OBJECT_VERSION_ID` compare needs (`VERSION_TREE_ID` column ints + the
/// stored per-version `creating_system_id` + the audit `time_committed`),
/// **without** node reassembly or the attestation read the
/// full [`crate::storage::version_repo::read::read_one`] pays. `None` when the
/// object has no current trunk version. The version identity is RM common
/// master06 §Version Identification; the commit instant is master06 §Committal
/// and Audits.
#[derive(Debug, Clone)]
pub struct CurrentMeta {
    /// The versioned object's id.
    pub vo_id: VoId,
    /// `VERSION_TREE_ID` first part.
    pub trunk_version: i32,
    /// `VERSION_TREE_ID` second part; `0` on a trunk row.
    pub branch_number: i32,
    /// `VERSION_TREE_ID` third part; `0` on a trunk row.
    pub branch_version: i32,
    /// The version's immutable `creating_system_id`.
    pub creating_system_id: String,
    /// The audit's server-computed commit instant.
    pub time_committed: jiff::Timestamp,
}

fn current_meta_row(row: &PgRow) -> Result<CurrentMeta, StorageError> {
    Ok(CurrentMeta {
        vo_id: row.try_get("vo_id")?,
        trunk_version: row.try_get("trunk_version")?,
        branch_number: row.try_get("branch_number")?,
        branch_version: row.try_get("branch_version")?,
        creating_system_id: row.try_get("creating_system_id")?,
        time_committed: row
            .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
            .to_jiff(),
    })
}

/// The current trunk version's metadata for an EHR's object of `kind`.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn current_version_meta_by_kind(
    pool: &PgPool,
    ehr_id: EhrId,
    kind: &str,
) -> Result<Option<CurrentMeta>, StorageError> {
    const SQL: &str = "SELECT v.vo_id, v.trunk_version, v.branch_number, v.branch_version, \
                       v.creating_system_id, a.time_committed \
                       FROM version v JOIN commit_audit_facts a ON a.id = v.commit_audit_id \
                       JOIN vo_head h ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
                       WHERE v.ehr_id = $1 AND v.kind = $2";
    let found = sqlx::query(SQL)
        .bind(ehr_id)
        .bind(kind)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?;
    let Some(row) = found else {
        return Ok(None);
    };
    Ok(Some(current_meta_row(&row)?))
}

/// The current trunk version's metadata for one EHR's object, by `vo_id`.
///
/// Scoped to that one EHR in a single `version`⋈`audit` statement (no node
/// reassembly), returning `None` when the object is not the EHR's (a foreign or
/// unknown id). The lean `ETag`/`If-Match` read for an EHR-owned object; the
/// version identity is RM common master06 §Version Identification, the commit
/// instant §Committal and Audits.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn current_version_meta_scoped(
    pool: &PgPool,
    vo_id: VoId,
    ehr_id: EhrId,
) -> Result<Option<CurrentMeta>, StorageError> {
    let found = sqlx::query(CURRENT_VERSION_META_SCOPED_SQL)
        .bind(vo_id)
        .bind(ehr_id)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?;
    let Some(row) = found else {
        return Ok(None);
    };
    Ok(Some(current_meta_row(&row)?))
}

/// The current trunk version's metadata of a demographic versioned object,
/// read in one `version` and `audit` statement without node reassembly.
///
/// `ehr_id IS NULL` scopes the read to the demographic repository (our own
/// design). The caller gates the route on `kind` and the not-deleted
/// precondition on `lifecycle_state` (RM common master06 §Logical Deletion).
#[derive(Debug, Clone)]
pub struct CurrentDemographicMeta {
    /// The `version.kind` discriminator text.
    pub kind: String,
    /// The `version_lifecycle_state` numeric code.
    pub lifecycle_state: String,
    /// `VERSION_TREE_ID` first part.
    pub trunk_version: i32,
    /// `VERSION_TREE_ID` second part; `0` on a trunk row.
    pub branch_number: i32,
    /// `VERSION_TREE_ID` third part; `0` on a trunk row.
    pub branch_version: i32,
    /// The version's immutable `creating_system_id`.
    pub creating_system_id: String,
    /// The audit's server-computed commit instant.
    pub time_committed: jiff::Timestamp,
}

/// Read the lean [`CurrentDemographicMeta`] for a demographic versioned object.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn current_demographic_meta(
    pool: &PgPool,
    vo_id: VoId,
) -> Result<Option<CurrentDemographicMeta>, StorageError> {
    const SQL: &str = "SELECT v.kind, v.lifecycle_state, v.trunk_version, v.branch_number, \
                       v.branch_version, v.creating_system_id, a.time_committed \
                       FROM version v JOIN commit_audit_facts a ON a.id = v.commit_audit_id \
                       JOIN vo_head h ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
                       WHERE v.vo_id = $1 AND v.ehr_id IS NULL";
    let found = sqlx::query(SQL)
        .bind(vo_id)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?;
    let Some(row) = found else {
        return Ok(None);
    };
    Ok(Some(CurrentDemographicMeta {
        kind: row.try_get("kind")?,
        lifecycle_state: row.try_get("lifecycle_state")?,
        trunk_version: row.try_get("trunk_version")?,
        branch_number: row.try_get("branch_number")?,
        branch_version: row.try_get("branch_version")?,
        creating_system_id: row.try_get("creating_system_id")?,
        time_committed: row
            .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
            .to_jiff(),
    }))
}
