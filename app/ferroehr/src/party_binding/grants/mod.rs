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

pub mod bootstrap;
pub mod capabilities;
pub(crate) mod derive;
pub mod live;
pub mod operation;
pub mod store;

use crate::error::ServiceError;
use crate::extensions::outbox::config::is_reader_name;
use crate::party_binding::derive::Facts;
use crate::party_binding::grants::capabilities::Credentials;
use crate::party_binding::grants::operation::{
    BINDING_ISSUER, GrantCeiling, ORIGIN, Operation, READER, TOKEN_SCOPE,
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

/// Refuses the committed body `facts` of a `ROLE` when a capability of an
/// operation that is read by one element does not name it exactly once: the
/// `origin` of [`Operation::DataOrigin`], the `reader` of
/// [`Operation::ReadChangeFeed`] (a name of the reader registry's grammar),
/// and the `binding_issuer` of [`Operation::ScimProvision`] with at least one
/// `token_scope`.
///
/// # Errors
/// [`ServiceError::BadRequest`] (`400`) naming the capability's operation
/// and the element.
pub(crate) fn check_elements(facts: &Facts) -> Result<(), ServiceError> {
    for capability in &facts.grants.capabilities {
        let credentials = Credentials::of(capability.credentials.as_ref());
        let one = |code: &str| -> Result<String, ServiceError> {
            let mut named: Vec<&str> = credentials.identified(code);
            named.retain(|value| !value.is_empty());
            named.dedup();
            match named.as_slice() {
                [value] => Ok((*value).to_owned()),
                _ => Err(ServiceError::precondition(format!(
                    "the capability granting {} names {} {code} elements: it names exactly \
                     one, a DV_TEXT or a DV_IDENTIFIER in an element whose name is \
                     ferroehr-operations::{code}",
                    capability.code,
                    named.len()
                ))),
            }
        };
        match Operation::of(&capability.code) {
            Some(Operation::DataOrigin) => {
                let origin = one(ORIGIN)?;
                if origin.chars().any(char::is_whitespace) {
                    return Err(ServiceError::precondition(format!(
                        "the origin {origin:?} carries whitespace: an origin is compared \
                         exactly with FEEDER_AUDIT.originating_system_audit.system_id"
                    )));
                }
            }
            Some(Operation::ReadChangeFeed) => {
                let reader = one(READER)?;
                if !is_reader_name(&reader) {
                    return Err(ServiceError::precondition(format!(
                        "the reader {reader:?} is not a reader name: use 1 to 63 lowercase \
                         letters, digits and `-`, starting with a letter"
                    )));
                }
            }
            Some(Operation::ScimProvision) => {
                one(BINDING_ISSUER)?;
                if credentials
                    .identified(TOKEN_SCOPE)
                    .iter()
                    .all(|scope| scope.is_empty() || scope.contains(char::is_whitespace))
                {
                    return Err(ServiceError::precondition(
                        "the capability granting scim_provision names no token_scope: it names \
                         the scope, one token, that the client's access token must hold",
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}
