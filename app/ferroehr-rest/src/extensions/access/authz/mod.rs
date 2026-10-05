// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authorization for the `FerroEHR` CDR.
//!
//! Two composable layers inside the protocol adapter, authorization being an
//! adapter concern by design.
//!
//! No openEHR spec governs this: the SM places authorization out of band (SM
//! `openehr_platform/master02-overview.adoc` §General Assumptions), so
//! everything here is our own enterprise design. The spec-grounded access
//! authority is the sibling `EHR_ACCESS` gate
//! ([`crate::extensions::access::ehr_access`]), which these layers compose on
//! top of:
//!
//! 1. RBAC, coarse and always on when auth is enabled: every generated ITS-REST
//!    operation is classified ([`classify`]) and gated by a role model
//!    ([`roles`]) driven by `ferroehr::config::authz::AuthzConfig`.
//! 2. ABAC, fine-grained and opt-in: a policy-decision-point seam
//!    ([`engine::PolicyEngine`]) consulted per clinical operation with resolved
//!    attributes, behind two interchangeable engines — the embedded [`cedar`]
//!    default and the [`remote::RemotePdp`].
//!
//! This module also carries the per-server authorization handle wired onto
//! [`AppState`](crate::state::AppState), which the binary builds from
//! `ferroehr::config::authz::AuthzConfig`.
//!
//! ## Module map
//! - `config` — the `ferroehr::config::authz::AuthzConfig` serde struct (the `[authz]`
//!   section of the one server config tree) + boot validation.
//! - [`roles`] — the role model, JWT-claim role extraction, and the RBAC gate
//!   decision.
//! - [`classify`] — `operation_id → OperationClass` / `ResourceKind` /
//!   `AccessMode` (total-coverage-guarded).
//! - [`request`] — the ABAC [`request::AuthzRequest`] + multi-valued fan-out.
//! - [`engine`] — the [`engine::PolicyEngine`] PDP seam + [`engine::AuthzError`].
//! - [`cedar`] — the embedded Cedar engine (schema, policy loading, reload).
//! - [`remote`] — the remote PDP client.

pub mod cedar;
pub mod classify;
pub mod engine;
pub mod remote;
pub mod request;
pub mod roles;

use crate::extensions::access::authz::classify::{OperationClass, class_of, is_write};
use crate::extensions::access::authz::engine::{AuthzError, PolicyEngine};
use crate::extensions::access::authz::roles::{RbacDecision, authorize, authorize_readonly};
use ferroehr::config::authz::{AbacConfig, AbacEngineKind, AuthzConfig, RbacConfig};

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::Method;

use self::cedar::CedarEngine;
use self::remote::RemotePdp;

/// An attribute-resolution failure (a DB lookup error). Fail-closed at the
/// PEP: a resolution failure denies (→ 403/500), never silently permits.
#[derive(Debug, thiserror::Error)]
#[error("attribute resolution failed: {context}")]
pub struct ResolveError {
    /// What was being resolved when the lookup failed.
    context: String,
    /// The lookup failure itself — an `sqlx` driver error, an identifier
    /// refusal — reachable through [`std::error::Error::source`].
    ///
    /// Kept out of the message: the PEP renders a resolution failure as a
    /// `500`, whose body must disclose no internal error value (OWASP REST
    /// Security Cheat Sheet §Error handling). The chain goes to the trace
    /// record instead.
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl ResolveError {
    /// A resolution failure: what was being resolved, and the error that broke
    /// it.
    #[must_use]
    pub fn new(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            context: context.into(),
            source: Box::new(source),
        }
    }
}

/// The future a resolver closure returns.
pub type ResolverFuture<T> = Pin<Box<dyn Future<Output = Result<T, ResolveError>> + Send>>;

/// `ehr_id → EHR subject external-ref id` (the promoted `ehr.subject_id`
/// column; the audit `SubjectResolver` query). Ids are passed as strings so the
/// REST layer stays `uuid`-free (the binary parses).
pub type SubjectFn = Arc<dyn Fn(String) -> ResolverFuture<Option<String>> + Send + Sync>;

/// `(vo_id, version) → template_id` (`version.template_id`, read back via
/// `ferroehr::service` in the binary).
///
/// `version` is the `VERSION_TREE_ID` lexical form (`N` or `N.B.V` — trunk or
/// branch, RM common master06 §The 'Virtual Version Tree'); `None` = the
/// current version.
pub type TemplateOfVersionFn =
    Arc<dyn Fn(String, Option<String>) -> ResolverFuture<Option<String>> + Send + Sync>;

/// The DB-backed attribute resolvers the ABAC PEP calls.
///
/// Defined here so the REST layer can hold them; the closures are built in
/// the binary (which owns the pool + service), the audit-`SubjectResolver`
/// precedent.
#[derive(Clone)]
pub struct AuthzResolvers {
    /// EHR subject external-ref id lookup.
    pub subject: SubjectFn,
    /// COMPOSITION version → template id lookup.
    pub template_of_version: TemplateOfVersionFn,
}

impl std::fmt::Debug for AuthzResolvers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthzResolvers").finish_non_exhaustive()
    }
}

/// Build the configured ABAC [`PolicyEngine`], or `None` when ABAC
/// is disabled. Called by the binary at boot after [`AuthzConfig::validate`].
///
/// # Errors
/// [`AuthzError`] if the Cedar schema/policies fail to load/validate, or the
/// remote-PDP client cannot be built.
pub fn build_engine(config: &AbacConfig) -> Result<Option<Arc<dyn PolicyEngine>>, AuthzError> {
    if !config.enabled {
        return Ok(None);
    }
    let engine: Arc<dyn PolicyEngine> = match config.engine {
        AbacEngineKind::Cedar => {
            let dir = config.cedar.policy_dir.as_deref().ok_or_else(|| {
                AuthzError::PolicyLoad("abac.cedar.policy_dir is not set".to_owned())
            })?;
            Arc::new(CedarEngine::new(dir, config.cedar.reload_secs)?)
        }
        AbacEngineKind::Remote => Arc::new(RemotePdp::new(config)?),
    };
    Ok(Some(engine))
}

/// The per-server authorization handle.
///
/// `None` on [`AppState`](crate::state::AppState) means authorization is off
/// (authentication-only behaviour). Carries the coarse RBAC gate and/or the
/// fine-grained ABAC gate.
#[derive(Debug)]
pub struct AuthzHandle {
    rbac: Option<RbacGate>,
    abac: Option<AbacGate>,
}

impl AuthzHandle {
    /// Build the full handle (the binary): the RBAC gate (when enabled) plus the
    /// ABAC gate (when `engine` is `Some`, i.e. `abac.enabled`). `None` when
    /// neither layer is active, so `AppState` stays empty (auth-only).
    #[must_use]
    pub fn build(
        config: &AuthzConfig,
        base_path: &str,
        engine: Option<Arc<dyn PolicyEngine>>,
        resolvers: AuthzResolvers,
    ) -> Option<Self> {
        let rbac = config
            .rbac
            .enabled
            .then(|| RbacGate::new(config.rbac.clone(), base_path));
        let abac = engine.map(|engine| AbacGate::new(&config.abac, engine, resolvers));
        if rbac.is_none() && abac.is_none() {
            return None;
        }
        Some(Self { rbac, abac })
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

    /// The fine-grained ABAC gate, if enabled.
    pub(crate) fn abac(&self) -> Option<&AbacGate> {
        self.abac.as_ref()
    }

    /// Whether the coarse RBAC gate is live on this handle (boot logging +
    /// wiring tests).
    #[must_use]
    pub fn rbac_active(&self) -> bool {
        self.rbac.is_some()
    }

    /// Whether the fine-grained ABAC gate is live on this handle — the loud
    /// counterpart of the silent `abac: None` mis-wiring this seam once
    /// carried (boot logging + wiring tests).
    #[must_use]
    pub fn abac_active(&self) -> bool {
        self.abac.is_some()
    }

    /// The configured JWT role-claim paths (fed into the [`Authenticator`] so
    /// Bearer role extraction uses them); defaults when RBAC is off.
    ///
    /// [`Authenticator`]: crate::extensions::access::authn::Authenticator
    #[must_use]
    pub fn role_claims(&self) -> Vec<String> {
        self.rbac.as_ref().map_or_else(
            || RbacConfig::default().role_claims,
            |r| r.rules.role_claims.clone(),
        )
    }
}

/// The fine-grained ABAC gate: the PDP engine, the DB-backed attribute
/// resolvers, and the resolved claim names + directory opt-in. The PEP
/// ([`crate::extensions::access::pep`]) drives it.
pub(crate) struct AbacGate {
    pub(crate) engine: Arc<dyn PolicyEngine>,
    pub(crate) resolvers: AuthzResolvers,
    /// The JWT claim carrying the caller's organization (blank → unused).
    pub(crate) organization_claim: Option<String>,
    /// The JWT claim carrying the patient id; its presence enables the subject
    /// gate.
    pub(crate) patient_claim: Option<String>,
    /// Whether DIRECTORY (FOLDER) operations reach the PDP — the
    /// `abac.check_directory` switch, so the opt-in is engine-independent
    /// rather than inferred from a remote-PDP-shaped policy map.
    pub(crate) directory_checked: bool,
}

impl std::fmt::Debug for AbacGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbacGate")
            .field("organization_claim", &self.organization_claim)
            .field("patient_claim", &self.patient_claim)
            .field("directory_checked", &self.directory_checked)
            .finish_non_exhaustive()
    }
}

impl AbacGate {
    fn new(config: &AbacConfig, engine: Arc<dyn PolicyEngine>, resolvers: AuthzResolvers) -> Self {
        Self {
            engine,
            resolvers,
            organization_claim: config.organization_claim().map(str::to_owned),
            patient_claim: config.patient_claim().map(str::to_owned),
            directory_checked: config.check_directory,
        }
    }
}

/// The coarse RBAC gate: a route-template → [`OperationClass`] map (built once
/// from the generated `ROUTES` tables) plus the role rules.
#[derive(Debug)]
pub(crate) struct RbacGate {
    rules: RbacConfig,
    /// `(method, full-path-template)` → (class, operation id), keyed by
    /// `base_path` + [`crate::api::normalize_path`] so a request's `MatchedPath`
    /// matches. The op id feeds the read-only write classifier
    /// ([`classify::is_write`]).
    routes: HashMap<(Method, String), (OperationClass, &'static str)>,
}

/// Extension routes whose HTTP verb MUTATES NOTHING — the read-only gate's
/// counterpart to [`is_write`]'s `query_execute_*` arm, for the surfaces
/// outside the generated ITS-REST tables.
///
/// Matched by method + path SUFFIX so the deployment's configured base path is
/// irrelevant. Each entry is a POST whose handler provably performs no write —
/// either a read whose selector is a whole structure and so travels as a
/// request body (the released ad-hoc AQL read's shape), or a bodyless
/// computation over held data:
///
/// - `POST /message/export` — SM `I_EHR_EXTRACT_SERVICE.export_ehr_extracts`
///   (`SM/docs/UML/classes/i_ehr_extract_service.adoc`), a query over held
///   versions whose selector is an `EXTRACT_SPEC` (RM `ehr_extract`
///   `extract_spec.adoc`). It commits nothing: the import operations are the
///   mutating half of that interface.
/// - `POST /admin/integrity/verify` — the storage-parity sweep (#2680): a
///   whole-store comparison of the two stored content copies that writes
///   nothing and answers identifiers + defect classes only, strictly less
///   than the admin read routes already expose. A read-only integrity
///   auditor is exactly who runs it (adjudicated on #2692).
/// - `POST /query/cohort` — the cohort query: AQL execution over a population
///   selected in the demographic domain. It selects and never commits, exactly
///   as the released `POST /query/aql` does, and travels as a body because its
///   selector is a structure (the predicate list) rather than a path segment.
///
/// No openEHR spec governs role semantics — our own design/extension.
const EXTENSION_READ_ROUTES: &[(&str, &str)] = &[
    ("POST", "/message/export"),
    ("POST", "/admin/integrity/verify"),
    ("POST", "/query/cohort"),
];

/// Extension routes that are [`OperationClass::Admin`] despite not sitting under
/// `/admin/` — the destructive half of the shared-definition surface.
///
/// Each deletes a deployment-wide definition artefact every EHR validates
/// against, and the deletion is physical: this store has no logical tier to
/// undelete from.
///
/// The class is OUR OWN DESIGN, on the blast radius, and the specification
/// explicitly leaves it to us rather than being silent on it. SM
/// `master02-overview.adoc` §Global Conventions → §Functional Style lists
/// "approach to access control and authorisation" among the implementation
/// choices and states that "Authentication and authorisation is assumed to have
/// been dealt with before any particular call has been made … and role-based
/// access control"; ITS-REST `Requests_and_responses.md` §Authentication and
/// authorization adds that it "does not mandate a specific authentication
/// scheme". So no released clause requires or forbids this, and the uploads
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
                let (Ok(method), Some(class)) = (method.parse::<Method>(), class_of(op)) else {
                    // An unparsable method or unclassified op is impossible for the
                    // generated tables (the coverage guard proves classification);
                    // skip defensively rather than panic in a constructor.
                    continue;
                };
                let full = format!("{base_path}{}", crate::api::normalize_path(path));
                routes.insert((method, full), (class, *op));
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
                .map_or_else(
                    || {
                        let admin = mp.contains("/admin/")
                            || EXTENSION_ADMIN_ROUTES.iter().any(|(verb, suffix)| {
                                method.as_str() == *verb && mp.ends_with(suffix)
                            });
                        if admin {
                            OperationClass::Admin
                        } else {
                            OperationClass::Clinical
                        }
                    },
                    |(class, _op)| *class,
                ),
        }
    }

    /// Whether the matched request is a **write** operation, for the read-only
    /// gate. A mapped route classifies via its op id ([`classify::is_write`]); an
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
                .map_or_else(
                    || extension_is_write(method, mp),
                    |(_class, op)| is_write(op),
                ),
        }
    }

    /// Apply the RBAC decision for a class + the caller's roles.
    pub(crate) fn decide(&self, class: OperationClass, principal_roles: &[String]) -> RbacDecision {
        authorize(class, principal_roles, &self.rules)
    }

    /// Whether the caller holds the configured subject-scoped audit role
    /// (`authz.rbac.subject_audit_role`), which reads the access log for one
    /// subject at a time (#3240). `false` when the key is unset. Role matching
    /// is ASCII-case-insensitive, like every other role here. Consulted by the
    /// ITI-81 retrieval, which lives in the `fhir` feature.
    #[cfg(feature = "fhir")]
    pub(crate) fn holds_subject_audit_role(&self, principal_roles: &[String]) -> bool {
        self.rules
            .subject_audit_role
            .as_deref()
            .is_some_and(|role| {
                principal_roles
                    .iter()
                    .any(|held| held.eq_ignore_ascii_case(role))
            })
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
    /// DOES, not by its HTTP verb: `POST /message/export` realizes SM
    /// `I_EHR_EXTRACT_SERVICE.export_ehr_extracts`, a query over held versions,
    /// while the import routes of the same interface commit and stay writes.
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
            g.is_write_for(&Method::POST, Some(&format!("{BASE}/message/tdd"))),
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

    /// Inert resolvers for handle-shape tests (nothing resolves).
    fn inert_resolvers() -> AuthzResolvers {
        AuthzResolvers {
            subject: Arc::new(|_| Box::pin(async { Ok::<_, ResolveError>(None) })),
            template_of_version: Arc::new(|_, _| Box::pin(async { Ok::<_, ResolveError>(None) })),
        }
    }

    #[test]
    fn handle_absent_when_rbac_disabled() {
        let mut cfg = AuthzConfig::default();
        cfg.rbac.enabled = false;
        let handle = AuthzHandle::build(&cfg, BASE, None, inert_resolvers());
        assert!(handle.is_none());
        let default = AuthzHandle::build(&AuthzConfig::default(), BASE, None, inert_resolvers())
            .expect("RBAC on by default");
        assert!(default.rbac_active());
        assert!(
            !default.abac_active(),
            "no engine supplied → no ABAC gate on the handle"
        );
    }

    /// DIRECTORY gating reads `abac.check_directory`, so it works identically
    /// under both engines and is not inferred from the remote-PDP policy map (a
    /// Cedar deployment would otherwise need a policy name its engine never
    /// reads). No openEHR spec governs authorization — our own design.
    #[test]
    fn directory_checked_is_engine_independent() {
        let engine: Arc<dyn PolicyEngine> = Arc::new(InertEngine);

        for kind in [AbacEngineKind::Cedar, AbacEngineKind::Remote] {
            let off = AbacConfig {
                enabled: true,
                engine: kind,
                ..AbacConfig::default()
            };
            let gate = AbacGate::new(&off, Arc::clone(&engine), inert_resolvers());
            assert!(
                !gate.directory_checked,
                "{kind:?}: DIRECTORY gating is off by default"
            );

            let on = AbacConfig {
                check_directory: true,
                ..off
            };
            let gate = AbacGate::new(&on, Arc::clone(&engine), inert_resolvers());
            assert!(
                gate.directory_checked,
                "{kind:?}: check_directory alone enables DIRECTORY gating"
            );
        }

        // A `directory` entry in the remote-PDP policy map no longer enables it.
        let mut policy = std::collections::BTreeMap::new();
        policy.insert(
            "directory".to_owned(),
            ferroehr::config::authz::PolicyRule {
                name: "directory-access".to_owned(),
                parameters: vec![],
            },
        );
        let map_only = AbacConfig {
            enabled: true,
            policy,
            ..AbacConfig::default()
        };
        let gate = AbacGate::new(&map_only, engine, inert_resolvers());
        assert!(
            !gate.directory_checked,
            "the wire policy map is not the opt-in switch"
        );
    }

    /// A PDP stand-in for gate-shape tests (never consulted).
    #[derive(Debug)]
    struct InertEngine;

    #[async_trait::async_trait]
    impl PolicyEngine for InertEngine {
        async fn decide(
            &self,
            _req: &request::AuthzRequest<'_>,
        ) -> Result<request::Decision, AuthzError> {
            Ok(request::Decision::Permit)
        }
    }

    #[test]
    fn build_engine_none_when_abac_disabled() {
        // ABAC off (the default) → no engine, current behaviour preserved.
        let engine = build_engine(&AbacConfig::default()).expect("build");
        assert!(engine.is_none());
    }

    #[tokio::test]
    async fn resolvers_invoke_their_closures() {
        // The AuthzResolvers type carries async DB closures; exercise the shape
        // with in-memory stand-ins (the binary supplies pool-backed ones).
        let resolvers = AuthzResolvers {
            subject: Arc::new(|ehr_id: String| {
                Box::pin(async move { Ok((ehr_id == "known").then(|| "subject-1".to_owned())) })
            }),
            template_of_version: Arc::new(|_vo: String, version: Option<String>| {
                Box::pin(async move { Ok(version.map(|v| format!("t.v{v}"))) })
            }),
        };
        assert_eq!(
            (resolvers.subject)("known".to_owned()).await.unwrap(),
            Some("subject-1".to_owned())
        );
        assert_eq!((resolvers.subject)("other".to_owned()).await.unwrap(), None);
        assert_eq!(
            (resolvers.template_of_version)("vo".to_owned(), Some("3".to_owned()))
                .await
                .unwrap(),
            Some("t.v3".to_owned())
        );
    }
}
