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

pub mod admission;
pub mod capabilities;
pub(crate) mod derive;
pub mod live;
pub mod operation;
pub mod store;

use crate::error::ServiceError;
use crate::party_binding::derive::Facts;
use crate::party_binding::grants::operation::GrantCeiling;

/// Refuses the committed body `facts` of a `ROLE` when it grants an
/// operation `ceiling` does not admit, or holds a `create_at_login` grant
/// that admits nothing ([`admission::check`]).
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming the first capability above the
/// ceiling and the configuration key, and the `422` of [`admission::check`].
pub(crate) fn check_role(facts: &Facts, ceiling: &GrantCeiling) -> Result<(), ServiceError> {
    let Some(above) = facts
        .grants
        .capabilities
        .iter()
        .find(|capability| !ceiling.allows_code(&capability.code))
    else {
        return admission::check(facts);
    };
    let allowed: Vec<&str> = ceiling.allowed().map(operation::Operation::code).collect();
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
