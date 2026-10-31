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
//!
//! Registration (master03), token issuance, grants and PKCE (master06), and
//! the launch-sequence UI (master07) are the authorization server's and the
//! launcher's; the CDR checks tokens and never issues them.

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

/// Whether `principal` is the record's person, or their representative, acting
/// through a patient-context credential: a bearer token with a `patient/`
/// scope and no `user/` or `system/` one (`Caller::is_the_person` of
/// `ferrohealth-smart`).
///
/// A `patient/` scope is "for applications acting in the context of a specific
/// patient/EHR", a `user/` one for those "acting on behalf of a logged-in user
/// (e.g., a practitioner)" (ITS-REST `docs/smart_app_launch/master08-scopes.adoc`
/// §Resource Scopes). The ABAC patient claim alone does not count, as a
/// professional's token in a patient's context carries it too.
// NOTE: EHDS Art. 5, the owner's ruling of 2026-10-31: authorship errs toward the
// professional, the strict answer of the family crate.
pub(crate) fn acts_as_person(principal: &crate::extensions::access::authn::Principal) -> bool {
    principal
        .access
        .caller()
        .is_some_and(ferrohealth_smart::resource::Caller::is_the_person)
}
