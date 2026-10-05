// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! REST-adapter configuration types.
//!
//! No openEHR spec governs configuration mechanics — our own design. There is no
//! loader here: the whole server configuration is one tree
//! ([`crate::config::FerroEhrConfig`]) loaded once by the binary. This module
//! owns the REST-adapter's slice of it:
//!
//! - [`ServerConfig`] — the `[server]` section (the HTTP listener + REST
//!   surface + the `OPTIONS /` System-Options identity).
//! - `AppConfig` — the adapter's runtime view, assembled by the binary (the
//!   composition root) from the root config's `[server]`, `[auth]`, `[admin]`,
//!   `[smart]` section plus the extension-group mount toggles.
//!   `ferroehr-rest` cannot depend on the `ferroehr` binary crate that owns the
//!   root config, so the binary supplies exactly what the adapter needs
//!   (dependency inversion).
//!
//! [`AdminConfig`] is the `[admin]` section of
//! the root tree that this crate owns and the root references.

use serde::{Deserialize, Serialize};

use crate::config::management::AccessLevel;

/// The `[server]` section — the HTTP listener and REST surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Socket address to bind (e.g. `0.0.0.0:8080`).
    pub bind: String,
    /// The ITS-REST base path all API routes hang off.
    pub base_path: String,
    /// Concurrent-request admission cap before the server sheds load with
    /// `503` and `Retry-After` (shed, never queued). `0` disables shedding.
    /// The public `/status`, health, and discovery endpoints are never
    /// limited. No openEHR spec governs server overload — our own design
    /// (RFC 9110 §15.6.4).
    pub max_in_flight: usize,
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
    /// (`FERROEHR__SERVER__SYSTEM_ID`). Three wire values carry it:
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
    /// Distinct from [`SystemOptionsConfig`] (`[server.identity]`), which is the
    /// display identity of the `OPTIONS` System-Options manifest; `system_id`
    /// names which system authored the data.
    ///
    /// Defaults to [`crate::service::DEFAULT_SYSTEM_ID`]. No openEHR spec
    /// governs the configuration mechanism — our own design; the specs govern
    /// only that a system has such an identifier.
    pub system_id: String,
    /// The `OPTIONS /` System-Options manifest identity
    /// (`[server.identity]`). Sourced from config so the public identity and advertised profile
    /// are not string literals in the handler; the live endpoint list is
    /// supplied separately by `ferroehr_rest::router`. This is the *display* identity
    /// of the manifest — the data-authoring identity is [`Self::system_id`].
    pub identity: SystemOptionsConfig,
    /// `[server.tls]` — native TLS termination + client-certificate
    /// authentication (the IHE ATNA ITI-19 node-authentication posture).
    pub tls: TlsConfig,
    /// `[server.limits]` — the accepted request-body sizes.
    pub limits: BodyLimits,
    /// `[server.rate_limit]` — per-caller request rates.
    pub rate_limit: RateLimitConfig,
    /// `[server.connection]` — connection-level bounds, before a request exists.
    pub connection: ConnectionConfig,
}

/// `[server.connection]` — the bounds that apply BEFORE a request exists.
///
/// Every other limit in this configuration engages once a request has been
/// parsed and dispatched: the body limit, the request timeout, the rate limiter,
/// the in-flight shed. A client that opens a socket and then trickles request
/// headers reaches none of them, and costs itself almost nothing while holding a
/// connection — the slow-HTTP shape the OWASP Denial of Service Cheat Sheet
/// names under "minimum ingress rate threshold". This block is where that is
/// bounded.
///
/// No openEHR spec governs connection handling — our own design.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionConfig {
    /// How long a connection may take to deliver its complete request head, in
    /// seconds; `0` disables the bound (hyper's default — no limit).
    ///
    /// Applies to both listeners. Ten seconds is far longer than any real client
    /// needs to write a request head on a working link, and short enough that a
    /// stalled connection is reclaimed rather than parked.
    ///
    /// HTTP/1 only, because it is an HTTP/1 concept — an HTTP/2 request head
    /// arrives in HEADERS frames on a multiplexed connection. The HTTP/2 side is
    /// bounded by [`Self::max_concurrent_streams`] and the keep-alive pair below.
    pub header_read_timeout_secs: u64,
    /// The most HTTP/2 streams one connection may have open at once; `0` leaves
    /// hyper's default.
    ///
    /// This is HTTP/2's equivalent exposure, and a sharper one: a peer that opens
    /// streams and immediately cancels them makes the server do request setup
    /// work at almost no cost to itself — the amplification behind CVE-2023-44487
    /// ("HTTP/2 Rapid Reset"). Bounding concurrency bounds the work in flight per
    /// connection.
    pub max_concurrent_streams: u32,
    /// Interval between HTTP/2 keep-alive PINGs, in seconds; `0` disables them.
    ///
    /// Without a ping, a connection whose peer has vanished without a FIN is held
    /// until the OS notices, which can be a very long time. With one, a dead peer
    /// is detected in [`Self::http2_keep_alive_timeout_secs`] and the connection
    /// released.
    pub http2_keep_alive_interval_secs: u64,
    /// How long to wait for a keep-alive PING response before closing the
    /// connection, in seconds.
    pub http2_keep_alive_timeout_secs: u64,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            header_read_timeout_secs: 10,
            // 256 concurrent streams per connection: two orders of magnitude
            // above what a real client multiplexes, and a hard bound on the
            // rapid-reset amplification. hyper's own default is 200 for
            // comparison; this is set explicitly so it is a decision rather
            // than an inherited value.
            max_concurrent_streams: 256,
            http2_keep_alive_interval_secs: 30,
            http2_keep_alive_timeout_secs: 10,
        }
    }
}

impl ConnectionConfig {
    /// The header-read bound as a [`std::time::Duration`], or `None` when
    /// disabled.
    #[must_use]
    pub const fn header_read_timeout(&self) -> Option<std::time::Duration> {
        if self.header_read_timeout_secs > 0 {
            Some(std::time::Duration::from_secs(
                self.header_read_timeout_secs,
            ))
        } else {
            None
        }
    }

    /// The HTTP/2 stream-concurrency bound, or `None` to leave hyper's default.
    #[must_use]
    pub const fn stream_cap(&self) -> Option<u32> {
        if self.max_concurrent_streams > 0 {
            Some(self.max_concurrent_streams)
        } else {
            None
        }
    }

    /// The HTTP/2 keep-alive interval, or `None` when disabled.
    #[must_use]
    pub const fn http2_keep_alive_interval(&self) -> Option<std::time::Duration> {
        if self.http2_keep_alive_interval_secs > 0 {
            Some(std::time::Duration::from_secs(
                self.http2_keep_alive_interval_secs,
            ))
        } else {
            None
        }
    }

    /// The HTTP/2 keep-alive response deadline.
    #[must_use]
    pub const fn http2_keep_alive_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.http2_keep_alive_timeout_secs)
    }
}

/// `[server.rate_limit]` — the two-tier request-rate ceiling.
///
/// The limiter answers a different question from the load shed
/// ([`ServerConfig::max_in_flight`]), and the statuses keep them apart: the shed
/// protects capacity, refusing requests in flight at once with `503` plus
/// `Retry-After` (RFC 9110 §15.6.4), while the limiter protects fairness,
/// refusing one caller asking too often over time with `429` plus `Retry-After`
/// (RFC 6585 §4).
///
/// The address tier sits outside authentication, so a flood of unauthenticated
/// requests is refused before it can make the server verify a signature per
/// request. The principal tier sits inside authentication, keyed on the
/// authenticated subject, which is the only fair key for a clinical API: a
/// hospital behind one NAT is a single address.
///
/// Both defaults derive from the committed step-load record
/// (`docs/conformance/ferroehr/stress.json`), the authority for maximum
/// sustainable whole-server throughput on the reference SUT: the principal tier
/// is twice that ceiling and the address tier four times, so neither refuses a
/// caller until it asks for more than the entire server could serve. A
/// deployment that earns a higher volumetric class raises both in proportion.
///
/// No openEHR spec governs request rates — our own design.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimitConfig {
    /// Whether rate limiting is active. Off disables both tiers entirely — no
    /// limiter state is allocated and no request pays a check.
    pub enabled: bool,
    /// Sustained requests per second allowed per authenticated principal, on
    /// the clinical API subtree.
    pub principal_per_second: u64,
    /// How far a principal may burst above [`Self::principal_per_second`]
    /// before refusal.
    pub principal_burst: u32,
    /// Sustained requests per second allowed per client address, across the
    /// whole tree including the always-on public health family.
    pub address_per_second: u64,
    /// How far one address may burst above [`Self::address_per_second`].
    pub address_burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            principal_per_second: 1024,
            principal_burst: 2048,
            address_per_second: 2048,
            address_burst: 4096,
        }
    }
}

/// `[server.limits]` — the largest request body each route family accepts.
///
/// The payloads differ by more than an order of magnitude, so there are two
/// tiers: `body_bytes` governs the ordinary clinical surface and
/// `bulk_body_bytes` the routes that accept bulk by design (operational-template
/// upload, EHR-Extract import, TDD import). A request over its tier's limit is
/// refused `413 Payload Too Large`, which the ITS-REST status table does not
/// list but admits as an additional non-conflicting code (overview
/// `Requests_and_responses.md` §HTTP status codes) and which RFC 9110 §15.5.14
/// defines for this refusal. No openEHR spec bounds a request body — our own
/// design.
///
/// The defaults are sized against measured payloads: the largest operational
/// template in the vendored CKM corpus is 5.4 MB and the largest example
/// composition 526 KB, so the 16 MiB clinical tier clears the largest real
/// template roughly threefold, and the bulk tier is four times that for payloads
/// with no published bound. A deployment whose compositions embed large
/// `DV_MULTIMEDIA` data raises `body_bytes` as a deliberate operator decision.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BodyLimits {
    /// The largest body the ordinary clinical surface accepts, in bytes.
    pub body_bytes: usize,
    /// The largest body the bulk-upload routes accept, in bytes (template
    /// upload, `/message/import`, `/message/tdd`).
    pub bulk_body_bytes: usize,
}

impl Default for BodyLimits {
    fn default() -> Self {
        Self {
            body_bytes: 16 * 1024 * 1024,
            bulk_body_bytes: 64 * 1024 * 1024,
        }
    }
}

impl BodyLimits {
    /// Returns the ceiling across every tier — the limit the outermost layer
    /// enforces, so that no request can exceed the most permissive tier
    /// whatever path it names.
    #[must_use]
    pub const fn ceiling(&self) -> usize {
        if self.bulk_body_bytes > self.body_bytes {
            self.bulk_body_bytes
        } else {
            self.body_bytes
        }
    }
}

/// Client-certificate (mutual-TLS) policy for the main listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClientAuth {
    /// No client certificate requested.
    #[default]
    Off,
    /// A client certificate is requested and verified when presented;
    /// connections without one are still accepted.
    Optional,
    /// A verified client certificate is mandatory (IHE ATNA ITI-19: mutually
    /// authenticated nodes).
    Required,
}

/// `[server.tls]` — native TLS on the main listener.
///
/// Off by default (deployments commonly terminate TLS at an ingress); with it
/// on, the protocol floor is the rustls safe default (TLS 1.2+, strong suites
/// — the IETF BCP 195 posture), and `client_auth` adds the IHE ATNA ITI-19
/// mutual-TLS node authentication against an explicit trust anchor. The
/// separate-port management listener stays plain HTTP (an internal surface).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TlsConfig {
    /// Terminate TLS natively (`FERROEHR__SERVER__TLS__ENABLED`).
    pub enabled: bool,
    /// The server certificate chain, PEM (`FERROEHR__SERVER__TLS__CERT_FILE`).
    pub cert_file: Option<String>,
    /// The server private key, PEM (`FERROEHR__SERVER__TLS__KEY_FILE`).
    pub key_file: Option<String>,
    /// Client-certificate policy (`FERROEHR__SERVER__TLS__CLIENT_AUTH`):
    /// `off` | `optional` | `required`.
    pub client_auth: ClientAuth,
    /// The CA bundle client certificates must chain to, PEM
    /// (`FERROEHR__SERVER__TLS__CLIENT_CA_FILE`). Required when `client_auth`
    /// is not `off` — an explicit trust anchor, never the web PKI.
    pub client_ca_file: Option<String>,
    /// The lowest TLS version this listener will negotiate
    /// (`FERROEHR__SERVER__TLS__MIN_VERSION`): `"1.3"` (default) or `"1.2"`.
    pub min_version: TlsVersion,
}

/// The TLS protocol floor for the native listener.
///
/// Defaults to 1.3, with 1.2 available for compatibility, the OWASP Transport
/// Layer Security Cheat Sheet's own position: "web applications must default to
/// TLS 1.3 and may support TLS 1.2 for compatibility."
///
/// 1.0 and 1.1 are not representable: RFC 8996 deprecates them, PCI DSS forbids
/// them and NIST SP 800-52 Rev. 2 disallows them, so the type has no variant for
/// them and the rustls provider offers none either. Choose `V1_2` only when a
/// real client requires it, such as an older integration engine or a pinned Java
/// runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TlsVersion {
    /// TLS 1.3 only (RFC 8446) — the default.
    #[default]
    #[serde(rename = "1.3")]
    V1_3,
    /// TLS 1.2 and 1.3, for clients that cannot do 1.3.
    #[serde(rename = "1.2")]
    V1_2,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8080".to_owned(),
            base_path: "/ferroehr/rest/openehr/v1".to_owned(),
            // 256 bounds the worst-case buffered-request memory to a sane
            // envelope (the knee ladder OOM-killed the container at 1024
            // in-flight clinical commits) while still permitting ~10k req/s at
            // 25 ms latency (throughput = in-flight / latency, Little's law).
            max_in_flight: 256,
            swagger_ui: AccessLevel::Private,
            cors_permissive: false,
            // The service layer's own default, so an unset `[server] system_id`
            // boots exactly as the service does.
            system_id: crate::service::DEFAULT_SYSTEM_ID.to_owned(),
            identity: SystemOptionsConfig::default(),
            tls: TlsConfig::default(),
            limits: BodyLimits::default(),
            rate_limit: RateLimitConfig::default(),
            connection: ConnectionConfig::default(),
        }
    }
}

impl ServerConfig {
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

/// Configuration of the ADMIN API group (`[admin]`; SM `I_ADMIN_SERVICE`).
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

/// `[server.identity]` — the `OPTIONS` System-Options manifest identity: the
/// **display** identity this deployment advertises (product, vendor, the
/// contract edition and conformance profile it claims).
///
/// Deliberately NOT the data-authoring identity: what a commit stamps into
/// `EHR.system_id`, `AUDIT_DETAILS.system_id`, and
/// `OBJECT_VERSION_ID.creating_system_id` is [`ServerConfig::system_id`]. A
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
    /// `Options.vendor` — the organisation providing the solution.
    pub vendor: String,
    /// `Options.restapi_specs_version` — the ITS-REST version targeted.
    pub restapi_specs_version: String,
    /// `Options.conformance_profile` — CORE / STANDARD (CNF master03 profiles).
    pub conformance_profile: String,
}

impl Default for SystemOptionsConfig {
    fn default() -> Self {
        Self {
            // `solution` names the product, `vendor` the organisation.
            solution: "FerroEHR".to_owned(),
            solution_version: env!("CARGO_PKG_VERSION").to_owned(),
            vendor: "FerroEHR project".to_owned(),
            restapi_specs_version: crate::telemetry::provenance::ITS_REST.to_owned(),
            conformance_profile: crate::telemetry::provenance::CONFORMANCE_PROFILE.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::server::ServerConfig;

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
            let cfg = ServerConfig {
                base_path: base_path.to_owned(),
                ..ServerConfig::default()
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
