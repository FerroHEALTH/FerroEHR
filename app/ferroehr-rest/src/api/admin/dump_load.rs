// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN **dump/load** wire — **our own extension**.
//!
//! No openEHR ITS-REST operation governs either route here. The released Admin
//! API is exactly two EHR deletes (`specifications/admin.openapi.yaml` →
//! `operations/admin_ehr_delete.yaml` + `operations/admin_ehr_delete_all.yaml`),
//! while the SM declares a dump/load interface the release never surfaced —
//! `docs/specs/openehr/SM/docs/UML/classes/i_admin_dump_load.adoc`:
//! `export_ehrs` ("Export all EHRs to a file-system location in a specified
//! format") and `load_ehrs` ("Populate EHR repository from export archive on
//! file system. Repository need not be empty, but import EHRs with duplicate
//! EHR ids will fail"), both declaring the single error `file_not_writable`.
//! No released ITS-REST operation covers either call.
//!
//! These routes are the honest realization of that service basis, and are
//! **excluded from ITS-REST wire conformance**: they gate the `EhrDumpLoad`
//! CAPABILITY verdict only.
//!
//! ## What the bodies carry
//!
//! `export_ehrs`'s signature passes `logical_fmt` / `comp_fmt` / `enc_format`
//! loose, while `export_spec.adoc` bundles the same three attributes
//! (`logical_format`, `compression_format`, `encoding`) with the mandatory
//! `segment_split_size: Integer [1..1]` (kb). `EXPORT_SPEC` is the SM's own
//! richer form for exactly this operation, so the request body is the
//! `EXPORT_SPEC` attribute set plus the separate `file_sys_loc` parameter, and
//! the enumeration values are the vendored literals verbatim
//! (`export_format.adoc`: `openehr_canonical_xml` / `openehr_canonical_json`;
//! `compression_format.adoc`: `zip` / `7z`).
//!
//! `encoding` is REFUSED with a `400`, and that is a derivation rather than a
//! gap: `encoding_format.adoc` declares `ENCODING_FORMAT` as an enumeration
//! with **no members at all**, so no text a client could send names one — SM
//! `precondition_violation`.
//!
//! Both operations return `List<DUMP_LOAD_FAIL_REPORT>`
//! (`dump_load_fail_report.adoc`: `entity_type`, `entity_id`, `dump_status`,
//! `error [0..1]`), so both answer `200` with a JSON array — empty when every
//! entity succeeded. The array is the resource; there is no envelope, because
//! no spec defines one.
//!
//! Gating: mounted under `/admin/`, so they inherit the group's RBAC Admin
//! class (`401`/`403`, our own authorization design — the released admin
//! operations carry `security: []`) and the `AppConfig::admin.enabled` config
//! gate (`405` with an empty `Allow` when off) unchanged.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use openehr_its::rest::runtime::ApiError;

use ferroehr::service::admin::types::{
    CompressionFormat, DumpLoadFailReport, ExportFormat, ExportSpec,
};

use crate::api::{BoxResponse, RequestParts, guarded_dispatch};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The dump/load extension routes as a native `utoipa-axum` router — **no
/// ITS-REST contract** (see the module docs). Group-relative paths (nested
/// under `base_path`); every operation runs through [`guarded_dispatch`] with
/// [`dispatch`].
pub(crate) fn dump_load_routes() -> OpenApiRouter<AppState> {
    // One `routes!` per PATH (handlers in a single call must share the path).
    OpenApiRouter::new()
        .routes(routes!(admin_dump))
        .routes(routes!(admin_load))
}

/// Export every EHR to a file-system archive (`POST /admin/dump`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_ADMIN_DUMP_LOAD.export_ehrs`.
#[utoipa::path(
    post, path = "/admin/dump", tag = "admin-dump-load",
    request_body(content = serde_json::Value,
                 description = "The SM `export_ehrs` parameters as one object: \
                                `file_sys_loc` (required, the archive location \
                                on the SERVER's file system) plus the \
                                `EXPORT_SPEC` attributes `logical_format` \
                                (`openehr_canonical_json`, the default, keeps \
                                each version's payload inline as canonical \
                                JSON \\| `openehr_canonical_xml` externalizes \
                                it as a `versions/<version_uid>.xml` \
                                `ORIGINAL_VERSION` document under the ITS-XML \
                                published `<version>` root; the archive's own \
                                manifest/segment envelope is JSON in both), \
                                `compression_format` \
                                (`zip` \\| `7z`; absent = uncompressed) and \
                                `segment_split_size` (kb, default 1024). \
                                `encoding` names an `ENCODING_FORMAT` member \
                                and the vendored enumeration is EMPTY, so any \
                                value is refused.",
                 example = json!({
                     "file_sys_loc": "/tmp/openehr-export",
                     "logical_format": "openehr_canonical_json",
                     "compression_format": "zip",
                     "segment_split_size": 1024
                 })),
    responses(
        (status = 200, description = "The archive was written. The body is the \
                                      `List<DUMP_LOAD_FAIL_REPORT>` the SM \
                                      operation returns — EMPTY when every EHR \
                                      was dumped successfully.",
         body = Vec<serde_json::Value>, example = json!([])),
        (status = 400, description = "`file_sys_loc` is absent or blank, a \
                                      format value names no member of its \
                                      vendored enumeration, `encoding` is \
                                      present (`ENCODING_FORMAT` is an empty \
                                      enumeration), or `segment_split_size` is \
                                      not a positive integer — SM \
                                      `precondition_violation`.",
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
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value),
        (status = 500, description = "SM `file_not_writable` — the location, a \
                                      segment entry, a version-payload entry or \
                                      the manifest could not be created or \
                                      written.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_dump(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_dump", parts, dispatch).await
}

/// Populate the repository from a file-system archive (`POST /admin/load`).
///
/// **Our own extension — no ITS-REST operation governs this** (module
/// docs). Realizes SM `I_ADMIN_DUMP_LOAD.load_ehrs`.
#[utoipa::path(
    post, path = "/admin/load", tag = "admin-dump-load",
    request_body(content = serde_json::Value,
                 description = "`{ \"file_sys_loc\": … }` — the archive \
                                location on the SERVER's file system. \
                                `load_ehrs` takes this ONE parameter \
                                (`i_admin_dump_load.adoc`), so the container \
                                (loose files, `archive.zip` or `archive.7z`) \
                                is detected from what the location holds and \
                                the payload form comes from the archive's own \
                                manifest `format` member.",
                 example = json!({ "file_sys_loc": "/tmp/openehr-export" })),
    responses(
        (status = 200, description = "The archive was read. The body is the \
                                      `List<DUMP_LOAD_FAIL_REPORT>` the SM \
                                      operation returns: one entry per entity \
                                      that did NOT load — notably \"import EHRs \
                                      with duplicate EHR ids will fail\", which \
                                      is reported per EHR and skipped, never \
                                      fatal, and so is a record whose \
                                      externalized `versions/*.xml` payload \
                                      will not read. EMPTY when the whole \
                                      archive loaded.",
         body = Vec<serde_json::Value>,
         example = json!([{ "entity_type": "EHR",
                            "entity_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                            "dump_status": false,
                            "error": "an EHR with this id already exists" }])),
        (status = 400, description = "`file_sys_loc` is absent or blank, or the \
                                      archive carries externalized multimedia \
                                      this server has no store for — SM \
                                      `precondition_violation`.",
         body = serde_json::Value),
        (status = 401, description = "Unauthenticated. Our own authorization \
                                      design.",
         body = serde_json::Value),
        (status = 403, description = "Authenticated but not in the Admin class. \
                                      Our own authorization design.",
         body = serde_json::Value),
        (status = 405, description = "The admin API is disabled on this server \
                                      (`AppConfig::admin.enabled`, default \
                                      false), answered with an empty `Allow` \
                                      per RFC 9110 §10.2.1.",
         body = serde_json::Value),
        (status = 415, description = "The request `Content-Type` is not \
                                      `application/json`.",
         body = serde_json::Value),
        (status = 500, description = "SM `file_not_writable` — the location \
                                      holds no archive container, an entry \
                                      could not be read, the manifest / a \
                                      segment is CORRUPT (mangled or truncated, \
                                      so it does not parse as part of this \
                                      archive format), or the manifest names no \
                                      `EXPORT_FORMAT` member. All are the same \
                                      fact — `file_sys_loc` does not hold a \
                                      readable archive — and carry the one \
                                      error `i_admin_dump_load.adoc` declares. \
                                      A corrupt per-version payload entry is \
                                      NOT here: it belongs to exactly one EHR, \
                                      so it is a per-entity `200` report.",
         body = serde_json::Value)
    )
)]
pub(crate) async fn admin_load(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Response {
    let parts = crate::api::into_parts(request).await;
    guarded_dispatch(state, "admin_load", parts, dispatch).await
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
    let body = negotiate::json_value(h, &parts.body)?;
    let file_sys_loc = file_sys_loc(&body)?;

    match op {
        "admin_dump" => {
            let spec = export_spec(&body)?;
            let reports = state.backend().export_ehrs(file_sys_loc, spec).await?;
            Ok(negotiate::respond(
                h,
                StatusCode::OK,
                &fail_reports(&reports),
            ))
        }
        "admin_load" => {
            let reports = state.backend().load_ehrs(file_sys_loc).await?;
            Ok(negotiate::respond(
                h,
                StatusCode::OK,
                &fail_reports(&reports),
            ))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted admin dump/load operation: {other}"
        )))),
    }
}

/// Read the mandatory `file_sys_loc` (`String [1]` on both SM operations). An
/// absent or blank location names no location at all — SM
/// `precondition_violation`, refused before any storage or filesystem work.
fn file_sys_loc(body: &Value) -> Result<String, RestError> {
    let raw = body
        .get("file_sys_loc")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            RestError(ApiError::BadRequest(
                "request body must carry a non-empty `file_sys_loc` string (the SM \
                 file-system location parameter)"
                    .to_owned(),
            ))
        })?;
    Ok(raw.to_owned())
}

/// The `EXPORT_SPEC` default `segment_split_size` in kb.
///
/// Build the SM `EXPORT_SPEC` from the request body.
fn export_spec(body: &Value) -> Result<ExportSpec, RestError> {
    // `ENCODING_FORMAT` (encoding_format.adoc) is an enumeration with NO
    // members, so no value a client can send names one.
    if body.get("encoding").is_some() {
        return Err(RestError(ApiError::BadRequest(
            "`encoding` names an ENCODING_FORMAT member, and the vendored SM enumeration \
             (encoding_format.adoc) declares none — no value is representable"
                .to_owned(),
        )));
    }
    let logical_format = enum_member::<ExportFormat>(body, "logical_format", "EXPORT_FORMAT")?;
    let compression_format =
        enum_member::<CompressionFormat>(body, "compression_format", "COMPRESSION_FORMAT")?;
    let segment_split_size = match body.get("segment_split_size") {
        // NOTE: `export_spec.adoc` makes `segment_split_size` mandatory but names
        // no default, and no openEHR spec governs this wire — our own extension:
        // an omitted size takes 1024 kb so "just dump it" needs no tuning knob.
        None | Some(Value::Null) => 1024,
        Some(value) => value.as_i64().ok_or_else(|| {
            RestError(ApiError::BadRequest(format!(
                "`segment_split_size` is the EXPORT_SPEC Integer size in kb, got {value}"
            )))
        })?,
    };
    #[expect(
        clippy::map_err_ignore,
        reason = "`TryFromIntError` says only \"out of range\", which the 400 body \
                  already states while echoing the rejected size"
    )]
    let segment_split_size = i32::try_from(segment_split_size).map_err(|_| {
        RestError(ApiError::BadRequest(format!(
            "`segment_split_size` {segment_split_size} is outside the representable kb range"
        )))
    })?;
    Ok(ExportSpec {
        logical_format,
        compression_format,
        segment_split_size,
    })
}

/// Read one optional SM enumeration member from `field`. A present value that
/// names no member is a `400` (`precondition_violation`) — never silently
/// dropped, which would export in a format the caller did not ask for.
fn enum_member<T: std::str::FromStr<Err = ()>>(
    body: &Value,
    field: &str,
    enumeration: &str,
) -> Result<Option<T>, RestError> {
    match body.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let raw = value.as_str().ok_or_else(|| {
                RestError(ApiError::BadRequest(format!(
                    "`{field}` must be a {enumeration} enumeration literal, got {value}"
                )))
            })?;
            raw.parse::<T>().map(Some).map_err(|()| {
                RestError(ApiError::BadRequest(format!(
                    "`{field}` names no {enumeration} member: {raw:?}"
                )))
            })
        }
    }
}

/// Render the SM `List<DUMP_LOAD_FAIL_REPORT>` return value
/// (`dump_load_fail_report.adoc`: `entity_type` 1..1, `entity_id` 1..1,
/// `dump_status` 1..1, `error` 0..1) as the response array. `error` is omitted
/// when the report carries none, matching its `[0..1]` multiplicity.
fn fail_reports(reports: &[DumpLoadFailReport]) -> Vec<Value> {
    reports
        .iter()
        .map(|r| {
            let mut entry = json!({
                "entity_type": r.entity_type,
                "entity_id": r.entity_id,
                "dump_status": r.dump_status,
            });
            if let Some(error) = &r.error
                && let Some(map) = entry.as_object_mut()
            {
                map.insert("error".to_owned(), Value::String(error.clone()));
            }
            entry
        })
        .collect()
}
