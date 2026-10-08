// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART service-discovery document
//! (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master04-service_discovery.adoc`).
//!
//! Serves `GET /.well-known/smart-configuration` (relative to the SMART
//! *Platform* base URL, master04 §Service Discovery ¶3-4), advertising the
//! external Authorization-Server endpoints, the `services` map (with the
//! required `org.openehr.rest`), the SMART `capabilities`, and the enforced
//! scope set. The document is **assembled from [`SmartConfig`]**
//! — the CDR advertises only what it (or its configured AS) actually offers; it
//! implements none of the `OAuth2` endpoints (those are Authorization-Server
//! duties — master02 §Glossary, master06/master07).
//!
//! The response is `application/json` (master04 §Service Discovery, R-02) and is
//! served **pre-auth**, on the same seam as the status router
//! (`crate::router::router` merges `overview::status::router` outside the auth layer;
//! `crate::overview::status`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use std::collections::BTreeMap;

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use bytes::Bytes;
use http::header;
use serde::Serialize;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppConfig;
use crate::state::AppState;

use ferroehr::config::smart::SmartConfig;

/// The `/.well-known/smart-configuration` document (master04 §Authentication
/// Endpoints + §Services + §Capabilities). Unset optional endpoints are omitted.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SmartConfiguration {
    /// The token/OIDC issuer (master04 §Authentication Endpoints).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// The JWKS document URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwks_uri: Option<String>,
    /// The `OAuth2` authorization endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_endpoint: Option<String>,
    /// The `OAuth2` token endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    /// Supported token-endpoint client-authentication methods.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub token_endpoint_auth_methods_supported: Vec<String>,
    /// Supported `OAuth2` grant types (master06 §Deprecated Flows: never
    /// `implicit`/password — `SmartConfig::validate` guards this).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub grant_types_supported: Vec<String>,
    /// The dynamic-client registration endpoint (master03: out-of-band by
    /// recommendation; advertised only when the AS offers it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    /// The scopes the Platform advertises as supported.
    pub scopes_supported: Vec<String>,
    /// Supported `OAuth2` response types.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub response_types_supported: Vec<String>,
    /// The user management endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub management_endpoint: Option<String>,
    /// The token introspection endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub introspection_endpoint: Option<String>,
    /// The token revocation endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation_endpoint: Option<String>,
    /// PKCE code-challenge methods.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub code_challenge_methods_supported: Vec<String>,
    /// The advertised SMART capabilities (master04 §Capabilities).
    pub capabilities: Vec<String>,
    /// The available service interfaces (master04 §Services). `org.openehr.rest`
    /// is always present (required). `org.fhir.rest` (master04 §Services,
    /// recommended) is never advertised: the one FHIR route this server serves is
    /// the ITI-81 audit retrieval, not a FHIR API for clinical data.
    pub services: BTreeMap<String, Service>,
}

/// One `services` entry (master04 §Services).
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Service {
    /// Absolute URL to the root of the API (required).
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Service API version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Link to service documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    /// Link to the `OpenAPI` definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openapi: Option<String>,
}

/// Build the discovery document from configuration.
///
/// - `openehr_base_url` is the CDR's openEHR REST base path (the one value the
///   CDR authoritatively owns — `services.org.openehr.rest.baseUrl`, R-04).
/// - `issuer_fallback` is used for `issuer` when `endpoints.issuer` is unset
///   (the configured OIDC bearer issuer, `auth.oidc.issuer`).
///
/// Every `services.*.baseUrl` is emitted ABSOLUTE (master04 §Services:
/// "Absolute URL to the root of the API `*(required)*`"): a relative base path
/// is prefixed with the configured external origin
/// (`SmartConfig::public_base_url`, boot-required when SMART is enabled).
#[must_use]
pub fn build_document(
    cfg: &SmartConfig,
    openehr_base_url: &str,
    issuer_fallback: Option<&str>,
) -> SmartConfiguration {
    let e = &cfg.endpoints;

    let mut services = BTreeMap::new();
    // R-04: `org.openehr.rest` is required.
    services.insert(
        "org.openehr.rest".to_owned(),
        Service {
            base_url: absolute_base(cfg, openehr_base_url),
            description: Some("The openEHR REST API baseUrl".to_owned()),
            version: None,
            documentation: None,
            openapi: None,
        },
    );

    SmartConfiguration {
        issuer: e
            .issuer
            .clone()
            .or_else(|| issuer_fallback.map(str::to_owned)),
        jwks_uri: e.jwks_uri.clone(),
        authorization_endpoint: e.authorization_endpoint.clone(),
        token_endpoint: e.token_endpoint.clone(),
        token_endpoint_auth_methods_supported: e.token_endpoint_auth_methods_supported.clone(),
        grant_types_supported: e.grant_types_supported.clone(),
        registration_endpoint: e.registration_endpoint.clone(),
        scopes_supported: scopes_supported(cfg),
        response_types_supported: e.response_types_supported.clone(),
        management_endpoint: e.management_endpoint.clone(),
        introspection_endpoint: e.introspection_endpoint.clone(),
        revocation_endpoint: e.revocation_endpoint.clone(),
        code_challenge_methods_supported: e.code_challenge_methods_supported.clone(),
        capabilities: capabilities(cfg),
        services,
    }
}

/// The advertised capabilities — **only** those the CDR actually enforces
/// (master04 §Capabilities, R-05/R-07). `context-openehr-ehr` is always present
/// (the CDR binds the `ehrId` launch context); `openehr-permission-v1` is
/// advertised ONLY in fail-closed mode (`require_smart_scopes`) — the
/// capability "Indicates support for fine-grained scopes and authorization
/// scheme over openEHR resources", and advisory mode does not enforce against
/// a scope-less caller, so advertising it there would over-claim. The two
/// experimental capabilities are gated on their sub-flags, and the operator's
/// `endpoints.capabilities` (the HL7-defined base capabilities the external
/// framework owns — "In addition to those scopes defined in the original SMART
/// App Launch framework") are appended, deduplicated.
fn capabilities(cfg: &SmartConfig) -> Vec<String> {
    let mut caps = vec!["context-openehr-ehr".to_owned()];
    if cfg.require_smart_scopes {
        caps.push("openehr-permission-v1".to_owned());
    }
    if cfg.episode.enabled {
        caps.push("context-openehr-episode".to_owned());
    }
    if cfg.launch_base64_json {
        caps.push("launch-base64-json".to_owned());
    }
    for extra in &cfg.endpoints.capabilities {
        if !caps.contains(extra) {
            caps.push(extra.clone());
        }
    }
    caps
}

/// Make a base path absolute by prefixing the configured external origin
/// (master04 §Services: `baseUrl` is an "Absolute URL"). An already-absolute
/// value passes through; a missing origin (impossible on a validated enabled
/// config) leaves the path as-is rather than fabricating one.
fn absolute_base(cfg: &SmartConfig, base: &str) -> String {
    if base.starts_with("http://") || base.starts_with("https://") {
        return base.to_owned();
    }
    match cfg.public_base_url.as_deref() {
        Some(origin) => format!("{}{base}", origin.trim_end_matches('/')),
        None => base.to_owned(),
    }
}

/// The advertised `scopes_supported` — the operator override when set, else a
/// default reflecting the scopes the CDR enforces (master08 grammar + the
/// launch/identity scopes).
fn scopes_supported(cfg: &SmartConfig) -> Vec<String> {
    if !cfg.endpoints.scopes_supported.is_empty() {
        return cfg.endpoints.scopes_supported.clone();
    }
    let mut scopes = vec![
        "openid".to_owned(),
        "profile".to_owned(),
        "offline_access".to_owned(),
        "launch".to_owned(),
        "launch/patient".to_owned(),
        "patient/composition-*.cruds".to_owned(),
        "patient/aql-*.rs".to_owned(),
        "user/composition-*.cruds".to_owned(),
        "user/template-*.cruds".to_owned(),
        "user/aql-*.cruds".to_owned(),
        "system/composition-*.cruds".to_owned(),
        "system/aql-*.cruds".to_owned(),
    ];
    if cfg.episode.enabled {
        scopes.push("launch/episode".to_owned());
    }
    scopes
}

/// The `.well-known/smart-configuration` path, relative to the SMART Platform
/// base (master04 §Service Discovery ¶3-4).
///
/// `platform_base_url` overrides the default (`rest_root`, e.g.
/// `/ferroehr/rest`); a base with a path segment is honoured (`/gateway/v1` →
/// `/gateway/v1/.well-known/smart-configuration`).
#[must_use]
pub fn discovery_path(cfg: &SmartConfig, rest_root: &str) -> String {
    let base = cfg
        .platform_base_url
        .as_deref()
        .unwrap_or(rest_root)
        .trim_end_matches('/');
    format!("{base}/.well-known/smart-configuration")
}

/// Build the pre-auth discovery router.
///
/// Returns an **empty** router when SMART is disabled, so a merge is a no-op
/// and the path is absent (a `404`) — matching the extension-group
/// `404`-when-off convention (`crate::config`). When enabled, serves the
/// document (`application/json`, R-02) at [`discovery_path`].
///
/// The document is a pure function of static configuration (the openEHR base
/// URL + the OIDC issuer fallback), so it is **built once here** and served as
/// ready [`Bytes`] — never rebuilt per request.
///
/// Mount it in `crate::router::router` beside `overview::status::router` — **outside**
/// the auth layer (the document is unauthenticated, master04).
pub fn router(cfg: &AppConfig, rest_root: &str) -> Router<AppState> {
    // Mounted from `crate::router::router`: merged beside `status::router(&rest_root)`,
    // OUTSIDE the `authn::AuthLayer` (this is a pre-auth, public document,
    // master04 §Service Discovery), with `rest_root` = the `/ferroehr/rest` root.
    // Disabled → an empty router (a no-op merge), so the path is absent (`404`).
    if !cfg.smart.enabled {
        return Router::new();
    }
    let issuer = cfg.auth.oidc.as_ref().map(|o| o.issuer.as_str());
    let doc = build_document(&cfg.smart, &cfg.server.base_path, issuer);
    let body = Bytes::from(serde_json::to_vec(&doc).unwrap_or_else(|_| b"{}".to_vec()));

    let path = discovery_path(&cfg.smart, rest_root);
    Router::new().route(
        &path,
        get(move || {
            let body = body.clone();
            async move { discovery_response(body) }
        }),
    )
}

/// Serve the pre-serialized discovery document (`application/json`, R-02) as a
/// clone-free (ref-counted) [`Bytes`] body write.
fn discovery_response(body: Bytes) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// The SMART App Launch service-discovery document — **our SMART Platform
/// surface, DEVELOPMENT status** (`GET
/// /ferroehr/rest/.well-known/smart-configuration`; the openEHR SMART sub-spec
/// `docs/specs/openehr/ITS-REST/docs/smart_app_launch/master04-service_discovery.adoc`
/// is `:spec_status: DEVELOPMENT` — a release-pinned reporting qualifier, per
/// a settled adjudication).
///
/// Served **pre-auth** (public): the *Application* fetches this document from
/// the launch `iss` BEFORE any OAuth exchange (master04 §Service Discovery;
/// master07 §SMART Authorization Flow — the posture inference is
/// adjudicated, the text never states it). Config-gated: when SMART is
/// disabled the [`router`] is empty and the path is absent (a router `404`,
/// zero wire drift). A configured `smart.platform_base_url` moves the served
/// path; [`openapi`] re-homes this declaration to match. The **live** route
/// serves the document pre-serialized once at assembly ([`router`]); this body
/// runs only if the endpoint is mounted directly.
#[utoipa::path(
    get, path = "/ferroehr/rest/.well-known/smart-configuration", tag = "smart",
    responses(
        (status = 200,
         description = "The SMART configuration document, always \
                        `application/json` (master04 §Service Discovery: \
                        'Responses to `/.well-known/smart-configuration` \
                        endpoint must be served with the `application/json` \
                        MIME type'). The `services` map always carries \
                        `org.openehr.rest` ('At a minimum, the `services` \
                        section must include the openEHR REST API using the \
                        key `org.openehr.rest`') with an ABSOLUTE `baseUrl` \
                        (§Services: 'Absolute URL to the root of the API \
                        (required)', built from `smart.public_base_url`); \
                        `org.fhir.rest` is never advertised, since this \
                        server serves no FHIR API for clinical data. `capabilities` advertises only \
                        what this server enforces: `context-openehr-ehr` \
                        always, `openehr-permission-v1` only in fail-closed \
                        mode (`require_smart_scopes`), the experimental pair \
                        only when their flags are on, plus any operator \
                        `smart.endpoints.capabilities`. The \
                        Authorization-Server endpoints are the operator's, \
                        verbatim; `issuer` falls back to the OIDC bearer \
                        issuer. This route exists only when SMART is enabled \
                        (disabled => absent => `404`).",
         body = SmartConfiguration,
         content_type = "application/json",
         example = json!({
             "issuer": "https://as.example",
             "jwks_uri": "https://as.example/jwks",
             "authorization_endpoint": "https://as.example/authorize",
             "token_endpoint": "https://as.example/token",
             "grant_types_supported": ["authorization_code", "client_credentials"],
             "scopes_supported": [
                 "openid", "profile", "launch", "launch/patient",
                 "patient/composition-*.cruds", "user/aql-*.cruds"
             ],
             "response_types_supported": ["code"],
             "code_challenge_methods_supported": ["S256"],
             "capabilities": ["context-openehr-ehr", "openehr-permission-v1"],
             "services": {
                 "org.openehr.rest": {
                     "baseUrl": "https://cdr.example.com/ferroehr/rest/openehr/v1",
                     "description": "The openEHR REST API baseUrl"
                 }
             }
         }))
    )
)]
async fn smart_configuration(State(state): State<AppState>) -> Json<SmartConfiguration> {
    let cfg = state.config();
    let issuer = cfg.auth.oidc.as_ref().map(|o| o.issuer.as_str());
    Json(build_document(&cfg.smart, &cfg.server.base_path, issuer))
}

/// The SMART discovery document's `OpenAPI`, its path derived from the SAME
/// [`discovery_path`] the live mount uses (a configured `platform_base_url`
/// moves the served path, and the published document must follow — the
/// `#[utoipa::path]` literal is only the default-root spelling). Config-gated:
/// `FERROEHR_REST_SMART__ENABLED`. Served pre-auth. Spec: ITS-REST
/// `smart_app_launch/master04`.
pub(crate) fn openapi(cfg: &AppConfig, rest_root: &str) -> utoipa::openapi::OpenApi {
    // Disabled → declare nothing, mirroring [`router`], which returns an empty
    // router in the same case. The served document's whole value is that it
    // describes the LIVE router; advertising a path that answers `404` breaks
    // exactly the property this project serves its own document to guarantee.
    if !cfg.smart.enabled {
        return utoipa::openapi::OpenApiBuilder::new().build();
    }
    let mut doc = OpenApiRouter::<AppState>::new()
        .routes(routes!(smart_configuration))
        .into_openapi();
    crate::extensions::openapi::rehome_path(
        &mut doc,
        "/ferroehr/rest/.well-known/smart-configuration",
        &discovery_path(&cfg.smart, rest_root),
    );
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_cfg() -> SmartConfig {
        let mut c = SmartConfig {
            enabled: true,
            public_base_url: Some("https://cdr.example.com".to_owned()),
            ..SmartConfig::default()
        };
        c.endpoints.authorization_endpoint = Some("https://as.example/authorize".to_owned());
        c.endpoints.token_endpoint = Some("https://as.example/token".to_owned());
        c.endpoints.grant_types_supported = vec![
            "authorization_code".to_owned(),
            "client_credentials".to_owned(),
        ];
        c.endpoints.code_challenge_methods_supported = vec!["S256".to_owned()];
        c
    }

    #[test]
    fn document_has_required_keys_and_openehr_service() {
        let doc = build_document(
            &enabled_cfg(),
            "/ferroehr/rest/openehr/v1",
            Some("https://as.example"),
        );
        // R-04: org.openehr.rest is required and carries the CDR base.
        let openehr = doc
            .services
            .get("org.openehr.rest")
            .expect("openehr service");
        // master04 §Services: baseUrl is an ABSOLUTE URL.
        assert_eq!(
            openehr.base_url,
            "https://cdr.example.com/ferroehr/rest/openehr/v1"
        );
        // No FHIR API for clinical data is served, so org.fhir.rest is never advertised.
        assert!(!doc.services.contains_key("org.fhir.rest"));
        // issuer falls back to the OIDC issuer.
        assert_eq!(doc.issuer.as_deref(), Some("https://as.example"));
        // R-05 baseline capabilities.
        assert!(doc.capabilities.contains(&"context-openehr-ehr".to_owned()));
        // Advisory mode (require_smart_scopes = false) must NOT claim the
        // fine-grained enforcement capability (master04 §Capabilities).
        assert!(
            !doc.capabilities
                .contains(&"openehr-permission-v1".to_owned())
        );
        // experimental caps are off by default.
        assert!(
            !doc.capabilities
                .contains(&"context-openehr-episode".to_owned())
        );
        assert!(!doc.capabilities.contains(&"launch-base64-json".to_owned()));
        assert!(!doc.scopes_supported.is_empty());
    }

    #[test]
    fn experimental_capabilities_gated_on_flags() {
        let mut c = enabled_cfg();
        c.episode.enabled = true;
        c.launch_base64_json = true;
        let doc = build_document(&c, "/openehr", None);
        assert!(
            doc.capabilities
                .contains(&"context-openehr-episode".to_owned())
        );
        assert!(doc.capabilities.contains(&"launch-base64-json".to_owned()));
        // The episode launch scope is advertised too.
        assert!(doc.scopes_supported.contains(&"launch/episode".to_owned()));
    }

    #[test]
    fn issuer_override_wins_over_fallback() {
        let mut c = enabled_cfg();
        c.endpoints.issuer = Some("https://issuer.override".to_owned());
        let doc = build_document(&c, "/openehr", Some("https://fallback"));
        assert_eq!(doc.issuer.as_deref(), Some("https://issuer.override"));
    }

    #[test]
    fn operator_scope_override_used_verbatim() {
        let mut c = enabled_cfg();
        c.endpoints.scopes_supported = vec!["openid".to_owned(), "patient/*.rs".to_owned()];
        let doc = build_document(&c, "/openehr", None);
        assert_eq!(doc.scopes_supported, vec!["openid", "patient/*.rs"]);
    }

    #[test]
    fn permission_capability_rides_fail_closed_mode() {
        let mut c = enabled_cfg();
        c.require_smart_scopes = true;
        let doc = build_document(&c, "/openehr", None);
        assert!(
            doc.capabilities
                .contains(&"openehr-permission-v1".to_owned())
        );
    }

    #[test]
    fn operator_capabilities_append_deduplicated() {
        let mut c = enabled_cfg();
        c.endpoints.capabilities = vec![
            "launch-ehr".to_owned(),
            "sso-openid-connect".to_owned(),
            "context-openehr-ehr".to_owned(), // duplicate of a derived one
        ];
        let doc = build_document(&c, "/openehr", None);
        assert!(doc.capabilities.contains(&"launch-ehr".to_owned()));
        assert!(doc.capabilities.contains(&"sso-openid-connect".to_owned()));
        assert_eq!(
            doc.capabilities
                .iter()
                .filter(|c| *c == "context-openehr-ehr")
                .count(),
            1
        );
    }

    #[test]
    fn discovery_path_defaults_to_rest_root() {
        let c = SmartConfig::default();
        assert_eq!(
            discovery_path(&c, "/ferroehr/rest"),
            "/ferroehr/rest/.well-known/smart-configuration"
        );
    }

    #[test]
    fn discovery_path_honours_platform_base() {
        let mut c = SmartConfig {
            platform_base_url: Some("/gateway/v1".to_owned()),
            ..SmartConfig::default()
        };
        assert_eq!(
            discovery_path(&c, "/ferroehr/rest"),
            "/gateway/v1/.well-known/smart-configuration"
        );
        // Trailing slash normalised.
        c.platform_base_url = Some("/gateway/v1/".to_owned());
        assert_eq!(
            discovery_path(&c, "/ferroehr/rest"),
            "/gateway/v1/.well-known/smart-configuration"
        );
    }

    #[test]
    fn document_serialises_with_baseurl_key() {
        let doc = build_document(&enabled_cfg(), "/openehr", None);
        let json = serde_json::to_value(&doc).expect("serialise");
        // master04 uses the camelCase `baseUrl` key.
        assert_eq!(
            json["services"]["org.openehr.rest"]["baseUrl"],
            serde_json::json!("https://cdr.example.com/openehr")
        );
        // deprecated grant flows never appear (they're rejected at validate()).
        assert_eq!(
            json["grant_types_supported"],
            serde_json::json!(["authorization_code", "client_credentials"])
        );
    }
}
