// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `node`-table writes: bulk-insert a decomposed version, or many, in one
//! fixed-text statement.
//!
//! No openEHR spec governs the `node` table — it is our own decomposed store.
//! This module is the single home for the `node` write: the folded commit,
//! the EHR Extract import, the integrity rebuild and the archive load all
//! bind their rows here, read off the one rendering of their body
//! ([`crate::storage::render`]); the reads are [`crate::storage::node_repo`].

use sqlx::PgConnection;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;
use crate::storage::render::Decomposed;
use crate::storage::render::nodes::StoredNode;

/// The promoted-leaf parts of a node insert: the column list, the converting
/// selects, the array parameters from `first_param` on, and the `unnest`
/// column names.
fn promoted_insert_parts(first_param: usize) -> (String, String, String, String) {
    use std::fmt::Write;
    let mut columns = String::new();
    let mut selects = String::new();
    let mut arrays = String::new();
    let mut names = String::new();
    for (i, leaf) in crate::storage::promoted::PROMOTED_LEAVES.iter().enumerate() {
        columns.push_str(", ");
        columns.push_str(sea_query::Iden::unquoted(&leaf.column));
        match leaf.kind {
            crate::storage::promoted::PromotedKind::Timestamp => {
                let _ = write!(selects, ", ext.openehr_timestamp(t.p{i})");
            }
        }
        let _ = write!(arrays, ", ${}::text[]", first_param + i);
        let _ = write!(names, ", p{i}");
    }
    // The shared members (crate::storage::shared) bind after the promoted leaves.
    columns.push_str(", shared");
    selects.push_str(", t.shared::jsonb");
    let _ = write!(
        arrays,
        ", ${}::text[]",
        first_param + crate::storage::promoted::PROMOTED_LEAVES.len()
    );
    names.push_str(", shared");
    (columns, selects, arrays, names)
}

/// The `event_time` of an inserted row `t`: the time of an event row or the
/// origin of a history row, read by the function an AQL comparison reads it
/// with, so the column and the comparison cannot differ; `NULL` on every
/// other row. The node migration `node_event_time` fills stored rows from
/// the same expression. No openEHR spec governs storage: our own design
/// (#6022).
const EVENT_TIME: &str = "CASE \
     WHEN t.rm_type IN ('POINT_EVENT', 'INTERVAL_EVENT') THEN ext.openehr_timestamp(\
     CASE WHEN node_marks.time_compact THEN t.data::jsonb ->> 'time' \
     ELSE t.data::jsonb #>> '{time,value}' END) \
     WHEN t.rm_type = 'HISTORY' THEN ext.openehr_timestamp(\
     CASE WHEN node_marks.origin_compact THEN t.data::jsonb ->> 'origin' \
     ELSE t.data::jsonb #>> '{origin,value}' END) END";

/// The node columns every row carries, bound dense, one element per row,
/// with the SQL type of each array.
const DENSE: [(&str, &str); 10] = [
    ("num", "int"),
    ("num_cap", "int"),
    ("parent_num", "int"),
    ("rm_type", "text"),
    ("archetype", "text"),
    ("arch_entity", "text"),
    ("arch_concept", "text"),
    ("arch_major", "int"),
    ("name", "text"),
    ("path", "text"),
];

/// The node columns most rows leave `NULL`, bound in groups: each group is
/// the 1-based positions of the rows that set any of its columns, then one
/// array per column over those rows only, so a row that sets none sends no
/// element of the group (#6244). A group that more rows set than leave it
/// empty goes dense, `NULL` positions and one element per row, whichever
/// costs the fewer bytes ([`Group::sparse`]). Each group is its alias, then
/// its columns with their SQL types.
const SPARSE: [(&str, &[(&str, &str)]); 4] = [
    (
        "node_names",
        &[("name_code", "text"), ("name_terminology", "text")],
    ),
    (
        "node_values",
        &[
            ("value_type", "text"),
            ("value_num", "text"),
            ("value_text", "text"),
            ("value_code", "text"),
            ("value_terminology", "text"),
        ],
    ),
    (
        "node_headers",
        &[
            ("rm_version", "text"),
            ("language", "text"),
            ("encoding", "text"),
        ],
    ),
    (
        "node_marks",
        &[
            ("origin_compact", "boolean"),
            ("time_compact", "boolean"),
            ("party_self", "boolean"),
            ("uid_derived", "boolean"),
        ],
    ),
];

/// The arrays [`bind_node_arrays`] binds before the promoted leaves: the
/// dense columns, `data`, and each sparse group's positions and columns.
const FIXED_ARRAYS: usize = 25 + SPARSE.len();

/// The FIXED-text node insert: the dense columns and `data` unnested with
/// their ordinality, each sparse group unnested and joined to its rows by
/// position, so the statement carries the same parameter count at ANY row
/// count, with no per-row placeholders (PostgreSQL caps a statement at 65,535
/// parameters), one prepared statement forever. The arrays start at
/// `first_param`. `context` is the storage context's select and, when it
/// unnests per row, its arrays and names; `tail` follows the joins.
fn node_insert(context: (&str, Option<(&str, &str)>), first_param: usize, tail: &str) -> String {
    use std::fmt::Write as _;
    let (context_select, per_row) = context;
    let mut param = first_param;
    let mut dense_arrays = String::new();
    let mut dense_names = String::new();
    if let Some((arrays, names)) = per_row {
        let _ = write!(dense_arrays, "{arrays}, ");
        let _ = write!(dense_names, "{names}, ");
    }
    for (column, ty) in DENSE.iter().chain(&[("data", "text")]) {
        let _ = write!(dense_arrays, "${param}::{ty}[], ");
        let _ = write!(dense_names, "{column}, ");
        param += 1;
    }
    dense_arrays.truncate(dense_arrays.len().saturating_sub(2));
    dense_names.truncate(dense_names.len().saturating_sub(2));
    let mut joins = String::new();
    for (alias, columns) in SPARSE {
        // NOTE: our own design (#6244), `NULL` positions say the group is dense: every row in
        // order, as many as its first column holds.
        let first_ty = columns.first().map_or("text", |(_, ty)| *ty);
        let _ = write!(
            joins,
            " LEFT JOIN unnest(coalesce(${param}::int[], \
             array(SELECT generate_series(1, cardinality(${}::{first_ty}[]))))",
            param + 1
        );
        param += 1;
        for (_, ty) in columns {
            let _ = write!(joins, ", ${param}::{ty}[]");
            param += 1;
        }
        let _ = write!(joins, ") AS {alias}(pos");
        for (column, _) in columns {
            let _ = write!(joins, ", {column}");
        }
        let _ = write!(joins, ") ON {alias}.pos = t.ord");
    }
    let (columns, selects, arrays, names) = promoted_insert_parts(param);
    format!(
        "INSERT INTO node (vo_id, sys_version, ehr_id, num, num_cap, parent_num, \
         rm_type, archetype, arch_entity, arch_concept, arch_major, name, name_code, \
         name_terminology, path, data, value_type, value_num, value_text, value_code, \
         value_terminology, rm_version, language, encoding, origin_compact, time_compact, \
         party_self, uid_derived, event_time{columns}) \
         SELECT {context_select}, t.num, t.num_cap, t.parent_num, t.rm_type, \
         t.archetype, t.arch_entity, t.arch_concept, t.arch_major, t.name, \
         node_names.name_code, node_names.name_terminology, t.path, t.data::jsonb, \
         node_values.value_type, node_values.value_num::numeric, node_values.value_text, \
         node_values.value_code, node_values.value_terminology, node_headers.rm_version, \
         node_headers.language, node_headers.encoding, node_marks.origin_compact, \
         node_marks.time_compact, node_marks.party_self, node_marks.uid_derived, \
         {EVENT_TIME}{selects} \
         FROM unnest({dense_arrays}{arrays}) WITH ORDINALITY \
         AS t({dense_names}{names}, ord){joins}{tail}"
    )
}

/// Build the single-version node insert, or with `per_row_context` the
/// cross-version batch, whose storage context (`vo_id`/`sys_version`/`ehr_id`)
/// unnests per row (the archive load) where the single version binds it as
/// three constant scalars (the commit write).
fn write_nodes_sql(per_row_context: bool) -> String {
    if per_row_context {
        node_insert(
            (
                "t.vo_id, t.sys_version, t.ehr_id",
                Some((
                    "$1::uuid[], $2::int[], $3::uuid[]",
                    "vo_id, sys_version, ehr_id",
                )),
            ),
            4,
            "",
        )
    } else {
        node_insert(("$1, $2, $3", None), 4, "")
    }
}

/// The parameters [`node_insert_cte`] reads from its first array on, which
/// [`bind_node_arrays`] binds: the dense columns and `data`, each sparse
/// group's positions and columns, one per promoted leaf and the shared
/// members.
pub(crate) fn node_array_count() -> usize {
    FIXED_ARRAYS + crate::storage::promoted::PROMOTED_LEAVES.len() + 1
}

/// The single-version node insert (constant-scalar storage context).
static WRITE_NODES_SQL: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| write_nodes_sql(false));

/// The node-insert body as a CTE fragment for the FOLDED commit statements:
/// the shape of [`node_insert`], with the storage context spelled as the
/// CALLER's parameter placeholders (`vo`/`sys`/`ehr`, e.g. `"$8"`) and the
/// array binds starting at `first_array_param`. `CROSS JOIN` the caller's
/// version-row CTE `version` orders this CTE after it, which is also what
/// makes the node rows' FK to the version row satisfiable inside the one
/// statement. Empty arrays insert nothing: a logical delete folds through
/// unchanged.
pub(crate) fn node_insert_cte(
    (vo, sys, ehr): (&str, &str, &str),
    version: &str,
    first_array_param: usize,
) -> String {
    node_insert(
        (&format!("{vo}, {sys}, {ehr}"), None),
        first_array_param,
        &format!(" CROSS JOIN {version}"),
    )
}

/// The cross-version batch node insert (per-row storage context).
static WRITE_NODES_BATCH_SQL: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| write_nodes_sql(true));

/// Bulk-insert the decomposed node rows of one stored version — ONE
/// fixed-text `unnest` statement whatever the row count.
///
/// `version` is the output of [`crate::storage::render::decompose`]; the
/// storage context (`vo_id`/`sys_version`/`ehr_id`) is supplied here and
/// written onto every row as constant scalars.
///
/// A logically-deleted version (data Void, RM common master06 §Logical
/// Deletion) has no node rows — the caller simply passes an empty set and no
/// statement runs.
///
/// The externalized multimedia URIs the body references are indexed in the
/// same transaction ([`crate::storage::blob_ref::record_uris`]), so the blob
/// collector never has to scan `node`.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on any driver/insert failure.
pub async fn write_nodes(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
    ehr_id: Option<EhrId>,
    version: &Decomposed,
) -> Result<(), StorageError> {
    if version.rows.is_empty() {
        return Ok(());
    }
    let query = sqlx::query(sqlx::AssertSqlSafe(WRITE_NODES_SQL.as_str()))
        .bind(vo_id)
        .bind(sys_version)
        .bind(ehr_id);
    bind_node_arrays(query, &version.rows.iter().collect::<Vec<_>>())
        .execute(&mut *tx)
        .await?;
    crate::storage::blob_ref::record_uris(tx, vo_id, sys_version, &version.blob_uris).await?;
    Ok(())
}

/// The node rows of one stored version, with its storage context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionNodes {
    /// The versioned object.
    pub vo_id: VoId,
    /// The stored version number.
    pub sys_version: i32,
    /// The owning EHR, `None` for a demographic party.
    pub ehr_id: Option<EhrId>,
    /// The decomposed rows and the blob URIs of the body.
    pub version: Decomposed,
}

/// Bulk-insert the decomposed node rows of MANY stored versions in ONE
/// statement.
///
/// The same fixed-text `unnest` shape as [`write_nodes`], with the storage
/// context (`vo_id`/`sys_version`/`ehr_id`) unnesting per row instead of
/// binding as constants. The archive load writes a whole record's node rows
/// through this (never one statement per version), and their externalized
/// multimedia URIs are indexed with them
/// ([`crate::storage::blob_ref::record_uris`]), so a loaded record's blobs are
/// as visible to the collector as a committed one's.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on any driver/insert failure.
pub async fn write_nodes_batch(
    tx: &mut PgConnection,
    versions: &[VersionNodes],
) -> Result<(), StorageError> {
    let n: usize = versions.iter().map(|v| v.version.rows.len()).sum();
    if n == 0 {
        return Ok(());
    }
    let mut vo_ids: Vec<Uuid> = Vec::with_capacity(n);
    let mut sys_versions: Vec<i32> = Vec::with_capacity(n);
    let mut ehr_ids: Vec<Option<Uuid>> = Vec::with_capacity(n);
    let mut rows: Vec<&StoredNode> = Vec::with_capacity(n);
    for version in versions {
        for row in &version.version.rows {
            vo_ids.push(version.vo_id.0);
            sys_versions.push(version.sys_version);
            ehr_ids.push(version.ehr_id.map(|e| e.0));
            rows.push(row);
        }
    }
    let query = sqlx::query(sqlx::AssertSqlSafe(WRITE_NODES_BATCH_SQL.as_str()))
        .bind(vo_ids)
        .bind(sys_versions)
        .bind(ehr_ids);
    bind_node_arrays(query, &rows).execute(&mut *tx).await?;
    for version in versions {
        crate::storage::blob_ref::record_uris(
            tx,
            version.vo_id,
            version.sys_version,
            &version.version.blob_uris,
        )
        .await?;
    }
    Ok(())
}

/// One group of [`SPARSE`]: the 1-based positions of the rows that set any
/// of its columns, each column over those rows, and the count of all rows.
struct Group<T> {
    positions: Vec<i32>,
    columns: Vec<Vec<Option<T>>>,
    rows: usize,
}

impl<T: Copy> Group<T> {
    /// A group of `width` columns over `rows` rows.
    fn new(width: usize, rows: usize) -> Self {
        Self {
            positions: Vec::with_capacity(rows),
            columns: (0..width).map(|_| Vec::with_capacity(rows)).collect(),
            rows,
        }
    }

    /// Adds the row at `position` when it sets any of `values`.
    fn push(&mut self, position: i32, values: &[Option<T>]) {
        if values.iter().all(Option::is_none) {
            return;
        }
        self.positions.push(position);
        for (column, value) in self.columns.iter_mut().zip(values) {
            column.push(*value);
        }
    }

    /// Whether the group costs fewer bytes on the wire sparse than dense:
    /// sparse, each of its rows costs a position (eight bytes) and a length
    /// word per column, and the positions array a 20-byte header; dense,
    /// every row costs a length word per column and the positions a `NULL`
    /// (four bytes).
    fn sparse(&self) -> bool {
        let width = self.columns.len();
        (8 + 4 * width) * self.positions.len() + 16 < 4 * width * self.rows
    }

    /// `query` with the positions and then each column bound, sparse or
    /// dense, whichever is smaller.
    fn bound<'q>(
        self,
        query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>
    where
        T: 'q + sqlx::Encode<'q, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
        Vec<Option<T>>: sqlx::Encode<'q, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
    {
        if self.sparse() {
            let mut query = query.bind(self.positions);
            for column in self.columns {
                query = query.bind(column);
            }
            return query;
        }
        let mut query = query.bind(None::<Vec<i32>>);
        for column in self.columns {
            let mut dense = vec![None; self.rows];
            for (position, value) in self.positions.iter().zip(column) {
                if let Some(slot) = usize::try_from(position - 1)
                    .ok()
                    .and_then(|at| dense.get_mut(at))
                {
                    *slot = value;
                }
            }
            query = query.bind(dense);
        }
        query
    }
}

/// Bind the dense node columns and `data`, each sparse group's positions and
/// columns ([`SPARSE`]), the promoted-leaf arrays and the shared members onto
/// a node-insert statement whose storage-context parameters (`$1..$3`) are
/// already bound.
///
/// Each row arrives slimmed ([`crate::storage::render::nodes`]): its fragment
/// text leaves out the keys its columns hold, a typed `ELEMENT` value fills
/// the five value columns instead, the header columns hold the archetype
/// header, language and encoding, and the four markers say which keys the row
/// left out beyond those.
pub(crate) fn bind_node_arrays<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    rows: &[&'q StoredNode],
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    let n = rows.len();
    let mut nums = Vec::with_capacity(n);
    let mut num_caps = Vec::with_capacity(n);
    let mut parent_nums = Vec::with_capacity(n);
    let mut rm_types: Vec<&str> = Vec::with_capacity(n);
    let mut archetypes: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut arch_entities: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut arch_concepts: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut arch_majors: Vec<Option<i32>> = Vec::with_capacity(n);
    let mut names: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut paths: Vec<&str> = Vec::with_capacity(n);
    let mut datas: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut name_codes = Group::new(2, n);
    let mut values = Group::new(5, n);
    let mut headers = Group::new(3, n);
    let mut marks = Group::new(4, n);
    for (row, position) in rows.iter().zip(1_i32..) {
        nums.push(row.num);
        num_caps.push(row.num_cap);
        parent_nums.push(row.parent_num);
        rm_types.push(row.rm_type.as_str());
        archetypes.push(row.archetype.as_deref());
        arch_entities.push(row.arch_entity.as_deref());
        arch_concepts.push(row.arch_concept.as_deref());
        arch_majors.push(row.arch_major);
        names.push(row.name.as_deref());
        paths.push(row.path.as_str());
        datas.push(row.data.as_deref());
        name_codes.push(
            position,
            &[row.name_code.as_deref(), row.name_terminology.as_deref()],
        );
        let value = row.value.as_ref();
        values.push(
            position,
            &[
                value.map(|v| v.value_type.as_str()),
                value.and_then(|v| v.num.as_deref()),
                value.and_then(|v| v.text.as_deref()),
                value.and_then(|v| v.code.as_deref()),
                value.and_then(|v| v.terminology.as_deref()),
            ],
        );
        headers.push(
            position,
            &[
                row.header.rm_version.as_deref(),
                row.header.language.as_deref(),
                row.header.encoding.as_deref(),
            ],
        );
        marks.push(
            position,
            &[
                row.compact.origin.then_some(true),
                row.compact.time.then_some(true),
                row.compact.party_self.then_some(true),
                row.compact.uid_derived.then_some(true),
            ],
        );
    }
    query = query
        .bind(nums)
        .bind(num_caps)
        .bind(parent_nums)
        .bind(rm_types)
        .bind(archetypes)
        .bind(arch_entities)
        .bind(arch_concepts)
        .bind(arch_majors)
        .bind(names)
        .bind(paths)
        .bind(datas);
    query = name_codes.bound(query);
    query = values.bound(query);
    query = headers.bound(query);
    query = marks.bound(query);
    for (i, _leaf) in crate::storage::promoted::PROMOTED_LEAVES.iter().enumerate() {
        let leaf_values: Vec<Option<&str>> = rows
            .iter()
            .map(|row| row.promoted.get(i).and_then(Option::as_deref))
            .collect();
        query = query.bind(leaf_values);
    }
    let shared: Vec<Option<&str>> = rows.iter().map(|row| row.shared.as_deref()).collect();
    query.bind(shared)
}

#[cfg(test)]
mod tests {
    use super::Group;

    /// A group with `set` of `rows` rows setting its one of two columns.
    fn group(rows: i32, set: i32) -> Group<bool> {
        let mut group = Group::new(2, usize::try_from(rows).expect("a count"));
        for position in 1..=rows {
            let value = (position <= set).then_some(true);
            group.push(position, &[value, None]);
        }
        group
    }

    /// A group most rows leave empty goes sparse, one most rows set or a
    /// group of a few rows goes dense (#6244).
    #[test]
    fn a_group_takes_the_smaller_form() {
        assert!(group(402, 20).sparse(), "20 of 402 rows: sparse");
        assert!(!group(402, 300).sparse(), "300 of 402 rows: dense");
        assert!(!group(3, 1).sparse(), "1 of 3 rows: dense");
        assert!(!group(0, 0).sparse(), "no rows: dense");
    }
}
