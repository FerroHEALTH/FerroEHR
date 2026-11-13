// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[api]` table: the openEHR REST API the listener serves.
//!
//! No openEHR spec governs configuration mechanics — our own design. There is no
//! loader here: the whole server configuration is one tree
//! ([`crate::config::FerroEhrConfig`]) loaded once by the binary. The listener,
//! its TLS, its limits and its drain are the family's `[server]` section
//! (`ferrohealth_server::section::ServerSection`); this module owns the API
//! the listener serves: the openEHR API base path, the system identifier, the
//! `OPTIONS /` System-Options identity, the `OpenAPI` UI access level, CORS,
//! the clinical body tier, and the ADMIN and terminology API groups.

use ferrohealth_config::units::{ByteSize, Duration};
use serde::{Deserialize, Serialize};

use crate::config::management::AccessLevel;

/// The `[api]` table — the openEHR REST API the listener serves.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct ApiConfig {
    /// The ITS-REST base path all API routes hang off, from the root of the
    /// listener. `management.base_path` is set the same way; the two never
    /// overlap.
    pub base_path: String,
    /// Who may read the Swagger UI and the `OpenAPI` documents at the REST
    /// root: `off` (not mounted), `admin_only`, `private` (any authenticated
    /// principal; the default) or `public`. The documents list the whole
    /// enabled operation surface, admin and message groups included, so
    /// `public` discloses it to anyone who can reach the port. The same
    /// vocabulary the management endpoints use; the name is the `OpenAPI`
    /// Specification's, the document the UI renders.
    pub openapi_ui: AccessLevel,
    /// `[api.cors]`: the CORS protocol answers of the API (Fetch Standard
    /// §3.2). Absent, no CORS header is sent, so a browser on another origin
    /// cannot read a response.
    pub cors: Option<CorsConfig>,
    /// This CDR's own openEHR **system identifier** — the identity the
    /// deployment stamps into the data it authors
    /// (`FERROEHR__API__SYSTEM_ID`). Three wire values carry it:
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
    /// Distinct from [`SystemOptionsConfig`] (`[api.options]`), which is the
    /// display identity of the `OPTIONS` System-Options manifest; `system_id`
    /// names which system authored the data.
    ///
    /// Defaults to [`crate::service::DEFAULT_SYSTEM_ID`]. No openEHR spec
    /// governs the configuration mechanism — our own design; the specs govern
    /// only that a system has such an identifier.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-must-set" = true)))]
    pub system_id: String,
    /// The `OPTIONS /` System-Options manifest identity (`[api.options]`).
    /// Sourced from config so the public identity is not a string literal in
    /// the handler; the live endpoint list is supplied separately by
    /// `ferroehr_rest::router`. This is the *display* identity of the
    /// manifest — the data-authoring identity is [`Self::system_id`].
    pub options: SystemOptionsConfig,
    /// The largest body the ordinary clinical surface accepts, below the
    /// listener's `server.body_limit`.
    ///
    /// The payloads differ by more than an order of magnitude: the routes that
    /// accept bulk by design (operational-template upload and EHR-Extract import)
    /// take up to `server.body_limit`, and every other route takes up to
    /// this value. Unset, every route takes the listener's limit.
    /// A request over its limit is refused `413 Payload Too Large`, which the
    /// ITS-REST status table does not list but admits as an additional
    /// non-conflicting code (overview `Requests_and_responses.md` §HTTP status
    /// codes) and which RFC 9110 §15.5.14 defines for this refusal. No openEHR
    /// spec bounds a request body — our own design.
    pub body_limit: Option<ByteSize>,
    /// Whether the ADMIN API group is served.
    ///
    /// NOTE: ITS-REST `docs/overview/Requests_and_responses.md` §"HTTP Methods":
    /// a recognized method not allowed answers `405`; off, every admin route does.
    ///
    /// Physical, irreversible deletion is dangerous, so the group stays off by
    /// default. When `false`, every admin route answers `405 Method Not
    /// Allowed` with an empty `Allow`, never a `403`, without touching the
    /// backend. No openEHR spec governs the gate itself — our own design.
    pub admin: bool,
    /// Whether the terminology extension API is mounted. Off by default — the
    /// routes answer `404` unless enabled. The terminology server the CDR asks
    /// is `[terminology]`.
    pub terminology: bool,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            base_path: "/ferroehr/rest/openehr/v1".to_owned(),
            openapi_ui: AccessLevel::Private,
            cors: None,
            // The service layer's own default, so an unset `[api] system_id`
            // boots exactly as the service does.
            system_id: crate::service::DEFAULT_SYSTEM_ID.to_owned(),
            options: SystemOptionsConfig::default(),
            body_limit: None,
            admin: false,
            terminology: false,
        }
    }
}

/// The `[api.cors]` table: the CORS protocol response headers (Fetch Standard
/// §3.2.3 "HTTP responses"), each key named for the header it sets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct CorsConfig {
    /// `Access-Control-Allow-Origin`: the origins a browser may read
    /// responses for, each a serialized origin such as
    /// `https://app.example`, or `["*"]` for any origin (dev only).
    pub allow_origins: Vec<String>,
    /// `Access-Control-Allow-Methods`: the methods a preflight admits; empty
    /// admits the method the preflight asks for.
    pub allow_methods: Vec<String>,
    /// `Access-Control-Allow-Headers`: the request headers a preflight
    /// admits; empty admits the headers the preflight asks for.
    pub allow_headers: Vec<String>,
    /// `Access-Control-Expose-Headers`: the response headers a script may
    /// read beyond the CORS-safelisted ones.
    pub expose_headers: Vec<String>,
    /// `Access-Control-Allow-Credentials`: whether a credentialed request's
    /// response may be read. Refused beside `allow_origins = ["*"]`, which
    /// the Fetch Standard forbids together.
    pub allow_credentials: bool,
    /// `Access-Control-Max-Age`: how long a preflight answer may be cached;
    /// unset sends no header.
    pub max_age: Option<Duration>,
}

impl CorsConfig {
    /// Whether any origin may read responses (`allow_origins = ["*"]`).
    #[must_use]
    pub fn any_origin(&self) -> bool {
        self.allow_origins.iter().any(|origin| origin == "*")
    }

    /// The boot refusals of `[api.cors]`, each naming its key.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.allow_origins.is_empty() {
            errors.push(
                "[api.cors] names no allow_origins: list the origins a browser may read \
                 responses for, or remove the table"
                    .to_owned(),
            );
        }
        if self.any_origin() && self.allow_origins.len() > 1 {
            errors.push("api.cors.allow_origins lists \"*\" beside other origins".to_owned());
        }
        if self.any_origin() && self.allow_credentials {
            errors.push(
                "api.cors.allow_credentials = true beside allow_origins = [\"*\"]: the Fetch \
                 Standard §3.2.5 refuses a credentialed response to the wildcard origin"
                    .to_owned(),
            );
        }
        for (key, values) in [
            ("allow_methods", &self.allow_methods),
            ("allow_headers", &self.allow_headers),
            ("expose_headers", &self.expose_headers),
        ] {
            for value in values
                .iter()
                .filter(|value| http::HeaderName::try_from(value.as_str()).is_err())
            {
                errors.push(format!(
                    "api.cors.{key} names {value:?}, which is no RFC 9110 §5.6.2 token"
                ));
            }
        }
        for origin in self.allow_origins.iter().filter(|origin| *origin != "*") {
            let ok = url::Url::parse(origin).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.has_host()
                    && url.path() == "/"
                    && !origin.ends_with('/')
                    && url.query().is_none()
                    && url.username().is_empty()
            });
            if !ok {
                errors.push(format!(
                    "api.cors.allow_origins names {origin:?}, which is no serialized origin such \
                     as \"https://app.example\" (scheme, host and port only)"
                ));
            }
        }
        errors
    }
}

/// Whether a listen address is loopback-only, so a plaintext listener there is
/// not reachable off the host.
#[must_use]
pub const fn binds_loopback(listen: std::net::SocketAddr) -> bool {
    listen.ip().is_loopback()
}

impl ApiConfig {
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

/// `[api.options]` — the `OPTIONS` System-Options manifest identity.
///
/// The **display** identity this deployment advertises (product and vendor).
/// The product version and the contract edition it implements are facts of
/// the build ([`Self::solution_version`], [`Self::restapi_specs_version`]).
///
/// Deliberately NOT the data-authoring identity: what a commit stamps into
/// `EHR.system_id`, `AUDIT_DETAILS.system_id`, and
/// `OBJECT_VERSION_ID.creating_system_id` is [`ApiConfig::system_id`]. A
/// rebrand changes this block and nothing in the stored data; a change of
/// `system_id` changes what newly authored data says about its origin and
/// leaves the manifest alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
pub struct SystemOptionsConfig {
    /// `Options.solution` — the product name.
    pub solution: String,
    /// `Options.vendor` — the organisation providing the solution
    /// (ITS-REST `specifications/schemas/others/Options.yaml`, a plain string);
    /// defaults to the manufacturer's name
    /// ([`crate::manufacturer::MANUFACTURER`]).
    pub vendor: String,
}

impl Default for SystemOptionsConfig {
    fn default() -> Self {
        Self {
            // `solution` names the product, `vendor` the organisation
            // providing it: the manufacturer of this release.
            solution: "FerroEHR".to_owned(),
            vendor: crate::manufacturer::MANUFACTURER.name.to_owned(),
        }
    }
}

impl SystemOptionsConfig {
    /// `Options.solution_version` — the version of this build.
    #[must_use]
    pub const fn solution_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// `Options.restapi_specs_version` — the ITS-REST release this build
    /// implements.
    #[must_use]
    pub const fn restapi_specs_version() -> &'static str {
        crate::telemetry::provenance::ITS_REST
    }
}

#[cfg(test)]
mod tests {
    use crate::config::rest::{ApiConfig, CorsConfig, binds_loopback};

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
            let cfg = ApiConfig {
                base_path: base_path.to_owned(),
                ..ApiConfig::default()
            };
            assert_eq!(cfg.rest_root(), expected, "base_path {base_path}");
            assert_eq!(cfg.swagger_ui_path(), format!("{expected}/swagger-ui"));
            assert_eq!(
                cfg.openapi_json_path(),
                format!("{expected}/api-docs/openapi.json")
            );
        }
    }

    #[test]
    fn cors_names_origins_and_never_pairs_the_wildcard_with_credentials() {
        let with = |origins: &[&str], credentials: bool| CorsConfig {
            allow_origins: origins.iter().map(|o| (*o).to_owned()).collect(),
            allow_credentials: credentials,
            ..CorsConfig::default()
        };
        assert_eq!(
            with(&["https://app.example"], true).errors(),
            Vec::<String>::new()
        );
        assert_eq!(with(&["*"], false).errors(), Vec::<String>::new());
        assert_eq!(with(&[], false).errors().len(), 1);
        assert_eq!(with(&["*"], true).errors().len(), 1);
        assert_eq!(with(&["*", "https://app.example"], false).errors().len(), 1);
        for bad in [
            "app.example",
            "https://app.example/",
            "https://app.example/x",
            "ftp://a.b",
        ] {
            assert_eq!(with(&[bad], false).errors().len(), 1, "{bad}");
        }
    }
}
