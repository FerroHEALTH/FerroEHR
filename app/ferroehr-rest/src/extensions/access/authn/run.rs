// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The authenticated request run: the credential verified once, then the
//! caller bound, granted, gated and served, and a bodiless `GET` or an AQL
//! `POST` run as a ride that may run once more from the binding on (#6320).
//! No openEHR spec governs the order: our own design.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use ferroehr::party_binding::grants::operation::Operation;
use ferroehr::system_log::event::ActorAuthentication;
use ferrohealth_smart::resource::Access;

use super::{
    AuthLayer, Authenticated, REQUEST_PRINCIPAL, gates, grants, portal, purpose, record, refusal,
    trc,
};
use gates::patient_data_refusal;

/// The authentication and RBAC middleware, attached to the API router; the
/// public endpoints are mounted outside it.
///
/// A request that may ride (#6320) and one that may not each run in an
/// allocation of their own size: the state of a ride never enlarges the
/// request that cannot ride.
pub(crate) fn middleware(
    State(layer): State<AuthLayer>,
    req: Request,
    next: Next,
) -> std::pin::Pin<Box<dyn Future<Output = Response> + Send>> {
    let bodiless = axum::body::HttpBody::size_hint(req.body()).exact() == Some(0);
    let enabled = layer.authenticator.enabled();
    if enabled && req.method() == http::Method::GET && bodiless {
        Box::pin(riding(layer, req, None, next))
    } else if enabled
        && req.method() == http::Method::POST
        && layer.post_rides.admits(
            req.extensions()
                .get::<MatchedPath>()
                .map(MatchedPath::as_str),
        )
    {
        Box::pin(riding_post(layer, req, next))
    } else {
        Box::pin(served(layer, req, next))
    }
}

/// The operations whose `POST` may ride (#6320): the AQL executions, reads
/// whose query travels as the body.
const POST_RIDES: [&str; 3] = [
    "query_execute_adhoc_query_body",
    "query_execute_stored_query_body",
    "query_execute_stored_query_version_body",
];

/// The route templates of [`POST_RIDES`] under a deployment's openEHR base
/// path, matched against a request's `MatchedPath`.
#[derive(Debug, Default)]
pub(crate) struct PostRides {
    templates: Vec<String>,
}

impl PostRides {
    /// The templates of the API mounted in `namespaces`.
    pub(crate) fn new(namespaces: &crate::api::Namespaces) -> Self {
        // NOTE: our own design (#6320), a POST rides only when its operation is named in
        // POST_RIDES, so a route added later stays off until it is named there.
        let templates = openehr_its::rest::generated::query::ROUTES
            .iter()
            .filter(|(method, _, op)| *method == "POST" && POST_RIDES.contains(op))
            .map(|(_, path, _)| {
                format!("{}{}", namespaces.openehr, crate::api::normalize_path(path))
            })
            .collect();
        Self { templates }
    }

    /// Whether a `POST` to the route template `matched` may ride.
    fn admits(&self, matched: Option<&str>) -> bool {
        matched.is_some_and(|matched| self.templates.iter().any(|held| held == matched))
    }
}

/// The request `req` authenticated and served once.
async fn served(layer: AuthLayer, req: Request, mut next: Next) -> Response {
    let auth = &layer.authenticator;
    if !auth.enabled() {
        return REQUEST_PRINCIPAL.scope(None, next.run(req)).await;
    }
    // Each part of the request is moved into the run before it is awaited, so this future keeps
    // no second copy of any.
    let run = {
        let (parts, body) = req.into_parts();
        let access = match auth.verify(&parts).await {
            Ok(access) => access,
            Err(e) => return refusal::refusal_of(auth, &parts.headers, &e),
        };
        decided(&layer, access, Request::from_parts(parts, body), &mut next)
    };
    run.await
}

/// The AQL `POST` `req` as a ride ([`riding`]), its body taken back as the
/// bytes the body limit buffered it into, which the rerun reads again.
async fn riding_post(layer: AuthLayer, req: Request, next: Next) -> Response {
    let (parts, body) = req.into_parts();
    let bytes = match buffered(body).await {
        Ok(bytes) => bytes,
        Err(refusal) => return *refusal,
    };
    let again = bytes.clone();
    riding(
        layer,
        Request::from_parts(parts, Body::from(bytes)),
        Some(again),
        next,
    )
    .await
}

/// `body` as the bytes `crate::limits::middleware` buffered it into, one
/// chunk taken back by its reference count, so nothing is copied.
async fn buffered(body: Body) -> Result<Bytes, Box<Response>> {
    axum::body::to_bytes(body, usize::MAX).await.map_err(|_unreadable| {
        // NOTE: RFC 9110 §15.5.1, as the body limit answers a body it cannot read.
        Box::new(crate::overview::error::status_error_response(
            http::StatusCode::BAD_REQUEST,
            "request body cannot be read",
        ))
    })
}

/// The bodiless `GET` or the AQL `POST` `req`, whose body `kept` holds,
/// authenticated once and, on a service whose clinical statements can read
/// the party store's revisions, served as a ride, so it can run again whole.
async fn riding(layer: AuthLayer, req: Request, kept: Option<Bytes>, mut next: Next) -> Response {
    // NOTE: our own design (#6320), each step of a ride runs in an allocation of its own, so the
    // frame that waits beside a run holds only what the rerun needs.
    let auth = &layer.authenticator;
    let (parts, body) = req.into_parts();
    let access = match Box::pin(auth.verify(&parts)).await {
        Ok(access) => access,
        Err(e) => return refusal::refusal_of(auth, &parts.headers, &e),
    };
    if !matches!(Box::pin(layer.service.party_door_open()).await, Ok(true)) {
        drop(kept);
        return Box::pin(decided(
            &layer,
            access,
            Request::from_parts(parts, body),
            &mut next,
        ))
        .await;
    }
    // NOTE: our own design (#6320), a ride decides on the party copies as they stand, and its
    // answer leaves only once they held the store's revisions its clinical statement read.

    // The rerun keeps the request's extensions (its matched route and request id) and an AQL
    // POST's body, the same bytes.
    let again = parts.clone();
    let (resp, ridden) = ferroehr::party_binding::request::ride(Box::pin(decided(
        &layer,
        access.clone(),
        Request::from_parts(parts, body),
        &mut next,
    )))
    .await;
    let op = ridden.op().unwrap_or("unnamed");
    if matches!(Box::pin(layer.service.ride_held(ridden)).await, Ok(true)) {
        return resp;
    }
    // NOTE: our own design (#6320), the run the store overtook leaves nothing: its answer and
    // what it would have counted go unused, and the request runs on the path that reads the
    // store's revisions first, which answers whatever they are.
    drop(resp);
    ferroehr::telemetry::access_path::count_policy_rerun(op);
    Box::pin(decided(
        &layer,
        access,
        Request::from_parts(again, kept.map_or_else(Body::empty, Body::from)),
        &mut next,
    ))
    .await
}

/// The request of the verified `access` bound, granted, gated and served:
/// everything of it that rests on the party copies.
async fn decided(layer: &AuthLayer, access: Access, mut req: Request, next: &mut Next) -> Response {
    let (auth, live) = (&layer.authenticator, layer.service.grants());
    match auth.bind(access).await {
        Ok(Authenticated { principal, fresh }) => {
            let granted = match grants::resolve(live, &principal, Some(&auth.deployment)).await {
                Ok(granted) => granted,
                Err(e) => return refusal::response(auth, req.headers(), &e),
            };
            let mut principal = match grants::bound(layer, principal, &granted, req.headers()) {
                Ok(principal) => principal,
                Err(refusal) => return *refusal,
            };
            if let Some(refusal) = purpose::mismatch(layer, &principal, &req) {
                return refusal;
            }
            if let Some(refusal) = trc::patient_context(layer, &mut principal, &req).await {
                return refusal;
            }
            // NOTE: RFC 9110 §15.5.6; whether a method is served is the
            // resource's property, so no grant changes the `405`.
            if let Some(mut refusal) = layer.served.refusal(
                req.method(),
                req.extensions()
                    .get::<MatchedPath>()
                    .map(MatchedPath::as_str),
            ) {
                refusal.extensions_mut().insert(Arc::new(principal));
                return refusal;
            }
            // NOTE: our own design (#5754), the gates above add grants and extensions to
            // the caller and never change its issuer, client or party, so `granted` holds.
            // NOTE: master08 §Resource Scopes, a `system/aql-*` grant reaches every
            // query, so it counts only for a client the CDR grants `query_system_wide`.
            principal.honoured.backend = grants::client_holds(&granted, Operation::QuerySystemWide);
            // How the person authenticated is judged before what they may do:
            // a step-up refusal is an authentication outcome (RFC 9470 §3).
            let actor = match layer.patient_data.as_deref() {
                None => ActorAuthentication::default(),
                Some(gate) => {
                    let matched = req
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str);
                    match gate.judge(&principal, matched) {
                        Ok(actor) => actor,
                        Err((refusal, actor)) => {
                            return patient_data_refusal(gate, &principal, refusal, actor);
                        }
                    }
                }
            };
            if let Some(mut refusal) = gates::rbac_refusal(layer, &req, &principal) {
                refusal.extensions_mut().insert(actor);
                return refusal;
            }
            if let Some(mut refusal) = gates::smart_refusal(layer, &req, &principal, &granted) {
                refusal.extensions_mut().insert(actor);
                return refusal;
            }
            crate::system_log::written::learn_actor(&actor);
            let patient = gates::patient_person(layer, &req, &principal).await;
            let principal = Arc::new(principal);
            req.extensions_mut().insert(Arc::clone(&principal));
            let (committer, patient) =
                match portal::committer(auth, &principal, &granted, patient).await {
                    Ok(committer) => committer,
                    Err(e) => return refusal::response(auth, req.headers(), &e),
                };
            // The roles ride beside the committer identity for the request
            // scope, so a record the service layer emits by itself carries
            // them too (#3239).
            let roles = principal.roles.clone();
            let binding = record::of_request(&principal, &committer, &granted, Some(auth));
            let mut resp = REQUEST_PRINCIPAL
                .scope(
                    Some(Arc::clone(&principal)),
                    ferroehr::system_log::access_context::with_roles(
                        roles,
                        record::recorded(
                            binding,
                            ferroehr::versioning::committer::with_committer(
                                Some(committer),
                                Box::pin(portal::as_patient(
                                    patient,
                                    grants::scoped(live, granted, onward(next, req)),
                                )),
                            ),
                        ),
                    ),
                )
                .await;
            // The outer ATNA audit layer cannot observe request-extension
            // mutations, so the principal is republished onto the response.
            resp.extensions_mut().insert(principal);
            resp.extensions_mut().insert(actor);
            gates::finish(auth, &mut resp, fresh);
            resp
        }
        Err(e) => refusal::refusal_of(auth, req.headers(), &e),
    }
}

/// The rest of the stack run on `req`, through `next` without spending it,
/// so a ride's rerun takes the same `next` (#6320).
async fn onward(next: &mut Next, req: Request) -> Response {
    match tower::Service::call(next, req).await {
        Ok(resp) => resp,
        Err(never) => match never {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Of every `POST` route the API mounts, only the three AQL executions
    /// ride, so a `POST` route added later does not (#6320).
    #[test]
    fn only_the_aql_executions_ride_on_post() {
        let namespaces = crate::api::Namespaces::of(&ferroehr::config::rest::ApiConfig::default());
        let rides = PostRides::new(&namespaces);
        let doc = crate::api::api_doc(&namespaces);
        let mut riding: Vec<&str> = Vec::new();
        let mut posts = 0;
        for (path, item) in &doc.paths.paths {
            let Some(post) = item.post.as_ref() else {
                continue;
            };
            posts += 1;
            let op = post.operation_id.as_deref().unwrap_or_default();
            assert_eq!(
                rides.admits(Some(path.as_str())),
                POST_RIDES.contains(&op),
                "POST {path} ({op})"
            );
            if rides.admits(Some(path.as_str())) {
                riding.push(op);
            }
        }
        riding.sort_unstable();
        let mut expected = POST_RIDES.to_vec();
        expected.sort_unstable();
        assert_eq!(riding, expected, "each AQL execution rides, and only those");
        assert!(
            posts > POST_RIDES.len(),
            "the walk saw the other POST routes"
        );
        assert!(!rides.admits(None), "an unmatched request never rides");
    }

    /// The rerun of a `POST` ride reads the very bytes the body limit
    /// buffered, taken back by reference and never copied (#6320).
    #[tokio::test]
    async fn the_rerun_reads_the_same_bytes() {
        let sent = Bytes::from_static(b"{\"q\":\"SELECT e/ehr_id/value FROM EHR e\"}");
        let taken = buffered(Body::from(sent.clone()))
            .await
            .expect("an in-memory body reads");
        assert_eq!(taken, sent);
        assert_eq!(
            taken.as_ptr(),
            sent.as_ptr(),
            "the bytes are shared, not copied"
        );
        let rerun = axum::body::to_bytes(Body::from(taken.clone()), usize::MAX)
            .await
            .expect("the rerun's body reads");
        assert_eq!(
            rerun.as_ptr(),
            sent.as_ptr(),
            "the rerun reads the same bytes"
        );
    }
}
