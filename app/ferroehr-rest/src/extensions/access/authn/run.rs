// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The authenticated request run: the credential verified once, then the
//! caller bound, granted, gated and served, and a bodiless `GET` run as a
//! ride that may run once more from the binding on (#6320). No openEHR spec
//! governs the order: our own design.

use std::sync::Arc;

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
    if layer.authenticator.enabled() && req.method() == http::Method::GET && bodiless {
        Box::pin(riding(layer, req, next))
    } else {
        Box::pin(served(layer, req, next))
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

/// The bodiless `GET` `req` authenticated once and, on a service whose
/// clinical statements can read the party store's revisions, served as a
/// ride, so it can run again whole.
async fn riding(layer: AuthLayer, req: Request, mut next: Next) -> Response {
    // NOTE: our own design (#6320), each step of a ride runs in an allocation of its own, so the
    // frame that waits beside a run holds only what the rerun needs.
    let auth = &layer.authenticator;
    let (parts, body) = req.into_parts();
    let access = match Box::pin(auth.verify(&parts)).await {
        Ok(access) => access,
        Err(e) => return refusal::refusal_of(auth, &parts.headers, &e),
    };
    if !matches!(Box::pin(layer.service.party_door_open()).await, Ok(true)) {
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
    // The rerun keeps the request's extensions (its matched route and request id), never a body.
    let again = parts.clone();
    let door = Some(layer.service.grants().held_snapshot());
    let (resp, ridden) = ferroehr::party_binding::request::ride(
        door,
        Box::pin(decided(
            &layer,
            access.clone(),
            Request::from_parts(parts, body),
            &mut next,
        )),
    )
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
        Request::from_parts(again, axum::body::Body::empty()),
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
