// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Viewer configuration: one TOML file (`ferroehr-viewer.toml`) with
//! `FERROEHR_VIEWER__…` environment overrides, read by `ferrohealth-config`
//! as the CDR's is.
//!
//! No openEHR spec governs configuration — our own design. The viewer is
//! stateless: sessions ride a sealed cookie, and there is no database or local store
//! of domain state either — every fact it shows lives in the CDR and is read
//! over ITS-REST.

use std::collections::BTreeMap;
use std::path::Path;

use ferrohealth_config::loader::Loader;
use ferrohealth_secret::file::read_secret_file;
use secrecy::ExposeSecret;
use serde::Deserialize;

/// The single configuration root for the viewer.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ViewerConfig {
    /// The CDR connection.
    pub cdr: CdrConfig,
    /// Viewer login modes.
    pub auth: AuthConfig,
    /// Login-screen presentation.
    pub login: LoginConfig,
    /// Session behaviour.
    pub session: SessionConfig,
    /// The language the viewer renders in.
    pub ui: UiConfig,
}

/// `[ui]`: the language the viewer renders in.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// The BCP 47 tag of the locale a page renders in when the request's
    /// `Accept-Language` names no language the viewer's catalog holds.
    pub locale: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            locale: String::from("en"),
        }
    }
}

/// Where and how to reach the CDR.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CdrConfig {
    /// Base URL of the CDR (scheme://host:port, no trailing path) — the
    /// ITS-REST base path ([`Self::base_path`]) is appended by the client.
    pub base_url: String,
    /// The CDR's ITS-REST base path, the mirror of its own `server.base_path`
    /// (default `/ferroehr/rest/openehr/v1`).
    ///
    /// A deployment that shortens the CDR's base path sets the same value here,
    /// or the viewer calls paths the CDR does not serve. The CDR validates the
    /// shape at its own boot (first segment `ferroehr`, last segment `v1`); the
    /// viewer only normalizes a trailing slash away. No openEHR spec governs
    /// where a server roots its API — our own design/extension.
    pub base_path: String,
    /// Base URL of the CDR's management surface **including its base path**
    /// (e.g. `http://cdr.internal:9464/management`). Empty = derive it from
    /// [`Self::base_url`] with the CDR's default `/management` base path.
    ///
    /// One knob covers both ways a deployment can move that surface: the CDR
    /// can serve it from its own internal listener (`management.port`) and can
    /// rename its base path (`management.base_path`), so the viewer takes the
    /// whole prefix rather than guessing a port and a path separately. No
    /// openEHR spec governs a management surface — our own operational
    /// extension.
    pub management_base_url: String,
    /// Per-request timeout in seconds.
    pub request_timeout_secs: u64,
}

impl Default for CdrConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8080".to_owned(),
            base_path: "/ferroehr/rest/openehr/v1".to_owned(),
            management_base_url: String::new(),
            request_timeout_secs: 30,
        }
    }
}

impl CdrConfig {
    /// The configured ITS-REST base path with no trailing slash.
    #[must_use]
    pub fn rest_base_path(&self) -> &str {
        self.base_path.trim_end_matches('/')
    }

    /// The CDR's product root: the base path with the segments that name the
    /// openEHR API removed.
    ///
    /// The rule mirrors the CDR's own `server.base_path` → REST-root
    /// derivation, restated here because the viewer talks to the CDR strictly
    /// over ITS-REST and never links against its crates: the trailing `v1`
    /// API-version segment comes off, and an `openehr` segment directly before
    /// it when the deployment spells one. So `/ferroehr/rest/openehr/v1` roots
    /// at `/ferroehr/rest` and `/ferroehr/v1` at `/ferroehr`. That root is
    /// where the CDR serves `/status`, `/api-docs/…` and
    /// `/.well-known/smart-configuration`.
    #[must_use]
    pub fn rest_root(&self) -> &str {
        let base = self.rest_base_path();
        let without_version = base.strip_suffix("/v1").unwrap_or(base);
        without_version
            .strip_suffix("/openehr")
            .unwrap_or(without_version)
    }

    /// The management-surface base URL with no trailing slash: the configured
    /// value, or `{base_url}/management` (the CDR's default base path) when it
    /// is empty.
    #[must_use]
    pub fn management_base(&self) -> String {
        let configured = self.management_base_url.trim_end_matches('/');
        if configured.is_empty() {
            format!("{}/management", self.base_url.trim_end_matches('/'))
        } else {
            configured.to_owned()
        }
    }
}

/// Login-screen presentation: what the sign-in card says beyond the forms.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoginConfig {
    /// Informational text rendered on the sign-in card (empty = none); line
    /// breaks are preserved. A demo deployment states its public credentials
    /// and usage expectations here.
    pub notice: String,
    /// Links rendered under the sign-in card — an API reference, a
    /// documentation page. Each entry needs both `label` and `href`.
    pub links: Vec<crate::auth::LoginLink>,
}

/// Which login modes the viewer offers (design: both ship in v1).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// Offer the Basic (username/password) form; credentials are validated
    /// against the CDR and held server-side in the session.
    pub basic_enabled: bool,
    /// OIDC authorization-code login.
    pub oidc: OidcConfig,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            basic_enabled: true,
            oidc: OidcConfig::default(),
        }
    }
}

/// OIDC (Keycloak-style) settings for the viewer's own login.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct OidcConfig {
    /// Enable the OIDC login button + routes.
    pub enabled: bool,
    /// Issuer URL (discovery is derived from it).
    pub issuer: String,
    /// The `OAuth2` client id registered for the viewer.
    pub client_id: String,
    /// The `OAuth2` client secret (prefer `client_secret_file` in deployments).
    pub client_secret: String,
    /// Path to a file holding the client secret; wins over `client_secret`.
    pub client_secret_file: String,
    /// Externally visible base URL of the viewer (for the redirect URI),
    /// e.g. `http://localhost:3000`.
    pub public_base_url: String,
    /// Split-horizon DNS override for the issuer host, `host=ip:port`
    /// (e.g. `keycloak=127.0.0.1:8081`). Lets the viewer reach an issuer
    /// whose canonical hostname only resolves inside a container network
    /// while browsers and tokens keep the canonical URL. Empty = none.
    pub resolve: String,
    /// Additional scopes requested beyond `openid`.
    pub scopes: Vec<String>,
}

/// Session behaviour (a sealed cookie — no server-side store).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    /// Idle expiry, minutes.
    pub idle_minutes: u64,
    /// Set the `Secure` cookie flag. On by default (fail closed): a
    /// plain-HTTP context (local development, the e2e harness) opts OUT
    /// explicitly, production behind TLS needs nothing.
    pub cookie_secure: bool,
    /// The cookie-sealing secret, base64 of at least 64 bytes. Every
    /// instance of a scaled deployment must hold the same value — the
    /// session is an encrypted cookie any instance can open. Empty = an
    /// ephemeral per-instance key (single-replica only), with a startup
    /// warning.
    pub secret: String,
    /// Path to a file holding the sealing secret; wins over `secret`.
    pub secret_file: String,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            idle_minutes: 60,
            cookie_secure: true,
            secret: String::new(),
            secret_file: String::new(),
        }
    }
}

/// One configuration error, rendered as a single human line.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file or the environment could not be read into the tree.
    #[error(transparent)]
    Load(#[from] ferrohealth_config::error::ConfigError),
    /// A `*_file` secret could not be read.
    #[error(transparent)]
    Secret(#[from] ferrohealth_secret::file::SecretError),
    /// The tree read but one of its values is refused.
    #[error("{0}")]
    Invalid(String),
}

/// The loader of the `FERROEHR_VIEWER` namespace.
///
/// The Compose parameters `FERROEHR_VIEWER_PORT` and `FERROEHR_VIEWER_IMAGE`
/// can reach the container environment, so the sweep passes them.
///
/// # Errors
/// [`ConfigError::Load`] if the prefix is refused, which it never is.
pub fn loader() -> Result<Loader, ConfigError> {
    Ok(Loader::new("FERROEHR_VIEWER")?
        .reserve("FERROEHR_VIEWER_PORT")
        .reserve("FERROEHR_VIEWER_IMAGE"))
}

/// Load the viewer configuration: defaults < file < environment.
///
/// The file is the one `FERROEHR_VIEWER_CONFIG` names; with none named, no
/// file is read. Env overrides use the uniform grammar
/// `FERROEHR_VIEWER__<SECTION>__<KEY>` (e.g. `FERROEHR_VIEWER__CDR__BASE_URL`).
///
/// # Errors
/// Returns a [`ConfigError`] on an unreadable/invalid file, an unknown key
/// or variable (strict deserialization), a type mismatch, or an unreadable
/// `*_file` secret.
pub fn load() -> Result<ViewerConfig, ConfigError> {
    let loaded = loader()?.load::<ViewerConfig>(None)?;
    finish(loaded.settings().clone())
}

/// Load the viewer configuration from explicit inputs, the seam the tests
/// drive: the file text `file`, the environment `env`.
///
/// # Errors
/// Returns every error of [`load`].
pub fn load_sources(
    file: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<ViewerConfig, ConfigError> {
    let loaded = loader()?.load_sources::<ViewerConfig>(file, None, env)?;
    finish(loaded.settings().clone())
}

/// Resolves the `*_file` secrets and checks the values the types cannot.
fn finish(mut cfg: ViewerConfig) -> Result<ViewerConfig, ConfigError> {
    if !cfg.auth.oidc.client_secret_file.is_empty() {
        let secret = read_secret_file(
            "auth.oidc.client_secret",
            Path::new(&cfg.auth.oidc.client_secret_file),
        )?;
        secret
            .expose_secret()
            .clone_into(&mut cfg.auth.oidc.client_secret);
    }

    if !cfg.session.secret_file.is_empty() {
        let secret = read_secret_file("session.secret", Path::new(&cfg.session.secret_file))?;
        secret.expose_secret().clone_into(&mut cfg.session.secret);
    }

    if ferrohealth_ui::i18n::Locale::parse(&cfg.ui.locale).is_err() {
        return Err(ConfigError::Invalid(format!(
            "ui.locale `{}` is not a language tag",
            cfg.ui.locale
        )));
    }

    if cfg.auth.oidc.enabled {
        let mut missing = Vec::new();
        for (key, value) in [
            ("auth.oidc.issuer", &cfg.auth.oidc.issuer),
            ("auth.oidc.client_id", &cfg.auth.oidc.client_id),
            ("auth.oidc.public_base_url", &cfg.auth.oidc.public_base_url),
        ] {
            if value.is_empty() {
                missing.push(key);
            }
        }
        if !missing.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "auth.oidc.enabled = true requires: {}",
                missing.join(", ")
            )));
        }
    }

    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{CdrConfig, ViewerConfig, load_sources};

    /// The base path defaults to the CDR's own default, and the product root
    /// derives from it by the CDR's rule: drop the trailing `v1` segment, and
    /// an `openehr` segment directly before it when the deployment spells one.
    #[test]
    fn the_rest_root_derives_from_the_configured_base_path() {
        assert_eq!(
            CdrConfig::default().base_path,
            "/ferroehr/rest/openehr/v1",
            "the viewer default must mirror the CDR default"
        );
        for (base_path, expected) in [
            ("/ferroehr/rest/openehr/v1", "/ferroehr/rest"),
            ("/ferroehr/v1", "/ferroehr"),
            ("/ferroehr/openehr/v1", "/ferroehr"),
            ("/ferroehr/cdr/v1", "/ferroehr/cdr"),
            ("/ferroehr/v1/", "/ferroehr"),
        ] {
            let cfg = CdrConfig {
                base_path: base_path.to_owned(),
                ..CdrConfig::default()
            };
            assert_eq!(cfg.rest_root(), expected, "base_path {base_path}");
            assert_eq!(cfg.rest_base_path(), base_path.trim_end_matches('/'));
        }
    }

    #[test]
    fn management_base_defaults_to_the_cdr_origin() {
        let cfg = CdrConfig::default();
        assert_eq!(cfg.management_base(), "http://localhost:8080/management");
    }

    #[test]
    fn a_configured_management_base_wins_and_may_move_host_port_and_path() {
        // The whole prefix is configurable: a separate internal listener
        // (`management.port` CDR-side) and a renamed `management.base_path`
        // are the same one knob here.
        let cfg = CdrConfig {
            management_base_url: "http://cdr.internal:9464/ops/".to_owned(),
            ..CdrConfig::default()
        };
        assert_eq!(cfg.management_base(), "http://cdr.internal:9464/ops");
    }

    #[test]
    fn the_login_section_parses_and_defaults_to_empty() {
        let parsed = load_sources(
            Some(
                "[login]\nnotice = \"demo\"\n\n[[login.links]]\nlabel = \"API reference\"\nhref = \"https://example.org/swagger\"\n",
            ),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(parsed.login.notice, "demo");
        assert_eq!(parsed.login.links.len(), 1);
        assert_eq!(parsed.login.links[0].label, "API reference");
        assert_eq!(parsed.login.links[0].href, "https://example.org/swagger");

        let defaults = ViewerConfig::default();
        assert_eq!(defaults.login.notice, "");
        assert_eq!(defaults.login.links, Vec::<crate::auth::LoginLink>::new());
    }

    #[test]
    fn a_login_link_without_an_href_is_refused() {
        assert!(
            load_sources(
                Some("[[login.links]]\nlabel = \"dangling\"\n"),
                &BTreeMap::new()
            )
            .is_err()
        );
    }

    /// The uniform variable grammar binds, the Compose parameters pass the
    /// sweep, and any other variable under the prefix refuses the start.
    #[test]
    fn variables_bind_by_the_uniform_grammar_and_strays_are_refused() {
        let env = BTreeMap::from([
            (
                "FERROEHR_VIEWER__CDR__BASE_URL".to_owned(),
                "http://cdr:8080".to_owned(),
            ),
            ("FERROEHR_VIEWER_PORT".to_owned(), "8081".to_owned()),
        ]);
        let cfg = load_sources(None, &env).unwrap();
        assert_eq!(cfg.cdr.base_url, "http://cdr:8080");

        let stray = BTreeMap::from([("FERROEHR_VIEWER_CDR_BASE_URL".to_owned(), "x".to_owned())]);
        let err = load_sources(None, &stray).unwrap_err().to_string();
        assert!(err.contains("FERROEHR_VIEWER_CDR_BASE_URL"), "{err}");
    }

    #[test]
    fn a_trailing_slash_on_the_cdr_base_url_does_not_double_up() {
        let cfg = CdrConfig {
            base_url: "http://cdr:8080/".to_owned(),
            ..CdrConfig::default()
        };
        assert_eq!(cfg.management_base(), "http://cdr:8080/management");
    }
}
