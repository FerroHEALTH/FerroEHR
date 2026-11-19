// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Viewer configuration: one TOML file (`ferroehr-viewer.toml`) with
//! `FERROEHR_VIEWER__…` environment overrides, read by `ferrohealth-config`
//! as the CDR's is.
//!
//! No openEHR spec governs configuration — our own design. The sign-in and
//! the session are `ferrohealth-session`'s `[oidc]` and `[session]` sections,
//! and there is no database or local store of domain state — every fact the
//! viewer shows lives in the CDR and is read over ITS-REST.

use std::collections::BTreeMap;
use std::path::Path;

use ferrohealth_config::loader::Loader;
use ferrohealth_config::tls::TlsClientSection;
use ferrohealth_config::tls::TlsVersion;
use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_server::section::ServerSection;
use ferrohealth_session::config::{OidcSection, SessionSection};
use ferrohealth_telemetry::section::TelemetrySection;
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
    /// Viewer login modes and the viewer's public URL.
    pub auth: AuthConfig,
    /// The viewer's client at its OpenID Provider: `ferrohealth-session`'s
    /// `[oidc]`, read when `auth.oidc_enabled` is set.
    pub oidc: OidcSection,
    /// Login-screen presentation.
    pub login: LoginConfig,
    /// The session store and its lifetimes: `ferrohealth-session`'s
    /// `[session]`.
    pub session: SessionSection,
    /// The language the viewer renders in.
    pub ui: UiConfig,
    /// The SMART apps an EHR's page launches through FerroSMART.
    pub launch: crate::launch::LaunchSection,
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
                request_timeout: ferrohealth_config::units::Duration::from_mins(1),
                body_limit: ferrohealth_config::units::ByteSize::from_mib(16),
                ..ServerSection::default()
            },
            cdr: CdrConfig::default(),
            auth: AuthConfig::default(),
            // NOTE: no specification governs this: our own design; the provider connection
            // keeps the TLS 1.3 floor the viewer's issuer client had before ferrohealth-session.
            oidc: OidcSection {
                tls: TlsClientSection {
                    min_version: TlsVersion::Tls13,
                    ..TlsClientSection::default()
                },
                ..OidcSection::default()
            },
            login: LoginConfig::default(),
            // NOTE: no specification governs this: our own design; the viewer keeps the hour
            // idle and twelve hours absolute it ran on before ferrohealth-session.
            session: SessionSection {
                idle_timeout: ferrohealth_config::units::Duration::from_hours(1),
                absolute_timeout: ferrohealth_config::units::Duration::from_hours(12),
                ..SessionSection::default()
            },
            ui: UiConfig::default(),
            launch: crate::launch::LaunchSection::default(),
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
    /// can serve it from its own internal listener (`management.listen`) and can
    /// rename its base path (`management.base_path`), so the viewer takes the
    /// whole prefix rather than guessing a port and a path separately. No
    /// openEHR spec governs a management surface — our own operational
    /// extension.
    pub management_base_url: String,
    /// How long one request to the CDR may take.
    pub timeout: ferrohealth_config::units::Duration,
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
            timeout: ferrohealth_config::units::Duration::from_secs(30),
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

/// Which login modes the viewer offers, and its public URL.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// Offer the Basic (username/password) form; credentials are validated
    /// against the CDR and held server-side in the session.
    pub basic_enabled: bool,
    /// Offer the OpenID Connect sign-in at the provider `[oidc]` names.
    pub oidc_enabled: bool,
    /// The externally visible base URL of the viewer, e.g.
    /// `http://localhost:3000`: the origin the same-origin check compares a
    /// post with, and the base of `oidc.redirect_uri` and
    /// `oidc.post_logout_redirect_uri` when they are not set.
    pub public_base_url: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            basic_enabled: true,
            oidc_enabled: false,
            public_base_url: String::new(),
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
    /// `[oidc]` or `[session]` is refused.
    #[error(transparent)]
    Session(Box<ferrohealth_session::config::ConfigError>),
    /// `[launch]` is refused.
    #[error(transparent)]
    Launch(#[from] crate::launch::LaunchConfigError),
}

impl From<ferrohealth_session::config::ConfigError> for ConfigError {
    fn from(error: ferrohealth_session::config::ConfigError) -> Self {
        Self::Session(Box::new(error))
    }
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
        .defaults("session", &ViewerConfig::default().session)?
        .defaults("oidc", &ViewerConfig::default().oidc)?
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

    if ferrohealth_ui::i18n::Locale::parse(&cfg.ui.locale).is_err() {
        return Err(ConfigError::Invalid(format!(
            "ui.locale `{}` is not a language tag",
            cfg.ui.locale
        )));
    }

    if !cfg.auth.public_base_url.is_empty() {
        let public = public_origin(&cfg.auth.public_base_url)?;
        if cfg.oidc.redirect_uri.is_empty() {
            cfg.oidc.redirect_uri = SecretUrl::new(
                public
                    .join(ferrohealth_session::paths::CALLBACK)
                    .map_err(|_unjoined| {
                        ConfigError::Invalid(format!(
                            "auth.public_base_url `{}` takes no callback path",
                            cfg.auth.public_base_url
                        ))
                    })?
                    .to_string(),
            );
        }
        // NOTE: OpenID Connect RP-Initiated Logout 1.0 §3: the provider returns the operator to the
        // registered URI, here the viewer's own signed-out page.
        if cfg.oidc.post_logout_redirect_uri.is_none() {
            cfg.oidc.post_logout_redirect_uri = Some(SecretUrl::new(
                public
                    .join(crate::session::SIGNED_OUT_PAGE)
                    .map_err(|_unjoined| {
                        ConfigError::Invalid(format!(
                            "auth.public_base_url `{}` takes no signed-out path",
                            cfg.auth.public_base_url
                        ))
                    })?
                    .to_string(),
            ));
        }
    }

    // NOTE: ferrohealth-session 0.2.1 `config`: a sealed store with no key, a key beside a
    // memory store and an absolute lifetime shorter than the idle one refuse the start.
    if cfg.auth.oidc_enabled {
        cfg.session.check(
            &cfg.oidc
                .check_with_tls(&ferrohealth_smart::config::ServerTls)?,
        )?;
    } else {
        cfg.session.check_with(false)?;
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
            "auth.public_base_url `{public_base_url}` is no URL"
        ))
    })?;
    if parsed.origin().is_tuple() {
        Ok(parsed)
    } else {
        Err(ConfigError::Invalid(format!(
            "auth.public_base_url `{public_base_url}` has no origin of its own"
        )))
    }
}

#[cfg(test)]
mod tests;
