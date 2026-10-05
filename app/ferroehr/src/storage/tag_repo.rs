// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for `item_tag` — the `ITEM_TAG` store shared by the EHR-scoped tag
//! surface (`ehr_id = <uuid>`) and the demographic (ehr-less,
//! `ehr_id IS NULL`) one.
//!
//! No openEHR spec governs the SQL (our own design). The RM `ITEM_TAG`
//! invariants (`Inv_key_valid`, `Inv_value_valid`) are enforced by the service
//! chapters before rows reach this module; the table is deliberately FK-less
//! (a tag may address a specific VERSION), so target-ownership checks also
//! live with the callers.

use std::collections::BTreeMap;

use sqlx::{PgConnection, PgPool, Row};

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// One `item_tag` row.
#[derive(Debug, Clone)]
pub struct TagRow {
    /// The tagged versioned object.
    pub target_vo_id: VoId,
    /// The `{creating_system_id}::{version_tree_id}` tail of a
    /// VERSION-addressed target; `None` = the container.
    pub target_version: Option<String>,
    /// The RM type of the tagged object.
    pub target_type: String,
    /// The tag key.
    pub key: String,
    /// The tag value, absent when the tag is a bare marker.
    pub value: Option<String>,
    /// The path within the target the tag applies to, if it is not whole-object.
    ///
    /// NOTE: stored and served **opaque** — never parsed, never resolved against
    /// the target's content. RM common
    /// `UML/classes/org.openehr.rm.common.item_tag.adoc` types `target_path` as a
    /// plain `String` admitting BOTH dialects in one attribute ("archetype (i.e.
    /// AQL) or RM path"), with no discriminator and no statement about when it
    /// resolves, so guessing a dialect would reject one of the two the RM allows
    /// and resolving at commit would invent an unstated precondition. The value
    /// round-trips byte-for-byte and participates only in the identity.
    pub target_path: Option<String>,
}

/// One tag to write (`replace_tags`).
#[derive(Debug, Clone)]
pub struct NewTag<'a> {
    /// The RM type of the tagged object.
    pub target_type: &'a str,
    /// The tag key.
    pub key: &'a str,
    /// The tag value, or `None` for a bare marker.
    pub value: Option<&'a str>,
    /// The path within the target the tag applies to, if it is not whole-object.
    pub target_path: Option<&'a str>,
}

fn tag_row(row: &sqlx::postgres::PgRow) -> Result<TagRow, StorageError> {
    Ok(TagRow {
        target_vo_id: row.try_get("target_vo_id")?,
        target_version: row.try_get("target_version")?,
        target_type: row.try_get("target_type")?,
        key: row.try_get("key")?,
        value: row.try_get("value")?,
        target_path: row.try_get("target_path")?,
    })
}

/// List tags in one scope (`ehr_id = $1`, `NULL` = the demographic store),
/// optionally narrowed to one target object and/or filtered by key / value /
/// target path, ordered by key.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn list_tags(
    pool: &PgPool,
    ehr_scope: Option<EhrId>,
    target: Option<(VoId, Option<&str>)>,
    key: Option<&str>,
    value: Option<&str>,
    target_path: Option<&str>,
) -> Result<Vec<TagRow>, StorageError> {
    // A named target narrows to ONE collection: the container's (`NULL`
    // target_version) or one VERSION's — the two are disjoint sets of the
    // same target_vo_id (RM ITEM_TAG.target: container or VERSION).
    let (target_vo_id, target_version) = match target {
        Some((vo, version)) => (Some(vo), version),
        None => (None, None),
    };
    let rows = sqlx::query(
        "SELECT target_vo_id, target_version, target_type, key, value, target_path \
         FROM item_tag \
         WHERE ehr_id IS NOT DISTINCT FROM $1 \
         AND ($2::uuid IS NULL OR (target_vo_id = $2 \
              AND target_version IS NOT DISTINCT FROM $3)) \
         AND ($4::text IS NULL OR key = $4) \
         AND ($5::text IS NULL OR value = $5) \
         AND ($6::text IS NULL OR target_path = $6) \
         ORDER BY key, target_path NULLS FIRST",
    )
    .bind(ehr_scope)
    .bind(target_vo_id)
    .bind(target_version)
    .bind(key)
    .bind(value)
    .bind(target_path)
    .fetch_all(pool)
    .await?;
    rows.iter().map(tag_row).collect()
}

/// Replaces the whole tag collection of one target in one scope.
///
/// Drops the existing collection, inserts the given set (`PUT`
/// full-collection semantics; an empty set clears all), and returns the
/// STORED collection in the `list_tags` order — the write path never re-reads
/// what it just wrote.
///
/// Runs on the caller's transaction.
///
/// A tag that survives the replace keeps its original `created_at`. An
/// `ITEM_TAG` identity, the (`key`, `target_path`) pair within one target
/// (ITS-REST overview `Requests_and_responses.md` §item-tag headers), appearing
/// in both the stored and the posted set is the same tag re-asserted, and its
/// creation instant is observable through the admin dump/export. An identity
/// absent from the posted set is removed and a new identity is created, so both
/// take the current instant.
///
/// NOTE: no openEHR spec governs this — our own design; RM common
/// `master07-tags.adoc` and RM ehr `master04-ehr_package.adoc` §Tags model
/// `ITEM_TAG` with no timestamp, so `created_at` is a storage column of ours.
///
/// The carry-forward is computed in three statements rather than one
/// data-modifying CTE because a `WITH prior AS (DELETE … RETURNING) INSERT …`
/// would have the insert's unique-index check race the CTE's delete within one
/// command, which PostgreSQL does not define away
/// (<https://www.postgresql.org/docs/18/queries-with.html> §Data-Modifying
/// Statements in WITH). All three run on the caller's transaction, so the
/// replace stays atomic.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn replace_tags(
    tx: &mut PgConnection,
    ehr_scope: Option<EhrId>,
    target_vo_id: VoId,
    target_version: Option<&str>,
    tags: &[NewTag<'_>],
) -> Result<Vec<TagRow>, StorageError> {
    // The creation instants of the collection as it stands, keyed by ITEM_TAG
    // identity, so a surviving tag can keep its own.
    let prior = sqlx::query(
        "SELECT key, target_path, created_at FROM item_tag \
         WHERE ehr_id IS NOT DISTINCT FROM $1 \
         AND target_vo_id = $2 AND target_version IS NOT DISTINCT FROM $3",
    )
    .bind(ehr_scope)
    .bind(target_vo_id)
    .bind(target_version)
    .fetch_all(&mut *tx)
    .await?;
    let mut created_before: BTreeMap<(String, Option<String>), jiff::Timestamp> = BTreeMap::new();
    for row in &prior {
        let key: String = row.try_get("key")?;
        let target_path: Option<String> = row.try_get("target_path")?;
        let created_at: jiff_sqlx::Timestamp = row.try_get("created_at")?;
        created_before.insert((key, target_path), created_at.to_jiff());
    }
    // Replace exactly ONE collection: the container's or one VERSION's —
    // never both (RM ITEM_TAG.target: container or VERSION are distinct
    // targets).
    sqlx::query(
        "DELETE FROM item_tag WHERE ehr_id IS NOT DISTINCT FROM $1 \
         AND target_vo_id = $2 AND target_version IS NOT DISTINCT FROM $3",
    )
    .bind(ehr_scope)
    .bind(target_vo_id)
    .bind(target_version)
    .execute(&mut *tx)
    .await?;
    if tags.is_empty() {
        return Ok(Vec::new());
    }
    // One multi-row insert for the whole set (never a per-tag round trip).
    // The ITEM_TAG identity within one target is the (key, target_path) PAIR
    // (ITS-REST overview Requests_and_responses.md §item-tag headers), so the
    // in-set last-wins dedupe keys on the pair — same-key tags on different
    // paths coexist. A single INSERT cannot upsert over its own rows ("ON
    // CONFLICT DO UPDATE command cannot affect row a second time").
    let mut last_by_identity: indexmap::IndexMap<(&str, Option<&str>), &NewTag<'_>> =
        indexmap::IndexMap::new();
    for tag in tags {
        last_by_identity.insert((tag.key, tag.target_path), tag);
    }
    let now = jiff::Timestamp::now();
    let (mut types, mut keys, mut values, mut paths, mut created) = (
        Vec::with_capacity(last_by_identity.len()),
        Vec::with_capacity(last_by_identity.len()),
        Vec::with_capacity(last_by_identity.len()),
        Vec::with_capacity(last_by_identity.len()),
        Vec::with_capacity(last_by_identity.len()),
    );
    for tag in last_by_identity.values() {
        types.push(tag.target_type);
        keys.push(tag.key);
        values.push(tag.value);
        paths.push(tag.target_path);
        // A surviving identity keeps its own creation instant; a new one is
        // created now (see the function doc).
        let carried = created_before
            .get(&(tag.key.to_owned(), tag.target_path.map(str::to_owned)))
            .copied()
            .unwrap_or(now);
        created.push(jiff_sqlx::Timestamp::from(carried));
    }
    // RETURNING hands back the stored collection (the replace's insert IS the
    // whole new set), in the same order `list_tags` serves, so the write path
    // never re-reads the collection it just wrote.
    let stored = sqlx::query(
        "INSERT INTO item_tag \
         (ehr_id, target_vo_id, target_version, target_type, key, value, target_path, created_at) \
         SELECT $1, $2, $3, t.* \
         FROM UNNEST($4::text[], $5::text[], $6::text[], $7::text[], $8::timestamptz[]) AS t \
         RETURNING target_vo_id, target_version, target_type, key, value, target_path \
         ",
    )
    .bind(ehr_scope)
    .bind(target_vo_id)
    .bind(target_version)
    .bind(&types)
    .bind(&keys)
    .bind(&values)
    .bind(&paths)
    .bind(&created)
    .fetch_all(&mut *tx)
    .await?;
    let mut out = Vec::with_capacity(stored.len());
    for row in &stored {
        out.push(tag_row(row)?);
    }
    out.sort_by(|a, b| {
        a.key
            .cmp(&b.key)
            .then_with(|| match (&a.target_path, &b.target_path) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (Some(x), Some(y)) => x.cmp(y),
            })
    });
    Ok(out)
}

/// Delete a target collection's tags by key, returning whether any row was
/// deleted.
///
/// The wire addresses tags by `key` alone (the Release-1.1.0 tag routes carry
/// no path selector), while the `ITEM_TAG` identity is the (`key`,
/// `target_path`) pair — so a key delete removes EVERY tag under that key in
/// the addressed collection, a set delete.
///
/// NOTE: the delete is PHYSICAL — no openEHR spec governs this, our own design,
/// and the alternative is unrepresentable: "logical delete" is a change-control
/// concept (a new VERSION in lifecycle state `523|deleted|`, RM common
/// `master06-change_control_package.adoc`), while RM ehr
/// `master04-ehr_package.adoc` §Tags puts `ITEM_TAG` outside change control
/// entirely — unversioned, no `uid`, no lifecycle state, so there is no object
/// for a tombstone to be a version OF. The released wire agrees by omission: the
/// tag DELETE's only success branch is `204`.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn delete_tag(
    pool: &PgPool,
    ehr_scope: Option<EhrId>,
    target_vo_id: VoId,
    target_version: Option<&str>,
    key: &str,
) -> Result<bool, StorageError> {
    let deleted = sqlx::query(
        "DELETE FROM item_tag WHERE ehr_id IS NOT DISTINCT FROM $1 \
         AND target_vo_id = $2 AND target_version IS NOT DISTINCT FROM $3 AND key = $4",
    )
    .bind(ehr_scope)
    .bind(target_vo_id)
    .bind(target_version)
    .bind(key)
    .execute(pool)
    .await?;
    Ok(deleted.rows_affected() > 0)
}
