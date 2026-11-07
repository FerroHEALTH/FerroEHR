// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The committer the server writes for a caller it knows, and the judgement
//! of a committer the client states.
//!
//! RM common `org.openehr.rm.common.audit_details.adoc`: `committer` is the
//! "Identity and optional reference into identity management service, of user
//! who committed the item". RM common master04 §Audit Details names the
//! "system login identifier", and allows `PARTY_SELF` only "when the committer
//! is the record subject". EHDS Annex II 3.1 and Art. 13(4)
//! (`docs/law/eu/ehds/text.html`) ask the record to identify the professional
//! and the provider. So the server supplies who committed, and a stated
//! committer is accepted only when it states nothing the server's does not.
//! ITS-REST `Requests_and_responses.md` §openehr-version and
//! openehr-audit-details lets a client supply a committer that is merged;
//! refusing one that differs deviates from that rule, deliberately, as
//! reported in #5019. The placeholder, the provider identifier and the login
//! identifier forms are our own design.

use openehr_base::prelude::{HierObjectId, ObjectId, PartyRef};
use openehr_rm::prelude::{
    DvIdentifier, PartyIdentified, PartyIdentifiedData, PartyProxy, PartyRelated, PartySelf,
};

use super::{CommitterIdentity, LOCAL_ISSUER};
use crate::error::ServiceError;

/// The `DV_IDENTIFIER.type` of the provider identifier a professional's
/// committer carries.
pub const PROVIDER_TYPE: &str = "healthcare-provider";

/// The openEHR `subject_relationship` code of an authorised person no other
/// code fits: `253|unknown|`.
const UNKNOWN_RELATIONSHIP: &str = "253";

/// What the server knows of a bound caller beyond the binding itself, filled
/// by the protocol adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoundDetails {
    /// The name of the bound `PERSON`, at the name path of the party mapping;
    /// `None` when the `PERSON` has none.
    pub name: Option<String>,
    /// The healthcare provider the deployment acts for, as `(issuer, id)`.
    pub provider: Option<(String, String)>,
    /// The `PARTY_REF.namespace` a bound party is written under.
    pub namespace: String,
}

/// The committer the server writes for `identity`, when it knows one.
///
/// It is the record subject's `PARTY_SELF`, a representative's
/// `PARTY_RELATED`, or a bound professional's `PARTY_IDENTIFIED`; `None` for
/// any other caller.
#[must_use]
pub fn server_committer(identity: &CommitterIdentity) -> Option<PartyProxy> {
    if identity.person && !identity.representative {
        // NOTE: RM common master04 §PARTY_SELF and Referring to the Patient, the
        // first scheme ("the most secure approach"): no external_ref.
        return Some(PartyProxy::PartySelf(PartySelf { external_ref: None }));
    }
    let bound = identity.party.as_ref()?;
    let details = identity.bound.clone().unwrap_or_default();
    // NOTE: the local source names each party by its container's UUID, so an
    // id that is no HIER_OBJECT_ID names no container of this store and no reference.
    let external_ref = HierObjectId::new(bound.party.id()).ok().map(|id| PartyRef {
        namespace: details.namespace.clone(),
        r#type: "PERSON".to_owned(),
        id: ObjectId::HierObjectId(id),
    });
    let login = login(identity);
    if identity.representative {
        let code = bound
            .represents
            .iter()
            .find_map(ferrohealth_smart::party::Represented::relationship)
            .unwrap_or(UNKNOWN_RELATIONSHIP);
        let rubric = openehr_term::bundle::openehr()
            .rubric("subject_relationship", code, "en")
            .unwrap_or("unknown")
            .to_owned();
        return Some(PartyProxy::PartyIdentified(PartyIdentified::PartyRelated(
            PartyRelated {
                external_ref,
                name: details.name,
                identifiers: Some(openehr_base::containers::NonEmptyVec::of(login)),
                relationship: crate::versioning::audit::openehr_coded_text(code, rubric),
            },
        )));
    }
    let mut identifiers = openehr_base::containers::NonEmptyVec::of(login);
    if let Some((issuer, id)) = details.provider {
        identifiers.push(DvIdentifier {
            issuer: Some(issuer),
            assigner: None,
            id,
            r#type: Some(PROVIDER_TYPE.to_owned()),
        });
    }
    Some(PartyProxy::PartyIdentified(
        PartyIdentified::PartyIdentified(PartyIdentifiedData {
            external_ref,
            name: details.name,
            identifiers: Some(identifiers),
        }),
    ))
}

/// The login identifier of `identity`: the issuer profile's `person_key`
/// value under the token issuer, or the Basic user under `ferroehr`.
fn login(identity: &CommitterIdentity) -> DvIdentifier {
    let id = match (&identity.party, identity.id_type) {
        (Some(bound), "oauth2") => bound.identifier.value().to_owned(),
        _ => identity.subject.clone(),
    };
    DvIdentifier {
        issuer: Some(
            identity
                .issuer
                .clone()
                .unwrap_or_else(|| LOCAL_ISSUER.to_owned()),
        ),
        assigner: None,
        id,
        r#type: Some(identity.id_type.to_owned()),
    }
}

/// Judges the committer the client `stated` against `server`, the committer
/// the server writes for `identity`.
///
/// The placeholder, one `DV_IDENTIFIER{issuer, id}` with no `type`, no name and
/// no `external_ref`, is accepted when it names the caller (the token's `iss`
/// and `sub`, or `ferroehr` and the Basic user). Any other stated committer
/// is accepted when it states nothing `server` does not: a name equal to its
/// name, no identifier other than the caller's login, no `external_ref` other
/// than the bound `PERSON`. A stated `PARTY_RELATED` is never the client's to
/// state.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`, `auth_failure`) naming
/// `AUDIT_DETAILS.committer` for every other committer.
pub fn judge(
    stated: &PartyProxy,
    server: &PartyProxy,
    identity: &CommitterIdentity,
) -> Result<(), ServiceError> {
    let accepted = match (stated, server) {
        (PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)), _)
            if placeholder(party) =>
        {
            names_caller(party, identity)
        }
        (PartyProxy::PartySelf(party), PartyProxy::PartySelf(_)) => party.external_ref.is_none(),
        (
            PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)),
            PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(ours)),
        ) => states_nothing_more(party, ours),
        _ => false,
    };
    if accepted {
        return Ok(());
    }
    Err(ServiceError::not_permitted(
        "AUDIT_DETAILS.committer: the server writes who committed; send no committer, the \
         placeholder naming the authenticated caller, or a committer that states nothing the \
         server's does not (RM common master04 §Audit Details; EHDS Art. 13(4))",
    ))
}

/// Whether `party` is the placeholder form: exactly one `DV_IDENTIFIER` with
/// an issuer and no type, assigner, name or `external_ref`.
fn placeholder(party: &PartyIdentifiedData) -> bool {
    party.name.is_none()
        && party.external_ref.is_none()
        && party.identifiers.as_ref().is_some_and(|ids| {
            ids.len() == 1
                && ids
                    .iter()
                    .all(|id| id.issuer.is_some() && id.r#type.is_none() && id.assigner.is_none())
        })
}

/// Whether the placeholder `party` names the authenticated caller.
fn names_caller(party: &PartyIdentifiedData, identity: &CommitterIdentity) -> bool {
    let issuer = identity.issuer.as_deref().unwrap_or(LOCAL_ISSUER);
    party.identifiers.as_ref().is_some_and(|ids| {
        ids.iter()
            .all(|id| id.issuer.as_deref() == Some(issuer) && id.id == identity.subject)
    })
}

/// Whether `party` states nothing `ours` does not: its name is ours, each of
/// its identifiers is our login (its type, when stated, ours), and its
/// `external_ref`, when stated, is ours.
fn states_nothing_more(party: &PartyIdentifiedData, ours: &PartyIdentifiedData) -> bool {
    let name = party.name.is_none() || party.name == ours.name;
    let reference = party.external_ref.is_none() || party.external_ref == ours.external_ref;
    let login = ours.identifiers.as_ref().and_then(|ids| ids.iter().next());
    let identifiers = party.identifiers.as_ref().is_none_or(|ids| {
        ids.iter().all(|id| {
            login.is_some_and(|login| {
                id.issuer == login.issuer
                    && id.id == login.id
                    && id.assigner.is_none()
                    && (id.r#type.is_none() || id.r#type == login.r#type)
            })
        })
    });
    name && reference && identifiers
}
