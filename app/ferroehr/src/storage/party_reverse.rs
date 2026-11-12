// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for `party_relationship_target`, the target-side index of the
//! `PARTY_RELATIONSHIP`s a party carries inline.
//!
//! RM demographic `master02-demographic_package.adoc` §Party Relationships
//! stores a relationship in the data of its source party and lists it, at the
//! target end, in `reverse_relationships`. The source body holds the forward
//! direction; this index holds the reverse one, brought to the source's
//! current trunk version on every write that changes it. No openEHR spec
//! governs the index relation or its SQL: our own design.

use sqlx::{PgConnection, Row};

use crate::ids::VoId;
use crate::storage::error::StorageError;

/// The relationships of the current trunk version of each party `h` names,
/// one row each with its position, its target root and its identity within
/// the body: the `FROM` clause of [`reverse_of`], completed by its own
/// `WHERE`.
///
/// The `relationships` attribute is read only when it is an array, so a body
/// without one contributes no row. A target id is a `HIER_OBJECT_ID` value,
/// `root` or `root::extension`, and the root is what names a party container.
macro_rules! current_relationships {
    () => {
        "JOIN version v ON v.tier = h.tier AND v.vo_id = h.vo_id \
                       AND v.sys_version = h.trunk_head_sys_version \
         CROSS JOIN LATERAL jsonb_array_elements( \
             CASE WHEN jsonb_typeof(v.body::jsonb -> 'relationships') = 'array' \
                  THEN v.body::jsonb -> 'relationships' ELSE '[]'::jsonb END \
         ) WITH ORDINALITY AS r(rel, ord) \
         CROSS JOIN LATERAL (SELECT lower(split_part( \
             r.rel -> 'target' -> 'id' ->> 'value', '::', 1)) AS root) AS target \
         WHERE v.body IS NOT NULL "
    };
}

/// Writes the index rows `source` contributes: one per target container of
/// its current trunk version's inline relationships, `targets`.
///
/// A row of a target the version no longer names goes, a row of a new
/// target comes, and every other row stays, so a change writes one row per
/// target it changes (#5220). The targets are derived from the body the
/// commit holds (`crate::party_binding::derive`), never from the stored body
/// again; a logically deleted head names none.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn replace(
    tx: &mut PgConnection,
    source: VoId,
    targets: &[uuid::Uuid],
) -> Result<(), StorageError> {
    sqlx::query(
        "WITH gone AS ( \
             DELETE FROM party_relationship_target t \
             WHERE t.source_vo_id = $1 \
               AND t.target_party_id NOT IN (SELECT unnest($2::uuid[])) \
         ) \
         INSERT INTO party_relationship_target (target_party_id, source_vo_id) \
         SELECT DISTINCT t.target, $1 FROM unnest($2::uuid[]) AS t(target) \
         ON CONFLICT ON CONSTRAINT pk_party_relationship_target DO NOTHING",
    )
    .bind(source)
    .bind(targets)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// One relationship whose target end is the party a read addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReverseRelationship {
    /// The source party's version container, which holds the relationship.
    pub source_vo_id: VoId,
    /// The relationship's `archetype_node_id`.
    pub node_id: Option<String>,
    /// The relationship's `name/value`.
    pub name: Option<String>,
    /// The relationship's `uid/value`.
    pub uid: Option<String>,
    /// The relationship's 1-based position in the source's `relationships`.
    pub position: i64,
    /// Whether no other relationship of the source has its `archetype_node_id`
    /// and its `name/value`.
    pub name_unique: bool,
    /// Whether no other relationship of the source has its `uid/value`.
    pub uid_unique: bool,
}

/// The relationships whose target is `target`, read from the current trunk
/// version of each source party the index names, ordered by source container
/// and then by position in its `relationships` list.
///
/// Each row carries what tells it from its siblings in that list.
///
/// A source under restriction of processing is left out, as every read path
/// leaves it out (`vo_head.restricted_at`).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn reverse_of(
    conn: &mut PgConnection,
    target: VoId,
) -> Result<Vec<ReverseRelationship>, StorageError> {
    let rows = sqlx::query(concat!(
        "SELECT h.vo_id AS source_vo_id, \
                r.rel ->> 'archetype_node_id' AS node_id, \
                r.rel -> 'name' ->> 'value' AS name, \
                r.rel -> 'uid' ->> 'value' AS uid, \
                r.ord AS position, \
                (SELECT count(*) = 1 FROM jsonb_array_elements(v.body::jsonb -> 'relationships') s(rel) \
                 WHERE s.rel ->> 'archetype_node_id' IS NOT DISTINCT FROM r.rel ->> 'archetype_node_id' \
                   AND s.rel -> 'name' ->> 'value' IS NOT DISTINCT FROM r.rel -> 'name' ->> 'value') \
                    AS name_unique, \
                (SELECT count(*) = 1 FROM jsonb_array_elements(v.body::jsonb -> 'relationships') s(rel) \
                 WHERE s.rel -> 'uid' ->> 'value' = r.rel -> 'uid' ->> 'value') AS uid_unique \
         FROM party_relationship_target t \
         JOIN vo_head h ON h.vo_id = t.source_vo_id AND h.restricted_at IS NULL ",
        current_relationships!(),
        "AND t.target_party_id = $1 AND target.root = $1::text \
         ORDER BY h.vo_id, r.ord"
    ))
    .bind(target)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(ReverseRelationship {
            source_vo_id: row.try_get("source_vo_id")?,
            node_id: row.try_get("node_id")?,
            name: row.try_get("name")?,
            uid: row.try_get("uid")?,
            position: row.try_get("position")?,
            name_unique: row.try_get("name_unique")?,
            uid_unique: row.try_get("uid_unique")?,
        });
    }
    Ok(out)
}
