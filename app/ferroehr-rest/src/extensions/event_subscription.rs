// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the event-subscription admin extension API group over the
//! `ferroehr::service::EventSubscriptionAdapter` seam.
//!
//! **No openEHR spec governs this — our own enterprise feature (eventing).**
//! Event/subscription semantics have no SM or ITS-REST governance, so this
//! surface is ours: exposed under the server's extension namespace and excluded
//! from the ITS-REST drift check. It is mounted under `/admin/` (a subscription
//! store is an administrative resource), so the coarse RBAC gate fail-safe
//! classes it as `Admin` (requires the admin role when RBAC is on), matching
//! the physical-delete ADMIN group.
//!
//! NOTE (no SM call, no ABAC/audit): the CRUD dispatches to the
//! `EventSubscriptionAdapter` extension, not an SM interface. Like the
//! terminology extension it carries no ABAC resource kind (the generic PEP
//! `Skip`s it) and no ATNA audit-table entry (subscriptions are configuration,
//! not PHI access) — the fallbacks apply automatically.
//!
//! The group is config-gated (`AppConfig::events_admin_api`, default
//! `false`): when disabled every route answers `404` without touching the
//! backend.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::overview::error::RestError;

use crate::negotiate;
use crate::state::AppState;

/// The event-subscription extension routes as a native `utoipa-axum` router
/// (group-relative paths; nested under `base_path`), mounted under `/admin`
/// (the coarse RBAC gate classes it `Admin`). Served through [`guarded_dispatch`]
/// → [`dispatch`]. No openEHR spec governs eventing — our own extension.
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (handlers in a single call must share the path;
    // mixing paths panics at router build with "Overlapping method route").
    OpenApiRouter::new()
        .routes(routes!(event_subscription_list, event_subscription_create))
        .routes(routes!(
            event_subscription_get,
            event_subscription_update,
            event_subscription_delete
        ))
}

/// List every event subscription (`GET /admin/event_subscription`).
///
/// Config-gated: `404` when `events_admin_api` is off (the route stays mounted
/// but the backend is never consulted).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines eventing or a subscription resource, so the whole group
/// (paths, payloads, status codes) is our own design.
#[utoipa::path(
    get, path = "/admin/event_subscription", tag = "event-subscription",
    responses(
        (status = 200, description = "The subscription records.", body = serde_json::Value),
        (status = 404, description = "The event-subscription extension is disabled (`events_admin_api` off). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
    )
)]
pub(crate) async fn event_subscription_list(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "event_subscription_list", parts, dispatch).await
}

/// Create a subscription (`POST /admin/event_subscription`).
///
/// Body: `{name, kind?, change_type?, template_id?, enabled?}`
/// (`enabled` defaults to `true`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines eventing or a subscription resource, so the whole group
/// (paths, payloads, status codes) is our own design.
#[utoipa::path(
    post, path = "/admin/event_subscription", tag = "event-subscription",
    request_body(content = serde_json::Value, description = "The subscription definition (canonical JSON)."),
    responses(
        (status = 201, description = "Created; the stored subscription record is returned.", body = serde_json::Value),
        (status = 400, description = "`name` is missing/empty or not matching `[A-Za-z0-9_.-]`, or the body is not valid JSON.", body = serde_json::Value),
        (status = 409, description = "A subscription with that name already exists.", body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not `application/json`.", body = serde_json::Value),
        (status = 404, description = "The event-subscription extension is disabled (`events_admin_api` off). With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
    )
)]
pub(crate) async fn event_subscription_create(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "event_subscription_create", parts, dispatch).await
}

/// Read one subscription by id
/// (`GET /admin/event_subscription/{subscription_id}`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines eventing or a subscription resource, so the whole group
/// (paths, payloads, status codes) is our own design.
#[utoipa::path(
    get, path = "/admin/event_subscription/{subscription_id}", tag = "event-subscription",
    params(("subscription_id" = String, Path, description = "The subscription UUID.")),
    responses(
        (status = 200, description = "The subscription record.", body = serde_json::Value),
        (status = 400, description = "`subscription_id` is not a valid UUID.", body = serde_json::Value),
        (status = 404, description = "No subscription with that id, or the event-subscription extension is disabled. With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
    )
)]
pub(crate) async fn event_subscription_get(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "event_subscription_get", parts, dispatch).await
}

/// Replace one subscription's predicates + enabled flag
/// (`PUT /admin/event_subscription/{subscription_id}`).
///
/// The `name` is immutable (it is the queue key); only the predicates and the
/// `enabled` flag are replaced.
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines eventing or a subscription resource, so the whole group
/// (paths, payloads, status codes) is our own design.
#[utoipa::path(
    put, path = "/admin/event_subscription/{subscription_id}", tag = "event-subscription",
    params(("subscription_id" = String, Path, description = "The subscription UUID.")),
    request_body(content = serde_json::Value, description = "The replacement subscription definition (canonical JSON). The update is a FULL REPLACE: `enabled` is required (omitting it is a 400 — a defaulted value could silently re-enable a disabled subscription), an omitted predicate becomes the wildcard, the `name` is immutable (an echoed one is ignored), and any unknown key is refused."),
    responses(
        (status = 200, description = "Updated; the stored subscription record is returned.", body = serde_json::Value),
        (status = 400, description = "`subscription_id` is not a valid UUID, or the body is not valid JSON.", body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not `application/json`.", body = serde_json::Value),
        (status = 404, description = "No subscription with that id, or the event-subscription extension is disabled. With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
    )
)]
pub(crate) async fn event_subscription_update(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "event_subscription_update", parts, dispatch).await
}

/// Delete one subscription
/// (`DELETE /admin/event_subscription/{subscription_id}`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: neither the SM nor
/// ITS-REST defines eventing or a subscription resource, so the whole group
/// (paths, payloads, status codes) is our own design.
#[utoipa::path(
    delete, path = "/admin/event_subscription/{subscription_id}", tag = "event-subscription",
    params(("subscription_id" = String, Path, description = "The subscription UUID.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 400, description = "`subscription_id` is not a valid UUID.", body = serde_json::Value),
        (status = 404, description = "No subscription with that id, or the event-subscription extension is disabled. With authentication enabled, an unauthenticated request to a disabled group is answered `401` first (the group gate sits behind authentication).", body = serde_json::Value)
    )
)]
pub(crate) async fn event_subscription_delete(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "event_subscription_delete", parts, dispatch).await
}

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
    // Config gate: the group is opt-in. When disabled every route answers 404
    // (as if unmounted) without consulting the backend.
    if !state.config().events_admin_api {
        return Err(RestError(ApiError::NotFound(
            "event subscription API is disabled".to_owned(),
        )));
    }

    let h = &parts.headers;

    match op {
        "event_subscription_list" => {
            let items = state.backend().event_subscription_list().await?;
            Ok(negotiate::respond(h, StatusCode::OK, &items))
        }
        "event_subscription_create" => {
            let body: ferroehr::extensions::events::subscription::SubscriptionDefinition =
                negotiate::typed_json(h, &parts.body)?;
            let created = state.backend().event_subscription_create(body).await?;
            Ok(negotiate::respond(h, StatusCode::CREATED, &created))
        }
        "event_subscription_get" => {
            let id = subscription_id(&parts)?;
            let item = state.backend().event_subscription_get(id).await?;
            Ok(negotiate::respond(h, StatusCode::OK, &item))
        }
        "event_subscription_update" => {
            let id = subscription_id(&parts)?;
            let body: ferroehr::extensions::events::subscription::SubscriptionUpdate =
                negotiate::typed_json(h, &parts.body)?;
            let updated = state.backend().event_subscription_update(id, body).await?;
            Ok(negotiate::respond(h, StatusCode::OK, &updated))
        }
        "event_subscription_delete" => {
            let id = subscription_id(&parts)?;
            state.backend().event_subscription_delete(id).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted event subscription operation: {other}"
        )))),
    }
}

/// Parse the `{subscription_id}` path parameter as a UUID → `400` when malformed
/// (a missing param is a routing bug → `500`).
#[expect(
    clippy::map_err_ignore,
    reason = "`uuid::Error` carries only \"this is not a UUID\", which the 400 body \
              already states"
)]
fn subscription_id(parts: &RequestParts) -> Result<Uuid, RestError> {
    let raw = parts.path.get("subscription_id").ok_or_else(|| {
        RestError(ApiError::Internal(
            "missing path parameter `subscription_id`".to_owned(),
        ))
    })?;
    raw.parse::<Uuid>().map_err(|_| {
        RestError(ApiError::BadRequest(format!(
            "invalid event subscription id `{raw}`"
        )))
    })
}
