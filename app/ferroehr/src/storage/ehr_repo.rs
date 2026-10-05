// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the `ehr` table and the `ehr_folder` membership index.
//!
//! Covers the per-EHR reads/writes the EHR service chapter needs that are not
//! part of the versioned-object spine ([`crate::storage::version_repo`]).
//!
//! No openEHR spec governs the `ehr` / `ehr_folder` schema — it is our own
//! PG18-native design. The EHR concepts these
//! rows realize are arch-overview `master06-design_of_the_ehr.adoc` §The EHR
//! (root, `system_id`, `time_created`) and RM ehr `master04-ehr_package.adoc`
//! §EHR Creation / §Folders / §EHR Status. All *semantics* (subject sync
//! policy, directory-slot resolution, the `is_modifiable` write guard) stay in
//! the service layer, which calls these functions with plain inputs.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;
use sqlx::{PgConnection, PgExecutor, PgPool, Row};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;
use crate::storage::version_repo::meta::CurrentMeta;

/// Inserts the `ehr` root row with its promoted `EHR_STATUS` columns.
///
/// The row carries the id and the immutable `system_id`, and the promoted
/// `EHR_STATUS` columns (`subject_id`, `subject_namespace`, `is_queryable`,
/// `is_modifiable`) are set in the same statement, the create path knowing them
/// from the incoming `EHR_STATUS`. Returns `Some(time_created)`, the
/// server-assigned `EHR.time_created` (arch-overview master06 §The EHR) captured
/// via `RETURNING`, or `None` when the id already existed (`ON CONFLICT (id) DO
/// NOTHING`, which the caller maps to a 409). `EHR.system_id` is recorded at
/// creation and immutable thereafter (arch-overview master06 §System Identity).
///
/// The subject columns back the one-EHR-per-subject unique index
/// (`uq_ehr_subject`, RM ehr master04 §EHR Status), so a second EHR for the same
/// subject is reported as [`StorageError::SubjectInUse`] distinctly from the id
/// conflict. `is_queryable` backs the AQL full-population gate (SM
/// `I_QUERY_SERVICE`) and `is_modifiable` the content-write guard (RM ehr
/// master04 §EHR Active Status). No openEHR spec governs the promoted columns —
/// our own storage design.
///
/// # Errors
/// Returns [`StorageError::SubjectInUse`] on a subject-uniqueness violation,
/// else [`StorageError::Database`] on a driver/insert failure.
pub async fn insert_ehr(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    system_id: &str,
    subject_id: Option<&str>,
    subject_namespace: Option<&str>,
    is_queryable: bool,
    is_modifiable: bool,
) -> Result<Option<jiff::Timestamp>, StorageError> {
    let row = sqlx::query(
        "INSERT INTO ehr (id, system_id, subject_id, subject_namespace, is_queryable, \
         is_modifiable) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (id) DO NOTHING RETURNING time_created",
    )
    .bind(ehr_id)
    .bind(system_id)
    .bind(subject_id)
    .bind(subject_namespace)
    .bind(is_queryable)
    .bind(is_modifiable)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        // A subject-uniqueness violation is NOT the `ON CONFLICT (id)` target, so
        // it raises here; map it to the distinct subject-conflict error.
        if let sqlx::Error::Database(db) = &e
            && db.constraint() == Some("uq_ehr_subject")
        {
            return StorageError::SubjectInUse(
                subject_id.unwrap_or_default().to_owned(),
                subject_namespace.unwrap_or_default().to_owned(),
            );
        }
        StorageError::Database(e)
    })?;
    match row {
        Some(row) => Ok(Some(
            row.try_get::<jiff_sqlx::Timestamp, _>("time_created")?
                .to_jiff(),
        )),
        None => Ok(None),
    }
}

/// The CURRENT `EHR_STATUS` root node fragment (`num = 0` of the latest trunk
/// version) of an EHR, read on the CALLER'S connection so a transaction sees
/// the `EHR_STATUS` it has just written.
///
/// `None` when the EHR has no current `EHR_STATUS`.
///
/// This is the read half of the promoted-column refresh the paths that land
/// `EHR_STATUS` versions WITHOUT the service write hook use — the EHR Extract
/// import and the admin archive load: they hand this fragment to the single
/// service-layer extraction (`service::ehr::status::ehr_promoted_columns`), so
/// every path promotes identical `(subject_id, subject_namespace,
/// is_queryable, is_modifiable)` values. The fragment carries all four inputs
/// verbatim — the decomposition splits only *structure* types into their own
/// `node` rows, and a subject `PARTY_SELF`/`PARTY_IDENTIFIED` is not one, so
/// `subject.external_ref` stays inline on the root. Semantics: RM ehr master04
/// §EHR Status (`subject`, `is_queryable`) / §EHR Active Status
/// (`is_modifiable`). No openEHR spec governs the promoted columns or the node
/// decomposition — our own storage design.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn current_status_root(
    tx: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<Option<Value>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT n.data FROM version v \
         JOIN node n ON n.vo_id = v.vo_id AND n.sys_version = v.sys_version AND n.num = 0 \
         JOIN vo_head h ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
         WHERE v.ehr_id = $1 AND v.kind = 'EHR_STATUS'",
    )
    .bind(ehr_id)
    .fetch_optional(&mut *tx)
    .await?)
}

/// The id of the EHR whose promoted subject columns match `(subject_id,
/// namespace)`, or `None`. Served from the unique `ehr.subject_*` columns (one
/// EHR per subject — RM ehr master04 §EHR Status).
///
/// Generic over the executor (the one such read in this module): the subject
/// lookup serves both the pooled `GET /ehr?subject_id` path and the
/// import/archive-load conflict pre-check, which must run on the caller's
/// transaction connection rather than take a second pooled connection while a
/// write transaction is open.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn ehr_id_by_subject<'e>(
    executor: impl PgExecutor<'e>,
    subject_id: &str,
    namespace: &str,
) -> Result<Option<EhrId>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT id FROM ehr WHERE subject_id = $1 AND subject_namespace = $2")
            .bind(subject_id)
            .bind(namespace)
            .fetch_optional(executor)
            .await?,
    )
}

/// The `ehr` row header `(system_id, time_created)`, or `None` when the EHR
/// does not exist.
///
/// `system_id` is the stored per-EHR value (immutable, arch-overview master06
/// §System Identity), never the live config.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn ehr_header(
    pool: &PgPool,
    ehr_id: EhrId,
) -> Result<Option<(String, jiff::Timestamp)>, StorageError> {
    let Some(row) = sqlx::query("SELECT system_id, time_created FROM ehr WHERE id = $1")
        .bind(ehr_id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    let system_id: String = row.try_get("system_id")?;
    let time_created = row
        .try_get::<jiff_sqlx::Timestamp, _>("time_created")?
        .to_jiff();
    Ok(Some((system_id, time_created)))
}

/// Reads the whole `GET /ehr/{ehr_id}` representation in ONE statement.
///
/// Returns the EHR header, the current `EHR_STATUS` version identity, the
/// `EHR_ACCESS` versioned-object id, and the LIVE folder-hierarchy ids in
/// `rank` order, in one round trip (no openEHR spec governs read batching —
/// our own design).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn ehr_summary_read(
    pool: &PgPool,
    ehr_id: EhrId,
) -> Result<Option<EhrSummaryRead>, StorageError> {
    let Some(row) = sqlx::query(
        "SELECT e.system_id, e.time_created, \
                s.vo_id AS status_vo, s.trunk_version, s.branch_number, \
                s.branch_version, s.creating_system_id, \
                a.vo_id AS access_vo, \
                COALESCE(f.folders, ARRAY[]::uuid[]) AS folders \
         FROM ehr e \
         LEFT JOIN LATERAL ( \
             SELECT version.vo_id, version.trunk_version, version.branch_number, \
                    version.branch_version, version.creating_system_id \
             FROM version \
             JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
             WHERE version.ehr_id = e.id AND version.kind = 'EHR_STATUS' \
         ) s ON true \
         LEFT JOIN LATERAL ( \
             SELECT version.vo_id FROM version \
             JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
             WHERE version.ehr_id = e.id AND version.kind = 'EHR_ACCESS' \
         ) a ON true \
         LEFT JOIN LATERAL ( \
             SELECT array_agg(f.vo_id ORDER BY f.rank) AS folders \
             FROM ehr_folder f \
             JOIN version v ON v.vo_id = f.vo_id \
             JOIN vo_head h ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
             WHERE f.ehr_id = e.id AND v.lifecycle_state <> '523' \
         ) f ON true \
         WHERE e.id = $1",
    )
    .bind(ehr_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let status = match row.try_get::<Option<VoId>, _>("status_vo")? {
        None => None,
        Some(vo_id) => Some(EhrStatusIdentity {
            vo_id,
            trunk_version: row.try_get("trunk_version")?,
            branch_number: row.try_get("branch_number")?,
            branch_version: row.try_get("branch_version")?,
            creating_system_id: row.try_get("creating_system_id")?,
        }),
    };
    Ok(Some(EhrSummaryRead {
        system_id: row.try_get("system_id")?,
        time_created: row
            .try_get::<jiff_sqlx::Timestamp, _>("time_created")?
            .to_jiff(),
        status,
        access_vo: row.try_get("access_vo")?,
        folders: row.try_get("folders")?,
    }))
}

/// The merged `GET /ehr/{ehr_id}` read ([`ehr_summary_read`]).
#[derive(Debug)]
pub struct EhrSummaryRead {
    /// The stored, immutable `EHR.system_id`.
    pub system_id: String,
    /// `EHR.time_created`.
    pub time_created: jiff::Timestamp,
    /// The current `EHR_STATUS` version identity.
    pub status: Option<EhrStatusIdentity>,
    /// The `EHR_ACCESS` versioned-object id.
    pub access_vo: Option<Uuid>,
    /// The LIVE folder-hierarchy ids in `rank` order — the members of
    /// `EHR.folders` (RM ehr, EHR class `Folders_valid`; RM ehr master04
    /// §Folders). "Live" = the current trunk version exists and is not
    /// logically deleted (lifecycle `523`). Empty when the EHR indexes no
    /// live hierarchy.
    pub folders: Vec<Uuid>,
}

/// The current `EHR_STATUS` version identity of the merged summary read.
#[derive(Debug)]
pub struct EhrStatusIdentity {
    /// The `EHR_STATUS` versioned object.
    pub vo_id: VoId,
    /// `VERSION_TREE_ID` trunk.
    pub trunk_version: i32,
    /// `VERSION_TREE_ID` branch number (0 = trunk).
    pub branch_number: i32,
    /// `VERSION_TREE_ID` branch version.
    pub branch_version: i32,
    /// The per-version creating system.
    pub creating_system_id: String,
}

/// The versioned-object id of the EHR's directory — `EHR.directory`
/// (`folders.item(1)`, RM ehr, EHR class `Directory_in_folders`).
///
/// Resolved as the lowest-`rank` LIVE hierarchy, falling back to the
/// lowest-`rank` still-existing one so a read after a logical delete resolves
/// to the deleted version (→ 204) rather than 404. `None` when the EHR
/// indexes no hierarchy.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn directory_vo(pool: &PgPool, ehr_id: EhrId) -> Result<Option<VoId>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT f.vo_id FROM ehr_folder f \
         JOIN version v ON v.vo_id = f.vo_id \
         JOIN vo_head h ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
         WHERE f.ehr_id = $1 \
         ORDER BY (v.lifecycle_state = '523'), f.rank \
         LIMIT 1",
    )
    .bind(ehr_id)
    .fetch_optional(pool)
    .await?)
}

/// Resolves the EHR's directory slot, current version metadata, and write flag.
///
/// One statement covers the directory slot **and** its current version metadata
/// **and** the EHR's `is_modifiable` content-write flag: the
/// `ehr_folder`⋈`version`⋈`audit`⋈`ehr` join, ordered live-first by `rank`
/// (the same slot resolution [`directory_vo`] applies), projecting the current
/// trunk version's `VERSION_TREE_ID` column ints + stored `creating_system_id` +
/// audit `time_committed`, plus the promoted `ehr.is_modifiable`. Folds the
/// [`directory_vo`] slot lookup, the metadata-only current-version read, and
/// the `is_modifiable` read into one round trip for the directory
/// `If-Match`/`412` write paths (`update`/`delete`). The
/// full-`OBJECT_VERSION_ID` compare the caller builds from these columns is
/// unchanged (ITS-REST overview §Concurrency control); the `is_modifiable` gate
/// is RM ehr master04 §EHR Active Status. Returns `(meta, is_modifiable)`; `None`
/// when the EHR indexes no folder hierarchy. No openEHR spec governs the
/// `ehr_folder` schema — our own design.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn directory_current_meta(
    pool: &PgPool,
    ehr_id: EhrId,
) -> Result<Option<(CurrentMeta, bool)>, StorageError> {
    let Some(row) = sqlx::query(
        "SELECT v.vo_id, v.trunk_version, v.branch_number, v.branch_version, \
         v.creating_system_id, a.time_committed, e.is_modifiable \
         FROM ehr_folder f \
         JOIN version v ON v.vo_id = f.vo_id \
         JOIN vo_head h ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
         JOIN commit_audit a ON a.id = v.commit_audit_id \
         JOIN ehr e ON e.id = f.ehr_id \
         WHERE f.ehr_id = $1 \
         ORDER BY (v.lifecycle_state = '523'), f.rank \
         LIMIT 1",
    )
    .bind(ehr_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let meta = CurrentMeta {
        vo_id: row.try_get("vo_id")?,
        trunk_version: row.try_get("trunk_version")?,
        branch_number: row.try_get("branch_number")?,
        branch_version: row.try_get("branch_version")?,
        creating_system_id: row.try_get("creating_system_id")?,
        time_committed: row
            .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
            .to_jiff(),
    };
    Ok(Some((meta, row.try_get("is_modifiable")?)))
}

/// Whether the EHR is modifiable.
///
/// Read from the promoted `ehr.is_modifiable` column (kept in lockstep with the
/// current `EHR_STATUS.is_modifiable` by the service's `sync_ehr_subject` hook
/// and its import/archive-load re-promotion over [`current_status_root`]; RM
/// ehr master04 §EHR Active Status). `None` when the EHR does not exist (the
/// caller treats that as modifiable so the guard never spuriously blocks). No
/// openEHR spec governs the promoted column — our own storage design.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn ehr_is_modifiable(pool: &PgPool, ehr_id: EhrId) -> Result<Option<bool>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT is_modifiable FROM ehr WHERE id = $1")
            .bind(ehr_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// The in-transaction, row-locked read of the promoted `ehr.is_modifiable`
/// flag — the commit engine's content-write gate.
///
/// The lock serializes the gate against a concurrent `EHR_STATUS` flip (whose
/// own commit updates this row): `FOR SHARE` for a content-only change set
/// (concurrent content commits stay parallel), `FOR NO KEY UPDATE` when the
/// set itself writes an `EHR_STATUS` (taking the exclusive lock up front
/// avoids the share-then-upgrade deadlock). Plain reads are unaffected by
/// either. `None` when the EHR row does not exist. No openEHR spec governs
/// the promoted column or the locking — our own storage design.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn ehr_is_modifiable_locked(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    exclusive: bool,
) -> Result<Option<bool>, StorageError> {
    let sql = if exclusive {
        "SELECT is_modifiable FROM ehr WHERE id = $1 FOR NO KEY UPDATE"
    } else {
        "SELECT is_modifiable FROM ehr WHERE id = $1 FOR SHARE"
    };
    Ok(sqlx::query_scalar(sql)
        .bind(ehr_id)
        .fetch_optional(tx)
        .await?)
}

/// The two content-write pre-checks plus the server clock in ONE round trip:
/// whether the EHR exists, whether it is modifiable, and the database `now()`.
///
/// Reads the `ehr` row directly — a present row is the existence signal and
/// carries the promoted `is_modifiable` column (synced with the current
/// `EHR_STATUS` by `sync_ehr_subject` and its import/archive-load
/// re-promotion over [`current_status_root`]). Returns `(exists,
/// is_modifiable, now)` where `is_modifiable` is `None` exactly when the EHR
/// does not exist and `now` is the database transaction timestamp of this
/// probe, carried forward as the commit instant of a create that follows it
/// (the database stays the only clock; the fresh versioned object has no
/// stored interval the slightly earlier instant could disorder). The concepts
/// guarded are RM ehr master04 §EHR Creation (existence) and §EHR Active
/// Status (`EHR_STATUS.is_modifiable`); no openEHR spec governs the promoted
/// column or the probe shape — our own storage design.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn ehr_writability<'e>(
    executor: impl PgExecutor<'e>,
    ehr_id: EhrId,
) -> Result<(bool, Option<bool>, jiff::Timestamp), StorageError> {
    let row = sqlx::query(
        "SELECT (SELECT is_modifiable FROM ehr WHERE id = $1) AS is_modifiable, now() AS ts",
    )
    .bind(ehr_id)
    .fetch_one(executor)
    .await?;
    let is_modifiable: Option<bool> = row.try_get("is_modifiable")?;
    let now = row.try_get::<jiff_sqlx::Timestamp, _>("ts")?.to_jiff();
    Ok((is_modifiable.is_some(), is_modifiable, now))
}

/// Whether ANY live (non-deleted) folder hierarchy is indexed for the EHR —
/// the `POST /directory` conflict probe.
///
/// Deliberately ignores logically deleted hierarchies: after a `523|deleted|`
/// version the container remains (RM common master06 §Logical Deletion) but
/// the directory slot is vacant, so a new hierarchy may be created (RM ehr
/// master04 §Folders — "an entirely new Folder hierarchy may be added"); only
/// a LIVE occupant conflicts (CNF master09 E.2 requires the error only for an
/// EHR *with* a directory).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn live_directory_exists(pool: &PgPool, ehr_id: EhrId) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS( \
           SELECT 1 FROM ehr_folder f \
           JOIN version v ON v.vo_id = f.vo_id \
           JOIN vo_head h ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
           WHERE f.ehr_id = $1 AND v.lifecycle_state <> '523')",
    )
    .bind(ehr_id)
    .fetch_one(pool)
    .await?)
}

/// Which of `roots` exist as versioned objects in `ehr_id`.
///
/// Reads the full physical store (`version`: hot + cold, so an
/// archived target still resolves), in any lifecycle state (a logically
/// deleted object still resolves — its container remains, RM common
/// master06 §Logical Deletion).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn existing_vo_roots<'e>(
    executor: impl PgExecutor<'e>,
    ehr_id: EhrId,
    roots: &[Uuid],
) -> Result<Vec<Uuid>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT vo_id FROM version WHERE ehr_id = $1 AND vo_id = ANY($2)",
    )
    .bind(ehr_id)
    .bind(roots)
    .fetch_all(executor)
    .await?)
}

/// Whether the EHR already has a LIVE folder hierarchy with that root identity.
///
/// The root must carry the given `archetype_node_id` AND name — the LOCATABLE
/// identity pair that distinguishes same-archetype siblings (RM common, paths:
/// the name predicate disambiguates same-archetype nodes). Backs the
/// CONTRIBUTION-route duplicate-directory rejection.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn live_folder_root_exists(
    pool: &PgPool,
    ehr_id: EhrId,
    root_archetype_node_id: &str,
    root_name: &str,
) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS( \
           SELECT 1 FROM ehr_folder f \
           JOIN version v ON v.vo_id = f.vo_id \
           JOIN vo_head h ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
           JOIN node n ON n.vo_id = v.vo_id AND n.sys_version = v.sys_version \
             AND n.num = 0 \
           WHERE f.ehr_id = $1 AND v.lifecycle_state <> '523' \
             AND n.data->>'archetype_node_id' = $2 \
             AND n.data#>>'{name,value}' = $3)",
    )
    .bind(ehr_id)
    .bind(root_archetype_node_id)
    .bind(root_name)
    .fetch_one(pool)
    .await?)
}
