// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy grants: the system `AGENT`s and the capabilities of the policy
//! `ROLE`s each party performs.
//!
//! They are held in the binding side table and resolved on every request
//! with no read of its own. A system client is the `AGENT` of the system
//! `AGENT` archetype carrying its client identifier (RM demographic
//! `master02-demographic_package.adoc`
//! §Party Identification); what a party may do is a `CAPABILITY` of a policy
//! `ROLE` whose `performer` names it (`org.openehr.rm.demographic.role.adoc`),
//! valid on the request's date by the `ROLE`'s and the `CAPABILITY`'s
//! `time_validity` and at its instant by the `Not before` and `Not after`
//! of the credentials. [`store`] writes the rows in each party commit's
//! transaction and in a refill; [`live::LiveGrants`] holds them on each
//! replica; [`capabilities::Capabilities`] is what a request's caller may do.
//! `[authz.grants] allowed` ([`kind::GrantCeiling`]) bounds the kinds the CDR
//! may grant: a commit of a capability above it is refused, and a stored one
//! is never honoured.
//!
//! No openEHR spec gives a capability an authorization meaning, or governs
//! the side table: our own design.

pub mod capabilities;
pub(crate) mod derive;
pub mod kind;
pub mod live;
pub mod store;

use crate::error::ServiceError;
use crate::party_binding::derive::Facts;
use crate::party_binding::grants::kind::GrantCeiling;

/// Refuses the committed body `facts` of a policy `ROLE` when it grants a
/// capability `ceiling` does not admit.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming the first such capability and
/// the configuration key.
pub(crate) fn check_ceiling(facts: &Facts, ceiling: &GrantCeiling) -> Result<(), ServiceError> {
    let Some(above) = facts
        .grants
        .capabilities
        .iter()
        .find(|capability| !ceiling.allows_name(&capability.kind))
    else {
        return Ok(());
    };
    let allowed: Vec<&str> = ceiling.allowed().map(kind::GrantKind::as_str).collect();
    Err(ServiceError::not_permitted(format!(
        "the ROLE grants a capability of the kind {}, which [authz.grants] allowed does not \
         admit, so this CDR may not grant it (allowed: {})",
        above.kind,
        if allowed.is_empty() {
            "none".to_owned()
        } else {
            allowed.join(", ")
        }
    )))
}
