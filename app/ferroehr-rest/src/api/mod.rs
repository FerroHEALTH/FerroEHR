// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST **API hub** — the assembly over the generated `ROUTES` tables.
//!
//! The crate is laid out per ITS-REST specification: one module per API group under
//! `api/`, each split along the spec's own resource boundaries and citing its
//! governing section. This hub is the wiring layer that turns the generated
//! contract into one axum router.
//!
//! ## The generated contract
//!
//! `emit-rest` produces, per API group, a `ROUTES` table of
//! `(method, path, operation_id)` and one `*Params` struct per operation
//! (`openehr_its::rest::generated::{ehr,query,definition,admin,demographic}`).
//! The System API is not part of that contract (no `system` group is emitted),
//! so its single `OPTIONS /` operation is hand-written in [`system`] and mounted
//! by [`crate::router::router`], not here.
//!
//! ## The per-group modules
//!
//! Each group owns a dispatcher — an operation-id `match` that rebuilds the
//! exact `*Params` from the raw request parts, calls the service method on
//! [`AppState`], and renders a negotiated response:
//!
//! - [`ehr`] — the EHR API (EHR, `EHR_STATUS`, `VERSIONED_EHR_STATUS`, COMPOSITION,
//!   `VERSIONED_COMPOSITION`, DIRECTORY, CONTRIBUTION), re-exporting
//!   `ehr::dispatch::dispatch`.
//! - [`query`] — the Query API (adhoc + stored), re-exporting `query::dispatch`.
//! - [`definition`] — the Definition API (ADL 1.4 / ADL 2 templates + stored
//!   queries), re-exporting `definition::dispatch`.
//! - [`demographic`] — the Demographic API (spec-governed, DEVELOPMENT) plus the
//!   own-design `PARTY_RELATIONSHIP` extension routes, re-exporting
//!   `demographic::dispatch` + `demographic::relationship_routes` (the native
//!   `utoipa-axum` router for the extension).
//! - [`admin`] — the Admin API (physical EHR delete), whose dispatcher is reached
//!   as `admin::dispatch::dispatch` (the group publishes no re-export), plus the
//!   own-design `archive` + activity-`report` + `dump_load` extension routes
//!   under the same `/admin/` gates.
//! - [`message`] — the own-design MESSAGE group (EHR Extract export/import);
//!   the release publishes no message API at all, so the whole group is an
//!   extension under its own `/message/` resource root.
//! - [`system`] — the System API manifest ([`system::options::SystemManifest`],
//!   `ferroehr::config::rest::SystemOptionsConfig`), assembled and mounted by [`crate::router::router`].
//!
//! `item_tags` is not a group: it is the one `ITEM_TAG` write-wrapper
//! implementation (`item_tags::pending` / `item_tags::persist` /
//! `item_tags::echo` / `item_tags::write_body`) the EHR and demographic
//! dispatchers share, so the two cannot drift.
//!
//! Every group — the standardised ITS-REST groups above and the own-design
//! extension surfaces ([`crate::extensions`]: terminology, event subscription,
//! the ITI-81 audit retrieval) — is built as a native `utoipa-axum`
//! router ([`OpenApiRouter`] + `routes!`), so each operation's route and its
//! `OpenAPI` path metadata come from ONE `#[utoipa::path]` handler. They are
//! composed by `api_openapi_router`; the extension groups are config-gated
//! inside each handler (a disabled group answers `404`), so a stock server
//! exposes only the standardised ITS-REST surface.

pub mod admin;
pub mod definition;
pub mod demographic;
pub mod ehr;
pub(crate) mod item_tags;
pub mod message;
pub mod query;
pub(crate) mod served;
pub mod system;

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;

use axum::Router;
use axum::extract::{FromRequestParts, RawPathParams};
use axum::response::Response;
use bytes::Bytes;
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use openehr_its::rest::routes::{Lookup, Param, ParamLocation, PathParam, RouteMatch, lookup};
use openehr_its::rest::runtime::ApiError;
use utoipa_axum::router::OpenApiRouter;

use crate::extensions::access::{delay, ehr_access, pep, restriction};
#[cfg(feature = "events")]
use crate::extensions::event_subscription;
#[cfg(feature = "fhir")]
use crate::extensions::fhir;
use crate::extensions::terminology;
use crate::state::AppState;

/// A boxed response future — the uniform return type of a group dispatcher.
pub(crate) type BoxResponse = Pin<Box<dyn Future<Output = Response> + Send>>;

/// A group dispatcher: `(state, operation_id, request parts) → response`.
type Dispatcher = fn(AppState, &'static str, RequestParts) -> BoxResponse;

/// The raw request sources a dispatcher needs to rebuild a `*Params` struct and
/// decode a body.
pub(crate) struct RequestParts {
    pub(crate) path: IndexMap<String, String>,
    pub(crate) query: Option<String>,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Bytes,
    /// The ITS-REST operation the method and group-relative path address
    /// (`openehr_its::rest::routes::lookup`), `None` on an extension route the
    /// ITS-REST route tables do not name.
    pub(crate) route: Option<RouteMatch>,
}

impl RequestParts {
    /// The route match a generated `*Params::from_request` decodes against.
    ///
    /// For an ITS-REST operation it is the match `routes::lookup` found. An
    /// extension route the tables do not name (the `PARTY_RELATIONSHIP` and admin
    /// extensions) reuses a generated struct, so its match is built from that
    /// struct's declared path parameters (`params`) and the request's captures,
    /// which is all `from_request` reads from it.
    /// Decodes a generated `*Params` struct from this request with its
    /// `from_request` and its declared `params`, over [`Self::route_for`].
    ///
    /// # Errors
    /// Returns the [`ApiError`] the struct's decoding reports.
    pub(crate) fn decode<P>(
        &self,
        from_request: fn(&RouteMatch, Option<&str>, &HeaderMap) -> Result<P, ApiError>,
        params: &'static [Param],
    ) -> Result<P, ApiError> {
        from_request(
            &self.route_for(params),
            self.query.as_deref(),
            &self.headers,
        )
    }

    pub(crate) fn route_for(&self, params: &'static [Param]) -> Cow<'_, RouteMatch> {
        if let Some(route) = &self.route {
            return Cow::Borrowed(route);
        }
        let path_params = params
            .iter()
            .filter(|p| p.location == ParamLocation::Path)
            .filter_map(|p| {
                self.path.get(p.name).map(|raw| PathParam {
                    name: p.name,
                    raw: raw.clone(),
                })
            })
            .collect();
        Cow::Owned(RouteMatch {
            group: "extension",
            operation_id: "extension",
            template: "",
            method: Method::GET,
            path_params,
            params,
            request_media: &[],
        })
    }
}

/// Decompose a whole axum [`Request`](axum::extract::Request) into the
/// [`RequestParts`] snapshot a group dispatcher consumes.
///
/// Every native `utoipa-axum` handler takes `State` + the whole `Request` and
/// funnels through here: taking the entire request (rather than the individual
/// `RawPathParams`/`RawQuery`/`HeaderMap`/`Bytes` extractors) is what keeps the
/// `#[utoipa::path]` argument introspection happy — `bytes::Bytes` has no
/// `ToSchema`, so a bare body extractor cannot appear in a documented handler
/// signature. The upstream `RequestBodyLimitLayer` has already bounded the body,
/// so the `to_bytes` limit here is only a backstop.
pub(crate) async fn into_parts(request: axum::extract::Request) -> RequestParts {
    let (mut parts, body) = request.into_parts();
    let path: IndexMap<String, String> = RawPathParams::from_request_parts(&mut parts, &())
        .await
        .map(|raw| {
            raw.iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect()
        })
        .unwrap_or_default();
    let query = parts.uri.query().map(str::to_owned);
    // The router nests this API under the base path, so `parts.uri` is already
    // group-relative, which is the path `lookup` matches.
    let route = match lookup(&parts.method, parts.uri.path()) {
        Lookup::Matched(route) => Some(route),
        Lookup::MethodNotAllowed { .. } | Lookup::NotFound => None,
    };
    let headers = parts.headers;
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    RequestParts {
        path,
        query,
        headers,
        body,
        route,
    }
}

/// The three API namespaces beside each other under the REST root (#4330):
/// ITS-REST under the openEHR base path, FerroEHR's own extensions under
/// `{rest_root}/extensions/v1`, and the standard FHIR and IHE interfaces under
/// `{rest_root}/fhir/r4`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Namespaces {
    /// The openEHR base path (`[rest] base_path`).
    pub(crate) openehr: String,
    /// The root of FerroEHR's own extensions.
    pub(crate) extensions: String,
    /// The REST root the FHIR routes are mounted at; their paths carry
    /// [`FHIR_PREFIX`].
    pub(crate) root: String,
}

impl Namespaces {
    /// The namespaces of the REST configuration `rest`.
    pub(crate) fn of(rest: &ferroehr::config::rest::RestConfig) -> Self {
        Self {
            openehr: rest.base_path.clone(),
            extensions: rest.extensions_path(),
            root: rest.rest_root(),
        }
    }

    /// A matched route's path relative to the namespace it sits in: below the
    /// openEHR base path for an ITS-REST route (`/ehr/{ehr_id}`), below the
    /// extensions root for an extension (`/admin/config`, `/terminology`), and
    /// below the REST root for a FHIR route (`/fhir/r4/AuditEvent`); `None`
    /// outside all three.
    #[must_use]
    pub(crate) fn relative<'a>(&self, matched: &'a str) -> Option<&'a str> {
        let below = |root: &str| {
            matched
                .strip_prefix(root)
                .filter(|rest| rest.is_empty() || rest.starts_with('/'))
        };
        below(&self.openehr)
            .or_else(|| below(&self.extensions))
            .or_else(|| {
                below(&self.root).filter(|rest| {
                    rest.strip_prefix(FHIR_PREFIX)
                        .is_some_and(|tail| tail.is_empty() || tail.starts_with('/'))
                })
            })
    }
}

/// The path prefix of the FHIR and IHE interfaces below the REST root.
pub(crate) const FHIR_PREFIX: &str = "/fhir/r4";

/// Builds the API router: the ITS-REST routes under the openEHR base path,
/// FerroEHR's own extensions under the extensions root, and the FHIR routes
/// under the REST root, all at their full server paths. State is applied by
/// the caller. This is the `Router` half of [`its_openapi_router`],
/// [`extension_openapi_router`] and [`fhir_openapi_router`]; the `OpenApi`
/// half feeds the served document ([`api_doc`]), so route and documentation
/// are single-sourced.
pub(crate) fn api_router(namespaces: &Namespaces) -> Router<AppState> {
    let its: Router<AppState> = its_openapi_router().into();
    let extensions: Router<AppState> = extension_openapi_router().into();
    let fhir: Router<AppState> = fhir_openapi_router().into();
    let mut router = Router::new()
        .nest(&namespaces.openehr, its)
        .merge(Router::new().nest(&namespaces.extensions, extensions));
    if fhir.has_routes() {
        router = router.merge(Router::new().nest(&namespaces.root, fhir));
    }
    router
}

/// The ITS-REST groups (EHR, DEMOGRAPHIC, DEFINITION, QUERY, ADMIN) as one
/// native `utoipa-axum` router (group-relative paths), so each operation's
/// route and its `OpenAPI` path metadata come from ONE `#[utoipa::path]`
/// handler; no vendored OAS served, no table-driven synthesis. Every handler
/// forwards to its group dispatcher through [`guarded_dispatch`], so the auth,
/// ATNA-audit and ABAC stack is applied identically across the whole surface
/// (the operation contract each dispatcher implements is generated by
/// `emit-rest` from the vendored OAS, the codegen input, never served).
///
/// Only the paths ITS-REST 1.1.0 defines are here: everything under the
/// openEHR base path reads as the openEHR REST API, so a FerroEHR route there
/// would claim to be part of it (#4330). The test
/// `api::tests::each_namespace_holds_only_its_own_routes` holds it.
pub(crate) fn its_openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(ehr::openapi_routes::routes())
        .merge(demographic::openapi_routes::routes())
        .merge(definition::openapi_routes::routes())
        .merge(query::openapi_routes::routes())
        .merge(admin::openapi_routes::routes())
}

/// FerroEHR's own routes, which ITS-REST does not define and which are no FHIR
/// or IHE interface, as one native `utoipa-axum` router (paths relative to the
/// extensions root): the ADMIN extensions, the archetype and artefact library,
/// `PARTY_RELATIONSHIP`, the EHR Extract messages, the cohort query,
/// terminology and event subscriptions. They are mounted under
/// `{rest_root}/extensions/v1`, beside the openEHR base path, on their own
/// version line.
///
/// The extension groups are always mounted; their config gate is enforced
/// inside the handler (a disabled group answers `404`). No openEHR spec governs
/// an OAS layout, the extension surfaces or where they are served: our own
/// design.
pub(crate) fn extension_openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(ehr::openapi_routes::extension_routes())
        .merge(demographic::relationship::relationship_routes())
        .merge(definition::archetype::archetype_routes())
        .merge(query::openapi_routes::cohort_routes())
        .merge(admin::openapi_routes::extension_routes())
        .merge(admin::archive::archive_routes())
        .merge(admin::report::report_routes())
        .merge(admin::dump_load::dump_load_routes())
        .merge(admin::integrity::integrity_routes())
        .merge(admin::marks::marks_routes())
        .merge(admin::access_delay::access_delay_routes())
        .merge(admin::identifiers::identifier_routes())
        .merge(message::extract::extract_routes())
        .merge(terminology::routes())
        .merge({
            #[cfg(feature = "events")]
            {
                event_subscription::routes()
            }
            #[cfg(not(feature = "events"))]
            {
                OpenApiRouter::new()
            }
        })
}

/// The standard FHIR and IHE interfaces: the `RESTful` ATNA ITI-81 retrieval,
/// whose paths carry [`FHIR_PREFIX`] and which is mounted at the REST root,
/// where a FHIR client or an IHE Audit Consumer expects `[base]/AuditEvent`.
pub(crate) fn fhir_openapi_router() -> OpenApiRouter<AppState> {
    #[cfg(feature = "fhir")]
    {
        fhir::routes()
    }
    #[cfg(not(feature = "fhir"))]
    {
        OpenApiRouter::new()
    }
}

/// The full API surface's `OpenAPI` document at full server paths: the
/// ITS-REST groups under the openEHR base path
/// (`/ferroehr/rest/openehr/v1/ehr`), the extensions under the extensions root
/// (`/ferroehr/rest/extensions/v1/terminology`) and the FHIR routes under the
/// REST root (`/ferroehr/rest/fhir/r4/AuditEvent`). Built from the same
/// routers that mount the routes, so the document cannot drift from the
/// router.
pub(crate) fn api_doc(namespaces: &Namespaces) -> utoipa::openapi::OpenApi {
    let mut doc = OpenApiRouter::<AppState>::new()
        .nest(&namespaces.openehr, its_openapi_router())
        .into_openapi();
    doc.merge(
        OpenApiRouter::<AppState>::new()
            .nest(&namespaces.extensions, extension_openapi_router())
            .into_openapi(),
    );
    doc.merge(
        OpenApiRouter::<AppState>::new()
            .nest(&namespaces.root, fhir_openapi_router())
            .into_openapi(),
    );
    doc
}

/// Run one operation through the uniform per-request stack the extension
/// handlers and `mount` share: the spec-grounded `EHR_ACCESS` gate first (RM
/// `org.openehr.rm.ehr.ehr_access.adoc` — "All access decisions to data in the
/// EHR must be made in accordance with the policies and rules in this object"),
/// then the ABAC PEP pre-check (short-circuits before the backend on a deny),
/// the dispatcher, the PEP post-check (may replace a success with `403`/`500`),
/// and finally tag the response with the matched operation id for the ATNA audit
/// layer. ABAC/`EHR_ACCESS` are inert unless wired (default-open); no openEHR
/// spec governs the ABAC layer — our own extension.
pub(crate) async fn guarded_dispatch(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
    dispatch: Dispatcher,
) -> Response {
    // NOTE: EHDS Art. 8 (`docs/law/eu/ehds/text.html`), restricted data reads as
    // absent to every operation but the controller's own and an emergency access.
    let enforced = restriction::applies(&state, op);
    // NOTE: EHDS Art. 3(3) (`docs/law/eu/ehds/text.html`), data a professional has not
    // released reads as absent to a patient-context request on a deployment that delays.
    let delayed = delay::applies(&state, op);
    let guarded = Box::pin(async move {
        match ehr_access::enforce(&state, op, &parts).await {
            Ok(()) => {
                if enforced {
                    match restriction::refuse_restricted(&state, op, &parts).await {
                        Ok(Some(absent)) => return absent,
                        Ok(None) => {}
                        Err(fault) => return *fault,
                    }
                }
                if delayed {
                    match delay::refuse_withheld(&state, op, &parts).await {
                        Ok(Some(absent)) => return absent,
                        Ok(None) => {}
                        Err(fault) => return *fault,
                    }
                }
                match pep::pre_check(&state, op, &parts).await {
                    Ok(()) => {
                        let resp = dispatch(state.clone(), op, parts).await;
                        pep::post_check(&state, op, resp).await
                    }
                    Err(deny) => deny,
                }
            }
            Err(deny) => *deny,
        }
    });
    let mut resp = ferroehr::system_log::access_context::with_access_restriction(
        enforced,
        ferroehr::system_log::access_context::with_access_delay(delayed, guarded),
    )
    .await;
    resp.extensions_mut()
        .insert(crate::system_log::middleware::AuditOpId(op));
    resp
}

/// Strip RFC 6570 query/expansion suffixes (e.g. `all{?ehr_id*}` → `all`) that
/// the `OpenAPI` paths carry but axum path templates do not use; `{name}` capture
/// segments are already axum-0.8 syntax and pass through unchanged.
///
/// Shared with the access-control classifier
/// ([`crate::extensions::access::authz`]), which keys its route→class map by the
/// same normalized templates so an axum `MatchedPath` resolves exactly.
pub(crate) fn normalize_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        // A `{` immediately followed by an RFC 6570 operator opens a
        // query/path-style expansion that is not part of the resource path.
        if c == '{'
            && chars
                .peek()
                .is_some_and(|n| matches!(n, '?' | '&' | '#' | '.' | ';' | '/' | '+'))
        {
            // Skip to and including the matching `}`.
            for skipped in chars.by_ref() {
                if skipped == '}' {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_query_expansion() {
        assert_eq!(normalize_path("/admin/ehr/all{?ehr_id*}"), "/admin/ehr/all");
        assert_eq!(normalize_path("/ehr/{ehr_id}"), "/ehr/{ehr_id}");
        assert_eq!(
            normalize_path("/ehr/{ehr_id}/composition/{uid_based_id}"),
            "/ehr/{ehr_id}/composition/{uid_based_id}"
        );
        assert_eq!(
            normalize_path("/definition/template/adl1.4"),
            "/definition/template/adl1.4"
        );
    }

    /// The served router covers every route of the generated ITS-REST tables.
    ///
    /// `RbacGate` (`extensions::access::authz`) builds its `(Method, path) →
    /// OperationClass` map from the five generated `ROUTES` tables and looks a
    /// request up by axum's `MatchedPath`. That indirection is only sound while
    /// the served templates and the generated ones are the SAME strings: a
    /// hand-written `#[utoipa::path]` that spells a parameter differently
    /// silently falls through to the gate's default class instead of failing a
    /// build. This test makes the generated tables a proven oracle for the
    /// served surface.
    ///
    /// Scope: the five API groups `emit-rest` generates. The SYSTEM group's one
    /// released operation (`OPTIONS` at the API base-path root,
    /// `crates/openehr-its/vendor/rest-oas/system-codegen.openapi.yaml`
    /// §`paths./.options`) has no generated `ROUTES` table and is mounted
    /// outside [`its_openapi_router`] (on the base path itself), so it is
    /// pinned by its own assertion below rather than by the table sweep.
    #[test]
    fn served_router_covers_every_generated_route() {
        // A concrete base path: `api_doc` nests the group router, which requires
        // a leading `/` (the served document always carries one).
        const BASE: &str = "/base";
        let doc = api_doc(&Namespaces {
            openehr: BASE.to_owned(),
            extensions: "/root/extensions/v1".to_owned(),
            root: "/root".to_owned(),
        });
        let served = |template: &str, method: &str| {
            doc.paths
                .paths
                .get(template)
                .is_some_and(|item| match method {
                    "GET" => item.get.is_some(),
                    "PUT" => item.put.is_some(),
                    "POST" => item.post.is_some(),
                    "DELETE" => item.delete.is_some(),
                    "OPTIONS" => item.options.is_some(),
                    "HEAD" => item.head.is_some(),
                    "PATCH" => item.patch.is_some(),
                    "TRACE" => item.trace.is_some(),
                    _ => false,
                })
        };
        let missing: Vec<String> = [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
        ]
        .into_iter()
        .flatten()
        .filter(|(method, path, _op)| !served(&format!("{BASE}{}", normalize_path(path)), method))
        .map(|(method, path, op)| format!("{method} {} ({op})", normalize_path(path)))
        .collect();
        assert!(
            missing.is_empty(),
            "generated ITS-REST routes with no served counterpart: {missing:#?}"
        );

        // The SYSTEM group: served from `crate::api::system::options`, declared
        // at whatever base path the deployment mounts.
        let system = system::options::openapi(BASE);
        assert!(
            system
                .paths
                .paths
                .get(BASE)
                .is_some_and(|item| item.options.is_some()),
            "the released SYSTEM `OPTIONS` operation is served at the API base path"
        );

        // The detector itself, so the sweep above cannot pass vacuously: a
        // drifted template (here, a mis-spelled path parameter) is NOT served,
        // which is exactly the drift the sweep would report.
        assert!(
            !served(&format!("{BASE}/ehr/{{ehr_id_drifted}}"), "GET"),
            "a mis-spelled route template must not match a served path"
        );
    }

    /// The FHIR R4 resource types served under the FHIR root: the ITI-81
    /// retrieval of `RESTful` ATNA §3.81.4.1.2, a FHIR search on `AuditEvent`.
    const FHIR_RESOURCES: [&str; 1] = ["AuditEvent"];

    /// The ITS-REST operation `definition_query_list` with an empty pattern:
    /// "when is empty, it will be treated as wildcard"
    /// (`docs/specs/openehr/ITS-REST/specifications/operations/definition_query_list.yaml`).
    const EMPTY_PATTERN_ROUTES: [(&str, &str); 1] = [("GET", "/definition/query")];

    /// Each of the three namespaces holds only its own routes (#4330): under
    /// the openEHR base path only what ITS-REST 1.1.0 defines (the generated
    /// `ROUTES` tables, from the vendored OpenAPI documents), under `/fhir/r4`
    /// only FHIR resources, and every other route under the extensions root.
    /// A route mounted in the wrong namespace fails here.
    #[test]
    fn each_namespace_holds_only_its_own_routes() {
        let namespaces = Namespaces::of(&ferroehr::config::rest::RestConfig::default());
        let its: std::collections::BTreeSet<(String, String)> = [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
        ]
        .into_iter()
        .flatten()
        .map(|(method, path, _op)| ((*method).to_owned(), normalize_path(path)))
        .chain(
            EMPTY_PATTERN_ROUTES
                .iter()
                .map(|(method, path)| ((*method).to_owned(), (*path).to_owned())),
        )
        .collect();
        let doc = api_doc(&namespaces);
        let mut misplaced = Vec::new();
        for (path, item) in &doc.paths.paths {
            let methods = [
                ("GET", item.get.is_some()),
                ("PUT", item.put.is_some()),
                ("POST", item.post.is_some()),
                ("DELETE", item.delete.is_some()),
                ("PATCH", item.patch.is_some()),
            ];
            if let Some(relative) = path
                .strip_prefix(&namespaces.openehr)
                .filter(|rest| rest.starts_with('/'))
            {
                for (method, _) in methods.iter().filter(|(_, served)| *served) {
                    if !its.contains(&((*method).to_owned(), relative.to_owned())) {
                        misplaced.push(format!(
                            "{method} {path}: not ITS-REST, under the openEHR base"
                        ));
                    }
                }
            } else if let Some(relative) = path.strip_prefix(&namespaces.root).and_then(|rest| {
                rest.strip_prefix(FHIR_PREFIX)
                    .filter(|tail| tail.is_empty() || tail.starts_with('/'))
            }) {
                let resource = relative
                    .trim_start_matches('/')
                    .split('/')
                    .next()
                    .unwrap_or("");
                if !FHIR_RESOURCES.contains(&resource) {
                    misplaced.push(format!("{path}: not a FHIR resource, under {FHIR_PREFIX}"));
                }
            } else if !path
                .strip_prefix(&namespaces.extensions)
                .is_some_and(|rest| rest.starts_with('/'))
            {
                misplaced.push(format!("{path}: outside the three namespaces"));
            }
        }
        assert!(
            misplaced.is_empty(),
            "routes in the wrong namespace: {misplaced:#?}"
        );

        // The detector itself: an extension route under the openEHR base, and a
        // non-FHIR route under the FHIR root, are what the sweep reports.
        assert!(!its.contains(&("GET".to_owned(), "/admin/config".to_owned())));
        assert!(!FHIR_RESOURCES.contains(&"terminology"));
        assert!(
            doc.paths
                .paths
                .contains_key("/ferroehr/rest/fhir/r4/AuditEvent")
                || !cfg!(feature = "fhir"),
            "the ITI-81 retrieval is served at the FHIR root"
        );
        assert!(
            doc.paths
                .paths
                .contains_key("/ferroehr/rest/extensions/v1/terminology"),
            "terminology is an extension"
        );
    }
}
