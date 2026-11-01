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
use ferrohealth_server::section::ServerSection;
use ferrohealth_server::tls_client::TlsClientSection;
use ferrohealth_telemetry::section::TelemetrySection;
use secrecy::ExposeSecret;
use serde::Deserialize;

/// The single configuration root for the viewer.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ViewerConfig {
    /// The listener, its TLS, the request bounds and the drain: the family's
    /// `[server]` section.
    pub server: ServerSection,
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
    /// The console, the request log and the OpenTelemetry export: the
    /// family's `[telemetry]` section.
    pub telemetry: TelemetrySection,
}

impl Default for ViewerConfig {
    fn default() -> Self {
        Self {
            // NOTE: no specification governs this: our own design; a page form posts
            // whole templates and compositions, and a page may wait on several CDR calls.
            server: ServerSection {
                listen: std::net::SocketAddr::from(([127, 0, 0, 1], 3000)),
                request_timeout_ms: 60_000,
                body_limit_bytes: 16 * 1024 * 1024,
                ..ServerSection::default()
            },
            cdr: CdrConfig::default(),
            auth: AuthConfig::default(),
            login: LoginConfig::default(),
            session: SessionConfig::default(),
            ui: UiConfig::default(),
            telemetry: TelemetrySection::default(),
        }
    }
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
    /// The trust roots beside the platform's and the client identity the CDR
    /// client presents (`[cdr.tls]`).
    pub tls: TlsClientSection,
}

impl Default for CdrConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8080".to_owned(),
            base_path: "/ferroehr/rest/openehr/v1".to_owned(),
            management_base_url: String::new(),
            request_timeout_secs: 30,
            tls: TlsClientSection::default(),
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
    /// The trust roots beside the platform's and the client identity the
    /// issuer client presents (`[auth.oidc.tls]`).
    pub tls: TlsClientSection,
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

/// The loader of the `FERROEHR_VIEWER` namespace, with the viewer's own
/// defaults for `[server]` under the operator's values and `RUST_LOG` as the
/// fallback of `telemetry.filter`, as the CDR reads it.
///
/// The Compose parameters `FERROEHR_VIEWER_PORT` and `FERROEHR_VIEWER_IMAGE`
/// can reach the container environment, so the sweep passes them.
///
/// # Errors
/// [`ConfigError::Load`] if the prefix is refused, which it never is.
pub fn loader() -> Result<Loader, ConfigError> {
    Ok(Loader::new("FERROEHR_VIEWER")?
        .defaults("server", &ViewerConfig::default().server)?
        .fallback("RUST_LOG", "telemetry.filter", None)
        .reserve("FERROEHR_VIEWER_PORT")
        .reserve("FERROEHR_VIEWER_IMAGE"))
}

/// Load the viewer configuration: defaults < file < environment < `sets`.
///
/// The file is `path`, or the one `FERROEHR_VIEWER_CONFIG` names; with none
/// named, no file is read. Env overrides use the uniform grammar
/// `FERROEHR_VIEWER__<SECTION>__<KEY>` (e.g. `FERROEHR_VIEWER__CDR__BASE_URL`),
/// and `sets` are the `--set key=value` pairs.
///
/// # Errors
/// Returns a [`ConfigError`] on an unreadable/invalid file, an unknown key
/// or variable (strict deserialization), a type mismatch, an unreadable
/// `*_file` secret, or a value the types cannot refuse (`server.base_path`,
/// `server.caller_rate`).
pub fn load(path: Option<&Path>, sets: &[(String, String)]) -> Result<ViewerConfig, ConfigError> {
    // NOTE: a variable that is not UTF-8 cannot spell a `FERROEHR_VIEWER_` name, so the
    // sweep may leave it out; the loader reads the environment itself.
    let environment: BTreeMap<String, String> = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    let viewer_loader = loader()?;
    sweep(&viewer_loader, &environment)?;
    let loaded = viewer_loader.load_with::<ViewerConfig>(path, sets)?;
    finish(loaded.settings().clone())
}

/// Refuses every variable under `FERROEHR_VIEWER_` that is neither an
/// override nor a name the loader reserved or reads.
// NOTE: ferrohealth-config 0.2.5 `strict::sweep`: a binary whose prefix no other binary
// extends may run the sweep itself, and no FerroHEALTH binary extends `FERROEHR_VIEWER`.
fn sweep(loader: &Loader, environment: &BTreeMap<String, String>) -> Result<(), ConfigError> {
    let config = loader.config_variable();
    let mut known: Vec<&str> = loader.reserved().iter().map(String::as_str).collect();
    known.push(&config);
    let mut refusals = ferrohealth_config::strict::sweep(loader.prefix(), environment, &known);
    match refusals.len() {
        0 => Ok(()),
        1 => Err(refusals.remove(0).into()),
        _ => Err(ferrohealth_config::error::ConfigError::Refusals { refusals }.into()),
    }
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
    let viewer_loader = loader()?;
    sweep(&viewer_loader, env)?;
    let loaded = viewer_loader.load_sources::<ViewerConfig>(file, None, env)?;
    finish(loaded.settings().clone())
}

/// Resolves the `*_file` secrets and checks the values the types cannot.
fn finish(mut cfg: ViewerConfig) -> Result<ViewerConfig, ConfigError> {
    // NOTE: no specification governs this: our own design; the viewer's pages and links
    // are rooted at `/`, and it mounts no per-caller limit, so neither key would apply.
    if !cfg.server.base_path.as_str().is_empty() {
        return Err(ConfigError::Invalid(
            "server.base_path is not supported by the viewer, whose pages are served at `/`"
                .to_owned(),
        ));
    }
    if cfg.server.caller_rate.is_some() {
        return Err(ConfigError::Invalid(
            "server.caller_rate is not supported by the viewer, which mounts no per-caller limit"
                .to_owned(),
        ));
    }

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

    if !cfg.auth.oidc.public_base_url.is_empty() {
        public_origin(&cfg.auth.oidc.public_base_url)?;
    }

    Ok(cfg)
}

/// The viewer's public URL as the same-origin check reads it.
///
/// # Errors
/// [`ConfigError::Invalid`] when `public_base_url` is no URL or has no
/// origin of its own (RFC 6454 §4).
pub fn public_origin(public_base_url: &str) -> Result<url::Url, ConfigError> {
    let parsed = url::Url::parse(public_base_url).map_err(|_unparsed| {
        ConfigError::Invalid(format!(
            "auth.oidc.public_base_url `{public_base_url}` is no URL"
        ))
    })?;
    if parsed.origin().is_tuple() {
        Ok(parsed)
    } else {
        Err(ConfigError::Invalid(format!(
            "auth.oidc.public_base_url `{public_base_url}` has no origin of its own"
        )))
    }
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

    /// `[telemetry]` is the family's section, and `RUST_LOG` sets its filter
    /// below the canonical override, as on the CDR.
    #[test]
    fn the_telemetry_section_parses_and_rust_log_is_the_filter_fallback() {
        let parsed = load_sources(
            Some("[telemetry]\nformat = \"json\"\nfilter = \"warn\"\nlogged_query_parameters = [\"limit\"]\n"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            parsed.telemetry.format,
            ferrohealth_telemetry::section::Format::Json
        );
        assert_eq!(parsed.telemetry.filter.as_str(), "warn");
        assert_eq!(parsed.telemetry.logged_query_parameters, ["limit"]);
        assert_eq!(
            ViewerConfig::default().telemetry.filter,
            ferrohealth_telemetry::section::Filter::default()
        );

        let rust_log = BTreeMap::from([("RUST_LOG".to_owned(), "debug".to_owned())]);
        let parsed = load_sources(Some("[telemetry]\nfilter = \"warn\"\n"), &rust_log).unwrap();
        assert_eq!(parsed.telemetry.filter.as_str(), "debug");

        let both = BTreeMap::from([
            ("RUST_LOG".to_owned(), "debug".to_owned()),
            (
                "FERROEHR_VIEWER__TELEMETRY__FILTER".to_owned(),
                "error".to_owned(),
            ),
        ]);
        assert_eq!(
            load_sources(None, &both).unwrap().telemetry.filter.as_str(),
            "error"
        );
    }

    #[test]
    fn a_log_filter_that_does_not_parse_refuses_the_start() {
        let refused = BTreeMap::from([("RUST_LOG".to_owned(), "info,=,,".to_owned())]);
        assert!(load_sources(None, &refused).is_err());
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

    /// `[server]` starts from the viewer's own defaults, and a partial table
    /// keeps them for every key it does not name.
    #[test]
    fn the_server_section_layers_over_the_viewers_defaults() {
        let defaults = load_sources(None, &BTreeMap::new()).unwrap();
        assert_eq!("127.0.0.1:3000", defaults.server.listen.to_string());
        assert_eq!(16 * 1024 * 1024, defaults.server.body_limit_bytes);
        assert_eq!(60_000, defaults.server.request_timeout_ms);

        let env = BTreeMap::from([(
            "FERROEHR_VIEWER__SERVER__LISTEN".to_owned(),
            "0.0.0.0:3000".to_owned(),
        )]);
        let partial = load_sources(Some("[server]\ndrain_delay_ms = 5000\n"), &env).unwrap();
        assert_eq!("0.0.0.0:3000", partial.server.listen.to_string());
        assert_eq!(5_000, partial.server.drain_delay_ms);
        assert_eq!(16 * 1024 * 1024, partial.server.body_limit_bytes);
    }

    /// The keys of `[server]` the viewer cannot apply refuse the start.
    #[test]
    fn a_base_path_or_a_caller_rate_is_refused() {
        for (file, key) in [
            ("[server]\nbase_path = \"/viewer\"\n", "server.base_path"),
            (
                "[server.caller_rate]\nrequests_per_second = 5\nburst = 10\n",
                "server.caller_rate",
            ),
        ] {
            let refused = load_sources(Some(file), &BTreeMap::new())
                .unwrap_err()
                .to_string();
            assert!(refused.contains(key), "{refused}");
        }
    }

    /// The outbound TLS tables of the CDR and the issuer are the family's.
    #[test]
    fn the_cdr_and_issuer_tls_tables_parse() {
        let parsed = load_sources(
            Some(
                "[cdr.tls]\ntrust_roots_file = \"/run/cdr-ca.pem\"\nmin_version = \"1.2\"\n\n[auth.oidc.tls]\ntrust_roots_file = \"/run/issuer-ca.pem\"\n",
            ),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            Some(std::path::PathBuf::from("/run/cdr-ca.pem")),
            parsed.cdr.tls.trust_roots_file
        );
        assert_eq!(
            Some(std::path::PathBuf::from("/run/issuer-ca.pem")),
            parsed.auth.oidc.tls.trust_roots_file
        );
        assert!(
            load_sources(Some("[cdr.tls]\nca_file = \"x\"\n"), &BTreeMap::new()).is_err(),
            "an unknown key of the table is refused"
        );
    }

    /// A public URL with no origin of its own cannot anchor the same-origin
    /// check.
    #[test]
    fn a_public_url_without_an_origin_is_refused() {
        for url in ["not a url", "data:text/plain,viewer"] {
            let env = BTreeMap::from([(
                "FERROEHR_VIEWER__AUTH__OIDC__PUBLIC_BASE_URL".to_owned(),
                url.to_owned(),
            )]);
            let refused = load_sources(None, &env).unwrap_err().to_string();
            assert!(refused.contains("auth.oidc.public_base_url"), "{refused}");
        }
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
