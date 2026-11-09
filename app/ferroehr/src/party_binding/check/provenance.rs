// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Who introduced a binding key: the first version of a `PERSON` that
//! carries it, its committer as the report names it, and the provenance
//! rule that judges it. No openEHR spec governs a deployment's migration:
//! our own design.

use std::collections::BTreeSet;

use openehr_base::prelude::ObjectId;
use openehr_rm::prelude::{PartyIdentified, PartyProxy};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use super::{CheckError, CheckInput, Committer, Origin, Provenance};
use crate::party_binding::BindingMapping;
use crate::party_binding::store::{self, HeldSelector};
use crate::service::demographic::jit;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::committer::bound::CLIENT_TYPE;

/// The committer that introduced `key` and who it is under the provenance
/// rule, with `administrators` the `PERSON`s of the named administrators;
/// `None` when no version carries the key.
pub(super) async fn provenance_of(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    key: &HeldSelector,
    input: &CheckInput,
    administrators: &BTreeSet<Uuid>,
) -> Result<Option<Provenance>, CheckError> {
    let Some(found) = introduced(tx, mapping, key).await? else {
        return Ok(None);
    };
    Ok(Some(Provenance {
        party: key.party_id,
        issuer: key.issuer.clone(),
        key: key.value.clone(),
        by: origin(&found, key.party_id, input, administrators),
        committer: named_committer(&found.committer),
        committed_at: found.committed_at,
        contribution: found.contribution,
    }))
}

/// The first version of a `PERSON` that carries a key.
struct Introduced {
    /// When it was committed (RFC 3339).
    committed_at: String,
    /// Its CONTRIBUTION.
    contribution: Uuid,
    /// Its committer.
    committer: PartyProxy,
    /// Its `audit change type` code.
    change_type: String,
    /// Its audit description text, when it has one.
    description: Option<String>,
}

/// The commit instant and CONTRIBUTION of the first version of the `PERSON`
/// whose body carries the key `key`, with its committer.
async fn introduced(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    key: &HeldSelector,
) -> Result<Option<Introduced>, CheckError> {
    let rows = sqlx::query(
        "SELECT v.body, v.committed_at::text AS committed_at, v.contribution_id, \
                a.committer::text AS committer, a.change_type, \
                a.description ->> 'value' AS description \
         FROM version v JOIN commit_audit a ON a.id = v.commit_audit_id \
         WHERE v.vo_id = $1 AND v.body IS NOT NULL ORDER BY v.sys_version",
    )
    .bind(key.party_id)
    .fetch_all(&mut *tx)
    .await?;
    for row in rows {
        let body: String = row.try_get("body")?;
        let facts = store::parsed(Kind::Person, &body, mapping)?;
        let carries = facts.identifiers.iter().any(|(selector, identifier)| {
            *selector == "key"
                && identifier.issuer == key.issuer
                && identifier.canonical() == key.value
        });
        if !carries {
            continue;
        }
        let committer: String = row.try_get("committer")?;
        let committer =
            openehr_its::json::from_canonical_json::<PartyProxy>(&committer).map_err(|source| {
                CheckError::Committer {
                    party: key.party_id,
                    source,
                }
            })?;
        return Ok(Some(Introduced {
            committed_at: row.try_get("committed_at")?,
            contribution: row.try_get("contribution_id")?,
            change_type: row.try_get("change_type")?,
            description: row.try_get("description")?,
            committer,
        }));
    }
    Ok(None)
}

/// The parts of a committer the rule reads.
struct Parts<'a> {
    /// `PARTY_IDENTIFIED.name`.
    name: Option<&'a str>,
    /// The `external_ref` id.
    external_ref: Option<&'a str>,
    /// Each identifier as `(issuer, type, id)`.
    identifiers: Vec<(&'a str, &'a str, &'a str)>,
}

/// The parts of `committer` the rule reads.
fn parts(committer: &PartyProxy) -> Parts<'_> {
    let (name, external_ref, identifiers) = match committer {
        PartyProxy::PartySelf(party) => (None, party.external_ref.as_ref(), None),
        PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) => (
            party.name.as_deref(),
            party.external_ref.as_ref(),
            party.identifiers.as_ref(),
        ),
        PartyProxy::PartyIdentified(PartyIdentified::PartyRelated(party)) => (
            party.name.as_deref(),
            party.external_ref.as_ref(),
            party.identifiers.as_ref(),
        ),
    };
    let reference = external_ref.map(|reference| match &reference.id {
        ObjectId::HierObjectId(id) => id.value(),
        ObjectId::GenericId(id) => id.value.as_str(),
        _ => "",
    });
    let identifiers = identifiers
        .into_iter()
        .flat_map(|ids| ids.iter())
        .map(|id| {
            (
                id.issuer.as_deref().unwrap_or(""),
                id.r#type.as_deref().unwrap_or(""),
                id.id.as_str(),
            )
        })
        .collect();
    Parts {
        name,
        external_ref: reference,
        identifiers,
    }
}

/// `committer` as the report names it.
fn named_committer(committer: &PartyProxy) -> Committer {
    let Parts {
        name,
        external_ref,
        identifiers,
    } = parts(committer);
    Committer {
        name: name.map(str::to_owned),
        external_ref: external_ref.map(str::to_owned),
        identifiers: identifiers
            .into_iter()
            .map(|(issuer, kind, id)| format!("{issuer}|{kind}|{id}"))
            .collect(),
    }
}

/// Who committed `introduced`, the version that introduced a key of the
/// `PERSON` `party`, under the provenance rule: a SCIM client names itself by
/// its client identifier under its token issuer with no `external_ref`
/// (`committer::bound::server_committer`), an administrator's committer
/// references its `PERSON`, a declared migration is named by its committer
/// name with no `external_ref`, and just-in-time creation is a
/// `249|creation|` version with its description (`jit::description`) whose
/// committer references the new `PERSON` itself.
fn origin(
    introduced: &Introduced,
    party: Uuid,
    input: &CheckInput,
    administrators: &BTreeSet<Uuid>,
) -> Option<Origin> {
    let committer = &introduced.committer;
    let Parts {
        name,
        external_ref,
        identifiers,
    } = parts(committer);
    let person = matches!(
        committer,
        PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(_))
    );
    if person && external_ref.is_none() {
        let scim = identifiers.iter().any(|(issuer, kind, id)| {
            *kind == CLIENT_TYPE
                && input
                    .scim_clients
                    .iter()
                    .any(|(i, client)| i == issuer && client == id)
        });
        if scim {
            return Some(Origin::Scim);
        }
        if name.is_some_and(|name| input.migrations.iter().any(|m| m == name)) {
            return Some(Origin::Migration);
        }
    }
    let referenced = external_ref.and_then(|id| id.parse::<Uuid>().ok());
    let jit = introduced.change_type == change_type::CREATION
        && introduced.description.as_deref().is_some_and(|text| {
            input
                .jit_issuers
                .iter()
                .any(|issuer| text == jit::description(issuer))
        });
    if person && jit && referenced == Some(party) {
        return Some(Origin::Jit);
    }
    if person && referenced.is_some_and(|party| administrators.contains(&party)) {
        return Some(Origin::Administrator);
    }
    None
}
