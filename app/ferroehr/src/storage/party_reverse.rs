// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for `party_relationship_target`, the target-side index of the
//! `PARTY_RELATIONSHIP`s a party carries inline and of FerroEHR's standalone
//! relationship containers.
//!
//! RM demographic `master02-demographic_package.adoc` §Party Relationships
//! stores a relationship in the data of its source party and lists it, at the
//! target end, in `reverse_relationships`. The source body holds the forward
//! direction; this index holds the reverse one, one row per interval over
//! which the source's trunk named the target, so a read answers the list as it
//! stands now or as it stood at an instant (#5473). No openEHR spec governs the
//! index relation or its SQL: our own design.

use sqlx::{PgConnection, Row};

use crate::ids::VoId;
use crate::storage::error::StorageError;

/// The relationships of one source version `v`, one row each with its
/// position and its target root: the lateral part of [`reverse_of`] and
/// [`rebuild`], completed by their own `WHERE`.
///
/// The `relationships` attribute is read only when it is an array, so a body
/// without one contributes no row; a standalone `PARTY_RELATIONSHIP` version
/// is its own one relationship. A target id is a `HIER_OBJECT_ID` value,
/// `root` or `root::extension`, and the root is what names a party container,
/// as `crate::party_binding::derive::container` reads it.
macro_rules! relationships_of_v {
    () => {
        "CROSS JOIN LATERAL jsonb_array_elements( \
             CASE WHEN v.kind = 'PARTY_RELATIONSHIP' THEN jsonb_build_array(v.body::jsonb) \
                  WHEN jsonb_typeof(v.body::jsonb -> 'relationships') = 'array' \
                  THEN v.body::jsonb -> 'relationships' ELSE '[]'::jsonb END \
         ) WITH ORDINALITY AS r(rel, ord) \
         CROSS JOIN LATERAL (SELECT lower(split_part( \
             r.rel -> 'target' -> 'id' ->> 'value', '::', 1)) AS root) AS target \
         WHERE v.body IS NOT NULL "
    };
}

/// The columns [`reverse_of`] reads, from the index row `t`, the source head
/// `h`, the relationship `r` and the source version `v` its siblings are
/// counted in.
macro_rules! reverse_columns {
    () => {
        "SELECT h.vo_id AS source_vo_id, \
                r.rel ->> 'archetype_node_id' AS node_id, \
                r.rel -> 'name' ->> 'value' AS name, \
                v.kind = 'PARTY_RELATIONSHIP' AS standalone, \
                r.rel -> 'uid' ->> 'value' AS uid, \
                r.ord AS position, \
                (SELECT count(*) = 1 FROM jsonb_array_elements(v.body::jsonb -> 'relationships') sib(rel) \
                 WHERE sib.rel ->> 'archetype_node_id' IS NOT DISTINCT FROM r.rel ->> 'archetype_node_id' \
                   AND sib.rel -> 'name' ->> 'value' IS NOT DISTINCT FROM r.rel -> 'name' ->> 'value') \
                    AS name_unique, \
                (SELECT count(*) = 1 FROM jsonb_array_elements(v.body::jsonb -> 'relationships') sib(rel) \
                 WHERE sib.rel -> 'uid' ->> 'value' = r.rel -> 'uid' ->> 'value') AS uid_unique \
         FROM party_relationship_target t \
         JOIN vo_head h ON h.vo_id = t.source_vo_id AND h.restricted_at IS NULL "
    };
}

/// Brings the rows of `source` to the targets its new trunk head names,
/// `targets`, committed at `at`.
///
/// The open interval of a target the head no longer names closes at `at`, a
/// new target opens one at `at`, and every other row stays, so a change writes
/// one row per target it changes (#5220). An interval that would close at the
/// instant it opened held for no instant and goes. The targets are derived
/// from the body the commit holds (`crate::party_binding::derive`), never from
/// the stored body again; a logically deleted head names none.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn replace(
    tx: &mut PgConnection,
    source: VoId,
    targets: &[uuid::Uuid],
    at: jiff::Timestamp,
) -> Result<(), StorageError> {
    sqlx::query(
        "WITH closed AS ( \
             UPDATE party_relationship_target t SET valid_to = $3 \
             WHERE t.source_vo_id = $1 AND t.valid_to IS NULL AND t.valid_from < $3 \
               AND t.target_party_id <> ALL($2::uuid[]) \
         ), emptied AS ( \
             DELETE FROM party_relationship_target t \
             WHERE t.source_vo_id = $1 AND t.valid_to IS NULL AND t.valid_from >= $3 \
               AND t.target_party_id <> ALL($2::uuid[]) \
         ) \
         INSERT INTO party_relationship_target (target_party_id, source_vo_id, valid_from) \
         SELECT DISTINCT t.target, $1, $3 FROM unnest($2::uuid[]) AS t(target) \
         ON CONFLICT (target_party_id, source_vo_id) WHERE valid_to IS NULL DO NOTHING",
    )
    .bind(source)
    .bind(targets)
    .bind(jiff_sqlx::Timestamp::from(at))
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Rebuilds every row of `source` from its stored trunk versions: what an
/// import or an archive load writes after the versions it lands.
///
/// Consecutive trunk versions that name a target form one interval, from the
/// commit of the first to the commit of the version after the last, open when
/// the last is the trunk head; a logically deleted version names none.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn rebuild(tx: &mut PgConnection, source: VoId) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM party_relationship_target WHERE source_vo_id = $1")
        .bind(source)
        .execute(&mut *tx)
        .await?;
    sqlx::query(concat!(
        "WITH v AS ( \
             SELECT v.trunk_version, v.committed_at, v.body, v.lifecycle_state, v.kind, \
                    lead(v.committed_at) OVER (ORDER BY v.trunk_version) AS next_at \
             FROM version v \
             WHERE v.vo_id = $1 AND v.branch_number = 0 \
         ), named AS ( \
             SELECT DISTINCT v.trunk_version, v.committed_at, v.next_at, \
                    target.root::uuid AS target \
             FROM v ",
        relationships_of_v!(),
        "AND v.lifecycle_state <> '523' \
               AND target.root ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' \
         ), islands AS ( \
             SELECT n.target, n.committed_at, n.next_at, \
                    n.trunk_version - row_number() OVER ( \
                        PARTITION BY n.target ORDER BY n.trunk_version) AS island \
             FROM named n \
         ) \
         INSERT INTO party_relationship_target \
             (target_party_id, source_vo_id, valid_from, valid_to) \
         SELECT i.target, $1, min(i.committed_at), \
                CASE WHEN bool_or(i.next_at IS NULL) THEN NULL ELSE max(i.next_at) END \
         FROM islands i \
         GROUP BY i.target, i.island \
         HAVING bool_or(i.next_at IS NULL) OR max(i.next_at) > min(i.committed_at)"
    ))
    .bind(source)
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
    /// Whether the relationship is a standalone `PARTY_RELATIONSHIP`
    /// container, `source_vo_id` then being that container.
    pub standalone: bool,
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

/// The relationships whose target is `target`, ordered by source container
/// and then by position in its `relationships` list: as they stand now when
/// `at` is `None`, else as they stood at `at`.
///
/// Now, each source's current trunk version holds them; at `at`, the index
/// names the sources whose interval holds `at`, and the trunk version each
/// committed last at or before `at` holds them. Both are index probes, never a
/// walk of a source's versions. A source under restriction of processing is
/// left out, as every read path leaves it out (`vo_head.restricted_at`).
///
/// Each row carries what tells it from its siblings in that list.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn reverse_of(
    conn: &mut PgConnection,
    target: VoId,
    at: Option<jiff::Timestamp>,
) -> Result<Vec<ReverseRelationship>, StorageError> {
    let rows = match at {
        None => {
            sqlx::query(concat!(
                reverse_columns!(),
                "JOIN version v ON v.tier = h.tier AND v.vo_id = h.vo_id \
                              AND v.sys_version = h.trunk_head_sys_version ",
                relationships_of_v!(),
                "AND t.target_party_id = $1 AND t.valid_to IS NULL \
                   AND target.root = $1::text \
                 ORDER BY h.vo_id, r.ord"
            ))
            .bind(target)
            .fetch_all(&mut *conn)
            .await?
        }
        Some(at) => {
            sqlx::query(concat!(
                reverse_columns!(),
                "CROSS JOIN LATERAL ( \
                     SELECT s.body, s.kind FROM version s \
                     WHERE s.tier = h.tier AND s.vo_id = h.vo_id AND s.branch_number = 0 \
                       AND s.committed_at <= $2 \
                     ORDER BY s.committed_at DESC, s.trunk_version DESC LIMIT 1 \
                 ) AS v ",
                relationships_of_v!(),
                "AND t.target_party_id = $1 AND t.valid_from <= $2 \
                   AND (t.valid_to IS NULL OR t.valid_to > $2) \
                   AND target.root = $1::text \
                 ORDER BY h.vo_id, r.ord"
            ))
            .bind(target)
            .bind(jiff_sqlx::Timestamp::from(at))
            .fetch_all(&mut *conn)
            .await?
        }
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(ReverseRelationship {
            source_vo_id: row.try_get("source_vo_id")?,
            node_id: row.try_get("node_id")?,
            name: row.try_get("name")?,
            standalone: row.try_get("standalone")?,
            uid: row.try_get("uid")?,
            position: row.try_get("position")?,
            name_unique: row.try_get("name_unique")?,
            uid_unique: row.try_get("uid_unique")?,
        });
    }
    Ok(out)
}
