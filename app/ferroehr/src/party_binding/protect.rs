// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The protection of the parties the login binding and SCIM provisioning
//! rely on, judged in each party commit's own transaction.
//!
//! A protected party is a `PERSON` of a mapped template carrying a selector
//! identifier (the binding key, a `userName` or an `externalId`) or the
//! source of a representation relationship, a `ROLE` of the professional
//! role template, and a `GROUP` of the group template with its memberships.
//! Only an administrator writes one through the demographic API, so nobody
//! can plant a key-bearing `PERSON`, grant themselves the professional
//! `ROLE` or point a colleague's `PERSON` at their own login. A SCIM client
//! writes the ones it owns: a `PERSON` whose binding keys its
//! `binding_issuer` issues, a professional `ROLE` that `PERSON` performs, and
//! a `GROUP` it created (RFC 7644 §3: a client manages its own resources). A present
//! binding key is immutable: RFC 7643 §2.2 `mutability: "immutable"`, "SHALL
//! NOT be updated", and only [`rekey`] changes one. An administrator's version
//! that leaves a protected identifier out is refused, so nothing is carried
//! forward in silence. No openEHR spec governs who may write a party: our own
//! design.

use std::collections::BTreeSet;
use std::future::Future;

use sqlx::{PgConnection, Row};
use tokio::task_local;
use uuid::Uuid;

use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::party_binding::BindingMapping;
use crate::party_binding::derive::Facts;
use crate::party_binding::store::parsed;
use crate::versioning::Kind;
use crate::versioning::change::Committed;

task_local! {
    /// The key change the running administrator re-key may make.
    static REKEY: Rekey;
    /// The `binding_issuer` of the SCIM client the running request is of.
    static PROVISIONER: String;
}

/// One key change an administrator re-key makes: on the `PERSON` `party`,
/// the key `from` (absent when it binds a key for the first time) becomes
/// `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rekey {
    /// The `PERSON`'s container.
    pub party: Uuid,
    /// The canonical key `(issuer, value)` the re-key retires.
    pub from: Option<(String, String)>,
    /// The canonical key `(issuer, value)` the re-key binds.
    pub to: (String, String),
}

/// Runs `fut` as the administrator re-key `change`: a commit inside it may
/// change that one `PERSON`'s binding key from `change.from` to `change.to`.
pub async fn rekey<F: Future>(change: Rekey, fut: F) -> F::Output {
    REKEY.scope(change, fut).await
}

/// Runs `fut` as a request of the SCIM client whose `binding_issuer` is
/// `binding_issuer`: a commit inside it may write the protected parties that
/// client owns.
pub async fn provisioning<F: Future>(binding_issuer: String, fut: F) -> F::Output {
    PROVISIONER.scope(binding_issuer, fut).await
}

/// One selector identifier of a body: which selector, its issuer and type,
/// and its canonical value.
type Held = (&'static str, String, String, String);

/// What a version of a party holds that the protection reads.
#[derive(Debug, Default)]
struct Protected {
    /// The selector identifiers of a `PERSON` of a mapped template.
    identifiers: BTreeSet<Held>,
    /// Whether the body is of a template or archetype the protection
    /// covers as a whole: the professional `ROLE`, the group, or a `PERSON`
    /// that is the source of a representation relationship.
    whole: bool,
}

impl Protected {
    /// Whether the version is protected.
    fn protected(&self) -> bool {
        self.whole || !self.identifiers.is_empty()
    }

    /// The canonical binding keys `(issuer, value)` of the version.
    fn keys(&self) -> BTreeSet<(String, String)> {
        self.identifiers
            .iter()
            .filter(|(selector, ..)| *selector == "key")
            .map(|(_, issuer, _, value)| (issuer.clone(), value.clone()))
            .collect()
    }
}

/// Judges the commit of the party `vo_id` of `kind`, whose new head the
/// commit transaction `tx` has just written, against the protection rules.
///
/// It reads the new head from `facts`, what the committed body holds (`None`
/// for a logical delete), and the version before it from storage: a
/// `PERSON`'s body, and only the template of a `ROLE` or a `GROUP`, so a
/// change to a large `GROUP` parses no body (#5220). It runs after the write
/// and before the side-table rows are rebuilt.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) when a caller who is neither an
/// administrator nor, inside [`provisioning`], the SCIM client that owns it
/// writes a protected party, or a write changes a present
/// binding key outside [`rekey`]; [`ServiceError::Unprocessable`] (`422`)
/// naming the identifier when an administrator's version leaves a protected
/// identifier out; a storage error when the versions cannot be read.
pub(crate) async fn check(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    committed: &Committed,
    facts: Option<&Facts>,
) -> Result<(), ServiceError> {
    let (kind, vo_id) = (committed.kind, committed.vo_id);
    if mapping.is_empty() || !matches!(kind, Kind::Person | Kind::Role | Kind::Group) {
        return Ok(());
    }
    let (current, previous) = versions(tx, vo_id).await?;
    let before = match previous {
        Some(sys_version) => read(tx, mapping, kind, vo_id, sys_version).await?,
        None => Protected::default(),
    };
    let after = match (current, facts) {
        (Some(_), Some(facts)) => protected(mapping, kind, facts),
        (Some(sys_version), None) => read(tx, mapping, kind, vo_id, sys_version).await?,
        (None, _) => Protected::default(),
    };
    if !(before.protected() || after.protected()) {
        return Ok(());
    }
    let owner = match PROVISIONER.try_with(Clone::clone) {
        Ok(issuer) => {
            let held = [current, previous];
            owned(
                tx,
                mapping,
                (kind, vo_id, held),
                (&[&before, &after], facts),
                &issuer,
            )
            .await?
        }
        Err(_) => false,
    };
    if !(owner || administrator(mapping)) {
        return Err(ServiceError::not_permitted(format!(
            "the {} {vo_id} is protected by the party binding: only an administrator, or the \
             SCIM client that owns it, writes it",
            kind.as_str()
        )));
    }
    if current.is_none() {
        return Ok(());
    }
    let (keys_before, keys_after) = (before.keys(), after.keys());
    if !keys_before.is_empty() && !keys_after.is_empty() && keys_before != keys_after {
        let permitted = REKEY
            .try_with(|change| {
                change.party == vo_id.0
                    && change
                        .from
                        .as_ref()
                        .is_none_or(|from| keys_before.contains(from))
                    && keys_after == BTreeSet::from([change.to.clone()])
            })
            .unwrap_or(false);
        if !permitted {
            return Err(ServiceError::not_permitted(format!(
                "the binding key of PERSON {vo_id} is immutable (RFC 7643 §2.2): only the \
                 administrator re-key changes it"
            )));
        }
    }
    let rekeyed = REKEY
        .try_with(|change| change.party == vo_id.0)
        .unwrap_or(false);
    let dropped: Vec<&Held> = before
        .identifiers
        .iter()
        .filter(|held| !(rekeyed && held.0 == "key"))
        .filter(|(selector, issuer, identifier_type, _)| {
            !after
                .identifiers
                .iter()
                .any(|(s, i, t, _)| (s, i, t) == (selector, issuer, identifier_type))
        })
        .collect();
    if let Some((selector, issuer, identifier_type, _)) = dropped.first() {
        return Err(ServiceError::content_invalid(
            Violation::new(format!(
                "this version of PERSON {vo_id} leaves out the protected {} identifier of \
                 issuer {issuer:?} and type {identifier_type:?}: a protected identifier is \
                 never dropped by a new version",
                label(selector)
            ))
            .with_path("/details"),
        ));
    }
    Ok(())
}

/// Whether the SCIM client of the binding issuer `issuer` owns the party
/// `vo_id` of `kind`, whose versions `held` (the new head and the one before
/// it) hold `versions`.
///
/// A `PERSON` is owned when it carries a binding key and each of its keys,
/// before and after the write, is of `issuer`. A professional `ROLE` is owned
/// when every `PERSON` it names as `performer` is. A `GROUP` is owned when
/// the client created it, and a version of any other `GROUP` is admitted
/// when all it does is drop memberships of `PERSON`s the client owns.
async fn owned(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    (kind, vo_id, held): (Kind, VoId, [Option<i32>; 2]),
    (versions, facts): (&[&Protected], Option<&Facts>),
    issuer: &str,
) -> Result<bool, ServiceError> {
    let keyed_by_client = |protected: &[&Protected]| {
        let keys: Vec<(String, String)> = protected.iter().flat_map(|p| p.keys()).collect();
        !keys.is_empty() && keys.iter().all(|(of, _)| of == issuer)
    };
    match kind {
        Kind::Person => Ok(keyed_by_client(versions)),
        Kind::Role => {
            let performers = performers(tx, vo_id, held, facts).await?;
            if performers.is_empty() {
                return Ok(false);
            }
            for named in performers {
                if !person_of(tx, mapping, &named, issuer).await? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Kind::Group => {
            if crate::party_binding::provisioned::owns_group(tx, issuer, vo_id).await? {
                return Ok(true);
            }
            let [Some(current), Some(previous)] = held else {
                return Ok(false);
            };
            let Some(removed) = removed_members(tx, vo_id, (current, previous)).await? else {
                return Ok(false);
            };
            for named in removed {
                if !person_of(tx, mapping, &named, issuer).await? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Whether the reference id `named` is a `PERSON` of this store whose last
/// body carries binding keys all of `issuer`.
async fn person_of(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    named: &str,
    issuer: &str,
) -> Result<bool, ServiceError> {
    // NOTE: a reference whose root is no UUID names no container of this store.
    let Ok(person) = VoId::from_uid_head(named) else {
        return Ok(false);
    };
    let Some(sys_version) = last_body(tx, person).await? else {
        return Ok(false);
    };
    let keys = match body(tx, person, sys_version).await? {
        Some(body) => parsed(Kind::Person, &body, mapping)?.keys(),
        None => BTreeSet::new(),
    };
    Ok(!keys.is_empty() && keys.iter().all(|(of, _)| of == issuer))
}

/// The targets of the inline relationships the version `current` of the
/// `GROUP` `vo_id` drops from `previous`, when that is all it changes;
/// `None` when it changes anything else.
///
/// A SCIM client deleting one of its Users removes the User's memberships
/// from every `GROUP` in the same CONTRIBUTION, an administrator's included.
async fn removed_members(
    tx: &mut PgConnection,
    vo_id: VoId,
    (current, previous): (i32, i32),
) -> Result<Option<Vec<String>>, ServiceError> {
    let (Some(after), Some(before)) = (
        body(tx, vo_id, current).await?,
        body(tx, vo_id, previous).await?,
    ) else {
        return Ok(None);
    };
    Ok(crate::party_binding::derive::removed_relationships(
        &before, &after,
    )?)
}

/// The `PARTY_REF.id` values the versions `held` of the `ROLE` `vo_id` name
/// as `performer`.
async fn performers(
    tx: &mut PgConnection,
    vo_id: VoId,
    held: [Option<i32>; 2],
    facts: Option<&Facts>,
) -> Result<BTreeSet<String>, ServiceError> {
    let [current, previous] = held;
    let mut out = BTreeSet::new();
    let mut read = |facts: &Facts| {
        // NOTE: RM demographic `role.adoc`: `performer` is 1..1, so a version without one names nobody.
        if let Some(performer) = &facts.performer {
            out.insert(performer.clone());
        }
    };
    match (current, facts) {
        (Some(_), Some(facts)) => read(facts),
        (Some(sys_version), None) => {
            if let Some(body) = body(tx, vo_id, sys_version).await? {
                read(&parsed(Kind::Role, &body, &BindingMapping::default())?);
            }
        }
        (None, _) => {}
    }
    if let Some(sys_version) = previous
        && let Some(body) = body(tx, vo_id, sys_version).await?
    {
        read(&parsed(Kind::Role, &body, &BindingMapping::default())?);
    }
    Ok(out)
}

/// The stored body of the version `sys_version` of `vo_id`, `None` when it
/// holds none.
async fn body(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
) -> Result<Option<String>, ServiceError> {
    Ok(
        sqlx::query_scalar(
            "SELECT v.body FROM version v WHERE v.vo_id = $1 AND v.sys_version = $2",
        )
        .bind(vo_id)
        .bind(sys_version)
        .fetch_optional(&mut *tx)
        .await?
        .flatten(),
    )
}

/// The `sys_version` of the last version of `vo_id` that holds a body.
async fn last_body(tx: &mut PgConnection, vo_id: VoId) -> Result<Option<i32>, ServiceError> {
    Ok(sqlx::query_scalar(
        "SELECT max(v.sys_version) FROM version v WHERE v.vo_id = $1 AND v.body IS NOT NULL",
    )
    .bind(vo_id)
    .fetch_one(&mut *tx)
    .await?)
}

/// The name a selector is reported under.
fn label(selector: &str) -> &'static str {
    match selector {
        "user_name" => "userName",
        "external_id" => "externalId",
        _ => "binding key",
    }
}

/// Whether the running request may write a protected party: a write with no
/// request committer is the server's own, and a request whose roles hold the
/// mapping's administrator role is an administrator's.
fn administrator(mapping: &BindingMapping) -> bool {
    if crate::versioning::committer::current_committer().is_none() {
        return true;
    }
    let Some(role) = mapping.admin_role.as_deref() else {
        return false;
    };
    crate::system_log::access_context::current_roles()
        .iter()
        .any(|held| held.eq_ignore_ascii_case(role))
}

/// The `sys_version` of the head the commit wrote (`None` for a deletion)
/// and of the version before it (`None` for a creation or a deleted
/// predecessor).
async fn versions(
    tx: &mut PgConnection,
    vo_id: VoId,
) -> Result<(Option<i32>, Option<i32>), ServiceError> {
    let rows = sqlx::query(
        "SELECT v.sys_version, v.body IS NOT NULL AS held FROM version v WHERE v.vo_id = $1 \
         ORDER BY v.sys_version DESC LIMIT 2",
    )
    .bind(vo_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut held = Vec::with_capacity(2);
    for row in rows {
        let sys_version: i32 = row.try_get("sys_version")?;
        held.push(row.try_get::<bool, _>("held")?.then_some(sys_version));
    }
    let mut held = held.into_iter();
    Ok((held.next().flatten(), held.next().flatten()))
}

/// What the version `sys_version` of the party `vo_id` of `kind` holds that
/// is protected: a `PERSON`'s from its stored body, and a `ROLE`'s or a
/// `GROUP`'s from the template its root node names, which reads no body.
async fn read(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    vo_id: VoId,
    sys_version: i32,
) -> Result<Protected, ServiceError> {
    if kind == Kind::Person {
        return Ok(match body(tx, vo_id, sys_version).await? {
            Some(body) => protected(mapping, kind, &parsed(kind, &body, mapping)?),
            None => Protected::default(),
        });
    }
    let template: Option<String> = sqlx::query_scalar(
        "SELECT CASE WHEN n.rm_version IS NULL OR (n.data -> 'archetype_details') IS NOT NULL \
                     THEN n.data -> 'archetype_details' \
                     ELSE ext.node_archetype_details(n.archetype, n.rm_version, v.template_id) \
                END -> 'template_id' ->> 'value' \
         FROM version v \
         JOIN node n ON n.tier = v.tier AND n.vo_id = v.vo_id \
                    AND n.sys_version = v.content_key AND n.num = 0 \
         WHERE v.vo_id = $1 AND v.sys_version = $2",
    )
    .bind(vo_id)
    .bind(sys_version)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    let facts = Facts {
        template: template
            .as_deref()
            .map(crate::templates::identity::canonical_key),
        ..Facts::default()
    };
    Ok(protected(mapping, kind, &facts))
}

/// What `facts`, held by a party of `kind`, hold that is protected.
fn protected(mapping: &BindingMapping, kind: Kind, facts: &Facts) -> Protected {
    let mut protected = Protected::default();
    let Some(template) = &facts.template else {
        return protected;
    };
    match kind {
        Kind::Role => protected.whole = mapping.professional_role.as_ref() == Some(template),
        Kind::Group => {
            protected.whole = mapping
                .group
                .as_ref()
                .is_some_and(|g| &g.template == template);
        }
        _ => {
            if !mapping.persons.contains_key(template) {
                return protected;
            }
            for (selector, identifier) in &facts.identifiers {
                protected.identifiers.insert((
                    selector,
                    identifier.issuer.clone(),
                    identifier.id_type.clone(),
                    identifier.canonical(),
                ));
            }
            protected.whole = facts.represents;
        }
    }
    protected
}
