// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `OpenAPI` document + Swagger UI (discoverability).
//!
//! No openEHR spec governs an OAS-serving endpoint — our own surface. The
//! Swagger UI's spec selector shows only documents this server generates itself,
//! never a vendored OAS: one filtered document per API family (`FAMILIES`, at
//! `{api-docs}/ferroehr-{family}.openapi.json`) plus the complete server surface
//! last, each derived from the single composed document built natively from
//! every `#[utoipa::path]` handler via `utoipa-axum`'s [`OpenApiRouter`]. Route
//! and `OpenAPI` path are single-sourced from one handler, so the document
//! cannot drift from the router. The vendored ITS-REST OAS bundles are codegen
//! input for the generated group contract only, and are never served.
//!
//! Authentication in the document is config-driven: the served document declares
//! exactly the one scheme in effect per [`AuthConfig`] (`openehr_auth`, bearer
//! JWT when OIDC is configured, else HTTP Basic; none when auth is disabled), so
//! the Swagger "Authorize" dialog and the per-endpoint padlocks match the running
//! server. The lock is applied to the authenticated surfaces only.
//!
//! The UI assets are served through [`utoipa_swagger_ui::serve`] rather than
//! [`utoipa_swagger_ui::SwaggerUi`]'s router, and the mount path redirects to
//! `index.html` under the mount. Both halves are load-bearing: the dist
//! `index.html` references its assets relatively, so a body served at the
//! slash-less mount path renders empty, and the trailing-slash target
//! `SwaggerUi`'s own router answers with is an infinite loop under the
//! serve-time `NormalizePathLayer`.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use tower_http::set_header::SetResponseHeaderLayer;
use utoipa::OpenApi;
use utoipa::openapi::security::{
    Http, HttpAuthScheme, HttpBuilder, SecurityRequirement, SecurityScheme,
};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::{Config, SwaggerFile, Url};

use crate::config::AppConfig;
use crate::extensions::health;
use crate::extensions::management;
use crate::overview::status;
use crate::state::AppState;
use ferroehr::config::auth::AuthConfig;
use ferroehr::telemetry::provenance;

/// The logical name of the single advertised security scheme. One name, one
/// scheme kind chosen by config, so the Swagger "Authorize" dialog shows exactly
/// what the server accepts.
const SECURITY_SCHEME: &str = "openehr_auth";

/// Info + tags carrier for the served document. It declares **no paths** — every
/// path comes from a `#[utoipa::path]` handler collected through `utoipa-axum`
/// (see [`extensions_document`]) — so there is no hand-maintained operation list
/// to drift.
///
/// `info.version` is stated explicitly (rather than left to utoipa's
/// `CARGO_PKG_VERSION` default) because the value is a deliberate choice: it is
/// the **product** `SemVer`, NOT an openEHR contract version. The ITS-REST
/// contract identity this build implements is published separately as the
/// document-level `x-openehr-its-rest` extension
/// ([`extensions_document`]), so a reader can tell the two apart. The document's
/// `openapi` field is `3.1.0` (utoipa emits only that version); the vendored
/// openEHR OAS bundles are 3.0.3 — a stated fact, and irrelevant to the served
/// surface, which is generated from our own handlers and never from a vendored
/// bundle.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "FerroEHR — openEHR ITS-REST + extensions",
        version = env!("CARGO_PKG_VERSION"),
        description = "The complete surface this server serves, generated natively from its \
                       handlers: the standardised ITS-REST API groups (EHR / COMPOSITION / \
                       CONTRIBUTION / DIRECTORY / DEMOGRAPHIC / DEFINITION / QUERY / ADMIN), the \
                       own-design extensions (terminology, PARTY_RELATIONSHIP, event-subscription, \
                       the ITI-81 audit retrieval), and the operational endpoints \
                       (health, status, management, SMART discovery, the OpenAPI endpoints). \
                       `info.version` is this product's own SemVer; the implemented openEHR \
                       ITS-REST contract version is the document-level `x-openehr-its-rest` \
                       extension. Everything outside the standardised ITS-REST groups is OUR OWN \
                       EXTENSION — no openEHR spec governs it — and every such operation says so \
                       in its own description.\n\n\
                       **Canonical-XML lineage selection (our own extension).** openEHR publishes \
                       canonical XML in two wire lineages that differ only by the document's root \
                       namespace: `http://schemas.openehr.org/v1` (ITS-XML Release-1.0.2, the \
                       STABLE bundle) and `http://schemas.openehr.org/v2` (ITS-XML Release-2.0.0, \
                       TRIAL upstream). Every operation whose media type is `application/xml` \
                       accepts a `version` media-type parameter selecting one — \
                       `Accept: application/xml; version=1` for the response, \
                       `Content-Type: application/xml; version=1` to declare a request payload. \
                       Omitting it (or sending `version=2`) means the v2 default — the only \
                       published lineage whose schemas model the RM 1.2.0 this server serves; a \
                       non-default v1 response is labelled \
                       `Content-Type: application/xml; version=1`. No openEHR specification \
                       governs the parameter — the ITS-REST text predates the dual bundles — but \
                       its refusal branches are the released ones: an unrecognized `version` on \
                       `Accept` is `406 Not Acceptable` and on `Content-Type` is `415 Unsupported \
                       Media Type` (`Resources.md` §\"XML Format\"). Operational templates \
                       (`.../definition/template/adl1.4/...`) are always served in the v1 lineage \
                       and ignore the parameter."
    ),
    servers(
        (url = "{origin}",
         description = "The server root. Every path in this document is absolute from that root \
                        and ALREADY carries this deployment's configured openEHR base path \
                        (`server.base_path`, default `/ferroehr/rest/openehr/v1`), so the server \
                        URL must not repeat it.",
         variables(
             ("origin" = (default = "",
                 description = "Scheme + authority (plus any reverse-proxy prefix) this \
                                deployment is reached at. The default is empty: same origin as \
                                the document itself."))
         ))
    ),
    external_docs(
        url = "https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0",
        description = "The openEHR ITS-REST Release-1.1.0 specification this server's \
                       standardised groups implement. The extension groups documented here are \
                       outside it — our own design."
    ),
    tags(
        // One tag per RM resource, exactly as the vendored group OAS documents
        // categorise them.
        (name = "EHR", description = "Management of EHRs — create/retrieve; physical delete (Admin API)."),
        (name = "EHR_STATUS", description = "Management of EHR_STATUS and its VERSIONED_EHR_STATUS reads."),
        (name = "COMPOSITION", description = "Management of COMPOSITION and its VERSIONED_COMPOSITION reads."),
        (name = "DIRECTORY", description = "Management of the directory (FOLDER) tree."),
        (name = "CONTRIBUTION", description = "Management of CONTRIBUTION (EHR + demographic)."),
        (name = "ITEM_TAG", description = "Management of ITEM_TAG sub-resources (EHR + demographic)."),
        (name = "PERSON", description = "Management of the demographic PERSON (ITS-REST, DEVELOPMENT)."),
        (name = "AGENT", description = "Management of the demographic AGENT (ITS-REST, DEVELOPMENT)."),
        (name = "GROUP", description = "Management of the demographic GROUP (ITS-REST, DEVELOPMENT)."),
        (name = "ORGANISATION", description = "Management of the demographic ORGANISATION (ITS-REST, DEVELOPMENT)."),
        (name = "ROLE", description = "Management of the demographic ROLE (ITS-REST, DEVELOPMENT)."),
        (name = "VERSIONED_PARTY", description = "Management of the VERSIONED_PARTY reads (ITS-REST, DEVELOPMENT)."),
        (name = "ADL1.4", description = "Management of AOM/ADL 1.4 operational templates."),
        (name = "ADL2", description = "Management of AOM2/ADL 2 templates."),
        (name = "Query", description = "Ad-hoc + stored AQL execution and stored-query definitions (ITS-REST)."),
        (name = "ADMIN", description = "The Admin API's own-design routes — template delete, stored-query-version delete, the redacted config read. The released Admin API defines only the two EHR physical deletes (tagged `EHR`); everything under this tag is OUR OWN EXTENSION (no ITS-REST operation governs it)."),
        (name = "system", description = "The ITS-REST System API (STABLE): the single `OPTIONS` Options-and-Conformance manifest at the API base-path root."),
        (name = "audit", description = "IHE RESTful-ATNA ITI-81 Retrieve ATNA Audit Event over the local Audit Record Repository. Governed by IHE ITI TF-2 (transaction ITI-81), not by any openEHR specification — our own extension (config-gated: the `[audit.store]` local repository)."),
        (name = "status", description = "Public operational status + the always-on health family (unauthenticated, never config-gated); each says only what a client or probe needs, the detail is under the `management` tag."),
        (name = "smart", description = "SMART App Launch service discovery (config-gated: FERROEHR_REST_SMART__ENABLED)."),
        (name = "openapi", description = "`OpenAPI` document + Swagger UI discoverability (config-gated: FERROEHR_REST_SWAGGER_UI)."),
        (name = "management", description = "Ops-introspection surface — info/prometheus/metrics/env/loggers/flamegraph, the per-indicator health detail and the full status document (config-gated: FERROEHR__MANAGEMENT__*); each endpoint opt-in via its access level. The orchestrator probes are NOT here: see the `status` tag."),
        (name = "terminology", description = "Terminology extension wire — SM I_TERMINOLOGY_SERVICE (config-gated: FERROEHR_REST_TERMINOLOGY__ENABLED)."),
        (name = "demographic-relationship", description = "PARTY_RELATIONSHIP demographic extension (SM-3; no ITS-REST contract)."),
        (name = "definition-archetype", description = "ADL 1.4 / ADL 2 archetype + artefact provisioning — SM I_DEFINITION_ADL14 / I_DEFINITION_ADL2 operations the released Definition API never surfaced (it provisions operational templates only). OUR OWN EXTENSION: no ITS-REST operation governs these routes."),
        (name = "admin-report", description = "The SM I_ADMIN_SERVICE activity-report calls (contribution/version statistics per PLATFORM_SERVICE). OUR OWN EXTENSION: the released Admin API is the two EHR deletes alone, so no ITS-REST operation governs these routes (same admin gate + RBAC Admin class)."),
        (name = "admin-archive", description = "The SM I_ADMIN_ARCHIVE calls (move selected EHRs / parties to archival storage) plus the reverse movement the SM declares no operation for (restore selected EHRs / parties from archival storage). OUR OWN EXTENSION: no ITS-REST operation governs these routes (same admin gate + RBAC Admin class)."),
        (name = "admin-integrity", description = "The storage-parity sweep: every stored version is re-derived from its decomposed node rows and compared with its materialized body, so tampering or corruption of either copy is reported. OUR OWN EXTENSION: no ITS-REST operation and no SM interface governs this route, and no openEHR spec governs storage mechanics (same admin gate + RBAC Admin class)."),
        (name = "admin-marks", description = "The legal marks storage carries beside the clinical content: the GDPR Art. 18 restriction register, the Art. 21(6) research objection, and the Art. 5(1)(e) / Art. 30(1)(f) retention register with its due list. OUR OWN EXTENSION: no ITS-REST operation and no SM interface governs these routes, and nothing here deletes anything (same admin gate + RBAC Admin class)."),
        (name = "admin-dump-load", description = "The SM I_ADMIN_DUMP_LOAD calls (export every EHR to a file-system archive; populate the repository from one). OUR OWN EXTENSION: no ITS-REST operation governs these routes (same admin gate + RBAC Admin class)."),
        (name = "message", description = "The SM MESSAGE component — I_EHR_EXTRACT_SERVICE (EHR-Extract export/import) and I_TDD_SERVICE (Template Data Document import). OUR OWN EXTENSION: ITS-REST 1.1.0 publishes no message/extract/TDD API at all, so no released operation governs any route here; they carry the ordinary clinical authentication class, not the admin gate."),
        (name = "event-subscription", description = "Event-subscription CRUD extension (config-gated: FERROEHR_REST_EVENT_SUBSCRIPTION__ENABLED)."),
    )
)]
#[derive(Debug)]
struct ExtensionsInfo;

/// Compose the served extension-surface `OpenAPI` document from the per-area
/// `utoipa-axum` routers, then declare the config-appropriate security scheme.
///
/// Public surfaces (the health family, status, SMART discovery, these OAS
/// endpoints) carry no auth requirement; the authenticated surfaces (the
/// API-nested extension groups + the management surface) get the single
/// `openehr_auth` requirement (a per-endpoint padlock) whenever authentication
/// is enabled. The scheme kind
/// (bearer JWT vs HTTP Basic) is chosen by `advertised_scheme`.
///
/// Every path in the result is the one the LIVE router mounts under this
/// configuration: the API groups are nested at `server.base_path`, and the
/// endpoints whose `#[utoipa::path]` literal can only spell the default
/// deployment (status, the OAS meta-endpoints, the System `OPTIONS` manifest,
/// SMART discovery) are re-homed with `rehome_path`.
#[must_use]
pub fn extensions_document(cfg: &AppConfig) -> utoipa::openapi::OpenApi {
    let mut doc = ExtensionsInfo::openapi();

    // Public (no lock). The health family is mounted at the process root and is
    // base-path-independent, so it needs no re-homing.
    doc.merge(health::openapi());
    let rest_root = cfg.server.rest_root();
    doc.merge(status::openapi(&rest_root));
    doc.merge(crate::smart::discovery::openapi(cfg, &rest_root));
    // The System API's OPTIONS operation is a closure route mounted outside
    // `OpenApiRouter` (above CORS), documented via its twin.
    doc.merge(crate::api::system::options::openapi(&cfg.server.base_path));
    doc.merge(meta_openapi(cfg));

    // Authenticated: the management surface plus the entire API surface, nested
    // under the configured base path so the paths read as full server paths.
    let mut protected = management::openapi();
    protected.merge(crate::api::api_doc(&cfg.server.base_path));

    let scheme = advertised_scheme(&cfg.auth);
    if scheme.is_some() {
        require_auth(&mut protected);
    }
    doc.merge(protected);

    if let Some(scheme) = scheme {
        doc.components
            .get_or_insert_with(Default::default)
            .add_security_scheme(SECURITY_SCHEME, scheme);
    }

    // A document-level `x-` extension, so the contract identity is
    // machine-readable and cannot be confused with `info.version` (the product
    // SemVer). The value is the shared provenance constant every identity
    // surface reports, so they cannot drift apart.
    doc.extensions = Some(utoipa::openapi::extensions::Extensions::from_iter([(
        ITS_REST_EXTENSION,
        provenance::ITS_REST,
    )]));

    doc
}

/// The document-level extension key carrying the implemented openEHR ITS-REST
/// contract version.
///
/// An `x-` prefixed key is the OAS-sanctioned place for vendor data
/// (<https://spec.openapis.org/oas/v3.1.0#specification-extensions>).
const ITS_REST_EXTENSION: &str = "x-openehr-its-rest";

/// Moves one declared `#[utoipa::path]` literal to the path the live router
/// mounts.
///
/// A declared path is a string literal, so it can only spell the default
/// deployment; a non-default `server.base_path` moves the live mount and the
/// served document must follow, or it advertises a path this deployment does not
/// serve. A no-op when the two are equal. No openEHR spec governs where a server
/// roots its API — our own design/extension.
pub(crate) fn rehome_path(doc: &mut utoipa::openapi::OpenApi, declared: &str, live: &str) {
    if live != declared
        && let Some(item) = doc.paths.paths.remove(declared)
    {
        doc.paths.paths.insert(live.to_owned(), item);
    }
}

/// Returns the single security scheme the running server advertises, or `None`
/// when authentication is disabled.
///
/// Bearer JWT when OIDC is configured, else HTTP Basic — never both. No openEHR
/// spec governs the authorization scheme (ITS-REST places it out of band) — our
/// own operational choice.
fn advertised_scheme(auth: &AuthConfig) -> Option<SecurityScheme> {
    if !auth.enabled {
        return None;
    }
    if auth.oidc.is_some() {
        Some(SecurityScheme::Http(
            HttpBuilder::new()
                .scheme(HttpAuthScheme::Bearer)
                .bearer_format("JWT")
                .build(),
        ))
    } else if auth.basic.is_some() {
        Some(SecurityScheme::Http(Http::new(HttpAuthScheme::Basic)))
    } else {
        None
    }
}

/// Stamps the single `openehr_auth` security requirement onto every operation in
/// `doc`, the per-endpoint padlock in Swagger.
fn require_auth(doc: &mut utoipa::openapi::OpenApi) {
    let requirement = SecurityRequirement::new(SECURITY_SCHEME, Vec::<String>::new());
    for item in doc.paths.paths.values_mut() {
        for op in [
            item.get.as_mut(),
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.options.as_mut(),
            item.head.as_mut(),
            item.patch.as_mut(),
            item.trace.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            op.security = Some(vec![requirement.clone()]);
        }
    }
}

// ── The OAS meta-endpoints (documented + served by real handlers) ─────────────

/// Serves the complete server-surface `OpenAPI` JSON document.
///
/// No openEHR spec governs an OAS-serving endpoint — our own discoverability
/// surface. Public, always `200 application/json`, and config-gated by the
/// Swagger UI (a router `404` when disabled). This handler carries the
/// `#[utoipa::path]` metadata that puts the endpoint into the composed document
/// ([`meta_openapi`]); the live route serves the document pre-serialized at
/// assembly ([`prebuild_docs`], [`swagger_router`]), so this body runs only if
/// the endpoint is mounted directly.
///
/// The sibling per-family documents are declared by [`family_openapi_json`]. The
/// UI asset route is deliberately not declared as an operation: it is UI
/// packaging from the vendored `utoipa-swagger-ui` distribution, with no API
/// contract of its own. The declared path is the default deployment spelling;
/// [`meta_openapi`] re-homes it.
#[utoipa::path(
    get, path = "/ferroehr/rest/api-docs/openapi.json", tag = "openapi",
    responses((status = 200, description = "The complete server-surface `OpenAPI` document (JSON).", body = serde_json::Value))
)]
async fn openapi_json(State(state): State<AppState>) -> Response {
    let doc = extensions_document(state.config());
    let body = serde_json::to_string(&doc).unwrap_or_else(|_| "{}".to_owned());
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// Answers the Swagger UI mount path with a `307` to `index.html` under it.
///
/// No openEHR spec governs a Swagger UI — our own discoverability surface.
/// Public, and config-gated by the Swagger UI (a router `404` when disabled).
/// The sibling asset route serves the UI bundle. The spec selector offers one
/// document per API family plus the complete document last, each filtered from
/// the one server-generated composed document.
#[utoipa::path(
    get, path = "/ferroehr/rest/swagger-ui", tag = "openapi",
    responses((status = 307, description = "Redirect to `index.html` under the mount path, \
        which is where the UI's relative asset URLs resolve."))
)]
async fn swagger_ui_index(State(state): State<AppState>) -> Response {
    redirect_to_index(&state.config().server.swagger_ui_path())
}

/// Redirects to `index.html` under the mount.
///
/// The dist `index.html` references its assets relatively, so serving it at the
/// slash-less mount path renders an empty page. The target names `index.html`
/// rather than the trailing-slash form, which `NormalizePathLayer` would strip
/// straight back to this route
/// (<https://docs.rs/tower-http/latest/tower_http/normalize_path/index.html>).
fn redirect_to_index(ui_path: &str) -> Response {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(header::LOCATION, format!("{ui_path}/index.html"))],
    )
        .into_response()
}

/// Declares one spec-selector family document.
///
/// No openEHR spec governs an OAS-serving endpoint — our own discoverability
/// surface. Public, and config-gated by the Swagger UI. Each document is the one
/// server-generated composed document filtered to a single API family, so it can
/// never drift from the router.
///
/// `family` is a fixed set, not a free parameter: the live routes are one static
/// route per family, because an axum path capture spans a whole segment and
/// cannot match part of the `ferroehr-….openapi.json` filename. Any other value
/// is not routed (`404`). This one parameterized declaration documents them all;
/// the complete composed document is [`openapi_json`].
#[utoipa::path(
    get, path = "/ferroehr/rest/api-docs/ferroehr-{family}.openapi.json", tag = "openapi",
    params(("family" = String, Path,
        description = "The API family slug — one of the fixed set `ehr`, `query`, `definition`, \
                       `demographic`, `admin`, `management`, `terminology`, `relationships`, \
                       `messaging`, `events`, `audit`, `smart`. Not a free parameter: each value is \
                       its own static route.")),
    responses(
        (status = 200, description = "The composed server document filtered to that API family (JSON).", body = serde_json::Value),
        (status = 404, description = "Not a known family slug (no such route), or the Swagger UI is disabled.")
    )
)]
#[expect(
    dead_code,
    reason = "the documentation twin of a live route: the served routes are the \
              twelve static per-family routes built in `swagger_router`, so only \
              the `#[utoipa::path]` attribute on this stub is consumed"
)]
fn family_openapi_json() {}

/// Returns the OAS meta-endpoints' `OpenAPI`, every path re-homed to the one the
/// live router mounts under `cfg` ([`rehome_path`]).
fn meta_openapi(cfg: &AppConfig) -> utoipa::openapi::OpenApi {
    #[derive(OpenApi)]
    #[openapi(paths(family_openapi_json))]
    struct FamilyDocs;

    // These paths exist only when the Swagger surface is mounted, and the
    // generator's documented property is that every path it lists is one the
    // live router mounts.
    if !cfg.server.swagger_ui.is_mounted() {
        return utoipa::openapi::OpenApiBuilder::new().build();
    }

    let mut doc = OpenApiRouter::<AppState>::new()
        .routes(routes!(openapi_json))
        .routes(routes!(swagger_ui_index))
        .into_openapi();
    doc.merge(FamilyDocs::openapi());

    let json_path = cfg.server.openapi_json_path();
    let docs_root = api_docs_root(&json_path);
    rehome_path(&mut doc, "/ferroehr/rest/api-docs/openapi.json", &json_path);
    rehome_path(
        &mut doc,
        "/ferroehr/rest/api-docs/ferroehr-{family}.openapi.json",
        &format!("{docs_root}/ferroehr-{{family}}.openapi.json"),
    );
    rehome_path(
        &mut doc,
        "/ferroehr/rest/swagger-ui",
        &cfg.server.swagger_ui_path(),
    );
    doc
}

// ── The spec-selector families ────────────────────────────────────────────────

/// How a spec-selector family picks its operations out of the one composed
/// server document.
///
/// The standardised ITS-REST groups categorise by resource path: their
/// operations carry per-resource tags, exactly as the vendored group OAS
/// documents tag them, so the base-path-relative path root identifies the group
/// rather than a shared tag. The server's own extensions have no shared path
/// root and are identified by tag.
enum Members {
    /// Operations whose base-path-relative path starts with `include` (on a
    /// segment boundary) and starts with none of `exclude`, PLUS any operation
    /// carrying one of `also_tagged`.
    Path {
        include: &'static str,
        exclude: &'static [&'static str],
        /// Operations belonging to this family although their path is outside
        /// `include`: the group's own-design routes, which sit under sibling
        /// paths but are part of the same API group.
        also_tagged: &'static [&'static str],
    },
    /// Operations carrying one of these tags.
    Tags(&'static [&'static str]),
}

/// Every API family offered as its own selector document, as
/// `(selector name, url slug, membership criterion)`.
///
/// All are filtered from the one server-generated composed document, so each
/// inherits the config-driven security scheme and can never drift from the
/// router.
const FAMILIES: &[(&str, &str, Members)] = &[
    // The standardised ITS-REST groups (our generated wire), by resource path.
    (
        "openEHR — EHR",
        "ehr",
        Members::Path {
            include: "/ehr",
            exclude: &[],
            also_tagged: &[],
        },
    ),
    (
        "openEHR — Query",
        "query",
        Members::Path {
            include: "/query",
            exclude: &[],
            also_tagged: &[],
        },
    ),
    (
        "openEHR — Definition",
        "definition",
        Members::Path {
            include: "/definition",
            exclude: &[],
            also_tagged: &[],
        },
    ),
    (
        "openEHR — Demographic",
        "demographic",
        Members::Path {
            include: "/demographic",
            // The own-design PARTY_RELATIONSHIP extension shares the /demographic
            // root but is its own family (below).
            exclude: &["/demographic/party_relationship"],
            also_tagged: &[],
        },
    ),
    (
        "openEHR — Admin",
        "admin",
        Members::Path {
            include: "/admin/ehr",
            exclude: &[],
            // The ADMIN group's own-design routes live under sibling
            // `/admin/*` paths rather than `/admin/ehr`, but belong in this
            // group's document.
            also_tagged: &[
                "ADMIN",
                "admin-report",
                "admin-archive",
                "admin-dump-load",
                "admin-integrity",
                "admin-marks",
            ],
        },
    ),
    // The server's own extension families, by tag.
    (
        "FerroEHR — Status & Management",
        "management",
        // `system` is the ITS-REST System Options-and-Conformance manifest, the
        // one released operation with no resource path of its own.
        Members::Tags(&["status", "management", "openapi", "system"]),
    ),
    (
        "FerroEHR — Terminology",
        "terminology",
        Members::Tags(&["terminology"]),
    ),
    (
        "FerroEHR — Party Relationships",
        "relationships",
        Members::Tags(&["demographic-relationship"]),
    ),
    (
        "FerroEHR — Messaging",
        "messaging",
        // The SM MESSAGE component on our own `/message/` routes; the release
        // publishes no message API, so the whole family is an extension.
        Members::Tags(&["message"]),
    ),
    (
        "FerroEHR — Event Subscriptions",
        "events",
        Members::Tags(&["event-subscription"]),
    ),
    (
        "FerroEHR — Audit",
        "audit",
        // The ITI-81 AuditEvent retrieval under `/fhir/r4`, gated by the local
        // audit repository.
        Members::Tags(&["audit"]),
    ),
    (
        "FerroEHR — SMART Discovery",
        "smart",
        Members::Tags(&["smart"]),
    ),
];

/// Whether `rel` equals `prefix` or continues past it on a path-segment boundary
/// (so `/ehr` matches `/ehr` and `/ehr/{id}`, never a hypothetical `/ehrx`).
fn on_segment_boundary(rel: &str, prefix: &str) -> bool {
    rel == prefix
        || rel
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with('/'))
}

/// Returns a copy of `doc` keeping the paths whose base-path-relative form is
/// under `include` and under none of `exclude`, plus the individual operations
/// tagged with one of `also_tagged`.
///
/// A matched resource path is kept whole, since an ITS-REST resource path
/// belongs entirely to one group; the `also_tagged` operations are picked out
/// individually.
fn filter_by_path(
    doc: &utoipa::openapi::OpenApi,
    base_path: &str,
    include: &str,
    exclude: &[&str],
    also_tagged: &[&str],
) -> utoipa::openapi::OpenApi {
    let mut out = doc.clone();
    out.paths.paths.retain(|path, item| {
        let in_group = path.strip_prefix(base_path).is_some_and(|rel| {
            on_segment_boundary(rel, include)
                && !exclude.iter().any(|ex| on_segment_boundary(rel, ex))
        });
        if in_group {
            return true;
        }
        retain_tagged_operations(item, also_tagged)
    });
    prune_tags(&mut out);
    out
}

/// Drops from `item` every operation not carrying one of `tags`, and reports
/// whether any operation survived.
fn retain_tagged_operations(item: &mut utoipa::openapi::path::PathItem, tags: &[&str]) -> bool {
    for op in [
        &mut item.get,
        &mut item.put,
        &mut item.post,
        &mut item.delete,
        &mut item.options,
        &mut item.head,
        &mut item.patch,
        &mut item.trace,
    ] {
        if let Some(inner) = op
            && !inner
                .tags
                .as_ref()
                .is_some_and(|t| t.iter().any(|t| tags.contains(&t.as_str())))
        {
            *op = None;
        }
    }
    item.get.is_some()
        || item.put.is_some()
        || item.post.is_some()
        || item.delete.is_some()
        || item.options.is_some()
        || item.head.is_some()
        || item.patch.is_some()
        || item.trace.is_some()
}

/// A copy of `doc` keeping only the operations tagged with one of `tags`
/// (paths whose every operation is filtered away are dropped entirely).
fn filter_by_tags(doc: &utoipa::openapi::OpenApi, tags: &[&str]) -> utoipa::openapi::OpenApi {
    let mut out = doc.clone();
    out.paths
        .paths
        .retain(|_, item| retain_tagged_operations(item, tags));
    prune_tags(&mut out);
    out
}

/// Drop tag declarations no retained operation carries, so a family document's
/// tag list shows exactly the resource groups it contains.
fn prune_tags(doc: &mut utoipa::openapi::OpenApi) {
    let mut used = std::collections::BTreeSet::new();
    for item in doc.paths.paths.values() {
        for op in [
            &item.get,
            &item.put,
            &item.post,
            &item.delete,
            &item.options,
            &item.head,
            &item.patch,
            &item.trace,
        ] {
            if let Some(inner) = op
                && let Some(tags) = &inner.tags
            {
                for t in tags {
                    used.insert(t.clone());
                }
            }
        }
    }
    if let Some(doc_tags) = &mut doc.tags {
        doc_tags.retain(|t| used.contains(&t.name));
    }
}

/// Every served `OpenAPI` document, pre-serialized **once** at router assembly.
///
/// The composed document and every family document are pure functions of the
/// static [`AppConfig`], so building them per request re-ran the full
/// `utoipa` reflection (all API groups + the auth walk) and, for each family,
/// an additional whole-document deep clone + filter. They are computed once here
/// and served as ready [`Bytes`], so a request is a clone-free body write.
struct PrebuiltDocs {
    /// The complete composed document as serialized JSON.
    full: Bytes,
    /// One filtered family document per [`FAMILIES`] entry, in the same order.
    families: Vec<Bytes>,
}

/// Build the composed document and every family document once, serializing each
/// to [`Bytes`] (the filter machinery — [`filter_by_path`]/[`filter_by_tags`] —
/// is applied here rather than per request).
fn prebuild_docs(cfg: &AppConfig) -> PrebuiltDocs {
    let full_doc = extensions_document(cfg);
    let full = to_json_bytes(&full_doc);

    let families = FAMILIES
        .iter()
        .map(|(name, _, members)| {
            let mut doc = match members {
                Members::Path {
                    include,
                    exclude,
                    also_tagged,
                } => filter_by_path(
                    &full_doc,
                    &cfg.server.base_path,
                    include,
                    exclude,
                    also_tagged,
                ),
                Members::Tags(tags) => filter_by_tags(&full_doc, tags),
            };
            doc.info.title = (*name).to_string();
            to_json_bytes(&doc)
        })
        .collect();

    PrebuiltDocs { full, families }
}

/// Serialize an `OpenAPI` document to JSON [`Bytes`] (empty object on the
/// unreachable serialization error, mirroring the previous per-request path).
fn to_json_bytes(doc: &utoipa::openapi::OpenApi) -> Bytes {
    Bytes::from(serde_json::to_vec(doc).unwrap_or_else(|_| b"{}".to_vec()))
}

/// Serve a pre-serialized document: a clone-free (ref-counted) [`Bytes`] body
/// write with the JSON content type.
fn json_document_response(body: Bytes) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

// ── The Swagger router (serving; kept on the loop-free `serve()` mechanism) ───

/// Builds the docs router: the Swagger UI, the complete `openapi.json`, and one
/// filtered JSON document per API family, all server-generated. Config paths come
/// from [`AppConfig`].
pub(crate) fn swagger_router(cfg: &AppConfig) -> Router<AppState> {
    let ui_path = cfg.server.swagger_ui_path();
    let json_path = cfg.server.openapi_json_path();
    let api_docs_root = api_docs_root(&json_path);

    // Built once — they are pure functions of static configuration — so a
    // request serves ready bytes and never re-runs utoipa reflection.
    let docs = prebuild_docs(cfg);

    let full = docs.full;
    let mut router = Router::new().route(
        &json_path,
        get(move || {
            let body = full.clone();
            async move { json_document_response(body) }
        }),
    );

    // One static route per family: an axum path capture spans a whole segment,
    // so a `{family}` inside the `ferroehr-….openapi.json` filename cannot be a
    // route parameter.
    for ((_, slug, _), body) in FAMILIES.iter().zip(docs.families) {
        router = router.route(
            &format!("{api_docs_root}/ferroehr-{slug}.openapi.json"),
            get(move || {
                let body = body.clone();
                async move { json_document_response(body) }
            }),
        );
    }

    // Assets straight from the embedded dist; the spec-selector config is
    // config-static, built once and shared by the mount path and the asset path.
    let config = swagger_config(cfg);
    let redirect_target = ui_path.clone();
    router
        .route(
            &ui_path,
            get(move || {
                let target = redirect_target.clone();
                async move { redirect_to_index(&target) }
            }),
        )
        .route(
            &format!("{ui_path}/{{*file}}"),
            get(move |Path(file): Path<String>| {
                let config = Arc::clone(&config);
                async move { serve_ui_file(&file, &config) }
            }),
        )
        // Swagger UI is genuinely rendered, so the API's `default-src 'none'`
        // would blank it; everything it loads is same-origin, so `'self'`
        // suffices with no inline allowance. The outer layer uses
        // `if_not_present`, so this narrower policy survives.
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            http::HeaderValue::from_static(SWAGGER_UI_CSP),
        ))
}

/// The Content-Security-Policy for the Swagger UI surface.
const SWAGGER_UI_CSP: &str = "default-src 'self'; \
     script-src 'self'; \
     style-src 'self'; \
     img-src 'self' data:; \
     font-src 'self'; \
     connect-src 'self'; \
     object-src 'none'; \
     base-uri 'self'; \
     frame-ancestors 'none'";

/// Returns the `api-docs` directory the documents live under, the parent of the
/// configured `openapi.json` path.
fn api_docs_root(json_path: &str) -> String {
    json_path
        .rsplit_once('/')
        .map_or("/api-docs", |(dir, _)| dir)
        .to_owned()
}

/// Builds the Swagger UI spec-selector config: one entry per API family, with
/// the complete composed document last.
///
/// URLs must outlive the server, so the derived paths are leaked once at
/// construction (the UI config requires `'static`).
fn swagger_config(cfg: &AppConfig) -> Arc<Config<'static>> {
    let json_path = cfg.server.openapi_json_path();
    let root = api_docs_root(&json_path);

    let mut urls: Vec<Url<'static>> = Vec::new();
    for (name, slug, _) in FAMILIES {
        let url = format!("{root}/ferroehr-{slug}.openapi.json");
        urls.push(Url::new(name, url.leak()));
    }
    urls.push(Url::new("FerroEHR — Complete surface", json_path.leak()));
    Arc::new(Config::new(urls))
}

/// Serves one embedded Swagger UI asset, mapping `""` to `index.html`.
fn serve_ui_file(file: &str, config: &Arc<Config<'static>>) -> Response {
    match utoipa_swagger_ui::serve(file, Arc::clone(config)) {
        Ok(Some(SwaggerFile {
            bytes,
            content_type,
            ..
        })) => ([(header::CONTENT_TYPE, content_type)], bytes.into_owned()).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
