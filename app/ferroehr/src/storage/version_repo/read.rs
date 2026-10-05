// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The full version reads.
//!
//! One `version`⋈`audit` statement (attestations folded in as a LATERAL
//! aggregate, the canonical body read off the row's own materialized `body`
//! column), yielding the [`StoredVersion`] shape the versioning layer maps
//! into a `VERSION`/`ORIGINAL_VERSION`.
//!
//! No openEHR spec governs the SQL — our own design. The version-access semantics realized are RM common master06
//! (§Versioned Objects, §Logical Deletion) and master08 §Change Management
//! (time-travel).
//!
//! NOTE (no openEHR spec governs storage tiering — our own design): every full
//! version read names the partitioned parents `version`/`vo_attestation` and
//! pins no tier, so ONE statement serves both partitions — an archived object
//! stays retrievable and a miss never pays a cold-tier retry transaction.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// A loaded `version`⋈`audit` row plus its reassembled content and
/// attestations — the storage read shape the versioning layer maps into a
/// `VERSION`/`ORIGINAL_VERSION`.
///
/// The tree is returned as its three column ints; the audit fields are
/// flattened (versioning rebuilds the `AUDIT_DETAILS`).
#[derive(Debug, Clone)]
pub struct StoredVersion {
    /// The versioned object's id.
    pub vo_id: VoId,
    /// The `version.kind` discriminator text (`COMPOSITION` / `EHR_STATUS` /
    /// `FOLDER` / …).
    pub kind: String,
    /// The owning EHR, or `None` for a demographic party (no EHR scope).
    pub ehr_id: Option<EhrId>,
    /// The per-vo storage commit ordinal.
    pub sys_version: i32,
    /// `VERSION_TREE_ID` first part.
    pub trunk_version: i32,
    /// `VERSION_TREE_ID` second part; `0` on a trunk row.
    pub branch_number: i32,
    /// `VERSION_TREE_ID` third part; `0` on a trunk row.
    pub branch_version: i32,
    /// `ORIGINAL_VERSION.preceding_version_uid`; `None` for a first version.
    pub preceding_version_uid: Option<String>,
    /// The merge provenance (`other_input_version_uids`); empty when not a merge.
    pub other_input_version_uids: Vec<String>,
    /// The `version_lifecycle_state` numeric code.
    pub lifecycle_state: String,
    /// The version's immutable `creating_system_id`.
    pub creating_system_id: String,
    /// The CONTRIBUTION this version was committed in — on an imported row the
    /// LOCAL import CONTRIBUTION (RM common master06 §Committal and Audits).
    pub contribution_id: Uuid,
    /// The version's `commit_audit` fields (master04 §Audit Details), flattened.
    pub audit_system_id: String,
    /// The version audit's numeric `audit_change_type` group code.
    pub audit_change_type: String,
    /// The canonical `DV_TEXT` fragment of the version audit's description,
    /// when the committer supplied one.
    pub audit_description: Option<Value>,
    /// Canonical `PARTY_PROXY` of the committer.
    pub audit_committer: Value,
    /// The canonical fragment of the `ATTESTATION`-declared attributes when the
    /// commit audit is an `ATTESTATION` (RM common master06 §Attestation).
    pub audit_attestation: Option<Value>,
    /// Server-computed commit time (master06 §Committal and Audits).
    pub time_committed: jiff::Timestamp,
    /// The OPT `template_id` a COMPOSITION was committed against (else `None`).
    pub template_id: Option<String>,
    /// `VERSION.signature` (0..1), opaque radix-64 — on an imported row the
    /// `IMPORTED_VERSION` wrapper's own.
    pub signature: Option<String>,
    /// Whether `signature` was supplied verbatim by the client (foreign — never
    /// re-verified at read; master06 §Digital Signature) rather than generated
    /// by this server.
    pub signature_client_supplied: bool,
    /// On an `IMPORTED_VERSION` row, the wrapped `ORIGINAL_VERSION`'s own
    /// `{contribution, commit_audit, signature?}` fragment; `None` on a locally
    /// created version (master06 §Committal and Audits).
    pub wrapped_original: Option<Value>,
    /// Whether the RELEASED openEHR generation set can express this version's
    /// body, as stamped at commit; `None` for a row nothing stamped (committed
    /// before the column existed, or written by a verbatim-replay path — the
    /// EHR-Extract import and the archive load), which the read-time
    /// `spec_profile` gate assesses on the fly. No openEHR spec governs runtime
    /// generation selection — our own design/extension.
    pub stable_compatible: Option<bool>,
    /// The commit-time origin stamp (`version.origins`), a JSON array of
    /// system ids; `None` for a row nothing stamped, which
    /// the read path (`versioning::origins::of_stored`) assesses from the body.
    pub origins: Option<Value>,
    /// The restriction-of-processing mark of the versioned object this version
    /// belongs to (`vo_head.restricted_at`), `None` when it is unrestricted.
    /// GDPR Art. 18(2) leaves storage as the only processing a restriction
    /// admits (`docs/law/eu/gdpr/text.html`), so the one seam a stored body
    /// leaves storage through refuses a marked object. No openEHR spec governs
    /// restriction of processing — our own design/extension.
    pub restricted_at: Option<jiff::Timestamp>,
    /// The materialized canonical body (`version.body` — written from the
    /// same value the node rows decompose from), or [`Value::Null`] for a
    /// logically deleted version (master06 §Logical Deletion). [`Value::Null`]
    /// on a raw read that populated [`Self::canonical_text`] instead.
    pub canonical: Value,
    /// The body as the database's own jsonb text rendering, populated ONLY by
    /// the raw read variants ([`read_current_raw`] / [`read_version_raw`]) —
    /// the JSON-accept passthrough source. `None` on every parsed read and on
    /// a logically deleted version.
    pub canonical_text: Option<String>,
    /// The `ORIGINAL_VERSION.attestations` that were on the version AT the act
    /// of committal, in commit order (master06 §Attestation "Signing content at
    /// committal") — the ones inside the version's signed canonical form.
    pub attestations_at_committal: Vec<Value>,
    /// The `ORIGINAL_VERSION.attestations` added AFTER committal, in commit
    /// order (master06 §Attestation: "Attestations can be added at any time
    /// after committal of the content being attested") — outside the signed
    /// canonical form.
    pub attestations_after_committal: Vec<Value>,
}

/// The `version`⋈`audit` column list every version read selects, as a
/// compile-time string concatenation so each query stays a static literal
/// (`sqlx` 0.9 `SqlSafeStr` — no runtime SQL assembly).
///
/// The version's `ATTESTATION`s (RM common master06 §Attestation) are folded in
/// as aggregated jsonb columns via a `LEFT JOIN LATERAL`, so one statement
/// carries the whole version read instead of a second round trip per versioned
/// read (empty in the common case → `[]`). The aggregates'
/// `ORDER BY time_committed, id` is the same commit order the per-object
/// attestation read
/// ([`crate::storage::version_repo::attestation::read_attestations_all`]) applies.
///
/// They arrive split on `at_committal`, the two halves standing in different
/// relations to `VERSION.signature`: an attestation present at the act of
/// committal is inside the version's signed canonical form ("serialising the
/// entire Version object", master06 §Digital Signature) while one added later is
/// not (§Attestation). The split uses the standard aggregate `FILTER` clause
/// (<https://www.postgresql.org/docs/18/sql-expressions.html#SYNTAX-AGGREGATES>).
///
/// The version's canonical body is the row's own `v.body` column, materialized
/// at write time from the same value the node rows decompose from, so a point
/// read is one statement and one TOAST detoast rather than a node-subtree
/// re-aggregation. `NULL` is a logically deleted version (RM common master06
/// §Logical Deletion).
macro_rules! version_select {
    ($tail:literal) => {
        concat!(
            "SELECT v.vo_id, v.kind, v.ehr_id, v.sys_version, v.trunk_version, v.branch_number, ",
            "v.branch_version, v.lifecycle_state, v.creating_system_id, v.preceding_version_uid, ",
            "v.other_input_version_uids, v.contribution_id, v.template_id, v.signature, ",
            "v.signature_client_supplied, v.wrapped_original, v.stable_compatible, v.origins, ",
            "v.body, h.restricted_at, ",
            "a.system_id, a.change_type, a.description, a.committer, a.attestation, ",
            "a.time_committed, ",
            "att.attestations_at_committal, att.attestations_after_committal ",
            "FROM version v JOIN commit_audit a ON a.id = v.commit_audit_id ",
            "JOIN vo_head h ON h.vo_id = v.vo_id ",
            "LEFT JOIN LATERAL (",
            "SELECT coalesce(jsonb_agg(x.data ORDER BY x.time_committed, x.id) ",
            "FILTER (WHERE x.at_committal), '[]'::jsonb) AS attestations_at_committal, ",
            "coalesce(jsonb_agg(x.data ORDER BY x.time_committed, x.id) ",
            "FILTER (WHERE NOT x.at_committal), '[]'::jsonb) AS attestations_after_committal ",
            "FROM vo_attestation x ",
            "WHERE x.vo_id = v.vo_id AND x.sys_version = v.sys_version",
            ") att ON true ",
            $tail
        )
    };
}

/// [`version_select!`] — with the body column already text, the
/// raw-read list for the JSON-accept passthrough ([`read_current_raw`] /
/// [`read_version_raw`]) is the same column list; the macro pair survives so
/// the two intents keep their own names.
macro_rules! version_select_raw {
    ($tail:literal) => {
        concat!(
            "SELECT v.vo_id, v.kind, v.ehr_id, v.sys_version, v.trunk_version, v.branch_number, ",
            "v.branch_version, v.lifecycle_state, v.creating_system_id, v.preceding_version_uid, ",
            "v.other_input_version_uids, v.contribution_id, v.template_id, v.signature, ",
            "v.signature_client_supplied, v.wrapped_original, v.stable_compatible, ",
            "v.origins, v.body, h.restricted_at, ",
            "a.system_id, a.change_type, a.description, a.committer, a.attestation, ",
            "a.time_committed, ",
            "att.attestations_at_committal, att.attestations_after_committal ",
            "FROM version v JOIN commit_audit a ON a.id = v.commit_audit_id ",
            "JOIN vo_head h ON h.vo_id = v.vo_id ",
            "LEFT JOIN LATERAL (",
            "SELECT coalesce(jsonb_agg(x.data ORDER BY x.time_committed, x.id) ",
            "FILTER (WHERE x.at_committal), '[]'::jsonb) AS attestations_at_committal, ",
            "coalesce(jsonb_agg(x.data ORDER BY x.time_committed, x.id) ",
            "FILTER (WHERE NOT x.at_committal), '[]'::jsonb) AS attestations_after_committal ",
            "FROM vo_attestation x ",
            "WHERE x.vo_id = v.vo_id AND x.sys_version = v.sys_version",
            ") att ON true ",
            $tail
        )
    };
}

// ── the rendered statements ──────────────────────────────────────────────────
// The three point reads below expose the statement they run as a named
// constant. Their access path is pinned by the plan-shape suite
// (`tests/it/storage_plans.rs`), which runs EXPLAIN over these constants, so
// what it explains is the statement the read path emits and not a copy that can
// drift away from it.

/// The statement [`read_current`] runs: the current trunk version of an object
/// by id, resolved through the head row's `trunk_head_sys_version`.
pub const READ_CURRENT_SQL: &str =
    version_select!("WHERE v.vo_id = $1 AND h.trunk_head_sys_version = v.sys_version");

/// The statement [`version_at`] runs: the trunk version of an object in force
/// at an instant.
pub const VERSION_AT_SQL: &str = version_select!(
    "WHERE v.vo_id = $1 AND v.branch_number = 0 \
     AND v.committed_at <= $2::timestamptz \
     AND NOT EXISTS (SELECT 1 FROM version s \
                     WHERE s.vo_id = v.vo_id AND s.branch_number = 0 \
                       AND s.committed_at > v.committed_at \
                       AND s.committed_at <= $2::timestamptz)"
);

/// The statement [`read_version_by_ordinal`] runs: one version of an object by
/// its storage ordinal, addressed on the partitioned parent.
pub const READ_VERSION_BY_ORDINAL_SQL: &str =
    version_select!("WHERE v.vo_id = $1 AND v.sys_version = $2");

/// Build a [`StoredVersion`] from a `version`⋈`audit` row; the canonical
/// body is the row's own materialized `body` column (`NULL` → [`Value::Null`],
/// a logical delete), so the whole version read is the ONE statement that
/// produced `row`, on whichever tier's connection ran it.
fn stored_version(vo_id: VoId, row: &PgRow) -> Result<StoredVersion, StorageError> {
    let canonical = match row.try_get::<Option<String>, _>("body")? {
        Some(text) => serde_json::from_str(&text).map_err(StorageError::BodyDecode)?,
        None => Value::Null,
    };
    stored_version_fields(vo_id, row, canonical, None)
}

/// [`stored_version`] for a raw-read row: the body text arrives as the
/// stored canonical bytes, kept verbatim in
/// [`StoredVersion::canonical_text`] with `canonical = Value::Null` — the
/// JSON-accept passthrough source (the caller parses the text wherever a
/// typed value is still needed).
fn stored_version_raw(vo_id: VoId, row: &PgRow) -> Result<StoredVersion, StorageError> {
    let canonical_text: Option<String> = row.try_get("body")?;
    stored_version_fields(vo_id, row, Value::Null, canonical_text)
}

/// The shared `version`⋈`audit` field mapping of [`stored_version`] /
/// [`stored_version_raw`] — everything except the body representation, which
/// the two builders extract each their own way.
fn stored_version_fields(
    vo_id: VoId,
    row: &PgRow,
    canonical: Value,
    canonical_text: Option<String>,
) -> Result<StoredVersion, StorageError> {
    let sys_version: i32 = row.try_get("sys_version")?;
    // A stored `other_input_version_uids` that does not decode is OUR data
    // gone wrong (the merge inputs of an IMPORTED_VERSION, RM common master06
    // §Distributed Versioning) — serving the version with an empty merge list
    // would answer wrongly rather than loudly, so the decode failure is a codec
    // fault, not a default.
    let other_input_version_uids: Vec<String> = row
        .try_get::<Option<Value>, _>("other_input_version_uids")?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| {
            StorageError::InvalidRows(format!(
                "version.other_input_version_uids of {vo_id} is not a list of \
                 version uids: {e}"
            ))
        })?
        .unwrap_or_default();
    // The attestations arrive folded into the version-select row (the LATERAL
    // aggregates in `version_select!`), in commit order and already split on
    // `at_committal` — no separate round trip.
    let attestation_list = |column: &str| -> Result<Vec<Value>, StorageError> {
        Ok(row
            .try_get::<Value, _>(column)?
            .as_array()
            .cloned()
            .unwrap_or_default())
    };
    let attestations_at_committal = attestation_list("attestations_at_committal")?;
    let attestations_after_committal = attestation_list("attestations_after_committal")?;
    Ok(StoredVersion {
        vo_id,
        kind: row.try_get("kind")?,
        ehr_id: row.try_get("ehr_id")?,
        sys_version,
        trunk_version: row.try_get("trunk_version")?,
        branch_number: row.try_get("branch_number")?,
        branch_version: row.try_get("branch_version")?,
        preceding_version_uid: row.try_get("preceding_version_uid")?,
        other_input_version_uids,
        lifecycle_state: row.try_get("lifecycle_state")?,
        creating_system_id: row.try_get("creating_system_id")?,
        contribution_id: row.try_get("contribution_id")?,
        audit_system_id: row.try_get("system_id")?,
        audit_change_type: row.try_get("change_type")?,
        audit_description: row.try_get("description")?,
        audit_committer: row.try_get("committer")?,
        audit_attestation: row.try_get("attestation")?,
        time_committed: row
            .try_get::<jiff_sqlx::Timestamp, _>("time_committed")?
            .to_jiff(),
        template_id: row.try_get("template_id")?,
        signature: row.try_get("signature")?,
        signature_client_supplied: row.try_get("signature_client_supplied")?,
        wrapped_original: row.try_get("wrapped_original")?,
        stable_compatible: row.try_get("stable_compatible")?,
        origins: row.try_get("origins")?,
        restricted_at: row
            .try_get::<Option<jiff_sqlx::Timestamp>, _>("restricted_at")?
            .map(jiff_sqlx::Timestamp::to_jiff),
        canonical,
        canonical_text,
        attestations_at_committal,
        attestations_after_committal,
    })
}

/// One version a batched `spec_profile` gate still has to decide on: the
/// identity facts its refusal names, plus the root node's nested-set interval
/// so the body can be batch-loaded.
///
/// No openEHR spec governs runtime generation selection — our own
/// design/extension.
#[derive(Debug, Clone)]
pub struct ProfileGateCandidate {
    /// The versioned object's id.
    pub vo_id: VoId,
    /// The per-vo storage commit ordinal.
    pub sys_version: i32,
    /// The `version.kind` discriminator text.
    pub kind: String,
    /// The stored stamp: `Some(false)` (the released generations cannot express
    /// the body) or `None` (nothing stamped the row).
    pub stable_compatible: Option<bool>,
    /// The version's immutable `creating_system_id`.
    pub creating_system_id: String,
    /// `VERSION_TREE_ID` first part.
    pub trunk_version: i32,
    /// `VERSION_TREE_ID` second part; `0` on a trunk row.
    pub branch_number: i32,
    /// `VERSION_TREE_ID` third part; `0` on a trunk row.
    pub branch_version: i32,
    /// The root node's `[num, num_cap]` interval, or `None` for a logically
    /// deleted version (no node rows — RM common master06 §Logical Deletion).
    pub root_interval: Option<(i32, i32)>,
}

/// Read, in ONE statement, the versions among `versions` that are NOT stamped
/// stable-compatible — the only ones a `stable`-profile gate has left to decide.
///
/// `versions` is a `(vo_id, sys_version)` pair list (the primary-key columns, so
/// the join is a key lookup). A version stamped `true` is served on the stamp
/// alone and never comes back here, which is what keeps the common page free of
/// per-row work; the rows returned are ordered by `(vo_id, sys_version)` so a
/// refusal names the same version on every execution.
///
/// AQL reads the primary tier only (an archived object leaves the queryable
/// store), so no cold-tier retry applies here — unlike the point reads above.
///
/// # Errors
/// Returns [`StorageError`] on a driver failure.
pub async fn read_profile_gate_candidates(
    pool: &PgPool,
    versions: &[(VoId, i32)],
) -> Result<Vec<ProfileGateCandidate>, StorageError> {
    if versions.is_empty() {
        return Ok(Vec::new());
    }
    let vo_ids: Vec<Uuid> = versions.iter().map(|(vo_id, _)| vo_id.0).collect();
    let sys_versions: Vec<i32> = versions.iter().map(|(_, sv)| *sv).collect();
    let rows = sqlx::query(
        "SELECT v.vo_id, v.sys_version, v.kind, v.stable_compatible, v.creating_system_id, \
                v.trunk_version, v.branch_number, v.branch_version, \
                r.num AS root_num, r.num_cap AS root_num_cap \
         FROM unnest($1::uuid[], $2::int[]) AS a(vo_id, sys_version) \
         JOIN version v ON v.vo_id = a.vo_id AND v.sys_version = a.sys_version \
         LEFT JOIN node r ON r.vo_id = v.vo_id AND r.sys_version = v.sys_version AND r.num = 0 \
         WHERE v.stable_compatible IS NOT TRUE \
         ORDER BY v.vo_id, v.sys_version",
    )
    .bind(&vo_ids)
    .bind(&sys_versions)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let root_num: Option<i32> = row.try_get("root_num")?;
        let root_num_cap: Option<i32> = row.try_get("root_num_cap")?;
        out.push(ProfileGateCandidate {
            vo_id: row.try_get("vo_id")?,
            sys_version: row.try_get("sys_version")?,
            kind: row.try_get("kind")?,
            stable_compatible: row.try_get("stable_compatible")?,
            creating_system_id: row.try_get("creating_system_id")?,
            trunk_version: row.try_get("trunk_version")?,
            branch_number: row.try_get("branch_number")?,
            branch_version: row.try_get("branch_version")?,
            root_interval: root_num.zip(root_num_cap),
        });
    }
    Ok(out)
}

/// Read the current TRUNK version of an object by id (any kind).
///
/// `None` if it never existed (`latest_trunk_version`, master06 §Versioned
/// Objects). A deleted current version returns with `canonical = Null` and
/// its `523` lifecycle so the caller can distinguish 404 from a deleted read.
///
/// # Errors
/// Returns [`StorageError`] on a driver/reassembly failure.
pub async fn read_current(
    pool: &PgPool,
    vo_id: VoId,
) -> Result<Option<StoredVersion>, StorageError> {
    sqlx::query(READ_CURRENT_SQL)
        .bind(vo_id)
        .fetch_optional(pool)
        .await?
        .map(|row| stored_version(vo_id, &row))
        .transpose()
}

/// [`read_current`] with the body as its stored jsonb text — the
/// JSON-accept passthrough read ([`StoredVersion::canonical_text`] carries
/// the text; `canonical` is [`Value::Null`]).
///
/// # Errors
/// Returns [`StorageError`] on a driver failure.
pub async fn read_current_raw(
    pool: &PgPool,
    vo_id: VoId,
) -> Result<Option<StoredVersion>, StorageError> {
    const SQL: &str =
        version_select_raw!("WHERE v.vo_id = $1 AND h.trunk_head_sys_version = v.sys_version");
    sqlx::query(SQL)
        .bind(vo_id)
        .fetch_optional(pool)
        .await?
        .map(|row| stored_version_raw(vo_id, &row))
        .transpose()
}

/// [`read_version`] with the body as its stored jsonb text — the
/// JSON-accept passthrough read.
///
/// # Errors
/// Returns [`StorageError`] on a driver failure.
pub async fn read_version_raw(
    pool: &PgPool,
    vo_id: VoId,
    trunk_version: i32,
    branch_number: i32,
    branch_version: i32,
) -> Result<Option<StoredVersion>, StorageError> {
    const SQL: &str = version_select_raw!(
        "WHERE v.vo_id = $1 AND v.trunk_version = $2 \
         AND v.branch_number = $3 AND v.branch_version = $4"
    );
    sqlx::query(SQL)
        .bind(vo_id)
        .bind(trunk_version)
        .bind(branch_number)
        .bind(branch_version)
        .fetch_optional(pool)
        .await?
        .map(|row| stored_version_raw(vo_id, &row))
        .transpose()
}

/// Read the current TRUNK versions of a SET of objects in ONE statement.
///
/// The extract export's demographics chapter resolves every referenced party
/// with one round trip instead of a point read per party. Objects that never
/// existed are simply missing from the result.
///
/// # Errors
/// Returns [`StorageError`] on a driver/reassembly failure.
pub async fn read_currents(
    pool: &PgPool,
    vo_ids: &[VoId],
) -> Result<Vec<StoredVersion>, StorageError> {
    const SQL: &str =
        version_select!("WHERE v.vo_id = ANY($1) AND h.trunk_head_sys_version = v.sys_version");
    if vo_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = vo_ids.iter().map(|id| id.0).collect();
    let rows = sqlx::query(SQL).bind(&ids).fetch_all(pool).await?;
    rows.into_iter()
        .map(|row| {
            let vo_id: VoId = row.try_get("vo_id")?;
            stored_version(vo_id, &row)
        })
        .collect()
}

/// Read a specific version by its STORAGE ORDINAL (`sys_version`) — for internal
/// callers that key rows by ordinal (the FHIR mapping table, extract export
/// iteration), never for wire version ids.
///
/// # Errors
/// Returns [`StorageError`] on a driver/reassembly failure.
pub async fn read_version_by_ordinal(
    pool: &PgPool,
    vo_id: VoId,
    ordinal: i32,
) -> Result<Option<StoredVersion>, StorageError> {
    sqlx::query(READ_VERSION_BY_ORDINAL_SQL)
        .bind(vo_id)
        .bind(ordinal)
        .fetch_optional(pool)
        .await?
        .map(|row| stored_version(vo_id, &row))
        .transpose()
}

/// Read a specific version by its `VERSION_TREE_ID` column ints (for
/// `.../version/{version_uid}` — trunk or branch; master05 §Syntaxes).
///
/// # Errors
/// Returns [`StorageError`] on a driver/reassembly failure.
pub async fn read_version(
    pool: &PgPool,
    vo_id: VoId,
    trunk_version: i32,
    branch_number: i32,
    branch_version: i32,
) -> Result<Option<StoredVersion>, StorageError> {
    const SQL: &str = version_select!(
        "WHERE v.vo_id = $1 AND v.trunk_version = $2 \
         AND v.branch_number = $3 AND v.branch_version = $4"
    );
    sqlx::query(SQL)
        .bind(vo_id)
        .bind(trunk_version)
        .bind(branch_number)
        .bind(branch_version)
        .fetch_optional(pool)
        .await?
        .map(|row| stored_version(vo_id, &row))
        .transpose()
}

/// Read a SET of specific versions in ONE statement.
///
/// Each is addressed by `(vo_id, VERSION_TREE_ID columns)` — the
/// resolved-CONTRIBUTION reader's batch (a K-member CONTRIBUTION resolves
/// its members without K point reads). Absent versions are simply missing
/// from the result; the caller maps absence to its own refusal.
///
/// # Errors
/// Returns [`StorageError`] on a driver/decode failure.
pub async fn read_versions_by_tree(
    pool: &PgPool,
    refs: &[(VoId, (i32, i32, i32))],
) -> Result<Vec<StoredVersion>, StorageError> {
    const SQL: &str = version_select!(
        "JOIN unnest($1::uuid[], $2::int[], $3::int[], $4::int[]) \
         AS q(vo_id, trunk_version, branch_number, branch_version) \
         ON q.vo_id = v.vo_id AND q.trunk_version = v.trunk_version \
         AND q.branch_number = v.branch_number AND q.branch_version = v.branch_version"
    );
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let vo_ids: Vec<Uuid> = refs.iter().map(|(id, _)| id.0).collect();
    let trunks: Vec<i32> = refs.iter().map(|(_, (t, _, _))| *t).collect();
    let branches: Vec<i32> = refs.iter().map(|(_, (_, b, _))| *b).collect();
    let branch_versions: Vec<i32> = refs.iter().map(|(_, (_, _, bv))| *bv).collect();
    let rows = sqlx::query(SQL)
        .bind(&vo_ids)
        .bind(&trunks)
        .bind(&branches)
        .bind(&branch_versions)
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|row| {
            let vo_id: VoId = row.try_get("vo_id")?;
            stored_version(vo_id, &row)
        })
        .collect()
}

/// Read the version of an object current at a given instant (time-travel).
///
/// The TRUNK row in force at `at` is the one with the greatest `committed_at`
/// at or before it (master08 §Change Management — any previous state
/// reconstructable).
///
/// `None` if the object had no trunk version then.
///
/// NOTE: `VERSIONED_OBJECT.version_at_time (a_time): VERSION[1]` returns exactly
/// one version (`UML/classes/org.openehr.rm.common.versioned_object.adoc`
/// §Functions), while a container may hold several valid tips at an instant, so
/// the read is restricted to the trunk, the lineage that makes the answer
/// unique.
///
/// A container holding branches but no trunk version is a state RM common
/// `master06-change_control_package.adoc` §Copying §Subsequent Local
/// Modifications rules out, branch versions never being copied without their
/// trunk versions, so every branch in a well-formed container has its trunk
/// ancestry beside it.
///
/// NOTE: the tie is unrepresentable rather than resolved — two trunk rows at one
/// `committed_at` would both satisfy the strict `>`, and the one path that
/// writes historical instants (the archive load) refuses that record outright.
///
/// # Errors
/// Returns [`StorageError`] on a driver/reassembly failure.
pub async fn version_at(
    pool: &PgPool,
    vo_id: VoId,
    at: jiff::Timestamp,
) -> Result<Option<StoredVersion>, StorageError> {
    let at = at.to_string();
    sqlx::query(VERSION_AT_SQL)
        .bind(vo_id)
        .bind(&at)
        .fetch_optional(pool)
        .await?
        .map(|row| stored_version(vo_id, &row))
        .transpose()
}

/// Decode a scoped-read row through [`stored_version`], keyed by the row's
/// own `vo_id` (the scoped reads resolve the container and the version in the
/// same statement, so no caller-supplied id exists).
fn stored_version_by_row(row: &PgRow) -> Result<StoredVersion, StorageError> {
    let vo_id: VoId = row.try_get("vo_id")?;
    stored_version(vo_id, row)
}

/// Read the current TRUNK version of the EHR's one container of `kind` in ONE
/// statement — the container resolution and the version read merged.
///
/// Valid only for the kinds an EHR holds exactly one container of
/// (`EHR_STATUS` — RM ehr `ehr.adoc` declares `ehr_status` singular);
/// `None` when the EHR has no such container.
///
/// # Errors
/// Returns [`StorageError`] on a driver/decode failure.
pub async fn read_current_of_kind(
    pool: &PgPool,
    ehr_id: EhrId,
    kind: &str,
) -> Result<Option<StoredVersion>, StorageError> {
    const SQL: &str = version_select!(
        "WHERE v.ehr_id = $1 AND v.kind = $2 AND h.trunk_head_sys_version = v.sys_version"
    );
    sqlx::query(SQL)
        .bind(ehr_id)
        .bind(kind)
        .fetch_optional(pool)
        .await?
        .as_ref()
        .map(stored_version_by_row)
        .transpose()
}

/// [`read_current_of_kind`]'s time-travel form: the TRUNK version of the
/// EHR's one container of `kind` in force at `at`.
///
/// # Errors
/// Returns [`StorageError`] on a driver/decode failure.
pub async fn version_at_of_kind(
    pool: &PgPool,
    ehr_id: EhrId,
    kind: &str,
    at: jiff::Timestamp,
) -> Result<Option<StoredVersion>, StorageError> {
    const SQL: &str = version_select!(
        "WHERE v.ehr_id = $1 AND v.kind = $2 AND v.branch_number = 0 \
         AND v.committed_at <= $3::timestamptz \
         AND NOT EXISTS (SELECT 1 FROM version s \
                         WHERE s.vo_id = v.vo_id AND s.branch_number = 0 \
                           AND s.committed_at > v.committed_at \
                           AND s.committed_at <= $3::timestamptz)"
    );
    let at = at.to_string();
    sqlx::query(SQL)
        .bind(ehr_id)
        .bind(kind)
        .bind(&at)
        .fetch_optional(pool)
        .await?
        .as_ref()
        .map(stored_version_by_row)
        .transpose()
}

/// Read the current version of the EHR's DIRECTORY folder in ONE statement.
///
/// The `ehr_folder` slot resolution folds into the version read, with the
/// same slot choice `crate::storage::ehr_repo::directory_vo` makes (prefer a
/// live slot over a logically deleted one, then the lowest `rank`, so a read
/// after a logical delete resolves to the deleted version → 204, never 404).
///
/// # Errors
/// Returns [`StorageError`] on a driver/decode failure.
pub async fn read_current_directory(
    pool: &PgPool,
    ehr_id: EhrId,
) -> Result<Option<StoredVersion>, StorageError> {
    const SQL: &str = version_select!(
        "JOIN ehr_folder f ON f.vo_id = v.vo_id \
         WHERE f.ehr_id = $1 AND h.trunk_head_sys_version = v.sys_version \
         ORDER BY (v.lifecycle_state = '523'), f.rank LIMIT 1"
    );
    sqlx::query(SQL)
        .bind(ehr_id)
        .fetch_optional(pool)
        .await?
        .as_ref()
        .map(stored_version_by_row)
        .transpose()
}

/// The stored canonical body BYTES of one version, across both storage tiers
/// (`version`), parsed back to a value with the stored key order kept.
///
/// This is the dump/export source: the archived payload carries the
/// codec's own field order because it IS the committed bytes — a node-row
/// reassembly would surface the `node.data` fragments' jsonb key order
/// instead. `Value::Null` for a deleted version or an absent row.
///
/// # Errors
/// [`StorageError::Database`] on row I/O; [`StorageError::BodyDecode`] when a
/// stored body does not parse (storage corruption).
pub async fn stored_body_all(
    pool: &PgPool,
    vo_id: VoId,
    sys_version: i32,
) -> Result<Value, StorageError> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT body FROM version WHERE vo_id = $1 AND sys_version = $2")
            .bind(vo_id)
            .bind(sys_version)
            .fetch_optional(pool)
            .await?;
    match row.and_then(|(body,)| body) {
        Some(text) => serde_json::from_str(&text).map_err(StorageError::BodyDecode),
        None => Ok(Value::Null),
    }
}

/// The distinct origins of a set of versions, for the access record (#3212).
///
/// The commit-time stamps are unioned in SQL, an unstamped row contributing
/// its creating system; the set is capped at `cap` with the true distinct
/// count beside it.
///
/// # Errors
/// [`StorageError::Database`] when the aggregate fails.
pub async fn read_origins(
    pool: &PgPool,
    versions: &[(VoId, i32)],
    cap: usize,
) -> Result<(Vec<String>, u64), StorageError> {
    if versions.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let vo_ids: Vec<Uuid> = versions.iter().map(|(vo_id, _)| vo_id.0).collect();
    let sys_versions: Vec<i32> = versions.iter().map(|(_, sv)| *sv).collect();
    let row = sqlx::query(
        "WITH o AS ( \
             SELECT DISTINCT e.origin \
             FROM unnest($1::uuid[], $2::int[]) AS a(vo_id, sys_version) \
             JOIN version v ON v.vo_id = a.vo_id AND v.sys_version = a.sys_version \
             CROSS JOIN LATERAL jsonb_array_elements_text( \
                 coalesce(v.origins, jsonb_build_array(v.creating_system_id))) AS e(origin) \
         ) \
         SELECT (SELECT count(*) FROM o) AS total, \
                (SELECT coalesce(jsonb_agg(c.origin ORDER BY c.origin), '[]'::jsonb) \
                 FROM (SELECT origin FROM o ORDER BY origin LIMIT $3) c) AS origins",
    )
    .bind(&vo_ids)
    .bind(&sys_versions)
    .bind(i64::try_from(cap).unwrap_or(i64::MAX))
    .fetch_one(pool)
    .await?;
    let total: i64 = row.try_get("total")?;
    let set: Value = row.try_get("origins")?;
    let set = set
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok((set, u64::try_from(total).unwrap_or(0)))
}
