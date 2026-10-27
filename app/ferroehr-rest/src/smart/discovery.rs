// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART service-discovery document
//! (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master04-service_discovery.adoc`).
//!
//! Serves `GET /.well-known/smart-configuration` relative to the SMART platform
//! base URL (master04 §Service Discovery ¶3-4). The `ferrohealth-smart` crate
//! builds the document from the first `[auth]` issuer's metadata (SMART App
//! Launch 2.2.0, Conformance, §Metadata); FerroEHR adds the `services` map with
//! the required `org.openehr.rest`, the openEHR capabilities it enforces, the
//! permission capabilities its route table enforces, and the scopes it honours.
//! The CDR implements none of the OAuth 2.0 endpoints the document names; they
//! are the authorization server's (master02 §Glossary).
//!
//! The response is `application/json` (master04 §Service Discovery) and is
//! served pre-auth, on the same seam as the status router.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::Router;
use axum::extract::Extension;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use ferrohealth_smart::resource::configuration::{
    OPENEHR_EPISODE_CAPABILITY, OPENEHR_REST_SERVICE, PermissionCapability, SmartOptions,
};
use http::header;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppConfig;
use crate::extensions::access::authn::Authenticator;
use crate::state::AppState;

use ferroehr::config::smart::SmartConfig;

/// The capability of the EHR launch context the `ehrId` claim conveys
/// (master04 §Capabilities), which the route table binds `patient/` grants to.
const CONTEXT_OPENEHR_EHR: &str = "context-openehr-ehr";

/// The capability of the openEHR resource scopes (master04 §Capabilities),
/// which every route enforces.
const OPENEHR_PERMISSION_V1: &str = "openehr-permission-v1";

/// The capability of the base64 JSON launch parameter (master09 §Launch
/// Parameter as a Token).
const LAUNCH_BASE64_JSON: &str = "launch-base64-json";

/// What FerroEHR adds to the document the crate builds from the issuer's
/// metadata.
///
/// `org.openehr.rest` is the one service (master04 §Services: "At a minimum,
/// the `services` section must include the openEHR REST API"), its `baseUrl`
/// absolute; `org.fhir.rest` is never advertised, since the one FHIR route this
/// server serves is the ITI-81 audit retrieval, not a FHIR API for clinical
/// data. The capabilities are those the server enforces: the openEHR resource
/// scopes and the `ehrId` launch context on every route, `permission-user`
/// because a `user/` grant counts, and `permission-patient` only when an issuer
/// is bound so a `patient/` grant counts (SMART App Launch 2.2.0, Conformance,
/// §Capabilities: a capability is one "that the server supports"). The
/// experimental pair rides its flags, and `[auth] capabilities` are added.
#[must_use]
pub fn options(cfg: &AppConfig) -> SmartOptions {
    let smart = &cfg.smart;
    let mut capabilities = vec![
        CONTEXT_OPENEHR_EHR.to_owned(),
        OPENEHR_PERMISSION_V1.to_owned(),
    ];
    if smart.episode.enabled {
        capabilities.push(OPENEHR_EPISODE_CAPABILITY.to_owned());
    }
    if smart.launch_base64_json {
        capabilities.push(LAUNCH_BASE64_JSON.to_owned());
    }
    for added in &cfg.auth.capabilities {
        if !capabilities.contains(added) {
            capabilities.push(added.clone());
        }
    }
    let mut permissions = BTreeSet::from([PermissionCapability::User]);
    if !smart.patient_issuers.is_empty() {
        permissions.insert(PermissionCapability::Patient);
    }
    let mut services = BTreeMap::new();
    services.insert(
        OPENEHR_REST_SERVICE.to_owned(),
        serde_json::json!({
            "baseUrl": absolute_base(smart, &cfg.rest.base_path),
            "description": "The openEHR REST API baseUrl",
        }),
    );
    SmartOptions {
        scopes_supported: scopes_supported(cfg),
        capabilities,
        permissions,
        services,
        ..SmartOptions::default()
    }
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

/// The advertised `scopes_supported`: `[auth] scopes_supported` when set, else
/// the scopes the route table honours (master08 grammar plus the launch and
/// identity scopes). "The server SHALL support all scopes listed here" (SMART
/// App Launch 2.2.0, Conformance, §Metadata), so a `patient/` scope is listed
/// only when an issuer is bound for it.
fn scopes_supported(cfg: &AppConfig) -> Vec<String> {
    if !cfg.auth.scopes_supported.is_empty() {
        return cfg.auth.scopes_supported.clone();
    }
    let mut scopes: Vec<String> = [
        "openid",
        "profile",
        "launch",
        "user/composition-*.cruds",
        "user/template-*.cruds",
        "user/aql-*.cruds",
        "system/composition-*.cruds",
        "system/template-*.cruds",
        "system/aql-*.cruds",
    ]
    .iter()
    .map(|scope| (*scope).to_owned())
    .collect();
    if !cfg.smart.patient_issuers.is_empty() {
        scopes.extend(
            [
                "launch/patient",
                "patient/composition-*.cruds",
                "patient/aql-*.s",
            ]
            .iter()
            .map(|scope| (*scope).to_owned()),
        );
    }
    if cfg.smart.episode.enabled {
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

/// Whether the document is served: `smart.enabled` with authentication on and
/// an `[auth]` issuer whose metadata the server reads at boot.
fn served(cfg: &AppConfig) -> bool {
    cfg.smart.enabled && cfg.auth.enabled && ferroehr::config::auth::has_bearer(&cfg.auth)
}

/// The document, serialized once at assembly.
#[derive(Debug, Clone)]
struct Document(Arc<Bytes>);

/// Build the pre-auth discovery router.
///
/// Returns an empty router when `smart.enabled` is off or no issuer's metadata
/// was read, so a merge is a no-op and the path is absent (a `404`) — the
/// extension-group `404`-when-off convention. The document is a pure function
/// of the issuer's metadata read at boot and of static configuration, so it is
/// built once here and served as ready [`Bytes`].
///
/// Mount it in `crate::router::router` beside `overview::status::router`,
/// outside the auth layer (the document is unauthenticated, master04).
pub fn router(cfg: &AppConfig, rest_root: &str, authenticator: &Authenticator) -> Router<AppState> {
    if !served(cfg) {
        return Router::new();
    }
    let Some(document) = authenticator.smart_configuration(&options(cfg)) else {
        return Router::new();
    };
    let body = Bytes::from(document.to_string());
    Router::new()
        .route(
            &discovery_path(&cfg.smart, rest_root),
            get(smart_configuration),
        )
        .layer(Extension(Document(Arc::new(body))))
}

/// The SMART App Launch service-discovery document — our SMART platform
/// surface, DEVELOPMENT status (`GET
/// /ferroehr/rest/.well-known/smart-configuration`; the openEHR SMART sub-spec
/// `docs/specs/openehr/ITS-REST/docs/smart_app_launch/master04-service_discovery.adoc`
/// is `:spec_status: DEVELOPMENT`).
///
/// Served pre-auth (public): the application fetches this document from the
/// launch `iss` before any OAuth exchange (master04 §Service Discovery).
/// Config-gated: when `smart.enabled` is off the [`router`] is empty and the
/// path is absent (a router `404`). A configured `smart.platform_base_url`
/// moves the served path; [`openapi`] re-homes this declaration to match.
#[utoipa::path(
    get, path = "/ferroehr/rest/.well-known/smart-configuration", tag = "smart",
    responses(
        (status = 200,
         description = "The SMART configuration document, always \
                        `application/json` (master04 §Service Discovery: \
                        'Responses to `/.well-known/smart-configuration` \
                        endpoint must be served with the `application/json` \
                        MIME type'). The authorization-server members come from \
                        the first `[auth]` issuer's metadata (SMART App Launch \
                        2.2.0, Conformance, §Metadata): `grant_types_supported` \
                        names only `authorization_code` and `client_credentials`, \
                        and `code_challenge_methods_supported` always carries \
                        `S256` and never `plain`. The `services` map carries \
                        `org.openehr.rest` with an absolute `baseUrl` built from \
                        `smart.public_base_url`. `capabilities` advertises what \
                        this server enforces: `context-openehr-ehr` and \
                        `openehr-permission-v1` always, `permission-user`, \
                        `permission-patient` only with `smart.patient_issuers`, \
                        the experimental pair only when their flags are on, plus \
                        `[auth] capabilities`. This route exists only when \
                        `smart.enabled` is on (otherwise absent => `404`).",
         body = Object,
         content_type = "application/json",
         example = json!({
             "issuer": "https://as.example",
             "jwks_uri": "https://as.example/jwks",
             "authorization_endpoint": "https://as.example/authorize",
             "token_endpoint": "https://as.example/token",
             "token_endpoint_auth_methods_supported": ["client_secret_basic", "private_key_jwt"],
             "grant_types_supported": ["authorization_code", "client_credentials"],
             "scopes_supported": [
                 "openid", "launch", "user/composition-*.cruds", "user/aql-*.cruds"
             ],
             "services": {
                 "org.openehr.rest": {
                     "baseUrl": "https://cdr.example.com/ferroehr/rest/openehr/v1",
                     "description": "The openEHR REST API baseUrl"
                 }
             },
             "response_types_supported": ["code"],
             "code_challenge_methods_supported": ["S256"],
             "capabilities": [
                 "permission-user", "client-confidential-symmetric",
                 "client-confidential-asymmetric", "launch-standalone",
                 "sso-openid-connect", "context-openehr-ehr", "openehr-permission-v1"
             ]
         }))
    )
)]
async fn smart_configuration(Extension(document): Extension<Document>) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        Bytes::clone(&document.0),
    )
        .into_response()
}

/// The SMART discovery document's `OpenAPI`, its path derived from the SAME
/// [`discovery_path`] the live mount uses (a configured `platform_base_url`
/// moves the served path, and the published document must follow — the
/// `#[utoipa::path]` literal is only the default-root spelling). Config-gated
/// on `smart.enabled`. Served pre-auth. Spec: ITS-REST
/// `smart_app_launch/master04`.
pub(crate) fn openapi(cfg: &AppConfig, rest_root: &str) -> utoipa::openapi::OpenApi {
    // Not served → declare nothing, mirroring [`router`], which returns an
    // empty router in the same cases: the document is off, or no issuer's
    // metadata is read because authentication is off or names no issuer.
    if !served(cfg) {
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

    fn enabled_cfg() -> AppConfig {
        AppConfig {
            smart: SmartConfig {
                enabled: true,
                public_base_url: Some("https://cdr.example.com".to_owned()),
                ..SmartConfig::default()
            },
            ..AppConfig::default()
        }
    }

    #[test]
    fn options_carry_the_openehr_service_and_the_enforced_capabilities() {
        let options = options(&enabled_cfg());
        // master04 §Services: org.openehr.rest is required, its baseUrl ABSOLUTE.
        assert_eq!(
            options.services[OPENEHR_REST_SERVICE]["baseUrl"],
            "https://cdr.example.com/ferroehr/rest/openehr/v1"
        );
        // No FHIR API for clinical data is served, so org.fhir.rest is never advertised.
        assert!(!options.services.contains_key("org.fhir.rest"));
        assert!(
            options
                .capabilities
                .contains(&CONTEXT_OPENEHR_EHR.to_owned())
        );
        assert!(
            options
                .capabilities
                .contains(&OPENEHR_PERMISSION_V1.to_owned()),
            "every route enforces the openEHR resource scopes"
        );
        // experimental caps are off by default.
        assert!(
            !options
                .capabilities
                .contains(&OPENEHR_EPISODE_CAPABILITY.to_owned())
        );
        assert!(
            !options
                .capabilities
                .contains(&LAUNCH_BASE64_JSON.to_owned())
        );
        // Only the permission capabilities the route table enforces.
        assert_eq!(
            options.permissions,
            BTreeSet::from([PermissionCapability::User])
        );
        assert_ne!(options.scopes_supported, Vec::<String>::new());
        assert!(
            !options
                .scopes_supported
                .iter()
                .any(|scope| scope.starts_with("patient/")),
            "no issuer is bound, so no patient/ scope is honoured"
        );
    }

    #[test]
    fn a_bound_issuer_declares_the_patient_capability_and_scopes() {
        let mut cfg = enabled_cfg();
        cfg.smart.patient_issuers = vec!["https://as.example".to_owned()];
        let options = options(&cfg);
        assert!(options.permissions.contains(&PermissionCapability::Patient));
        assert!(
            options
                .scopes_supported
                .contains(&"patient/composition-*.cruds".to_owned())
        );
    }

    #[test]
    fn experimental_capabilities_gated_on_flags() {
        let mut cfg = enabled_cfg();
        cfg.smart.episode.enabled = true;
        cfg.smart.launch_base64_json = true;
        let options = options(&cfg);
        assert!(
            options
                .capabilities
                .contains(&OPENEHR_EPISODE_CAPABILITY.to_owned())
        );
        assert!(
            options
                .capabilities
                .contains(&LAUNCH_BASE64_JSON.to_owned())
        );
        assert!(
            options
                .scopes_supported
                .contains(&"launch/episode".to_owned())
        );
    }

    #[test]
    fn operator_scopes_are_used_verbatim() {
        let mut cfg = enabled_cfg();
        cfg.auth.scopes_supported = vec!["openid".to_owned(), "user/aql-*.s".to_owned()];
        assert_eq!(
            options(&cfg).scopes_supported,
            vec!["openid", "user/aql-*.s"]
        );
    }

    #[test]
    fn operator_capabilities_append_deduplicated() {
        let mut cfg = enabled_cfg();
        cfg.auth.capabilities = vec![
            "launch-ehr".to_owned(),
            CONTEXT_OPENEHR_EHR.to_owned(), // duplicate of a derived one
        ];
        let options = options(&cfg);
        assert!(options.capabilities.contains(&"launch-ehr".to_owned()));
        assert_eq!(
            options
                .capabilities
                .iter()
                .filter(|c| *c == CONTEXT_OPENEHR_EHR)
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
}
