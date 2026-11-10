// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's configuration renders no credential a URL carries
//! (#5322). No openEHR spec governs configuration: our own design.

use ferroehr_viewer::config::ViewerConfig;
use ferrohealth_secret::secret_url::SecretUrl;

/// Userinfo in an OpenID Provider endpoint reaches neither the serialized
/// `[oidc]` table nor the configuration's `Debug`: `ferrohealth-session`
/// types each endpoint as a `SecretUrl`, and the viewer adds no mask of its
/// own.
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
    let rendered = toml::Table::try_from(&cfg.oidc)
        .expect("the [oidc] table renders")
        .to_string();
    let debug = format!("{cfg:?}");
    for (surface, text) in [("toml", &rendered), ("debug", &debug)] {
        assert!(!text.contains("OIDC_PW_SENTINEL"), "{surface}: {text}");
        for key in ["authorize", "token", "jwks", "logout", "revoke"] {
            assert!(
                text.contains(&format!("https://***@idp.example/{key}")),
                "{surface} lost the {key} endpoint: {text}"
            );
        }
    }
}
