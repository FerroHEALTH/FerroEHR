// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR_STATUS` resource + its item tags.
//!
//! Spec: `docs/specs/openehr/ITS-REST/specifications/docs/ehr/` (`EHR_STATUS`) +
//! `specifications/operations/{ehr_status_get_by_version_id,
//! ehr_status_get_at_time,ehr_status_update,ehr_status_tags_get,
//! ehr_status_tags_update,ehr_status_tags_delete}.yaml`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::Response;
use http::StatusCode;
use serde_json::Value;

use openehr_its::rest::generated::ehr::{
    EhrStatusGetAtTimeParams, EhrStatusGetByVersionIdParams, EhrStatusTagsDeleteParams,
    EhrStatusTagsGetParams, EhrStatusTagsUpdateParams, EhrStatusUpdateParams,
};
use openehr_rm::prelude::EhrStatus;

use ferroehr::error::ServiceError;
use ferroehr::service::ehr::object::EhrObject;
use ferroehr::service::response::ServiceResponse;

use ferroehr::versioning::object_version_id::parse_uid_based_id;

use crate::api::RequestParts;
use crate::api::ehr::EhrObjectType;
use crate::api::item_tags;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::negotiate::resource::Tree;
use crate::overview::version_id::{parse_ehr_id, parse_version_uid, require_if_match};
use crate::state::AppState;

/// The `ehr_status_get_by_version_id` operation, and the read of the same
/// shape the `EHR_ACCESS` extension serves for `T`.
pub(crate) async fn get_by_version_id<T: EhrObjectType>(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    // EHR_STATUS and EHR_ACCESS are not templated → no Simplified-Formats
    // mapping; reject a simplified Content-Type/Accept uniformly.
    crate::formats::dispatch::guard_non_templated(h)?;
    super::refuse_unmapped_xml::<T>(h)?;
    let p = parts.decode(
        EhrStatusGetByVersionIdParams::from_request,
        EhrStatusGetByVersionIdParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let (vo_id, version) = super::version_components(&parse_version_uid(&p.version_uid)?)?;
    // The bare object carries no `commit_audit`, so `Last-Modified`
    // (Requests_and_responses.md §"ETag and Last-Modified") comes from
    // the version metadata rather than the served body.
    let resp = state
        .backend()
        .ehr_object_at_version(ehr_id, T::OBJECT, ferroehr::ids::VoId(vo_id), &version)
        .await?;
    // A DV_MULTIMEDIA committed in the object is externalized by the
    // generic versioning path, so this is where it comes back.
    let mut resp = resp;
    resp.body = super::expand_multimedia_if_requested(&state, q, resp.body).await?;
    // The addressed version_uid must equal the served version's full
    // three-part identity, case-insensitively (overview `Resources.md`
    // §Identifier types; BASE master05 §Composite Identifiers and Case).
    super::ensure_served_version(&p.version_uid, &resp.body)?;
    Ok(negotiate::resource::read_rm::<T>(
        h,
        &resp,
        openehr_its::xml::LOCATABLE_ROOT,
    ))
}

/// The `ehr_status_get_at_time` operation, and the read of the same shape the
/// `EHR_ACCESS` extension serves for `T`.
pub(crate) async fn get_at_time<T: EhrObjectType>(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    // Not templated, as on the by-version read.
    crate::formats::dispatch::guard_non_templated(h)?;
    super::refuse_unmapped_xml::<T>(h)?;
    let p = parts.decode(
        EhrStatusGetAtTimeParams::from_request,
        EhrStatusGetAtTimeParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    // Same derivation as the by-version read: the version metadata
    // carries the `Last-Modified` instant.
    let resp = state
        .backend()
        .ehr_object(ehr_id, T::OBJECT, p.version_at_time)
        .await?;
    let mut resp = resp;
    resp.body = super::expand_multimedia_if_requested(&state, q, resp.body).await?;
    Ok(negotiate::resource::read_rm::<T>(
        h,
        &resp,
        openehr_its::xml::LOCATABLE_ROOT,
    ))
}

/// The `ehr_status_update` operation.
pub(crate) async fn update(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let ok = StatusCode::OK;
    let no_content = StatusCode::NO_CONTENT;
    let base = state.config().api.base_path.clone();
    // EHR_STATUS is not templated → no Simplified-Formats mapping; reject a
    // simplified Content-Type/Accept uniformly (see `formats::dispatch`).
    crate::formats::dispatch::guard_non_templated(h)?;
    let p = parts.decode(
        EhrStatusUpdateParams::from_request,
        EhrStatusUpdateParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let body = negotiate::decode::rm_value::<EhrStatus>(h, &parts.body)?;
    let uv = crate::overview::committal::write_request(
        h,
        ferroehr::versioning::committer::request_committer(),
        body,
        Some(require_if_match(&p.if_match)?),
    )?;
    // 204 on the default minimal preference, 200_EHR_STATUS_updated on
    // representation; `ETag`, `Last-Modified` and `Location` on both,
    // the instant taken from the commit result so the write path never
    // re-reads the row it just wrote. Tags are judged before the commit
    // and written after it.
    let pending_tags = item_tags::pending(h)?;
    match Box::pin(state.backend().replace_ehr_status(ehr_id, uv)).await {
        Ok(meta) => {
            let stored_tags = item_tags::persist(
                &state,
                item_tags::TagTarget::EhrContent {
                    ehr_id,
                    target_type: "EHR_STATUS",
                },
                &meta.uid,
                pending_tags,
            )
            .await?;
            let repr =
                if negotiate::prefer::prefers(h, negotiate::prefer::Preference::Representation) {
                    state
                        .backend()
                        .ehr_object(ehr_id, EhrObject::Status, None)
                        .await?
                        .body
                } else {
                    Value::Null
                };
            let resp = ServiceResponse::new(repr, meta);
            let mut resp = negotiate::resource::write_rm::<EhrStatus>(
                h,
                &base,
                no_content,
                ok,
                Some("ehr_status"),
                &resp,
                openehr_its::xml::LOCATABLE_ROOT,
            );
            stored_tags.echo(&mut resp);
            Ok(resp)
        }
        Err(e @ ServiceError::VersionConflict(_)) => {
            let meta = state
                .backend()
                .ehr_status_latest_meta(ehr_id)
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

/// The `ehr_status_tags_get` operation.
pub(crate) async fn tags_get(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let ok = StatusCode::OK;
    // EHR_STATUS is not templated → no Simplified-Formats mapping; reject a
    // simplified Content-Type/Accept uniformly (see `formats::dispatch`).
    crate::formats::dispatch::guard_non_templated(h)?;
    let p = parts.decode(
        EhrStatusTagsGetParams::from_request,
        EhrStatusTagsGetParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let tags = state
        .backend()
        .target_tags_get(ehr_id, parse_uid_based_id(&p.uid_based_id)?, "EHR_STATUS")
        .await?;
    Ok(negotiate::body::respond(
        h,
        ok,
        &openehr_its::json::to_canonical_value(&tags),
    ))
}

/// The `ehr_status_tags_update` operation.
pub(crate) async fn tags_update(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    // EHR_STATUS is not templated → no Simplified-Formats mapping; reject a
    // simplified Content-Type/Accept uniformly (see `formats::dispatch`).
    crate::formats::dispatch::guard_non_templated(h)?;
    let p = parts.decode(
        EhrStatusTagsUpdateParams::from_request,
        EhrStatusTagsUpdateParams::PARAMS,
    )?;
    let target = item_tags::TagTarget::EhrContent {
        ehr_id: parse_ehr_id(&p.ehr_id)?,
        target_type: "EHR_STATUS",
    };
    item_tags::replace(&state, target, p.uid_based_id, &parts).await
}

/// The `ehr_status_tags_delete` operation.
pub(crate) async fn tags_delete(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let no_content = StatusCode::NO_CONTENT;
    // EHR_STATUS is not templated → no Simplified-Formats mapping; reject a
    // simplified Content-Type/Accept uniformly (see `formats::dispatch`).
    crate::formats::dispatch::guard_non_templated(h)?;
    let p = parts.decode(
        EhrStatusTagsDeleteParams::from_request,
        EhrStatusTagsDeleteParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    state
        .backend()
        .target_tag_delete(
            ehr_id,
            parse_uid_based_id(&p.uid_based_id)?,
            "EHR_STATUS",
            p.key,
        )
        .await?;
    Ok(negotiate::body::empty(no_content))
}
