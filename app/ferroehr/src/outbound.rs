// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One outbound integration built from its `[<x>.endpoint]` table, the
//! family's `EndpointSection`, in one place for every integration.
//!
//! [`Endpoint::connect`] checks the table, builds the HTTP client (its
//! timeouts, its TLS through `ferrohealth-server`'s `TlsClient`, no redirect,
//! and the HTTP Basic credential as a sensitive default header) and resolves
//! the client credentials grant into a [`TokenSource`] each integration hands
//! its own client. [`Endpoint::retrying`] applies the table's `retry` to one
//! call. No openEHR spec governs an outbound client: our own design.

use std::future::Future;
use std::time::Duration;

use ferrohealth_server::tls_client::{TlsClient, TlsClientError};
use ferrohealth_smart::client::cache::TokenSource;
use ferrohealth_smart::config::credentials::Outbound;
use ferrohealth_smart::config::endpoint::{EndpointSection, RetrySection};
use url::Url;

/// An outbound integration's table could not be turned into a client.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EndpointError {
    /// The table breaks a rule of the endpoint section.
    #[error("{key} is refused")]
    Section {
        /// The dotted key of the table.
        key: &'static str,
        /// The section's refusal.
        #[source]
        source: Box<ferrohealth_smart::config::ConfigError>,
    },
    /// The table's TLS material could not be read.
    #[error("{key}.tls is refused")]
    Tls {
        /// The dotted key of the table.
        key: &'static str,
        /// The reader's refusal.
        #[source]
        source: Box<TlsClientError>,
    },
    /// The HTTP client could not be built.
    #[error("the HTTP client of {key} could not be built")]
    Client {
        /// The dotted key of the table.
        key: &'static str,
        /// reqwest's refusal.
        #[source]
        source: reqwest::Error,
    },
    /// The client credentials grant could not start its token source.
    #[error("the client credentials grant of {key} could not start")]
    Grant {
        /// The dotted key of the table.
        key: &'static str,
        /// The token source's refusal.
        #[source]
        source: Box<ferrohealth_smart::client::TokenError>,
    },
    /// The table names an onward token exchange, which no outbound
    /// integration of FerroEHR performs.
    #[error("{key}.credentials.onward is not read by this integration")]
    Onward {
        /// The dotted key of the table.
        key: &'static str,
    },
}

/// One outbound integration, ready to call.
#[derive(Debug, Clone)]
pub struct Endpoint {
    url: Url,
    http: reqwest::Client,
    token: Option<TokenSource>,
    timeout: Duration,
    retry: RetrySection,
}

impl Endpoint {
    /// Builds the integration the table at the dotted `key` describes.
    ///
    /// # Errors
    /// [`EndpointError`] naming `key` when the section, its TLS, its
    /// credential or the HTTP client is refused.
    pub fn connect(key: &'static str, section: &EndpointSection) -> Result<Self, EndpointError> {
        let url = section
            .validate()
            .map_err(|source| EndpointError::Section {
                key,
                source: Box::new(source),
            })?;
        if section.credentials.onward.is_some() {
            return Err(EndpointError::Onward { key });
        }
        let credential = section
            .credentials
            .resolve_with_tls(&ferrohealth_smart::config::ServerTls)
            .map_err(|source| EndpointError::Section {
                key,
                source: Box::new(source),
            })?;
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(section.timeout.get());
        if let Some(connect) = section.connect_timeout {
            builder = builder.connect_timeout(connect.get());
        }
        let mut token = None;
        match credential {
            Some(Outbound::Basic(basic)) => {
                let mut value = basic.header_value();
                value.set_sensitive(true);
                builder = builder.default_headers(http::HeaderMap::from_iter([(
                    http::header::AUTHORIZATION,
                    value,
                )]));
            }
            Some(Outbound::Oauth(grant)) => {
                token = Some(
                    TokenSource::new(*grant).map_err(|source| EndpointError::Grant {
                        key,
                        source: Box::new(source),
                    })?,
                );
            }
            None => {}
        }
        let http = TlsClient::read(&format!("{key}.tls"), &section.tls)
            .and_then(|tls| tls.apply(builder))
            .map_err(|source| EndpointError::Tls {
                key,
                source: Box::new(source),
            })?
            .build()
            .map_err(|source| EndpointError::Client { key, source })?;
        Ok(Self {
            url,
            http,
            token,
            timeout: section.timeout.get(),
            retry: section.retry,
        })
    }

    /// The service's URL.
    #[must_use]
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// The HTTP client: the timeouts, the TLS and a Basic credential.
    #[must_use]
    pub fn http(&self) -> reqwest::Client {
        self.http.clone()
    }

    /// The token source of the client credentials grant, when one is set.
    #[must_use]
    pub fn token(&self) -> Option<TokenSource> {
        self.token.clone()
    }

    /// How long one whole call may take.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The table's `retry`.
    #[must_use]
    pub const fn retry(&self) -> RetrySection {
        self.retry
    }

    /// Calls `call` until it succeeds, it fails with an error `transient`
    /// does not hold to be passing, or `retry.max_attempts` calls were made,
    /// waiting `retry.initial_backoff` before the second call and doubling
    /// the wait up to `retry.max_backoff`.
    ///
    /// # Errors
    /// The last call's error.
    pub async fn retrying<T, E, F, Fut>(
        &self,
        transient: impl Fn(&E) -> bool,
        mut call: F,
    ) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let mut wait = self.retry.initial_backoff.get();
        let ceiling = self.retry.max_backoff.get();
        let mut attempt = 1;
        loop {
            match call().await {
                Err(error) if attempt < self.retry.max_attempts && transient(&error) => {
                    tokio::time::sleep(wait).await;
                    wait = wait.saturating_mul(2).min(ceiling);
                    attempt += 1;
                }
                outcome => return outcome,
            }
        }
    }
}

/// Every rule the table at the dotted `key` breaks, as boot refusals.
///
/// The rules: the section's own, an `http` or `https` URL, TLS material that
/// reads, no onward exchange, one credential at most, a grant the client
/// credentials rules admit, and no credential in the URL beside one in
/// `credentials`.
#[must_use]
pub fn problems(key: &str, section: &EndpointSection) -> Vec<String> {
    let mut problems = Vec::new();
    match section.validate() {
        Ok(url) if matches!(url.scheme(), "http" | "https") => {
            let userinfo = !url.username().is_empty() || url.password().is_some();
            let credential =
                !section.credentials.user.is_empty() || section.credentials.oauth.is_some();
            if userinfo && credential {
                problems.push(format!(
                    "{key}.url carries credentials and so does [{key}.credentials]; \
                     authenticate one way: remove the userinfo or the credential"
                ));
            }
        }
        Ok(_) => problems.push(format!("{key}.url must be an http or https URL")),
        Err(error) => problems.push(format!("{key}: {error}")),
    }
    if let Err(error) = TlsClient::read(&format!("{key}.tls"), &section.tls) {
        problems.push(error.to_string());
    }
    let credentials = &section.credentials;
    if credentials.onward.is_some() {
        problems.push(format!(
            "{key}.credentials.onward is not read by this integration: it exchanges no caller \
             token"
        ));
    }
    if !credentials.user.is_empty() && credentials.oauth.is_some() {
        problems.push(format!(
            "{key}.credentials sets user and oauth; authenticate one way"
        ));
    }
    if let Some(oauth) = &credentials.oauth {
        problems.extend(oauth.problems(&format!("{key}.credentials.oauth")));
    }
    problems
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use ferrohealth_config::units::Duration as ConfigDuration;

    use super::*;

    fn endpoint(max_attempts: u32) -> Endpoint {
        let section = EndpointSection {
            url: "http://127.0.0.1:1/".to_owned(),
            retry: RetrySection {
                max_attempts,
                initial_backoff: ConfigDuration::from_millis(1),
                max_backoff: ConfigDuration::from_millis(4),
            },
            ..EndpointSection::default()
        };
        Endpoint::connect("test.endpoint", &section).expect("a plain endpoint")
    }

    /// A transient failure is tried again up to `max_attempts` calls, a
    /// success ends the calls, and a failure that is not transient is never
    /// tried again.
    #[tokio::test]
    async fn retry_follows_the_section() {
        let calls = AtomicU32::new(0);
        let failing: Result<(), &str> = endpoint(3)
            .retrying(
                |_| true,
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err("down")
                },
            )
            .await;
        assert_eq!(failing, Err("down"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        calls.store(0, Ordering::SeqCst);
        let second = endpoint(3)
            .retrying(
                |_| true,
                || async {
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        Err("down")
                    } else {
                        Ok("up")
                    }
                },
            )
            .await;
        assert_eq!(second, Ok("up"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        calls.store(0, Ordering::SeqCst);
        let refused: Result<(), &str> = endpoint(3)
            .retrying(
                |_| false,
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err("refused")
                },
            )
            .await;
        assert_eq!(refused, Err("refused"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// A table with no URL, or with an onward exchange, is refused naming
    /// the table.
    #[test]
    fn a_table_without_url_or_with_onward_is_refused() {
        let err =
            Endpoint::connect("pixm.endpoint", &EndpointSection::default()).expect_err("no url");
        assert!(err.to_string().contains("pixm.endpoint"), "{err}");
    }
}
