// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The version-tree placement reads: the next storage commit ordinal, the
//! transaction timestamp, and the next branch number at a fork point.
//!
//! The placement *decision* (classify, tree placement, lifecycle) stays in
//! the versioning layer (`versioning::change`), which takes the per-vo
//! advisory lock first and then calls these reads.
//!
//! No openEHR spec governs the SQL — our own design; the version tree
//! realized is RM common master06 §The 'Virtual Version Tree'.

use sqlx::{PgConnection, Row};

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// The preceding lineage-tip row read for the version-tree placement
/// decision: the addressed version (`expected = Some((t, b, v))`) or the
/// current open TRUNK tip (`expected = None`).
///
/// Plain row values — the versioning layer maps them onto its `PrecedingTip`
/// (tree id + kind + lifecycle).
#[derive(Debug, Clone)]
pub struct TipRow {
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
    /// Whether this version is still the tip of its own lineage: no later
    /// commit ordinal exists under the same branch number (RM common master06
    /// §The 'Virtual Version Tree' — a branch fork does not supersede the
    /// trunk).
    pub open: bool,
}

/// Decode the tip columns of a placement row, or `None` when the object has no
/// version yet.
///
/// One decoder for both placement queries: a `TipRow` field added in only one
/// of two hand-written decodes is a field the other silently drops, and the
/// two queries select the same columns precisely so they can share this.
fn tip_from_row(row: &sqlx::postgres::PgRow) -> Result<Option<TipRow>, StorageError> {
    let Some(kind) = row.try_get::<Option<String>, _>("kind")? else {
        return Ok(None);
    };
    Ok(Some(TipRow {
        ehr_id: row.try_get("ehr_id")?,
        kind,
        sys_version: row.try_get("sys_version")?,
        trunk_version: row.try_get("trunk_version")?,
        branch_number: row.try_get("branch_number")?,
        branch_version: row.try_get("branch_version")?,
        creating_system_id: row.try_get("creating_system_id")?,
        lifecycle_state: row.try_get("lifecycle_state")?,
        open: row.try_get("open")?,
    }))
}

/// The merged placement read ([`next_placement`]).
#[derive(Debug)]
pub struct Placement {
    /// The preceding lineage tip, when the object has one.
    pub tip: Option<TipRow>,
    /// The next storage commit ordinal (`MAX(sys_version) + 1`).
    pub next_ordinal: i32,
    /// The transaction timestamp — the commit instant every row of this
    /// transaction stamps.
    pub now: jiff::Timestamp,
}

/// The leading data-modifying CTEs that bring an archived object back to the
/// hot tier before a write lands on it.
///
/// One `UPDATE` of the partition key per relation set: the node and attestation
/// rows follow their version rows through the foreign keys' `ON UPDATE
/// CASCADE`, so neither is named here. Both statements match nothing in the
/// common unarchived case.
macro_rules! thaw_cte {
    () => {
        concat!(
            "WITH tv AS (UPDATE version SET tier = 'hot' ",
            "            WHERE vo_id = $1 AND tier = 'cold'), ",
            "th AS (UPDATE vo_head SET tier = 'hot', archived_at = NULL, archive_reason = NULL ",
            "       WHERE vo_id = $1 AND tier = 'cold'), "
        )
    };
}

/// Whether the row `t` is still the tip of its own lineage.
///
/// The store is append-only, so there is no open-ended validity interval to
/// test: a version is superseded exactly when a later commit ordinal exists
/// under the same branch number. A fork onto a branch carries a branch number
/// of its own and therefore does not supersede the trunk (RM common master06
/// §The 'Virtual Version Tree').
macro_rules! is_lineage_tip {
    () => {
        "NOT EXISTS (SELECT 1 FROM src s \
         WHERE s.branch_number = t.branch_number AND s.sys_version > t.sys_version)"
    };
}

/// The version-tree placement read, merged into ONE statement — the thaw
/// included.
///
/// Returns the preceding lineage tip (the version `expected` names, or the open
/// TRUNK tip), the next storage commit ordinal, and the transaction timestamp.
/// The timestamp is the commit instant every row of this transaction stamps
/// (`now()` is stable for the whole transaction), so the caller can compute the
/// `VERSION.signature` over `time_committed` BEFORE any insert (RM common
/// master06 §Digital Signature) and commit through the folded CTE
/// unconditionally.
///
/// A new version must never land in the hot tier while its predecessors sit in
/// the cold one, so the statement's leading data-modifying CTEs move any
/// archived rows back first. The reads need no union to see them: `version` is
/// partitioned by tier, so naming the parent relation reads both tiers, and the
/// thaw's own effects being invisible to the sibling scans
/// (<https://www.postgresql.org/docs/18/queries-with.html>) does not matter
/// when the pre-statement snapshot already carries the rows.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn next_placement(
    tx: &mut PgConnection,
    vo_id: VoId,
    expected: Option<(i32, i32, i32)>,
) -> Result<Placement, StorageError> {
    macro_rules! placement_select {
        ($tip_where:literal) => {
            concat!(
                thaw_cte!(),
                "src AS (SELECT vo_id, ehr_id, kind, sys_version, trunk_version, branch_number, ",
                "               branch_version, creating_system_id, lifecycle_state ",
                "        FROM version WHERE vo_id = $1) ",
                "SELECT o.next_ordinal, now() AS ts, tip.ehr_id, tip.kind, tip.sys_version, ",
                "tip.trunk_version, tip.branch_number, tip.branch_version, ",
                "tip.creating_system_id, tip.lifecycle_state, tip.open ",
                "FROM (SELECT (COALESCE(MAX(sys_version), 0) + 1)::int AS next_ordinal ",
                "      FROM src) o ",
                "LEFT JOIN LATERAL ( ",
                "    SELECT t.ehr_id, t.kind, t.sys_version, t.trunk_version, ",
                "           t.branch_number, t.branch_version, t.creating_system_id, ",
                "           t.lifecycle_state, ",
                is_lineage_tip!(),
                " AS open ",
                "    FROM src t WHERE ",
                $tip_where,
                ") tip ON true"
            )
        };
    }
    let row = match expected {
        None => {
            sqlx::query(placement_select!(
                "t.branch_number = 0 \
                 AND t.sys_version = (SELECT MAX(s.sys_version) FROM src s \
                                      WHERE s.branch_number = 0) "
            ))
            .bind(vo_id)
            .fetch_one(&mut *tx)
            .await?
        }
        Some((t, b, v)) => {
            sqlx::query(placement_select!(
                "t.trunk_version = $2 AND t.branch_number = $3 \
                 AND t.branch_version = $4 "
            ))
            .bind(vo_id)
            .bind(t)
            .bind(b)
            .bind(v)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    let tip = tip_from_row(&row)?;
    Ok(Placement {
        tip,
        next_ordinal: row.try_get("next_ordinal")?,
        now: row.try_get::<jiff_sqlx::Timestamp, _>("ts")?.to_jiff(),
    })
}

/// The composition-update mega-read: [`next_placement`]'s current-trunk-tip
/// form plus every column the update pre-checks need
/// ([`UpdatePlacement`]).
#[derive(Debug)]
pub struct UpdatePlacement {
    /// The version-tree placement (tip + next ordinal + the transaction
    /// timestamp). `tip = None` when the object has no current open trunk
    /// version.
    pub placement: Placement,
    /// The tip's commit instant (`version.committed_at`) — the `ETag` /
    /// `If-Match` metadata instant. `None` iff there is no tip.
    pub tip_time_committed: Option<jiff::Timestamp>,
    /// The owning EHR's promoted `is_modifiable` flag. `None` iff there is no
    /// tip or the tip has no owning EHR.
    pub is_modifiable: Option<bool>,
    /// The tip body's `archetype_details.template_id.value`, or `None` for a
    /// deleted tip (NULL body) or an undeclared template.
    pub stored_template: Option<String>,
    /// The FIRST stored content version's root fields —
    /// `(archetype_node_id, category code)` — for the `VERSIONED_COMPOSITION`
    /// cross-version invariants; `None` when no content version exists.
    pub first_root: Option<(Option<String>, Option<String>)>,
}

/// The composition-update placement + pre-check read, merged into ONE
/// in-transaction statement — the thaw included.
///
/// [`next_placement`]'s current-trunk-tip form (the update route's `If-Match`
/// gate has already pinned the addressed version to the current trunk tip, so
/// no expectation-addressed variant exists here) extended with the columns the
/// former pool pre-read (`super::meta::current_composition_meta`) carried: the
/// tip's commit instant, the owning EHR's `is_modifiable`, the stored template
/// id, and the first content version's root fields. The commit instant needs no
/// join now that the version row carries `committed_at` (RM common master06
/// §Committal and Audits: the contribution audit is copied into every version).
/// Run under the per-vo advisory lock inside the write transaction, it replaces
/// that pool round trip entirely. No openEHR spec governs the SQL — our own
/// design.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn update_placement(
    tx: &mut PgConnection,
    vo_id: VoId,
) -> Result<UpdatePlacement, StorageError> {
    // `src` stays NARROW (no body): it is referenced more than once, so the
    // planner materializes it, and a body column would copy every version's
    // document into the tuplestore. The two body-derived facts are read by
    // targeted laterals instead — each touches exactly one row's body (the
    // tip's, and the earliest content version's).
    const SQL: &str = concat!(
        thaw_cte!(),
        "src AS (SELECT vo_id, ehr_id, kind, sys_version, trunk_version, branch_number, ",
        "               branch_version, creating_system_id, lifecycle_state, committed_at ",
        "        FROM version WHERE vo_id = $1) ",
        "SELECT o.next_ordinal, now() AS ts, tip.ehr_id, tip.kind, tip.sys_version, ",
        "tip.trunk_version, tip.branch_number, tip.branch_version, ",
        "tip.creating_system_id, tip.lifecycle_state, tip.open, ",
        "tip.committed_at AS time_committed, e.is_modifiable, tb.stored_template, ",
        "fv.found AS first_found, fv.ani AS first_ani, fv.category AS first_category ",
        "FROM (SELECT (COALESCE(MAX(sys_version), 0) + 1)::int AS next_ordinal ",
        "      FROM src) o ",
        "LEFT JOIN LATERAL ( ",
        "    SELECT t.ehr_id, t.kind, t.sys_version, t.trunk_version, ",
        "           t.branch_number, t.branch_version, t.creating_system_id, ",
        "           t.lifecycle_state, t.committed_at, ",
        is_lineage_tip!(),
        " AS open ",
        "    FROM src t WHERE t.branch_number = 0 ",
        "      AND t.sys_version = (SELECT MAX(s.sys_version) FROM src s ",
        "                           WHERE s.branch_number = 0) ",
        ") tip ON true ",
        "LEFT JOIN ehr e ON e.id = tip.ehr_id ",
        "LEFT JOIN LATERAL ( ",
        "    SELECT (b.body)::jsonb #>> '{archetype_details,template_id,value}' AS stored_template ",
        "    FROM version b ",
        "    WHERE b.vo_id = $1 AND b.sys_version = tip.sys_version ",
        "    LIMIT 1 ",
        ") tb ON true ",
        "LEFT JOIN LATERAL ( ",
        // The ORDER BY + LIMIT sit INSIDE the subquery and the jsonb
        // extractions OUTSIDE it: evaluated inline, the planner computes the
        // extractions for every version row below the sort.
        "    SELECT true AS found, ",
        "           (b.body)::jsonb ->> 'archetype_node_id' AS ani, ",
        "           (b.body)::jsonb #>> '{category,defining_code,code_string}' AS category ",
        "    FROM (SELECT f.body FROM version f ",
        "          WHERE f.vo_id = $1 AND f.body IS NOT NULL ",
        "          ORDER BY f.sys_version LIMIT 1) b ",
        ") fv ON true"
    );
    let row = sqlx::query(SQL).bind(vo_id).fetch_one(&mut *tx).await?;
    let tip = tip_from_row(&row)?;
    let first_root = match row.try_get::<Option<bool>, _>("first_found")? {
        Some(true) => Some((row.try_get("first_ani")?, row.try_get("first_category")?)),
        _ => None,
    };
    Ok(UpdatePlacement {
        placement: Placement {
            tip,
            next_ordinal: row.try_get("next_ordinal")?,
            now: row.try_get::<jiff_sqlx::Timestamp, _>("ts")?.to_jiff(),
        },
        tip_time_committed: row
            .try_get::<Option<jiff_sqlx::Timestamp>, _>("time_committed")?
            .map(jiff_sqlx::Timestamp::to_jiff),
        is_modifiable: row.try_get("is_modifiable")?,
        stored_template: row.try_get("stored_template")?,
        first_root,
    })
}

/// The transaction timestamp (`now()`), stable for the whole transaction —
/// the commit instant for a create (no placement read exists to carry it).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn tx_now(tx: &mut PgConnection) -> Result<jiff::Timestamp, StorageError> {
    Ok(
        sqlx::query_scalar::<_, jiff_sqlx::Timestamp>("SELECT now()")
            .fetch_one(&mut *tx)
            .await?
            .to_jiff(),
    )
}

/// The next branch number at a trunk fork point (`MAX(branch_number) + 1`
/// among the versions at `trunk_version`).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn next_branch_number(
    tx: &mut PgConnection,
    vo_id: VoId,
    trunk_version: i32,
) -> Result<i32, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(MAX(branch_number), 0) + 1 FROM version \
         WHERE vo_id = $1 AND trunk_version = $2",
    )
    .bind(vo_id)
    .bind(trunk_version)
    .fetch_one(&mut *tx)
    .await?)
}
