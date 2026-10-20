// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OAuth 2.0 client credentials grant (RFC 6749 §4.4) a terminology
//! provider authenticates with, on the `ferrohealth-smart` client.
//!
//! A [`super::fhir::FhirTerminologyProvider`] whose
//! [`FhirProviderConfig::oauth2_client`](super::config::FhirProviderConfig::oauth2_client)
//! names a configured client sends that client's bearer token with every FHIR
//! operation. The crate's [`TokenSource`] caches the token, renews it
//! `refresh_leeway_secs` before its stated expiry (the margin capped at half
//! the lifetime), shares one fetch between concurrent callers, and sends a
//! call once more under a new token when the server answers `401`.
//!
//! NOTE: no openEHR spec governs terminology-server authentication — our own
//! design/extension; the grant is RFC 6749 §4.4.

use std::time::Duration;

use ferrohealth_smart::client::cache::TokenSource;
use ferrohealth_smart::client::{ClientAuth, ClientCredentials, Endpoint};
use secrecy::SecretString;

use crate::service::status::SmError;

use super::config::{Oauth2AuthMethod, TerminologyOauth2Config};

/// Returns the token source of the client `name` configures.
///
/// # Errors
/// [`SmError`] naming the client when its `client_id` is blank, its
/// `token_url` is no URL, its secret is missing, or the crate refuses the
/// grant.
pub fn token_source(name: &str, cfg: &TerminologyOauth2Config) -> Result<TokenSource, SmError> {
    let fail =
        |what: &str| SmError::exception(format!("terminology oauth2 client '{name}': {what}"));
    let client_id = cfg.client_id.trim();
    if client_id.is_empty() {
        return Err(fail("client_id must not be empty"));
    }
    let token_url = url::Url::parse(cfg.token_url.expose().trim())
        .map_err(|e| fail(&format!("invalid token_url: {e}")))?;
    let secret = cfg
        .client_secret
        .as_ref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            fail("client_secret (or client_secret_file) is required for the client-credentials grant")
        })?;
    let mut grant = ClientCredentials::new(
        Endpoint::Token(token_url),
        client_id.to_owned(),
        SecretString::from(secret.expose().to_owned()),
    );
    grant.client_auth = match cfg.auth_method {
        Oauth2AuthMethod::ClientSecretBasic => ClientAuth::ClientSecretBasic,
        Oauth2AuthMethod::ClientSecretPost => ClientAuth::ClientSecretPost,
    };
    let scopes: Vec<&str> = cfg
        .scopes
        .iter()
        .map(|scope| scope.trim())
        .filter(|scope| !scope.is_empty())
        .collect();
    grant.scope = (!scopes.is_empty()).then(|| scopes.join(" "));
    grant.refresh_margin = Duration::from_secs(cfg.refresh_leeway_secs);
    TokenSource::new(grant).map_err(|e| {
        fail(&format!(
            "the grant is refused: {}",
            crate::service::error::ErrorChain::new(&e)
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_config(token_url: &str) -> TerminologyOauth2Config {
        TerminologyOauth2Config {
            token_url: crate::config::secret::SecretUrl::new(token_url),
            client_id: "cdr".to_owned(),
            client_secret: Some(crate::config::secret::Secret::new("s3cret")),
            client_secret_file: None,
            scopes: vec!["system/*.read".to_owned()],
            refresh_leeway_secs: 30,
            auth_method: Oauth2AuthMethod::ClientSecretBasic,
        }
    }

    #[test]
    fn a_missing_secret_is_rejected() {
        let mut cfg = client_config("https://idp.example/token");
        cfg.client_secret = None;
        let err = token_source("ts", &cfg).expect_err("no secret must be rejected");
        assert!(err.message.contains("client_secret"), "got {}", err.message);
    }

    #[test]
    fn an_invalid_token_url_is_rejected() {
        let err = token_source("ts", &client_config("not a url"))
            .expect_err("an unparseable token_url must be rejected");
        assert!(err.message.contains("token_url"), "got {}", err.message);
    }

    #[test]
    fn an_empty_client_id_is_rejected() {
        let mut cfg = client_config("https://idp.example/token");
        cfg.client_id = "  ".to_owned();
        let err = token_source("ts", &cfg).expect_err("a blank client_id must be rejected");
        assert!(err.message.contains("client_id"), "got {}", err.message);
    }

    #[test]
    fn a_valid_client_builds_its_source() {
        assert!(token_source("ts", &client_config("https://idp.example/token")).is_ok());
    }
}
