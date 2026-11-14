// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binary's authorization construction seam
//! ([`ferroehr_server::build_authz`]): the shipped run path builds the RBAC
//! gate with the configured `[authz.rbac]` keys (the mis-wiring #1815 caught:
//! the binary once constructed a handle that dropped a configured gate).
//!
//! Offline by design, like the rest of this crate's suite: handle
//! CONSTRUCTION touches no database. A patient is confined to their own EHR
//! by the access evaluator (#5586), which no handle carries. RBAC is our own
//! extension: no openEHR spec governs it (ITS-REST places authorization out
//! of band).

use ferroehr::config::FerroEhrConfig;
use ferroehr::config::authz::EhrAccessDefault;

use ferroehr_server::build_authz;

/// The default config (RBAC on) builds a handle with the RBAC gate.
#[test]
fn default_config_builds_an_rbac_handle() {
    let handle = build_authz(&FerroEhrConfig::default()).expect("RBAC is on by default");
    assert!(handle.rbac_active());
}

/// The RBAC gate off with an `open` per-EHR default leaves nothing to
/// enforce; a `restricted` default alone still builds a handle.
#[test]
fn a_handle_is_built_only_when_something_is_enforced() {
    let mut config = FerroEhrConfig::default();
    config.authz.rbac.enabled = false;
    assert!(build_authz(&config).is_none(), "nothing to enforce");
    config.authz.rbac.ehr_access_default = EhrAccessDefault::Restricted;
    let handle = build_authz(&config).expect("a restricted default needs a handle");
    assert!(!handle.rbac_active());
    assert_eq!(handle.ehr_access_default(), EhrAccessDefault::Restricted);
}
