// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binary's authorization construction seam
//! ([`ferroehr_server::build_authz`]): the shipped run path builds the RBAC
//! gate AND, when `[auth] patient_claim` is set, the patient scope (the
//! mis-wiring #1815 caught: the binary once constructed an RBAC-only handle
//! unconditionally).
//!
//! Offline by design, like the rest of this crate's suite: handle
//! CONSTRUCTION touches no database — the subject lookup is an inert stub
//! here, and the DB-backed enforcement path is proven end to end where it
//! lives (`app/ferroehr-rest/tests/it/patient_scope_e2e.rs`). The patient
//! scope is our own extension — no openEHR spec governs it (ITS-REST places
//! authorization out of band).

use std::sync::Arc;

use ferroehr::config::FerroEhrConfig;
use ferroehr_rest::extensions::access::patient_scope::{PatientScope, ResolveError, SubjectFn};

use ferroehr_server::build_authz;

/// An inert subject lookup: construction under test, resolution out of scope.
fn inert_lookup() -> SubjectFn {
    Arc::new(|_| Box::pin(async { Ok::<_, ResolveError>(None) }))
}

/// `[auth] patient_claim` set → the handle the binary serves with carries a
/// LIVE patient scope on the configured claim (the regression assertion).
#[test]
fn a_patient_claim_makes_the_scope_live_on_the_built_handle() {
    let mut config = FerroEhrConfig::default();
    config.auth.patient_claim = Some("patient_id".to_owned());
    let handle = build_authz(&config, inert_lookup()).expect("RBAC default-on → a handle");
    assert!(handle.rbac_active(), "the RBAC gate is on by default");
    assert!(
        handle.patient_scope_active(),
        "a patient claim must produce a live patient scope on the served handle"
    );
    assert_eq!(
        handle.patient().map(PatientScope::claim),
        Some("patient_id")
    );
}

/// The default config (RBAC on, no patient claim) builds a handle with the
/// RBAC gate only.
#[test]
fn default_config_builds_an_rbac_only_handle() {
    let handle =
        build_authz(&FerroEhrConfig::default(), inert_lookup()).expect("RBAC is on by default");
    assert!(handle.rbac_active());
    assert!(
        !handle.patient_scope_active(),
        "no patient scope unless a claim is configured"
    );
}

/// A blank claim is no claim: no patient scope.
#[test]
fn a_blank_patient_claim_builds_no_scope() {
    let mut config = FerroEhrConfig::default();
    config.auth.patient_claim = Some("  ".to_owned());
    let handle = build_authz(&config, inert_lookup()).expect("RBAC is on by default");
    assert!(!handle.patient_scope_active());
}
