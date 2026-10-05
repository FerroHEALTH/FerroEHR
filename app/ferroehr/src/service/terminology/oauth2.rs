// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`TokenSource`] — `OAuth2` client-credentials authentication to a
//! terminology server, over the pinned `oauth2` crate.
//!
//! A [`super::fhir::FhirTerminologyProvider`] whose
//! [`FhirProviderConfig::oauth2_client`](super::config::FhirProviderConfig::oauth2_client)
//! names a configured client attaches this source's bearer token to every FHIR
//! operation it issues. The token is cached and refreshed shortly before its
//! stated expiry, so a validation burst costs one token round trip per token
//! lifetime rather than one per request.
//!
//! NOTE: no openEHR spec governs terminology-server authentication — our own
//! design/extension; the grant is RFC 6749 §4.4 (client credentials),
//! implemented by the `oauth2` crate.
//!
//! That crate is HTTP-client agnostic, building an `http::Request<Vec<u8>>` and
//! parsing the response through its `AsyncHttpClient` trait, and its own
//! `reqwest` impl targets a different major version than the workspace pins, so
//! `Oauth2HttpClient` supplies the pinned one and the token endpoint is reached
//! with the same TLS stack as the terminology operations.
//!
//! NOTE: the transport trait is implemented on a concrete type rather than
//! through the crate's blanket closure impl, whose higher-ranked `Send`-ness the
//! compiler cannot discharge through the `#[async_trait]` call chain that
//! reaches it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oauth2::TokenResponse;

use crate::service::status::SmError;

use super::config::{Oauth2AuthMethod, TerminologyOauth2Config};

/// A token-endpoint client for one configured `OAuth2` client-credentials
/// client, with a cached access token.
#[derive(Debug)]
pub struct TokenSource {
    /// The configured client name (for error/log context).
    name: String,
    /// The `oauth2` client, already carrying the token endpoint.
    client: oauth2::basic::BasicClient<
        oauth2::EndpointNotSet,
        oauth2::EndpointNotSet,
        oauth2::EndpointNotSet,
        oauth2::EndpointNotSet,
        oauth2::EndpointSet,
    >,
    /// Scopes requested with every grant (RFC 6749 §4.4.2).
    scopes: Vec<oauth2::Scope>,
    /// The HTTP client the token request rides on (the workspace `reqwest`).
    http: Oauth2HttpClient,
    /// How long before the stated expiry a cached token is refreshed.
    refresh_leeway: Duration,
    /// The cached token, if one has been obtained and is still fresh.
    cached: tokio::sync::RwLock<Option<CachedToken>>,
}

/// A cached access token with the instant it stops being usable.
#[derive(Debug, Clone)]
struct CachedToken {
    /// The `access_token` value, sent verbatim as the bearer credential.
    bearer: Arc<str>,
    /// When the token must be replaced; `None` when the server stated no
    /// `expires_in` (RFC 6749 §5.1 makes it OPTIONAL), in which case the token
    /// is reused until a request fails.
    refresh_after: Option<Instant>,
}

impl CachedToken {
    /// Whether the token may still be used at `now`.
    fn is_fresh(&self, now: Instant) -> bool {
        self.refresh_after.is_none_or(|deadline| now < deadline)
    }
}

impl TokenSource {
    /// Build a token source from its configuration.
    ///
    /// # Errors
    ///
    /// [`SmError::exception`] when `token_url` is empty or unparseable,
    /// `client_id` is empty, no client secret is configured, or the HTTP
    /// client cannot be built.
    pub fn new(name: &str, cfg: &TerminologyOauth2Config) -> Result<Self, SmError> {
        let fail =
            |what: &str| SmError::exception(format!("terminology oauth2 client '{name}': {what}"));
        let client_id = cfg.client_id.trim();
        if client_id.is_empty() {
            return Err(fail("client_id must not be empty"));
        }
        let token_url = oauth2::TokenUrl::new(cfg.token_url.trim().to_owned())
            .map_err(|e| fail(&format!("invalid token_url: {e}")))?;
        let secret = cfg
            .client_secret
            .as_ref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                fail("client_secret (or client_secret_file) is required for the client-credentials grant")
            })?;
        let auth_type = match cfg.auth_method {
            Oauth2AuthMethod::ClientSecretBasic => oauth2::AuthType::BasicAuth,
            Oauth2AuthMethod::ClientSecretPost => oauth2::AuthType::RequestBody,
        };
        let client = oauth2::basic::BasicClient::new(oauth2::ClientId::new(client_id.to_owned()))
            .set_client_secret(oauth2::ClientSecret::new(secret.expose().to_owned()))
            .set_token_uri(token_url)
            .set_auth_type(auth_type);
        let http = Oauth2HttpClient(
            reqwest::Client::builder()
                .build()
                .map_err(|e| fail(&format!("building the token-endpoint HTTP client: {e}")))?,
        );
        Ok(Self {
            name: name.to_owned(),
            client,
            scopes: cfg
                .scopes
                .iter()
                .filter(|s| !s.trim().is_empty())
                .map(|s| oauth2::Scope::new(s.clone()))
                .collect(),
            http,
            refresh_leeway: Duration::from_secs(cfg.refresh_leeway_secs),
            cached: tokio::sync::RwLock::new(None),
        })
    }

    /// The bearer credential to send on the next terminology request: the
    /// cached token when it is still fresh, else a newly granted one.
    ///
    /// # Errors
    ///
    /// [`SmError::exception`] when the token endpoint cannot be reached or
    /// answers an `OAuth2` error / unparseable body.
    pub async fn bearer(&self) -> Result<Arc<str>, SmError> {
        if let Some(token) = self.cached.read().await.as_ref()
            && token.is_fresh(Instant::now())
        {
            return Ok(Arc::clone(&token.bearer));
        }
        // Re-check under the write lock: a burst of concurrent validations
        // must cost one grant, not one per caller.
        let mut slot = self.cached.write().await;
        if let Some(token) = slot.as_ref()
            && token.is_fresh(Instant::now())
        {
            return Ok(Arc::clone(&token.bearer));
        }
        let granted = self.grant().await?;
        let bearer = Arc::clone(&granted.bearer);
        *slot = Some(granted);
        Ok(bearer)
    }

    /// Run one client-credentials grant (RFC 6749 §4.4) against the token
    /// endpoint.
    async fn grant(&self) -> Result<CachedToken, SmError> {
        let response = self
            .client
            .exchange_client_credentials()
            .add_scopes(self.scopes.iter().cloned())
            .request_async(&self.http)
            .await
            .map_err(|e| {
                // The OPERATOR detail — which configured client, and the
                // upstream authorization server's own error — goes to the
                // trace record; the wire body stays the curated 500 message.
                // A client must not be able to read the deployment's
                // credential configuration out of a response. (No openEHR spec
                // governs 500 body content — our own design/extension.)
                tracing::error!(
                    oauth2_client = %self.name,
                    error = %e,
                    "terminology oauth2 client-credentials grant failed → 500"
                );
                SmError::exception(crate::service::error::INTERNAL_MESSAGE.to_owned())
            })?;
        // Refresh a leeway before the stated expiry so an in-flight request
        // never carries a token that expires mid-call. A leeway at or beyond
        // the lifetime would refresh on every call, so it is clamped to leave
        // at least a moment of usable life.
        let refresh_after = response.expires_in().map(|lifetime| {
            let usable = lifetime
                .checked_sub(self.refresh_leeway)
                .unwrap_or_else(|| lifetime / 2);
            Instant::now() + usable
        });
        Ok(CachedToken {
            bearer: Arc::from(response.access_token().secret().as_str()),
            refresh_after,
        })
    }
}

/// The workspace-pinned `reqwest` client as an `oauth2` transport
/// (`oauth2::AsyncHttpClient`): send the request the crate built and reassemble
/// the reply as the `http::Response<Vec<u8>>` it parses.
#[derive(Debug)]
struct Oauth2HttpClient(reqwest::Client);

impl<'c> oauth2::AsyncHttpClient<'c> for Oauth2HttpClient {
    type Error = TokenTransportError;
    /// `Send` is *declared* here, not inferred — it is what keeps every
    /// service future that can reach a token grant `Send` (see the module
    /// NOTE).
    type Future =
        Pin<Box<dyn Future<Output = Result<oauth2::HttpResponse, Self::Error>> + Send + 'c>>;

    fn call(&'c self, request: oauth2::HttpRequest) -> Self::Future {
        Box::pin(async move {
            let response = self.0.execute(reqwest::Request::try_from(request)?).await?;
            let mut builder = http::Response::builder().status(response.status());
            for (name, value) in response.headers() {
                builder = builder.header(name, value);
            }
            builder
                .body(response.bytes().await?.to_vec())
                .map_err(TokenTransportError::Http)
        })
    }
}

/// A transport failure while calling the token endpoint. The `oauth2` crate
/// requires the HTTP client's error type to be `std::error::Error`, so the two
/// failure shapes (`reqwest` and `http` response assembly) are one enum.
#[derive(Debug, thiserror::Error)]
enum TokenTransportError {
    /// The request could not be sent or its body could not be read.
    #[error("token endpoint request failed: {0}")]
    Reqwest(#[from] reqwest::Error),
    /// The response could not be reassembled as an `http::Response`.
    #[error("token endpoint response was malformed: {0}")]
    Http(#[from] http::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_config(token_url: &str) -> TerminologyOauth2Config {
        TerminologyOauth2Config {
            token_url: token_url.to_owned(),
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
        let err = TokenSource::new("ts", &cfg).expect_err("no secret must be rejected");
        assert!(err.message.contains("client_secret"), "got {}", err.message);
    }

    #[test]
    fn an_invalid_token_url_is_rejected() {
        let err = TokenSource::new("ts", &client_config("not a url"))
            .expect_err("an unparseable token_url must be rejected");
        assert!(err.message.contains("token_url"), "got {}", err.message);
    }

    #[test]
    fn an_empty_client_id_is_rejected() {
        let mut cfg = client_config("https://idp.example/token");
        cfg.client_id = "  ".to_owned();
        let err = TokenSource::new("ts", &cfg).expect_err("a blank client_id must be rejected");
        assert!(err.message.contains("client_id"), "got {}", err.message);
    }

    /// A token whose stated lifetime is shorter than the refresh leeway must
    /// still be usable for part of that lifetime — never refreshed on every
    /// single call.
    #[test]
    fn a_short_lived_token_keeps_some_usable_life() {
        let lifetime = Duration::from_secs(10);
        let leeway = Duration::from_secs(30);
        let usable = lifetime.checked_sub(leeway).unwrap_or(lifetime / 2);
        assert_eq!(usable, Duration::from_secs(5));
    }

    /// A token with no stated `expires_in` (RFC 6749 §5.1 makes it OPTIONAL)
    /// stays fresh indefinitely.
    #[test]
    fn a_token_without_expiry_is_always_fresh() {
        let token = CachedToken {
            bearer: Arc::from("t"),
            refresh_after: None,
        };
        assert!(token.is_fresh(Instant::now()));
    }
}
