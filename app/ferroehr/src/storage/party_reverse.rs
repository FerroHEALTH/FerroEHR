// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for `party_relationship_target`, the target-side index of the
//! `PARTY_RELATIONSHIP`s a party carries inline.
//!
//! RM demographic `master02-demographic_package.adoc` §Party Relationships
//! stores a relationship in the data of its source party and lists it, at the
//! target end, in `reverse_relationships`. The source body holds the forward
//! direction; this index holds the reverse one, rebuilt from the source's
//! current trunk version on every write that changes it. No openEHR spec
//! governs the index relation or its SQL: our own design.

use sqlx::{PgConnection, Row};

use crate::ids::VoId;
use crate::storage::error::StorageError;

/// The relationships of the current trunk version of each party `h` names,
/// one row each with its position, its target root and its identity within
/// the body: the `FROM` clause the two statements below share, completed by
/// their own `WHERE`.
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

/// A party container id is a UUID; a target whose root is another `UID` form
/// (an ISO OID, an internet id) names no container of this store.
const UUID_PATTERN: &str = "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$";

/// Rebuilds the index rows `source` contributes, on the caller's transaction.
///
/// Every row naming it as the source goes, and one row per distinct UUID
/// target of its current trunk version's `relationships` comes back. A
/// logically deleted current version carries no body, so it contributes none.
///
/// Runs after the write has advanced the object's head row, so it reads the
/// version that write made current.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn reindex(tx: &mut PgConnection, source: VoId) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM party_relationship_target WHERE source_vo_id = $1")
        .bind(source)
        .execute(&mut *tx)
        .await?;
    sqlx::query(concat!(
        "INSERT INTO party_relationship_target (target_party_id, source_vo_id) \
         SELECT DISTINCT target.root::uuid, h.vo_id FROM vo_head h ",
        current_relationships!(),
        "AND h.vo_id = $1 AND target.root ~ $2"
    ))
    .bind(source)
    .bind(UUID_PATTERN)
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
}

/// The relationships whose target is `target`, read from the current trunk
/// version of each source party the index names, ordered by source container
/// and then by position in its `relationships` list.
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
                r.rel -> 'name' ->> 'value' AS name \
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
        });
    }
    Ok(out)
}
