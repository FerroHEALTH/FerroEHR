// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The exhaustive **role × route** authorization matrix over the assembled
//! router.
//!
//! `rbac_e2e` proves the gate's mechanics on one representative operation per
//! class. This module proves the CLASSIFICATION of every route: the route set
//! is enumerated from the router itself (`extensions_document`, built from the
//! same `utoipa-axum` composition that mounts the routes, so it cannot drift
//! from what is served) and matched against the explicit tables below. A route
//! added to the router and left out of a table fails
//! [`every_mounted_route_carries_a_declared_authorization_class`] — which is
//! the point: `RbacGate::class_for` classifies an unmapped template `Admin`
//! only under `/admin/` and `Clinical` everywhere else, so an admin-grade route
//! mounted outside that prefix would otherwise be gated as clinical with
//! nothing to notice.
//!
//! The decision matrix itself is asserted on the wire: every gated route is
//! driven with a roleless, an ordinary, an admin, and a read-only principal,
//! and the refusal is identified by the reason the gate renders
//! (`extensions::access::authz::roles`), so a `403` from any OTHER gate is
//! never mistaken for an RBAC decision.
//!
//! The shape follows the OWASP **Authorization Testing Automation** Cheat
//! Sheet's data-driven matrix
//! (<https://cheatsheetseries.owasp.org/cheatsheets/Authorization_Testing_Automation_Cheat_Sheet.html>).
//! No openEHR spec governs authorization — the SM places it out of band
//! (`docs/specs/openehr/SM/docs/openehr_platform/master02-overview.adoc`
//! §General Assumptions) — so every expectation here is our own design, except
//! the 401-vs-403 split (ITS-REST overview `Requests_and_responses.md`).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::config::management::AccessLevel;
use ferroehr::config::server::{AdminConfig, ServerConfig};
use ferroehr::config::smart::SmartConfig;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common;
use crate::common::BASE;

const HMAC_SECRET: &str = "route-matrix-secret";
const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "ferroehr";

/// The one value substituted for every `{param}` capture: the gate keys on the
/// route TEMPLATE (axum's `MatchedPath`), so a probe value only has to route.
const PROBE_PARAM: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

// ── the declared classification ───────────────────────────────────────────────

/// The authorization class a route is declared to carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// Served outside the authentication layer — no credential, no gate.
    Public,
    /// Any authenticated principal holding at least one role.
    Clinical,
    /// Requires the configured admin role (`rbac.admin_role`).
    Admin,
    /// Coarse class `Clinical`, admin enforced inside the handler: the class
    /// gate refuses a roleless caller first, the handler refuses a non-admin.
    HandlerAdmin,
    /// The management surface, gated by its own per-endpoint router
    /// (`[management.endpoints].<name>`) rather than by `RbacGate::class_for`.
    Management,
}

/// Whether a route MUTATES stored state — the axis the read-only role
/// restricts (`roles::authorize_readonly`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    Read,
    Write,
}

/// Ungated surfaces, mounted OUTSIDE the authentication layer (absolute paths):
/// the always-on health family, the operational status document, SMART
/// discovery, the OAS/Swagger pair (mounted only when `server.swagger_ui` is
/// on) and the System `OPTIONS` manifest, which the router mounts on the outer
/// router above the CORS layer and therefore above authentication too.
///
/// This is the inventory of everything reachable without a credential (#3671):
/// each row names why it is public, and
/// [`every_route_outside_the_public_inventory_needs_a_credential`] proves that
/// nothing else is.
const PUBLIC: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/health",
        "orchestrator probe: a kubelet probe carries no credential",
    ),
    (
        "GET",
        "/health/liveness",
        "orchestrator probe: a kubelet probe carries no credential",
    ),
    (
        "GET",
        "/health/readiness",
        "orchestrator probe: a kubelet probe carries no credential",
    ),
    (
        "GET",
        "/ferroehr/rest/status",
        "documented client contract: status, versions and profile only",
    ),
    (
        "GET",
        "/ferroehr/rest/.well-known/smart-configuration",
        "SMART master04 Service Discovery: always available, read before any token exists",
    ),
    (
        "GET",
        "/ferroehr/rest/api-docs/openapi.json",
        "only when server.swagger_ui = public; the default is private",
    ),
    (
        "GET",
        "/ferroehr/rest/api-docs/ferroehr-{family}.openapi.json",
        "only when server.swagger_ui = public; the default is private",
    ),
    (
        "GET",
        "/ferroehr/rest/swagger-ui",
        "only when server.swagger_ui = public; the default is private",
    ),
    (
        "OPTIONS",
        "/ferroehr/rest/openehr/v1",
        "ITS-REST System API: its released OAS declares `security: []`",
    ),
];

/// The ops-introspection surface (absolute paths), gated by its own router
/// (`extensions::management`) against the per-endpoint `[management]` levels.
const MANAGEMENT: &[(&str, &str)] = &[
    ("GET", "/management/info"),
    ("GET", "/management/env"),
    ("GET", "/management/prometheus"),
    ("GET", "/management/metrics"),
    ("GET", "/management/metrics/{name}"),
    ("GET", "/management/loggers"),
    ("POST", "/management/loggers"),
    ("DELETE", "/management/loggers"),
    ("GET", "/management/flamegraph"),
    ("GET", "/management/health"),
    ("GET", "/management/status"),
];

/// Clinical reads (base-relative). `POST /query/**` is a read despite its verb
/// (AQL execution selects, it never commits), and so is `POST /message/export`
/// — SM `I_EHR_EXTRACT_SERVICE.export_ehr_extracts`, whose selector is an
/// `EXTRACT_SPEC` body.
const CLINICAL_READ: &[(&str, &str)] = &[
    ("GET", "/ehr"),
    ("GET", "/ehr/{ehr_id}"),
    ("GET", "/ehr/{ehr_id}/tags"),
    ("GET", "/ehr/{ehr_id}/composition/{uid_based_id}"),
    ("GET", "/ehr/{ehr_id}/composition/{uid_based_id}/tags"),
    ("GET", "/ehr/{ehr_id}/contribution"),
    ("GET", "/ehr/{ehr_id}/contribution/{contribution_uid}"),
    ("GET", "/ehr/{ehr_id}/directory"),
    ("GET", "/ehr/{ehr_id}/directory/{version_uid}"),
    ("GET", "/ehr/{ehr_id}/ehr_status"),
    ("GET", "/ehr/{ehr_id}/ehr_status/{version_uid}"),
    ("GET", "/ehr/{ehr_id}/ehr_status/{uid_based_id}/tags"),
    (
        "GET",
        "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}",
    ),
    (
        "GET",
        "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/revision_history",
    ),
    (
        "GET",
        "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version",
    ),
    (
        "GET",
        "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version/{version_uid}",
    ),
    ("GET", "/ehr/{ehr_id}/versioned_ehr_status"),
    ("GET", "/ehr/{ehr_id}/versioned_ehr_status/revision_history"),
    ("GET", "/ehr/{ehr_id}/versioned_ehr_status/version"),
    (
        "GET",
        "/ehr/{ehr_id}/versioned_ehr_status/version/{version_uid}",
    ),
    ("GET", "/query/aql"),
    ("POST", "/query/aql"),
    ("GET", "/query/{qualified_query_name}"),
    ("POST", "/query/{qualified_query_name}"),
    ("GET", "/query/{qualified_query_name}/{version}"),
    ("POST", "/query/{qualified_query_name}/{version}"),
    // The cohort extension: AQL over a demographically-selected population.
    // A read like every other query execution, and its own authorization
    // beyond the class is the deployment's `[cohort.predicates]` allow-list.
    ("POST", "/query/cohort"),
    ("GET", "/definition/query"),
    ("GET", "/definition/query/{qualified_query_name}"),
    ("GET", "/definition/query/{qualified_query_name}/{version}"),
    ("GET", "/definition/template/adl1.4"),
    ("GET", "/definition/template/adl1.4/{template_id}"),
    ("GET", "/definition/template/adl1.4/{template_id}/example"),
    ("GET", "/definition/template/adl2"),
    ("GET", "/definition/template/adl2/{template_id}"),
    ("GET", "/definition/template/adl2/{template_id}/example"),
    ("GET", "/definition/template/adl2/{template_id}/{version}"),
    ("GET", "/definition/archetype/adl1.4"),
    ("GET", "/definition/archetype/adl1.4/{archetype_id}"),
    ("GET", "/definition/archetype/adl2"),
    ("GET", "/definition/archetype/adl2/count"),
    ("GET", "/definition/artefact/adl2"),
    ("GET", "/definition/artefact/adl2/count"),
    ("GET", "/demographic/tags"),
    ("GET", "/demographic/contribution/{contribution_uid}"),
    ("GET", "/demographic/agent/{uid_based_id}"),
    ("GET", "/demographic/agent/{uid_based_id}/tags"),
    ("GET", "/demographic/group/{uid_based_id}"),
    ("GET", "/demographic/group/{uid_based_id}/tags"),
    ("GET", "/demographic/organisation/{uid_based_id}"),
    ("GET", "/demographic/organisation/{uid_based_id}/tags"),
    ("GET", "/demographic/person/{uid_based_id}"),
    ("GET", "/demographic/person/{uid_based_id}/tags"),
    ("GET", "/demographic/role/{uid_based_id}"),
    ("GET", "/demographic/role/{uid_based_id}/tags"),
    ("GET", "/demographic/party_relationship/{uid_based_id}"),
    ("GET", "/demographic/versioned_party/{versioned_object_uid}"),
    (
        "GET",
        "/demographic/versioned_party/{versioned_object_uid}/revision_history",
    ),
    (
        "GET",
        "/demographic/versioned_party/{versioned_object_uid}/version",
    ),
    (
        "GET",
        "/demographic/versioned_party/{versioned_object_uid}/version/{version_uid}",
    ),
    (
        "GET",
        "/demographic/versioned_party_relationship/{versioned_object_uid}",
    ),
    (
        "GET",
        "/demographic/versioned_party_relationship/{versioned_object_uid}/revision_history",
    ),
    (
        "GET",
        "/demographic/versioned_party_relationship/{versioned_object_uid}/version",
    ),
    (
        "GET",
        "/demographic/versioned_party_relationship/{versioned_object_uid}/version/{version_uid}",
    ),
    ("GET", "/terminology"),
    ("GET", "/terminology/{terminology_id}"),
    ("GET", "/terminology/{terminology_id}/subsumes"),
    ("GET", "/terminology/{terminology_id}/term/{code}"),
    (
        "GET",
        "/terminology/{terminology_id}/value_set/{value_set_id}",
    ),
    (
        "GET",
        "/terminology/{terminology_id}/value_set/{value_set_id}/validate",
    ),
    ("GET", "/message/export/{ehr_id}"),
    ("POST", "/message/export"),
];

/// Clinical writes (base-relative). The `/message` imports and the TDD
/// intake sit here deliberately: the group carries the ordinary clinical
/// authentication class rather than the admin gate (`api::message` module
/// docs), like every other route that commits clinical content.
const CLINICAL_WRITE: &[(&str, &str)] = &[
    ("POST", "/ehr"),
    ("PUT", "/ehr/{ehr_id}"),
    ("PUT", "/ehr/{ehr_id}/ehr_status"),
    ("PUT", "/ehr/{ehr_id}/ehr_status/{uid_based_id}/tags"),
    (
        "DELETE",
        "/ehr/{ehr_id}/ehr_status/{uid_based_id}/tags/{key}",
    ),
    ("POST", "/ehr/{ehr_id}/composition"),
    ("PUT", "/ehr/{ehr_id}/composition/{uid_based_id}"),
    ("DELETE", "/ehr/{ehr_id}/composition/{uid_based_id}"),
    ("PUT", "/ehr/{ehr_id}/composition/{uid_based_id}/tags"),
    (
        "DELETE",
        "/ehr/{ehr_id}/composition/{uid_based_id}/tags/{key}",
    ),
    ("POST", "/ehr/{ehr_id}/contribution"),
    ("POST", "/ehr/{ehr_id}/directory"),
    ("PUT", "/ehr/{ehr_id}/directory"),
    ("DELETE", "/ehr/{ehr_id}/directory"),
    ("PUT", "/definition/query/{qualified_query_name}"),
    ("PUT", "/definition/query/{qualified_query_name}/{version}"),
    ("POST", "/definition/template/adl1.4"),
    ("POST", "/definition/template/adl2"),
    ("POST", "/definition/archetype/adl1.4"),
    ("POST", "/demographic/contribution"),
    ("POST", "/demographic/agent"),
    ("PUT", "/demographic/agent/{uid_based_id}"),
    ("DELETE", "/demographic/agent/{uid_based_id}"),
    ("PUT", "/demographic/agent/{uid_based_id}/tags"),
    ("DELETE", "/demographic/agent/{uid_based_id}/tags/{key}"),
    ("POST", "/demographic/group"),
    ("PUT", "/demographic/group/{uid_based_id}"),
    ("DELETE", "/demographic/group/{uid_based_id}"),
    ("PUT", "/demographic/group/{uid_based_id}/tags"),
    ("DELETE", "/demographic/group/{uid_based_id}/tags/{key}"),
    ("POST", "/demographic/organisation"),
    ("PUT", "/demographic/organisation/{uid_based_id}"),
    ("DELETE", "/demographic/organisation/{uid_based_id}"),
    ("PUT", "/demographic/organisation/{uid_based_id}/tags"),
    (
        "DELETE",
        "/demographic/organisation/{uid_based_id}/tags/{key}",
    ),
    ("POST", "/demographic/person"),
    ("PUT", "/demographic/person/{uid_based_id}"),
    ("DELETE", "/demographic/person/{uid_based_id}"),
    ("PUT", "/demographic/person/{uid_based_id}/tags"),
    ("DELETE", "/demographic/person/{uid_based_id}/tags/{key}"),
    ("POST", "/demographic/role"),
    ("PUT", "/demographic/role/{uid_based_id}"),
    ("DELETE", "/demographic/role/{uid_based_id}"),
    ("PUT", "/demographic/role/{uid_based_id}/tags"),
    ("DELETE", "/demographic/role/{uid_based_id}/tags/{key}"),
    ("POST", "/demographic/party_relationship"),
    ("PUT", "/demographic/party_relationship/{uid_based_id}"),
    ("DELETE", "/demographic/party_relationship/{uid_based_id}"),
    ("POST", "/message/import"),
    ("POST", "/message/import/{ehr_id}"),
    ("POST", "/message/tdd/{ehr_id}"),
    ("POST", "/message/tdd/{ehr_id}/batch"),
];

/// Admin-class reads (base-relative) — every one under the `/admin/` prefix
/// the coarse gate keys on.
const ADMIN_READ: &[(&str, &str)] = &[
    ("GET", "/admin/config"),
    ("GET", "/admin/report/contribution"),
    ("GET", "/admin/report/contribution/count"),
    ("GET", "/admin/report/composition_version/count"),
    ("GET", "/admin/report/versioned_composition/count"),
    ("GET", "/admin/event_subscription"),
    ("GET", "/admin/event_subscription/{subscription_id}"),
    // The storage-parity sweep mutates nothing and answers identifiers +
    // defect classes only — a pinned EXTENSION_READ_ROUTES read despite the
    // POST verb, so a read-only integrity auditor can run it (#2692).
    ("POST", "/admin/integrity/verify"),
    // The legal registers, read side: the restriction sequence a controller
    // answers an Art. 18(3) notice from, the declared retention periods, and
    // what has fallen due. They carry ids, grounds and citations, never
    // content.
    ("GET", "/admin/restriction"),
    ("GET", "/admin/retention/policy"),
    ("GET", "/admin/retention/due"),
];

/// Admin-class writes (base-relative).
const ADMIN_WRITE: &[(&str, &str)] = &[
    ("DELETE", "/admin/ehr/all"),
    ("DELETE", "/admin/ehr/{ehr_id}"),
    ("DELETE", "/admin/template/{template_id}"),
    // The other two destructive shared-definition routes. They do not sit under
    // /admin/, and they used to take Clinical from that fallback — so the
    // privilege depended on the path prefix rather than on the blast radius
    // (issue #2071). SM master04 puts removal of archetypes AND templates in one
    // clause and one pair of interfaces, so the three must match.
    ("DELETE", "/definition/archetype/adl1.4/{archetype_id}"),
    ("DELETE", "/definition/artefact/adl2/{artefact_id}"),
    ("DELETE", "/admin/query/{qualified_query_name}/{version}"),
    ("POST", "/admin/archive/ehrs"),
    ("POST", "/admin/archive/parties"),
    ("POST", "/admin/archive/ehrs/restore"),
    ("POST", "/admin/archive/parties/restore"),
    ("POST", "/admin/dump"),
    ("POST", "/admin/load"),
    // The legal marks, write side. Each one changes what every other route
    // answers for the named record, so a read-only principal is refused: a
    // restriction takes an object out of every read, query, export and event,
    // and a retention hold decides what a disposal would cover.
    ("POST", "/admin/restriction"),
    ("POST", "/admin/restriction/lift"),
    ("POST", "/admin/research-objection"),
    ("PUT", "/admin/retention/policy"),
    ("PUT", "/admin/retention/anchor"),
    ("POST", "/admin/retention/hold"),
    // The repair the parity sweep is the diagnosis for. Unlike the sweep it
    // REPLACES stored rows, so it stays out of EXTENSION_READ_ROUTES and a
    // read-only server refuses it (#3143).
    ("POST", "/admin/integrity/rebuild-nodes"),
    ("POST", "/admin/event_subscription"),
    ("PUT", "/admin/event_subscription/{subscription_id}"),
    ("DELETE", "/admin/event_subscription/{subscription_id}"),
];

/// The ITI-81 audit retrieval: the node's security-surveillance record, so the
/// handler enforces the admin role itself because the coarse gate would class
/// this FHIR-rooted path `Clinical` (`extensions::fhir::audit_search`).
const HANDLER_ADMIN_READ: &[(&str, &str)] = &[("GET", "/fhir/r4/AuditEvent")];

/// The declared classification of every mounted route, keyed by
/// `(method, absolute path)`.
fn declared() -> BTreeMap<(String, String), (Class, Effect)> {
    let mut out = BTreeMap::new();
    // `prefix` is empty for the tables that already spell absolute paths.
    let mut rows = |rows: &[(&str, &str)], prefix: &str, class: Class, effect: Effect| {
        for (method, path) in rows {
            let prior = out.insert(
                ((*method).to_owned(), format!("{prefix}{path}")),
                (class, effect),
            );
            assert!(prior.is_none(), "duplicate table row: {method} {path}");
        }
    };
    let public: Vec<(&str, &str)> = PUBLIC.iter().map(|(m, p, _)| (*m, *p)).collect();
    rows(&public, "", Class::Public, Effect::Read);
    rows(MANAGEMENT, "", Class::Management, Effect::Read);
    rows(CLINICAL_READ, BASE, Class::Clinical, Effect::Read);
    rows(CLINICAL_WRITE, BASE, Class::Clinical, Effect::Write);
    rows(ADMIN_READ, BASE, Class::Admin, Effect::Read);
    rows(ADMIN_WRITE, BASE, Class::Admin, Effect::Write);
    rows(HANDLER_ADMIN_READ, BASE, Class::HandlerAdmin, Effect::Read);
    out
}

// ── the router's own route set ────────────────────────────────────────────────

/// Every `(method, path)` the assembled router mounts, read from the served
/// `OpenAPI` document — built from the same `utoipa-axum` composition that
/// mounts the routes, so it cannot drift from the live surface.
fn mounted(cfg: &AppConfig) -> Vec<(String, String)> {
    let doc = ferroehr_rest::extensions::openapi::extensions_document(cfg);
    let mut out = Vec::new();
    for (path, item) in &doc.paths.paths {
        for (method, present) in [
            ("GET", item.get.is_some()),
            ("PUT", item.put.is_some()),
            ("POST", item.post.is_some()),
            ("DELETE", item.delete.is_some()),
            ("PATCH", item.patch.is_some()),
            ("HEAD", item.head.is_some()),
            ("OPTIONS", item.options.is_some()),
            ("TRACE", item.trace.is_some()),
        ] {
            if present {
                out.push((method.to_owned(), path.clone()));
            }
        }
    }
    out.sort();
    out
}

// ── app assembly ──────────────────────────────────────────────────────────────

/// Authentication on (bearer only — an HMAC token is orders of magnitude
/// cheaper than an Argon2 verification, and this suite issues hundreds of
/// requests), the admin group reachable so the RBAC gate is what decides.
fn matrix_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            // ON: the PUBLIC table declares the OAS/Swagger routes, and they are
            // mounted only when this is set — with it off the coverage assertion
            // reports them as declared-but-unmounted.
            swagger_ui: AccessLevel::Public,
            ..Default::default()
        },
        auth: common::hs256_auth_config(ISSUER, AUDIENCE, HMAC_SECRET),
        admin: AdminConfig { enabled: true },
        // ON for the same reason as the Swagger pair: the PUBLIC table declares
        // the discovery document, and it is mounted only when SMART is enabled.
        smart: SmartConfig {
            enabled: true,
            ..SmartConfig::default()
        },
        ..Default::default()
    }
}

/// The RBAC gate over the default rule set (`ADMIN` / `READONLY`), no ABAC.
fn authz() -> Option<Arc<AuthzHandle>> {
    AuthzHandle::build(
        &AuthzConfig::default(),
        &matrix_config().server.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new)
}

async fn app() -> (testkit::TestDb, Router) {
    let (pg, pool) = common::migrated_pool().await;
    // The local Audit Record Repository is wired because the ITI-81 route's
    // store gate runs BEFORE its handler-level admin gate (`fhir::audit_search`)
    // — without a store that route answers `404` and its admin cell would be
    // unobservable.
    let svc = ferroehr::service::FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&pool),
    )
    .with_audit_store(ferroehr::system_log::store::AuditStore::new(pool));
    let app = ferroehr_rest::build_full(
        matrix_config(),
        Arc::new(svc),
        authz(),
        ferroehr_rest::extensions::management::Observability::default(),
    )
    .expect("build app");
    (pg, app)
}

// ── principals + probing ──────────────────────────────────────────────────────

/// A bearer token carrying `roles` — the RFC 9068 §2.2.3.1 carrier the gate
/// reads. An empty list is a valid authenticated principal with no roles.
fn bearer(roles: &[&str]) -> String {
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).expect("timestamp") + 3600;
    let claims: Value = json!({
        "sub": "matrix",
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": exp,
        "roles": roles,
    });
    common::hs256_bearer(HMAC_SECRET, &claims)
}

/// The refusal the authorization layer rendered, identified by the reason text
/// `roles::authorize`/`authorize_readonly` produced. A `403` from any other
/// gate is `None`: it is not an RBAC decision and must never be read as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// `Clinical` with no roles at all.
    NeedsRole,
    /// A class (or a handler) requiring the admin role.
    NeedsAdmin,
    /// The read-only restriction on a write.
    ReadOnly,
}

/// Substitute every `{param}` capture with [`PROBE_PARAM`].
fn concrete(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c == '{' {
            for skipped in chars.by_ref() {
                if skipped == '}' {
                    break;
                }
            }
            out.push_str(PROBE_PARAM);
        } else {
            out.push(c);
        }
    }
    out
}

/// Drive one route with one principal and report the authorization refusal, if
/// any. `roles = None` sends no `Authorization` header at all.
async fn probe(app: &Router, method: &str, path: &str, roles: Option<&[&str]>) -> Probe {
    let mut builder = Request::builder()
        .method(method)
        .uri(concrete(path))
        .header("content-type", "application/json");
    if let Some(roles) = roles {
        builder = builder.header("authorization", bearer(roles));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from("{}")).expect("request"))
        .await
        .expect("oneshot");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let refusal = (status == StatusCode::FORBIDDEN)
        .then(|| reason_of(&bytes))
        .flatten();
    Probe { status, refusal }
}

/// One probe's outcome.
struct Probe {
    status: StatusCode,
    refusal: Option<Refusal>,
}

/// Classify a `403` body by the reason the RBAC layer renders — the openEHR
/// `{ error, message }` shape, or a FHIR `OperationOutcome` where the handler
/// gate speaks FHIR (the ITI-81 retrieval).
fn reason_of(body: &[u8]) -> Option<Refusal> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("issue")
                .and_then(Value::as_array)
                .and_then(|issues| issues.first())
                .and_then(|issue| issue.get("diagnostics"))
                .and_then(Value::as_str)
        })?
        .to_owned();
    if message.contains("requires the 'ADMIN' role") {
        Some(Refusal::NeedsAdmin)
    } else if message.contains("at least one role") {
        Some(Refusal::NeedsRole)
    } else if message.contains("read-only role") {
        Some(Refusal::ReadOnly)
    } else {
        None
    }
}

// ── the tests ─────────────────────────────────────────────────────────────────

/// Every route the router mounts is classified in one of the tables above, and
/// no table row names a route the router does not mount.
///
/// This is the property the one-per-class coverage could not give: a new route
/// — anywhere, including outside `/admin/` where the coarse gate's fallback
/// would silently make it clinical — has to be classified deliberately or this
/// test fails.
#[test]
fn every_mounted_route_carries_a_declared_authorization_class() {
    let cfg = matrix_config();
    let declared = declared();
    let mounted = mounted(&cfg);

    let undeclared: Vec<String> = mounted
        .iter()
        .filter(|key| !declared.contains_key(*key))
        .map(|(method, path)| format!("{method} {path}"))
        .collect();
    assert!(
        undeclared.is_empty(),
        "mounted routes with no declared authorization class — classify each one in \
         app/ferroehr-rest/tests/it/authz_route_matrix.rs: {undeclared:#?}"
    );

    let stale: Vec<String> = declared
        .keys()
        .filter(|key| !mounted.contains(key))
        .map(|(method, path)| format!("{method} {path}"))
        .collect();
    assert!(
        stale.is_empty(),
        "declared routes the router no longer mounts: {stale:#?}"
    );
}

/// The role × route decision matrix on the wire: for every gated route, each
/// principal gets exactly the outcome its declared class prescribes.
///
/// | class | no roles | ordinary | admin | admin + read-only |
/// |---|---|---|---|---|
/// | `Clinical` read | refused (needs a role) | allowed | allowed | allowed |
/// | `Clinical` write | refused (needs a role) | allowed | allowed | refused (read-only) |
/// | `Admin` read | refused (needs `ADMIN`) | refused (needs `ADMIN`) | allowed | allowed |
/// | `Admin` write | refused (needs `ADMIN`) | refused (needs `ADMIN`) | allowed | refused (read-only) |
/// | `HandlerAdmin` read | refused (needs a role) | refused (needs `ADMIN`) | allowed | allowed |
///
/// "Allowed" means the authorization layer did not refuse — the concrete
/// post-gate status is real-backend behaviour (`rbac_e2e`'s doctrine), not the
/// gate's, so pinning it here would couple authorization to unrelated handler
/// outcomes.
#[tokio::test]
async fn the_role_matrix_holds_for_every_gated_route() {
    let (_pg, app) = app().await;
    let mut failures: Vec<String> = Vec::new();

    for ((method, path), (class, effect)) in declared() {
        let expected = match (class, effect) {
            (Class::Public | Class::Management, _) => continue,
            (Class::Clinical, Effect::Read) => [Some(Refusal::NeedsRole), None, None, None],
            (Class::Clinical, Effect::Write) => [
                Some(Refusal::NeedsRole),
                None,
                None,
                Some(Refusal::ReadOnly),
            ],
            (Class::Admin, Effect::Read) => [
                Some(Refusal::NeedsAdmin),
                Some(Refusal::NeedsAdmin),
                None,
                None,
            ],
            (Class::Admin, Effect::Write) => [
                Some(Refusal::NeedsAdmin),
                Some(Refusal::NeedsAdmin),
                None,
                Some(Refusal::ReadOnly),
            ],
            (Class::HandlerAdmin, _) => [
                Some(Refusal::NeedsRole),
                Some(Refusal::NeedsAdmin),
                None,
                None,
            ],
        };
        let principals: [(&str, &[&str]); 4] = [
            ("no roles", &[]),
            ("ordinary", &["USER"]),
            ("admin", &["ADMIN"]),
            ("admin + read-only", &["ADMIN", "READONLY"]),
        ];
        for (index, (label, roles)) in principals.into_iter().enumerate() {
            let outcome = probe(&app, &method, &path, Some(roles)).await;
            let want = expected[index];
            if outcome.refusal != want {
                failures.push(format!(
                    "{method} {path} [{label}]: expected {want:?}, observed {:?} (status {})",
                    outcome.refusal, outcome.status
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "authorization matrix cells that did not hold: {failures:#?}"
    );
}

/// An unauthenticated caller reaches every route declared `Public` and no
/// other: the gated surface answers `401` without a credential (ITS-REST
/// overview `Requests_and_responses.md` — unauthenticated is `401`,
/// authenticated-but-refused is `403`).
///
/// Scope: the always-on family plus the System `OPTIONS` manifest. The OAS /
/// Swagger pair is mounted only when `server.swagger_ui` is on (off here), and
/// the SMART discovery document only when SMART is enabled.
#[tokio::test]
async fn the_public_family_needs_no_credential_and_the_gated_surface_does() {
    let (_pg, app) = app().await;

    for (method, path) in [
        ("GET", "/health"),
        ("GET", "/health/liveness"),
        ("GET", "/health/readiness"),
        ("GET", "/ferroehr/rest/status"),
        ("OPTIONS", BASE),
    ] {
        let outcome = probe(&app, method, path, None).await;
        assert_ne!(
            outcome.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} is declared public and must need no credential"
        );
        assert_ne!(
            outcome.status,
            StatusCode::FORBIDDEN,
            "{method} {path} is declared public and must not be refused"
        );
    }

    // The counter-proof: a gated route with no credential is `401`, so the
    // assertions above are not passing because authentication is off.
    let gated = probe(&app, "GET", &format!("{BASE}/ehr/{{ehr_id}}"), None).await;
    assert_eq!(
        gated.status,
        StatusCode::UNAUTHORIZED,
        "an unauthenticated read of a gated route must be 401"
    );
}

/// The public inventory is closed (#3671): every mounted route, driven with no
/// `Authorization` header, is reachable only if [`PUBLIC`] declares it, and
/// every other route answers `401` (ITS-REST overview `Requests_and_responses.md`).
///
/// The management surface keeps its own per-endpoint gate
/// (`management::private_endpoint_401_then_200` and siblings); its endpoints
/// are not mounted here, so they must not answer at all.
#[tokio::test]
async fn every_route_outside_the_public_inventory_needs_a_credential() {
    let (_pg, app) = app().await;
    let declared = declared();
    let mut failures = Vec::new();

    for (method, path) in mounted(&matrix_config()) {
        let Some(&(class, _)) = declared.get(&(method.clone(), path.clone())) else {
            // `every_mounted_route_carries_a_declared_authorization_class` owns
            // the undeclared case.
            continue;
        };
        let status = probe(&app, &method, &path, None).await.status;
        let holds = match class {
            Class::Public => status != StatusCode::UNAUTHORIZED && status != StatusCode::FORBIDDEN,
            Class::Management => {
                status == StatusCode::UNAUTHORIZED || status == StatusCode::NOT_FOUND
            }
            Class::Clinical | Class::Admin | Class::HandlerAdmin => {
                status == StatusCode::UNAUTHORIZED
            }
        };
        if !holds {
            failures.push(format!("{method} {path} ({class:?}) answered {status}"));
        }
    }

    assert!(
        failures.is_empty(),
        "routes reachable without a credential outside the public inventory: {failures:#?}"
    );
}

/// `server.swagger_ui` is an access level over the Swagger UI and the OpenAPI
/// documents (#3094): `private` needs a credential (`401` with the server's
/// `WWW-Authenticate` challenge for an anonymous request, so a browser prompts),
/// `admin_only` needs the admin role (`403` for a role-less principal), `off`
/// mounts nothing (`404`). No openEHR spec governs the documentation surface —
/// our own design.
#[tokio::test]
async fn the_swagger_surface_honours_its_access_level() {
    async fn app_at(level: AccessLevel) -> (testkit::TestDb, Router) {
        let (pg, pool) = common::migrated_pool().await;
        let cfg = AppConfig {
            server: ServerConfig {
                swagger_ui: level,
                ..Default::default()
            },
            ..matrix_config()
        };
        let app = ferroehr_rest::build_full(
            cfg,
            Arc::new(ferroehr::service::FerroEhrService::new(
                &ferroehr::db::domain::DomainPools::from_shared(&pool),
            )),
            authz(),
            ferroehr_rest::extensions::management::Observability::default(),
        )
        .expect("build app");
        (pg, app)
    }
    let ui = "/ferroehr/rest/swagger-ui";
    let doc = "/ferroehr/rest/api-docs/openapi.json";

    let (_pg, app) = app_at(AccessLevel::Private).await;
    for path in [ui, doc] {
        assert_eq!(
            probe(&app, "GET", path, None).await.status,
            StatusCode::UNAUTHORIZED,
            "{path} needs a credential under `private`"
        );
        let authed = probe(&app, "GET", path, Some(&[])).await.status;
        assert!(
            authed.is_success() || authed.is_redirection(),
            "{path} is served to any authenticated principal under `private`, got {authed}"
        );
    }
    let anonymous = app
        .clone()
        .oneshot(Request::get(ui).body(Body::empty()).expect("request"))
        .await
        .expect("oneshot");
    assert!(
        anonymous.headers().contains_key("www-authenticate"),
        "the refusal carries the challenge a browser answers"
    );

    let (_pg, app) = app_at(AccessLevel::AdminOnly).await;
    assert_eq!(
        probe(&app, "GET", ui, Some(&[])).await.status,
        StatusCode::FORBIDDEN,
        "a role-less principal is refused under `admin_only`"
    );
    assert_eq!(
        probe(&app, "GET", ui, Some(&["ADMIN"])).await.status,
        StatusCode::TEMPORARY_REDIRECT,
        "the admin role reaches the UI under `admin_only`"
    );

    let (_pg, app) = app_at(AccessLevel::Off).await;
    assert_eq!(
        probe(&app, "GET", ui, None).await.status,
        StatusCode::NOT_FOUND,
        "`off` mounts nothing"
    );
}
