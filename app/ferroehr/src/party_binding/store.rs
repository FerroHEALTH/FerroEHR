// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the binding side table.
//!
//! `binding_identifier`, `binding_membership` and `binding_representation`
//! hold the rows each party head contributes. A commit derives them from the
//! body it holds (`party_binding::derive`) and writes only the rows that changed
//! (`party_binding::rows`), in its own transaction; [`refill`], an archive load and
//! an import derive them with the same function from the stored heads; the
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

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored canonical party bodies \
              a refill or a load derives the side table from"
)]

use std::collections::BTreeMap;

use serde_json::Value;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::error::ServiceError;
use crate::ids::VoId;
use crate::party_binding::BindingMapping;
use crate::party_binding::derive::{Facts, container};
use crate::party_binding::grants;
use crate::party_binding::key_space;
use crate::party_binding::rows::{
    self, IdentifierRow, Professional, RepresentationRow, write_identifiers,
};
use crate::storage::error::StorageError;
use crate::storage::party_reverse;
use crate::versioning::change::Committed;
use crate::versioning::{Kind, SigningCtx};

mod conflict;
pub mod held;
mod standalone;

use conflict::selector_in_use;

/// The `version_lifecycle_state` code of a logically deleted head.
const DELETED: &str = "523";
/// The parties one refill page derives.
const PAGE: i64 = 500;

/// The trunk head of a party as the side table reads it.
#[derive(Debug)]
struct Head {
    /// The party's container.
    vo_id: Uuid,
    /// The head's `version_lifecycle_state` code.
    lifecycle_state: String,
    /// What the head's body holds; `None` for a head without one.
    facts: Option<Facts>,
    /// What the last version with a body holds, with its lifecycle state, for
    /// a deleted head: the key its tombstone keeps.
    last: Option<(String, Facts)>,
}

impl Head {
    /// The head as the grant rows read it.
    fn grant_head(&self) -> grants::store::GrantHead<'_> {
        grants::store::GrantHead {
            vo_id: self.vo_id,
            lifecycle_state: &self.lifecycle_state,
            facts: self.facts.as_ref(),
        }
    }

    /// What a live head of the template `template` holds; `None` for a
    /// deleted head, one without a body and one of another template.
    fn live_of(&self, template: &str) -> Option<&Facts> {
        if self.lifecycle_state == DELETED {
            return None;
        }
        self.facts
            .as_ref()
            .filter(|facts| facts.template.as_deref() == Some(template))
    }
}

/// Judges the party commit `committed` against the protection rules
/// ([`crate::party_binding::protect`]), then brings its reverse-relationship
/// index and side-table rows to its new head, on its commit transaction `tx`.
/// A `PARTY_RELATIONSHIP` of its own holds the one representation row it is,
/// when it carries the representation code ([`standalone::reindex`]).
///
/// `facts` is what the committed body holds, `None` for a logical delete: the
/// hooks read it and never the stored body (#5220).
///
/// # Errors
/// The protection refusals, the `409` of [`selector_in_use`], and a storage
/// failure.
pub(crate) async fn reindex_committed(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    committed: &Committed,
    facts: Option<&Facts>,
) -> Result<(), ServiceError> {
    grants::caller::check_content(facts)?;
    grants::caller::subject::check(facts, &ctx.grants.namespace(), ctx.privacy)?;
    if !ctx.binding.is_empty() {
        key_space::gate_shared(tx).await?;
    }
    if committed.kind == Kind::PartyRelationship {
        return standalone::committed(tx, ctx, committed, facts).await;
    }
    if crate::versioning::committer::person_commits() {
        // NOTE: RM common master04 §Audit Details, PARTY_SELF names the record
        // subject, and a demographic version has none (#5005).
        return Err(ServiceError::not_permitted(
            "a patient-context credential commits no demographic version: a demographic \
             version has no record subject for PARTY_SELF to name",
        ));
    }
    crate::party_binding::protect::check(tx, ctx.binding, committed, facts).await?;
    if committed.kind == Kind::Role
        && let Some(facts) = facts
    {
        grants::check_role(tx, committed.vo_id.0, facts, ctx.grants).await?;
    }
    let (lifecycle_state, trunk_head) = head_of(tx, committed.vo_id).await?;
    let head = if trunk_head == Some(committed.sys_version) {
        Head {
            vo_id: committed.vo_id.0,
            lifecycle_state,
            facts: facts.cloned(),
            last: None,
        }
    } else {
        let mut heads = stored_heads(
            tx,
            ctx.binding,
            committed.kind,
            Some(&[committed.vo_id.0]),
            None,
        )
        .await?;
        match heads.pop() {
            Some(head) => head,
            None => return Ok(()),
        }
    };
    let head = with_last(tx, ctx.binding, committed.kind, head).await?;
    let targets = head
        .facts
        .as_ref()
        .map_or(&[][..], |f| f.targets.as_slice());
    party_reverse::replace(tx, committed.vo_id, targets, committed.time_committed).await?;
    let _changed = grants::caller::check_care(tx, ctx.binding, committed, facts).await?;
    reindex_heads(tx, ctx.binding, committed.kind, vec![head])
        .await
        .map_err(selector_in_use)
}

/// Rebuilds the reverse-relationship index of the party `vo_id` of `kind` from
/// its stored versions and brings its side-table rows to its stored head, on
/// `tx`: what an import or an archive load writes after the versions it lands.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn reindex_stored(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    vo_id: VoId,
) -> Result<(), StorageError> {
    if !mapping.is_empty() {
        key_space::gate_shared(tx).await?;
    }
    party_reverse::rebuild(tx, vo_id).await?;
    let mut heads = stored_heads(tx, mapping, kind, Some(&[vo_id.0]), None).await?;
    let Some(head) = heads.pop() else {
        return Ok(());
    };
    if kind == Kind::PartyRelationship {
        grants::store::write(tx, kind, head.grant_head()).await?;
        let facts = head.facts.filter(|_| head.lifecycle_state != DELETED);
        return standalone::reindex(tx, mapping, vo_id.0, facts.as_ref()).await;
    }
    let head = with_last(tx, mapping, kind, head).await?;
    reindex_heads(tx, mapping, kind, vec![head]).await
}

/// What an import does after it lands the versions of `vo_id` of `kind`, on
/// `tx`: marks a demographic object one of whose versions is a policy party
/// ([`crate::party_binding::policy_mark::mark_stored`]), and brings a
/// demographic object's reverse-relationship index and a party's side-table
/// rows to its stored head ([`reindex_stored`]).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn reindex_imported(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    vo_id: VoId,
) -> Result<(), StorageError> {
    if kind.is_demographic() {
        crate::party_binding::policy_mark::mark_stored(tx, Some(vo_id)).await?;
    }
    if kind.is_demographic() {
        reindex_stored(tx, mapping, kind, vo_id).await?;
    }
    Ok(())
}

/// Rebuilds the reverse-relationship index and the side-table rows of the
/// party `vo_id`, whose stored kind is `kind`, loaded from an archive on `tx`.
///
/// A `ROLE` loaded after its `PERSON` rebuilds that `PERSON`'s rows, so the
/// rows do not depend on the order the archive holds the parties in. A party
/// with a policy version is marked ([`crate::party_binding::policy_mark::mark_stored`]).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn reindex_loaded(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: &str,
    vo_id: VoId,
) -> Result<(), StorageError> {
    crate::party_binding::policy_mark::mark_stored(tx, Some(vo_id)).await?;
    match Kind::from_type(kind) {
        Some(kind) => reindex_stored(tx, mapping, kind, vo_id).await,
        None => party_reverse::rebuild(tx, vo_id).await,
    }
}

/// Writes the grant rows and the side-table rows of `heads`, each of
/// `kind`.
async fn reindex_heads(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    heads: Vec<Head>,
) -> Result<(), StorageError> {
    for head in &heads {
        grants::store::write(tx, kind, head.grant_head()).await?;
    }
    if mapping.is_empty() {
        return Ok(());
    }
    match kind {
        Kind::Person => write_persons(tx, mapping, &heads, Scope::Heads { tombstones: true }).await,
        Kind::Role => {
            let mut performers = Vec::new();
            for head in &heads {
                performers.extend(performers_of(tx, head.vo_id).await?);
            }
            performers.sort_unstable();
            performers.dedup();
            if performers.is_empty() {
                return Ok(());
            }
            let persons = stored_heads(tx, mapping, Kind::Person, Some(&performers), None).await?;
            write_persons(tx, mapping, &persons, Scope::Heads { tombstones: false }).await
        }
        Kind::Group => write_groups(tx, mapping, &heads).await,
        _ => Ok(()),
    }
}

/// Rebuilds every row of the side table from the party versions alone, on
/// `tx`.
///
/// Every row goes, and the rows of every party head come back, the
/// tombstones of the deleted `PERSON`s included, as the commits wrote them.
/// Every party with a policy version gets the mark it lacks
/// (`policy_mark::mark_stored`); no mark is cleared.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn refill(tx: &mut PgConnection, mapping: &BindingMapping) -> Result<(), StorageError> {
    crate::party_binding::live::note_change();
    crate::party_binding::policy_mark::mark_stored(tx, None).await?;
    for statement in [
        "DELETE FROM binding_identifier",
        "DELETE FROM binding_membership",
        "DELETE FROM binding_representation",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    refill_grants(tx).await?;
    standalone::refill(tx).await?;
    for kind in [Kind::Person, Kind::Group] {
        let mut after = None;
        loop {
            let page = stored_heads(tx, mapping, kind, None, after).await?;
            let Some(last) = page.last() else {
                break;
            };
            after = Some(last.vo_id);
            let mut heads = Vec::with_capacity(page.len());
            for head in page {
                heads.push(with_last(tx, mapping, kind, head).await?);
            }
            match kind {
                Kind::Person => {
                    write_persons(tx, mapping, &heads, Scope::Heads { tombstones: true }).await?;
                }
                _ => write_groups(tx, mapping, &heads).await?,
            }
        }
    }
    Ok(())
}

/// Rebuilds every grant row from the `ACTOR` and `ROLE` versions alone, on
/// `tx`, and moves the grant revision.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, the unique violation of
/// two `AGENT`s naming one client included.
pub async fn refill_grants(tx: &mut PgConnection) -> Result<(), StorageError> {
    grants::store::clear(tx).await?;
    for kind in grants::store::HOLDERS {
        let mut after = None;
        loop {
            let page = stored_heads(tx, &BindingMapping::default(), kind, None, after).await?;
            let Some(last) = page.last() else {
                break;
            };
            after = Some(last.vo_id);
            for head in &page {
                grants::store::write(tx, kind, head.grant_head()).await?;
            }
        }
    }
    grants::store::refilled(tx).await
}

/// Which rows [`write_persons`] writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope<'a> {
    /// Every row of the heads; with `tombstones`, the key of each deleted one
    /// is left as a tombstone.
    Heads {
        /// Whether a deleted head leaves its keys as tombstones.
        tombstones: bool,
    },
    /// The key rows and tombstones under one issuer alone: the online fill of
    /// that issuer's key space ([`crate::party_binding::key_space`]).
    KeySpace(&'a str),
}

impl Scope<'_> {
    /// Whether the identifier of `selector` under `issuer` is in scope.
    fn holds(self, selector: &str, issuer: &str) -> bool {
        match self {
            Self::Heads { .. } => true,
            Self::KeySpace(only) => selector == "key" && issuer == only,
        }
    }
}

/// Writes the key, `userName`, `externalId`, representation and membership
/// rows of the `PERSON` heads `heads` that `scope` holds, with the tombstones
/// it asks for.
///
/// A key row is written for a `PERSON` that performs a professional `ROLE`
/// (RM demographic `role.adoc`: a `ROLE` listed in `roles`, whose
/// `performer` names the `PERSON` and whose head is `complete` or
/// `incomplete`) or is the source of a representation relationship, inline
/// or of its own.
async fn write_persons(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    heads: &[Head],
    scope: Scope<'_>,
) -> Result<(), StorageError> {
    let parties: Vec<Uuid> = heads.iter().map(|h| h.vo_id).collect();
    let mut identifiers = Vec::new();
    let mut representations = Vec::new();
    let mut memberships = Vec::new();
    let mut listed = Vec::new();
    let standalone = rows::representatives(tx, &parties).await?;
    for head in heads {
        if mapping.membership.is_some()
            && head.lifecycle_state != DELETED
            && let Some(facts) = &head.facts
        {
            memberships.extend(facts.groups.iter().map(|group| (*group, head.vo_id)));
        }
        let Some(facts) = mapping
            .persons
            .keys()
            .find_map(|template| head.live_of(template))
        else {
            continue;
        };
        representations.extend(facts.representations.iter().map(|r| RepresentationRow {
            representative: head.vo_id,
            subject: r.subject,
            code: r.code.clone(),
            validity: r.validity.clone(),
        }));
        for (selector, identifier) in &facts.identifiers {
            if !scope.holds(selector, &identifier.issuer) {
                continue;
            }
            identifiers.push(IdentifierRow {
                party: head.vo_id,
                selector,
                issuer: identifier.issuer.clone(),
                identifier_type: identifier.id_type.clone(),
                value: if *selector == "key" {
                    identifier.canonical()
                } else {
                    identifier.id.clone()
                },
                lifecycle_state: head.lifecycle_state.clone(),
                representative: !facts.representations.is_empty()
                    || standalone.contains(&head.vo_id),
                portal: *selector == "key" && mapping.is_portal_issuer(&identifier.issuer),
            });
        }
        listed.extend(facts.roles.iter().map(|role| (*role, head.vo_id)));
    }
    let professional = professional(tx, mapping, &listed).await?;
    let (selectors, issuer, tombstones): (&[&str], _, _) = match scope {
        Scope::Heads { tombstones } => (&["key", "user_name", "external_id"], None, tombstones),
        Scope::KeySpace(issuer) => (&["key"], Some(issuer), true),
    };
    if issuer.is_none() {
        rows::write_representations(tx, &parties, &representations).await?;
    }
    write_identifiers(
        tx,
        &parties,
        (selectors, issuer),
        &identifiers,
        &professional,
    )
    .await?;
    if issuer.is_none() {
        rows::write_memberships(tx, &parties, &memberships).await?;
    }
    if tombstones {
        let mut rows = tombstones_of(mapping, heads);
        rows.retain(|row| scope.holds(row.selector, &row.issuer));
        rows::insert_tombstones(tx, &rows).await?;
    }
    rows::revive_keys(tx, &parties).await
}

/// Writes the key rows and tombstones under `issuer` of one page of the
/// `PERSON` heads after `after` in container order, on `tx`, returning the
/// last container of the page, `None` past the last page: one step of the
/// online fill of `issuer`'s key space.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, the unique violation of
/// two `PERSON`s carrying one key under `issuer` included.
pub(crate) async fn fill_key_space(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    issuer: &str,
    after: Option<Uuid>,
) -> Result<Option<Uuid>, StorageError> {
    let page = stored_heads(tx, mapping, Kind::Person, None, after).await?;
    let Some(last) = page.last().map(|head| head.vo_id) else {
        return Ok(None);
    };
    let mut heads = Vec::with_capacity(page.len());
    for head in page {
        heads.push(with_last(tx, mapping, Kind::Person, head).await?);
    }
    write_persons(tx, mapping, &heads, Scope::KeySpace(issuer)).await?;
    Ok(Some(last))
}

/// The tombstone rows of the deleted heads among `heads`: the binding keys
/// each carried in its last version with a body, of a mapped template.
fn tombstones_of(mapping: &BindingMapping, heads: &[Head]) -> Vec<IdentifierRow> {
    let mut out = Vec::new();
    for head in heads.iter().filter(|h| h.lifecycle_state == DELETED) {
        let Some((lifecycle_state, facts)) = &head.last else {
            continue;
        };
        if !facts
            .template
            .as_ref()
            .is_some_and(|t| mapping.persons.contains_key(t))
        {
            continue;
        }
        for (_, identifier) in facts.identifiers.iter().filter(|(s, _)| *s == "key") {
            out.push(IdentifierRow {
                party: head.vo_id,
                selector: "key",
                issuer: identifier.issuer.clone(),
                identifier_type: identifier.id_type.clone(),
                value: identifier.canonical(),
                lifecycle_state: lifecycle_state.clone(),
                representative: false,
                portal: false,
            });
        }
    }
    out
}

/// The validities of the professional `ROLE`s among `listed`, each a `ROLE`
/// container and the `PERSON` that lists it: a `ROLE` of the professional
/// template whose head is `complete` or `incomplete` and whose `performer`
/// names that `PERSON`.
async fn professional(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    listed: &[(Uuid, Uuid)],
) -> Result<Vec<Professional>, StorageError> {
    let Some(template) = mapping.professional_role.as_deref() else {
        return Ok(Vec::new());
    };
    if listed.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<Uuid> = listed.iter().map(|(role, _)| *role).collect();
    ids.sort_unstable();
    ids.dedup();
    let roles: BTreeMap<Uuid, Facts> = stored_heads(tx, mapping, Kind::Role, Some(&ids), None)
        .await?
        .into_iter()
        .filter(|head| matches!(head.lifecycle_state.as_str(), "532" | "553"))
        .filter_map(|head| Some((head.vo_id, head.facts?)))
        .filter(|(_, facts)| facts.template.as_deref() == Some(template))
        .collect();
    Ok(listed
        .iter()
        .filter_map(|(role, person)| {
            let facts = roles.get(role)?;
            let performer = facts.performer.as_deref().and_then(container)?;
            (performer == *person).then(|| Professional {
                party: *person,
                validity: facts.validity.clone(),
            })
        })
        .collect())
}

/// Writes the `externalId` and `displayName` rows of the `GROUP` heads
/// `heads`; a `GROUP`'s members hold their memberships themselves.
async fn write_groups(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    heads: &[Head],
) -> Result<(), StorageError> {
    let Some(group) = &mapping.group else {
        return Ok(());
    };
    let groups: Vec<Uuid> = heads.iter().map(|h| h.vo_id).collect();
    let mut identifiers = Vec::new();
    for head in heads {
        let Some(facts) = head.live_of(&group.template) else {
            continue;
        };
        let row = |selector, issuer: &str, identifier_type: &str, value: &str| IdentifierRow {
            party: head.vo_id,
            selector,
            issuer: issuer.to_owned(),
            identifier_type: identifier_type.to_owned(),
            value: value.to_owned(),
            lifecycle_state: head.lifecycle_state.clone(),
            representative: false,
            portal: false,
        };
        for id in &facts.group_external_ids {
            identifiers.push(row("group_external_id", &id.issuer, &id.id_type, &id.id));
        }
        for name in &facts.display_names {
            identifiers.push(row("group_display_name", "", "", name));
        }
    }
    write_identifiers(
        tx,
        &groups,
        (&["group_external_id", "group_display_name"], None),
        &identifiers,
        &[],
    )
    .await
}

/// The head's lifecycle state and trunk head of the party `vo_id`.
async fn head_of(
    tx: &mut PgConnection,
    vo_id: VoId,
) -> Result<(String, Option<i32>), StorageError> {
    let row =
        sqlx::query("SELECT lifecycle_state, trunk_head_sys_version FROM vo_head WHERE vo_id = $1")
            .bind(vo_id)
            .fetch_one(&mut *tx)
            .await?;
    Ok((
        row.try_get("lifecycle_state")?,
        row.try_get("trunk_head_sys_version")?,
    ))
}

/// The trunk heads of the parties of `kind` among `ids` (every party of
/// `kind` when `None`), after `after` in container order, one page at most,
/// each with what its body holds.
async fn stored_heads(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    ids: Option<&[Uuid]>,
    after: Option<Uuid>,
) -> Result<Vec<Head>, StorageError> {
    let rows = sqlx::query(
        "SELECT h.vo_id, h.lifecycle_state, v.body FROM vo_head h \
         LEFT JOIN version v ON v.tier = h.tier AND v.vo_id = h.vo_id \
                            AND v.sys_version = h.trunk_head_sys_version \
         WHERE h.kind = $1 AND ($2::uuid[] IS NULL OR h.vo_id = ANY($2::uuid[])) \
           AND ($3::uuid IS NULL OR h.vo_id > $3) \
         ORDER BY h.vo_id LIMIT $4",
    )
    .bind(kind.as_str())
    .bind(ids)
    .bind(after)
    .bind(PAGE)
    .fetch_all(&mut *tx)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let body: Option<String> = row.try_get("body")?;
        out.push(Head {
            vo_id: row.try_get("vo_id")?,
            lifecycle_state: row.try_get("lifecycle_state")?,
            facts: body
                .as_deref()
                .map(|body| parsed(kind, body, mapping))
                .transpose()?,
            last: None,
        });
    }
    Ok(out)
}

/// `head` with what its last version with a body holds, when it is a deleted
/// `PERSON`: the key its tombstone keeps.
async fn with_last(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    mut head: Head,
) -> Result<Head, StorageError> {
    if kind != Kind::Person || head.lifecycle_state != DELETED {
        return Ok(head);
    }
    let row = sqlx::query(
        "SELECT v.body, v.lifecycle_state FROM version v \
         WHERE v.vo_id = $1 AND v.body IS NOT NULL ORDER BY v.sys_version DESC LIMIT 1",
    )
    .bind(head.vo_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = row {
        let body: String = row.try_get("body")?;
        head.last = Some((
            row.try_get("lifecycle_state")?,
            parsed(kind, &body, mapping)?,
        ));
    }
    Ok(head)
}

/// The `PERSON` containers every version of the `ROLE` `role` names as its
/// performer, so a `ROLE` whose performer changed rebuilds both.
async fn performers_of(tx: &mut PgConnection, role: Uuid) -> Result<Vec<Uuid>, StorageError> {
    let bodies: Vec<String> =
        sqlx::query_scalar("SELECT body FROM version WHERE vo_id = $1 AND body IS NOT NULL")
            .bind(role)
            .fetch_all(&mut *tx)
            .await?;
    let mut out = Vec::new();
    for body in bodies {
        let facts = parsed(Kind::Role, &body, &BindingMapping::default())?;
        out.extend(facts.performer.as_deref().and_then(container));
    }
    Ok(out)
}

/// What the stored body `body` of a party of `kind` holds.
///
/// # Errors
/// [`StorageError::BodyDecode`] when the stored body is no JSON, which a
/// committed version never is.
pub(crate) fn parsed(
    kind: Kind,
    body: &str,
    mapping: &BindingMapping,
) -> Result<Facts, StorageError> {
    let value: Value = serde_json::from_str(body).map_err(StorageError::BodyDecode)?;
    Ok(Facts::of(kind, &value, mapping))
}

/// Deletes every side-table row naming any of `vo_ids`, on the erasure
/// transaction `tx`: an erased party leaves no key, tombstone, membership,
/// representation or contact-erasure record behind.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn erase(tx: &mut PgConnection, vo_ids: &[VoId]) -> Result<(), StorageError> {
    crate::party_binding::live::note_change();
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
    sqlx::query("DELETE FROM scim_contact_erasure WHERE party_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?;
    let ids: Vec<Uuid> = vo_ids.iter().map(|id| id.0).collect();
    grants::store::erase(tx, &ids).await
}
