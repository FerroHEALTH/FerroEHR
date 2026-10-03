// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `VERSIONED_EHR_STATUS` container (the versioned-object view of `EHR_STATUS`).
//!
//! Spec: `docs/specs/openehr/ITS-REST/specifications/docs/ehr/` +
//! `specifications/operations/{versioned_ehr_status_get,
//! versioned_ehr_status_revision_history,
//! versioned_ehr_status_version_get_at_time,
//! versioned_ehr_status_version_get_by_id}.yaml`.
//!
//! Every read here carries the container- or version-uid `ETag` and a
//! `Last-Modified` derived from `VERSION.commit_audit.time_committed.value`
//! (`Requests_and_responses.md` §"`ETag` and Last-Modified"), both taken from the
//! service metadata; none carries `Location`, which "MUST NOT be used to
//! indicate an alternate representation of an existing resource" (§Location).

use axum::response::Response;

use openehr_its::rest::generated::ehr::{
    VersionedEhrStatusGetParams, VersionedEhrStatusRevisionHistoryParams,
    VersionedEhrStatusVersionGetAtTimeParams, VersionedEhrStatusVersionGetByIdParams,
};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::{EhrStatus, RevisionHistory, Version, VersionedEhrStatus};

use crate::api::RequestParts;
use crate::api::ehr::VERSION_ROOT_TAG;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::{parse_ehr_id, parse_version_uid};
use crate::state::AppState;

pub(super) async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let base = state.config().server.base_path.clone();

    match op {
        "versioned_ehr_status_get" => {
            let p = parts.decode(
                VersionedEhrStatusGetParams::from_request,
                VersionedEhrStatusGetParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let resp = state
                .backend()
                .versioned_ehr_status_response(ehr_id)
                .await?;
            Ok(negotiate::read_rm::<VersionedEhrStatus>(
                h,
                &base,
                None,
                &resp,
                "versioned_ehr_status",
            ))
        }
        "versioned_ehr_status_revision_history" => {
            let p = parts.decode(
                VersionedEhrStatusRevisionHistoryParams::from_request,
                VersionedEhrStatusRevisionHistoryParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let resp = state
                .backend()
                .ehr_status_revision_history_response(ehr_id)
                .await?;
            Ok(negotiate::read_rm::<RevisionHistory>(
                h,
                &base,
                None,
                &resp,
                "revision_history",
            ))
        }
        "versioned_ehr_status_version_get_at_time" => {
            let p = parts.decode(
                VersionedEhrStatusVersionGetAtTimeParams::from_request,
                VersionedEhrStatusVersionGetAtTimeParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let body = state
                .backend()
                .ehr_status_version_at_time(ehr_id, p.version_at_time)
                .await?;
            // The envelope carries the same externalized media the bare
            // resource does, so it re-inlines on the same request.
            let body = super::expand_multimedia_if_requested(&state, q, body).await?;
            let resp = super::read_resp(&p.ehr_id, body);
            Ok(negotiate::read_rm::<Version<EhrStatus>>(
                h,
                &base,
                None,
                &resp,
                VERSION_ROOT_TAG,
            ))
        }
        "versioned_ehr_status_version_get_by_id" => {
            let p = parts.decode(
                VersionedEhrStatusVersionGetByIdParams::from_request,
                VersionedEhrStatusVersionGetByIdParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let (vo_id, version) = super::version_components(&parse_version_uid(&p.version_uid)?)?;
            let body = state
                .backend()
                .ehr_status_version_envelope(ehr_id, ferroehr::ids::VoId(vo_id), &version)
                .await?;
            // Full-identity check as on the ehr_status by-version read
            // (`Resources.md` §Identifier types; BASE master05 case rule).
            super::ensure_served_version(&p.version_uid, &body)?;
            // The VERSION envelope carries the same externalized media the bare
            // resource does, so it re-inlines on the same request.
            let body = super::expand_multimedia_if_requested(&state, q, body).await?;
            let resp = super::read_resp(&p.ehr_id, body);
            Ok(negotiate::read_rm::<Version<EhrStatus>>(
                h,
                &base,
                None,
                &resp,
                VERSION_ROOT_TAG,
            ))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted ehr operation: {other}"
        )))),
    }
}
