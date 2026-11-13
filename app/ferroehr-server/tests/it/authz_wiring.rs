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

use std::sync::Arc;

use ferroehr::config::authz::AuthzConfig;
use ferroehr::policy_register::LiveRegisters;
use ferroehr_rest::extensions::access::authz::{AuthzResolvers, ResolveError};

use ferroehr_server::build_authz;

/// Inert resolvers: construction under test, resolution out of scope.
fn inert_resolvers() -> AuthzResolvers {
    AuthzResolvers {
        subject: Arc::new(|_| Box::pin(async { Ok::<_, ResolveError>(None) })),
        content_of_version: Arc::new(|_, _| Box::pin(async { Ok::<_, ResolveError>(None) })),
        content: Arc::default(),
    }
}

/// The policy registers at their defaults, over a pool that never connects:
/// the Cedar engine is built from the register in force, which touches no
/// database.
#[expect(
    clippy::expect_used,
    reason = "a test helper: a lazy pool over a well-formed URL always builds"
)]
fn registers() -> Arc<LiveRegisters> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/unused")
        .expect("a lazy pool");
    Arc::new(LiveRegisters::new(pool))
}

/// `abac.enabled = true` with the Cedar engine → the handle the binary serves
/// with carries a LIVE ABAC gate (the regression assertion); its policy set is
/// the Cedar register in force.
#[tokio::test]
async fn enabled_abac_is_live_on_the_built_handle() {
    let mut cfg = AuthzConfig::default();
    cfg.abac.enabled = true;
    let handle = build_authz(
        &cfg,
        "/ferroehr/rest/openehr/v1",
        &registers(),
        inert_resolvers(),
    )
    .expect("a valid ABAC block builds")
    .expect("RBAC default-on + ABAC on → a handle");
    assert!(handle.rbac_active(), "the RBAC gate is on by default");
    assert!(
        handle.abac_active(),
        "abac.enabled must produce a live ABAC gate on the served handle"
    );
}

/// `abac.enabled = true` with an engine that cannot be built → boot ABORTS.
/// A configuration that promises fine-grained authorization must never
/// degrade to authz-off.
#[tokio::test]
async fn enabled_but_unbuildable_abac_fails_boot() {
    let cfg = AuthzConfig {
        abac: serde_json::from_str(
            r#"{"enabled": true, "engine": "remote",
                "remote": {"endpoint": {"url": "https://pdp.example/"}},
                "policy": {"nonsense": {"name": "p"}}}"#,
        )
        .expect("the ABAC table reads"),
        ..AuthzConfig::default()
    };
    let err = build_authz(
        &cfg,
        "/ferroehr/rest/openehr/v1",
        &registers(),
        inert_resolvers(),
    )
    .expect_err("an enabled ABAC block whose engine cannot be built must refuse to boot");
    assert!(
        format!("{err:#}").contains("nonsense"),
        "the boot error names what could not be built: {err:#}"
    );
}

/// The default config (RBAC on, ABAC off) keeps today's shape: a handle with
/// the RBAC gate only.
#[tokio::test]
async fn default_config_builds_an_rbac_only_handle() {
    let handle = build_authz(
        &AuthzConfig::default(),
        "/ferroehr/rest/openehr/v1",
        &registers(),
        inert_resolvers(),
    )
    .expect("the default block builds")
    .expect("RBAC is on by default");
    assert!(handle.rbac_active());
    assert!(!handle.abac_active(), "ABAC stays off unless enabled");
}
