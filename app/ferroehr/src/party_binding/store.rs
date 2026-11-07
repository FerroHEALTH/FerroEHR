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

use crate::error::ServiceError;
use crate::error::refusal::{Clash, Refusal};
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

/// The `409` of a violation of one of the selector unique indexes: the commit
/// gives a binding key, a `userName`, an `externalId` or a group
/// `displayName` a second live party. Every other failure is classified as
/// storage.
pub(crate) fn selector_in_use(e: StorageError) -> ServiceError {
    let StorageError::Database(source) = e else {
        return ServiceError::Storage(e);
    };
    let selector = source
        .as_database_error()
        .filter(|db| db.is_unique_violation())
        .and_then(|db| db.constraint())
        .and_then(|constraint| match constraint {
            "uq_binding_identifier_key" => Some("binding key"),
            "uq_binding_identifier_user_name" => Some("userName"),
            "uq_binding_identifier_external_id" => Some("externalId"),
            "uq_binding_identifier_display_name" => Some("displayName"),
            _ => None,
        });
    match selector {
        Some(selector) => ServiceError::Conflict(
            Refusal::new(
                Clash::State,
                format!("the {selector} this party carries is already held by another party"),
            )
            .with_source(source),
        ),
        None => ServiceError::Storage(StorageError::Database(source)),
    }
}

/// Judges the party commit `committed` against the protection rules
/// ([`crate::party_binding::protect`]), then rebuilds its
/// reverse-relationship index and side-table rows, on its commit transaction
/// `tx`.
///
/// # Errors
/// The protection refusals, the `409` of [`selector_in_use`], and a storage
/// failure.
pub(crate) async fn reindex_committed(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    committed: &Committed,
) -> Result<(), ServiceError> {
    if crate::versioning::committer::person_commits() {
        // NOTE: RM common master04 §Audit Details, PARTY_SELF names the record
        // subject, and a demographic version has none (#5005).
        return Err(ServiceError::not_permitted(
            "a patient-context credential commits no demographic version: a demographic \
             version has no record subject for PARTY_SELF to name",
        ));
    }
    crate::party_binding::protect::check(tx, ctx.binding, committed.kind, committed.vo_id).await?;
    party_reverse::reindex(tx, committed.vo_id).await?;
    reindex(tx, ctx.binding, committed.kind, committed.vo_id)
        .await
        .map_err(selector_in_use)
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

/// The statement `sql` with the seven parameters the selector derivations
/// share: the parties `$1`, the kind `$2`, the canonical template `$3`, and
/// the selector's path `$4`, variables `$5`, type `$6` and issuers `$7`
/// (NULL for every issuer).
fn selected<'q>(
    sql: &'static str,
    ids: Option<&'q [Uuid]>,
    kind: Kind,
    template: &'q str,
    selector: &'q IdentifierSelector,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    sqlx::query(sql)
        .bind(ids)
        .bind(kind.as_str())
        .bind(template)
        .bind(&selector.json_path)
        .bind(&selector.variables)
        .bind(&selector.identifier_type)
        .bind(selector.issuer_filter())
}

/// The condition that `id` is a complete identifier of the selector's type and
/// issuers, with the parameters [`selected`] binds.
macro_rules! identifier_of_selector {
    () => {
        "id ->> 'type' = $6 AND id ->> 'issuer' IS NOT NULL AND id ->> 'id' IS NOT NULL \
         AND ($7::text[] IS NULL OR id ->> 'issuer' = ANY($7::text[]))"
    };
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

/// One selector identifier of a live `PERSON` head, as the unique indexes
/// compare it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HeldSelector {
    /// `key`, `user_name` or `external_id`.
    pub selector: &'static str,
    /// The `DV_IDENTIFIER.issuer`; empty for a `userName`, which is unique
    /// across issuers.
    pub issuer: String,
    /// The compared value: the canonical key, the folded `userName`, or the
    /// `externalId`.
    pub value: String,
    /// The `PERSON`'s container.
    pub party_id: Uuid,
}

/// Every selector identifier of every live `PERSON` head of a mapped
/// template, whether or not it binds, on `tx`: what [`refill`] would hold
/// each value to one party by.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn held_selectors(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
) -> Result<Vec<HeldSelector>, StorageError> {
    let mut out = Vec::new();
    for (template, selectors) in &mapping.persons {
        for (selector, chosen) in selectors.each() {
            let rows = selected(
                concat!(
                    heads!(),
                    "SELECT DISTINCT p.vo_id, \
                            CASE WHEN $8 = 'user_name' THEN '' ELSE id ->> 'issuer' END AS issuer, \
                            CASE WHEN $8 = 'user_name' THEN party.binding_fold(id ->> 'id') \
                                 WHEN $8 = 'key' THEN ",
                    canonical_value!("id"),
                    " ELSE id ->> 'id' END AS value \
                 FROM p CROSS JOIN LATERAL jsonb_path_query(p.body, $4::jsonpath, $5::jsonb) AS id \
                 WHERE ",
                    identifier_of_selector!()
                ),
                None,
                Kind::Person,
                template,
                chosen,
            )
            .bind(selector)
            .fetch_all(&mut *tx)
            .await?;
            for row in rows {
                out.push(HeldSelector {
                    selector,
                    issuer: row.try_get("issuer")?,
                    value: row.try_get("value")?,
                    party_id: row.try_get("vo_id")?,
                });
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
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
             WHERE party_id = ANY($1) AND selector IN ('key', 'user_name', 'external_id') \
               AND retired_at IS NULL",
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
    for (template, selectors) in &mapping.persons {
        insert_representations(tx, mapping, ids, template).await?;
        insert_keys(tx, mapping, ids, template, &selectors.key).await?;
        for (selector, chosen) in [
            ("user_name", selectors.user_name.as_ref()),
            ("external_id", selectors.external_id.as_ref()),
        ] {
            if let Some(chosen) = chosen {
                insert_selector(tx, ids, Kind::Person, template, selector, chosen).await?;
            }
        }
        if tombstones {
            insert_tombstones(tx, ids, template, &selectors.key).await?;
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
    selected(
        concat!(
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
             WHERE $8::text IS NOT NULL AND rh.kind = 'ROLE' \
               AND rh.lifecycle_state IN ('532', '553') \
               AND rv.body IS NOT NULL \
               AND lower(btrim(rv.body::jsonb -> 'archetype_details' -> 'template_id' ->> 'value')) \
                   = $8 \
               AND ",
        container!("rv.body::jsonb -> 'performer' -> 'id' ->> 'value'"),
        " = p.vo_id \
             GROUP BY p.vo_id \
         ) \
         INSERT INTO binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state, \
              professional, representative) \
         SELECT DISTINCT ON (p.vo_id, id ->> 'issuer', ",
        canonical_value!("id"),
        ") p.vo_id, 'key', id ->> 'issuer', id ->> 'type', c.value, c.value, \
                p.lifecycle_state, coalesce(pr.dates, '{}'), rep.present \
         FROM p \
         CROSS JOIN LATERAL jsonb_path_query(p.body, $4::jsonpath, $5::jsonb) AS id \
         CROSS JOIN LATERAL (SELECT ",
        canonical_value!("id"),
        " AS value) AS c \
         LEFT JOIN professional pr ON pr.vo_id = p.vo_id \
         CROSS JOIN LATERAL (SELECT EXISTS ( \
             SELECT 1 FROM binding_representation b \
             WHERE b.representative_id = p.vo_id AND b.retired_at IS NULL) AS present) AS rep \
         WHERE ",
        identifier_of_selector!(),
        " AND (NOT isempty(coalesce(pr.dates, '{}')) OR rep.present)"
        ),
        ids,
        Kind::Person,
        template,
        key,
    )
    .bind(mapping.professional_role.as_deref())
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Inserts the live `selector` rows (`user_name`, `external_id` or
/// `group_external_id`) of the parties `ids` of `kind` and `template`: one
/// per identifier `chosen` selects, of every live head, whether or not it
/// binds, so the unique indexes hold each value to one party. A `userName` is
/// compared by its fold.
async fn insert_selector(
    tx: &mut PgConnection,
    ids: Option<&[Uuid]>,
    kind: Kind,
    template: &str,
    selector: &str,
    chosen: &IdentifierSelector,
) -> Result<(), StorageError> {
    selected(
        concat!(
            heads!(),
            "INSERT INTO binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state) \
         SELECT DISTINCT ON (p.vo_id, id ->> 'issuer', id ->> 'id') \
                p.vo_id, $8, id ->> 'issuer', id ->> 'type', id ->> 'id', \
                CASE WHEN $8 = 'user_name' THEN party.binding_fold(id ->> 'id') \
                     ELSE id ->> 'id' END, \
                p.lifecycle_state \
         FROM p CROSS JOIN LATERAL jsonb_path_query(p.body, $4::jsonpath, $5::jsonb) AS id \
         WHERE ",
            identifier_of_selector!()
        ),
        ids,
        kind,
        template,
        chosen,
    )
    .bind(selector)
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
    selected(
        concat!(
            "INSERT INTO binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state, \
              retired_at, deprovisioned_at) \
         SELECT DISTINCT h.vo_id, 'key', id ->> 'issuer', id ->> 'type', ",
            canonical_value!("id"),
            ", ",
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
           AND ",
            identifier_of_selector!(),
            " AND NOT EXISTS ( \
               SELECT 1 FROM binding_identifier t \
               WHERE t.party_id = h.vo_id AND t.deprovisioned_at IS NOT NULL)"
        ),
        ids,
        Kind::Person,
        template,
        key,
    )
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
    let Some(group) = &mapping.group else {
        return Ok(());
    };
    let template = &group.template;
    if let Some(ids) = ids {
        sqlx::query(
            "UPDATE binding_identifier SET retired_at = now() \
             WHERE party_id = ANY($1) AND selector IN ('group_external_id', 'group_display_name') \
               AND retired_at IS NULL",
        )
        .bind(ids)
        .execute(&mut *tx)
        .await?;
    }
    insert_selector(
        tx,
        ids,
        Kind::Group,
        template,
        "group_external_id",
        &group.external_id,
    )
    .await?;
    if let Some(display_name) = &group.display_name {
        sqlx::query(concat!(
            heads!(),
            "INSERT INTO binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state) \
         SELECT DISTINCT ON (p.vo_id, party.binding_fold(name ->> 'value')) \
                p.vo_id, 'group_display_name', '', '', name ->> 'value', \
                party.binding_fold(name ->> 'value'), p.lifecycle_state \
         FROM p CROSS JOIN LATERAL jsonb_path_query(p.body, $4::jsonpath, $5::jsonb) AS name \
         WHERE name ->> 'value' IS NOT NULL"
        ))
        .bind(ids)
        .bind(Kind::Group.as_str())
        .bind(template)
        .bind(&display_name.json_path)
        .bind(&display_name.variables)
        .execute(&mut *tx)
        .await?;
    }
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
