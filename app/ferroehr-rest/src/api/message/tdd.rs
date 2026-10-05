// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_TDD_SERVICE` over the MESSAGE extension wire — **our own extension**
//! (see [`super`] for the whole group's spec-silence flag).
//!
//! `i_tdd_service.adoc` declares `import_tdd(an_ehr_id, tdd: String)`; the
//! batch form `import_tdds` carries no SM signature and is this product's
//! all-or-nothing extension of the interface (flagged in
//! `ferroehr::service::message::tdd`). The routes:
//!
//! | SM operation | route |
//! |---|---|
//! | `import_tdd(an_ehr_id, tdd)` | `POST /message/tdd/{ehr_id}` |
//! | `import_tdds(an_ehr_id, tdds)` | `POST /message/tdd/{ehr_id}/batch` |
//!
//! A TDD is an opaque `String` to the SM — concretely a template-namespaced XML
//! instance of a COMPOSITION conforming to the template-derived TDS ("a kind of
//! XSD", AM OPT2 `master02-overview.adoc` §Purpose of the OPT) — so the single
//! import takes the document as an `application/xml` body, and the batch takes
//! the SM's `List<String>` as a JSON array of such documents. Each import
//! commits through the ordinary validated COMPOSITION path, so its created
//! `OBJECT_VERSION_ID` is what the response names.
//!
//! NOTE (no openEHR spec governs role semantics on an unspecified route — our
//! own design/extension): both routes are writes behind the shared
//! authentication + RBAC layer, so both answer `401` without a valid principal
//! and `403` for a principal holding the configured read-only role, in both
//! cases before the payload is read.
//!
//! NOTE (no openEHR spec governs a batch bound — our own design/extension):
//! the batch carries NO cardinality bound of its own. The only bound is the
//! server-wide request-body limit (`ferroehr-rest::router`), which the
//! `tower-http` `RequestBodyLimitLayer` enforces as `413 Payload Too Large`
//! before routing — so an absurd batch is refused by size, never by count.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The TDD extension routes as a native `utoipa-axum` router — **no ITS-REST
/// contract** (see the module docs). Group-relative paths (nested under
/// `base_path`); every operation runs through [`guarded_dispatch`] with
/// [`dispatch`].
pub(crate) fn tdd_routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (handlers in a single call must share the path).
    OpenApiRouter::new()
        .routes(routes!(message_import_tdd))
        .routes(routes!(message_import_tdds))
}

/// Import one Template Data Document (`POST /message/tdd/{ehr_id}`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_TDD_SERVICE.import_tdd`.
#[utoipa::path(
    post, path = "/message/tdd/{ehr_id}", tag = "message",
    params(("ehr_id" = String, Path,
            description = "The EHR the converted COMPOSITION is committed to.")),
    request_body(content = String, content_type = "application/xml",
                 description = "The TDD document — the SM `tdd: String` \
                                parameter. Its root element declares the \
                                template-data namespace and names the \
                                operational template it instantiates, which \
                                must already be provisioned through the \
                                DEFINITION API."),
    responses(
        (status = 201, description = "The TDD converted and committed. The body \
                                      names the created COMPOSITION's \
                                      `OBJECT_VERSION_ID` (`{ \"uid\": … }`).",
         body = serde_json::Value,
         example = json!({ "uid": "8849182c-82ad-4088-a07f-48ead4180515::local.ferroehr.eu::1" })),
        (status = 400, description = "The root is not in the template-data \
                                      namespace, carries no `template_id`, or \
                                      the body does not conform to the \
                                      operational template — SM \
                                      `precondition_violation`.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "The authenticated principal carries the \
                                      configured read-only role: a TDD import \
                                      commits a COMPOSITION, so it is refused \
                                      before the document is read.",
         body = serde_json::Value),
        (status = 404, description = "SM `ehr_id_does_not_exist`, or \
                                      `template_does_not_exist` — the TDD names \
                                      an operational template this server does \
                                      not hold.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/xml`.",
         body = serde_json::Value),
        (status = 422, description = "The payload is not well-formed XML, or \
                                      the produced COMPOSITION fails \
                                      WebTemplate / RM-invariant / terminology \
                                      validation at commit.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn message_import_tdd(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "message_import_tdd", parts, dispatch).await
}

/// Import a batch of Template Data Documents
/// (`POST /message/tdd/{ehr_id}/batch`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes `I_TDD_SERVICE.import_tdds`, itself an extension
/// of the SM interface (which declares no signature for it): the batch is
/// all-or-nothing — every TDD is converted before any is committed, so one
/// unconvertible document rejects the whole batch with nothing committed.
///
/// An EMPTY array is a fulfilled no-op answered `200` with `[]`: the target
/// EHR is still checked (an unknown one is `404` whatever the batch holds),
/// but nothing is created, so `201` would misreport the outcome (RFC 9110
/// §15.3.2).
#[utoipa::path(
    post, path = "/message/tdd/{ehr_id}/batch", tag = "message",
    params(("ehr_id" = String, Path,
            description = "The EHR every converted COMPOSITION is committed to.")),
    request_body(content = Vec<String>,
                 description = "The `List<String>` of TDD documents, as a JSON \
                                array of XML strings.",
                 example = json!(["<template_data …/>"])),
    responses(
        (status = 200, description = "The batch was EMPTY: a fulfilled \
                                      no-op — the target EHR was checked, \
                                      nothing was converted, nothing was \
                                      committed, and the body is `[]`. \
                                      Distinguished from `201` because no \
                                      resource was created (RFC 9110 §15.3.2: \
                                      `201` reports that the request \
                                      \"resulted in one or more new resources \
                                      being created\"; §15.3.1 is the \
                                      fulfilled-with-a-representation case).",
         body = Vec<String>, example = json!([])),
        (status = 201, description = "Every TDD converted and committed. The \
                                      body is the created \
                                      `OBJECT_VERSION_ID`s in input order.",
         body = Vec<String>),
        (status = 400, description = "The body is not a JSON array of strings, \
                                      or any TDD in it does not conform to its \
                                      operational template — nothing is \
                                      committed.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "The authenticated principal carries the \
                                      configured read-only role: the batch \
                                      commits COMPOSITIONs, so it is refused \
                                      before the array is read.",
         body = serde_json::Value),
        (status = 404, description = "SM `ehr_id_does_not_exist`, or a TDD names \
                                      an operational template this server does \
                                      not hold. The EHR precondition is checked \
                                      for EVERY batch, the empty one included.",
         body = serde_json::Value),
        (status = 413, description = "The request body exceeds the server-wide \
                                      request-body limit. The batch has no \
                                      cardinality bound of its own — see the \
                                      module note.",
         body = String, content_type = "text/plain"),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value),
        (status = 422, description = "Any document is not well-formed XML, or \
                                      any produced COMPOSITION fails \
                                      validation — nothing is committed.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn message_import_tdds(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "message_import_tdds", parts, dispatch).await
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub(crate) fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        run(state, op, parts)
            .await
            .unwrap_or_else(IntoResponse::into_response)
    })
}

async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let ehr_id = super::path_ehr_id(&parts)?;

    match op {
        "message_import_tdd" => {
            // A TDD is an XML document, so the body is read as text under an
            // `application/xml` declaration — never through the canonical-RM
            // decoder, which would try to parse it as an RM class.
            negotiate::require_content_type(
                h,
                &[negotiate::WireFormat::CanonicalXml],
                "application/xml",
            )?;
            let tdd = negotiate::text_body(&parts.body)?;
            let uid = state.backend().import_tdd(ehr_id, tdd).await?;
            Ok(negotiate::identifier_response(h, StatusCode::CREATED, &uid))
        }
        "message_import_tdds" => {
            let items = negotiate::json_vec(h, &parts.body)?;
            let mut tdds = Vec::with_capacity(items.len());
            for item in &items {
                let text = item.as_str().ok_or_else(|| {
                    RestError(ApiError::BadRequest(format!(
                        "every batch element must be a TDD document string, got {item}"
                    )))
                })?;
                tdds.push(text.to_owned());
            }
            let uids = state.backend().import_tdds(ehr_id, tdds).await?;
            // An empty batch created nothing, so it is fulfilled-with-a-
            // representation, not a creation: RFC 9110 §15.3.2 reserves `201`
            // for a request that "resulted in one or more new resources being
            // created". No openEHR spec governs this route — our own
            // design/extension.
            let status = if uids.is_empty() {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            let body: Vec<Value> = uids.into_iter().map(Value::String).collect();
            Ok(negotiate::respond(h, status, &body))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted message TDD operation: {other}"
        )))),
    }
}
