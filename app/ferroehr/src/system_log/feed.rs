// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-20 ATX: FHIR Feed client (`[audit.fhir_feed]`): `ihe-atna`'s
//! [`FeedRepository`], which asks the
//! feed's IHE IUA bearer token for each request and renews it once on a `401`
//! (RFC 6750 §3.1).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the feed carries an \
              already-rendered FHIR document, never a typed resource"
)]

use std::sync::Arc;
use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};

use ihe_atna::atna::feed::{Credentials, CredentialsError, FeedError, FeedRepository};

use crate::system_log::AuditError;
use crate::system_log::config::FhirFeedConfig;

/// The ITI-20 ATX:FHIR Feed, posted through `ihe-atna`'s
/// [`FeedRepository`].
#[derive(Clone)]
pub(super) struct FeedClient {
    repository: FeedRepository,
    max_retries: usize,
}

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
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AuditError::Transport(Box::new(e)))?;
        // NOTE: an `http` feed opens the `plaintext_audit_feed` deployment gap, which
        // production refuses unless it is accepted by name.
        let repository = if base.scheme() == "http" {
            FeedRepository::cleartext(base, http, FEED_TIMEOUT)
        } else {
            FeedRepository::new(base, http, FEED_TIMEOUT)
        }
        .map_err(|e| AuditError::Transport(Box::new(e)))?;
        let repository = match token {
            Some(token) => repository.with_credentials(Arc::new(Bearer(token))),
            None => repository,
        };
        Ok(FeedClient {
            repository,
            max_retries: config.max_retries,
        })
    }

    /// One `POST` of `body`; `ihe-atna` sends it under the feed's bearer
    /// token when it has one, once more under a new token on a `401` (RFC
    /// 6750 §3.1).
    async fn send_once(&self, body: &secrecy::SecretSlice<u8>) -> Result<(), AuditError> {
        match self.repository.send(body).await {
            Ok(()) => Ok(()),
            Err(FeedError::Rejected { status } | FeedError::Unavailable { status }) => {
                Err(AuditError::FeedRejected(status))
            }
            Err(FeedError::Unauthorized) => {
                Err(AuditError::FeedRejected(http::StatusCode::UNAUTHORIZED))
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

/// The feed's IHE IUA bearer token, as `ihe-atna` asks for the value of each
/// request and tells of a refused one.
struct Bearer(ferrohealth_smart::client::cache::TokenSource);

#[async_trait::async_trait]
impl Credentials for Bearer {
    async fn authorization(&self) -> Result<secrecy::SecretString, CredentialsError> {
        let token = self
            .0
            .token()
            .await
            .map_err(|e| CredentialsError(Box::new(SharedError(e))))?;
        let value = token
            .header()
            .to_str()
            .map_err(|e| CredentialsError(Box::new(e)))?;
        Ok(secrecy::SecretString::from(value.to_owned()))
    }

    async fn refused(&self, refused: &secrecy::SecretString) {
        use secrecy::ExposeSecret as _;
        // NOTE: a value this source gave is a header value, so a parse failure names none it
        // holds and there is nothing to invalidate.
        if let Ok(stale) = http::HeaderValue::from_str(refused.expose_secret()) {
            self.0.invalidate(&stale);
        }
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
