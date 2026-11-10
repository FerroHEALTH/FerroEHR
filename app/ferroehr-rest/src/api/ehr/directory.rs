// SPDX-FileCopyrightText: Cadasto B.V.
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
use openehr_rm::prelude::Folder;

use ferroehr::error::ServiceError;
use ferroehr::service::response::ServiceResponse;

use crate::api::RequestParts;
use crate::api::item_tags;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::negotiate::resource::Tree;
use crate::overview::version_id::{parse_ehr_id, parse_version_uid, require_if_match};
use crate::state::AppState;

/// `directory_get_at_time` — serve the EHR's directory as of an instant.
///
/// # Errors
/// The parameter and read rejections the operation declares.
pub(crate) async fn get_at_time(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    // DIRECTORY is not templated, so it has no Simplified-Formats mapping.
    crate::formats::dispatch::guard_non_templated(&parts.headers)?;
    let p = parts.decode(
        DirectoryGetAtTimeParams::from_request,
        DirectoryGetAtTimeParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let resp = state
        .backend()
        .get_directory_at_time(ehr_id, p.version_at_time, p.path)
        .await?;
    serve_folder(&state, &parts, resp).await
}

/// Serves one directory read: a `204` for a directory deleted at the
/// addressed point (`directory_get_at_time.yaml`
/// `204_because_deleted_at_time`), otherwise the FOLDER with any requested
/// multimedia expanded and the versioning headers its metadata carries.
///
/// # Errors
/// The multimedia expansion's rejections.
async fn serve_folder(
    state: &AppState,
    parts: &RequestParts,
    mut resp: ServiceResponse,
) -> Result<Response, RestError> {
    if resp.body.is_null() {
        return Ok(negotiate::body::empty(StatusCode::NO_CONTENT));
    }
    resp.body =
        super::expand_multimedia_if_requested(state, parts.query.as_deref(), resp.body).await?;
    let mut out = negotiate::body::respond_rm::<Folder>(
        &parts.headers,
        StatusCode::OK,
        &resp.body,
        openehr_its::xml::LOCATABLE_ROOT,
    );
    if let Some(meta) = &resp.meta {
        negotiate::resource::set_versioning_headers(&mut out, meta, Tree::Ehr);
    }
    Ok(out)
}

/// `directory_update` — commit a new version of the EHR's directory.
///
/// # Errors
/// The parameter, precondition and commit rejections the operation declares.
pub(crate) async fn update(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    // DIRECTORY is not templated, so it has no Simplified-Formats mapping.
    crate::formats::dispatch::guard_non_templated(&parts.headers)?;
    let h = &parts.headers;
    let ok = StatusCode::OK;
    let no_content = StatusCode::NO_CONTENT;
    let base = state.config().api.base_path.clone();
    let p = parts.decode(
        DirectoryUpdateParams::from_request,
        DirectoryUpdateParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let body = negotiate::decode::rm_value::<Folder>(h, &parts.body)?;
    let uv = crate::overview::committal::write_request(
        h,
        ferroehr::versioning::committer::request_committer(),
        body,
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
            let repr =
                if negotiate::prefer::prefers(h, negotiate::prefer::Preference::Representation) {
                    state
                        .backend()
                        .get_directory_at_time(ehr_id, None, None)
                        .await?
                        .body
                } else {
                    Value::Null
                };
            let resp = ServiceResponse::new(repr, meta);
            let mut resp = negotiate::resource::write_rm::<Folder>(
                h,
                &base,
                no_content,
                ok,
                Some("directory"),
                &resp,
                openehr_its::xml::LOCATABLE_ROOT,
            );
            stored_tags.echo(&mut resp);
            Ok(resp)
        }
        Err(e @ ServiceError::VersionConflict(_)) => {
            let meta = state
                .backend()
                .directory_latest_meta(ehr_id)
                .await
                .ok()
                .flatten();
            Ok(negotiate::resource::error_with_meta(
                crate::overview::service_error::api_error(e),
                meta.as_ref(),
                Tree::Ehr,
            ))
        }
        Err(e) => Err(RestError::from(e)),
    }
}

/// `directory_create` — commit the EHR's first directory version.
///
/// # Errors
/// The parameter and commit rejections the operation declares.
pub(crate) async fn create(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    // DIRECTORY is not templated, so it has no Simplified-Formats mapping.
    crate::formats::dispatch::guard_non_templated(&parts.headers)?;
    let h = &parts.headers;
    let created = StatusCode::CREATED;
    let base = state.config().api.base_path.clone();
    let p = parts.decode(
        DirectoryCreateParams::from_request,
        DirectoryCreateParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let body = negotiate::decode::rm_value::<Folder>(h, &parts.body)?;
    let uv = crate::overview::committal::write_request(
        h,
        ferroehr::versioning::committer::request_committer(),
        body,
        None,
    )?;
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
    let repr = if negotiate::prefer::prefers(h, negotiate::prefer::Preference::Representation) {
        state
            .backend()
            .get_directory_at_time(ehr_id, None, None)
            .await?
            .body
    } else {
        Value::Null
    };
    let resp = ServiceResponse::new(repr, meta);
    let mut resp = negotiate::resource::write_rm::<Folder>(
        h,
        &base,
        created,
        created,
        Some("directory"),
        &resp,
        openehr_its::xml::LOCATABLE_ROOT,
    );
    stored_tags.echo(&mut resp);
    Ok(resp)
}

/// `directory_delete` — commit a `523|deleted|` version of the EHR's
/// directory.
///
/// # Errors
/// The parameter, precondition and commit rejections the operation declares.
pub(crate) async fn delete(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    // DIRECTORY is not templated, so it has no Simplified-Formats mapping.
    crate::formats::dispatch::guard_non_templated(&parts.headers)?;
    let h = &parts.headers;
    let p = parts.decode(
        DirectoryDeleteParams::from_request,
        DirectoryDeleteParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    // A DELETE commits a `523|deleted|` version, so the committal headers apply
    // here too (overview §"openehr-version and openehr-audit-details").
    let mut request = crate::overview::committal::delete_request(
        h,
        ferroehr::versioning::committer::request_committer(),
        None,
    )?;
    request.preceding = Some(require_if_match(&p.if_match)?);
    match state.backend().delete_directory(ehr_id, request).await {
        // The 204 carries the new deleted version's weak `ETag` and
        // `Last-Modified` (RM common master06 §Logical Deletion).
        Ok(resp) => Ok(negotiate::resource::deleted_with_headers(&resp)),
        Err(e @ ServiceError::VersionConflict(_)) => {
            let meta = state
                .backend()
                .directory_latest_meta(ehr_id)
                .await
                .ok()
                .flatten();
            Ok(negotiate::resource::error_with_meta(
                crate::overview::service_error::api_error(e),
                meta.as_ref(),
                Tree::Ehr,
            ))
        }
        Err(e) => Err(RestError::from(e)),
    }
}

/// `directory_get_by_version_id` — serve one version of the EHR's directory.
///
/// # Errors
/// The parameter and read rejections the operation declares.
pub(crate) async fn get_by_version_id(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    // DIRECTORY is not templated, so it has no Simplified-Formats mapping.
    crate::formats::dispatch::guard_non_templated(&parts.headers)?;
    let p = parts.decode(
        DirectoryGetByVersionIdParams::from_request,
        DirectoryGetByVersionIdParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let ovid = parse_version_uid(&p.version_uid)?;
    let resp = state
        .backend()
        .get_directory_at_version(ehr_id, ovid, p.path.as_deref())
        .await?;
    serve_folder(&state, &parts, resp).await
}
