// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR **SMART App Launch and Service Discovery** (Release-1.1.0,
//! DEVELOPMENT status)
//! — `docs/specs/openehr/ITS-REST/docs/smart_app_launch/master00-09*.adoc`.
//!
//! This server is the SMART platform's openEHR CDR (the `org.openehr.rest`
//! resource server), on the FerroHEALTH family's `ferrohealth-smart` crate,
//! which parses the master08 scope grammar and checks every requirement:
//!
//! - [`discovery`] — the `/.well-known/smart-configuration` document
//!   (master04 §Service Discovery), served pre-auth when `smart.enabled`;
//! - `routes` — the SMART on openEHR requirement of every route, checked by
//!   the authentication middleware after RBAC, and for a COMPOSITION by the
//!   policy-enforcement point once its template is known, with the `ehrId`
//!   launch-context binding of a `patient/` grant (master07 §Context
//!   Selection, master08 §Resource Scopes).

use ferrohealth_smart::openehr::smart_scopes::{Compartment, SmartScope};

pub mod discovery;
pub(crate) mod routes;

/// Whether `scopes` hold at least one SMART on openEHR resource scope and every
/// one of them is a `system/` scope: a backend client, judged by its scopes alone.
pub(crate) fn holds_only_system_scopes(scopes: &[SmartScope]) -> bool {
    // NOTE: owner ruling on #4467, master08 §Resource Scopes: `system` access is "based on
    // pre-configured client-specific policy", so only a `system/` token skips the role gate.
    let mut resources = scopes.iter().filter_map(|scope| match scope {
        SmartScope::Resource(resource) => Some(resource.compartment),
        _ => None,
    });
    resources
        .next()
        .is_some_and(|first| first == Compartment::System)
        && resources.all(|compartment| compartment == Compartment::System)
}

/// Whether `principal` is the record's person: a caller the party binding
/// binds to the `PERSON` they are ([`Principal::own`](crate::extensions::access::authn::Principal::own)).
///
/// The binding is the one source of who that is. A token `ferrohealth-smart`
/// reads as the patient's own (`Caller::is_the_person`) is only checked
/// against it, and a `patient/` scope names a launch context, which a
/// professional's token carries too (ITS-REST
/// `docs/smart_app_launch/master08-scopes.adoc` §Resource Scopes).
// NOTE: EHDS Art. 3 and 5, the orchestrator's ruling on #6080: own access has one source,
// the party binding, so authorship never rests on a token's claims alone.
pub(crate) fn acts_as_person(principal: &crate::extensions::access::authn::Principal) -> bool {
    principal.own.is_some()
}
