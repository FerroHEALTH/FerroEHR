// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The request scope of a caller a gate other than the API's admitted, such
//! as the SCIM endpoint's: the committer, the roles and the binding record
//! the API's authentication layer sets for its own callers, so a commit names
//! the caller and its access record carries the same fields. No openEHR spec
//! governs access logging or the committer of a request: our own design.

use axum::response::Response;
use ferroehr::party_binding::grants::capabilities::Capabilities;

use super::{Authenticator, Principal, party, record};

impl Authenticator {
    /// Runs `fut` as the request of `principal`: under its committer and
    /// roles, recording the binding outcome and the principal on the
    /// response for the access record.
    pub(crate) async fn run_admitted<F>(&self, principal: Principal, fut: F) -> Response
    where
        F: Future<Output = Response>,
    {
        let granted = Capabilities::default();
        let committer = party::committer_identity(&principal, &granted);
        let binding = record::of_request(&principal, &committer, None, None);
        let mut resp = ferroehr::system_log::access_context::with_roles(
            principal.roles.clone(),
            record::recorded(
                binding,
                ferroehr::versioning::committer::with_committer(Some(committer), fut),
            ),
        )
        .await;
        resp.extensions_mut().insert(principal);
        resp
    }
}
