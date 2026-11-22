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
     CASE WHEN t.time_compact THEN t.data::jsonb ->> 'time' \
     ELSE t.data::jsonb #>> '{time,value}' END) \
     WHEN t.rm_type = 'HISTORY' THEN ext.openehr_timestamp(\
     CASE WHEN t.origin_compact THEN t.data::jsonb ->> 'origin' \
     ELSE t.data::jsonb #>> '{origin,value}' END) END";

/// Build the FIXED-text node insert: one array bind per column over `unnest`,
/// so the statement carries the same parameter count at ANY row count — no
/// per-row placeholders (PostgreSQL caps a statement at 65,535 parameters,
/// which a per-row shape hits at ~4,095 nodes), one prepared statement
/// forever. Built from the shared promoted-leaf registry
/// (`crate::storage::promoted`), each leaf bound through its kind's
/// conversion (non-castable text becomes NULL, the ext baseline). With
/// `per_row_context` the storage context (`vo_id`/`sys_version`/`ehr_id`)
/// unnests per row (the archive-load batch across versions); without, it
/// binds as three constant scalars (the single-version commit write).
fn write_nodes_sql(per_row_context: bool) -> String {
    let (columns, selects, arrays, names) = promoted_insert_parts(29);
    let (context_select, context_arrays, context_names) = if per_row_context {
        (
            "t.vo_id, t.sys_version, t.ehr_id",
            "$1::uuid[], $2::int[], $3::uuid[]",
            "vo_id, sys_version, ehr_id, ",
        )
    } else {
        ("$1, $2, $3", "", "")
    };
    let separator = if per_row_context { ", " } else { "" };
    format!(
        "INSERT INTO node (vo_id, sys_version, ehr_id, num, num_cap, parent_num, \
         rm_type, archetype, arch_entity, arch_concept, arch_major, name, name_code, \
         name_terminology, path, data, value_type, value_num, value_text, value_code, \
         value_terminology, rm_version, language, encoding, origin_compact, time_compact, \
         party_self, uid_derived, event_time{columns}) \
         SELECT {context_select}, t.num, t.num_cap, t.parent_num, t.rm_type, \
         t.archetype, t.arch_entity, t.arch_concept, t.arch_major, t.name, t.name_code, \
         t.name_terminology, t.path, t.data::jsonb, t.value_type, t.value_num::numeric, t.value_text, \
         t.value_code, t.value_terminology, t.rm_version, t.language, t.encoding, \
         t.origin_compact, t.time_compact, t.party_self, t.uid_derived, {EVENT_TIME}{selects} \
         FROM unnest({context_arrays}{separator}$4::int[], $5::int[], $6::int[], $7::text[], $8::text[], \
         $9::text[], $10::text[], $11::int[], $12::text[], $13::text[], $14::text[], \
         $15::text[], $16::text[], $17::text[], $18::text[], $19::text[], $20::text[], \
         $21::text[], $22::text[], $23::text[], $24::text[], $25::boolean[], $26::boolean[], \
         $27::boolean[], $28::boolean[]{arrays}) \
         AS t({context_names}num, num_cap, parent_num, rm_type, archetype, arch_entity, \
         arch_concept, arch_major, name, name_code, name_terminology, path, data, value_type, \
         value_num, value_text, value_code, value_terminology, rm_version, language, \
         encoding, origin_compact, time_compact, party_self, uid_derived{names})"
    )
}

/// The parameters [`node_insert_cte`] reads from its first array on, which
/// [`bind_node_arrays`] binds: the 25 fixed columns, one per promoted leaf and
/// the shared members.
pub(crate) fn node_array_count() -> usize {
    25 + crate::storage::promoted::PROMOTED_LEAVES.len() + 1
}

/// The single-version node insert (constant-scalar storage context).
static WRITE_NODES_SQL: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| write_nodes_sql(false));

/// The node-insert body as a CTE fragment for the FOLDED commit statements:
/// the same fixed-text `unnest` shape as [`write_nodes_sql`], with the
/// storage context spelled as the CALLER's parameter placeholders
/// (`vo`/`sys`/`ehr`, e.g. `"$8"`) and the array binds starting at
/// `first_array_param`. `CROSS JOIN` the caller's version-row CTE `version`
/// orders this CTE after it, which is also what makes the node rows' FK to the
/// version row satisfiable inside the one statement. Empty arrays insert
/// nothing — a logical delete folds through unchanged.
pub(crate) fn node_insert_cte(
    (vo, sys, ehr): (&str, &str, &str),
    version: &str,
    first_array_param: usize,
) -> String {
    let (columns, selects, arrays, names) = promoted_insert_parts(first_array_param + 25);
    let p = |offset: usize| first_array_param + offset;
    format!(
        "INSERT INTO node (vo_id, sys_version, ehr_id, num, num_cap, parent_num, \
         rm_type, archetype, arch_entity, arch_concept, arch_major, name, name_code, \
         name_terminology, path, data, value_type, value_num, value_text, value_code, \
         value_terminology, rm_version, language, encoding, origin_compact, time_compact, \
         party_self, uid_derived, event_time{columns}) \
         SELECT {vo}, {sys}, {ehr}, t.num, t.num_cap, t.parent_num, t.rm_type, \
         t.archetype, t.arch_entity, t.arch_concept, t.arch_major, t.name, t.name_code, \
         t.name_terminology, t.path, t.data::jsonb, t.value_type, t.value_num::numeric, t.value_text, \
         t.value_code, t.value_terminology, t.rm_version, t.language, t.encoding, \
         t.origin_compact, t.time_compact, t.party_self, t.uid_derived, {EVENT_TIME}{selects} \
         FROM unnest(${}::int[], ${}::int[], ${}::int[], ${}::text[], ${}::text[], \
         ${}::text[], ${}::text[], ${}::int[], ${}::text[], ${}::text[], ${}::text[], \
         ${}::text[], ${}::text[], ${}::text[], ${}::text[], ${}::text[], ${}::text[], \
         ${}::text[], ${}::text[], ${}::text[], ${}::text[], ${}::boolean[], ${}::boolean[], \
         ${}::boolean[], ${}::boolean[]{arrays}) \
         AS t(num, num_cap, parent_num, rm_type, archetype, arch_entity, \
         arch_concept, arch_major, name, name_code, name_terminology, path, data, value_type, \
         value_num, value_text, value_code, value_terminology, rm_version, language, \
         encoding, origin_compact, time_compact, party_self, uid_derived{names}) \
         CROSS JOIN {version}",
        p(0),
        p(1),
        p(2),
        p(3),
        p(4),
        p(5),
        p(6),
        p(7),
        p(8),
        p(9),
        p(10),
        p(11),
        p(12),
        p(13),
        p(14),
        p(15),
        p(16),
        p(17),
        p(18),
        p(19),
        p(20),
        p(21),
        p(22),
        p(23),
        p(24),
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

/// Bind the twenty-five per-node column arrays, the promoted-leaf arrays and
/// the shared members onto a node-insert statement whose storage-context parameters (`$1..$3`)
/// are already bound.
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
    let mut name_codes: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut name_terminologies: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut paths: Vec<&str> = Vec::with_capacity(n);
    let mut datas: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut value_types: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut value_nums: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut value_texts: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut value_codes: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut value_terminologies: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut rm_versions: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut languages: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut encodings: Vec<Option<&str>> = Vec::with_capacity(n);
    let mut origin_compacts: Vec<Option<bool>> = Vec::with_capacity(n);
    let mut time_compacts: Vec<Option<bool>> = Vec::with_capacity(n);
    let mut party_selfs: Vec<Option<bool>> = Vec::with_capacity(n);
    let mut uid_deriveds: Vec<Option<bool>> = Vec::with_capacity(n);
    for row in rows {
        nums.push(row.num);
        num_caps.push(row.num_cap);
        parent_nums.push(row.parent_num);
        rm_types.push(row.rm_type.as_str());
        archetypes.push(row.archetype.as_deref());
        arch_entities.push(row.arch_entity.as_deref());
        arch_concepts.push(row.arch_concept.as_deref());
        arch_majors.push(row.arch_major);
        names.push(row.name.as_deref());
        name_codes.push(row.name_code.as_deref());
        name_terminologies.push(row.name_terminology.as_deref());
        paths.push(row.path.as_str());
        datas.push(row.data.as_deref());
        rm_versions.push(row.header.rm_version.as_deref());
        languages.push(row.header.language.as_deref());
        encodings.push(row.header.encoding.as_deref());
        origin_compacts.push(row.compact.origin.then_some(true));
        time_compacts.push(row.compact.time.then_some(true));
        party_selfs.push(row.compact.party_self.then_some(true));
        uid_deriveds.push(row.compact.uid_derived.then_some(true));
        let value = row.value.as_ref();
        value_types.push(value.map(|v| v.value_type.as_str()));
        value_nums.push(value.and_then(|v| v.num.as_deref()));
        value_texts.push(value.and_then(|v| v.text.as_deref()));
        value_codes.push(value.and_then(|v| v.code.as_deref()));
        value_terminologies.push(value.and_then(|v| v.terminology.as_deref()));
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
        .bind(name_codes)
        .bind(name_terminologies)
        .bind(paths)
        .bind(datas)
        .bind(value_types)
        .bind(value_nums)
        .bind(value_texts)
        .bind(value_codes)
        .bind(value_terminologies)
        .bind(rm_versions)
        .bind(languages)
        .bind(encodings)
        .bind(origin_compacts)
        .bind(time_compacts)
        .bind(party_selfs)
        .bind(uid_deriveds);
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
