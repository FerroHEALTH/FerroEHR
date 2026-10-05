// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The local commit write path.
//!
//! Covers the `commit_audit` + `contribution` inserts, the folded
//! one-statement version commit with its head upsert, and the
//! folder-membership + event-outbox writes that ride along inside the same
//! commit transaction.
//!
//! No openEHR spec governs the SQL — our own design. The change-control law realized here is RM common master06
//! (§Committal and Audits, §The 'Virtual Version Tree'); `AUDIT_DETAILS` is
//! master04.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// The `AUDIT_DETAILS` fields to persist, as the `commit_audit` row's own
/// columns.
///
/// master04 §Audit Details: `change_type` is the numeric `audit_change_type`
/// group code, never a rubric (`Change_type_valid`), and the three jsonb
/// columns arrive as the canonical RM fragments they store
/// (`DV_TEXT` / `PARTY_PROXY` / the `ATTESTATION`-declared attributes).
///
/// The fragments are OWNED because the versioning layer holds these attributes
/// as their RM values and encodes them once, here at its boundary
/// (`crate::versioning::audit::AuditInput::row`): storage takes plain value
/// inputs and never decodes RM types (see the module docs), and the
/// `ATTESTATION`-declared subset is not an RM class it could name.
#[derive(Debug)]
pub struct AuditRow<'a> {
    /// `AUDIT_DETAILS.system_id`.
    pub system_id: &'a str,
    /// The numeric `audit_change_type` group code.
    pub change_type: &'a str,
    /// The canonical `DV_TEXT` fragment of `AUDIT_DETAILS.description`, when
    /// the committer supplied one.
    pub description: Option<Value>,
    /// The canonical `PARTY_PROXY` JSON of the committer.
    pub committer: Value,
    /// The canonical fragment of the `ATTESTATION`-declared attributes when
    /// this commit audit is an `ATTESTATION` (master06 §Attestation), else
    /// `None`.
    pub attestation: Option<Value>,
}

/// Takes the per-vo transaction advisory lock that serializes concurrent
/// writers of one versioned object, so branch writers do not all contend on one
/// current row.
///
/// The versioning tree-placement decision calls this before it reads the
/// preceding version.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
// The preceding-version reads and the next-ordinal/next-branch computation that
// surround this lock are the version-tree placement DECISION — they live in the
// versioning layer (`versioning::change`), which calls this lock first.
pub async fn advisory_lock(tx: &mut PgConnection, vo_id: VoId) -> Result<(), StorageError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(vo_id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Inserts a `commit_audit` row, returning its id and the server-computed
/// timestamp.
///
/// The `time_committed` (master06 §Committal and Audits) is captured via `RETURNING` so
/// the commit path can build the exact `ORIGINAL_VERSION` it will later serve —
/// the signed bytes must match the read-time canonical form.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
pub async fn insert_audit(
    tx: &mut PgConnection,
    audit: &AuditRow<'_>,
) -> Result<(Uuid, jiff::Timestamp), StorageError> {
    let row = sqlx::query(
        "INSERT INTO commit_audit (system_id, change_type, description, committer, attestation) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id, time_committed",
    )
    .bind(audit.system_id)
    .bind(audit.change_type)
    .bind(&audit.description)
    .bind(&audit.committer)
    .bind(&audit.attestation)
    .fetch_one(&mut *tx)
    .await?;
    let id: Uuid = row.try_get("id")?;
    let time_committed = row
        .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
        .to_jiff();
    Ok((id, time_committed))
}

/// Insert a `contribution` row referencing its audit under the server-minted
/// `id` ([`crate::licence::stamp`]), returning it. `ehr_id` is `None` for a
/// demographic CONTRIBUTION (no EHR scope).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
/// ([`StorageError::ContributionUidInUse`] is structurally unreachable here —
/// the id is freshly minted — but kept as the absent-`RETURNING` mapping so
/// the statement stays identical to the supplied-uid path in
/// [`write_contribution`].)
pub async fn insert_contribution(
    tx: &mut PgConnection,
    id: Uuid,
    ehr_id: Option<EhrId>,
    commit_audit_id: Uuid,
) -> Result<Uuid, StorageError> {
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO contribution (id, ehr_id, commit_audit_id) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (id) DO NOTHING RETURNING id",
    )
    .bind(id)
    .bind(ehr_id)
    .bind(commit_audit_id)
    .fetch_optional(&mut *tx)
    .await?;
    inserted.ok_or(StorageError::ContributionUidInUse(None))
}

/// Insert a `commit_audit` row and its enclosing `contribution` in ONE round trip
/// via a data-modifying CTE, returning `(contribution_id, commit_audit_id,
/// time_committed)`.
///
/// The `contribution` references the just-inserted `audit`; `time_committed`
/// is the server-computed commit instant (master06 §Committal and Audits) the
/// version's `commit_audit` is signed against. A client-supplied CONTRIBUTION
/// uid is honoured (`supplied`); a duplicate id is a
/// [`StorageError::ContributionUidInUse`] conflict, never an overwrite
/// (ITS-REST `contribution_create`).
///
/// The audit-to-contribution insert is a dependent chain, the CONTRIBUTION and
/// its `AUDIT_DETAILS` committing together (master06 §Committal and Audits);
/// merging the two statements into one CTE is a round-trip optimisation whose
/// rows and returned values are byte-identical to two separate inserts, both
/// inside the caller's transaction. No openEHR spec governs statement batching —
/// our own design.
///
/// On a `supplied`-uid conflict the `contribution` CTE inserts nothing (`ON
/// CONFLICT DO NOTHING`), so the outer `LEFT JOIN` yields a NULL
/// `contribution_id` and [`StorageError::ContributionUidInUse`]; the audit CTE
/// is discarded when the transaction unwinds.
///
/// # Errors
/// Returns [`StorageError::ContributionUidInUse`] on a duplicate supplied uid,
/// else [`StorageError::Database`] on a driver/insert failure.
pub async fn write_contribution(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    audit: &AuditRow<'_>,
    supplied: Option<Uuid>,
) -> Result<(Uuid, Uuid, jiff::Timestamp), StorageError> {
    let row = sqlx::query(
        "WITH a AS ( \
             INSERT INTO commit_audit (system_id, change_type, description, committer, attestation) \
             VALUES ($1, $2, $3, $4, $5) RETURNING id, time_committed \
         ), c AS ( \
             INSERT INTO contribution (id, ehr_id, commit_audit_id) \
             SELECT COALESCE($6, uuidv7()), $7, a.id FROM a \
             ON CONFLICT (id) DO NOTHING \
             RETURNING id \
         ) \
         SELECT a.id AS commit_audit_id, a.time_committed, c.id AS contribution_id \
         FROM a LEFT JOIN c ON true",
    )
    .bind(audit.system_id)
    .bind(audit.change_type)
    .bind(&audit.description)
    .bind(&audit.committer)
    .bind(&audit.attestation)
    .bind(supplied)
    .bind(ehr_id)
    .fetch_one(&mut *tx)
    .await?;
    let contribution_id: Option<Uuid> = row.try_get("contribution_id")?;
    let contribution_id = contribution_id.ok_or(StorageError::ContributionUidInUse(supplied))?;
    let commit_audit_id: Uuid = row.try_get("commit_audit_id")?;
    let time_committed = row
        .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
        .to_jiff();
    Ok((contribution_id, commit_audit_id, time_committed))
}

/// The `version` columns for a **folded** commit.
///
/// Every content column of a stored version EXCEPT
/// `contribution_id`/`commit_audit_id`, which come from the same statement's
/// `contribution`/`commit_audit` CTEs.
///
/// `time_committed` is the caller's pre-read commit instant, a database `now()`
/// fetched earlier on this request and so still server-assigned (master06
/// §Committal and Audits). Binding it makes the stored audit time, the version row's
/// own `committed_at` and the instant the `VERSION.signature` was computed over
/// one value by construction.
///
/// Nothing is superseded in place: the store is append-only, so a commit
/// inserts its version row and updates the object's one mutable head row in the
/// same statement. Which version is current is the head row's answer
/// (master06 §The 'Virtual Version Tree'), not a validity interval.
#[derive(Debug)]
pub struct FoldedVersion<'a> {
    /// The versioned object's id.
    pub vo_id: VoId,
    /// The `version.kind` discriminator text.
    pub kind: &'a str,
    /// The owning EHR, or `None` for a demographic versioned object.
    pub ehr_id: Option<EhrId>,
    /// The per-object storage commit ordinal — NOT the wire version number.
    pub sys_version: i32,
    /// `VERSION_TREE_ID` first part.
    pub trunk_version: i32,
    /// `VERSION_TREE_ID` second part; `0` on a trunk row.
    pub branch_number: i32,
    /// `VERSION_TREE_ID` third part; `0` on a trunk row.
    pub branch_version: i32,
    /// The `version_lifecycle_state` numeric code.
    pub lifecycle_state: &'a str,
    /// The version's immutable `creating_system_id`.
    pub creating_system_id: &'a str,
    /// `ORIGINAL_VERSION.preceding_version_uid`; `None` for a first version.
    pub preceding_version_uid: Option<&'a str>,
    /// The OPT `template_id` a COMPOSITION was committed against (else `None`).
    pub template_id: Option<&'a str>,
    /// `VERSION.signature` (0..1), opaque radix-64.
    pub signature: Option<&'a str>,
    /// Whether `signature` was supplied verbatim by the client (foreign — never
    /// re-verified at read; master06 §Digital Signature) vs generated by this
    /// server. `false` for a server signature or an unsigned version.
    pub signature_client_supplied: bool,
    /// Whether the RELEASED openEHR generation set can express this version's
    /// body — the `version.stable_compatible` stamp the read-time
    /// `spec_profile` gate consults. No openEHR spec governs runtime
    /// generation selection — our own design/extension.
    pub stable_compatible: bool,
    /// The distinct origins of the body (`version.origins`, a JSON array
    /// of `FEEDER_AUDIT` originating system ids, or this server's own), the
    /// access log's answer to EHDS Annex II 3.2(e) (#3212).
    pub origins: &'a [String],
    /// The canonical body bytes (`version.body`, text): the accepted,
    /// uid-stamped value serialized before node decomposition, so a point read
    /// serves the codec's field order verbatim. `None` on a logical delete (no
    /// content, RM common master06 §Logical Deletion).
    pub body: Option<&'a str>,
    /// The commit instant: the database `now()` the caller read on this
    /// request (the placement read, the writability gate, or the owning
    /// CONTRIBUTION's committal), stored as the audit `time_committed` and as
    /// the version row's own `committed_at`.
    pub time_committed: jiff::Timestamp,
    /// The decomposed node rows — inserted by the SAME statement through a
    /// node CTE ordered after the version row (empty on a logical delete:
    /// the unnest yields no rows and the CTE writes nothing).
    pub rows: &'a [crate::storage::row::NodeRow],
}

/// The head upsert both folded commit statements carry.
///
/// One mutable row per versioned object, written in the same statement as the
/// version row it describes. A TRUNK commit advances every column; a BRANCH
/// commit advances only what a branch changes — the any-lineage head ordinal
/// and the commit instant — leaving the trunk's own head, lifecycle state and
/// template alone, because a branch does not supersede the trunk (RM common
/// master06 §The 'Virtual Version Tree').
///
/// `{branch}`, `{ordinal}`, `{lifecycle}` and `{template}` are the caller's
/// parameter placeholders; `FROM v` orders this CTE after the version insert.
///
/// The update is MONOTONIC: it applies only when the committing version's
/// ordinal is greater than the one the head already records, and a trunk commit
/// additionally only when it is greater than the recorded trunk head. The
/// ordinal comes from a placement read under the per-object advisory lock, so
/// under that lock it always is; the guard is what keeps a replayed or
/// out-of-order statement from walking the head backwards, which would make an
/// older version read as current.
struct HeadUpsert<'a> {
    /// The versioned object's id.
    vo: &'a str,
    /// The versioned object's RM type.
    kind: &'a str,
    /// The owning EHR, `NULL` for a party.
    ehr: &'a str,
    /// The new version's `sys_version`.
    ordinal: &'a str,
    /// The new version's `branch_number`; `0` is a trunk commit.
    branch: &'a str,
    /// The new version's `lifecycle_state`.
    lifecycle: &'a str,
    /// The template the new version was validated against.
    template: &'a str,
    /// The commit instant.
    committed_at: &'a str,
}

impl HeadUpsert<'_> {
    /// Render the CTE body with this statement's placeholders substituted.
    fn sql(&self) -> String {
        let Self {
            vo,
            kind,
            ehr,
            ordinal,
            branch,
            lifecycle,
            template,
            committed_at,
        } = *self;
        format!(
            "INSERT INTO vo_head (vo_id, kind, ehr_id, head_sys_version, \
             trunk_head_sys_version, lifecycle_state, template_id, committed_at) \
         SELECT {vo}, {kind}, {ehr}, {ordinal}, \
                CASE WHEN {branch} = 0 THEN {ordinal} ELSE NULL END, \
                {lifecycle}, {template}, {committed_at}::timestamptz \
         FROM v \
         ON CONFLICT (vo_id) DO UPDATE SET \
             head_sys_version = EXCLUDED.head_sys_version, \
             committed_at = EXCLUDED.committed_at, \
             trunk_head_sys_version = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.head_sys_version ELSE vo_head.trunk_head_sys_version END, \
             lifecycle_state = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.lifecycle_state ELSE vo_head.lifecycle_state END, \
             template_id = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.template_id ELSE vo_head.template_id END \
         WHERE EXCLUDED.head_sys_version > vo_head.head_sys_version \
           AND ({branch} <> 0 \
                OR vo_head.trunk_head_sys_version IS NULL \
                OR EXCLUDED.head_sys_version > vo_head.trunk_head_sys_version)"
        )
    }
}

/// A **standalone** folded commit.
///
/// `audit` + `contribution` + `version` + the decomposed `node` rows in
/// ONE data-modifying CTE chain, returning `(contribution_id, commit_audit_id,
/// time_committed)`.
///
/// The single audit row serves both the CONTRIBUTION and the version's
/// `commit_audit` (a direct write is one CONTRIBUTION of one change —
/// master06 §Committal and Audits). `time_committed` is the server-computed
/// commit instant (master06 §Committal and Audits).
///
/// The externalized multimedia URIs the node rows reference are indexed in the
/// same transaction ([`crate::storage::blob_ref::record`]), which issues no
/// statement when there are none.
///
/// This is the round-trip-collapsed equivalent of [`write_contribution`]
/// followed by a plain `version` insert, byte-identical in the rows written
/// and the values returned: the version's `committed_at` and the audit's
/// `time_committed` are both the caller's bound
/// [`FoldedVersion::time_committed`], and everything runs inside the caller's
/// transaction. No openEHR spec governs statement batching — our own design.
///
/// # Errors
/// Returns [`StorageError::ContributionUidInUse`] on a duplicate supplied uid
/// (the `contribution` CTE inserts nothing → NULL `contribution_id`), else
/// [`StorageError::Database`] on a driver/insert failure.
pub async fn commit_new_version(
    tx: &mut PgConnection,
    audit: &AuditRow<'_>,
    supplied: Option<Uuid>,
    v: &FoldedVersion<'_>,
) -> Result<(Uuid, Uuid, jiff::Timestamp), StorageError> {
    static SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "WITH a AS ( \
                 INSERT INTO commit_audit (system_id, change_type, description, committer, \
                                           attestation, time_committed) \
                 VALUES ($1, $2, $3, $4, $5, $22::timestamptz) RETURNING id, time_committed \
             ), c AS ( \
                 INSERT INTO contribution (id, ehr_id, commit_audit_id) \
                 SELECT COALESCE($6, uuidv7()), $7, a.id FROM a \
                 ON CONFLICT (id) DO NOTHING \
                 RETURNING id \
             ), v AS ( \
                 INSERT INTO version \
                   (vo_id, kind, ehr_id, sys_version, trunk_version, branch_number, branch_version, \
                    lifecycle_state, creating_system_id, preceding_version_uid, \
                    contribution_id, commit_audit_id, template_id, signature, \
                    signature_client_supplied, stable_compatible, body, origins, committed_at) \
                 SELECT $8, $9, $7, $10, $11, $12, $13, \
                        $14, $15, $16, c.id, a.id, $17, $18, $19, $20, $21, $23, \
                        $22::timestamptz \
                 FROM a, c \
                 RETURNING 1 \
             ), h AS ( {} \
             ), n AS ( {} ) \
             SELECT a.id AS commit_audit_id, a.time_committed, c.id AS contribution_id \
             FROM a LEFT JOIN c ON true",
            HeadUpsert {
                vo: "$8",
                kind: "$9",
                ehr: "$7",
                ordinal: "$10",
                branch: "$12",
                lifecycle: "$14",
                template: "$17",
                committed_at: "$22",
            }
            .sql(),
            crate::storage::node_repo::node_insert_cte("$8", "$10", "$7", 24)
        )
    });
    let row = sqlx::query(sqlx::AssertSqlSafe(SQL.as_str()))
        .bind(audit.system_id)
        .bind(audit.change_type)
        .bind(&audit.description)
        .bind(&audit.committer)
        .bind(&audit.attestation)
        .bind(supplied)
        .bind(v.ehr_id)
        .bind(v.vo_id)
        .bind(v.kind)
        .bind(v.sys_version)
        .bind(v.trunk_version)
        .bind(v.branch_number)
        .bind(v.branch_version)
        .bind(v.lifecycle_state)
        .bind(v.creating_system_id)
        .bind(v.preceding_version_uid)
        .bind(v.template_id)
        .bind(v.signature)
        .bind(v.signature_client_supplied)
        .bind(v.stable_compatible)
        .bind(v.body)
        .bind(v.time_committed.to_string())
        .bind(serde_json::json!(v.origins));
    let node_refs: Vec<&crate::storage::row::NodeRow> = v.rows.iter().collect();
    let row = crate::storage::node_repo::bind_node_arrays(row, &node_refs)
        .fetch_one(&mut *tx)
        .await?;
    let contribution_id: Option<Uuid> = row.try_get("contribution_id")?;
    let contribution_id = contribution_id.ok_or(StorageError::ContributionUidInUse(supplied))?;
    let commit_audit_id: Uuid = row.try_get("commit_audit_id")?;
    let time_committed = row
        .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
        .to_jiff();
    crate::storage::blob_ref::record(tx, &[(v.vo_id, v.sys_version, v.rows)]).await?;
    Ok((contribution_id, commit_audit_id, time_committed))
}

/// A folded commit WITHIN an already-opened CONTRIBUTION.
///
/// The version's own `commit_audit` + `version` + the decomposed `node`
/// rows in ONE data-modifying CTE chain, referencing the pre-existing
/// `contribution_id`.
///
/// Returns `(commit_audit_id, time_committed)`, and indexes the externalized
/// multimedia URIs of the node rows in the same transaction
/// ([`crate::storage::blob_ref::record`]). The CONTRIBUTION and its own audit
/// were written earlier in the same transaction ([`write_contribution`]);
/// each change carries its own `commit_audit` (master06 §Committal and
/// Audits), stamped with the caller's bound [`FoldedVersion::time_committed`]
/// exactly as [`commit_new_version`] does. No openEHR spec governs statement
/// batching — our own design.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
pub async fn commit_version_into(
    tx: &mut PgConnection,
    audit: &AuditRow<'_>,
    contribution_id: Uuid,
    v: &FoldedVersion<'_>,
) -> Result<(Uuid, jiff::Timestamp), StorageError> {
    static SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "WITH a AS ( \
                 INSERT INTO commit_audit (system_id, change_type, description, committer, \
                                           attestation, time_committed) \
                 VALUES ($1, $2, $3, $4, $5, $22::timestamptz) RETURNING id, time_committed \
             ), v AS ( \
                 INSERT INTO version \
                   (vo_id, kind, ehr_id, sys_version, trunk_version, branch_number, branch_version, \
                    lifecycle_state, creating_system_id, preceding_version_uid, \
                    contribution_id, commit_audit_id, template_id, signature, \
                    signature_client_supplied, stable_compatible, body, origins, committed_at) \
                 SELECT $6, $7, $8, $9, $10, $11, $12, \
                        $13, $14, $15, $16, a.id, $17, $18, $19, $20, $21, $23, \
                        $22::timestamptz \
                 FROM a \
                 RETURNING 1 \
             ), h AS ( {} \
             ), n AS ( {} ) \
             SELECT a.id AS commit_audit_id, a.time_committed FROM a",
            HeadUpsert {
                vo: "$6",
                kind: "$7",
                ehr: "$8",
                ordinal: "$9",
                branch: "$11",
                lifecycle: "$13",
                template: "$17",
                committed_at: "$22",
            }
            .sql(),
            crate::storage::node_repo::node_insert_cte("$6", "$9", "$8", 24)
        )
    });
    let row = sqlx::query(sqlx::AssertSqlSafe(SQL.as_str()))
        .bind(audit.system_id)
        .bind(audit.change_type)
        .bind(&audit.description)
        .bind(&audit.committer)
        .bind(&audit.attestation)
        .bind(v.vo_id)
        .bind(v.kind)
        .bind(v.ehr_id)
        .bind(v.sys_version)
        .bind(v.trunk_version)
        .bind(v.branch_number)
        .bind(v.branch_version)
        .bind(v.lifecycle_state)
        .bind(v.creating_system_id)
        .bind(v.preceding_version_uid)
        .bind(contribution_id)
        .bind(v.template_id)
        .bind(v.signature)
        .bind(v.signature_client_supplied)
        .bind(v.stable_compatible)
        .bind(v.body)
        .bind(v.time_committed.to_string())
        .bind(serde_json::json!(v.origins));
    let node_refs: Vec<&crate::storage::row::NodeRow> = v.rows.iter().collect();
    let row = crate::storage::node_repo::bind_node_arrays(row, &node_refs)
        .fetch_one(&mut *tx)
        .await?;
    let commit_audit_id: Uuid = row.try_get("commit_audit_id")?;
    let time_committed = row
        .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
        .to_jiff();
    crate::storage::blob_ref::record(tx, &[(v.vo_id, v.sys_version, v.rows)]).await?;
    Ok((commit_audit_id, time_committed))
}

// ── folder membership ─────────────────────────────────────────────────────────

/// Append a new folder-hierarchy membership row for an EHR (RM ehr master04
/// §Folders; RM ehr EHR class `Directory_in_folders`).
///
/// `rank` is 1-based, append-only and never reused: the next rank is
/// `max(rank)+1` for this EHR. Called once per FOLDER *creation*. No openEHR
/// spec governs the `ehr_folder` storage mechanism (our own design).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
pub async fn insert_ehr_folder_rank(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    vo_id: VoId,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO ehr_folder (ehr_id, rank, vo_id) VALUES \
         ($1, (SELECT COALESCE(MAX(rank), 0) + 1 FROM ehr_folder WHERE ehr_id = $1), $2)",
    )
    .bind(ehr_id)
    .bind(vo_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

// ── event outbox ──────────────────────────────────────────────────────────────

/// Write the contribution-outbox event row **inside the commit transaction**
/// it announces — no commit without its event, no event without its commit.
///
/// No openEHR spec governs eventing (our own extension). The PHI-free
/// per-version entries are built by the versioning layer
/// (`Committed::envelope_entry`); this function wraps them in the fixed
/// envelope shape the events-extension drainer consumes (`{contribution_id,
/// ehr_id, committed_at, versions[]}`).
///
/// An EHR carrying a legal mark emits no event. The `INSERT ... SELECT` writes
/// nothing when the EHR is restricted (GDPR Art. 18(2) leaves only storage) or
/// carries an unoverridden research objection (Art. 21(6)), so nothing about
/// that record reaches a downstream consumer; both are read in the same
/// statement rather than a probe before it, so a mark recorded concurrently
/// cannot slip between the two. `ehr_id IS NULL` is a demographic commit, which
/// no EHR mark reaches. `docs/law/eu/gdpr/text.html`.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
pub async fn write_outbox(
    tx: &mut PgConnection,
    contribution_id: Uuid,
    ehr_id: Option<EhrId>,
    committed_at: jiff::Timestamp,
    versions: Vec<Value>,
) -> Result<(), StorageError> {
    let envelope = serde_json::json!({
        "contribution_id": contribution_id,
        "ehr_id": ehr_id,
        "committed_at": committed_at.to_string(),
        "versions": versions,
    });
    // The party domain carries no `ehr` relation, and a demographic commit has
    // no EHR to mark, so the marked-EHR filter is on the clinical statement
    // alone: naming `ehr` in the party domain would fail at parse time.
    let sql = if ehr_id.is_some() {
        "INSERT INTO event_outbox (contribution_id, ehr_id, envelope, committed_at) \
         SELECT $1, $2, $3, $4::timestamptz \
         WHERE NOT EXISTS (SELECT 1 FROM ehr e WHERE e.id = $2 \
             AND (e.restricted_at IS NOT NULL \
                  OR (e.research_objected_at IS NOT NULL \
                      AND e.research_objection_ground IS NULL)))"
    } else {
        "INSERT INTO event_outbox (contribution_id, ehr_id, envelope, committed_at) \
         VALUES ($1, $2, $3, $4::timestamptz)"
    };
    sqlx::query(sql)
        .bind(contribution_id)
        .bind(ehr_id)
        .bind(&envelope)
        .bind(committed_at.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}
