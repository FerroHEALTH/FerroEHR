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
/// through a patient-context credential: a bearer token whose every SMART
/// resource scope is a `patient/` one.
///
/// A `patient/` scope is "for applications acting in the context of a specific
/// patient/EHR", a `user/` one for those "acting on behalf of a logged-in user
/// (e.g., a practitioner)" (ITS-REST `docs/smart_app_launch/master08-scopes.adoc`
/// §Resource Scopes). A patient-only grant is therefore read as the access
/// service of EHDS Art. 4, through which Art. 5 lets a person insert
/// information; a token with a `user/` or `system/` scope acts for a
/// professional or a system.
// NOTE: the ABAC patient claim is no such marker, as a professional's token in a
// patient's context carries it too; our own design, no openEHR spec governs it.
pub(crate) fn acts_as_person(principal: &crate::extensions::access::authn::Principal) -> bool {
    principal.method == crate::extensions::access::authn::AuthMethod::Bearer
        && principal.access.caller().is_some_and(|caller| {
            ferrohealth_smart::openehr::patient_grant(&caller.openehr_scopes())
        })
}
