// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A request whose own commit changed the binding side table drops this
//! replica's copy of the binding lookups and names before it answers
//! ([`ferroehr::party_binding::live`]), so the change holds on the next
//! request here whatever the delay of its notification: a deprovisioned key
//! refuses at once. No openEHR spec governs resolving a login to a party: our
//! own design (#5771).

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use ferroehr::service::FerroEhrService;

/// Runs the request, then drops the binding copy when its commit changed the
/// side table.
pub(crate) async fn middleware(
    State(service): State<Arc<FerroEhrService>>,
    request: Request,
    next: Next,
) -> Response {
    let (response, changed) = ferroehr::party_binding::live::tracking(next.run(request)).await;
    if changed {
        service.bindings().forget_all();
    }
    response
}
