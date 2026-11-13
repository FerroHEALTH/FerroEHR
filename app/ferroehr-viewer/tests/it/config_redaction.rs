// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's configuration renders no credential a URL carries
//! (#5322). No openEHR spec governs configuration: our own design.

use ferroehr_viewer::config::ViewerConfig;
use ferrohealth_secret::secret_url::SecretUrl;

/// Userinfo in an OpenID Provider URL reaches neither the serialized `[oidc]`
/// table nor the configuration's `Debug`: `ferrohealth-session` 0.13 types
/// the issuer, each endpoint and both redirect URIs as a `SecretUrl` (#5394),
/// and the viewer adds no mask of its own.
#[test]
fn oidc_endpoint_userinfo_reaches_no_rendering() {
    let leaky = |key: &str| {
        Some(SecretUrl::new(format!(
            "https://u:OIDC_PW_SENTINEL@idp.example/{key}"
        )))
    };
    let mut cfg = ViewerConfig::default();
    cfg.oidc.authorization_endpoint = leaky("authorize");
    cfg.oidc.token_endpoint = leaky("token");
    cfg.oidc.jwks_uri = leaky("jwks");
    cfg.oidc.end_session_endpoint = leaky("logout");
    cfg.oidc.revocation_endpoint = leaky("revoke");
    cfg.oidc.issuer = SecretUrl::new("https://u:OIDC_PW_SENTINEL@idp.example/realm");
    cfg.oidc.redirect_uri = SecretUrl::new("https://u:OIDC_PW_SENTINEL@idp.example/callback");
    cfg.oidc.post_logout_redirect_uri = leaky("signed-out");
    let rendered = toml::Table::try_from(&cfg.oidc)
        .expect("the [oidc] table renders")
        .to_string();
    let debug = format!("{cfg:?}");
    for (surface, text) in [("toml", &rendered), ("debug", &debug)] {
        assert!(!text.contains("OIDC_PW_SENTINEL"), "{surface}: {text}");
        for key in [
            "authorize",
            "token",
            "jwks",
            "logout",
            "revoke",
            "realm",
            "callback",
            "signed-out",
        ] {
            assert!(
                text.contains(&format!("https://***@idp.example/{key}")),
                "{surface} lost the {key} endpoint: {text}"
            );
        }
    }
}

/// `ferroehr-viewer config check` reads the configuration through the same
/// load, so a refused `[oidc]` table names its key and never the password
/// its URLs carry (#5394).
#[test]
fn a_config_check_refusal_renders_no_oidc_password() {
    let env = std::collections::BTreeMap::from([
        (
            "FERROEHR_VIEWER__AUTH__OIDC_ENABLED".to_owned(),
            "true".to_owned(),
        ),
        (
            "FERROEHR_VIEWER__OIDC__CLIENT_ID".to_owned(),
            "ferroehr-viewer".to_owned(),
        ),
        (
            "FERROEHR_VIEWER__OIDC__ISSUER".to_owned(),
            "http://u:OIDC_PW_SENTINEL@keycloak:8080/realms/ferroehr".to_owned(),
        ),
        (
            "FERROEHR_VIEWER__OIDC__REDIRECT_URI".to_owned(),
            "https://u:OIDC_PW_SENTINEL@viewer.example/auth/callback".to_owned(),
        ),
    ]);
    let refused = ferroehr_viewer::config::load_sources(None, &env)
        .expect_err("an http issuer is refused")
        .to_string();
    assert!(refused.contains("oidc.issuer"), "{refused}");
    assert!(!refused.contains("OIDC_PW_SENTINEL"), "{refused}");
}
