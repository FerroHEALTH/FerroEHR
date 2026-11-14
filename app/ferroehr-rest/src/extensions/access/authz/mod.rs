// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authorization for the `FerroEHR` CDR.
//!
//! The coarse role gate inside the protocol adapter, authorization being an
//! adapter concern by design, and the per-server handle that carries it with
//! the patient scope.
//!
//! No openEHR spec governs this: ITS-REST `Requests_and_responses.md`
//! §Authentication and authorization mandates no scheme, so everything here is
//! our own enterprise design. The spec-grounded access authority is the
//! sibling `EHR_ACCESS` gate ([`crate::extensions::access::ehr_access`]) with
//! the access evaluator, which these layers compose on top of: RBAC, coarse
//! and always on when auth is enabled, classifies every generated ITS-REST
//! operation ([`classify`]) and gates it by a role model ([`roles`]) driven by
//! `ferroehr::config::authz::AuthzConfig`.
//!
//! This module also carries the per-server authorization handle wired onto
//! [`AppState`](crate::state::AppState), which the binary builds from
//! `ferroehr::config::authz::AuthzConfig` and the `[auth]` patient binding.
//!
//! ## Module map
//! - `config`: the `ferroehr::config::authz::AuthzConfig` serde struct (the
//!   `[authz]` section of the one server config tree) and boot validation.
//! - [`roles`]: the RBAC gate decision over the caller's roles.
//! - [`classify`]: `operation_id → OperationClass` / `ResourceKind` / `Mode`
//!   (total-coverage-guarded).

pub mod classify;
pub mod roles;

use crate::api::operation::{Operation, operation};
use crate::extensions::access::authz::classify::OperationClass;
use crate::extensions::access::authz::roles::{RbacDecision, authorize, authorize_readonly};
use crate::extensions::access::patient_scope::PatientScope;
use ferroehr::config::authz::{AuthzConfig, EhrAccessDefault, RbacConfig};

use std::collections::HashMap;

use http::Method;

/// The per-server authorization handle.
///
/// `None` on [`AppState`](crate::state::AppState) means authorization is off
/// (authentication-only behaviour). Carries the coarse RBAC gate and/or the
/// patient scope, plus the `[authz.rbac]` keys the per-EHR `EHR_ACCESS` gate
/// reads whether or not the RBAC gate is on.
#[derive(Debug)]
pub struct AuthzHandle {
    rbac: Option<RbacGate>,
    patient: Option<PatientScope>,
    /// The configured `[authz.rbac]` keys, kept whatever `rbac.enabled` says:
    /// `ehr_access_default` and `admin_role` are read by the per-EHR gate,
    /// which runs without the coarse gate.
    rules: RbacConfig,
}

impl AuthzHandle {
    /// Builds the full handle the binary serves with.
    ///
    /// The handle carries the RBAC gate when `rbac.enabled`, the patient
    /// scope when `patient` is `Some` (`[auth] patient_claim` is set), and the
    /// configured `[authz.rbac]` keys in every case. It is `None` only when
    /// there is nothing to enforce: the RBAC gate off, no patient scope and
    /// `rbac.ehr_access_default` open, so `AppState` stays empty (auth-only).
    /// A `restricted` default alone yields a handle: the per-EHR default
    /// applies with the coarse gate off.
    #[must_use]
    pub fn build(
        config: &AuthzConfig,
        base_path: &str,
        patient: Option<PatientScope>,
    ) -> Option<Self> {
        let rbac = config
            .rbac
            .enabled
            .then(|| RbacGate::new(config.rbac.clone(), base_path));
        if rbac.is_none()
            && patient.is_none()
            && config.rbac.ehr_access_default == EhrAccessDefault::Open
        {
            return None;
        }
        Some(Self {
            rbac,
            patient,
            rules: config.rbac.clone(),
        })
    }

    /// The coarse RBAC gate, if enabled.
    pub(crate) fn rbac(&self) -> Option<&RbacGate> {
        self.rbac.as_ref()
    }

    /// The RBAC rule set, when the coarse gate is live — the management
    /// surface's `AdminOnly` level gates on the same rules (issue #1879).
    pub(crate) fn rbac_rules(&self) -> Option<&RbacConfig> {
        self.rbac.as_ref().map(RbacGate::rules)
    }

    /// What an EHR carrying no `ACCESS_CONTROL_SETTINGS` admits, as configured
    /// in `[authz.rbac] ehr_access_default`, whether or not the RBAC gate is on.
    #[must_use]
    pub fn ehr_access_default(&self) -> EhrAccessDefault {
        self.rules.ehr_access_default
    }

    /// The role a `restricted` per-EHR default admits (`[authz.rbac]
    /// admin_role`), whether or not the RBAC gate is on.
    pub(crate) fn admin_role(&self) -> &str {
        &self.rules.admin_role
    }

    /// The patient scope, when `[auth] patient_claim` is set.
    #[must_use]
    pub fn patient(&self) -> Option<&PatientScope> {
        self.patient.as_ref()
    }

    /// Whether the coarse RBAC gate is live on this handle (boot logging +
    /// wiring tests).
    #[must_use]
    pub fn rbac_active(&self) -> bool {
        self.rbac.is_some()
    }

    /// Whether the patient scope is live on this handle (boot logging +
    /// wiring tests).
    #[must_use]
    pub fn patient_scope_active(&self) -> bool {
        self.patient.is_some()
    }
}

/// The coarse RBAC gate: a route-template → [`OperationClass`] map (built once
/// from the generated `ROUTES` tables) plus the role rules.
#[derive(Debug)]
pub(crate) struct RbacGate {
    rules: RbacConfig,
    /// `(method, full-path-template)` → operation, keyed by
    /// `base_path` + [`crate::api::normalize_path`] so a request's `MatchedPath`
    /// matches. Its row gives the class and the write class ([`Operation::is_write`]).
    routes: HashMap<(Method, String), &'static Operation>,
}

/// Extension routes whose HTTP verb MUTATES NOTHING — the read-only gate's
/// counterpart to the `Search` verb of [`Operation::is_write`], for the surfaces
/// outside the generated ITS-REST tables.
///
/// Matched by method + path SUFFIX so the deployment's configured base path is
/// irrelevant. Each entry is a POST whose handler provably performs no write —
/// either a read whose selector is a whole structure and so travels as a
/// request body (the released ad-hoc AQL read's shape), or a bodyless
/// computation over held data:
///
/// - `POST /message/export` — `export_ehr_extracts`, a query over held
///   versions whose selector is an `EXTRACT_SPEC` (RM `ehr_extract`
///   `extract_spec.adoc`). It commits nothing: the import operations are the
///   mutating half of the group.
/// - `POST /admin/integrity/verify` — the storage-parity sweep (#2680): a
///   whole-store comparison of the two stored content copies that writes
///   nothing and answers identifiers + defect classes only, strictly less
///   than the admin read routes already expose. A read-only integrity
///   auditor is exactly who runs it (adjudicated on #2692).
/// - `POST /admin/access-log/verify` — the access log's verification against
///   its checkpoint (#5129): it recomputes the chain and checks signatures,
///   answers findings only, and moves no record; the verification cursor it
///   advances is the one the scheduled chain check moves.
/// - `POST /query/cohort` — the cohort query: AQL execution over a population
///   selected in the demographic domain. It selects and never commits, exactly
///   as the released `POST /query/aql` does, and travels as a body because its
///   selector is a structure (the predicate list) rather than a path segment.
///
/// No openEHR spec governs role semantics — our own design/extension.
const EXTENSION_READ_ROUTES: &[(&str, &str)] = &[
    ("POST", "/message/export"),
    ("POST", "/admin/integrity/verify"),
    ("POST", "/admin/access-log/verify"),
    ("POST", "/query/cohort"),
];

/// Extension routes that are [`OperationClass::Admin`] despite not sitting under
/// `/admin/` — the destructive half of the shared-definition surface.
///
/// Each deletes a deployment-wide definition artefact every EHR validates
/// against, and the deletion is physical: this store has no logical tier to
/// undelete from.
///
/// The class is OUR OWN DESIGN, on the blast radius. ITS-REST
/// `Requests_and_responses.md` §Authentication and authorization "does not
/// mandate a specific authentication scheme", so no released clause requires
/// or forbids this, and the uploads
/// staying Clinical is a deliberate asymmetry: an upload is additive and
/// reversible, a delete is neither.
///
/// Matched by method + path SUFFIX so the configured base path is irrelevant.
const EXTENSION_ADMIN_ROUTES: &[(&str, &str)] = &[
    ("DELETE", "/definition/archetype/adl1.4/{archetype_id}"),
    ("DELETE", "/definition/artefact/adl2/{artefact_id}"),
];

/// Whether an EXTENSION route (one outside the generated ITS-REST tables) is a
/// write for the read-only gate: the mutating verbs are, except for the pinned
/// [`EXTENSION_READ_ROUTES`] reads. Fail-safe by construction — an unlisted
/// mutating verb stays a write.
fn extension_is_write(method: &Method, matched: &str) -> bool {
    if EXTENSION_READ_ROUTES
        .iter()
        .any(|(verb, suffix)| method.as_str() == *verb && matched.ends_with(suffix))
    {
        return false;
    }
    !matches!(*method, Method::GET | Method::HEAD)
}

impl RbacGate {
    /// The configured rule set (role names + switches).
    pub(crate) fn rules(&self) -> &RbacConfig {
        &self.rules
    }

    fn new(rules: RbacConfig, base_path: &str) -> Self {
        let mut routes = HashMap::new();
        for table in [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
            openehr_its::rest::generated::system::ROUTES,
        ] {
            for (method, path, op) in table {
                let (Ok(method), Some(row)) = (method.parse::<Method>(), operation(op)) else {
                    // The coverage guard proves every generated op parses and has a class.
                    continue;
                };
                let full = format!("{base_path}{}", crate::api::normalize_path(path));
                routes.insert((method, full), row);
            }
        }
        Self { rules, routes }
    }

    /// The operation class for a request. `matched` is the axum `MatchedPath`
    /// template (the full nested path). A route not in the map defaults to
    /// [`OperationClass::Admin`] when its template is under `/admin/` (fail
    /// safe), else [`OperationClass::Clinical`]; a request with no matched path
    /// (a 404) is [`OperationClass::Public`] so routing still yields the 404.
    pub(crate) fn class_for(&self, method: &Method, matched: Option<&str>) -> OperationClass {
        match matched {
            None => OperationClass::Public,
            Some(mp) => self
                .routes
                .get(&(method.clone(), mp.to_owned()))
                .and_then(|row| row.class)
                .unwrap_or_else(|| {
                    let admin = mp.contains("/admin/")
                        || EXTENSION_ADMIN_ROUTES
                            .iter()
                            .any(|(verb, suffix)| method.as_str() == *verb && mp.ends_with(suffix));
                    if admin {
                        OperationClass::Admin
                    } else {
                        OperationClass::Clinical
                    }
                }),
        }
    }

    /// Whether the matched request is a **write** operation, for the read-only
    /// gate. A mapped route classifies via its operation ([`Operation::is_write`]); an
    /// unmapped route (an extension surface outside the generated tables) falls
    /// back to its HTTP method — only `GET`/`HEAD` are reads, every mutating verb
    /// is a write so the read-only restriction cannot be bypassed there. A
    /// request with no matched path (a 404) is not a write to gate.
    pub(crate) fn is_write_for(&self, method: &Method, matched: Option<&str>) -> bool {
        match matched {
            None => false,
            Some(mp) => self
                .routes
                .get(&(method.clone(), mp.to_owned()))
                .map_or_else(|| extension_is_write(method, mp), |row| row.is_write()),
        }
    }

    /// Apply the RBAC decision for a class + the caller's roles.
    pub(crate) fn decide(&self, class: OperationClass, principal_roles: &[String]) -> RbacDecision {
        authorize(class, principal_roles, &self.rules)
    }

    /// Apply the read-only restriction for a write flag + the caller's roles: a
    /// principal carrying the configured read-only role is refused on writes.
    pub(crate) fn decide_readonly(
        &self,
        is_write: bool,
        principal_roles: &[String],
    ) -> RbacDecision {
        authorize_readonly(is_write, principal_roles, &self.rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "/ferroehr/rest/openehr/v1";

    fn gate() -> RbacGate {
        RbacGate::new(RbacConfig::default(), BASE)
    }

    /// The read-only gate classifies an EXTENSION route by what the operation
    /// DOES, not by its HTTP verb: `POST /message/export` is
    /// `export_ehr_extracts`, a query over held versions, while the import
    /// routes of the same group commit and stay writes.
    #[test]
    fn message_export_is_a_read_for_the_readonly_gate() {
        let g = gate();
        assert!(
            !g.is_write_for(&Method::POST, Some(&format!("{BASE}/message/export"))),
            "export_ehr_extracts selects held content and commits nothing"
        );
        assert!(
            g.is_write_for(&Method::POST, Some(&format!("{BASE}/message/import"))),
            "the imports are the mutating half of the same interface"
        );
        assert!(
            g.is_write_for(&Method::POST, Some(&format!("{BASE}/message/unlisted"))),
            "an unlisted extension POST stays a write (fail-safe)"
        );
        assert!(
            !g.is_write_for(&Method::GET, Some(&format!("{BASE}/message/export"))),
            "a GET is a read on every surface"
        );
    }

    #[test]
    fn admin_template_is_admin_class() {
        let g = gate();
        assert_eq!(
            g.class_for(
                &Method::DELETE,
                Some(&format!("{BASE}/admin/ehr/{{ehr_id}}"))
            ),
            OperationClass::Admin
        );
    }

    #[test]
    fn clinical_template_is_clinical_class() {
        let g = gate();
        assert_eq!(
            g.class_for(
                &Method::POST,
                Some(&format!("{BASE}/ehr/{{ehr_id}}/composition"))
            ),
            OperationClass::Clinical
        );
        assert_eq!(
            g.class_for(&Method::GET, Some(&format!("{BASE}/ehr/{{ehr_id}}"))),
            OperationClass::Clinical
        );
    }

    #[test]
    fn unmatched_is_public_but_admin_substring_fails_safe() {
        let g = gate();
        // A genuine 404 (no MatchedPath) is not gated.
        assert_eq!(g.class_for(&Method::GET, None), OperationClass::Public);
        // A matched-but-unmapped admin-looking template still gates as Admin.
        assert_eq!(
            g.class_for(&Method::GET, Some("/x/admin/unknown")),
            OperationClass::Admin
        );
    }

    #[test]
    fn decide_delegates_to_rbac() {
        let g = gate();
        assert_eq!(
            g.decide(OperationClass::Admin, &["USER".to_owned()]),
            RbacDecision::Deny("operation requires the 'ADMIN' role".to_owned())
        );
        assert_eq!(
            g.decide(OperationClass::Clinical, &["USER".to_owned()]),
            RbacDecision::Allow
        );
    }

    #[test]
    fn is_write_for_maps_generated_ops() {
        let g = gate();
        // Writes on the clinical + definition surface.
        assert!(g.is_write_for(&Method::POST, Some(&format!("{BASE}/ehr"))));
        assert!(g.is_write_for(
            &Method::POST,
            Some(&format!("{BASE}/ehr/{{ehr_id}}/composition"))
        ));
        assert!(g.is_write_for(
            &Method::POST,
            Some(&format!("{BASE}/definition/template/adl1.4"))
        ));
        // Reads: a GET, and AQL execution (POST-but-read).
        assert!(!g.is_write_for(&Method::GET, Some(&format!("{BASE}/ehr/{{ehr_id}}"))));
        assert!(!g.is_write_for(&Method::POST, Some(&format!("{BASE}/query/aql"))));
        // Unmapped route: method-based fail-safe.
        assert!(g.is_write_for(&Method::POST, Some("/x/extension/unknown")));
        assert!(!g.is_write_for(&Method::GET, Some("/x/extension/unknown")));
        // A 404 (no matched path) is not a write to gate.
        assert!(!g.is_write_for(&Method::POST, None));
    }

    #[test]
    fn decide_readonly_denies_writes() {
        let g = gate();
        assert!(matches!(
            g.decide_readonly(true, &["READONLY".to_owned()]),
            RbacDecision::Deny(_)
        ));
        assert_eq!(
            g.decide_readonly(false, &["READONLY".to_owned()]),
            RbacDecision::Allow
        );
        assert_eq!(
            g.decide_readonly(true, &["USER".to_owned()]),
            RbacDecision::Allow
        );
    }

    #[test]
    fn handle_absent_when_rbac_disabled() {
        let mut cfg = AuthzConfig::default();
        cfg.rbac.enabled = false;
        let handle = AuthzHandle::build(&cfg, BASE, None);
        assert!(handle.is_none());
        let default =
            AuthzHandle::build(&AuthzConfig::default(), BASE, None).expect("RBAC on by default");
        assert!(default.rbac_active());
        assert!(
            !default.patient_scope_active(),
            "no patient claim configured → no patient scope on the handle"
        );
    }

    /// A `restricted` per-EHR default is carried with the RBAC gate off, with
    /// the configured admin role, so the per-EHR gate applies it instead of
    /// falling back to open.
    #[test]
    fn restricted_default_survives_rbac_disabled() {
        let mut cfg = AuthzConfig::default();
        cfg.rbac.enabled = false;
        cfg.rbac.ehr_access_default = EhrAccessDefault::Restricted;
        cfg.rbac.admin_role = "SUPERADMIN".to_owned();
        let handle =
            AuthzHandle::build(&cfg, BASE, None).expect("a restricted default needs a handle");
        assert!(!handle.rbac_active(), "the coarse gate stays off");
        assert!(!handle.patient_scope_active());
        assert_eq!(handle.ehr_access_default(), EhrAccessDefault::Restricted);
        assert_eq!(handle.admin_role(), "SUPERADMIN");
    }

    /// A configured patient scope alone yields a handle, with the coarse gate
    /// off and the per-EHR default open.
    #[test]
    fn a_patient_scope_alone_yields_a_handle() {
        let mut cfg = AuthzConfig::default();
        cfg.rbac.enabled = false;
        let scope = PatientScope::new(
            "patient_id",
            None,
            std::sync::Arc::new(|_| Box::pin(async { Ok(None) })),
        );
        let handle =
            AuthzHandle::build(&cfg, BASE, Some(scope)).expect("a patient scope needs a handle");
        assert!(!handle.rbac_active());
        assert!(handle.patient_scope_active());
        assert_eq!(
            handle.patient().map(PatientScope::claim),
            Some("patient_id")
        );
    }
}
