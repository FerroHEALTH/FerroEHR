// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `node`-table writes: bulk-insert a decomposed version, or many, in one
//! fixed-text statement.
//!
//! No openEHR spec governs the `node` table — it is our own decomposed store.
//! This module is the single home for the `node` write: the folded commit,
//! the EHR Extract import, the integrity rebuild and the archive load all
//! bind their rows here, slimmed by [`crate::storage::slim::slim`] against
//! the identity of their version; the reads are [`crate::storage::node_repo`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::storage::compact::RootIdentity;
use crate::storage::error::StorageError;
use crate::storage::row::NodeRow;

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
    (columns, selects, arrays, names)
}

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
         party_self, uid_derived{columns}) \
         SELECT {context_select}, t.num, t.num_cap, t.parent_num, t.rm_type, \
         t.archetype, t.arch_entity, t.arch_concept, t.arch_major, t.name, t.name_code, \
         t.name_terminology, t.path, t.data, t.value_type, t.value_num::numeric, t.value_text, \
         t.value_code, t.value_terminology, t.rm_version, t.language, t.encoding, \
         t.origin_compact, t.time_compact, t.party_self, t.uid_derived{selects} \
         FROM unnest({context_arrays}{separator}$4::int[], $5::int[], $6::int[], $7::text[], $8::text[], \
         $9::text[], $10::text[], $11::int[], $12::text[], $13::text[], $14::text[], \
         $15::text[], $16::jsonb[], $17::text[], $18::text[], $19::text[], $20::text[], \
         $21::text[], $22::text[], $23::text[], $24::text[], $25::boolean[], $26::boolean[], \
         $27::boolean[], $28::boolean[]{arrays}) \
         AS t({context_names}num, num_cap, parent_num, rm_type, archetype, arch_entity, \
         arch_concept, arch_major, name, name_code, name_terminology, path, data, value_type, \
         value_num, value_text, value_code, value_terminology, rm_version, language, \
         encoding, origin_compact, time_compact, party_self, uid_derived{names})"
    )
}

/// The single-version node insert (constant-scalar storage context).
static WRITE_NODES_SQL: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| write_nodes_sql(false));

/// The node-insert body as a CTE fragment for the FOLDED commit statements:
/// the same fixed-text `unnest` shape as [`write_nodes_sql`], with the
/// storage context spelled as the CALLER's parameter placeholders
/// (`vo`/`sys`/`ehr`, e.g. `"$8"`) and the array binds starting at
/// `first_array_param`. `CROSS JOIN v` orders this CTE after the caller's
/// version-row CTE `v`, which is also what makes the node rows' FK to the
/// version row satisfiable inside the one statement. Empty arrays insert
/// nothing — a logical delete folds through unchanged.
pub(crate) fn node_insert_cte(vo: &str, sys: &str, ehr: &str, first_array_param: usize) -> String {
    let (columns, selects, arrays, names) = promoted_insert_parts(first_array_param + 25);
    let p = |offset: usize| first_array_param + offset;
    format!(
        "INSERT INTO node (vo_id, sys_version, ehr_id, num, num_cap, parent_num, \
         rm_type, archetype, arch_entity, arch_concept, arch_major, name, name_code, \
         name_terminology, path, data, value_type, value_num, value_text, value_code, \
         value_terminology, rm_version, language, encoding, origin_compact, time_compact, \
         party_self, uid_derived{columns}) \
         SELECT {vo}, {sys}, {ehr}, t.num, t.num_cap, t.parent_num, t.rm_type, \
         t.archetype, t.arch_entity, t.arch_concept, t.arch_major, t.name, t.name_code, \
         t.name_terminology, t.path, t.data, t.value_type, t.value_num::numeric, t.value_text, \
         t.value_code, t.value_terminology, t.rm_version, t.language, t.encoding, \
         t.origin_compact, t.time_compact, t.party_self, t.uid_derived{selects} \
         FROM unnest(${}::int[], ${}::int[], ${}::int[], ${}::text[], ${}::text[], \
         ${}::text[], ${}::text[], ${}::int[], ${}::text[], ${}::text[], ${}::text[], \
         ${}::text[], ${}::jsonb[], ${}::text[], ${}::text[], ${}::text[], ${}::text[], \
         ${}::text[], ${}::text[], ${}::text[], ${}::text[], ${}::boolean[], ${}::boolean[], \
         ${}::boolean[], ${}::boolean[]{arrays}) \
         AS t(num, num_cap, parent_num, rm_type, archetype, arch_entity, \
         arch_concept, arch_major, name, name_code, name_terminology, path, data, value_type, \
         value_num, value_text, value_code, value_terminology, rm_version, language, \
         encoding, origin_compact, time_compact, party_self, uid_derived{names}) \
         CROSS JOIN v",
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
/// `rows` is the output of [`crate::storage::codec::decompose`]; the storage
/// context (`vo_id`/`sys_version`/`ehr_id`) is supplied here and written onto
/// every row as constant scalars.
///
/// A logically-deleted version (data Void, RM common master06 §Logical
/// Deletion) has no node rows — the caller simply passes an empty slice and no
/// statement runs.
///
/// The externalized multimedia URIs the rows reference are indexed in the same
/// transaction ([`crate::storage::blob_ref::record`]), so the blob collector
/// never has to scan `node`.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on any driver/insert failure.
// The versioning commit path calls `decompose` + `reassemble` (for the signed
// served form) itself, then hands the rows here — deliberately, so the signed
// bytes and the stored rows come from the one transform. This function stays a
// pure row-writer over pre-decomposed rows.
pub async fn write_nodes(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
    ehr_id: Option<EhrId>,
    root: &RootIdentity<'_>,
    rows: &[NodeRow],
) -> Result<(), StorageError> {
    if rows.is_empty() {
        return Ok(());
    }
    let query = sqlx::query(sqlx::AssertSqlSafe(WRITE_NODES_SQL.as_str()))
        .bind(vo_id)
        .bind(sys_version)
        .bind(ehr_id);
    let refs: Vec<(&NodeRow, &RootIdentity<'_>)> = rows.iter().map(|row| (row, root)).collect();
    bind_node_arrays(query, &refs).execute(&mut *tx).await?;
    crate::storage::blob_ref::record(tx, &[(vo_id, sys_version, rows)]).await?;
    Ok(())
}

/// The node rows of one stored version, with the storage context and the
/// version identity the root row's fragment is slimmed against.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionNodes {
    /// The versioned object.
    pub vo_id: VoId,
    /// The stored version number.
    pub sys_version: i32,
    /// The owning EHR, `None` for a demographic party.
    pub ehr_id: Option<EhrId>,
    /// The version's `OBJECT_VERSION_ID` text
    /// ([`crate::storage::compact::version_uid`]).
    pub version_uid: String,
    /// `version.template_id`.
    pub template_id: Option<String>,
    /// The decomposed rows.
    pub rows: Vec<NodeRow>,
}

impl VersionNodes {
    /// The root identity the rows are slimmed against.
    fn root(&self) -> RootIdentity<'_> {
        RootIdentity {
            version_uid: &self.version_uid,
            template_id: self.template_id.as_deref(),
        }
    }
}

/// Bulk-insert the decomposed node rows of MANY stored versions in ONE
/// statement.
///
/// The same fixed-text `unnest` shape as [`write_nodes`], with the storage
/// context (`vo_id`/`sys_version`/`ehr_id`) unnesting per row instead of
/// binding as constants. The archive load writes a whole record's node rows
/// through this (never one statement per version), and their externalized
/// multimedia URIs are indexed with them
/// ([`crate::storage::blob_ref::record`]), so a loaded record's blobs are as
/// visible to the collector as a committed one's.
///
/// # Errors
///
/// Returns [`StorageError::Database`] on any driver/insert failure.
pub async fn write_nodes_batch(
    tx: &mut PgConnection,
    versions: &[VersionNodes],
) -> Result<(), StorageError> {
    let n: usize = versions.iter().map(|version| version.rows.len()).sum();
    if n == 0 {
        return Ok(());
    }
    let mut vo_ids: Vec<Uuid> = Vec::with_capacity(n);
    let mut sys_versions: Vec<i32> = Vec::with_capacity(n);
    let mut ehr_ids: Vec<Option<Uuid>> = Vec::with_capacity(n);
    let roots: Vec<RootIdentity<'_>> = versions.iter().map(VersionNodes::root).collect();
    let mut refs: Vec<(&NodeRow, &RootIdentity<'_>)> = Vec::with_capacity(n);
    for (version, root) in versions.iter().zip(&roots) {
        for row in &version.rows {
            vo_ids.push(version.vo_id.0);
            sys_versions.push(version.sys_version);
            ehr_ids.push(version.ehr_id.map(|e| e.0));
            refs.push((row, root));
        }
    }
    let query = sqlx::query(sqlx::AssertSqlSafe(WRITE_NODES_BATCH_SQL.as_str()))
        .bind(vo_ids)
        .bind(sys_versions)
        .bind(ehr_ids);
    bind_node_arrays(query, &refs).execute(&mut *tx).await?;
    let blob_refs: Vec<(VoId, i32, &[NodeRow])> = versions
        .iter()
        .map(|version| (version.vo_id, version.sys_version, version.rows.as_slice()))
        .collect();
    crate::storage::blob_ref::record(tx, &blob_refs).await?;
    Ok(())
}

/// Bind the twenty-five per-node column arrays plus the promoted-leaf arrays
/// onto a node-insert statement whose storage-context parameters (`$1..$3`)
/// are already bound.
///
/// The fragment is stored slimmed ([`crate::storage::slim::slim`]): the keys
/// the row's columns hold are left out, a typed `ELEMENT` value fills the
/// five value columns instead, the header columns hold the archetype header,
/// language and encoding, and the four markers say which keys the row left
/// out beyond those. Each row comes with the identity of its version, which
/// the root row (`num = 0`) is slimmed against.
pub(crate) fn bind_node_arrays<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    rows: &[(&'q NodeRow, &'q RootIdentity<'q>)],
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
    let mut datas: Vec<Option<Value>> = Vec::with_capacity(n);
    let mut value_types: Vec<Option<String>> = Vec::with_capacity(n);
    let mut value_nums: Vec<Option<String>> = Vec::with_capacity(n);
    let mut value_texts: Vec<Option<String>> = Vec::with_capacity(n);
    let mut value_codes: Vec<Option<String>> = Vec::with_capacity(n);
    let mut value_terminologies: Vec<Option<String>> = Vec::with_capacity(n);
    let mut rm_versions: Vec<Option<String>> = Vec::with_capacity(n);
    let mut languages: Vec<Option<String>> = Vec::with_capacity(n);
    let mut encodings: Vec<Option<String>> = Vec::with_capacity(n);
    let mut origin_compacts: Vec<Option<bool>> = Vec::with_capacity(n);
    let mut time_compacts: Vec<Option<bool>> = Vec::with_capacity(n);
    let mut party_selfs: Vec<Option<bool>> = Vec::with_capacity(n);
    let mut uid_deriveds: Vec<Option<bool>> = Vec::with_capacity(n);
    for (row, root) in rows {
        nums.push(row.num);
        num_caps.push(row.num_cap);
        parent_nums.push(row.parent_num);
        rm_types.push(&row.rm_type);
        archetypes.push(row.archetype.as_deref());
        arch_entities.push(row.arch_entity.as_deref());
        arch_concepts.push(row.arch_concept.as_deref());
        arch_majors.push(row.arch_major);
        names.push(row.name.as_deref());
        name_codes.push(row.name_code.as_deref());
        name_terminologies.push(row.name_terminology.as_deref());
        paths.push(&row.path);
        let stored = crate::storage::slim::slim(
            &row.rm_type,
            (row.num == 0).then_some(*root),
            row.data.clone(),
        );
        datas.push(stored.data);
        rm_versions.push(stored.header.rm_version);
        languages.push(stored.header.language);
        encodings.push(stored.header.encoding);
        origin_compacts.push(stored.compact.origin.then_some(true));
        time_compacts.push(stored.compact.time.then_some(true));
        party_selfs.push(stored.compact.party_self.then_some(true));
        uid_deriveds.push(stored.compact.uid_derived.then_some(true));
        let value = stored.value;
        value_types.push(value.as_ref().map(|v| v.value_type.clone()));
        value_nums.push(value.as_ref().and_then(|v| v.num.clone()));
        value_texts.push(value.as_ref().and_then(|v| v.text.clone()));
        value_codes.push(value.as_ref().and_then(|v| v.code.clone()));
        value_terminologies.push(value.and_then(|v| v.terminology));
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
            .map(|(row, _)| row.promoted.get(i).and_then(Option::as_deref))
            .collect();
        query = query.bind(leaf_values);
    }
    query
}
