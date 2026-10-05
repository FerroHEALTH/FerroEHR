// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **archive** wire — **our own extension**.
//!
//! No openEHR ITS-REST operation governs any route here. The released Admin
//! API is exactly two EHR deletes (`specifications/admin.openapi.yaml`), while
//! the SM declares an archive interface the release never surfaced —
//! `docs/specs/openehr/SM/docs/UML/classes/i_admin_archive.adoc`:
//! `archive_ehrs` ("Move selected EHRs to archival storage") and
//! `archive_parties` ("Move selected Parties and relationships to archival
//! storage"), with `ehr_id_does_not_exist` / `party_id_does_not_exist` as their
//! declared errors. No released ITS-REST operation covers either call.
//!
//! These routes are the honest realization of that service basis, and are
//! **excluded from ITS-REST wire conformance**: they gate the `EhrArchive` /
//! `DemographicArchive` CAPABILITY verdicts only.
//!
//! Every route is all-or-nothing: each id in the list is existence-checked
//! before anything is written, so an unknown id leaves the repository untouched
//! (the SM declares the not-found error on the operation, not per element).
//! Archiving marks the objects archived AND physically moves their rows to the
//! server's cold storage tier; archival stays read-neutral — the archived
//! objects remain retrievable, served from that tier — so no resource
//! representation changes and the success is a bodyless `204`.
//!
//! The two `…/restore` routes are the reverse movement, and they realize NO SM
//! operation: `i_admin_archive.adoc` declares the two archive calls and no
//! un-archive counterpart, so naming one would be a false claim. They are our
//! own design end to end — an archival tier is only trustworthy if the move it
//! performs can be undone deliberately rather than only as the side effect of a
//! write (which thaws the object it touches). Restoring is idempotent in the
//! same way archiving is: an object with nothing archived restores nothing and
//! succeeds.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`401`/`403`, our own authorization design) and the
//! `AppConfig::admin.enabled` config gate (`405` with an empty `Allow` when
//! off) unchanged.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The archive extension routes as a native `utoipa-axum` router — **no
/// ITS-REST contract** (see the module docs). Group-relative paths (nested
/// under `base_path`); every operation runs through [`guarded_dispatch`] with
/// [`dispatch`].
pub(crate) fn archive_routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (handlers in a single call must share the path).
    OpenApiRouter::new()
        .routes(routes!(admin_archive_ehrs))
        .routes(routes!(admin_archive_parties))
        .routes(routes!(admin_restore_archived_ehrs))
        .routes(routes!(admin_restore_archived_parties))
}

/// Move selected EHRs to archival storage (`POST /admin/archive/ehrs`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_ADMIN_ARCHIVE.archive_ehrs`.
#[utoipa::path(
    post, path = "/admin/archive/ehrs", tag = "admin-archive",
    request_body(content = serde_json::Value,
                 description = "`{ \"ehr_ids\": [ … ] }` — the EHR ids to \
                                archive, as the SM `List<String>` parameter. \
                                An empty list archives nothing and succeeds.",
                 example = json!({ "ehr_ids": ["7d44b88c-4199-4bad-97dc-d78268e01398"] })),
    responses(
        (status = 204, description = "Every named EHR is marked archived \
                                      (idempotent — re-archiving an archived \
                                      EHR is a no-op). No body: the SM \
                                      operation returns nothing and archival is \
                                      read-neutral, so no representation \
                                      changed."),
        (status = 400, description = "The body is not `{ \"ehr_ids\": [ … ] }`, \
                                      or an id is not a well-formed UUID — SM \
                                      `precondition_violation`. The whole \
                                      request is rejected before anything is \
                                      archived.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released admin operations carry \
                                      `security: []`.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class \
                                      (`OperationClass::Admin`, keyed off the \
                                      `/admin/` path). Our own authorization \
                                      design.",
         body = serde_json::Value),
        (status = 404, description = "An id in the list names no EHR — SM \
                                      `ehr_id_does_not_exist`. Nothing is \
                                      archived (all-or-nothing).",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_archive_ehrs(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_archive_ehrs", parts, dispatch).await
}

/// Move selected parties to archival storage (`POST /admin/archive/parties`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_ADMIN_ARCHIVE.archive_parties`.
#[utoipa::path(
    post, path = "/admin/archive/parties", tag = "admin-archive",
    request_body(content = serde_json::Value,
                 description = "`{ \"party_ids\": [ … ] }` — the demographic \
                                PARTY version-container ids to archive, as the \
                                SM `List<String>` parameter. An empty list \
                                archives nothing and succeeds.",
                 example = json!({ "party_ids": ["8849182c-82ad-4088-a07f-48ead4180515"] })),
    responses(
        (status = 204, description = "Every named party is marked archived \
                                      (idempotent). No body — as for the EHR \
                                      half."),
        (status = 400, description = "The body is not `{ \"party_ids\": [ … ] }`, \
                                      or an id is not a well-formed UUID — SM \
                                      `precondition_violation`. The whole \
                                      request is rejected before anything is \
                                      archived.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class. \
                                      Our own authorization design.",
         body = serde_json::Value),
        (status = 404, description = "An id in the list names no demographic \
                                      PARTY root — SM `party_id_does_not_exist` \
                                      (a `PARTY_RELATIONSHIP` id is not a party \
                                      root and is refused the same way). Nothing \
                                      is archived.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_archive_parties(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_archive_parties", parts, dispatch).await
}

/// Bring selected EHRs back from archival storage
/// (`POST /admin/archive/ehrs/restore`).
///
/// **Our own extension — no ITS-REST operation governs this, and it realizes no
/// SM operation either** (module docs): `i_admin_archive.adoc` declares
/// `archive_ehrs` / `archive_parties` and no un-archive counterpart. The reverse
/// of [`admin_archive_ehrs`], addressed as an action on the archive itself
/// because a `DELETE` carrying the id list would be the one method whose
/// "content … has no generally defined semantics" (RFC 9110 §9.3.5), while
/// `POST` is exactly "providing a block of data … to a data-handling process"
/// (RFC 9110 §9.3.3).
#[utoipa::path(
    post, path = "/admin/archive/ehrs/restore", tag = "admin-archive",
    request_body(content = serde_json::Value,
                 description = "`{ \"ehr_ids\": [ … ] }` — the EHR ids to bring \
                                back from archival storage, in the same shape \
                                the archive route takes. An empty list restores \
                                nothing and succeeds.",
                 example = json!({ "ehr_ids": ["7d44b88c-4199-4bad-97dc-d78268e01398"] })),
    responses(
        (status = 204, description = "Every archived versioned object of every \
                                      named EHR is back in the primary storage \
                                      tier and its archive marker is gone \
                                      (idempotent — an EHR with nothing \
                                      archived restores nothing and succeeds). \
                                      No body: like archiving, restoring is a \
                                      move, and reads were already served \
                                      unchanged from the archival tier, so no \
                                      representation changed. What DOES change \
                                      is AQL visibility — the query engine reads \
                                      the primary tier only, so a restored \
                                      record appears in query results again."),
        (status = 400, description = "The body is not `{ \"ehr_ids\": [ … ] }`, \
                                      or an id is not a well-formed UUID. The \
                                      whole request is rejected before anything \
                                      is restored.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design \
                                      — the released admin operations carry \
                                      `security: []`.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class \
                                      (`OperationClass::Admin`, keyed off the \
                                      `/admin/` path). Our own authorization \
                                      design.",
         body = serde_json::Value),
        (status = 404, description = "An id in the list names no EHR. Nothing is \
                                      restored (all-or-nothing) — the same \
                                      existence check the archive half applies.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_restore_archived_ehrs(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_restore_archived_ehrs", parts, dispatch).await
}

/// Bring selected parties back from archival storage
/// (`POST /admin/archive/parties/restore`).
///
/// **Our own extension — no ITS-REST operation governs this, and it realizes no
/// SM operation either** (module docs). The reverse of
/// [`admin_archive_parties`], with the same action-on-the-archive shape its EHR
/// twin above documents.
#[utoipa::path(
    post, path = "/admin/archive/parties/restore", tag = "admin-archive",
    request_body(content = serde_json::Value,
                 description = "`{ \"party_ids\": [ … ] }` — the demographic \
                                PARTY version-container ids to bring back from \
                                archival storage, in the same shape the archive \
                                route takes. An empty list restores nothing and \
                                succeeds.",
                 example = json!({ "party_ids": ["8849182c-82ad-4088-a07f-48ead4180515"] })),
    responses(
        (status = 204, description = "Every named party's archived versioned \
                                      object is back in the primary storage \
                                      tier and its archive marker is gone \
                                      (idempotent). No body — as for the EHR \
                                      half, whose declaration states the AQL \
                                      visibility effect."),
        (status = 400, description = "The body is not `{ \"party_ids\": [ … ] }`, \
                                      or an id is not a well-formed UUID. The \
                                      whole request is rejected before anything \
                                      is restored.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated (auth enabled, no valid \
                                      principal). Our own authorization design.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class. \
                                      Our own authorization design.",
         body = serde_json::Value),
        (status = 404, description = "An id in the list names no demographic \
                                      PARTY root (a `PARTY_RELATIONSHIP` id is \
                                      not a party root and is refused the same \
                                      way). The check spans BOTH storage tiers, \
                                      so an archived party is found and \
                                      restored, never reported missing. Nothing \
                                      is restored.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_restore_archived_parties(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_restore_archived_parties", parts, dispatch).await
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

    match op {
        "admin_archive_ehrs" => {
            let ids = id_list(&negotiate::json_value(h, &parts.body)?, "ehr_ids")?;
            state.backend().archive_ehrs(ids).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_archive_parties" => {
            let ids = id_list(&negotiate::json_value(h, &parts.body)?, "party_ids")?;
            state.backend().archive_parties(ids).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_restore_archived_ehrs" => {
            let ids = id_list(&negotiate::json_value(h, &parts.body)?, "ehr_ids")?;
            state.backend().restore_archived_ehrs(ids).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_restore_archived_parties" => {
            let ids = id_list(&negotiate::json_value(h, &parts.body)?, "party_ids")?;
            state.backend().restore_archived_parties(ids).await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted admin archive operation: {other}"
        )))),
    }
}

/// Read the `{ "<field>": [ … ] }` id list out of the request body. A missing
/// or non-array member, or a non-string element, is a `400` — the SM parameter
/// is a `List<String>`, and guessing a shape would archive the wrong set. The
/// id VALUES are validated by the service (`precondition_violation` on a
/// malformed UUID), which rejects the whole request before writing anything.
fn id_list(body: &serde_json::Value, field: &str) -> Result<Vec<String>, RestError> {
    let items = body
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            RestError(ApiError::BadRequest(format!(
                "request body must be an object with a `{field}` array of ids"
            )))
        })?;
    items
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                RestError(ApiError::BadRequest(format!(
                    "every `{field}` element must be a string id, got {item}"
                )))
            })
        })
        .collect()
}
