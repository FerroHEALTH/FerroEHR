// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **legal marks** wire — **our own extension**.
//!
//! No openEHR ITS-REST operation and no SM interface governs any route here.
//! The released Admin API is two EHR deletes
//! (`docs/specs/openehr/ITS-REST/specifications/admin.openapi.yaml`), and the
//! SM's admin interfaces are the statistics, archive and dump/load calls; none
//! of them mentions restriction of processing, an objection to research, or a
//! retention period. The RM is equally silent, and deliberately so: the one
//! flag that looks like a fit, `EHR_STATUS.is_queryable`, limits population
//! queries and nothing else (RM ehr `master04-ehr_package.adoc` §EHR Status),
//! while access decisions run through `EHR_ACCESS` (§EHR Access) and content is
//! indelible (RM common `master06-change_control_package.adoc` §Logical
//! Deletion).
//!
//! What the routes serve is the law: GDPR Art. 18 (restriction), Art. 21(6)
//! (the research objection), Art. 5(1)(e) and Art. 30(1)(f) (retention), with
//! DSG Art. 25 Abs. 2 lit. d and EPDV Art. 10 beside them
//! (`docs/law/eu/gdpr/text.html`, `docs/law/ch/fadp/text-de.html`,
//! `docs/law/ch/epdv/text-de.html`). Setting a mark is the controller's act,
//! never the server's, and none of it deletes anything: the retention routes
//! answer what has fallen due and leave the decision where it belongs.
//!
//! Gating: mounted under `/admin/`, so every route inherits the group's RBAC
//! Admin class (`401`/`403`) and the `AppConfig::admin.enabled` config gate
//! (`405` with an empty `Allow` when off) unchanged.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the admin register bodies are genuinely open \
              operational JSON, not RM content"
)]

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::overview::error::RestError;
use crate::state::AppState;
use crate::{negotiate, params};

/// The legal-marks extension routes as a native `utoipa-axum` router — **no
/// ITS-REST contract** (see the module docs).
pub(crate) fn marks_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            admin_restrict_processing,
            admin_restriction_register
        ))
        .routes(routes!(admin_lift_restriction))
        .routes(routes!(admin_research_objection))
        .routes(routes!(
            admin_retention_policies,
            admin_put_retention_policy
        ))
        .routes(routes!(admin_retention_anchor))
        .routes(routes!(admin_retention_hold))
        .routes(routes!(admin_retention_due))
}

/// Record a restriction of processing (`POST /admin/restriction`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    post, path = "/admin/restriction", tag = "admin-marks",
    request_body(content = serde_json::Value,
                 description = "`{ \"ehr_id\": …, \"vo_id\": …?, \"ground\": …, \
                                \"note\": …? }`. Omitting `vo_id` restricts the \
                                whole EHR; `ground` is one of the four GDPR \
                                Art. 18(1) points or `national`.",
                 example = json!({
                     "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                     "ground": "gdpr-18-1-a",
                     "note": "accuracy contested on 2026-09-15"
                 })),
    responses(
        (status = 204, description = "The restriction is recorded and the marks \
                                      it implies are in force. No body: the \
                                      register is read back through \
                                      `GET /admin/restriction`."),
        (status = 400, description = "The body is not the object above, or the \
                                      ground is outside the closed list.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated. Our own authorization \
                                      design — the released admin operations \
                                      carry `security: []`.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class.",
         body = serde_json::Value),
        (status = 404, description = "The EHR is unknown, or the named \
                                      versioned object is not in it. Nothing is \
                                      recorded.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_restrict_processing(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_restrict_processing", parts, dispatch).await
}

/// Read one EHR's restriction register (`GET /admin/restriction`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    get, path = "/admin/restriction", tag = "admin-marks",
    params(("ehr_id" = String, Query, description = "The EHR whose register to read.")),
    responses(
        (status = 200, description = "The register rows, newest request first. \
                                      A lift is a row with `lifted_at` set, \
                                      never a deletion: GDPR Art. 18(3) makes \
                                      the sequence the obligation.",
         body = serde_json::Value),
        (status = 400, description = "`ehr_id` is missing or not a UUID.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 404, description = "The EHR is unknown.", body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_restriction_register(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_restriction_register", parts, dispatch).await
}

/// Lift a restriction (`POST /admin/restriction/lift`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    post, path = "/admin/restriction/lift", tag = "admin-marks",
    request_body(content = serde_json::Value,
                 description = "`{ \"ehr_id\": …, \"vo_id\": …? }` — the grain \
                                to lift. Omitting `vo_id` lifts the whole-EHR \
                                restrictions and leaves any object-scoped one \
                                standing.",
                 example = json!({ "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398" })),
    responses(
        (status = 204, description = "Every in-force restriction at that grain \
                                      carries a lift instant, and the marks are \
                                      recomputed. Idempotent: lifting nothing \
                                      succeeds. The controller informs the \
                                      subject BEFORE calling this \
                                      (GDPR Art. 18(3))."),
        (status = 400, description = "The body is not the object above.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 404, description = "The EHR is unknown, or the named object is \
                                      not in it.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_lift_restriction(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_lift_restriction", parts, dispatch).await
}

/// Record, override or withdraw the research objection
/// (`POST /admin/research-objection`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    post, path = "/admin/research-objection", tag = "admin-marks",
    request_body(content = serde_json::Value,
                 description = "`{ \"ehr_id\": …, \"objected\": true|false, \
                                \"ground\": …? }`. `objected: true` records the \
                                objection; `ground` beside it records the \
                                controller's Art. 21(6) public-interest \
                                override, which puts the EHR back in the \
                                research population and says on whose \
                                authority; `objected: false` withdraws the \
                                objection entirely.",
                 example = json!({
                     "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                     "objected": true
                 })),
    responses(
        (status = 204, description = "The mark is in force. While it stands, \
                                      the EHR is absent from population AQL, \
                                      from every Extract and from the event \
                                      stream; a read of the record for care is \
                                      unaffected, because Art. 21(6) reaches \
                                      research processing and not the care \
                                      record."),
        (status = 400, description = "The body is not the object above, or a \
                                      ground was given with a withdrawal.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 404, description = "The EHR is unknown.", body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_research_objection(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_research_objection", parts, dispatch).await
}

/// Read the retention register (`GET /admin/retention/policy`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    get, path = "/admin/retention/policy", tag = "admin-marks",
    responses(
        (status = 200, description = "Every declared period, with the content \
                                      category, the jurisdiction, the anchor \
                                      rule and the legal citation it rests on. \
                                      This is the register GDPR Art. 30(1)(f) \
                                      and DSG Art. 25 Abs. 2 lit. d are \
                                      answered from.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_retention_policies(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_retention_policies", parts, dispatch).await
}

/// Declare a retention period (`PUT /admin/retention/policy`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    put, path = "/admin/retention/policy", tag = "admin-marks",
    request_body(content = serde_json::Value,
                 description = "`{ \"kind\": …, \"jurisdiction\": …, \
                                \"period\": …, \"anchor\": …, \"source\": … }`. \
                                `kind` is COMPOSITION | EHR_STATUS | FOLDER | \
                                EHR, `period` a PostgreSQL interval, `anchor` \
                                one of last_commit | death | majority, and \
                                `source` the legal citation the period rests \
                                on.",
                 example = json!({
                     "kind": "COMPOSITION",
                     "jurisdiction": "CH",
                     "period": "20 years",
                     "anchor": "last_commit",
                     "source": "EPDV Art. 10 Abs. 1 lit. d"
                 })),
    responses(
        (status = 204, description = "The period is declared, replacing any \
                                      earlier one for that category and \
                                      jurisdiction."),
        (status = 400, description = "The body is not the object above, or the \
                                      register refused the category, the anchor \
                                      rule or a non-positive period.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_put_retention_policy(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_put_retention_policy", parts, dispatch).await
}

/// Record an EHR's retention anchor and any EHR-wide hold
/// (`PUT /admin/retention/anchor`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    put, path = "/admin/retention/anchor", tag = "admin-marks",
    request_body(content = serde_json::Value,
                 description = "`{ \"ehr_id\": …, \"jurisdiction\": …, \
                                \"anchored_at\": …?, \"hold_at\": …?, \
                                \"hold_ground\": …? }`. `anchored_at` stays \
                                absent until the anchor event is known to the \
                                deployment; a hold needs both its instant and \
                                its ground.",
                 example = json!({
                     "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                     "jurisdiction": "CH",
                     "anchored_at": "2026-01-31T00:00:00Z"
                 })),
    responses(
        (status = 204, description = "The anchor is recorded. While a hold is \
                                      set, the EHR is absent from the due list \
                                      whatever its period says."),
        (status = 400, description = "The body is not the object above, or the \
                                      register refused a hold instant with no \
                                      ground.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 404, description = "The EHR is unknown.", body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_retention_anchor(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_retention_anchor", parts, dispatch).await
}

/// Place or release a per-object retention hold
/// (`POST /admin/retention/hold`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    post, path = "/admin/retention/hold", tag = "admin-marks",
    request_body(content = serde_json::Value,
                 description = "`{ \"vo_id\": …, \"held\": true|false }` — the \
                                per-object exemption EPDV Art. 10 Abs. 2 lit. b \
                                lets a patient ask for.",
                 example = json!({
                     "vo_id": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21",
                     "held": true
                 })),
    responses(
        (status = 204, description = "The hold is in force (or released). A \
                                      held object is counted separately in the \
                                      due list and is never part of what a \
                                      disposal would cover."),
        (status = 400, description = "The body is not the object above.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 404, description = "No versioned object with that id.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_retention_hold(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_retention_hold", parts, dispatch).await
}

/// What has fallen due (`GET /admin/retention/due`).
///
/// **Our own extension — no ITS-REST operation governs this** (module docs).
#[utoipa::path(
    get, path = "/admin/retention/due", tag = "admin-marks",
    params(("limit" = Option<i64>, Query,
            description = "Maximum rows to return; 100 when absent.")),
    responses(
        (status = 200, description = "The EHRs whose period has run and which \
                                      carry no EHR-wide hold, oldest first, \
                                      each with the citation the period rests \
                                      on and the counts of objects due and \
                                      objects exempted by a per-object hold. \
                                      A LIST, never a disposal: the server \
                                      deletes no clinical content on a timer \
                                      (RM common master06 §Logical Deletion), \
                                      and acting on a row is the controller's \
                                      decision, carried out through the \
                                      physical delete.",
         body = serde_json::Value),
        (status = 400, description = "`limit` is not a positive number.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated.", body = serde_json::Value),
        (status = 403, description = "Not in the Admin class.", body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_retention_due(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_retention_due", parts, dispatch).await
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub(crate) fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        run(state, op, parts)
            .await
            .unwrap_or_else(IntoResponse::into_response)
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per operation, each three lines; splitting the table \
              would hide that the routes share one gate and one body reader"
)]
async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    if let Some(refusal) = super::dispatch::admin_group_gate(&state) {
        return Ok(refusal);
    }
    let h = &parts.headers;

    match op {
        "admin_restrict_processing" => {
            let body = negotiate::json_value(h, &parts.body)?;
            state
                .backend()
                .restrict_processing(
                    required_str(&body, "ehr_id")?,
                    optional_str(&body, "vo_id")?,
                    required_str(&body, "ground")?,
                    optional_str(&body, "note")?,
                )
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_restriction_register" => {
            let ehr_id =
                params::query_param(parts.query.as_deref(), "ehr_id").ok_or_else(|| {
                    RestError(ApiError::BadRequest(
                        "the `ehr_id` query parameter is required".to_owned(),
                    ))
                })?;
            let rows = state.backend().restriction_register(&ehr_id).await?;
            let body: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "id": row.id,
                        "ehr_id": row.ehr_id.to_string(),
                        "vo_id": row.vo_id.map(|id| id.to_string()),
                        "ground": row.ground,
                        "requested_at": row.requested_at.to_string(),
                        "lifted_at": row.lifted_at.map(|at| at.to_string()),
                        "note": row.note,
                    })
                })
                .collect();
            Ok(negotiate::respond(
                h,
                StatusCode::OK,
                &serde_json::json!({ "restrictions": body }),
            ))
        }
        "admin_lift_restriction" => {
            let body = negotiate::json_value(h, &parts.body)?;
            state
                .backend()
                .lift_restriction(
                    required_str(&body, "ehr_id")?,
                    optional_str(&body, "vo_id")?,
                )
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_research_objection" => {
            let body = negotiate::json_value(h, &parts.body)?;
            let objected = body
                .get("objected")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| {
                    RestError(ApiError::BadRequest(
                    "`objected` must be a boolean: true records the objection, false withdraws it"
                        .to_owned(),
                ))
                })?;
            state
                .backend()
                .set_research_objection(
                    required_str(&body, "ehr_id")?,
                    objected,
                    optional_str(&body, "ground")?,
                )
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_retention_policies" => {
            let rows = state.backend().retention_policies().await?;
            let body: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "kind": row.kind,
                        "jurisdiction": row.jurisdiction,
                        "period": row.period,
                        "anchor": row.anchor,
                        "source": row.source,
                    })
                })
                .collect();
            Ok(negotiate::respond(
                h,
                StatusCode::OK,
                &serde_json::json!({ "policies": body }),
            ))
        }
        "admin_put_retention_policy" => {
            let body = negotiate::json_value(h, &parts.body)?;
            state
                .backend()
                .put_retention_policy(
                    required_str(&body, "kind")?,
                    required_str(&body, "jurisdiction")?,
                    required_str(&body, "period")?,
                    required_str(&body, "anchor")?,
                    required_str(&body, "source")?,
                )
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_retention_anchor" => {
            let body = negotiate::json_value(h, &parts.body)?;
            let hold = match (
                optional_str(&body, "hold_at")?,
                optional_str(&body, "hold_ground")?,
            ) {
                (Some(at), Some(ground)) => Some((at, ground)),
                (None, None) => None,
                _ => {
                    return Err(RestError(ApiError::BadRequest(
                        "a retention hold needs both `hold_at` and `hold_ground`: a hold with \
                         no stated reason is not a record of anything"
                            .to_owned(),
                    )));
                }
            };
            state
                .backend()
                .put_retention_anchor(
                    required_str(&body, "ehr_id")?,
                    required_str(&body, "jurisdiction")?,
                    optional_str(&body, "anchored_at")?,
                    hold,
                )
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_retention_hold" => {
            let body = negotiate::json_value(h, &parts.body)?;
            let held = body
                .get("held")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| {
                    RestError(ApiError::BadRequest(
                        "`held` must be a boolean: true places the hold, false releases it"
                            .to_owned(),
                    ))
                })?;
            state
                .backend()
                .set_retention_hold(required_str(&body, "vo_id")?, held)
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        "admin_retention_due" => {
            let limit = match params::query_param(parts.query.as_deref(), "limit") {
                Some(raw) => raw.parse::<i64>().map_err(|_e| {
                    RestError(ApiError::BadRequest(format!(
                        "`limit` must be a positive whole number, got {raw:?}"
                    )))
                })?,
                None => 100,
            };
            let rows = state.backend().retention_due(limit).await?;
            let body: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "ehr_id": row.ehr_id.to_string(),
                        "jurisdiction": row.jurisdiction,
                        "kind": row.kind,
                        "source": row.source,
                        "due_at": row.due_at.to_string(),
                        "objects_due": row.objects_due,
                        "objects_held": row.objects_held,
                    })
                })
                .collect();
            Ok(negotiate::respond(
                h,
                StatusCode::OK,
                &serde_json::json!({ "due": body }),
            ))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted admin marks operation: {other}"
        )))),
    }
}

/// A required string member of the request body.
///
/// A missing or non-string member is a `400`: guessing a shape here would
/// record a legal mark against something the caller did not name.
fn required_str<'a>(body: &'a serde_json::Value, field: &str) -> Result<&'a str, RestError> {
    body.get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            RestError(ApiError::BadRequest(format!(
                "request body must carry a `{field}` string"
            )))
        })
}

/// An optional string member of the request body; a present non-string is a
/// `400` rather than a silent absence.
fn optional_str<'a>(
    body: &'a serde_json::Value,
    field: &str,
) -> Result<Option<&'a str>, RestError> {
    match body.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.as_str())),
        Some(other) => Err(RestError(ApiError::BadRequest(format!(
            "`{field}` must be a string when present, got {other}"
        )))),
    }
}
