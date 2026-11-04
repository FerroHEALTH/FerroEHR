// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `node`-table reads: one version or many back as canonical JSON, the
//! subtrees an AQL page projects, and the row delete the rebuild starts with.
//!
//! No openEHR spec governs the `node` table — it is our own decomposed store.
//! This module is the single home for the `node`→canonical reload: the
//! version read path, the dump/load export, and the AQL result assembly all
//! funnel here; the write is [`crate::storage::node_write`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::VoId;
use crate::storage::codec::reassemble;
use crate::storage::compact::{Compact, RootIdentity};
use crate::storage::error::StorageError;
use crate::storage::row::ReadRow;

/// The columns [`row_fragment`] reads after `data`, in the order every read
/// statement selects them: the fragment's columns, the header columns, the
/// four markers and, for a root row, the version's uid and template (`NULL`
/// on every other row).
const FRAGMENT_COLUMNS: &str = "n.rm_type, n.archetype, n.name, n.name_code, \
                                n.name_terminology, n.value_type, \
                                n.value_num::text AS value_num, n.value_text, n.value_code, \
                                n.value_terminology, n.rm_version, n.language, n.encoding, \
                                n.origin_compact, n.time_compact, n.party_self, n.uid_derived";

/// The version's uid and template a root row's fragment is rebuilt with,
/// off the joined `version` alias `v`, and `NULL` on every row below the
/// root ([`FRAGMENT_COLUMNS`] continued).
const ROOT_COLUMNS: &str = "CASE WHEN n.num = 0 THEN ext.version_uid(v.vo_id, \
                            v.creating_system_id, v.trunk_version, v.branch_number, \
                            v.branch_version) END AS version_uid, \
                            CASE WHEN n.num = 0 THEN v.template_id END AS template_id";

/// The join from a version row `v` to the node rows that hold its content:
/// its own rows, or the rows of the version `content_sys_version` names when
/// its content equalled its predecessor's at commit (#4415), the stored
/// `content_key`. Every read of a version's rows goes through this key, never
/// through `v.sys_version`.
pub(crate) const CONTENT_ROWS_JOIN: &str =
    "JOIN node n ON n.tier = v.tier AND n.vo_id = v.vo_id AND n.sys_version = v.content_key";

/// The lean read-row statement: **only** the five columns
/// [`crate::storage::codec::reassemble`] and the nested-set contract need, the
/// columns the fragment is rebuilt from, and the version row the root's `uid`
/// and template come back from and whose content key the rows are reached
/// through — the promoted query columns are not read back.
static READ_ROWS_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "SELECT n.num, n.num_cap, n.parent_num, n.path, n.data, {FRAGMENT_COLUMNS}, \
         {ROOT_COLUMNS} \
         FROM version v {CONTENT_ROWS_JOIN} \
         WHERE v.vo_id = $1 AND v.sys_version = $2 ORDER BY n.num"
    )
});

/// The canonical fragment of one fetched node row: its stored `data` with the
/// keys its columns hold put back ([`crate::storage::slim::rebuild`]).
///
/// The row carries, from column `at` on, `data` and then
/// [`FRAGMENT_COLUMNS`] and [`ROOT_COLUMNS`] in that order; they are read by
/// position, because a by-name read searches the column list for every value.
///
/// # Errors
/// [`StorageError::Database`] when a column cannot be read, and
/// [`StorageError::InvalidRows`] when a typed number is no JSON number.
fn row_fragment(row: &sqlx::postgres::PgRow, at: usize) -> Result<Value, StorageError> {
    let rm_type: String = row.try_get(at + 1)?;
    let archetype: Option<String> = row.try_get(at + 2)?;
    let name: Option<String> = row.try_get(at + 3)?;
    let name_code: Option<String> = row.try_get(at + 4)?;
    let name_terminology: Option<String> = row.try_get(at + 5)?;
    let rm_version: Option<String> = row.try_get(at + 11)?;
    let language: Option<String> = row.try_get(at + 12)?;
    let encoding: Option<String> = row.try_get(at + 13)?;
    let marker = |i: usize| -> Result<bool, StorageError> {
        Ok(row.try_get::<Option<bool>, _>(at + i)?.unwrap_or(false))
    };
    let compact = Compact {
        origin: marker(14)?,
        time: marker(15)?,
        party_self: marker(16)?,
        uid_derived: marker(17)?,
    };
    let version_uid: Option<String> = row.try_get(at + 18)?;
    let template_id: Option<String> = row.try_get(at + 19)?;
    let root = version_uid.as_deref().map(|version_uid| RootIdentity {
        version_uid,
        template_id: template_id.as_deref(),
    });
    let value = row
        .try_get::<Option<String>, _>(at + 6)?
        .map(|value_type| -> Result<_, StorageError> {
            Ok(crate::storage::slim::TypedValue {
                value_type,
                num: row.try_get(at + 7)?,
                text: row.try_get(at + 8)?,
                code: row.try_get(at + 9)?,
                terminology: row.try_get(at + 10)?,
            })
        })
        .transpose()?;
    let columns = crate::storage::slim::Columns {
        rm_type: &rm_type,
        archetype: archetype.as_deref(),
        name: name.as_deref(),
        name_code: name_code.as_deref(),
        name_terminology: name_terminology.as_deref(),
        rm_version: rm_version.as_deref(),
        language: language.as_deref(),
        encoding: encoding.as_deref(),
        compact,
        root,
    };
    crate::storage::slim::rebuild(row.try_get(at)?, &columns, value.as_ref()).ok_or_else(|| {
        StorageError::InvalidRows("a node row holds a typed number that is no JSON number".into())
    })
}

/// Fetch the lean read rows of one stored version, ordered by `num`.
///
/// `node` is partitioned by tier, so naming the parent relation reads both
/// tiers in one statement and an archived version stays retrievable. A caller
/// that wants the hot tier alone says `tier = 'hot'`, which the planner prunes
/// at plan time because it is a literal. No openEHR spec governs storage
/// tiering — our own design.
///
/// Generic over the executor so a caller inside a transaction reads its OWN
/// uncommitted rows through the same statement: the node rebuild
/// (`crate::service::admin::integrity::rebuild`) re-verifies what it just
/// wrote before it commits, and a pool-typed read would see the pre-rebuild
/// rows from another connection.
async fn read_rows<'e, E>(
    executor: E,
    vo_id: VoId,
    sys_version: i32,
) -> Result<Vec<ReadRow>, StorageError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(sqlx::AssertSqlSafe(READ_ROWS_SQL.as_str()))
        .bind(vo_id)
        .bind(sys_version)
        .fetch_all(executor)
        .await?;
    let mut read = Vec::with_capacity(rows.len());
    for row in rows {
        read.push(ReadRow {
            num: row.try_get("num")?,
            num_cap: row.try_get("num_cap")?,
            parent_num: row.try_get("parent_num")?,
            path: row.try_get("path")?,
            data: row_fragment(&row, 4)?,
        });
    }
    Ok(read)
}

/// Reassemble one stored version's canonical JSON from its `node` rows on
/// `tx`, over the primary tier: the consolidated node-to-canonical reload.
///
/// A version with no stored nodes (a logical delete: data Void, RM common
/// master06 §Logical Deletion) reassembles to [`Value::Null`], so callers need
/// no separate deleted-version guard before calling.
///
/// The rebuild writes a version's node rows and then re-derives the version
/// from them BEFORE committing, so it must read its own uncommitted rows; a
/// pool-typed read runs on another connection and would see the rows the
/// rebuild is replacing.
///
/// # Errors
///
/// Returns [`StorageError`] on a DB error or if a non-empty row set does not
/// form one tree rooted at `num = 0`.
pub async fn read_version_canonical(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
) -> Result<Value, StorageError> {
    let rows = read_rows(&mut *tx, vo_id, sys_version).await?;
    if rows.is_empty() {
        return Ok(Value::Null);
    }
    reassemble(&rows)
}

/// Makes a version read its own node rows again, after the node rebuild wrote
/// them.
///
/// Clears the `content_sys_version` a shared-content commit set (#4415), and
/// points the head's content key at the version's own rows when the version
/// is the trunk head (#4944).
///
/// # Errors
///
/// Returns [`StorageError::Database`] on any driver/statement failure.
pub async fn unshare_content(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
) -> Result<(), StorageError> {
    sqlx::query(
        "WITH unshared AS ( \
             UPDATE version SET content_sys_version = NULL \
             WHERE vo_id = $1 AND sys_version = $2 AND content_sys_version IS NOT NULL \
             RETURNING vo_id, sys_version \
         ) \
         UPDATE vo_head h SET trunk_head_content_key = u.sys_version FROM unshared u \
         WHERE h.vo_id = u.vo_id AND h.trunk_head_sys_version = u.sys_version",
    )
    .bind(vo_id)
    .bind(sys_version)
    .execute(tx)
    .await?;
    Ok(())
}

/// Deletes every primary-tier `node` row of one stored version, returning how
/// many were removed.
///
/// The rebuild's first write: the row set is replaced wholesale rather than
/// reconciled, because a nested-set numbering is only meaningful as a whole
/// (`crate::storage::codec`).
///
/// # Errors
///
/// Returns [`StorageError::Database`] on any driver/statement failure.
pub async fn delete_version_nodes(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
) -> Result<u64, StorageError> {
    let result = sqlx::query("DELETE FROM node WHERE vo_id = $1 AND sys_version = $2")
        .bind(vo_id)
        .bind(sys_version)
        .execute(&mut *tx)
        .await?;
    Ok(result.rows_affected())
}

/// Fetch the read rows of whole stored versions in **one** statement, keyed by
/// `(vo_id, sys_version)` and ordered by `num` within each version.
///
/// This is the row half of [`read_version_canonical`] for a page of
/// versions: the same tier-inclusive rows, one round trip instead of one per
/// version. A version with no stored nodes (a logical delete, RM common
/// master06 §Logical Deletion) is absent from the map.
///
/// It returns rows rather than reassembled values on purpose. Its caller is the
/// storage-parity sweep, for which a set of rows that does not form a tree is a
/// REPORTED defect rather than a failure: reassembling here would turn one
/// damaged record into an error that aborts the sweep that found it. For the
/// same reason it never shortcuts to the materialized `version.body` the way
/// [`read_subtrees_canonical`] does for a root anchor — the sweep exists to
/// compare the node rows against that body.
///
/// No openEHR spec governs the `node` store — our own decomposed design.
///
/// # Errors
/// Returns [`StorageError`] on a driver failure or an undecodable row.
pub async fn read_version_rows_all(
    pool: &PgPool,
    keys: &[(VoId, i32)],
) -> Result<BTreeMap<(VoId, i32), Vec<ReadRow>>, StorageError> {
    static SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "SELECT v.vo_id, v.sys_version, n.num, n.num_cap, n.parent_num, n.path, n.data, \
             {FRAGMENT_COLUMNS}, {ROOT_COLUMNS} \
             FROM unnest($1::uuid[], $2::int[]) AS k(vo_id, sys_version) \
             JOIN version v ON v.vo_id = k.vo_id AND v.sys_version = k.sys_version \
             {CONTENT_ROWS_JOIN}"
        )
    });
    if keys.is_empty() {
        return Ok(BTreeMap::new());
    }
    let vo_ids: Vec<Uuid> = keys.iter().map(|(vo_id, _)| vo_id.0).collect();
    let sys_versions: Vec<i32> = keys.iter().map(|(_, sys_version)| *sys_version).collect();

    let db_rows = sqlx::query(sqlx::AssertSqlSafe(SQL.as_str()))
        .bind(&vo_ids)
        .bind(&sys_versions)
        .fetch_all(&mut *acquire(pool).await?)
        .await?;

    let mut grouped: BTreeMap<(VoId, i32), Vec<ReadRow>> = BTreeMap::new();
    for row in db_rows {
        let key = (VoId(row.try_get("vo_id")?), row.try_get("sys_version")?);
        grouped.entry(key).or_default().push(ReadRow {
            num: row.try_get("num")?,
            num_cap: row.try_get("num_cap")?,
            parent_num: row.try_get("parent_num")?,
            path: row.try_get("path")?,
            data: row_fragment(&row, 6)?,
        });
    }

    for rows in grouped.values_mut() {
        // `read_rows` orders by `num` in SQL; the join above cannot, because one
        // statement carries every version's rows interleaved.
        rows.sort_by_key(|row| row.num);
    }
    Ok(grouped)
}

/// One whole-object cell's subtree locator: the `[num, num_cap]` interval of
/// a stored version.
///
/// The AQL executor collects one per whole-object cell across a whole
/// `RESULT_SET` page so [`read_subtrees_canonical`] can load them all in a
/// single round trip. No openEHR spec governs the `node` store — our own
/// decomposed design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubtreeAnchor {
    /// The versioned object id.
    pub vo_id: VoId,
    /// The stored version number.
    pub sys_version: i32,
    /// The anchor node's pre-order number (subtree lower bound).
    pub num: i32,
    /// The anchor node's `num_cap` (subtree upper bound).
    pub num_cap: i32,
}

/// Reassemble every distinct subtree in `anchors` in **one** statement,
/// returning a map from anchor to its canonical JSON.
///
/// Each anchor's subtree is re-based so the anchor node becomes the fragment
/// root (the codec requires `num == 0` and an empty path at the root).
///
/// This closes the AQL result-assembly N+1: a P-row whole-object projection page
/// would otherwise issue P separate subtree SELECTs. The rows of every anchor's
/// subtree are fetched by a single `unnest`-array join over the anchors, tagged
/// by anchor index, then reassembled per anchor in memory.
///
/// Anchors are de-duplicated, so a page projecting the same version more than
/// once reassembles each distinct subtree exactly once. An anchor with no stored
/// nodes (a logical delete, RM common master06 §Logical Deletion) is absent from
/// the map and the caller treats the miss as [`Value::Null`].
///
/// A root anchor (`num == 0`, the dominant projection shape) is served straight
/// from the materialized `version.body` with no node rows and no reassembly;
/// only genuine sub-tree anchors take the interval join. The two forms are
/// byte-identical by the commit-time parity invariant.
///
/// # Errors
/// Returns [`StorageError`] on a driver failure, or if any anchor's fetched rows
/// do not reassemble into one tree.
pub async fn read_subtrees_canonical(
    pool: &PgPool,
    anchors: &[SubtreeAnchor],
) -> Result<HashMap<SubtreeAnchor, Value>, StorageError> {
    // No anchor here is a root (`num = 0`), so no row needs its version's
    // uid or template: the two root columns are `NULL`. The anchor names the
    // version, whose row resolves the content key the rows are read under.
    static SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "SELECT a.idx, n.num, n.path, n.data, {FRAGMENT_COLUMNS}, \
                    NULL::text AS version_uid, NULL::text AS template_id \
             FROM unnest($1::int[], $2::uuid[], $3::int[], $4::int[], $5::int[]) \
                  AS a(idx, vo_id, sys_version, anum, acap) \
             JOIN version v ON v.vo_id = a.vo_id AND v.sys_version = a.sys_version \
             {CONTENT_ROWS_JOIN} AND n.num BETWEEN a.anum AND a.acap"
        )
    });
    if anchors.is_empty() {
        return Ok(HashMap::new());
    }

    // De-duplicate, preserving one entry per distinct anchor; the position in
    // `distinct` is the `idx` tag bound into the query and joined back below.
    let mut distinct: Vec<SubtreeAnchor> = Vec::new();
    let mut seen: HashMap<SubtreeAnchor, usize> = HashMap::new();
    for anchor in anchors {
        seen.entry(*anchor).or_insert_with(|| {
            distinct.push(*anchor);
            distinct.len() - 1
        });
    }

    // Root anchors (`num == 0`) are whole versions — served from the
    // materialized body with no node rows; only genuine sub-tree anchors
    // proceed to the interval join.
    let roots: Vec<SubtreeAnchor> = distinct.iter().filter(|a| a.num == 0).copied().collect();
    let mut out = read_root_bodies(pool, &roots).await?;
    let distinct: Vec<SubtreeAnchor> = distinct.into_iter().filter(|a| a.num != 0).collect();
    if distinct.is_empty() {
        return Ok(out);
    }

    let idx: Vec<i32> = (0..distinct.len())
        .map(|i| i32::try_from(i).unwrap_or(i32::MAX))
        .collect();
    let vo_ids: Vec<Uuid> = distinct.iter().map(|a| a.vo_id.0).collect();
    let sys_versions: Vec<i32> = distinct.iter().map(|a| a.sys_version).collect();
    let anums: Vec<i32> = distinct.iter().map(|a| a.num).collect();
    let acaps: Vec<i32> = distinct.iter().map(|a| a.num_cap).collect();

    // One interval join: each anchor row contributes its subtree's node rows,
    // tagged by `idx`. `num BETWEEN anum AND acap` is the nested-set containment
    // predicate the single-row read also uses, now driven by the anchor set.
    let db_rows = sqlx::query(sqlx::AssertSqlSafe(SQL.as_str()))
        .bind(&idx)
        .bind(&vo_ids)
        .bind(&sys_versions)
        .bind(&anums)
        .bind(&acaps)
        .fetch_all(&mut *acquire(pool).await?)
        .await?;

    // Bucket the returned node rows by anchor index.
    let mut grouped: BTreeMap<i32, Vec<(i32, String, Value)>> = BTreeMap::new();
    for row in db_rows {
        let group_idx: i32 = row.try_get("idx")?;
        grouped.entry(group_idx).or_default().push((
            row.try_get("num")?,
            row.try_get("path")?,
            row_fragment(&row, 3)?,
        ));
    }

    for (group_idx, mut nodes) in grouped {
        // The anchor is the lowest-`num` row (the queried `num`); its path is the
        // prefix to strip so descendants re-root at it (the anchor-relative
        // `num`/`path` rebasing the reassembly codec requires).
        nodes.sort_by_key(|(num, _, _)| *num);
        // `idx` was generated by the query from the same `distinct` list, so it
        // addresses one of its entries; fetched rather than indexed so a
        // mismatch skips the group instead of panicking on a read path.
        let Some(anchor) = usize::try_from(group_idx)
            .ok()
            .and_then(|i| distinct.get(i))
            .copied()
        else {
            continue;
        };
        let base_path = nodes
            .first()
            .map(|(_, path, _)| path.clone())
            .unwrap_or_default();
        let read_rows: Vec<ReadRow> = nodes
            .into_iter()
            .map(|(num, path, data)| {
                let rebased = path.strip_prefix(&base_path).unwrap_or(&path).to_owned();
                ReadRow {
                    num: num - anchor.num,
                    // num_cap/parent_num are unused by `reassemble`; re-root them.
                    num_cap: 0,
                    parent_num: 0,
                    path: rebased,
                    data,
                }
            })
            .collect();
        out.insert(anchor, reassemble(&read_rows)?);
    }
    Ok(out)
}

/// The materialized bodies of whole-version (root) anchors, keyed by anchor —
/// one `unnest` join against `version.body`, no node rows, no reassembly.
///
/// An anchor whose version has a `NULL` body (a logical delete) is absent
/// from the map — the same miss contract the interval join produces.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
async fn read_root_bodies(
    pool: &PgPool,
    roots: &[SubtreeAnchor],
) -> Result<HashMap<SubtreeAnchor, Value>, StorageError> {
    let mut out: HashMap<SubtreeAnchor, Value> = HashMap::with_capacity(roots.len());
    if roots.is_empty() {
        return Ok(out);
    }
    let vo_ids: Vec<Uuid> = roots.iter().map(|a| a.vo_id.0).collect();
    let sys_versions: Vec<i32> = roots.iter().map(|a| a.sys_version).collect();
    let rows = sqlx::query(
        "SELECT v.vo_id, v.sys_version, v.body \
         FROM unnest($1::uuid[], $2::int[]) AS a(vo_id, sys_version) \
         JOIN version v \
           ON v.vo_id = a.vo_id AND v.sys_version = a.sys_version \
         WHERE v.body IS NOT NULL",
    )
    .bind(&vo_ids)
    .bind(&sys_versions)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    let mut by_key: HashMap<(Uuid, i32), Value> = rows
        .into_iter()
        .map(|row| {
            let text: String = row.try_get("body")?;
            let value: Value = serde_json::from_str(&text).map_err(StorageError::BodyDecode)?;
            Ok(((row.try_get("vo_id")?, row.try_get("sys_version")?), value))
        })
        .collect::<Result<_, StorageError>>()?;
    for anchor in roots {
        if let Some(body) = by_key.remove(&(anchor.vo_id.0, anchor.sys_version)) {
            out.insert(*anchor, body);
        }
    }
    Ok(out)
}

/// Reads the first stored content version's `(archetype_node_id, category
/// code)` pair — the two scalars the cross-version invariants compare.
///
/// Read as text off the materialized body over BOTH storage tiers
/// (`version`: the read runs before the commit path's thaw, and an
/// archived first version still fixes the invariants).
///
/// `None` when no content version exists (e.g. every prior version deleted);
/// either scalar is `None` when the stored body lacks that field.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn first_version_root(
    tx: &mut PgConnection,
    vo_id: VoId,
) -> Result<Option<(Option<String>, Option<String>)>, StorageError> {
    // Two text scalars off the materialized body — never a fragment fetch and
    // a Value parse for a two-field comparison.
    Ok(sqlx::query_as(
        "SELECT (body)::jsonb ->> 'archetype_node_id', \
         (body)::jsonb #>> '{category,defining_code,code_string}' \
         FROM version WHERE vo_id = $1 AND body IS NOT NULL \
         ORDER BY sys_version LIMIT 1",
    )
    .bind(vo_id)
    .fetch_optional(&mut *tx)
    .await?)
}
