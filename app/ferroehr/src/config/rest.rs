// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! REST-adapter configuration types.
//!
//! No openEHR spec governs configuration mechanics — our own design. There is no
//! loader here: the whole server configuration is one tree
//! ([`crate::config::FerroEhrConfig`]) loaded once by the binary. The listener,
//! its TLS, its limits and its drain are the family's `[server]` section
//! (`ferrohealth_server::section::ServerSection`); this module owns the
//! REST surface the listener serves:
//!
//! - [`RestConfig`] — the `[rest]` section (the openEHR API base path, the
//!   system identifier, the `OPTIONS /` System-Options identity, the Swagger
//!   UI access level, CORS and the clinical body tier).
//! - [`AdminConfig`] — the `[admin]` section.

use serde::{Deserialize, Serialize};

use crate::config::management::AccessLevel;

/// The `[rest]` section — the openEHR REST surface the listener serves.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RestConfig {
    /// The ITS-REST base path all API routes hang off.
    pub base_path: String,
    /// Who may read the Swagger UI and the `OpenAPI` documents at the REST
    /// root: `off` (not mounted), `admin_only`, `private` (any authenticated
    /// principal; the default) or `public`. The documents list the whole
    /// enabled operation surface, admin and message groups included, so
    /// `public` discloses it to anyone who can reach the port. The same
    /// vocabulary the management endpoints use.
    pub swagger_ui: AccessLevel,
    /// Permissive CORS (dev only). Production configures explicit origins.
    pub cors_permissive: bool,
    /// This CDR's own openEHR **system identifier** — the identity the
    /// deployment stamps into the data it authors
    /// (`FERROEHR__REST__SYSTEM_ID`). Three wire values carry it:
    ///
    /// - `EHR.system_id` at EHR creation — RM ehr
    ///   `master04-ehr_package.adoc` §EHR Identifier Allocation: "the
    ///   `EHR._system_id_` value should be set to the value that would normally
    ///   be used for locally created EHRs";
    /// - the `AUDIT_DETAILS.system_id` server default when the client supplies
    ///   none through `openehr-audit-details` — ITS-REST
    ///   `specifications/docs/overview/Requests_and_responses.md`
    ///   §"openehr-version and openehr-audit-details": "when `system_id` is
    ///   not provided by the client, the server MUST set it to its own
    ///   configured system identifier";
    /// - every `OBJECT_VERSION_ID.creating_system_id` a commit mints — RM
    ///   common `master06-change_control_package.adoc` §Distributed
    ///   Versioning. That value is stored per version, so changing this key
    ///   never rewrites identifiers already committed.
    ///
    /// Distinct from [`SystemOptionsConfig`] (`[rest.identity]`), which is the
    /// display identity of the `OPTIONS` System-Options manifest; `system_id`
    /// names which system authored the data.
    ///
    /// Defaults to [`crate::service::DEFAULT_SYSTEM_ID`]. No openEHR spec
    /// governs the configuration mechanism — our own design; the specs govern
    /// only that a system has such an identifier.
    pub system_id: String,
    /// The `OPTIONS /` System-Options manifest identity
    /// (`[rest.identity]`). Sourced from config so the public identity and advertised profile
    /// are not string literals in the handler; the live endpoint list is
    /// supplied separately by `ferroehr_rest::router`. This is the *display* identity
    /// of the manifest — the data-authoring identity is [`Self::system_id`].
    pub identity: SystemOptionsConfig,
    /// The largest body the ordinary clinical surface accepts, in bytes, below
    /// the listener's `server.body_limit_bytes`.
    ///
    /// The payloads differ by more than an order of magnitude: the routes that
    /// accept bulk by design (operational-template upload and EHR-Extract import)
    /// take up to `server.body_limit_bytes`, and every other route takes up to
    /// this value. Unset, every route takes the listener's limit.
    /// A request over its limit is refused `413 Payload Too Large`, which the
    /// ITS-REST status table does not list but admits as an additional
    /// non-conflicting code (overview `Requests_and_responses.md` §HTTP status
    /// codes) and which RFC 9110 §15.5.14 defines for this refusal. No openEHR
    /// spec bounds a request body — our own design.
    pub body_limit_bytes: Option<usize>,
}

impl Default for RestConfig {
    fn default() -> Self {
        Self {
            base_path: "/ferroehr/rest/openehr/v1".to_owned(),
            swagger_ui: AccessLevel::Private,
            cors_permissive: false,
            // The service layer's own default, so an unset `[rest] system_id`
            // boots exactly as the service does.
            system_id: crate::service::DEFAULT_SYSTEM_ID.to_owned(),
            identity: SystemOptionsConfig::default(),
            body_limit_bytes: None,
        }
    }
}

/// Whether a listen address is loopback-only, so a plaintext listener there is
/// not reachable off the host.
#[must_use]
pub const fn binds_loopback(listen: std::net::SocketAddr) -> bool {
    listen.ip().is_loopback()
}

impl RestConfig {
    /// The Swagger UI mount path, derived from the base path's parent.
    #[must_use]
    pub fn swagger_ui_path(&self) -> String {
        format!("{}/swagger-ui", self.rest_root())
    }

    /// The `OpenAPI` document path.
    #[must_use]
    pub fn openapi_json_path(&self) -> String {
        format!("{}/api-docs/openapi.json", self.rest_root())
    }

    /// The root of FerroEHR's own API extensions, beside the openEHR base
    /// path on their own version line: `{rest_root}/extensions/v1`.
    ///
    /// No openEHR spec governs where a server serves its extensions: our own
    /// design (#4330), so nothing ITS-REST does not define is served under
    /// the openEHR base path.
    #[must_use]
    pub fn extensions_path(&self) -> String {
        format!("{}/extensions/v1", self.rest_root())
    }

    /// The deployment's product root, where the non-openEHR surfaces live
    /// (`/status`, `/swagger-ui`, `/api-docs/openapi.json`,
    /// `/.well-known/smart-configuration`).
    ///
    /// The rule is the base path with the segments that name the openEHR API
    /// removed: the trailing `v1` API-version segment, which
    /// [`crate::config::FerroEhrConfig::validate`] guarantees is present, and an
    /// `openehr` segment immediately before it when the deployment spells one.
    /// So the default `/ferroehr/rest/openehr/v1` roots at `/ferroehr/rest`,
    /// `/ferroehr/openehr/v1` and `/ferroehr/v1` both root at `/ferroehr`, and
    /// `/ferroehr/cdr/v1` roots at `/ferroehr/cdr`. The result is always a
    /// strict parent of the base path, so a root-hosted route can never collide
    /// with the API nest. No openEHR spec governs where a server roots its
    /// non-API surfaces — our own design/extension.
    #[must_use]
    pub fn rest_root(&self) -> String {
        let without_version = self
            .base_path
            .strip_suffix("/v1")
            .unwrap_or(&self.base_path);
        without_version
            .strip_suffix("/openehr")
            .unwrap_or(without_version)
            .to_owned()
    }
}

/// Configuration of the ADMIN API group (`[admin]`).
///
/// NOTE: gating the admin surface behind an opt-in flag — when inactive every
/// admin route answers `405 Method Not Allowed` with an empty `Allow`
/// ("If a method is recognized but not allowed for the target resource, the
/// response SHOULD be `405 Method Not Allowed` status code" —
/// `docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §"HTTP Methods"), never a `403`. Physical, irreversible deletion is
/// dangerous, so the group stays off by default. No openEHR spec governs the
/// gate itself — our own design.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdminConfig {
    /// Whether the ADMIN API group is active. When `false`, every admin route
    /// answers `405 Method Not Allowed` without touching the backend.
    pub enabled: bool,
}

/// `[rest.identity]` — the `OPTIONS` System-Options manifest identity: the
/// **display** identity this deployment advertises (product, vendor and the
/// contract edition it implements).
///
/// Deliberately NOT the data-authoring identity: what a commit stamps into
/// `EHR.system_id`, `AUDIT_DETAILS.system_id`, and
/// `OBJECT_VERSION_ID.creating_system_id` is [`RestConfig::system_id`]. A
/// rebrand changes this block and nothing in the stored data; a change of
/// `system_id` changes what newly authored data says about its origin and
/// leaves the manifest alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SystemOptionsConfig {
    /// `Options.solution` — the product name.
    pub solution: String,
    /// `Options.solution_version` — the product version.
    pub solution_version: String,
    /// `Options.vendor` — the organisation providing the solution
    /// (ITS-REST `specifications/schemas/others/Options.yaml`, a plain string);
    /// defaults to the manufacturer's name
    /// ([`crate::manufacturer::MANUFACTURER`]).
    pub vendor: String,
    /// `Options.restapi_specs_version` — the ITS-REST version targeted.
    pub restapi_specs_version: String,
}

impl Default for SystemOptionsConfig {
    fn default() -> Self {
        Self {
            // `solution` names the product, `vendor` the organisation
            // providing it: the manufacturer of this release.
            solution: "FerroEHR".to_owned(),
            solution_version: env!("CARGO_PKG_VERSION").to_owned(),
            vendor: crate::manufacturer::MANUFACTURER.name.to_owned(),
            restapi_specs_version: crate::telemetry::provenance::ITS_REST.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::rest::{RestConfig, binds_loopback};

    #[test]
    fn loopback_listen_addresses_are_recognized() {
        for listen in ["127.0.0.1:8080", "127.0.0.53:8080", "[::1]:8080"] {
            assert!(binds_loopback(listen.parse().unwrap()), "{listen}");
        }
    }

    /// The plaintext-listener gap must open for anything reachable off the host.
    #[test]
    fn routable_listen_addresses_are_not_loopback() {
        for listen in ["0.0.0.0:8080", "10.0.0.4:8080", "[::]:8080"] {
            assert!(
                !binds_loopback(listen.parse().unwrap()),
                "{listen} must count as routable"
            );
        }
    }

    /// The REST root drops the openEHR API segments and nothing else, for every
    /// base-path shape `FerroEhrConfig::validate` admits.
    #[test]
    fn rest_root_drops_the_openehr_api_segments() {
        for (base_path, expected) in [
            ("/ferroehr/rest/openehr/v1", "/ferroehr/rest"),
            ("/ferroehr/v1", "/ferroehr"),
            ("/ferroehr/openehr/v1", "/ferroehr"),
            ("/ferroehr/cdr/v1", "/ferroehr/cdr"),
            (
                "/ferroehr/rest/openehr/v1/openehr/v1",
                "/ferroehr/rest/openehr/v1",
            ),
        ] {
            let cfg = RestConfig {
                base_path: base_path.to_owned(),
                ..RestConfig::default()
            };
            assert_eq!(cfg.rest_root(), expected, "base_path {base_path}");
            assert_eq!(cfg.swagger_ui_path(), format!("{expected}/swagger-ui"));
            assert_eq!(
                cfg.openapi_json_path(),
                format!("{expected}/api-docs/openapi.json")
            );
        }
    }
}
