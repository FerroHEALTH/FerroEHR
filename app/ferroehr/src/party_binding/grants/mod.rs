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
use crate::party_binding::grants::capabilities::Credentials;
use crate::party_binding::grants::operation::{GrantCeiling, Operation, ROLE};

/// Refuses the committed body `facts` of a `ROLE` when it grants an
/// operation `ceiling` does not admit, a `hold_roles` grant names a role
/// the ceiling reserves, or a `create_at_login` grant admits nothing
/// ([`admission::check`]).
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming the first such capability and
/// the configuration key, and the `422` of [`admission::check`].
pub(crate) fn check_role(facts: &Facts, ceiling: &GrantCeiling) -> Result<(), ServiceError> {
    let reserved = facts
        .grants
        .capabilities
        .iter()
        .filter(|capability| Operation::of(&capability.code) == Some(Operation::HoldRoles))
        .flat_map(|capability| Credentials::of(capability.credentials.as_ref()).texts(ROLE))
        .find(|role| ceiling.reserves(role));
    if let Some(role) = reserved {
        return Err(ServiceError::not_permitted(format!(
            "the ROLE grants the privileged role {role} through hold_roles; a privileged role \
             of [authz.rbac] comes only from the configuration, a role_issuers token or an \
             operation of its own"
        )));
    }
    let Some(above) = facts
        .grants
        .capabilities
        .iter()
        .find(|capability| !ceiling.allows_code(&capability.code))
    else {
        return admission::check(facts);
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
