// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy grants: the client identifiers of the actors and the
//! operations the capabilities of their roles grant.
//!
//! They are held in the binding side table and resolved on every request
//! with no read of its own. Nothing is recognised by archetype: a client is
//! any `ACTOR` carrying a `DV_IDENTIFIER` of type `client_id` whose issuer is
//! the token issuer (RM demographic `master02-demographic_package.adoc`
//! §Party Identification), and a party may do an operation when a
//! `CAPABILITY` of a `ROLE` whose `performer` names it
//! (`org.openehr.rm.demographic.role.adoc`) holds that operation's code of
//! [`super::policy_mark::TERMINOLOGY`] anywhere in its `credentials`, valid on the
//! request's date by the `ROLE`'s and the `CAPABILITY`'s `time_validity` and
//! at its instant by the `not_before` and `not_after` elements. [`store`]
//! writes the rows in each party commit's transaction and in a refill;
//! [`live::LiveGrants`] holds them on each replica;
//! [`capabilities::Capabilities`] is what a request's caller may do.
//! `[authz.grants] allowed` ([`operation::GrantCeiling`]) bounds the
//! operations the CDR may grant: a commit of a capability above it is
//! refused, and a stored one is never honoured.
//!
//! No openEHR spec gives a capability an authorization meaning, or governs
//! the side table: our own design.

pub mod capabilities;
pub(crate) mod derive;
pub mod live;
pub mod operation;
pub mod store;

use crate::error::{ServiceError, Violation};
use crate::party_binding::derive::Facts;
use crate::party_binding::grants::capabilities::Credentials;
use crate::party_binding::grants::operation::{
    DEVICE, DEVICE_IDENTIFIER_ISSUER, FACILITY_PARTS, GrantCeiling, ORIGIN, Operation, RELAY,
    SCOPES,
};

/// Refuses the committed body `facts` of a `ROLE` when it grants an
/// operation `ceiling` does not admit.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming the first such capability and
/// the configuration key.
pub(crate) fn check_ceiling(facts: &Facts, ceiling: &GrantCeiling) -> Result<(), ServiceError> {
    let Some(above) = facts
        .grants
        .capabilities
        .iter()
        .find(|capability| !ceiling.allows_code(&capability.code))
    else {
        return Ok(());
    };
    let allowed: Vec<&str> = ceiling.allowed().map(Operation::code).collect();
    Err(ServiceError::not_permitted(format!(
        "the ROLE grants the operation {}, which [authz.grants] allowed does not admit, so \
         this CDR may not grant it (allowed: {})",
        above.code,
        if allowed.is_empty() {
            "none".to_owned()
        } else {
            allowed.join(", ")
        }
    )))
}

/// Refuses the committed body `facts` of a `ROLE` when a grant it carries
/// lacks the terms its operation is checked by: a `commit_composition` grant
/// without an origin, a `device` grant without a device identifier issuer, a
/// facility without its issuer and exactly one of its identifier and prefix,
/// and an import or export grant without a subject namespace.
///
/// # Errors
/// [`ServiceError::Unprocessable`] (`422`) naming the first such capability.
pub(crate) fn check_terms(facts: &Facts) -> Result<(), ServiceError> {
    for capability in &facts.grants.capabilities {
        let credentials = Credentials::of(capability.credentials.as_ref());
        let refusal = match Operation::of(&capability.code) {
            Some(Operation::CommitComposition) => commit_terms(credentials),
            Some(
                Operation::ImportEhr | Operation::ImportEhrExtract | Operation::ExportEhrExtract,
            ) if credentials.texts(SCOPES[2]).is_empty() => Some(
                "names no subject_namespace, and an import or export grant covers only the EHRs \
                 whose subject namespace it names",
            ),
            _ => None,
        };
        if let Some(refusal) = refusal {
            return Err(ServiceError::content_invalid(
                Violation::new(format!(
                    "the capability granting {} {refusal}",
                    capability.code
                ))
                .with_path(format!(
                    "ROLE.capabilities[{}].credentials",
                    capability.ordinal
                )),
            ));
        }
    }
    Ok(())
}

/// What a `commit_composition` grant's `credentials` lack, when they lack a
/// term.
fn commit_terms(credentials: Credentials<'_>) -> Option<&'static str> {
    let origins = credentials.codes(ORIGIN);
    if origins.is_empty()
        || origins
            .iter()
            .any(|origin| ![RELAY, DEVICE].contains(origin))
    {
        return Some("needs an origin element coding relay or device, or both");
    }
    if origins.contains(&DEVICE) && credentials.texts(DEVICE_IDENTIFIER_ISSUER).is_empty() {
        return Some("has the origin device and needs a device_identifier_issuer");
    }
    let malformed = credentials.clusters(SCOPES[1]).into_iter().any(|facility| {
        facility.texts(FACILITY_PARTS[0]).len() != 1
            || facility.texts(FACILITY_PARTS[1]).len() + facility.texts(FACILITY_PARTS[2]).len()
                != 1
    });
    malformed.then_some(
        "has a facility cluster without one issuer and exactly one of identifier and \
         identifier_prefix",
    )
}
