// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **activity-report** wire — **our own extension**.
//!
//! No openEHR ITS-REST operation governs any route here. The released Admin API
//! is exactly two operations, both EHR deletes
//! (`specifications/admin.openapi.yaml` → `operations/admin_ehr_delete.yaml` +
//! `operations/admin_ehr_delete_all.yaml`). The four reporting calls here
//! (`list_contributions`, `contribution_count`, `versioned_composition_count`,
//! `composition_version_count`) each take the platform service to report on
//! (`a_service`) and an optional time interval.
//!
//! These routes are **excluded from ITS-REST wire conformance**: they gate the
//! `ActivityReport` CAPABILITY verdict only.
//!
//! Gating: mounted under `/admin/`, so they inherit the group's two gates
//! unchanged — the coarse RBAC `OperationClass::Admin` classifier (`401`
//! unauthenticated / `403` non-admin, our own authorization design; the
//! released admin operations carry `security: []`) and the
//! `AppConfig::api.admin` config gate, which answers `405` with an empty
//! `Allow` while the group is off (`crate::api::admin::dispatch`, whose ground
//! is the overview rule "If a method is recognized but not allowed for the
//! target resource, the response SHOULD be `405 Method Not Allowed` status
//! code" — `docs/overview/Requests_and_responses.md` §"HTTP Methods").

use axum::response::Response;
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use ferroehr::service::admin::types::StatTimeRange;
use ferroehr::service::platform_service::PlatformService;
use ferroehr::storage::statistics::CompositionCount;

use super::body::{AdminDisabled, AdminUnauthenticated, NotAdminClass};
use crate::api::{RequestParts, guarded_handler};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::{negotiate, params};

/// The activity-report extension routes as a native `utoipa-axum` router —
/// **no ITS-REST contract** (see the module docs). Group-relative paths
/// (nested under the extensions root); every operation runs through
/// [`guarded_dispatch`](crate::api::guarded_dispatch) onto its handler.
pub(crate) fn report_routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (handlers in a single call must share the path).
    OpenApiRouter::new()
        .routes(routes!(admin_report_contributions))
        .routes(routes!(admin_report_contribution_count))
        .routes(routes!(admin_report_versioned_composition_count))
        .routes(routes!(admin_report_composition_version_count))
}

guarded_handler! {
    /// List the CONTRIBUTION ids of one platform service
    /// (`GET /admin/report/contribution`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module
    /// docs).
    #[utoipa::path(
        get, path = "/admin/report/contribution", tag = "admin-report",
        params(
            ("a_service" = String, Query,
             description = "The platform service naming the \
                            versioned-content service to report on — one of \
                            `Admin`, `Definitions`, `Ehr`, `Ehr_index`, \
                            `Demographic`, `Message`, `Query`, `System_log`, \
                            matched case-insensitively. \
                            A service that holds no versioned content reports \
                            empty/zero rather than failing.",
             example = "Ehr"),
            ("time_interval" = Option<String>, Query,
             description = "The optional time interval as \
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
                                          platform service, `time_interval` \
                                          is not `<lower>/<upper>`, a bound is not a \
                                          valid ISO 8601 date-time, or the bounded \
                                          pair has its lower bound AFTER its upper \
                                          bound (BASE `Interval` invariant \
                                          `Limits_consistent`).",
             body = serde_json::Value),
            AdminUnauthenticated,
            NotAdminClass,
            AdminDisabled
        )
    )]
    admin_report_contributions => |state, parts| super::body::gated(state, parts, contributions)
}

guarded_handler! {
    /// Count the CONTRIBUTIONs of one platform service
    /// (`GET /admin/report/contribution/count`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module
    /// docs). The body is a bare JSON number — the count IS the
    /// resource, and an object wrapper would invent a schema no spec defines.
    #[utoipa::path(
        get, path = "/admin/report/contribution/count", tag = "admin-report",
        params(
            ("a_service" = String, Query,
             description = "The platform service naming the \
                            versioned-content service to report on — one of \
                            `Admin`, `Definitions`, `Ehr`, `Ehr_index`, \
                            `Demographic`, `Message`, `Query`, `System_log`, \
                            matched case-insensitively. \
                            A service that holds no versioned content reports \
                            empty/zero rather than failing.",
             example = "Ehr"),
            ("time_interval" = Option<String>, Query,
             description = "The optional time interval as \
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
                                          platform service, `time_interval` \
                                          is not `<lower>/<upper>`, a bound is not a \
                                          valid ISO 8601 date-time, or the bounded \
                                          pair has its lower bound AFTER its upper \
                                          bound (BASE `Interval` invariant \
                                          `Limits_consistent`).",
             body = serde_json::Value),
            AdminUnauthenticated,
            NotAdminClass,
            AdminDisabled
        )
    )]
    admin_report_contribution_count
        => |state, parts| super::body::gated(state, parts, contribution_count)
}

guarded_handler! {
    /// Count the `VERSIONED_COMPOSITION`s of one platform service
    /// (`GET /admin/report/versioned_composition/count`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module
    /// docs). It counts distinct version
    /// CONTAINERS, as against the individual versions its sibling counts.
    #[utoipa::path(
        get, path = "/admin/report/versioned_composition/count", tag = "admin-report",
        params(
            ("a_service" = String, Query,
             description = "The platform service naming the \
                            versioned-content service to report on — one of \
                            `Admin`, `Definitions`, `Ehr`, `Ehr_index`, \
                            `Demographic`, `Message`, `Query`, `System_log`, \
                            matched case-insensitively. \
                            A service that holds no versioned content reports \
                            empty/zero rather than failing.",
             example = "Ehr"),
            ("time_interval" = Option<String>, Query,
             description = "The optional time interval as \
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
                                          platform service, `time_interval` \
                                          is not `<lower>/<upper>`, a bound is not a \
                                          valid ISO 8601 date-time, or the bounded \
                                          pair has its lower bound AFTER its upper \
                                          bound (BASE `Interval` invariant \
                                          `Limits_consistent`).",
             body = serde_json::Value),
            AdminUnauthenticated,
            NotAdminClass,
            AdminDisabled
        )
    )]
    admin_report_versioned_composition_count
        => |state, parts| super::body::gated(state, parts, versioned_composition_count)
}

guarded_handler! {
    /// Count the individual COMPOSITION versions of one platform service
    /// (`GET /admin/report/composition_version/count`).
    ///
    /// **Our own extension — no ITS-REST operation governs this** (module
    /// docs).
    #[utoipa::path(
        get, path = "/admin/report/composition_version/count", tag = "admin-report",
        params(
            ("a_service" = String, Query,
             description = "The platform service naming the \
                            versioned-content service to report on — one of \
                            `Admin`, `Definitions`, `Ehr`, `Ehr_index`, \
                            `Demographic`, `Message`, `Query`, `System_log`, \
                            matched case-insensitively. \
                            A service that holds no versioned content reports \
                            empty/zero rather than failing.",
             example = "Ehr"),
            ("time_interval" = Option<String>, Query,
             description = "The optional time interval as \
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
                                          platform service, `time_interval` \
                                          is not `<lower>/<upper>`, a bound is not a \
                                          valid ISO 8601 date-time, or the bounded \
                                          pair has its lower bound AFTER its upper \
                                          bound (BASE `Interval` invariant \
                                          `Limits_consistent`).",
             body = serde_json::Value),
            AdminUnauthenticated,
            NotAdminClass,
            AdminDisabled
        )
    )]
    admin_report_composition_version_count
        => |state, parts| super::body::gated(state, parts, composition_version_count)
}

// ── operations ────────────────────────────────────────────────────────────────

/// The `admin_report_contributions` operation.
pub(crate) async fn contributions(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let a_service = platform_service(q)?;
    let range = time_interval(q)?;
    let ids = state
        .backend()
        .admin_list_contributions(a_service, range)
        .await?;
    Ok(negotiate::body::respond(h, StatusCode::OK, &ids))
}

/// The `admin_report_contribution_count` operation.
pub(crate) async fn contribution_count(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let a_service = platform_service(q)?;
    let range = time_interval(q)?;
    let count = state
        .backend()
        .admin_contribution_count(a_service, range)
        .await?;
    Ok(negotiate::body::respond(h, StatusCode::OK, &count))
}

/// The `admin_report_versioned_composition_count` operation.
pub(crate) async fn versioned_composition_count(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let a_service = platform_service(q)?;
    let range = time_interval(q)?;
    let count = state
        .backend()
        .composition_count(a_service, range, CompositionCount::VersionedObjects)
        .await?;
    Ok(negotiate::body::respond(h, StatusCode::OK, &count))
}

/// The `admin_report_composition_version_count` operation.
pub(crate) async fn composition_version_count(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let a_service = platform_service(q)?;
    let range = time_interval(q)?;
    let count = state
        .backend()
        .composition_count(a_service, range, CompositionCount::Versions)
        .await?;
    Ok(negotiate::body::respond(h, StatusCode::OK, &count))
}

/// Read the mandatory `a_service` query parameter as a [`PlatformService`];
/// absent or unknown → `400`.
fn platform_service(query: Option<&str>) -> Result<PlatformService, RestError> {
    let members = PlatformService::ALL
        .iter()
        .map(|member| member.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let raw = params::query_param(query, "a_service").ok_or_else(|| {
        RestError::from(ApiError::BadRequest(format!(
            "query parameter `a_service` is required: one of {members}"
        )))
    })?;
    raw.parse::<PlatformService>().map_err(|()| {
        RestError::from(ApiError::BadRequest(format!(
            "query parameter `a_service` names no platform service: {raw:?} \
             (expected one of {members})"
        )))
    })
}

/// Read the optional `time_interval` query parameter as an ISO 8601 date-time
/// bound pair. The wire form is `<lower>/<upper>` with either side empty for
/// an open bound; the bound VALUES are validated by the service (a `400` on a
/// malformed ISO 8601 date-time), so this only splits the pair.
///
/// A `/`-less value is rejected here rather than silently read as a lower
/// bound: an interval that names one instant with no separator is not an
/// interval, and guessing which bound was meant would be a silent wrong answer.
fn time_interval(query: Option<&str>) -> Result<StatTimeRange, RestError> {
    let Some(raw) = params::query_param(query, "time_interval") else {
        return Ok(None);
    };
    let (lower, upper) = raw.split_once('/').ok_or_else(|| {
        RestError::from(ApiError::BadRequest(format!(
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
