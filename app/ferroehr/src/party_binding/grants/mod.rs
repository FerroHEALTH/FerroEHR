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
//! refused, and a stored one is never honoured. A federation agreement
//! ([`agreement::Agreement`]) is a grant whose terms are judged at its commit.
//!
//! No openEHR spec gives a capability an authorization meaning, or governs
//! the side table: our own design.

pub mod agreement;
pub mod capabilities;
pub(crate) mod derive;
pub mod live;
pub mod operation;
pub mod store;
pub mod trust;

use std::sync::Arc;

use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{ServiceError, Violation};
use crate::party_binding::derive::{Facts, container};
use crate::party_binding::grants::agreement::{Accessor, Agreement};
use crate::party_binding::grants::capabilities::Grant;
use crate::party_binding::grants::operation::{GrantCeiling, Operation};
use crate::party_binding::grants::trust::FederationTrust;

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

/// Refuses the committed body `facts` of the `ROLE` `role` when a federation
/// agreement it grants breaks a rule of [`Agreement::refusals`] for an issuer
/// of its performer's client identifiers under `trust`, holds an element of
/// no value of its kind, or grants an accessor its performer already holds
/// an agreement for.
///
/// The performer's client identifiers are the ones committed before the
/// `ROLE`, read on `tx`, so a contribution lists the partner's `ACTOR`
/// first.
///
/// # Errors
/// [`ServiceError::Unprocessable`] (`422`) naming every rule broken, and a
/// storage failure.
pub(crate) async fn check_agreements(
    tx: &mut PgConnection,
    role: Uuid,
    facts: &Facts,
    trust: &FederationTrust,
) -> Result<(), ServiceError> {
    let agreements: Vec<(Accessor, &derive::CapabilityFacts)> = facts
        .grants
        .capabilities
        .iter()
        .filter_map(|capability| {
            let accessor = Operation::of(&capability.code).and_then(Accessor::of)?;
            Some((accessor, capability))
        })
        .collect();
    if agreements.is_empty() {
        return Ok(());
    }
    let performer = facts.performer.as_deref().and_then(container);
    let issuers: Vec<String> =
        match performer {
            Some(performer) => sqlx::query_scalar(
                "SELECT DISTINCT issuer FROM binding_client WHERE party_id = $1 ORDER BY issuer",
            )
            .bind(performer)
            .fetch_all(&mut *tx)
            .await?,
            None => Vec::new(),
        };
    let mut errors = Vec::new();
    if issuers.is_empty() {
        errors.push(
            "its performer carries no client_id identifier, so it names no federation partner"
                .to_owned(),
        );
    }
    for (index, (accessor, capability)) in agreements.iter().enumerate() {
        if agreements
            .iter()
            .take(index)
            .any(|(earlier, _)| earlier == accessor)
        {
            errors.push(format!(
                "the ROLE grants {} twice: a partner holds one agreement per accessor",
                accessor.operation()
            ));
            continue;
        }
        let grant = Grant::new(
            accessor.operation(),
            role,
            capability.credentials.clone().map(Arc::new),
        );
        match Agreement::read(*accessor, grant.credentials()) {
            Err(unread) => errors.extend(unread),
            Ok(agreement) => {
                for issuer in &issuers {
                    errors.extend(agreement.refusals(issuer, trust));
                }
            }
        }
        if let Some(performer) = performer {
            let held: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM binding_capability \
                 WHERE performer_id = $1 AND role_id <> $2 AND operation = $3 \
                 AND lifecycle_state IN ('532', '553')",
            )
            .bind(performer)
            .bind(role)
            .bind(accessor.operation().code())
            .fetch_one(&mut *tx)
            .await?;
            if held > 0 {
                errors.push(format!(
                    "its performer already holds a {} agreement in another ROLE: a partner \
                     holds one agreement per accessor",
                    accessor.operation()
                ));
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    errors.retain(|error| seen.insert(error.clone()));
    if errors.is_empty() {
        return Ok(());
    }
    Err(ServiceError::content_invalid(Violation::new(format!(
        "the federation agreement of this ROLE is refused: {}",
        errors.join("; ")
    ))))
}
