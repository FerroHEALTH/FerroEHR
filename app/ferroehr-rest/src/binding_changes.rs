// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A request whose own commit changed the binding side table drops this
//! replica's copy of the bound `PERSON`s' names before it answers
//! ([`ferroehr::party_binding::live`]), so a rename holds on the next request
//! here whatever the delay of its notification; that request reads the moved
//! binding revision, so a deprovisioned key refuses at once. No openEHR spec
//! governs resolving a login to a party: our own design (#5771, #6403).

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use ferroehr::service::FerroEhrService;

/// Runs the request, then drops the held names when its commit changed the
/// side table.
pub(crate) async fn middleware(
    State(service): State<Arc<FerroEhrService>>,
    request: Request,
    next: Next,
) -> Response {
    let (response, changed) = ferroehr::party_binding::request::scope(
        ferroehr::party_binding::live::tracking(next.run(request)),
    )
    .await;
    if changed {
        service.bindings().forget_names();
    }
    response
}
