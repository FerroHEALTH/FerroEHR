// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals more than one API group declares word for word, each as a
//! response map an operation's `responses(...)` names.
//!
//! The served document inlines a response map, so naming one here changes no
//! byte of it. No specification governs this: our own design.

/// A stored version that uses specification surface the active `spec_profile` does not define.
#[derive(utoipa::IntoResponses)]
#[expect(
    dead_code,
    reason = "a response map the OpenAPI document reads; the refusal itself is built by the service"
)]
pub(crate) enum SpecProfileConflict {
    #[response(
        status = 409,
        description = "OUR OWN EXTENSION — no openEHR spec \
                      governs runtime specification-generation \
                      selection. The stored version body uses \
                      openEHR specification surface this \
                      deployment's active `spec_profile` does \
                      not define, so it is refused rather than \
                      served under a generation set that cannot \
                      express it — and never down-converted. \
                      Reachable only where `spec_profile = \"stable\"` \
                      is configured; the body names switching \
                      back to `development` as the remedy."
    )]
    Conflict,
}

/// A request body declared as anything other than `application/json`.
#[derive(utoipa::IntoResponses)]
#[expect(
    dead_code,
    reason = "a response map the OpenAPI document reads; the refusal itself is built by the handler"
)]
pub(crate) enum JsonBodyRequired {
    #[response(
        status = 415,
        description = "The request `Content-Type` is not `application/json`."
    )]
    NotJson,
}
