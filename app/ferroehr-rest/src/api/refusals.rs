// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals more than one API group declares word for word, each as a
//! response map an operation's `responses(...)` names.
//!
//! The served document inlines a response map, so naming one here changes no
//! byte of it. No specification governs this: our own design.

/// A stored version that uses specification surface the active `spec_profile` does not define.
#[derive(utoipa::IntoResponses)]
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
pub(crate) struct SpecProfileConflict;

/// A request body declared as anything other than `application/json`.
#[derive(utoipa::IntoResponses)]
#[response(
    status = 415,
    description = "The request `Content-Type` is not `application/json`."
)]
pub(crate) struct JsonBodyRequired;

/// The responses of an [`utoipa::IntoResponses`] map, by status.
pub(crate) type ResponseMap =
    std::collections::BTreeMap<String, utoipa::openapi::RefOr<utoipa::openapi::response::Response>>;

/// One `application/json` response per `(status, description)`, whose body is
/// any JSON value.
#[expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (OpenAPI \
              schema literals)"
)]
pub(crate) fn json_responses(answers: &[(u16, &str)]) -> ResponseMap {
    answers
        .iter()
        .map(|&(status, description)| {
            let content = utoipa::openapi::ContentBuilder::new()
                .schema(Some(<serde_json::Value as utoipa::PartialSchema>::schema()))
                .build();
            let response = utoipa::openapi::ResponseBuilder::new()
                .description(description)
                .content("application/json", content)
                .build();
            (status.to_string(), response.into())
        })
        .collect()
}
