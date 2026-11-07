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
//! `ROLE` or point a colleague's `PERSON` at their own login. A present
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
use crate::party_binding::{BindingMapping, IdentifierSelector};
use crate::versioning::Kind;

task_local! {
    /// The key change the running administrator re-key may make.
    static REKEY: Rekey;
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
/// It reads the new head and the version before it, so it runs after the
/// write and before the side-table rows are rebuilt.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) when a caller who is not an
/// administrator writes a protected party, or a write changes a present
/// binding key outside [`rekey`]; [`ServiceError::Unprocessable`] (`422`)
/// naming the identifier when an administrator's version leaves a protected
/// identifier out; a storage error when the versions cannot be read.
pub(crate) async fn check(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    vo_id: VoId,
) -> Result<(), ServiceError> {
    if mapping.is_empty() || !matches!(kind, Kind::Person | Kind::Role | Kind::Group) {
        return Ok(());
    }
    let (current, previous) = versions(tx, vo_id).await?;
    let before = match previous {
        Some(sys_version) => read(tx, mapping, kind, vo_id, sys_version).await?,
        None => Protected::default(),
    };
    let after = match current {
        Some(sys_version) => read(tx, mapping, kind, vo_id, sys_version).await?,
        None => Protected::default(),
    };
    if !(before.protected() || after.protected()) {
        return Ok(());
    }
    if !administrator(mapping) {
        // TODO(#5014): admit the SCIM client that owns the party.
        return Err(ServiceError::not_permitted(format!(
            "the {} {vo_id} is protected by the party binding: only an administrator writes it \
             through the demographic API",
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
        .filter(|held| !after.identifiers.contains(*held))
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
/// is protected.
async fn read(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    vo_id: VoId,
    sys_version: i32,
) -> Result<Protected, ServiceError> {
    let row = sqlx::query(
        "SELECT lower(btrim(v.body::jsonb -> 'archetype_details' -> 'template_id' ->> 'value')) \
                    AS template, \
                $3::text IS NOT NULL AND EXISTS ( \
                    SELECT 1 FROM jsonb_array_elements( \
                        CASE WHEN jsonb_typeof(v.body::jsonb -> 'relationships') = 'array' \
                             THEN v.body::jsonb -> 'relationships' ELSE '[]'::jsonb END) AS r(rel) \
                    WHERE lower(r.rel ->> 'archetype_node_id') = $3) AS represents \
         FROM version v WHERE v.vo_id = $1 AND v.sys_version = $2",
    )
    .bind(vo_id)
    .bind(sys_version)
    .bind(mapping.representation.as_deref())
    .fetch_optional(&mut *tx)
    .await?;
    let mut protected = Protected::default();
    let Some(row) = row else {
        return Ok(protected);
    };
    let Some(template): Option<String> = row.try_get("template")? else {
        return Ok(protected);
    };
    match kind {
        Kind::Role => protected.whole = mapping.professional_role.as_ref() == Some(&template),
        Kind::Group => {
            protected.whole = mapping
                .group
                .as_ref()
                .is_some_and(|g| g.template == template);
        }
        _ => {
            let Some(selectors) = mapping.persons.get(&template) else {
                return Ok(protected);
            };
            for (selector, chosen) in selectors.each() {
                for (issuer, identifier_type, value) in
                    identifiers(tx, vo_id, sys_version, chosen).await?
                {
                    protected
                        .identifiers
                        .insert((selector, issuer, identifier_type, value));
                }
            }
            protected.whole = row.try_get("represents")?;
        }
    }
    Ok(protected)
}

/// The identifiers `chosen` selects in the version `sys_version` of `vo_id`,
/// each as its issuer, type and canonical value (a GUID or UUID in lower
/// case), the form the side table holds.
async fn identifiers(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
    chosen: &IdentifierSelector,
) -> Result<Vec<(String, String, String)>, ServiceError> {
    let rows = sqlx::query(
        "SELECT id ->> 'issuer' AS issuer, id ->> 'type' AS identifier_type, \
                CASE WHEN lower(id ->> 'type') IN ('guid', 'uuid') THEN lower(id ->> 'id') \
                     ELSE id ->> 'id' END AS value \
         FROM version v \
         CROSS JOIN LATERAL jsonb_path_query(v.body::jsonb, $3::jsonpath, $4::jsonb) AS id \
         WHERE v.vo_id = $1 AND v.sys_version = $2 \
           AND id ->> 'type' = $5 AND id ->> 'issuer' IS NOT NULL AND id ->> 'id' IS NOT NULL \
           AND ($6::text[] IS NULL OR id ->> 'issuer' = ANY($6::text[]))",
    )
    .bind(vo_id)
    .bind(sys_version)
    .bind(&chosen.json_path)
    .bind(&chosen.variables)
    .bind(&chosen.identifier_type)
    .bind(chosen.issuer_filter())
    .fetch_all(&mut *tx)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push((
            row.try_get("issuer")?,
            row.try_get("identifier_type")?,
            row.try_get("value")?,
        ));
    }
    Ok(out)
}
