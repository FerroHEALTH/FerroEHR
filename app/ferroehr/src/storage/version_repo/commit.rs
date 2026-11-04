// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The local commit write path.
//!
//! Covers the folded commit statement ([`commit_folded`]): the `commit_audit`
//! row with its `contribution`, its version with the head upsert and node
//! rows, or both, in one statement; and the
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

use std::sync::LazyLock;

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
    /// `ORIGINAL_VERSION.other_input_version_uids`; empty when not a merge.
    pub other_input_version_uids: &'a [String],
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
    /// node CTE ordered after the version row (empty on a logical delete and
    /// on a version that shares its predecessor's rows: the unnest yields no
    /// rows and the CTE writes nothing).
    pub rows: &'a [crate::storage::row::NodeRow],
    /// SHA-256 over the canonical body with the root `uid` removed
    /// (`version.content_digest`); `None` on a logical delete.
    pub content_digest: Option<&'a [u8]>,
    /// The version in the same object whose node rows hold this version's
    /// content, when its content equalled the preceding tip's
    /// (`version.content_sys_version`, #4415); `None` for its own rows.
    pub content_sys_version: Option<i32>,
}

/// The head upsert both folded commit statements carry.
///
/// One mutable row per versioned object, written in the same statement as the
/// version row it describes. A TRUNK commit advances every column; a BRANCH
/// commit advances only what a branch changes — the any-lineage head ordinal
/// and the commit instant — leaving the trunk's own head, its content key
/// (the version insert's `content_key`, #4944), lifecycle state and template
/// alone, because a branch does not supersede the trunk (RM common
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
             trunk_head_sys_version, trunk_head_content_key, lifecycle_state, template_id, \
             committed_at) \
         SELECT {vo}, {kind}, {ehr}, {ordinal}, \
                CASE WHEN {branch} = 0 THEN {ordinal} ELSE NULL END, \
                CASE WHEN {branch} = 0 THEN v.content_key ELSE NULL END, \
                {lifecycle}, {template}, {committed_at}::timestamptz \
         FROM v \
         ON CONFLICT (vo_id) DO UPDATE SET \
             head_sys_version = EXCLUDED.head_sys_version, \
             committed_at = EXCLUDED.committed_at, \
             trunk_head_sys_version = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.head_sys_version ELSE vo_head.trunk_head_sys_version END, \
             trunk_head_content_key = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.trunk_head_content_key ELSE vo_head.trunk_head_content_key END, \
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

/// What one folded commit statement writes beside its `commit_audit` row.
///
/// Each shape is one data-modifying CTE chain, run inside the caller's
/// transaction: the round-trip-collapsed equivalent of the separate inserts,
/// byte-identical in the rows written and the values returned. No openEHR spec
/// governs statement batching: our own design.
#[derive(Debug, Clone, Copy)]
pub enum Folded<'a> {
    /// The CONTRIBUTION `id` and its own audit, the versions of its change set
    /// following under [`Folded::Into`]. `ehr_id` is `None` for a demographic
    /// CONTRIBUTION. The audit's `time_committed` is the database's.
    Contribution {
        /// The CONTRIBUTION uid, server-minted or client-supplied.
        id: Uuid,
        /// The owning EHR.
        ehr_id: Option<EhrId>,
    },
    /// A standalone version under the new CONTRIBUTION `id`, the one audit row
    /// serving both (a direct write is one CONTRIBUTION of one change, master06
    /// §Committal and Audits).
    Standalone {
        /// The CONTRIBUTION uid.
        id: Uuid,
        /// The version, its head upsert and its node rows.
        version: &'a FoldedVersion<'a>,
    },
    /// A version within the CONTRIBUTION `id` an earlier
    /// [`Folded::Contribution`] of this transaction wrote, under the version's
    /// own `commit_audit`.
    Into {
        /// The CONTRIBUTION uid.
        id: Uuid,
        /// The version, its head upsert and its node rows.
        version: &'a FoldedVersion<'a>,
    },
}

/// The rows one folded commit wrote.
#[derive(Debug, Clone, Copy)]
pub struct FoldedRows {
    /// The CONTRIBUTION the statement wrote or wrote into.
    pub contribution_id: Uuid,
    /// The `commit_audit` row the statement inserted.
    pub commit_audit_id: Uuid,
    /// The audit's `time_committed` as stored (master06 §Committal and Audits).
    pub time_committed: jiff::Timestamp,
}

/// The three statement texts, one per [`Folded`] variant.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// [`Folded::Contribution`].
    Contribution,
    /// [`Folded::Standalone`].
    Standalone,
    /// [`Folded::Into`].
    Into,
}

/// The statement text of one [`Folded`] shape.
///
/// Every shape binds the same parameters in the same order: `$1` to `$5` the
/// audit, `$6` the CONTRIBUTION uid, `$7` the owning EHR, `$8` to `$26` the
/// version and `$27` on the node arrays, so one binder serves all three.
fn folded_statement(shape: Shape) -> String {
    let contribution = matches!(shape, Shape::Contribution | Shape::Standalone);
    let version = matches!(shape, Shape::Standalone | Shape::Into);
    let audit_time = if version { ", time_committed" } else { "" };
    let audit_time_value = if version { ", $22::timestamptz" } else { "" };
    let contribution_cte = if contribution {
        ", c AS ( \
             INSERT INTO contribution (id, ehr_id, commit_audit_id) \
             SELECT $6, $7, a.id FROM a \
             ON CONFLICT (id) DO NOTHING \
             RETURNING id \
         )"
    } else {
        ""
    };
    let version_cte = if version {
        let (contribution_id, from) = if contribution {
            ("c.id", "a, c")
        } else {
            ("$6", "a")
        };
        format!(
            ", v AS ( \
                 INSERT INTO version \
                   (vo_id, kind, ehr_id, sys_version, trunk_version, branch_number, branch_version, \
                    lifecycle_state, creating_system_id, preceding_version_uid, \
                    contribution_id, commit_audit_id, template_id, signature, \
                    signature_client_supplied, stable_compatible, body, origins, committed_at, \
                    other_input_version_uids, content_digest, content_sys_version) \
                 SELECT $8, $9, $7, $10, $11, $12, $13, \
                        $14, $15, $16, {contribution_id}, a.id, $17, $18, $19, $20, $21, $23, \
                        $22::timestamptz, $24, $25, $26 \
                 FROM {from} \
                 RETURNING content_key \
             ), h AS ( {} \
             ), n AS ( {} )",
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
            crate::storage::node_write::node_insert_cte("$8", "$10", "$7", 27)
        )
    } else {
        String::new()
    };
    let select = if contribution {
        " SELECT a.id AS commit_audit_id, a.time_committed, c.id AS contribution_id \
          FROM a LEFT JOIN c ON true"
    } else {
        " SELECT a.id AS commit_audit_id, a.time_committed, $6::uuid AS contribution_id FROM a"
    };
    format!(
        "WITH a AS ( \
             INSERT INTO commit_audit (system_id, change_type, description, committer, \
                                       attestation{audit_time}) \
             VALUES ($1, $2, $3, $4, $5{audit_time_value}) RETURNING id, time_committed \
         ){contribution_cte}{version_cte}{select}"
    )
}

/// Run one folded commit statement ([`Folded`]), returning the rows it wrote.
///
/// A version's audit `time_committed` and its row's `committed_at` are both the
/// caller's bound [`FoldedVersion::time_committed`], the instant its
/// `VERSION.signature` was computed over, so stored and signed agree by
/// construction. The externalized multimedia URIs a version's node rows
/// reference are indexed in the same transaction
/// ([`crate::storage::blob_ref::record`]), which issues no statement when there
/// are none.
///
/// On a CONTRIBUTION uid already in use the `contribution` CTE inserts nothing
/// (`ON CONFLICT DO NOTHING`), so the outer `LEFT JOIN` yields a NULL id; the
/// audit row is discarded when the transaction unwinds.
///
/// # Errors
/// Returns [`StorageError::ContributionUidInUse`] when a new CONTRIBUTION's uid
/// is already in use (ITS-REST `contribution_create`: a duplicate is a
/// conflict, never an overwrite), else [`StorageError::Database`] on a
/// driver/insert failure.
pub async fn commit_folded(
    tx: &mut PgConnection,
    audit: &AuditRow<'_>,
    folded: Folded<'_>,
) -> Result<FoldedRows, StorageError> {
    static CONTRIBUTION: LazyLock<String> = LazyLock::new(|| folded_statement(Shape::Contribution));
    static STANDALONE: LazyLock<String> = LazyLock::new(|| folded_statement(Shape::Standalone));
    static INTO: LazyLock<String> = LazyLock::new(|| folded_statement(Shape::Into));
    let (sql, id, ehr_id, version) = match folded {
        Folded::Contribution { id, ehr_id } => (&CONTRIBUTION, id, ehr_id, None),
        Folded::Standalone { id, version } => (&STANDALONE, id, version.ehr_id, Some(version)),
        Folded::Into { id, version } => (&INTO, id, version.ehr_id, Some(version)),
    };
    let query = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(audit.system_id)
        .bind(audit.change_type)
        .bind(&audit.description)
        .bind(&audit.committer)
        .bind(&audit.attestation)
        .bind(id)
        .bind(ehr_id);
    let row = match version {
        None => query.fetch_one(&mut *tx).await?,
        Some(v) => {
            let query = query
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
                .bind(serde_json::json!(v.origins))
                .bind(super::optional_json_array(v.other_input_version_uids))
                .bind(v.content_digest)
                .bind(v.content_sys_version);
            let version_uid = crate::storage::compact::version_uid(
                v.vo_id,
                v.creating_system_id,
                v.trunk_version,
                v.branch_number,
                v.branch_version,
            );
            let root = crate::storage::compact::RootIdentity {
                version_uid: &version_uid,
                template_id: v.template_id,
            };
            let node_refs: Vec<(&crate::storage::row::NodeRow, _)> =
                v.rows.iter().map(|row| (row, &root)).collect();
            crate::storage::node_write::bind_node_arrays(query, &node_refs)
                .fetch_one(&mut *tx)
                .await?
        }
    };
    let contribution_id: Option<Uuid> = row.try_get("contribution_id")?;
    let rows = FoldedRows {
        contribution_id: contribution_id.ok_or(StorageError::ContributionUidInUse(Some(id)))?,
        commit_audit_id: row.try_get("commit_audit_id")?,
        time_committed: row
            .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
            .to_jiff(),
    };
    if let Some(v) = version {
        crate::storage::blob_ref::record(tx, &[(v.vo_id, v.sys_version, v.rows)]).await?;
    }
    Ok(rows)
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
/// envelope shape the outbox feed serves (`{contribution_id, ehr_id,
/// committed_at, versions[]}`). The insert takes the outbox lock shared and
/// holds it to the commit's end, so a feed read never passes a `seq` this
/// commit drew ([`crate::extensions::outbox`]).
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
        "WITH gate AS (SELECT pg_advisory_xact_lock_shared( \
             hashtext(current_schema() || '.event_outbox'), 0)) \
         INSERT INTO event_outbox (contribution_id, ehr_id, envelope, committed_at) \
         SELECT $1, $2, $3, $4::timestamptz FROM gate \
         WHERE NOT EXISTS (SELECT 1 FROM ehr e WHERE e.id = $2 \
             AND (e.restricted_at IS NOT NULL \
                  OR (e.research_objected_at IS NOT NULL \
                      AND e.research_objection_ground IS NULL)))"
    } else {
        "WITH gate AS (SELECT pg_advisory_xact_lock_shared( \
             hashtext(current_schema() || '.event_outbox'), 0)) \
         INSERT INTO event_outbox (contribution_id, ehr_id, envelope, committed_at) \
         SELECT $1, $2, $3, $4::timestamptz FROM gate"
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
