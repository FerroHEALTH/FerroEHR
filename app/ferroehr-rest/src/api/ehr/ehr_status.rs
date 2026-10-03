// SPDX-FileCopyrightText: Vernum Projecten B.V.
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
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::EhrStatus;

use ferroehr::service::response::ServiceResponse;
use ferroehr::service::status::CallStatusType;

use crate::api::RequestParts;
use crate::api::item_tags;
use crate::negotiate;
use crate::overview::error::{RestError, sm_api_error};
use crate::overview::version_id::{parse_ehr_id, parse_version_uid, require_if_match};
use crate::state::AppState;

#[expect(
    clippy::too_many_lines,
    reason = "one arm per EHR_STATUS operation: a flat match keeps every \
              operation's wire behaviour readable in one place"
)]
pub(super) async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let no_content = StatusCode::NO_CONTENT;
    let base = state.config().server.base_path.clone();

    // EHR_STATUS is not templated → no Simplified-Formats mapping; reject a
    // simplified Content-Type/Accept uniformly (see `formats::dispatch`).
    crate::formats::dispatch::guard_non_templated(h)?;

    match op {
        "ehr_status_get_by_version_id" => {
            let p = EhrStatusGetByVersionIdParams::from_request(
                &parts.route_for(EhrStatusGetByVersionIdParams::PARAMS),
                q,
                h,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let (vo_id, version) = super::version_components(&parse_version_uid(&p.version_uid)?)?;
            // A bare EHR_STATUS carries no `commit_audit`, so `Last-Modified`
            // (Requests_and_responses.md §"ETag and Last-Modified") comes from
            // the version metadata rather than the served body.
            let resp = state
                .backend()
                .ehr_status_at_version_response(ehr_id, ferroehr::ids::VoId(vo_id), &version)
                .await?;
            // A DV_MULTIMEDIA committed in an EHR_STATUS is externalized by
            // the generic versioning path, so this is where it comes back.
            let mut resp = resp;
            resp.body = super::expand_multimedia_if_requested(&state, q, resp.body).await?;
            // The addressed version_uid must equal the served version's full
            // three-part identity, case-insensitively (overview `Resources.md`
            // §Identifier types; BASE master05 §Composite Identifiers and Case).
            super::ensure_served_version(&p.version_uid, &resp.body)?;
            Ok(negotiate::read_rm::<EhrStatus>(
                h,
                &base,
                Some("ehr_status"),
                &resp,
                "ehr_status",
            ))
        }
        "ehr_status_get_at_time" => {
            let p = EhrStatusGetAtTimeParams::from_request(
                &parts.route_for(EhrStatusGetAtTimeParams::PARAMS),
                q,
                h,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            // Same derivation as the by-version read: the version metadata
            // carries the `Last-Modified` instant.
            let resp = state
                .backend()
                .ehr_status_at_time_response(ehr_id, p.version_at_time)
                .await?;
            let mut resp = resp;
            resp.body = super::expand_multimedia_if_requested(&state, q, resp.body).await?;
            Ok(negotiate::read_rm::<EhrStatus>(
                h,
                &base,
                Some("ehr_status"),
                &resp,
                "ehr_status",
            ))
        }
        "ehr_status_update" => {
            let p = EhrStatusUpdateParams::from_request(
                &parts.route_for(EhrStatusUpdateParams::PARAMS),
                q,
                h,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let body = negotiate::rm_value::<EhrStatus>(h, &parts.body)?;
            let uv = super::mk_update_version(
                h,
                body,
                super::CHANGE_MODIFICATION,
                "EHR_STATUS update",
                Some(require_if_match(&p.if_match)?),
            )?;
            // 204 on the default minimal preference, 200_EHR_STATUS_updated on
            // representation; `ETag`, `Last-Modified` and `Location` on both,
            // the instant taken from the commit result so the write path never
            // re-reads the row it just wrote. Tags are judged before the commit
            // and written after it.
            let pending_tags = item_tags::pending(h)?;
            match Box::pin(state.backend().replace_ehr_status_meta(ehr_id, uv)).await {
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
                    let repr = if negotiate::prefers_representation(h) {
                        state.backend().get_ehr_status(ehr_id).await?
                    } else {
                        Value::Null
                    };
                    let resp = ServiceResponse::new(repr, meta);
                    let mut resp = negotiate::write_rm::<EhrStatus>(
                        h,
                        &base,
                        no_content,
                        ok,
                        Some("ehr_status"),
                        &resp,
                        "ehr_status",
                    );
                    stored_tags.echo(&mut resp);
                    Ok(resp)
                }
                Err(e) if e.status == CallStatusType::VersionMismatch => {
                    let meta = state
                        .backend()
                        .ehr_status_latest_meta(ehr_id)
                        .await
                        .ok()
                        .flatten();
                    Ok(negotiate::error_with_meta(
                        sm_api_error(e),
                        &base,
                        Some("ehr_status"),
                        meta.as_ref(),
                    ))
                }
                Err(e) => Err(RestError::from(e)),
            }
        }
        "ehr_status_tags_get" => {
            let p = EhrStatusTagsGetParams::from_request(
                &parts.route_for(EhrStatusTagsGetParams::PARAMS),
                q,
                h,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let tags = state
                .backend()
                .target_tags_get(ehr_id, p.uid_based_id, "EHR_STATUS")
                .await?;
            Ok(negotiate::respond(
                h,
                ok,
                &openehr_its::json::to_canonical_value(&tags),
            ))
        }
        "ehr_status_tags_update" => {
            let p = EhrStatusTagsUpdateParams::from_request(
                &parts.route_for(EhrStatusTagsUpdateParams::PARAMS),
                q,
                h,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let body = item_tags::write_body(h, &parts.body)?;
            let tags = state
                .backend()
                .target_tags_replace(ehr_id, p.uid_based_id, "EHR_STATUS", body)
                .await?;
            // ehr_status_tags_update.yaml: 200 on `Prefer: return=representation`,
            // 204 when `Prefer` is missing or `return=minimal` (overview §Prefer).
            Ok(negotiate::write_collection(
                h,
                no_content,
                ok,
                &openehr_its::json::to_canonical_value(&tags),
            ))
        }
        "ehr_status_tags_delete" => {
            let p = EhrStatusTagsDeleteParams::from_request(
                &parts.route_for(EhrStatusTagsDeleteParams::PARAMS),
                q,
                h,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            state
                .backend()
                .target_tag_delete(ehr_id, p.uid_based_id, "EHR_STATUS", p.key)
                .await?;
            Ok(negotiate::empty(no_content))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted ehr operation: {other}"
        )))),
    }
}
