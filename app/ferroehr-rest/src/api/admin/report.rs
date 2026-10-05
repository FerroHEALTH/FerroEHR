// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **activity-report** wire — **our own extension**.
//!
//! No openEHR ITS-REST operation governs any route here. The released Admin API
//! is exactly two operations, both EHR deletes
//! (`specifications/admin.openapi.yaml` → `operations/admin_ehr_delete.yaml` +
//! `operations/admin_ehr_delete_all.yaml`), while the SM declares four
//! reporting calls the release never surfaced —
//! `docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc`:
//! `list_contributions`, `contribution_count`, `versioned_composition_count`,
//! `composition_version_count`, each taking a `PLATFORM_SERVICE` (`a_service`,
//! "Name of a versioned content service") and an optional
//! `Interval<Iso8601_date_time>`. No released ITS-REST operation covers these calls.
//!
//! These routes are the honest realization of that service basis, and are
//! **excluded from ITS-REST wire conformance**: they gate the `ActivityReport`
//! CAPABILITY verdict only.
//!
//! Gating: mounted under `/admin/`, so they inherit the group's two gates
//! unchanged — the coarse RBAC `OperationClass::Admin` classifier (`401`
//! unauthenticated / `403` non-admin, our own authorization design; the
//! released admin operations carry `security: []`) and the
//! `AppConfig::admin.enabled` config gate, which answers `405` with an empty
//! `Allow` while the group is off (`crate::api::admin::dispatch`, whose ground
//! is the overview rule "If a method is recognized but not allowed for the
//! target resource, the response SHOULD be `405 Method Not Allowed` status
//! code" — `docs/overview/Requests_and_responses.md` §"HTTP Methods").

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use ferroehr::service::admin::types::StatTimeRange;
use ferroehr::service::platform_service::PlatformService;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::{negotiate, params};

/// The activity-report extension routes as a native `utoipa-axum` router —
/// **no ITS-REST contract** (see the module docs). Group-relative paths
/// (nested under `base_path`); every operation runs through
/// [`guarded_dispatch`] with [`dispatch`].
pub(crate) fn report_routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (handlers in a single call must share the path).
    OpenApiRouter::new()
        .routes(routes!(admin_report_contributions))
        .routes(routes!(admin_report_contribution_count))
        .routes(routes!(admin_report_versioned_composition_count))
        .routes(routes!(admin_report_composition_version_count))
}

/// List the CONTRIBUTION ids of one platform service
/// (`GET /admin/report/contribution`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_ADMIN_SERVICE.list_contributions`.
#[utoipa::path(
    get, path = "/admin/report/contribution", tag = "admin-report",
    params(
        ("a_service" = String, Query,
         description = "The `PLATFORM_SERVICE` member naming the \
                        versioned-content service to report on — one of \
                        `Admin`, `Definitions`, `Ehr`, `Ehr_index`, \
                        `Demographic`, `Message`, `Query`, `System_log` (SM \
                        `platform_service.adoc`; matched case-insensitively). \
                        A service that holds no versioned content reports \
                        empty/zero rather than failing.",
         example = "Ehr"),
        ("time_interval" = Option<String>, Query,
         description = "The optional SM `Interval<Iso8601_date_time>` as \
                        `<lower>/<upper>`, matched CLOSED against the \
                        CONTRIBUTION / version audit `time_committed`. Either \
                        bound may be empty for an open interval \
                        (`/2026-01-01T00:00:00Z`, `2020-01-01T00:00:00Z/`); an \
                        absent parameter is the fully open interval. A pair \
                        bounded on BOTH sides must satisfy `lower <= upper` \
                        (BASE `Interval` invariant `Limits_consistent`).",
         example = "2020-01-01T00:00:00Z/2026-12-31T00:00:00Z")
    ),
    responses(
        (status = 200, description = "The matching CONTRIBUTION ids, ordered by \
                                      commit time then id. A service that is \
                                      not a versioned-content service yields \
                                      `[]`.",
         body = Vec<String>,
         example = json!(["8849182c-82ad-4088-a07f-48ead4180515"])),
        (status = 400, description = "`a_service` is absent or names no \
                                      `PLATFORM_SERVICE` member, `time_interval` \
                                      is not `<lower>/<upper>`, a bound is not a \
                                      valid ISO 8601 date-time, or the bounded \
                                      pair has its lower bound AFTER its upper \
                                      bound (BASE `Interval` invariant \
                                      `Limits_consistent`) — SM \
                                      `precondition_violation`.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released admin operations carry \
                                      `security: []` and declare no such \
                                      branch.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class \
                                      (`OperationClass::Admin`, keyed off the \
                                      `/admin/` path). Our own authorization \
                                      design.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_report_contributions(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_report_contributions", parts, dispatch).await
}

/// Count the CONTRIBUTIONs of one platform service
/// (`GET /admin/report/contribution/count`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_ADMIN_SERVICE.contribution_count`. The SM
/// return is an `Integer`, so the body is a bare JSON number — the count IS the
/// resource, and an object wrapper would invent a schema no spec defines.
#[utoipa::path(
    get, path = "/admin/report/contribution/count", tag = "admin-report",
    params(
        ("a_service" = String, Query,
         description = "The `PLATFORM_SERVICE` member naming the \
                        versioned-content service to report on — one of \
                        `Admin`, `Definitions`, `Ehr`, `Ehr_index`, \
                        `Demographic`, `Message`, `Query`, `System_log` (SM \
                        `platform_service.adoc`; matched case-insensitively). \
                        A service that holds no versioned content reports \
                        empty/zero rather than failing.",
         example = "Ehr"),
        ("time_interval" = Option<String>, Query,
         description = "The optional SM `Interval<Iso8601_date_time>` as \
                        `<lower>/<upper>`, matched CLOSED against the \
                        CONTRIBUTION / version audit `time_committed`. Either \
                        bound may be empty for an open interval \
                        (`/2026-01-01T00:00:00Z`, `2020-01-01T00:00:00Z/`); an \
                        absent parameter is the fully open interval. A pair \
                        bounded on BOTH sides must satisfy `lower <= upper` \
                        (BASE `Interval` invariant `Limits_consistent`).",
         example = "2020-01-01T00:00:00Z/2026-12-31T00:00:00Z")
    ),
    responses(
        (status = 200, description = "The number of matching CONTRIBUTIONs, as \
                                      a bare JSON number (`0` for a service \
                                      that holds no versioned content).",
         body = i64, example = json!(0)),
        (status = 400, description = "`a_service` is absent or names no \
                                      `PLATFORM_SERVICE` member, `time_interval` \
                                      is not `<lower>/<upper>`, a bound is not a \
                                      valid ISO 8601 date-time, or the bounded \
                                      pair has its lower bound AFTER its upper \
                                      bound (BASE `Interval` invariant \
                                      `Limits_consistent`) — SM \
                                      `precondition_violation`.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released admin operations carry \
                                      `security: []` and declare no such \
                                      branch.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class \
                                      (`OperationClass::Admin`, keyed off the \
                                      `/admin/` path). Our own authorization \
                                      design.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_report_contribution_count(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_report_contribution_count", parts, dispatch).await
}

/// Count the `VERSIONED_COMPOSITION`s of one platform service
/// (`GET /admin/report/versioned_composition/count`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM
/// `I_ADMIN_SERVICE.versioned_composition_count` — distinct version
/// CONTAINERS, as against the individual versions its sibling counts.
#[utoipa::path(
    get, path = "/admin/report/versioned_composition/count", tag = "admin-report",
    params(
        ("a_service" = String, Query,
         description = "The `PLATFORM_SERVICE` member naming the \
                        versioned-content service to report on — one of \
                        `Admin`, `Definitions`, `Ehr`, `Ehr_index`, \
                        `Demographic`, `Message`, `Query`, `System_log` (SM \
                        `platform_service.adoc`; matched case-insensitively). \
                        A service that holds no versioned content reports \
                        empty/zero rather than failing.",
         example = "Ehr"),
        ("time_interval" = Option<String>, Query,
         description = "The optional SM `Interval<Iso8601_date_time>` as \
                        `<lower>/<upper>`, matched CLOSED against the \
                        CONTRIBUTION / version audit `time_committed`. Either \
                        bound may be empty for an open interval \
                        (`/2026-01-01T00:00:00Z`, `2020-01-01T00:00:00Z/`); an \
                        absent parameter is the fully open interval. A pair \
                        bounded on BOTH sides must satisfy `lower <= upper` \
                        (BASE `Interval` invariant `Limits_consistent`).",
         example = "2020-01-01T00:00:00Z/2026-12-31T00:00:00Z")
    ),
    responses(
        (status = 200, description = "The number of distinct COMPOSITION \
                                      version containers with a version \
                                      committed in the interval, as a bare JSON \
                                      number. COMPOSITIONs are EHR-scoped, so \
                                      only `a_service=Ehr` can be non-zero.",
         body = i64, example = json!(0)),
        (status = 400, description = "`a_service` is absent or names no \
                                      `PLATFORM_SERVICE` member, `time_interval` \
                                      is not `<lower>/<upper>`, a bound is not a \
                                      valid ISO 8601 date-time, or the bounded \
                                      pair has its lower bound AFTER its upper \
                                      bound (BASE `Interval` invariant \
                                      `Limits_consistent`) — SM \
                                      `precondition_violation`.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released admin operations carry \
                                      `security: []` and declare no such \
                                      branch.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class \
                                      (`OperationClass::Admin`, keyed off the \
                                      `/admin/` path). Our own authorization \
                                      design.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_report_versioned_composition_count(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "admin_report_versioned_composition_count",
        parts,
        dispatch,
    )
    .await
}

/// Count the individual COMPOSITION versions of one platform service
/// (`GET /admin/report/composition_version/count`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM
/// `I_ADMIN_SERVICE.composition_version_count`.
#[utoipa::path(
    get, path = "/admin/report/composition_version/count", tag = "admin-report",
    params(
        ("a_service" = String, Query,
         description = "The `PLATFORM_SERVICE` member naming the \
                        versioned-content service to report on — one of \
                        `Admin`, `Definitions`, `Ehr`, `Ehr_index`, \
                        `Demographic`, `Message`, `Query`, `System_log` (SM \
                        `platform_service.adoc`; matched case-insensitively). \
                        A service that holds no versioned content reports \
                        empty/zero rather than failing.",
         example = "Ehr"),
        ("time_interval" = Option<String>, Query,
         description = "The optional SM `Interval<Iso8601_date_time>` as \
                        `<lower>/<upper>`, matched CLOSED against the \
                        CONTRIBUTION / version audit `time_committed`. Either \
                        bound may be empty for an open interval \
                        (`/2026-01-01T00:00:00Z`, `2020-01-01T00:00:00Z/`); an \
                        absent parameter is the fully open interval. A pair \
                        bounded on BOTH sides must satisfy `lower <= upper` \
                        (BASE `Interval` invariant `Limits_consistent`).",
         example = "2020-01-01T00:00:00Z/2026-12-31T00:00:00Z")
    ),
    responses(
        (status = 200, description = "The number of individual COMPOSITION \
                                      version rows committed in the interval, \
                                      as a bare JSON number. COMPOSITIONs are \
                                      EHR-scoped, so only `a_service=Ehr` can \
                                      be non-zero.",
         body = i64, example = json!(0)),
        (status = 400, description = "`a_service` is absent or names no \
                                      `PLATFORM_SERVICE` member, `time_interval` \
                                      is not `<lower>/<upper>`, a bound is not a \
                                      valid ISO 8601 date-time, or the bounded \
                                      pair has its lower bound AFTER its upper \
                                      bound (BASE `Interval` invariant \
                                      `Limits_consistent`) — SM \
                                      `precondition_violation`.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released admin operations carry \
                                      `security: []` and declare no such \
                                      branch.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class \
                                      (`OperationClass::Admin`, keyed off the \
                                      `/admin/` path). Our own authorization \
                                      design.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_report_composition_version_count(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(
        state,
        "admin_report_composition_version_count",
        parts,
        dispatch,
    )
    .await
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
    // The whole ADMIN group is opt-in; the gate and its two grounds are stated
    // once, in the group dispatcher.
    if let Some(refusal) = super::dispatch::admin_group_gate(&state) {
        return Ok(refusal);
    }
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let a_service = platform_service(q)?;
    let range = time_interval(q)?;

    match op {
        "admin_report_contributions" => {
            let ids = state
                .backend()
                .admin_list_contributions(a_service, range)
                .await?;
            Ok(negotiate::respond(h, StatusCode::OK, &ids))
        }
        "admin_report_contribution_count" => {
            let count = state
                .backend()
                .admin_contribution_count(a_service, range)
                .await?;
            Ok(negotiate::respond(h, StatusCode::OK, &count))
        }
        "admin_report_versioned_composition_count" => {
            let count = state
                .backend()
                .versioned_composition_count(a_service, range)
                .await?;
            Ok(negotiate::respond(h, StatusCode::OK, &count))
        }
        "admin_report_composition_version_count" => {
            let count = state
                .backend()
                .composition_version_count(a_service, range)
                .await?;
            Ok(negotiate::respond(h, StatusCode::OK, &count))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted admin report operation: {other}"
        )))),
    }
}

/// Read the mandatory `a_service` query parameter as a `PLATFORM_SERVICE`
/// member; absent or unknown → `400` (SM `precondition_violation`).
fn platform_service(query: Option<&str>) -> Result<PlatformService, RestError> {
    let raw = params::query_param(query, "a_service").ok_or_else(|| {
        RestError(ApiError::BadRequest(
            "query parameter `a_service` is required (a PLATFORM_SERVICE member)".to_owned(),
        ))
    })?;
    raw.parse::<PlatformService>().map_err(|()| {
        RestError(ApiError::BadRequest(format!(
            "query parameter `a_service` names no PLATFORM_SERVICE member: {raw:?}"
        )))
    })
}

/// Read the optional `time_interval` query parameter as the SM
/// `Interval<Iso8601_date_time>` bound pair. The wire form is
/// `<lower>/<upper>` with either side empty for an open bound; the bound VALUES
/// are validated by the service (SM `precondition_violation` on a malformed
/// ISO 8601 date-time), so this only splits the pair.
///
/// A `/`-less value is rejected here rather than silently read as a lower
/// bound: an interval that names one instant with no separator is not an
/// interval, and guessing which bound was meant would be a silent wrong answer.
fn time_interval(query: Option<&str>) -> Result<StatTimeRange, RestError> {
    let Some(raw) = params::query_param(query, "time_interval") else {
        return Ok(None);
    };
    let (lower, upper) = raw.split_once('/').ok_or_else(|| {
        RestError(ApiError::BadRequest(format!(
            "query parameter `time_interval` must be `<lower>/<upper>` (either bound may be \
             empty), got {raw:?}"
        )))
    })?;
    Ok(Some((bound(lower), bound(upper))))
}

/// One interval bound: an empty side is an OPEN bound (`None`).
fn bound(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
