// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `DIRECTORY` (FOLDER) resource.
//!
//! Spec: `docs/specs/openehr/ITS-REST/specifications/docs/ehr/` (DIRECTORY) +
//! `specifications/operations/{directory_get_at_time,directory_update,
//! directory_create,directory_delete,directory_get_by_version_id}.yaml`.
//!
//! Every read here carries `ETag` and `Last-Modified` and no `Location`
//! (overview §"`ETag` and Last-Modified", §Location), and a `FOLDER`'s
//! `DV_MULTIMEDIA` is externalized by the same generic versioning path as a
//! COMPOSITION's, so it re-inlines on the same request.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::Response;
use http::StatusCode;
use serde_json::Value;

use openehr_its::rest::generated::ehr::{
    DirectoryCreateParams, DirectoryDeleteParams, DirectoryGetAtTimeParams,
    DirectoryGetByVersionIdParams, DirectoryUpdateParams,
};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::Folder;

use ferroehr::service::response::ServiceResponse;
use ferroehr::service::status::CallStatusType;

use crate::api::RequestParts;
use crate::api::item_tags;
use crate::negotiate;
use crate::overview::error::{RestError, sm_api_error};
use crate::overview::version_id::{parse_ehr_id, parse_version_uid, require_if_match};
use crate::state::AppState;

pub(super) async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    // DIRECTORY is not templated, so it has no Simplified-Formats mapping.
    crate::formats::dispatch::guard_non_templated(&parts.headers)?;

    match op {
        "directory_get_at_time" => get_at_time(state, parts).await,
        "directory_update" => Box::pin(update(state, parts)).await,
        "directory_create" => create(state, parts).await,
        "directory_delete" => delete(state, parts).await,
        "directory_get_by_version_id" => get_by_version_id(state, parts).await,
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted ehr operation: {other}"
        )))),
    }
}

/// `directory_get_at_time` — serve the EHR's directory as of an instant.
///
/// # Errors
/// The parameter and read rejections the operation declares.
async fn get_at_time(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let no_content = StatusCode::NO_CONTENT;
    let p = DirectoryGetAtTimeParams::from_request(parts.route()?, q, h)?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let resp = state
        .backend()
        .get_directory_at_time(ehr_id, p.version_at_time, p.path)
        .await?;
    // A deleted directory is a 204
    // (directory_get_at_time.yaml `204_because_deleted_at_time`).
    if resp.body.is_null() {
        return Ok(negotiate::empty(no_content));
    }
    let mut resp = resp;
    resp.body = super::expand_multimedia_if_requested(&state, q, resp.body).await?;
    let mut out = negotiate::respond_rm::<Folder>(h, ok, &resp.body, "folder");
    if let Some(meta) = &resp.meta {
        negotiate::set_versioning_headers(&mut out, meta);
    }
    Ok(out)
}

/// `directory_update` — commit a new version of the EHR's directory.
///
/// # Errors
/// The parameter, precondition and commit rejections the operation declares.
async fn update(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let no_content = StatusCode::NO_CONTENT;
    let base = state.config().server.base_path.clone();
    let p = DirectoryUpdateParams::from_request(parts.route()?, q, h)?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let body = negotiate::rm_value::<Folder>(h, &parts.body)?;
    let uv = super::mk_update_version(
        h,
        body,
        super::CHANGE_MODIFICATION,
        "DIRECTORY update",
        Some(require_if_match(&p.if_match)?),
    )?;
    // Tags are judged before the commit and written after it (see
    // `crate::api::item_tags::pending`).
    let pending_tags = item_tags::pending(h)?;
    match state.backend().update_directory(ehr_id, uv).await {
        Ok(meta) => {
            let stored_tags = item_tags::persist(
                &state,
                item_tags::TagTarget::EhrContent {
                    ehr_id,
                    target_type: "FOLDER",
                },
                &meta.uid,
                pending_tags,
            )
            .await?;
            let repr = if negotiate::prefers_representation(h) {
                state
                    .backend()
                    .get_directory_at_time(ehr_id, None, None)
                    .await?
                    .body
            } else {
                Value::Null
            };
            let resp = ServiceResponse::new(repr, meta);
            let mut resp = negotiate::write_rm::<Folder>(
                h,
                &base,
                no_content,
                ok,
                Some("directory"),
                &resp,
                "folder",
            );
            stored_tags.echo(&mut resp);
            Ok(resp)
        }
        Err(e) if e.status == CallStatusType::VersionMismatch => {
            let meta = state
                .backend()
                .directory_latest_meta(ehr_id)
                .await
                .ok()
                .flatten();
            Ok(negotiate::error_with_meta(
                sm_api_error(e),
                &base,
                Some("directory"),
                meta.as_ref(),
            ))
        }
        Err(e) => Err(RestError::from(e)),
    }
}

/// `directory_create` — commit the EHR's first directory version.
///
/// # Errors
/// The parameter and commit rejections the operation declares.
async fn create(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let created = StatusCode::CREATED;
    let base = state.config().server.base_path.clone();
    let p = DirectoryCreateParams::from_request(parts.route()?, q, h)?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let body = negotiate::rm_value::<Folder>(h, &parts.body)?;
    let uv = super::mk_update_version(h, body, super::CHANGE_CREATION, "DIRECTORY creation", None)?;
    let pending_tags = item_tags::pending(h)?;
    let meta = state.backend().create_directory(ehr_id, uv).await?;
    let stored_tags = item_tags::persist(
        &state,
        item_tags::TagTarget::EhrContent {
            ehr_id,
            target_type: "FOLDER",
        },
        &meta.uid,
        pending_tags,
    )
    .await?;
    let repr = if negotiate::prefers_representation(h) {
        state
            .backend()
            .get_directory_at_time(ehr_id, None, None)
            .await?
            .body
    } else {
        Value::Null
    };
    let resp = ServiceResponse::new(repr, meta);
    let mut resp = negotiate::write_rm::<Folder>(
        h,
        &base,
        created,
        created,
        Some("directory"),
        &resp,
        "folder",
    );
    stored_tags.echo(&mut resp);
    Ok(resp)
}

/// `directory_delete` — commit a `523|deleted|` version of the EHR's
/// directory.
///
/// # Errors
/// The parameter, precondition and commit rejections the operation declares.
async fn delete(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let base = state.config().server.base_path.clone();
    let p = DirectoryDeleteParams::from_request(parts.route()?, q, h)?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    // A DELETE commits a `523|deleted|` version, so the committal headers apply
    // here too (overview §"openehr-version and openehr-audit-details").
    let update_audit =
        crate::overview::committal::committal_audit_for_delete(h, super::committer_proxy())?;
    match state
        .backend()
        .delete_directory(
            ehr_id,
            Some(require_if_match(&p.if_match)?),
            update_audit.as_ref(),
        )
        .await
    {
        // The 204 carries the new deleted version's weak `ETag` and
        // `Last-Modified` (RM common master06 §Logical Deletion).
        Ok(resp) => Ok(negotiate::deleted_with_headers(
            &base,
            Some("directory"),
            &resp,
        )),
        Err(e) if e.status == CallStatusType::VersionMismatch => {
            let meta = state
                .backend()
                .directory_latest_meta(ehr_id)
                .await
                .ok()
                .flatten();
            Ok(negotiate::error_with_meta(
                sm_api_error(e),
                &base,
                Some("directory"),
                meta.as_ref(),
            ))
        }
        Err(e) => Err(RestError::from(e)),
    }
}

/// `directory_get_by_version_id` — serve one version of the EHR's directory.
///
/// # Errors
/// The parameter and read rejections the operation declares.
async fn get_by_version_id(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let no_content = StatusCode::NO_CONTENT;
    let p = DirectoryGetByVersionIdParams::from_request(parts.route()?, q, h)?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let ovid = parse_version_uid(&p.version_uid)?;
    let resp = state
        .backend()
        .get_directory_at_version(ehr_id, ovid, p.path.as_deref())
        .await?;
    if resp.body.is_null() {
        return Ok(negotiate::empty(no_content));
    }
    let mut resp = resp;
    resp.body = super::expand_multimedia_if_requested(&state, q, resp.body).await?;
    let mut out = negotiate::respond_rm::<Folder>(h, ok, &resp.body, "folder");
    if let Some(meta) = &resp.meta {
        negotiate::set_versioning_headers(&mut out, meta);
    }
    Ok(out)
}
