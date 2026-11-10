// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the terminology extension API group over the
//! `ferroehr::service::TerminologyService` seam.
//!
//! The routes list the known terminologies, read a terminology, a term and a
//! value set, and answer subsumption and value-set membership; an unknown
//! terminology, term or value set is a `404`.
//!
//! No openEHR spec governs the wire shape — our own design/extension. No
//! released OAS set defines a terminology REST contract, so this surface is
//! ours: exposed under the extension namespace `/terminology` and excluded from
//! the ITS-REST drift check.
//!
//! The extension groups mount inside the ITS-REST API router, so the full path
//! is `{rest_root}/extensions/v1/terminology/...`; nesting keeps the auth, ATNA-audit and ABAC
//! middleware stack uniform across the whole HTTP surface. Existence is
//! answered through the `200`-versus-`404` of each read rather than by
//! separate endpoints: the bundle provider maps an unknown terminology, term or
//! value set to `versioned_object_does_not_exist`.
//!
//! The group is config-gated (`api.terminology`, default
//! `false`), answering `404` on every route when disabled.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6): FHIR resources are an external standard \
              with no RM type (typed-FHIR evaluation tracked separately)"
)]

use axum::response::Response;
use http::StatusCode;
use serde_json::json;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use crate::api::{RequestParts, guarded_handler};
use crate::overview::error::RestError;

use crate::state::AppState;
use crate::{negotiate, params};

/// The terminology extension routes as a native `utoipa-axum` router: each
/// `#[utoipa::path]` handler single-sources its route and its `OpenAPI` path.
/// Group-relative paths (nested under the extensions root); every
/// operation is served through [`guarded_dispatch`](crate::api::guarded_dispatch) onto its handler, so the
/// wire behaviour is identical to the generated groups' `mount` adapter.
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (handlers in a single call must share the path;
    // mixing paths panics at router build with "Overlapping method route").
    OpenApiRouter::new()
        .routes(routes!(terminology_ids))
        .routes(routes!(terminology_description))
        .routes(routes!(terminology_get_term))
        .routes(routes!(terminology_subsumes))
        .routes(routes!(terminology_value_set))
        .routes(routes!(terminology_value_set_validate))
}

// ── Handlers (our own wire shape) ─────────────────────────────────────────────
// Every handler snapshots the request into `RequestParts` (identical to the
// generated-group adapter) and runs it through the shared guarded dispatch, so
// the EHR_ACCESS gate, ABAC PEP, and ATNA audit tagging apply uniformly.

guarded_handler! {
    /// Every terminology id the server knows
    /// (`GET /terminology`).
    ///
    /// Body: `{"terminology_ids": [..]}`. Config-gated: `404` when
    /// `api.terminology` is off (the route stays mounted but the backend is
    /// never consulted).
    ///
    /// OUR OWN EXTENSION — no openEHR spec governs this wire shape: no ITS-REST
    /// contract defines a terminology API. The URL layout, the JSON envelopes, and
    /// the existence-by-`404` convention are ours.
    #[utoipa::path(
        get, path = "/terminology", tag = "terminology",
        responses(
            (status = 200, description = "The known terminology ids.", body = serde_json::Value),
            (status = 404, description = "The terminology extension is disabled (`api.terminology` off). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
        )
    )]
    terminology_ids => |state, parts| async move {
            ids(&state, &parts)
        }
}

guarded_handler! {
    /// One terminology's descriptor, which also answers whether it exists
    /// (`GET /terminology/{terminology_id}`).
    ///
    /// An unknown terminology maps to `404`.
    ///
    /// OUR OWN EXTENSION — no openEHR spec governs this wire shape: no ITS-REST
    /// contract defines a terminology API. The URL layout, the JSON envelopes, and
    /// the existence-by-`404` convention are ours.
    #[utoipa::path(
        get, path = "/terminology/{terminology_id}", tag = "terminology",
        params(("terminology_id" = String, Path, description = "The terminology id.")),
        responses(
            (status = 200, description = "The terminology descriptor.", body = serde_json::Value),
            (status = 404, description = "Unknown terminology, or the terminology extension is disabled. With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
        )
    )]
    terminology_description => |state, parts| async move { description(&state, &parts) }
}

guarded_handler! {
    /// A term definition
    /// (`GET /terminology/{terminology_id}/term/{code}`).
    ///
    /// The optional `at_date` selects an effective-dated definition. An unknown
    /// terminology or code maps to `404`.
    ///
    /// OUR OWN EXTENSION — no openEHR spec governs this wire shape: no ITS-REST
    /// contract defines a terminology API. The URL layout, the JSON envelopes, and
    /// the existence-by-`404` convention are ours.
    #[utoipa::path(
        get, path = "/terminology/{terminology_id}/term/{code}", tag = "terminology",
        params(
            ("terminology_id" = String, Path, description = "The terminology id."),
            ("code" = String, Path, description = "The term code."),
            ("at_date" = Option<String>, Query, description = "Optional ISO-8601 effective date; absent means the current definition.")
        ),
        responses(
            (status = 200, description = "The term extract.", body = serde_json::Value),
            (status = 404, description = "Unknown terminology or code, or the terminology extension is disabled. With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
        )
    )]
    terminology_get_term => get_term
}

guarded_handler! {
    /// Strict subsumption test
    /// (`GET /terminology/{terminology_id}/subsumes`).
    ///
    /// Body: `{"subsumes": bool}`. Both codes are required query parameters.
    ///
    /// OUR OWN EXTENSION — no openEHR spec governs this wire shape: no ITS-REST
    /// contract defines a terminology API. The URL layout, the JSON envelopes, and
    /// the existence-by-`404` convention are ours.
    #[utoipa::path(
        get, path = "/terminology/{terminology_id}/subsumes", tag = "terminology",
        params(
            ("terminology_id" = String, Path, description = "The terminology id."),
            ("ref_code" = String, Query, description = "The reference (ancestor-candidate) code. Required."),
            ("candidate" = String, Query, description = "The candidate (descendant) code. Required.")
        ),
        responses(
            (status = 200, description = "The subsumption result.", body = serde_json::Value),
            (status = 400, description = "A required query parameter (`ref_code`/`candidate`) is missing or blank.", body = serde_json::Value),
            (status = 404, description = "The terminology extension is disabled. With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
        )
    )]
    terminology_subsumes => subsumes
}

guarded_handler! {
    /// A value set's extract, which also answers whether it exists
    /// (`GET /terminology/{terminology_id}/value_set/{value_set_id}`).
    ///
    /// An unknown value set maps to `404`.
    ///
    /// OUR OWN EXTENSION — no openEHR spec governs this wire shape: no ITS-REST
    /// contract defines a terminology API. The URL layout, the JSON envelopes, and
    /// the existence-by-`404` convention are ours.
    #[utoipa::path(
        get, path = "/terminology/{terminology_id}/value_set/{value_set_id}", tag = "terminology",
        params(
            ("terminology_id" = String, Path, description = "The terminology id."),
            ("value_set_id" = String, Path, description = "The value set id.")
        ),
        responses(
            (status = 200, description = "The value set extract.", body = serde_json::Value),
            (status = 404, description = "Unknown terminology or value set, or the terminology extension is disabled. With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
        )
    )]
    terminology_value_set => value_set
}

guarded_handler! {
    /// Value-set membership test
    /// (`GET /terminology/{terminology_id}/value_set/{value_set_id}/validate`).
    ///
    /// Body: `{"valid": bool}`. `candidate_code` is a required query parameter.
    ///
    /// OUR OWN EXTENSION — no openEHR spec governs this wire shape: no ITS-REST
    /// contract defines a terminology API. The URL layout, the JSON envelopes, and
    /// the existence-by-`404` convention are ours.
    #[utoipa::path(
        get, path = "/terminology/{terminology_id}/value_set/{value_set_id}/validate", tag = "terminology",
        params(
            ("terminology_id" = String, Path, description = "The terminology id."),
            ("value_set_id" = String, Path, description = "The value set id."),
            ("candidate_code" = String, Query, description = "The candidate code to test for membership. Required."),
            ("at_date" = Option<String>, Query, description = "Optional ISO-8601 effective date; absent means the current value set.")
        ),
        responses(
            (status = 200, description = "The membership result.", body = serde_json::Value),
            (status = 400, description = "The required `candidate_code` query parameter is missing or blank.", body = serde_json::Value),
            (status = 404, description = "The terminology extension is disabled. With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
        )
    )]
    terminology_value_set_validate => value_set_validate
}

/// The config gate: the terminology extension is opt-in, and while it is off
/// every route answers `404` (as if unmounted) without consulting the backend.
///
/// # Errors
/// The `404` while the extension is disabled.
fn enabled(state: &AppState) -> Result<(), RestError> {
    if state.config().api.terminology {
        return Ok(());
    }
    Err(RestError::from(ApiError::NotFound(
        "terminology API is disabled".to_owned(),
    )))
}

/// The `terminology_ids` operation.
pub(crate) fn ids(state: &AppState, parts: &RequestParts) -> Result<Response, RestError> {
    enabled(state)?;
    let h = &parts.headers;
    let ok = StatusCode::OK;
    let ids = ferroehr::service::terminology::bundle::terminology_ids();
    Ok(negotiate::body::respond(
        h,
        ok,
        &json!({ "terminology_ids": ids }),
    ))
}

/// The `terminology_description` operation.
pub(crate) fn description(state: &AppState, parts: &RequestParts) -> Result<Response, RestError> {
    enabled(state)?;
    let h = &parts.headers;
    let ok = StatusCode::OK;
    let tid = path_get(parts, "terminology_id")?;
    // An unknown terminology → NotFound (404).
    let desc = ferroehr::service::terminology::bundle::terminology_description(&tid)?;
    Ok(negotiate::body::respond(h, ok, &desc))
}

/// The `terminology_get_term` operation.
pub(crate) async fn get_term(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    enabled(&state)?;
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let tid = path_get(&parts, "terminology_id")?;
    let code = path_get(&parts, "code")?;
    let at_date = params::query_param(q, "at_date");
    // NOTE: no specification governs this extension: our own design; no
    // attribute allow-list is surfaced on the wire.
    let extract = state.backend().get_term(&tid, &code, None, at_date).await?;
    Ok(negotiate::body::respond(h, ok, &extract))
}

/// The `terminology_subsumes` operation.
pub(crate) async fn subsumes(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    enabled(&state)?;
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let tid = path_get(&parts, "terminology_id")?;
    let ref_code = require_query(q, "ref_code")?;
    let candidate = require_query(q, "candidate")?;
    let subsumes = state
        .backend()
        .subsumes(&tid, &ref_code, &candidate)
        .await?;
    Ok(negotiate::body::respond(
        h,
        ok,
        &json!({ "subsumes": subsumes }),
    ))
}

/// The `terminology_value_set` operation.
pub(crate) async fn value_set(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    enabled(&state)?;
    let h = &parts.headers;
    let ok = StatusCode::OK;
    let tid = path_get(&parts, "terminology_id")?;
    let value_set_id = path_get(&parts, "value_set_id")?;
    // An unknown value set → NotFound (404).
    let extract = state.backend().get_value_set(&tid, &value_set_id).await?;
    Ok(negotiate::body::respond(h, ok, &extract))
}

/// The `terminology_value_set_validate` operation.
pub(crate) async fn value_set_validate(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    enabled(&state)?;
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let tid = path_get(&parts, "terminology_id")?;
    let value_set_id = path_get(&parts, "value_set_id")?;
    let candidate_code = require_query(q, "candidate_code")?;
    let at_date = params::query_param(q, "at_date");
    let valid = state
        .backend()
        .value_set_validate(&tid, &value_set_id, &candidate_code, at_date)
        .await?;
    Ok(negotiate::body::respond(h, ok, &json!({ "valid": valid })))
}

/// Read a matched path parameter (guaranteed present by the route template; an
/// absence is a routing bug → `500`).
fn path_get(parts: &RequestParts, key: &str) -> Result<String, RestError> {
    parts.path.get(key).cloned().ok_or_else(|| {
        RestError::from(ApiError::Internal(format!(
            "missing path parameter `{key}`"
        )))
    })
}

/// Read a required, non-empty query parameter → `400` when absent/blank.
fn require_query(query: Option<&str>, key: &str) -> Result<String, RestError> {
    params::query_param(query, key)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            RestError::from(ApiError::BadRequest(format!(
                "missing required query parameter `{key}`"
            )))
        })
}
