// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binary's authorization construction seam
//! ([`ferroehr_server::build_authz`]): the shipped run path builds the RBAC
//! gate AND the ABAC engine, and an enabled-but-unbuildable ABAC block aborts
//! boot instead of silently degrading to authz-off (the mis-wiring #1815
//! caught: the binary once constructed an RBAC-only handle unconditionally).
//!
//! Offline by design, like the rest of this crate's suite: handle
//! CONSTRUCTION touches no database — the resolvers are inert stubs here, and
//! the DB-backed enforcement path is proven end to end where it lives
//! (`app/ferroehr-rest/tests/it/abac_e2e.rs`, service-backed resolvers).
//! Fine-grained authorization is our own extension — no openEHR spec governs
//! it (ITS-REST places authorization out of band).

use std::path::PathBuf;
use std::sync::Arc;

use ferroehr::config::authz::AuthzConfig;
use ferroehr_rest::extensions::access::authz::{AuthzResolvers, ResolveError};

use ferroehr_server::build_authz;

/// Inert resolvers: construction under test, resolution out of scope.
fn inert_resolvers() -> AuthzResolvers {
    AuthzResolvers {
        subject: Arc::new(|_| Box::pin(async { Ok::<_, ResolveError>(None) })),
        template_of_version: Arc::new(|_, _| Box::pin(async { Ok::<_, ResolveError>(None) })),
    }
}

/// The shipped Cedar example policy set (the same fixture the engine's own
/// golden-decision tests load).
fn example_policies() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ferroehr-rest/examples/policies")
}

/// `abac.enabled = true` with a valid policy directory → the handle the
/// binary serves with carries a LIVE ABAC gate (the regression assertion).
#[test]
fn enabled_abac_is_live_on_the_built_handle() {
    let mut cfg = AuthzConfig::default();
    cfg.abac.enabled = true;
    cfg.abac.cedar.policy_dir = Some(example_policies());
    let handle = build_authz(&cfg, "/ferroehr/rest/openehr/v1", inert_resolvers())
        .expect("a valid ABAC block builds")
        .expect("RBAC default-on + ABAC on → a handle");
    assert!(handle.rbac_active(), "the RBAC gate is on by default");
    assert!(
        handle.abac_active(),
        "abac.enabled must produce a live ABAC gate on the served handle"
    );
}

/// `abac.enabled = true` with no policy directory → boot ABORTS. A
/// configuration that promises fine-grained authorization must never degrade
/// to authz-off.
#[test]
fn enabled_but_unbuildable_abac_fails_boot() {
    let mut cfg = AuthzConfig::default();
    cfg.abac.enabled = true;
    cfg.abac.cedar.policy_dir = None;
    let err = build_authz(&cfg, "/ferroehr/rest/openehr/v1", inert_resolvers())
        .expect_err("an enabled ABAC block without policies must refuse to boot");
    assert!(
        format!("{err:#}").contains("policy_dir"),
        "the boot error names the missing setting: {err:#}"
    );
}

/// The default config (RBAC on, ABAC off) keeps today's shape: a handle with
/// the RBAC gate only.
#[test]
fn default_config_builds_an_rbac_only_handle() {
    let handle = build_authz(
        &AuthzConfig::default(),
        "/ferroehr/rest/openehr/v1",
        inert_resolvers(),
    )
    .expect("the default block builds")
    .expect("RBAC is on by default");
    assert!(handle.rbac_active());
    assert!(!handle.abac_active(), "ABAC stays off unless enabled");
}
