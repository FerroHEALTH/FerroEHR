// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-level layer every management route sits behind: `404` for an
//! endpoint that is off, `401` for a request without a valid credential, and
//! `403` for a caller the login binding refuses or, at `AdminOnly`, a caller
//! without `authz.rbac.admin_role`. No openEHR spec governs the management
//! surface: our own operational design.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ferroehr::config::authz::RbacConfig;
use ferroehr::config::management::AccessLevel;
use http::{StatusCode, header};
use openehr_its::rest::runtime::ApiError;

use crate::extensions::access::authn::refusal::refusal_of;
use crate::extensions::access::authn::{AuthError, Authenticator};
use crate::extensions::access::authz::classify::OperationClass;
use crate::extensions::access::authz::roles::RbacDecision;
use crate::overview::error::RestError;

/// The per-endpoint access guard: the shared authenticator plus the required
/// level for one route.
#[derive(Clone)]
pub(crate) struct AccessGuard {
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) rbac: RbacConfig,
    pub(crate) level: AccessLevel,
}

/// The access-level middleware: enforce the guard's level, then run the route
/// (or short-circuit with `401`/`403`/`404`). Installed per-route via the `mk`
/// closure in [`super::router`], and around the Swagger surface by the main router.
pub(crate) async fn access_middleware(
    State(guard): State<AccessGuard>,
    req: Request,
    next: Next,
) -> Response {
    let (parts, body) = req.into_parts();
    match guard.check(&parts).await {
        Ok(principal) => {
            let mut resp = next.run(Request::from_parts(parts, body)).await;
            // Republished for the audit layer outside this router, which
            // names the person behind a management request from the response
            // extensions the way the API's authentication layer does (#3244).
            if let Some(principal) = principal {
                resp.extensions_mut().insert(principal);
            }
            resp
        }
        Err(resp) => *resp,
    }
}

impl AccessGuard {
    /// Enforces the access level on the request, handing back its principal.
    async fn check(
        &self,
        headers: &http::request::Parts,
    ) -> Result<Option<crate::extensions::access::authn::Principal>, Box<Response>> {
        match self.level {
            // Defensive: an `Off` endpoint is never mounted; if reached, 404.
            AccessLevel::Off => Err(Box::new(StatusCode::NOT_FOUND.into_response())),
            AccessLevel::Public => Ok(None),
            AccessLevel::Private | AccessLevel::AdminOnly => {
                // Auth disabled (dev): the surface is unauthenticated by design.
                if !self.authenticator.enabled() {
                    return Ok(None);
                }
                // NOTE: RFC 9110 §15.5.4, a binding refusal follows a credential
                // that authenticated, so it answers `403` as every other route does.
                let authenticated = match self.authenticator.authenticate(headers).await {
                    Ok(authenticated) => authenticated,
                    Err(e @ (AuthError::PartyRefused { .. } | AuthError::KindRefused { .. })) => {
                        return Err(Box::new(refusal_of(
                            &self.authenticator,
                            &headers.headers,
                            &e,
                        )));
                    }
                    // One `401` challenge for every other authentication failure:
                    // which part of the credential was rejected is never disclosed.
                    Err(_) => return Err(Box::new(unauthorized(&self.authenticator))),
                };
                if self.level == AccessLevel::AdminOnly
                    && let RbacDecision::Deny(reason) =
                        crate::extensions::access::authz::roles::authorize(
                            OperationClass::Admin,
                            &authenticated.principal.roles,
                            &self.rbac,
                        )
                {
                    return Err(Box::new(forbidden(&reason)));
                }
                Ok(Some(authenticated.principal))
            }
        }
    }
}

fn unauthorized(authenticator: &Authenticator) -> Response {
    let mut resp = RestError::from(ApiError::Unauthorized("authentication required".to_owned()))
        .into_response();
    // No credential was presented, so the challenge carries no RFC 6750 §3.1
    // error code: "If the request lacks any authentication information … the
    // resource server SHOULD NOT include an error code".
    resp.headers_mut()
        .insert(header::WWW_AUTHENTICATE, authenticator.challenge(None));
    resp
}

fn forbidden(reason: &str) -> Response {
    RestError::from(ApiError::Forbidden(format!(
        "management endpoint: {reason}"
    )))
    .into_response()
}
