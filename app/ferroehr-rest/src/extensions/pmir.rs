// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-93 feed route: `POST {public root}/pmir/feed`, where the PMIR
//! Patient Identity Registry sends its Mobile Patient Identity Feed messages
//! (PMIR 1.6.0 §2:3.93).
//!
//! The route admits only the client `[pix]` trusts: a bearer token of a
//! client acting for nobody, from `pix.feed_issuer`, issued to
//! `pix.feed_client_id`. ITI-93 asks the Consumer to authenticate its Supplier
//! by mutual TLS or IUA (§2:3.93.5); FerroEHR takes the IUA access token its
//! `[auth]` issuers sign. The message is stored before the response leaves
//! and applied off the request path (`ferroehr::service::demographic::
//! identity_feed`). Any replica answers. The route itself is our own design.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::identity_feed::FeedSender;
use ferroehr::service::demographic::identity_feed::config::{FEED_PATH, PixConfig};
use ferroehr::service::demographic::identity_feed::receive::ReceiveError;
use ferrohealth_smart::kind::CallerKind;
use http::{HeaderValue, StatusCode, header};

use crate::config::AppConfig;
use crate::extensions::access::authn::{self, AuthError, Authenticator};
use crate::state::AppState;

/// The FHIR JSON media type of the response (ITI TF-2 Appendix Z.6).
const FHIR_JSON: &str = "application/fhir+json";

/// The largest message the route reads: a PMIR message of a few identities,
/// far below the server's body ceiling.
const LIMIT: usize = 4 << 20;

/// What the route reads.
#[derive(Clone)]
struct Feed {
    authenticator: Arc<Authenticator>,
    service: Arc<FerroEhrService>,
    pix: Arc<PixConfig>,
    /// The route's own URL, which the response names as its source.
    source: Arc<str>,
}

/// The feed route, or `None` without `[pix]` or a public base URL.
pub(crate) fn router(
    config: &AppConfig,
    authenticator: &Arc<Authenticator>,
    service: Arc<FerroEhrService>,
    state: AppState,
) -> Option<Router<AppState>> {
    let pix = config.pix.as_ref()?;
    let source = config.public_root.as_ref()?.at(FEED_PATH);
    let feed = Feed {
        authenticator: Arc::clone(authenticator),
        service,
        pix: Arc::clone(pix),
        source: source.into(),
    };
    let backend = state.backend_handle();
    Some(
        Router::new()
            .route(FEED_PATH, post(receive).with_state(feed))
            .layer(axum::middleware::from_fn(
                crate::router::release_after_handler,
            ))
            .layer(from_fn_with_state(
                state,
                crate::system_log::middleware::middleware,
            ))
            .layer(from_fn_with_state(
                backend,
                crate::binding_changes::middleware,
            ))
            .layer(axum::middleware::from_fn(
                crate::router::one_connection_per_request,
            )),
    )
}

/// An `OperationOutcome`-free refusal: the status alone, with the bearer
/// challenge on a `401` (RFC 6750 §3).
fn refused(status: StatusCode) -> Response {
    let mut response = status.into_response();
    if status == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    response
}

/// A FHIR JSON answer of `status`.
fn fhir(status: StatusCode, body: Vec<u8>) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, FHIR_JSON)],
        Body::from(body),
    )
        .into_response()
}

/// `POST /pmir/feed`.
async fn receive(State(feed): State<Feed>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let principal = match feed.authenticator.authenticate(&parts).await {
        Ok(authenticated) => authenticated.principal,
        Err(error) => return refused(AuthError::status(&error)),
    };
    let caller = principal.access.caller();
    let issuer = caller
        .and_then(|caller| caller.issuer())
        .unwrap_or_default();
    let client_id = authn::client_of(&principal).unwrap_or_default();
    let trusted = caller.is_some_and(|caller| matches!(caller.kind(), CallerKind::System))
        && issuer == feed.pix.feed_issuer.trim()
        && client_id == feed.pix.feed_client_id.trim();
    if !trusted {
        tracing::warn!("an ITI-93 message from a client [pix] does not trust is refused");
        return refused(StatusCode::FORBIDDEN);
    }
    let sender = FeedSender {
        issuer: issuer.to_owned(),
        client_id,
    };
    let Ok(bytes) = to_bytes(body, LIMIT).await else {
        return refused(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let media = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    match feed
        .service
        .pmir_receive(&sender, media, &bytes, &feed.source)
        .await
    {
        Ok(received) => fhir(StatusCode::OK, received.acknowledgement),
        Err(ReceiveError::Malformed { outcome, .. }) => fhir(StatusCode::BAD_REQUEST, outcome),
        Err(error) => {
            tracing::error!(
                error = %ferroehr::error::ErrorChain::new(&error),
                "an ITI-93 message could not be stored; the Registry sends it again"
            );
            refused(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
