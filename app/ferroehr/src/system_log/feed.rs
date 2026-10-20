// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-20 ATX: FHIR Feed client (`[audit.fhir_feed]`): `ihe-atna`'s
//! [`FeedRepository`](ihe_atna::atna::feed::FeedRepository) under the feed's
//! IHE IUA bearer token, renewed once on a `401` (RFC 6750 §3.1).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the feed carries an \
              already-rendered FHIR document, never a typed resource"
)]

use std::sync::Arc;
use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};

use crate::system_log::AuditError;
use crate::system_log::config::FhirFeedConfig;

/// The ITI-20 ATX:FHIR Feed, posted through `ihe-atna`'s
/// [`FeedRepository`](ihe_atna::atna::feed::FeedRepository).
#[derive(Clone)]
pub(super) struct FeedClient {
    base: url::Url,
    cleartext: bool,
    max_retries: usize,
    /// The bearer token of `[audit.fhir_feed.oauth]`, the IHE IUA authorized
    /// client; `None` sends the Basic credentials of the URL, if any.
    token: Option<ferrohealth_smart::client::cache::TokenSource>,
    /// The repository under the token it was built with, so a new client is
    /// built only when the token is renewed.
    held: Arc<std::sync::Mutex<Option<HeldRoute>>>,
}

type FeedRoute = ihe_atna::atna::feed::FeedRepository;

/// The repository route and the bearer token it was built under.
type HeldRoute = (Option<http::HeaderValue>, FeedRoute);

/// How long one `POST` may take.
///
/// No specification governs the bound: our own design.
const FEED_TIMEOUT: Duration = Duration::from_secs(30);

impl FeedClient {
    /// The feed client for `config`.
    ///
    /// # Errors
    /// [`AuditError::Transport`] when the URL is not an `http(s)` URL, the
    /// configured grant is incomplete or its HTTP engine cannot be built; boot
    /// validation refuses an incomplete grant first.
    pub(super) fn new(config: &FhirFeedConfig) -> Result<Self, AuditError> {
        let token = config
            .oauth
            .as_ref()
            .map(|oauth| {
                let grant = oauth
                    .grant("audit.fhir_feed.oauth")
                    .map_err(|e| AuditError::Transport(e.into()))?;
                ferrohealth_smart::client::cache::TokenSource::new(grant)
                    .map_err(|e| AuditError::Transport(Box::new(e)))
            })
            .transpose()?;
        let base =
            url::Url::parse(config.url.expose()).map_err(|e| AuditError::Transport(Box::new(e)))?;
        // NOTE: an `http` feed opens the `plaintext_audit_feed` deployment gap, which
        // production refuses unless it is accepted by name.
        let cleartext = base.scheme() == "http";
        Ok(FeedClient {
            base,
            cleartext,
            max_retries: config.max_retries,
            token,
            held: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// The repository route under `bearer`, built once per token.
    fn route(&self, bearer: Option<&http::HeaderValue>) -> Result<FeedRoute, AuditError> {
        let mut held = self
            .held
            .lock()
            .map_err(|poisoned| AuditError::Transport(poisoned.to_string().into()))?;
        if let Some((token, route)) = held.as_ref()
            && token.as_ref() == bearer
        {
            return Ok(route.clone());
        }
        let mut headers = http::HeaderMap::new();
        if let Some(bearer) = bearer {
            headers.insert(http::header::AUTHORIZATION, bearer.clone());
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AuditError::Transport(Box::new(e)))?;
        let route = if self.cleartext {
            FeedRoute::cleartext_for_development(self.base.clone(), http, FEED_TIMEOUT)
        } else {
            FeedRoute::new(self.base.clone(), http, FEED_TIMEOUT)
        }
        .map_err(|e| AuditError::Transport(Box::new(e)))?;
        *held = Some((bearer.cloned(), route.clone()));
        Ok(route)
    }

    /// One `POST` of `body`, under the feed's bearer token when it has one,
    /// sent once more under a new token when the ARR refuses the held one
    /// with `401` (RFC 6750 §3.1).
    async fn send_once(&self, body: &secrecy::SecretSlice<u8>) -> Result<(), AuditError> {
        let Some(token) = &self.token else {
            return self.post_under(None, body).await;
        };
        let first = token
            .token()
            .await
            .map_err(|e| AuditError::Transport(Box::new(SharedError(e))))?;
        match self.post_under(Some(first.header()), body).await {
            Err(AuditError::FeedRejected(status)) if status == http::StatusCode::UNAUTHORIZED => {
                token.invalidate(first.header());
                let renewed = token
                    .token()
                    .await
                    .map_err(|e| AuditError::Transport(Box::new(SharedError(e))))?;
                self.post_under(Some(renewed.header()), body).await
            }
            other => other,
        }
    }

    /// One `POST` of `body` under `bearer`, a refused or unavailable answer
    /// carrying its status.
    async fn post_under(
        &self,
        bearer: Option<&http::HeaderValue>,
        body: &secrecy::SecretSlice<u8>,
    ) -> Result<(), AuditError> {
        use ihe_atna::atna::feed::FeedError;
        match self.route(bearer)?.send(body).await {
            Ok(()) => Ok(()),
            Err(FeedError::Rejected { status } | FeedError::Unavailable { status }) => {
                Err(AuditError::FeedRejected(status))
            }
            Err(e) => Err(AuditError::Transport(Box::new(e))),
        }
    }

    /// POST one FHIR `AuditEvent` (ITI-20 ATX:FHIR Feed) with bounded jittered
    /// retries.
    pub(super) async fn post(&self, body: &serde_json::Value) -> Result<(), AuditError> {
        let bytes = serde_json::to_vec(body).map_err(|e| AuditError::Render(Box::new(e)))?;
        let body = secrecy::SecretSlice::from(bytes);
        let send = || async { self.send_once(&body).await };
        send.retry(
            ExponentialBuilder::default()
                .with_jitter()
                .with_max_times(self.max_retries),
        )
        .await
    }
}

/// A token error shared between the callers that waited on one fetch.
#[derive(Debug)]
struct SharedError(Arc<ferrohealth_smart::client::TokenError>);

impl std::fmt::Display for SharedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for SharedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
