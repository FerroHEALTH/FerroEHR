// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! SMART App Launch configuration
//! (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master04-service_discovery.adoc`,
//! `.../master08-scopes.adoc`, `.../master09-experimental_features.adoc`).
//!
//! Off by default (`enabled = false`): a stock server serves no
//! `/.well-known/smart-configuration` document and runs no SMART scope gate, so
//! the wire is byte-identical to a non-SMART deployment — the same opt-in
//! extension-group convention the ADMIN/terminology/FHIR groups follow.
//!
//! This is the `[smart]` section of the one server configuration tree; it carries **no loader of its own** — the
//! whole tree is assembled once by `ferroehr::config` and this struct is
//! deserialized as a field of it. The discovery router is mounted from it in
//! `ferroehr_rest::router` and the scope gate reads it in
//! `ferroehr_rest::extensions::access::pep`.

use serde::{Deserialize, Serialize};
use url::Url;

/// SMART App Launch resource-server configuration.
///
/// The CDR is the SMART Platform's `org.openehr.rest` resource server (master02
/// §Glossary): it advertises the external Authorization-Server endpoints, parses
/// and enforces the master08 resource-scope grammar, and binds the master07/09
/// launch context to the patient compartment. It never issues tokens, registers
/// clients, or runs the `OAuth2` endpoints (those are Authorization-Server duties).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SmartConfig {
    /// Master switch. When `false`, the discovery document is not served (404)
    /// and the scope gate is inert — a stock server is unchanged.
    #[serde(default)]
    pub enabled: bool,

    /// The SMART *Platform* base URL the discovery document hangs off (master04
    /// §Service Discovery: the doc is served relative to the Platform/gateway
    /// base, not the FHIR base). When unset the discovery router defaults it to
    /// the REST root (`/ferroehr/rest`). A leading path segment is honoured, e.g.
    /// `/gateway/v1` → `/gateway/v1/.well-known/smart-configuration`.
    #[serde(default)]
    pub platform_base_url: Option<String>,

    /// The server's externally-reachable origin (scheme + host [+ port]), e.g.
    /// `https://cdr.example.com`. master04 §Services makes every `services.*`
    /// entry's `baseUrl` an **"Absolute URL to the root of the API (required)"**,
    /// so the discovery document prefixes this origin onto the served base
    /// paths. REQUIRED when SMART is enabled ([`Self::validate`]).
    #[serde(default)]
    pub public_base_url: Option<String>,

    /// The token claim carrying the resolved openEHR `EHR` id for the launch
    /// context (master07 §Context Selection token-response table: `ehrId`).
    /// Default `ehrId`.
    pub ehr_id_claim: String,

    /// The fallback launch-context claim when [`Self::ehr_id_claim`] is absent
    /// (the standard SMART `patient` context attribute, master07). Default
    /// `patient`.
    pub patient_claim: String,

    /// Fail-closed switch (master08 §Scopes ¶2: the Platform must validate
    /// requested scopes against access-control policy). When `true`, a Bearer
    /// token that carries **no** matching SMART resource scope for a
    /// scope-governed operation is denied. When `false` (default) SMART is
    /// advisory: the gate enforces only when the token actually carries SMART
    /// resource scopes for that resource family, so a non-SMART token is
    /// unaffected.
    #[serde(default)]
    pub require_smart_scopes: bool,

    /// Episode context (master09 §Experimental: Episode Context) — experimental,
    /// advertised only. `episode.enabled = true` advertises
    /// `context-openehr-episode` and accepts the `launch/episode` scope +
    /// `episodeId` claim, but applies **no** episode-scoped filtering (openEHR
    /// has no first-class Episode resource yet; master09 states the semantics
    /// are "currently implementation-defined").
    #[serde(default)]
    pub episode: EpisodeConfig,

    /// Advertise `launch-base64-json` (master09 §Launch Parameter as a Token) —
    /// experimental. The base64-JSON `launch` object is consumed by the
    /// *Application*, not the CDR; we only advertise the capability. Default off.
    #[serde(default)]
    pub launch_base64_json: bool,

    /// The external Authorization-Server / OIDC endpoints advertised verbatim in
    /// the discovery document (master04 §Authentication Endpoints). The CDR does
    /// not implement any of these; it copies the operator's values.
    #[serde(default)]
    pub endpoints: SmartEndpoints,
}

/// Episode-context sub-config (master09 §Experimental: Episode Context).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EpisodeConfig {
    /// Advertise + accept episode context. Advisory only (no filtering).
    #[serde(default)]
    pub enabled: bool,
}

/// The Authorization-Server endpoints + `OAuth2` metadata advertised in the
/// `/.well-known/smart-configuration` document (master04 §Authentication
/// Endpoints).
///
/// Every field is operator-supplied; an unset optional endpoint is simply
/// omitted from the document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SmartEndpoints {
    /// The token/OIDC `issuer`. When unset the discovery router falls back to
    /// the configured OIDC bearer issuer (`auth.oidc.issuer`).
    #[serde(default)]
    pub issuer: Option<String>,
    /// The JWKS document URL (`jwks_uri`).
    #[serde(default)]
    pub jwks_uri: Option<String>,
    /// The `OAuth2` `authorization_endpoint`.
    #[serde(default)]
    pub authorization_endpoint: Option<String>,
    /// The `OAuth2` `token_endpoint`.
    #[serde(default)]
    pub token_endpoint: Option<String>,
    /// The dynamic-client `registration_endpoint` (master03 recommends
    /// out-of-band registration; advertised only when the AS offers it).
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    /// The token `introspection_endpoint`.
    #[serde(default)]
    pub introspection_endpoint: Option<String>,
    /// The token `revocation_endpoint`.
    #[serde(default)]
    pub revocation_endpoint: Option<String>,
    /// The user `management_endpoint`.
    #[serde(default)]
    pub management_endpoint: Option<String>,
    /// `token_endpoint_auth_methods_supported` (e.g. `client_secret_basic`,
    /// `private_key_jwt`).
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
    /// `grant_types_supported`. master06 §Deprecated Flows: `implicit` and the
    /// resource-owner-password grant MUST NOT appear — [`SmartConfig::validate`]
    /// rejects them.
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
    /// `response_types_supported` (e.g. `code`).
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    /// `code_challenge_methods_supported` (e.g. `S256`).
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    /// `scopes_supported`. When empty the discovery router advertises a default
    /// list reflecting the scopes the CDR actually enforces.
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    /// Accept `http://` (or otherwise non-absolute-`https`) advertised endpoints
    /// and `public_base_url`.
    ///
    /// DEVELOPMENT AND TEST ONLY, and named rather than silent for the same
    /// reason `auth.oidc.allow_insecure_issuer` is: this document tells
    /// third-party applications where to obtain tokens, so a plaintext endpoint
    /// exposes an authorization code or an access token to anyone on the path.
    #[serde(default)]
    pub allow_insecure_endpoints: bool,
    /// `capabilities` the operator additionally advertises (the HL7-defined base
    /// capabilities — `launch-ehr`, `sso-openid-connect`, `client-public`, … —
    /// live in the external SMART App Launch framework; master04 §Capabilities:
    /// the openEHR list is "In addition to those scopes defined in the original
    /// SMART App Launch framework"). Appended to the openEHR capabilities the
    /// CDR derives itself; duplicates are dropped.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl Default for SmartConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            platform_base_url: None,
            public_base_url: None,
            ehr_id_claim: "ehrId".to_owned(),
            patient_claim: "patient".to_owned(),
            require_smart_scopes: false,
            episode: EpisodeConfig::default(),
            launch_base64_json: false,
            endpoints: SmartEndpoints::default(),
        }
    }
}

impl SmartConfig {
    /// Boot validation.
    ///
    /// - master06 §Deprecated Flows: the CDR must never advertise the Implicit
    ///   or Resource-Owner-Password grants.
    /// - When SMART is enabled, `public_base_url` is required (master04
    ///   §Services types `baseUrl` as an "Absolute URL … (required)") and the
    ///   three core Authorization-Server endpoints `issuer`,
    ///   `authorization_endpoint` and `token_endpoint` must be present (master04
    ///   §Authentication Endpoints delegates their requiredness to OIDC
    ///   Discovery and the HL7 SMART metadata, both of which require them).
    ///
    /// # Errors
    /// A message naming the offending grant type or the missing required field.
    pub fn validate(&self) -> Result<(), String> {
        for grant in &self.endpoints.grant_types_supported {
            let g = grant.to_ascii_lowercase();
            if g == "implicit" || g == "password" || g == "resource_owner_password" {
                return Err(format!(
                    "grant type '{grant}' is deprecated and MUST NOT be advertised \
                     (master06 §Deprecated Flows)"
                ));
            }
        }
        self.validate_no_userinfo()?;
        if self.enabled {
            let origin = self.public_base_url.as_deref().unwrap_or("");
            if !(origin.starts_with("http://") || origin.starts_with("https://")) {
                return Err("smart.public_base_url (an absolute http(s) origin, e.g. \
                     'https://cdr.example.com') is required when SMART is \
                     enabled — master04 §Services makes every services.*.baseUrl \
                     an absolute URL"
                    .to_owned());
            }
            if self.endpoints.authorization_endpoint.is_none()
                || self.endpoints.token_endpoint.is_none()
            {
                return Err(
                    "smart.endpoints.authorization_endpoint and .token_endpoint \
                     are required when SMART is enabled (master04 \
                     §Authentication Endpoints via OIDC Discovery / HL7 SMART \
                     metadata requiredness)"
                        .to_owned(),
                );
            }
            self.validate_published_endpoints()?;
            self.validate_published_metadata()?;
        }
        Ok(())
    }

    /// No URL this section holds carries a `userinfo` component, whether or not
    /// SMART is enabled.
    ///
    /// The discovery document publishes these URLs to every application, so a
    /// credential in one would be served to anyone who asks; refusing it at
    /// boot also keeps the redacted configuration views free of it. The
    /// issuer is additionally an OpenID Connect issuer identifier, whose only
    /// components are scheme, host, port and path (OpenID Connect Core 1.0
    /// §1.2). A `platform_base_url` given as a bare path holds no authority and
    /// passes.
    ///
    /// # Errors
    /// A message naming the offending key; it never quotes the value.
    fn validate_no_userinfo(&self) -> Result<(), String> {
        let e = &self.endpoints;
        let candidates: [(&str, Option<&String>); 10] = [
            ("smart.platform_base_url", self.platform_base_url.as_ref()),
            ("smart.public_base_url", self.public_base_url.as_ref()),
            ("smart.endpoints.issuer", e.issuer.as_ref()),
            ("smart.endpoints.jwks_uri", e.jwks_uri.as_ref()),
            (
                "smart.endpoints.authorization_endpoint",
                e.authorization_endpoint.as_ref(),
            ),
            ("smart.endpoints.token_endpoint", e.token_endpoint.as_ref()),
            (
                "smart.endpoints.registration_endpoint",
                e.registration_endpoint.as_ref(),
            ),
            (
                "smart.endpoints.introspection_endpoint",
                e.introspection_endpoint.as_ref(),
            ),
            (
                "smart.endpoints.revocation_endpoint",
                e.revocation_endpoint.as_ref(),
            ),
            (
                "smart.endpoints.management_endpoint",
                e.management_endpoint.as_ref(),
            ),
        ];
        for (key, value) in candidates {
            let Some(raw) = value else { continue };
            // NOTE: no openEHR spec governs this — our own design; a value that does
            // not parse as an absolute URL has no authority, so no `userinfo`.
            let Ok(url) = Url::parse(raw.trim()) else {
                continue;
            };
            if !url.username().is_empty() || url.password().is_some() {
                return Err(format!(
                    "{key} must carry no userinfo: the discovery document publishes it to \
                     every application, so a credential in it would be served to anyone"
                ));
            }
        }
        Ok(())
    }

    /// Every endpoint the discovery document publishes is an absolute `https`
    /// URL.
    ///
    /// This document is read by third-party applications to decide where to send
    /// an authorization request and where to exchange a code, so a relative path
    /// is unusable and a plaintext one exposes the code and the resulting access
    /// token (RFC 8414 §2 requires `https` for the issuer; §6.2 states the
    /// transport requirement for the metadata itself; RFC 6749 §3.1.2.1 requires
    /// `https` for the redirection and endpoint URIs). `public_base_url` is held
    /// to the same bar, because the `services` map's `baseUrl` is built from it.
    ///
    /// # Errors
    /// A message naming the offending key and what it must be.
    fn validate_published_endpoints(&self) -> Result<(), String> {
        let e = &self.endpoints;
        let candidates: [(&str, Option<&String>); 9] = [
            ("smart.public_base_url", self.public_base_url.as_ref()),
            ("smart.endpoints.issuer", e.issuer.as_ref()),
            ("smart.endpoints.jwks_uri", e.jwks_uri.as_ref()),
            (
                "smart.endpoints.authorization_endpoint",
                e.authorization_endpoint.as_ref(),
            ),
            ("smart.endpoints.token_endpoint", e.token_endpoint.as_ref()),
            (
                "smart.endpoints.registration_endpoint",
                e.registration_endpoint.as_ref(),
            ),
            (
                "smart.endpoints.introspection_endpoint",
                e.introspection_endpoint.as_ref(),
            ),
            (
                "smart.endpoints.revocation_endpoint",
                e.revocation_endpoint.as_ref(),
            ),
            (
                "smart.endpoints.management_endpoint",
                e.management_endpoint.as_ref(),
            ),
        ];
        for (key, value) in candidates {
            let Some(raw) = value else { continue };
            let url = Url::parse(raw.trim())
                .map_err(|err| format!("{key} is not an absolute URL: {err} (RFC 8414 §2)"))?;
            if url.scheme() != "https" && !e.allow_insecure_endpoints {
                return Err(format!(
                    "{key} must use the https scheme: this document tells third-party \
                     applications where to obtain tokens, so a plaintext endpoint exposes the \
                     authorization code and the access token (RFC 6749 §3.1.2.1, RFC 8414 §6.2); \
                     set smart.endpoints.allow_insecure_endpoints = true for a development \
                     authorization server"
                ));
            }
        }
        // The issuer additionally carries RFC 8414 §2's structural rules — the
        // same ones `auth.oidc.issuer` is held to, since it is the same identity.
        if let Some(issuer) = &e.issuer {
            let url = Url::parse(issuer.trim())
                .map_err(|err| format!("smart.endpoints.issuer is not an absolute URL: {err}"))?;
            if url.query().is_some() || url.fragment().is_some() {
                return Err(
                    "smart.endpoints.issuer must have no query and no fragment component \
                     (RFC 8414 §2)"
                        .to_owned(),
                );
            }
        }
        Ok(())
    }

    /// The metadata arrays the document publishes must describe a usable, current
    /// authorization server.
    ///
    /// An empty array is not silence — it is a positive claim that the server
    /// supports none of that thing, and an app that believes it complies. The two
    /// that matter are `response_types_supported`, which RFC 8414 §2 marks
    /// REQUIRED, and `code_challenge_methods_supported`: SMART App Launch requires
    /// PKCE (RFC 7636) and publishing an empty list tells every app not to use it.
    ///
    /// # Errors
    /// A message naming the offending key and the clause requiring it.
    fn validate_published_metadata(&self) -> Result<(), String> {
        let e = &self.endpoints;
        if e.response_types_supported.is_empty() {
            return Err(
                "smart.endpoints.response_types_supported must list at least one response type \
                 (RFC 8414 §2 marks the field REQUIRED); SMART App Launch uses the \
                 authorization-code flow, so `code` is the usual value"
                    .to_owned(),
            );
        }
        if e.token_endpoint_auth_methods_supported.is_empty() {
            return Err(
                "smart.endpoints.token_endpoint_auth_methods_supported must list at least one \
                 method: an empty list advertises an authorization server that authenticates no \
                 client at all"
                    .to_owned(),
            );
        }
        let advertises_pkce = e
            .code_challenge_methods_supported
            .iter()
            .any(|m| m.trim().eq_ignore_ascii_case("S256"));
        if !advertises_pkce {
            return Err(
                "smart.endpoints.code_challenge_methods_supported must include \"S256\": SMART \
                 App Launch requires PKCE (RFC 7636), and publishing a list without it tells \
                 every app that this authorization server cannot do it — a downgrade we would \
                 be asking clients to accept. `plain` alone is not sufficient (RFC 7636 §7.2)"
                    .to_owned(),
            );
        }
        Ok(())
    }

    /// Emit a boot warning when SMART is enabled but no Bearer/OIDC mechanism is
    /// configured: SMART scopes ride only Bearer tokens (Basic carries none), so
    /// an enabled-but-bearerless deployment can serve discovery yet never
    /// enforce a scope. Non-fatal.
    pub fn warn_if_bearerless(&self, oidc_configured: bool) {
        if self.enabled && !oidc_configured {
            tracing::warn!(
                "SMART is enabled but no OIDC bearer validation is configured; \
                 SMART scopes only ride Bearer tokens, so the scope gate will \
                 never engage (master06 §Authentication)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_and_sane() {
        let c = SmartConfig::default();
        assert!(!c.enabled);
        assert!(c.platform_base_url.is_none());
        assert!(c.public_base_url.is_none());
        assert!(c.endpoints.capabilities.is_empty());
        assert_eq!(c.ehr_id_claim, "ehrId");
        assert_eq!(c.patient_claim, "patient");
        assert!(!c.require_smart_scopes);
        assert!(!c.episode.enabled);
        assert!(!c.launch_base64_json);
        assert!(c.endpoints.issuer.is_none());
    }

    /// A published URL carrying `userinfo` is refused whether or not SMART is
    /// enabled, and the refusal never quotes the credential.
    #[test]
    fn userinfo_in_any_published_url_is_refused() {
        let leaky = "https://as:hunter2@as.example/x".to_owned();
        let setters: [fn(&mut SmartConfig, String); 10] = [
            |c, v| c.platform_base_url = Some(v),
            |c, v| c.public_base_url = Some(v),
            |c, v| c.endpoints.issuer = Some(v),
            |c, v| c.endpoints.jwks_uri = Some(v),
            |c, v| c.endpoints.authorization_endpoint = Some(v),
            |c, v| c.endpoints.token_endpoint = Some(v),
            |c, v| c.endpoints.registration_endpoint = Some(v),
            |c, v| c.endpoints.introspection_endpoint = Some(v),
            |c, v| c.endpoints.revocation_endpoint = Some(v),
            |c, v| c.endpoints.management_endpoint = Some(v),
        ];
        for set in setters {
            let mut c = SmartConfig::default();
            set(&mut c, leaky.clone());
            let err = c
                .validate()
                .expect_err("userinfo is refused while disabled");
            assert!(err.contains("must carry no userinfo"), "{err}");
            assert!(!err.contains("hunter2"), "{err}");
        }
        // A bare-path platform base holds no authority.
        let c = SmartConfig {
            platform_base_url: Some("/gateway/v1".to_owned()),
            ..SmartConfig::default()
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn validate_rejects_deprecated_grants() {
        let mut c = SmartConfig::default();
        c.endpoints.grant_types_supported = vec!["authorization_code".to_owned()];
        assert!(c.validate().is_ok());

        c.endpoints.grant_types_supported = vec!["implicit".to_owned()];
        assert!(c.validate().is_err());

        c.endpoints.grant_types_supported = vec!["PASSWORD".to_owned()];
        assert!(c.validate().is_err());
    }

    #[test]
    fn enabled_requires_origin_and_core_endpoints() {
        let mut c = SmartConfig {
            enabled: true,
            ..SmartConfig::default()
        };
        // Missing everything → the origin requirement fires first.
        assert!(c.validate().is_err());
        c.public_base_url = Some("https://cdr.example.com".to_owned());
        // Origin present but no AS endpoints → still an error.
        assert!(c.validate().is_err());
        c.endpoints.authorization_endpoint = Some("https://as.example/authorize".to_owned());
        c.endpoints.token_endpoint = Some("https://as.example/token".to_owned());
        // Endpoints alone still do not make a publishable document: the metadata
        // arrays below are what an app reads to decide HOW to talk to them.
        assert!(c.validate().is_err());
        c.endpoints.response_types_supported = vec!["code".to_owned()];
        c.endpoints.token_endpoint_auth_methods_supported = vec!["client_secret_basic".to_owned()];
        c.endpoints.code_challenge_methods_supported = vec!["S256".to_owned()];
        assert!(c.validate().is_ok());
        // A relative origin is not an absolute URL.
        c.public_base_url = Some("/ferroehr".to_owned());
        assert!(c.validate().is_err());
    }

    /// A publishable `[smart]` table: the endpoints plus the metadata an app
    /// needs to use them.
    fn publishable() -> SmartConfig {
        SmartConfig {
            enabled: true,
            public_base_url: Some("https://cdr.example.com".to_owned()),
            endpoints: SmartEndpoints {
                issuer: Some("https://as.example/realms/ferroehr".to_owned()),
                authorization_endpoint: Some("https://as.example/authorize".to_owned()),
                token_endpoint: Some("https://as.example/token".to_owned()),
                response_types_supported: vec!["code".to_owned()],
                token_endpoint_auth_methods_supported: vec!["client_secret_basic".to_owned()],
                code_challenge_methods_supported: vec!["S256".to_owned()],
                ..SmartEndpoints::default()
            },
            ..SmartConfig::default()
        }
    }

    /// This document tells third-party applications where to send an
    /// authorization request and where to exchange a code, so a plaintext
    /// endpoint exposes the code and the access token (RFC 6749 §3.1.2.1,
    /// RFC 8414 §6.2). The opt-out is explicit, mirroring
    /// `auth.oidc.allow_insecure_issuer`.
    #[test]
    fn a_plaintext_advertised_endpoint_is_a_boot_error() {
        let mut c = publishable();
        c.endpoints.token_endpoint = Some("http://as.example/token".to_owned());
        let err = c
            .validate()
            .expect_err("plaintext endpoint must be refused");
        assert!(err.contains("token_endpoint"), "{err}");
        assert!(err.contains("https"), "{err}");

        c.endpoints.allow_insecure_endpoints = true;
        assert!(
            c.validate().is_ok(),
            "the development opt-out must be honoured once set explicitly",
        );
    }

    /// RFC 8414 §2 gives the issuer identifier structural rules — the same ones
    /// `auth.oidc.issuer` is held to, because it is the same identity.
    #[test]
    fn an_advertised_issuer_with_query_or_fragment_is_a_boot_error() {
        for bad in [
            "https://as.example/realms/ferroehr?tenant=a",
            "https://as.example/realms/ferroehr#frag",
        ] {
            let mut c = publishable();
            c.endpoints.issuer = Some(bad.to_owned());
            assert!(
                c.validate().is_err(),
                "`{bad}` must be refused (RFC 8414 §2)",
            );
        }
    }

    /// An empty array is not silence — it is a claim that the authorization
    /// server supports none of that thing, and an app that reads it complies.
    #[test]
    fn published_metadata_must_describe_a_usable_server() {
        // RFC 8414 §2 marks `response_types_supported` REQUIRED.
        let mut c = publishable();
        c.endpoints.response_types_supported.clear();
        assert!(c.validate().is_err());

        // An authorization server that authenticates no client at all.
        let mut c = publishable();
        c.endpoints.token_endpoint_auth_methods_supported.clear();
        assert!(c.validate().is_err());

        // SMART App Launch requires PKCE, and `plain` alone is not sufficient
        // (RFC 7636 §7.2).
        let mut c = publishable();
        c.endpoints.code_challenge_methods_supported = vec!["plain".to_owned()];
        let err = c.validate().expect_err("PKCE S256 must be advertised");
        assert!(err.contains("S256"), "{err}");
    }
}
