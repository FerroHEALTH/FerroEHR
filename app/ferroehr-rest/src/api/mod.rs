// SPDX-FileCopyrightText: Vernum Projecten B.V.
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
//! exact `*Params` from the raw request parts, calls the SM native-API method on
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
//! - [`message`] — the own-design MESSAGE group (`I_EHR_EXTRACT_SERVICE` +
//!   `I_TDD_SERVICE`); the release publishes no message API at all, so the whole
//!   group is an extension under its own `/message/` resource root.
//! - [`system`] — the System API manifest ([`system::options::SystemManifest`],
//!   `ferroehr::config::server::SystemOptionsConfig`), assembled and mounted by [`crate::router::router`].
//!
//! `item_tags` is not a group: it is the one `ITEM_TAG` write-wrapper
//! implementation (`item_tags::pending` / `item_tags::persist` /
//! `item_tags::echo` / `item_tags::write_body`) the EHR and demographic
//! dispatchers share, so the two cannot drift.
//!
//! Every group — the standardised ITS-REST groups above and the own-design
//! extension surfaces ([`crate::extensions`]: terminology, event subscription,
//! FHIR connector) — is built as a native `utoipa-axum`
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
use utoipa_axum::router::OpenApiRouter;

use crate::extensions::access::{ehr_access, pep};
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
    /// extension route the tables do not name (the PARTY_RELATIONSHIP and admin
    /// extensions) reuses a generated struct, so its match is built from that
    /// struct's declared path parameters (`params`) and the request's captures,
    /// which is all `from_request` reads from it.
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

/// Build the API router covering every ITS-REST + extension route
/// (group-relative paths; nested under the configured base path by
/// [`crate::router::router`]). State is applied by the caller. This is the `Router` half
/// of [`api_openapi_router`]; the `OpenApi` half feeds the served document
/// ([`api_doc`]), so route and documentation are single-sourced.
pub(crate) fn api_router() -> Router<AppState> {
    api_openapi_router().into()
}

/// Every API group as one native `utoipa-axum` router (group-relative paths), so
/// each operation's route and its `OpenAPI` path metadata come from ONE
/// `#[utoipa::path]` handler — no vendored OAS served, no table-driven synthesis.
/// The standardised ITS-REST groups (EHR / DEMOGRAPHIC / DEFINITION / QUERY /
/// ADMIN) and the own-design extensions (terminology, event-subscription,
/// FHIR connector, `PARTY_RELATIONSHIP`) are composed uniformly; every
/// handler forwards to its group dispatcher through [`guarded_dispatch`], so the
/// auth / ATNA-audit / ABAC stack is applied identically across the whole surface
/// (the operation contract each dispatcher implements is generated by `emit-rest`
/// from the vendored OAS — the codegen input, never served).
///
/// The extension groups are always mounted — their config gate is enforced inside
/// the handler (a disabled group answers `404`). No openEHR spec governs an OAS
/// layout or the extension surfaces — our own design.
pub(crate) fn api_openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(ehr::openapi_routes::routes())
        .merge(demographic::openapi_routes::routes())
        .merge(demographic::relationship::relationship_routes())
        .merge(definition::openapi_routes::routes())
        .merge(definition::archetype::archetype_routes())
        .merge(query::openapi_routes::routes())
        .merge(admin::openapi_routes::routes())
        .merge(admin::archive::archive_routes())
        .merge(admin::report::report_routes())
        .merge(admin::dump_load::dump_load_routes())
        .merge(admin::integrity::integrity_routes())
        .merge(admin::marks::marks_routes())
        .merge(message::extract::extract_routes())
        .merge(message::tdd::tdd_routes())
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
        .merge({
            #[cfg(feature = "fhir")]
            {
                fhir::routes()
            }
            #[cfg(not(feature = "fhir"))]
            {
                OpenApiRouter::new()
            }
        })
}

/// The full API surface's `OpenAPI` document, paths nested under `base_path` so
/// they read as full server paths (e.g. `/ferroehr/rest/openehr/v1/ehr`). Built
/// from the same [`api_openapi_router`] composition that mounts the routes, so
/// the document cannot drift from the router.
pub(crate) fn api_doc(base_path: &str) -> utoipa::openapi::OpenApi {
    OpenApiRouter::<AppState>::new()
        .nest(base_path, api_openapi_router())
        .into_openapi()
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
    let mut resp = match ehr_access::enforce(&state, op, &parts).await {
        Ok(()) => match pep::pre_check(&state, op, &parts).await {
            Ok(()) => {
                let resp = dispatch(state.clone(), op, parts).await;
                pep::post_check(&state, op, resp).await
            }
            Err(deny) => deny,
        },
        Err(deny) => *deny,
    };
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
    /// served surface — the served paths are a superset (the extension groups
    /// add own-design routes the tables do not carry).
    ///
    /// Scope: the five API groups `emit-rest` generates. The SYSTEM group's one
    /// released operation (`OPTIONS` at the API base-path root,
    /// `crates/openehr-its/vendor/rest-oas/system-codegen.openapi.yaml`
    /// §`paths./.options`) has no generated `ROUTES` table and is mounted
    /// outside [`api_openapi_router`] (on the base path itself), so it is
    /// pinned by its own assertion below rather than by the table sweep.
    #[test]
    fn served_router_covers_every_generated_route() {
        // A concrete base path: `api_doc` nests the group router, which requires
        // a leading `/` (the served document always carries one).
        const BASE: &str = "/base";
        let doc = api_doc(BASE);
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
}
