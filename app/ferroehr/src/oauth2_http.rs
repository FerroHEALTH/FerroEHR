// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`Oauth2HttpClient`]: the workspace-pinned `reqwest` client as the transport
//! of the `oauth2` and `openidconnect` crates.
//!
//! Both crates are HTTP-client agnostic: they build an `http::Request<Vec<u8>>`
//! and parse an `http::Response<Vec<u8>>` through `oauth2::AsyncHttpClient`,
//! which `openidconnect` re-exports as its own transport trait. Their bundled
//! `reqwest` integrations target an older major version whose TLS trusts only
//! the bundled web-PKI roots, so this adapter sends their requests over the
//! workspace `reqwest` and the platform trust store instead. The terminology
//! token source and the OIDC key discovery of the REST adapter share it.
//!
//! NOTE: no openEHR spec governs the transport — our own design; the trait is
//! implemented on a concrete type because the crate's blanket closure impl has
//! a higher-ranked `Send` bound the compiler cannot discharge across
//! `#[async_trait]` call chains.

use std::future::Future;
use std::pin::Pin;

/// The workspace `reqwest` client as an `oauth2::AsyncHttpClient` (and so as an
/// `openidconnect::AsyncHttpClient`, the same trait).
///
/// It sends the request the crate built and reassembles the reply as the
/// `http::Response<Vec<u8>>` the crate parses. Redirect, timeout and TLS policy
/// are the wrapped client's own, so each caller builds the client it needs.
#[derive(Debug, Clone)]
pub struct Oauth2HttpClient(reqwest::Client);

impl Oauth2HttpClient {
    /// Wraps `client` as the transport.
    #[must_use]
    pub fn new(client: reqwest::Client) -> Self {
        Self(client)
    }

    /// The wrapped client, for requests the `oauth2`/`openidconnect` crates do
    /// not issue themselves (a JWKS fetch from a discovered URI).
    #[must_use]
    pub fn client(&self) -> &reqwest::Client {
        &self.0
    }
}

impl<'c> oauth2::AsyncHttpClient<'c> for Oauth2HttpClient {
    type Error = Oauth2TransportError;
    /// `Send` is declared here, not inferred: it keeps every future that can
    /// reach a token grant or an issuer discovery `Send` (see the module NOTE).
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
                .map_err(Oauth2TransportError::Http)
        })
    }
}

/// A transport failure of [`Oauth2HttpClient`].
///
/// The `oauth2` crate requires the transport error to be a `std::error::Error`,
/// so the two failure shapes are one enum. The display text names neither the
/// endpoint nor the underlying error, whose own text carries the request URL:
/// that detail is reachable only through [`std::error::Error::source`], for the
/// log, never for a response body.
#[derive(Debug, thiserror::Error)]
pub enum Oauth2TransportError {
    /// The request could not be sent or its body could not be read.
    #[error("the HTTP request failed")]
    Reqwest(#[from] reqwest::Error),
    /// The reply could not be reassembled as an `http::Response`.
    #[error("the HTTP response was malformed")]
    Http(#[from] http::Error),
}

#[cfg(test)]
mod tests {
    use oauth2::AsyncHttpClient;

    use super::*;

    /// A refused connection surfaces as the `reqwest` variant, whose display
    /// text does not carry the URL its source names.
    #[tokio::test]
    async fn a_transport_failure_keeps_the_url_out_of_its_display() {
        let transport = Oauth2HttpClient::new(reqwest::Client::new());
        // Port 1 refuses the connection.
        let request = http::Request::builder()
            .uri("http://127.0.0.1:1/token")
            .body(Vec::new())
            .expect("request");
        let err = transport.call(request).await.expect_err("refused");
        assert!(matches!(err, Oauth2TransportError::Reqwest(_)), "{err:?}");
        assert!(!err.to_string().contains("127.0.0.1"), "{err}");
        let cause = std::error::Error::source(&err)
            .and_then(|cause| cause.downcast_ref::<reqwest::Error>())
            .expect("the reqwest error is the source");
        assert!(cause.url().is_some(), "{cause:?}");
    }
}
