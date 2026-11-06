// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the binding side table.
//!
//! `binding_identifier`, `binding_membership` and `binding_representation`
//! hold the rows each party head contributes, derived in the commit's
//! transaction; [`refill`] rebuilds them from the versions alone, and the
//! lookup reads them.
//!
//! A `PERSON` of a mapped template has a key row for each identifier at its
//! key selector when it performs a professional `ROLE` or is the source of a
//! representation relationship, and none otherwise. A professional `ROLE` is
//! a `ROLE` of the mapped role template listed in the `PERSON`'s `roles`
//! (RM demographic `org.openehr.rm.demographic.actor.adoc`, "Identifiers of
//! the Version container for each Role played by this Party"), whose
//! `performer` names the `PERSON`'s container
//! (`org.openehr.rm.demographic.role.adoc`) and whose head is `complete` or
//! `incomplete`; its `time_validity` is kept as dates, so whether it is live
//! is read at the request. A membership and a representation are inline
//! `PARTY_RELATIONSHIP`s of the source party (`master02-demographic_package.adoc`
//! §Party Relationships); standalone relationship containers never count. No
//! openEHR spec governs the side table or its SQL: our own design.

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::ids::VoId;
use crate::party_binding::{BindingMapping, IdentifierSelector};
use crate::storage::error::StorageError;
use crate::storage::party_reverse;
use crate::versioning::change::Committed;
use crate::versioning::{Kind, SigningCtx};

/// The trunk head of each party of `$1` (every party when `$1` is NULL) of
/// the kind `$2` whose body names the canonical template `$3` in
/// `archetype_details.template_id`, not deleted, with its body:
/// the `p` relation the derivations below share.
macro_rules! heads {
    () => {
        "WITH p AS ( \
             SELECT h.vo_id, h.lifecycle_state, v.body::jsonb AS body \
             FROM vo_head h \
             JOIN version v ON v.tier = h.tier AND v.vo_id = h.vo_id \
                           AND v.sys_version = h.trunk_head_sys_version \
             WHERE h.kind = $2 AND h.lifecycle_state <> '523' AND v.body IS NOT NULL \
               AND lower(btrim(v.body::jsonb -> 'archetype_details' -> 'template_id' ->> 'value')) \
                   = $3 \
               AND ($1::uuid[] IS NULL OR h.vo_id = ANY($1::uuid[])) \
         ) "
    };
}

/// The root of a `PARTY_REF.id` value `$reference`, as a container UUID when
/// it is one: a party container id is a UUID, and a reference whose root is
/// another `UID` form names no container of this store.
macro_rules! container {
    ($reference:literal) => {
        concat!(
            "(SELECT CASE WHEN root ~ '",
            "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
            "' THEN root::uuid END FROM (SELECT lower(split_part(",
            $reference,
            ", '::', 1)) AS root) AS r)"
        )
    };
}

/// The inline `relationships` of each `p.body`, one `rel` row each.
macro_rules! relationships {
    () => {
        "CROSS JOIN LATERAL jsonb_array_elements( \
             CASE WHEN jsonb_typeof(p.body -> 'relationships') = 'array' \
                  THEN p.body -> 'relationships' ELSE '[]'::jsonb END) AS r(rel) "
    };
}

/// Rebuilds the side-table rows the party `vo_id` of `kind` contributes, on
/// the commit transaction `tx`, after the write has advanced its head row.
///
/// A `PERSON` rebuilds its key and representation rows, and a deleted one
/// leaves its key as a tombstone; a `ROLE` rebuilds the rows of every
/// `PERSON` it names as performer; a `GROUP` rebuilds its `externalId` and its
/// memberships. Every other kind contributes nothing.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn reindex(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    vo_id: VoId,
) -> Result<(), StorageError> {
    if mapping.is_empty() {
        return Ok(());
    }
    match kind {
        Kind::Person => reindex_persons(tx, mapping, Some(&[vo_id.0]), true).await,
        Kind::Role => {
            let performers = performers(tx, vo_id).await?;
            if performers.is_empty() {
                return Ok(());
            }
            reindex_persons(tx, mapping, Some(&performers), false).await
        }
        Kind::Group => reindex_groups(tx, mapping, Some(&[vo_id.0])).await,
        _ => Ok(()),
    }
}

/// Rebuilds the reverse-relationship index and the side-table rows of the
/// party `committed` wrote, on its commit transaction `tx`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn reindex_committed(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    committed: &Committed,
) -> Result<(), StorageError> {
    party_reverse::reindex(tx, committed.vo_id).await?;
    reindex(tx, ctx.binding, committed.kind, committed.vo_id).await
}

/// Rebuilds the reverse-relationship index and the side-table rows of the
/// party `vo_id`, whose stored kind is `kind`, loaded from an archive on `tx`.
///
/// A `ROLE` loaded after its `PERSON` rebuilds that `PERSON`'s rows, so the
/// rows do not depend on the order the archive holds the parties in.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn reindex_loaded(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: &str,
    vo_id: VoId,
) -> Result<(), StorageError> {
    party_reverse::reindex(tx, vo_id).await?;
    match Kind::from_type(kind) {
        Some(kind) => reindex(tx, mapping, kind, vo_id).await,
        None => Ok(()),
    }
}

/// Rebuilds every row of the side table from the party versions alone, on
/// `tx`.
///
/// Every row goes, and the rows of every party head come back, the
/// tombstones of the deleted `PERSON`s included, as the commits wrote them.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn refill(tx: &mut PgConnection, mapping: &BindingMapping) -> Result<(), StorageError> {
    for statement in [
        "DELETE FROM binding_identifier",
        "DELETE FROM binding_membership",
        "DELETE FROM binding_representation",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    reindex_persons(tx, mapping, None, true).await?;
    reindex_groups(tx, mapping, None).await
}

/// The `PERSON` containers every version of the `ROLE` `role` names as its
/// performer, so a `ROLE` whose performer changed rebuilds both.
async fn performers(tx: &mut PgConnection, role: VoId) -> Result<Vec<Uuid>, StorageError> {
    let ids: Vec<Option<Uuid>> = sqlx::query_scalar(concat!(
        "SELECT DISTINCT ",
        container!("v.body::jsonb -> 'performer' -> 'id' ->> 'value'"),
        " FROM version v WHERE v.vo_id = $1 AND v.body IS NOT NULL"
    ))
    .bind(role)
    .fetch_all(&mut *tx)
    .await?;
    Ok(ids.into_iter().flatten().collect())
}

/// Rebuilds the key and representation rows of the `PERSON`s `ids` (every
/// `PERSON` when `None`), and, when `tombstones` is set, leaves the key of
/// each deleted one as a tombstone.
async fn reindex_persons(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    ids: Option<&[Uuid]>,
    tombstones: bool,
) -> Result<(), StorageError> {
    if let Some(ids) = ids {
        sqlx::query(
            "UPDATE binding_identifier SET retired_at = now() \
             WHERE party_id = ANY($1) AND selector = 'key' AND retired_at IS NULL",
        )
        .bind(ids)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE binding_representation SET retired_at = now() \
             WHERE representative_id = ANY($1) AND retired_at IS NULL",
        )
        .bind(ids)
        .execute(&mut *tx)
        .await?;
    }
    for (template, key) in &mapping.persons {
        insert_representations(tx, mapping, ids, template).await?;
        insert_keys(tx, mapping, ids, template, key).await?;
        if tombstones {
            insert_tombstones(tx, ids, template, key).await?;
        }
    }
    // A key a live row binds again is no longer a tombstone.
    sqlx::query(
        "UPDATE binding_identifier t SET deprovisioned_at = NULL \
         FROM binding_identifier l \
         WHERE ($1::uuid[] IS NULL OR l.party_id = ANY($1::uuid[])) \
           AND l.selector = 'key' AND l.retired_at IS NULL \
           AND t.selector = 'key' AND t.deprovisioned_at IS NOT NULL \
           AND t.issuer = l.issuer AND t.identifier_type = l.identifier_type \
           AND t.value = l.value",
    )
    .bind(ids)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Inserts the representation rows of the `PERSON`s `ids` of `template`.
async fn insert_representations(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    ids: Option<&[Uuid]>,
    template: &str,
) -> Result<(), StorageError> {
    let Some(archetype) = &mapping.representation else {
        return Ok(());
    };
    let (codes, mapped): (Vec<&str>, Vec<&str>) = mapping
        .relationship_codes
        .iter()
        .map(|(code, mapped)| (code.as_str(), mapped.as_str()))
        .unzip();
    sqlx::query(concat!(
        heads!(),
        "INSERT INTO binding_representation \
             (representative_id, subject_id, relationship, validity) \
         SELECT p.vo_id, t.subject, \
                (SELECT c.mapped FROM unnest($5::text[], $6::text[]) AS c(code, mapped) \
                 WHERE c.code = r.rel -> 'name' -> 'defining_code' ->> 'code_string'), \
                range_agg(party.binding_validity(r.rel -> 'time_validity')) \
         FROM p ",
        relationships!(),
        "CROSS JOIN LATERAL (SELECT ",
        container!("r.rel -> 'target' -> 'id' ->> 'value'"),
        " AS subject) AS t \
         WHERE lower(r.rel ->> 'archetype_node_id') = $4 AND t.subject IS NOT NULL \
         GROUP BY 1, 2, 3"
    ))
    .bind(ids)
    .bind(Kind::Person.as_str())
    .bind(template)
    .bind(archetype)
    .bind(codes)
    .bind(mapped)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The canonical form of `DV_IDENTIFIER` `id`, the value of `id.type`: lower
/// case for a GUID or UUID, as `ferrohealth-smart` puts a claim value.
macro_rules! canonical_value {
    ($id:literal) => {
        concat!(
            "CASE WHEN lower(",
            $id,
            " ->> 'type') IN ('guid', 'uuid') THEN lower(",
            $id,
            " ->> 'id') ELSE ",
            $id,
            " ->> 'id' END"
        )
    };
}

/// Inserts the live key rows of the `PERSON`s `ids` of `template`: one per
/// identifier at `key`, for a `PERSON` that performs a professional `ROLE` or
/// is the source of a representation relationship.
async fn insert_keys(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    ids: Option<&[Uuid]>,
    template: &str,
    key: &IdentifierSelector,
) -> Result<(), StorageError> {
    sqlx::query(concat!(
        heads!(),
        ", professional AS ( \
             SELECT p.vo_id, range_agg(party.binding_validity(rv.body::jsonb -> 'time_validity')) \
                    AS dates \
             FROM p \
             CROSS JOIN LATERAL jsonb_array_elements( \
                 CASE WHEN jsonb_typeof(p.body -> 'roles') = 'array' \
                      THEN p.body -> 'roles' ELSE '[]'::jsonb END) AS r(ref) \
             CROSS JOIN LATERAL (SELECT ",
        container!("r.ref -> 'id' ->> 'value'"),
        " AS role_id) AS listed \
             JOIN vo_head rh ON rh.vo_id = listed.role_id \
             JOIN version rv ON rv.tier = rh.tier AND rv.vo_id = rh.vo_id \
                            AND rv.sys_version = rh.trunk_head_sys_version \
             WHERE $7::text IS NOT NULL AND rh.kind = 'ROLE' \
               AND rh.lifecycle_state IN ('532', '553') \
               AND rv.body IS NOT NULL \
               AND lower(btrim(rv.body::jsonb -> 'archetype_details' -> 'template_id' ->> 'value')) \
                   = $7 \
               AND ",
        container!("rv.body::jsonb -> 'performer' -> 'id' ->> 'value'"),
        " = p.vo_id \
             GROUP BY p.vo_id \
         ) \
         INSERT INTO binding_identifier \
             (party_id, selector, issuer, identifier_type, value, lifecycle_state, \
              professional, representative) \
         SELECT p.vo_id, 'key', id ->> 'issuer', id ->> 'type', ",
        canonical_value!("id"),
        ", p.lifecycle_state, coalesce(pr.dates, '{}'), rep.present \
         FROM p \
         CROSS JOIN LATERAL jsonb_path_query(p.body, $4::jsonpath, $5::jsonb) AS id \
         LEFT JOIN professional pr ON pr.vo_id = p.vo_id \
         CROSS JOIN LATERAL (SELECT EXISTS ( \
             SELECT 1 FROM binding_representation b \
             WHERE b.representative_id = p.vo_id AND b.retired_at IS NULL) AS present) AS rep \
         WHERE id ->> 'type' = $6 AND id ->> 'issuer' IS NOT NULL AND id ->> 'id' IS NOT NULL \
           AND (NOT isempty(coalesce(pr.dates, '{}')) OR rep.present)"
    ))
    .bind(ids)
    .bind(Kind::Person.as_str())
    .bind(template)
    .bind(&key.json_path)
    .bind(&key.variables)
    .bind(&key.identifier_type)
    .bind(mapping.professional_role.as_deref())
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Inserts the tombstones of the deleted `PERSON`s `ids` of `template`: the
/// key each carried in its last version with content, retired and
/// deprovisioned, once per key.
async fn insert_tombstones(
    tx: &mut PgConnection,
    ids: Option<&[Uuid]>,
    template: &str,
    key: &IdentifierSelector,
) -> Result<(), StorageError> {
    sqlx::query(concat!(
        "INSERT INTO binding_identifier \
             (party_id, selector, issuer, identifier_type, value, lifecycle_state, \
              retired_at, deprovisioned_at) \
         SELECT DISTINCT h.vo_id, 'key', id ->> 'issuer', id ->> 'type', ",
        canonical_value!("id"),
        ", last.lifecycle_state, now(), now() \
         FROM vo_head h \
         CROSS JOIN LATERAL ( \
             SELECT v.body::jsonb AS body, v.lifecycle_state FROM version v \
             WHERE v.tier = h.tier AND v.vo_id = h.vo_id AND v.body IS NOT NULL \
             ORDER BY v.sys_version DESC LIMIT 1) AS last \
         CROSS JOIN LATERAL jsonb_path_query(last.body, $4::jsonpath, $5::jsonb) AS id \
         WHERE h.kind = $2 AND h.lifecycle_state = '523' \
           AND ($1::uuid[] IS NULL OR h.vo_id = ANY($1::uuid[])) \
           AND lower(btrim(last.body -> 'archetype_details' -> 'template_id' ->> 'value')) = $3 \
           AND id ->> 'type' = $6 AND id ->> 'issuer' IS NOT NULL AND id ->> 'id' IS NOT NULL \
           AND NOT EXISTS ( \
               SELECT 1 FROM binding_identifier t \
               WHERE t.party_id = h.vo_id AND t.deprovisioned_at IS NOT NULL)"
    ))
    .bind(ids)
    .bind(Kind::Person.as_str())
    .bind(template)
    .bind(&key.json_path)
    .bind(&key.variables)
    .bind(&key.identifier_type)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Rebuilds the `externalId` and membership rows of the `GROUP`s `ids`
/// (every `GROUP` when `None`).
///
/// A membership a version keeps keeps its row; a removed one is retired, and
/// an added one inserted, so a change writes one row per member it changes.
async fn reindex_groups(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    ids: Option<&[Uuid]>,
) -> Result<(), StorageError> {
    let Some((template, external_id)) = &mapping.group else {
        return Ok(());
    };
    if let Some(ids) = ids {
        sqlx::query(
            "UPDATE binding_identifier SET retired_at = now() \
             WHERE party_id = ANY($1) AND selector = 'group_external_id' AND retired_at IS NULL",
        )
        .bind(ids)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(concat!(
        heads!(),
        "INSERT INTO binding_identifier \
             (party_id, selector, issuer, identifier_type, value, lifecycle_state) \
         SELECT p.vo_id, 'group_external_id', id ->> 'issuer', id ->> 'type', id ->> 'id', \
                p.lifecycle_state \
         FROM p CROSS JOIN LATERAL jsonb_path_query(p.body, $4::jsonpath, $5::jsonb) AS id \
         WHERE id ->> 'type' = $6 AND id ->> 'issuer' IS NOT NULL AND id ->> 'id' IS NOT NULL"
    ))
    .bind(ids)
    .bind(Kind::Group.as_str())
    .bind(template)
    .bind(&external_id.json_path)
    .bind(&external_id.variables)
    .bind(&external_id.identifier_type)
    .execute(&mut *tx)
    .await?;
    sqlx::query(concat!(
        heads!(),
        ", wanted AS ( \
             SELECT DISTINCT p.vo_id AS group_id, t.member AS member_id FROM p ",
        relationships!(),
        "CROSS JOIN LATERAL (SELECT ",
        container!("r.rel -> 'target' -> 'id' ->> 'value'"),
        " AS member) AS t \
             WHERE $4::text IS NOT NULL AND lower(r.rel ->> 'archetype_node_id') = $4 \
               AND t.member IS NOT NULL \
         ), gone AS ( \
             UPDATE binding_membership m SET retired_at = now() \
             WHERE m.retired_at IS NULL \
               AND ($1::uuid[] IS NULL OR m.group_id = ANY($1::uuid[])) \
               AND NOT EXISTS (SELECT 1 FROM wanted w \
                               WHERE w.group_id = m.group_id AND w.member_id = m.member_id) \
         ) \
         INSERT INTO binding_membership (group_id, member_id) \
         SELECT w.group_id, w.member_id FROM wanted w \
         WHERE NOT EXISTS (SELECT 1 FROM binding_membership m \
                           WHERE m.group_id = w.group_id AND m.member_id = w.member_id \
                             AND m.retired_at IS NULL)"
    ))
    .bind(ids)
    .bind(Kind::Group.as_str())
    .bind(template)
    .bind(mapping.membership.as_deref())
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// One key row the lookup reads: a live row, or the tombstone of a deleted
/// `PERSON`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    /// The `PERSON`'s container.
    pub party_id: Uuid,
    /// The head's `version_lifecycle_state` code.
    pub lifecycle_state: String,
    /// Whether a professional `ROLE` is valid on the lookup's date.
    pub professional: bool,
    /// Whether the `PERSON` is the source of a representation relationship.
    pub representative: bool,
    /// Whether the row is live; a row that is not is a tombstone.
    pub live: bool,
}

/// The key rows of the identifier `value` of `identifier_type` under
/// `issuer`, live and tombstoned, with the professional state on `today`
/// (`YYYY-MM-DD`); answered from `ix_binding_identifier_lookup` alone.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn key_rows(
    conn: &mut PgConnection,
    issuer: &str,
    identifier_type: &str,
    value: &str,
    today: &str,
) -> Result<Vec<KeyRow>, StorageError> {
    let rows = sqlx::query(LOOKUP)
        .bind(issuer)
        .bind(identifier_type)
        .bind(value)
        .bind(today)
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(KeyRow {
            party_id: row.try_get("party_id")?,
            lifecycle_state: row.try_get("lifecycle_state")?,
            professional: row.try_get("professional")?,
            representative: row.try_get("representative")?,
            live: row.try_get("live")?,
        });
    }
    Ok(out)
}

/// The lookup statement of [`key_rows`], public so a test can `EXPLAIN` it.
pub const LOOKUP: &str = "SELECT party_id, lifecycle_state, professional @> $4::date AS professional, \
            representative, retired_at IS NULL AS live \
     FROM binding_identifier \
     WHERE selector = 'key' AND issuer = $1 AND identifier_type = $2 AND value = $3 \
       AND (retired_at IS NULL OR deprovisioned_at IS NOT NULL) \
     ORDER BY party_id";

/// One live membership: a member, its group and the group's `externalId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipRow {
    /// The member `PERSON`'s container.
    pub member: Uuid,
    /// The `GROUP`'s container.
    pub group: Uuid,
    /// The `GROUP`'s `externalId` value under the queried issuer.
    pub external_id: String,
}

/// The live memberships of `members` in a `GROUP` whose head is `complete` or
/// `incomplete` and whose `externalId` is under `issuer`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn memberships(
    conn: &mut PgConnection,
    members: &[Uuid],
    issuer: &str,
) -> Result<Vec<MembershipRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT m.member_id, g.party_id AS group_id, g.value AS external_id \
         FROM binding_membership m \
         JOIN binding_identifier g ON g.party_id = m.group_id AND g.retired_at IS NULL \
                                  AND g.selector = 'group_external_id' \
         WHERE m.member_id = ANY($1) AND m.retired_at IS NULL AND g.issuer = $2 \
           AND g.lifecycle_state IN ('532', '553') \
         ORDER BY m.member_id, g.party_id, g.value",
    )
    .bind(members)
    .bind(issuer)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(MembershipRow {
            member: row.try_get("member_id")?,
            group: row.try_get("group_id")?,
            external_id: row.try_get("external_id")?,
        });
    }
    Ok(out)
}

/// One live representation relationship: the representative, the subject and
/// the mapped `subject_relationship` code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentationRow {
    /// The representative `PERSON`'s container.
    pub representative_id: Uuid,
    /// The subject `PERSON`'s container.
    pub subject_id: Uuid,
    /// The openEHR `subject_relationship` code, when the type maps to one.
    pub relationship: Option<String>,
}

/// The representation relationships of `representatives` valid on `today`
/// (`YYYY-MM-DD`).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn representations(
    conn: &mut PgConnection,
    representatives: &[Uuid],
    today: &str,
) -> Result<Vec<RepresentationRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT representative_id, subject_id, relationship FROM binding_representation \
         WHERE representative_id = ANY($1) AND retired_at IS NULL AND validity @> $2::date \
         ORDER BY representative_id, subject_id, relationship",
    )
    .bind(representatives)
    .bind(today)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(RepresentationRow {
            representative_id: row.try_get("representative_id")?,
            subject_id: row.try_get("subject_id")?,
            relationship: row.try_get("relationship")?,
        });
    }
    Ok(out)
}

/// Deletes every side-table row naming any of `vo_ids`, on the erasure
/// transaction `tx`: an erased party leaves no key, tombstone, membership or
/// representation behind.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn erase(tx: &mut PgConnection, vo_ids: &[VoId]) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM binding_identifier WHERE party_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM binding_membership WHERE group_id = ANY($1) OR member_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM binding_representation \
         WHERE representative_id = ANY($1) OR subject_id = ANY($1)",
    )
    .bind(vo_ids)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
