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

pub mod admission;
pub mod agreement;
pub mod bootstrap;
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
use crate::extensions::outbox::config::is_reader_name;
use crate::party_binding::derive::{Facts, container};
use crate::party_binding::grants::agreement::{Accessor, Agreement};
use crate::party_binding::grants::capabilities::{Credentials, Grant};
use crate::party_binding::grants::operation::{
    BINDING_ISSUER, DEVICE, DEVICE_IDENTIFIER_ISSUER, FACILITY, FACILITY_PARTS, ORIGIN, Operation,
    READER, RELAY, ROLE, SUBJECT_NAMESPACE, TOKEN_SCOPE,
};
use crate::party_binding::grants::trust::FederationTrust;

/// Refuses the committed body `facts` of the `ROLE` `role` when it grants an
/// operation the ceiling of `grants` does not admit, a `hold_roles` grant
/// names a role the ceiling reserves, a capability breaks the element rules
/// ([`check_elements`]), a grant lacks the terms of its operation
/// ([`check_terms`]), a `create_at_login` grant admits nothing
/// ([`admission::check`]), or a federation agreement breaks its rules
/// ([`check_agreements`]).
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming the first such capability and
/// the configuration key, the `400` of [`check_elements`], the `422` of
/// [`check_terms`], of [`admission::check`] and of [`check_agreements`], and
/// a storage failure.
pub(crate) async fn check_role(
    tx: &mut PgConnection,
    role: Uuid,
    facts: &Facts,
    grants: &live::LiveGrants,
) -> Result<(), ServiceError> {
    let ceiling = grants.ceiling();
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
        check_elements(facts)?;
        check_terms(facts)?;
        admission::check(facts)?;
        return check_agreements(tx, role, facts, &grants.federation()).await;
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
fn check_terms(facts: &Facts) -> Result<(), ServiceError> {
    for capability in &facts.grants.capabilities {
        let credentials = Credentials::of(capability.credentials.as_ref());
        let refusal = match Operation::of(&capability.code) {
            Some(Operation::CommitComposition) => commit_terms(credentials),
            Some(
                Operation::ImportEhr | Operation::ImportEhrExtract | Operation::ExportEhrExtract,
            ) if credentials.texts(SUBJECT_NAMESPACE).is_empty() => Some(
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
fn check_elements(facts: &Facts) -> Result<(), ServiceError> {
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

/// What a `commit_composition` grant's `credentials` lack, when they lack a
/// term.
fn commit_terms(credentials: Credentials<'_>) -> Option<&'static str> {
    let origins = credentials.words(ORIGIN);
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
    let malformed = credentials.clusters(FACILITY).into_iter().any(|facility| {
        facility.texts(FACILITY_PARTS[0]).len() != 1
            || facility.texts(FACILITY_PARTS[1]).len() + facility.texts(FACILITY_PARTS[2]).len()
                != 1
    });
    malformed.then_some(
        "has a facility cluster without one issuer and exactly one of identifier and \
         identifier_prefix",
    )
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
async fn check_agreements(
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
