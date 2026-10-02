// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `OPTIONS /` — **System Options and Conformance** (ITS-REST System API, the
//! single operation `operationId: options`).
//!
//! Oracle: the ITS-REST **docs text** —
//! `docs/specs/openehr/ITS-REST/specifications/docs/system/Description.md`
//! (the STABLE System API), the operation source
//! `specifications/operations/options.yaml` ("Services SHOULD respond to
//! this method with the appropriate HTTP codes, headers and potentially with
//! a payload revealing more details about themselves … exposing service
//! capabilities for a conformance manifest"), its `200_options` response
//! (`Allow` + `Content-Type` headers, the `Options` payload), and the
//! overview `Requests_and_responses.md` §HTTP Methods (`OPTIONS`: "Describe
//! the communication options for the target resource"). The vendored OAS
//! bundle is `emit-rest` codegen-input provenance only, never a behavioural
//! oracle.
//!
//! The response body is the GENERATED contract DTO
//! (`openehr_its::rest::generated::system::Options`, #1822) — no hand wire
//! shape survives; `OptionsDoc` below is the utoipa DOCUMENTATION
//! projection only (never serialized on the wire), pinned field-lockstep to
//! the generated DTO by `options_doc_matches_the_generated_dto`. The handler
//! stays hand-written like every group's (the generated traits are the
//! route-table/parity oracle, not the dispatch mechanism).
//!
//! This module owns the manifest's *shape and content*; the wiring layer
//! (`crate::router::router`) constructs the [`SystemManifest`] from config plus the
//! live mounted-group set and mounts [`route`] at the API base-path root.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use std::sync::Arc;

use axum::response::Response;
use axum::routing::{MethodRouter, options};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde::Serialize;

use crate::overview::negotiate;

/// The HTTP methods this API surface supports — the `Allow` header the
/// `200_options` response carries (`specifications/responses/200_options.yaml`
/// via `headers/Allow.yaml`: "The `Allow` header lists the set of methods
/// supported", example `GET, POST, PUT, DELETE, OPTIONS`).
const ALLOW_METHODS: &str = "GET, POST, PUT, DELETE, OPTIONS";

/// The API groups the ITS-REST resource specifications define, and that the
/// `Options.endpoints` example enumerates
/// (`specifications/schemas/others/Options.yaml`).
///
/// This is the spec-defined **default** only. The manifest requires the *live* list —
/// exactly the groups the router mounts — so [`crate::router::router`] passes its
/// actual mounted-group set to [`SystemManifest::new`] rather than relying on
/// this constant.
pub const SPEC_ENDPOINTS: &[&str] = &["/ehr", "/demographic", "/definition", "/query", "/admin"];

/// Identity + conformance fields of the System-Options manifest, sourced from
/// configuration so the public identity and the advertised conformance
/// profile are not string literals baked into the handler.
///
/// The defaults are the single shared provenance source
/// ([`ferroehr::telemetry::provenance`]): `restapi_specs_version` quotes the
/// tested-contract identity [`ferroehr::telemetry::provenance::ITS_REST`] and `conformance_profile`
/// quotes [`ferroehr::telemetry::provenance::CONFORMANCE_PROFILE`] — the last machine-computed ECC
/// verdict, updated at each conformance re-baseline
/// (`docs/conformance/ferroehr/CONFORMANCE_REPORT.md` §"Profile verdict"). The manifest
/// MUST NOT out-claim that verdict. [`crate::config::server::ServerConfig`] carries a
/// `SystemOptionsConfig` as its `identity` field (the `[server.identity]`
/// section of the one config tree), so an operator MAY override any identity
/// field while the defaults stay measured.
use ferroehr::config::server::SystemOptionsConfig;

/// The System Options and Conformance manifest — the `OPTIONS /` response
/// body (ITS-REST System API `Options` schema,
/// `specifications/schemas/others/Options.yaml`): the service's identity and
/// the conformance profile it claims, "exposing service capabilities for a
/// conformance manifest" (`operations/options.yaml`).
///
/// The utoipa DOCUMENTATION projection of the generated wire DTO
/// (`openehr_its::rest::generated::system::Options`) — the served OpenAPI
/// must be our own generated document (owner hard rule), and the contract
/// DTO carries no utoipa derive. NEVER serialized on the wire; the lockstep
/// test pins the field sets identical so this cannot drift from the carrier.
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    as = Options,
    title = "Options",
    example = json!({
        "solution": "FerroEHR",
        "solution_version": "3.11.0",
        "vendor": "FerroEHR project",
        "restapi_specs_version": "1.1.0",
        "conformance_profile": "STANDARD",
        "endpoints": ["/ehr", "/demographic", "/definition", "/query"]
    })
)]
struct OptionsDoc<'a> {
    /// The product implementing the API.
    solution: &'a str,
    /// The product's own version.
    solution_version: &'a str,
    /// The organisation shipping the product.
    vendor: &'a str,
    /// The released ITS-REST contract version this server implements.
    restapi_specs_version: &'a str,
    /// The claimed CNF conformance profile — never out-claiming the last
    /// machine-computed verdict.
    conformance_profile: &'a str,
    /// The API groups this server actually mounts (the live set, not a
    /// static list).
    endpoints: &'a [String],
}

/// The immutable System-Options manifest, built once at wiring time from
/// [`SystemOptionsConfig`] plus the live mounted-group list, then shared
/// (behind an [`Arc`]) by the `OPTIONS /` handler.
#[derive(Debug, Clone)]
pub struct SystemManifest {
    config: SystemOptionsConfig,
    endpoints: Vec<String>,
}

impl SystemManifest {
    /// Build a manifest from its identity/conformance config and the endpoint
    /// list the server actually serves.
    ///
    /// `endpoints` is the *live* mounted-group set — the wiring layer
    /// passes what the router mounts (see [`route`]), so the manifest never
    /// advertises less (or more) than the server serves. Duplicates are
    /// removed and order preserved; use [`SPEC_ENDPOINTS`] as the spec-defined
    /// default when a live list is not available.
    pub fn new(config: SystemOptionsConfig, endpoints: impl IntoIterator<Item = String>) -> Self {
        let mut seen = Vec::new();
        for ep in endpoints {
            if !seen.contains(&ep) {
                seen.push(ep);
            }
        }
        Self {
            config,
            endpoints: seen,
        }
    }

    /// Returns the manifest as the generated contract `Options` DTO, the wire
    /// carrier; every field is populated, so its skip-if-none serialization
    /// emits them all.
    fn body(&self) -> openehr_its::rest::generated::system::Options {
        openehr_its::rest::generated::system::Options {
            solution: Some(self.config.solution.clone()),
            solution_version: Some(self.config.solution_version.clone()),
            vendor: Some(self.config.vendor.clone()),
            restapi_specs_version: Some(self.config.restapi_specs_version.clone()),
            conformance_profile: Some(self.config.conformance_profile.clone()),
            endpoints: Some(self.endpoints.clone()),
            additional_properties: std::collections::BTreeMap::new(),
        }
    }

    /// Render the `OPTIONS /` response: a `200 OK` with the `Allow` header and
    /// the `Options` body, honouring `Accept`.
    ///
    /// Content negotiation goes through `crate::overview::negotiate::respond`:
    /// JSON for `application/json`/`*/*`/absent `Accept` (the OAS constrains
    /// `Accept` to `application/json`, `system-codegen.openapi.yaml`
    /// `Accept_JSON` lines 71-78), and `406 Not Acceptable` for an
    /// exclusively-XML `Accept` — the `Options` manifest is not a spec-typed RM
    /// object and has no canonical-XML shape, so XML is genuinely not offered.
    /// The `Allow` header is attached only to the successful (`200`) response.
    pub fn respond(&self, headers: &HeaderMap) -> Response {
        let mut resp = negotiate::respond(headers, StatusCode::OK, &self.body());
        if resp.status() == StatusCode::OK {
            resp.headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static(ALLOW_METHODS));
        }
        resp
    }
}

/// Build the `OPTIONS` [`MethodRouter`] serving the System-Options manifest.
///
/// The handler is stateless (the manifest is captured), so the returned
/// `MethodRouter<S>` composes with a router of any state type. It is generic
/// so the wiring layer can mount it on the `AppState`-typed application router.
///
/// [`crate::router::router`] wires this:
///   1. it builds a [`SystemManifest`] from the server config
///      ([`SystemOptionsConfig`]) and the **live** mounted-group list — the
///      groups `crate::api::api_router` actually merges (`/ehr`,
///      `/demographic`, `/definition`, `/query`, and `/admin` when its group
///      is enabled), not [`SPEC_ENDPOINTS`] hardcoded;
///   2. it mounts this handler at the **API base-path root** (`cfg.base_path`,
///      e.g. `OPTIONS /ferroehr/rest/openehr/v1`) — the ONE location the
///      System API defines (`system.openapi.yaml` `servers` `{baseUrl}/v1`,
///      path `/`);
///   3. the mount sits **above** the `CorsLayer` (that layer treats every
///      `OPTIONS` as a CORS preflight and short-circuits it), which is why the
///      handler is added after the middleware stack in `crate::router::router`.
pub fn route<S>(manifest: Arc<SystemManifest>) -> MethodRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    options(move |headers: HeaderMap| {
        let manifest = Arc::clone(&manifest);
        async move { manifest.respond(&headers) }
    })
}

/// The documented twin of the live closure route (the same pattern as the
/// SMART discovery document): the closure keeps its special mounting (the
/// API base-path root, above the CORS layer), and this handler exists so the
/// operation appears — fully described — in the served `OpenAPI`. The path is
/// the DEFAULT base path; a redeployed `base_path` moves the live route with
/// it.
#[utoipa::path(
    options,
    path = "/ferroehr/rest/openehr/v1",
    tag = "system",
    operation_id = "options",
    summary = "Options and Conformance",
    description = "Describes the communication options and capabilities of \
                   this openEHR service as a conformance manifest: the \
                   product identity, the implemented ITS-REST contract \
                   version, the claimed CNF conformance profile, and the API \
                   groups this deployment actually mounts. ITS-REST System \
                   API (STABLE), operation `options`.",
    params(
        ("Accept" = Option<String>, Header,
            description = "The manifest is served as `application/json` \
                           (`*/*` and an absent header negotiate the same); \
                           the manifest is not an RM type and has no \
                           canonical-XML shape, so an exclusively-XML Accept \
                           is refused.",
            example = "application/json"),
    ),
    responses(
        (status = 200, description = "The Options and Conformance manifest.",
            body = OptionsDoc<'_>, content_type = "application/json",
            headers(
                ("Allow" = String,
                    description = "The set of HTTP methods this API surface \
                                   supports."),
                ("Content-Type" = String,
                    description = "`application/json` — the manifest's only \
                                   representation."),
            ),
            example = json!({
                "solution": "FerroEHR",
                "solution_version": "3.11.0",
                "vendor": "FerroEHR project",
                "restapi_specs_version": "1.1.0",
                "conformance_profile": "STANDARD",
                "endpoints": ["/ehr", "/demographic", "/definition", "/query"]
            })),
        (status = 406, description = "No acceptable representation: the \
                                      manifest has no canonical-XML shape, \
                                      so an exclusively-XML `Accept` cannot \
                                      be satisfied."),
    ),
    security(())
)]
#[expect(
    dead_code,
    reason = "the documentation twin of a live route: the served route is the \
              closure above, so only the `#[utoipa::path]` attribute on this \
              stub is consumed"
)]
fn options_documented() {}

/// The System API's `OpenAPI` fragment, merged into the composed served
/// document by `crate::extensions::openapi` — the live route is a closure
/// mounted outside `OpenApiRouter`, so the twin above carries the
/// documentation.
///
/// The documented path is derived from the SAME `base_path` the live mount uses
/// (`crate::router::router` mounts the manifest at the API base-path root), so a
/// redeployed base path moves the served declaration with the route; the
/// `#[utoipa::path]` literal is only the default spelling.
pub(crate) fn openapi(base_path: &str) -> utoipa::openapi::OpenApi {
    use utoipa::OpenApi;
    #[derive(OpenApi)]
    #[openapi(paths(options_documented), components(schemas(OptionsDoc<'_>)))]
    struct SystemApiDoc;
    let mut doc = SystemApiDoc::openapi();
    crate::extensions::openapi::rehome_path(&mut doc, "/ferroehr/rest/openehr/v1", base_path);
    doc
}

#[cfg(test)]
mod tests {

    /// The doc-only [`OptionsDoc`] stays field-lockstep with the GENERATED
    /// wire carrier (#1822): serializing a fully-populated sample of each
    /// yields the same key set, so the served OpenAPI cannot drift from the
    /// contract DTO it documents.
    #[test]
    fn options_doc_matches_the_generated_dto() {
        let endpoints = vec!["/ehr".to_owned()];
        let doc = serde_json::to_value(OptionsDoc {
            solution: "s",
            solution_version: "v",
            vendor: "o",
            restapi_specs_version: "r",
            conformance_profile: "p",
            endpoints: &endpoints,
        })
        .expect("doc projection serializes");
        let wire = serde_json::to_value(openehr_its::rest::generated::system::Options {
            solution: Some("s".to_owned()),
            solution_version: Some("v".to_owned()),
            vendor: Some("o".to_owned()),
            restapi_specs_version: Some("r".to_owned()),
            conformance_profile: Some("p".to_owned()),
            endpoints: Some(endpoints.clone()),
            additional_properties: std::collections::BTreeMap::new(),
        })
        .expect("wire carrier serializes");
        assert_eq!(
            doc, wire,
            "the OpenAPI projection drifted from the generated carrier"
        );
    }
    use super::*;
    use http_body_util::BodyExt;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    fn manifest() -> SystemManifest {
        SystemManifest::new(
            SystemOptionsConfig::default(),
            SPEC_ENDPOINTS.iter().map(|s| (*s).to_owned()),
        )
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        serde_json::from_slice(&bytes).expect("json body")
    }

    #[tokio::test]
    async fn options_returns_200_with_allow_header_and_all_six_fields() {
        let resp = manifest().respond(&HeaderMap::new());
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::ALLOW).unwrap(),
            ALLOW_METHODS,
            "Allow header enumerates supported methods"
        );
        assert_eq!(
            resp.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json"),
        );

        let v = body_json(resp).await;
        // All six OAS `Options` properties are present and correctly typed.
        assert!(v["solution"].is_string());
        assert!(v["solution_version"].is_string());
        assert!(v["vendor"].is_string());
        // The released ITS-REST contract version (shared provenance, matching
        // management `/info` + the ECC report); the profile is the
        // machine-computed verdict.
        assert_eq!(
            v["restapi_specs_version"],
            ferroehr::telemetry::provenance::ITS_REST
        );
        assert_eq!(
            v["conformance_profile"],
            ferroehr::telemetry::provenance::CONFORMANCE_PROFILE
        );
        assert!(v["endpoints"].is_array());
    }

    #[tokio::test]
    async fn solution_and_vendor_are_distinct_and_config_driven() {
        // `solution` (product) and `vendor` (organisation) differ, and
        // both come from config — not a shared placeholder.
        let cfg = SystemOptionsConfig {
            solution: "MySolution".to_owned(),
            vendor: "MyOrg".to_owned(),
            ..SystemOptionsConfig::default()
        };
        let m = SystemManifest::new(cfg, SPEC_ENDPOINTS.iter().map(|s| (*s).to_owned()));
        let v = body_json(m.respond(&HeaderMap::new())).await;
        assert_eq!(v["solution"], "MySolution");
        assert_eq!(v["vendor"], "MyOrg");
        assert_ne!(v["solution"], v["vendor"]);
    }

    #[tokio::test]
    async fn conformance_profile_comes_from_config_not_a_literal() {
        // the handler reports whatever the config carries — so it can be
        // reconciled with / capped at the runner's machine verdict.
        let cfg = SystemOptionsConfig {
            conformance_profile: "CORE".to_owned(),
            ..SystemOptionsConfig::default()
        };
        let m = SystemManifest::new(cfg, SPEC_ENDPOINTS.iter().map(|s| (*s).to_owned()));
        let v = body_json(m.respond(&HeaderMap::new())).await;
        assert_eq!(v["conformance_profile"], "CORE");
    }

    #[tokio::test]
    async fn endpoints_reflect_the_supplied_live_list() {
        // the manifest advertises exactly the groups it was built with.
        let live = ["/ehr", "/query", "/definition", "/demographic", "/admin"];
        let m = SystemManifest::new(
            SystemOptionsConfig::default(),
            live.iter().map(|s| (*s).to_owned()),
        );
        let v = body_json(m.respond(&HeaderMap::new())).await;
        let got: Vec<String> = v["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(got, live);
    }

    #[tokio::test]
    async fn duplicate_endpoints_are_deduplicated_order_preserved() {
        let m = SystemManifest::new(
            SystemOptionsConfig::default(),
            ["/ehr", "/query", "/ehr", "/admin"]
                .iter()
                .map(|s| (*s).to_owned()),
        );
        let v = body_json(m.respond(&HeaderMap::new())).await;
        let got: Vec<String> = v["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(got, ["/ehr", "/query", "/admin"]);
    }

    #[tokio::test]
    async fn json_accept_is_honoured() {
        let resp = manifest().respond(&headers(&[("accept", "application/json")]));
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn xml_only_accept_is_not_acceptable() {
        // `Options` has no canonical-XML shape (it is not a spec-typed RM
        // object); an exclusively-XML `Accept` negotiates to 406.
        let resp = manifest().respond(&headers(&[("accept", "application/xml")]));
        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
        // The failed negotiation carries no `Allow` header.
        assert!(resp.headers().get(header::ALLOW).is_none());
    }
}
