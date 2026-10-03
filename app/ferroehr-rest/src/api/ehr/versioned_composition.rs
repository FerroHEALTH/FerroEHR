// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `VERSIONED_COMPOSITION` container (the versioned-object view of a COMPOSITION).
//!
//! Spec: `docs/specs/openehr/ITS-REST/specifications/docs/ehr/` +
//! `specifications/operations/{versioned_composition_get,
//! versioned_composition_revision_history,
//! versioned_composition_version_get_at_time,
//! versioned_composition_version_get_by_id}.yaml`.
//!
//! Every read here carries the container- or version-uid `ETag` and a
//! `Last-Modified` derived from `VERSION.commit_audit.time_committed.value`
//! (`Requests_and_responses.md` §"`ETag` and Last-Modified"), both taken from the
//! service metadata; none carries `Location`, which "MUST NOT be used to
//! indicate an alternate representation of an existing resource" (§Location).

use axum::response::Response;

use openehr_its::rest::generated::ehr::{
    VersionedCompositionGetParams, VersionedCompositionRevisionHistoryParams,
    VersionedCompositionVersionGetAtTimeParams, VersionedCompositionVersionGetByIdParams,
};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::{Composition, RevisionHistory, Version, VersionedComposition};

use crate::api::RequestParts;
use crate::api::ehr::VERSION_ROOT_TAG;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::{parse_ehr_id, parse_uuid, parse_version_uid};
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
        "versioned_composition_get" => {
            let p = parts.decode(
                VersionedCompositionGetParams::from_request,
                VersionedCompositionGetParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let vo_id = parse_uuid(&p.versioned_object_uid, "versioned_object_uid")?;
            let resp = state
                .backend()
                .versioned_composition_response(ehr_id, ferroehr::ids::VoId(vo_id))
                .await?;
            Ok(negotiate::read_rm::<VersionedComposition>(
                h,
                &base,
                None,
                &resp,
                "versioned_composition",
            ))
        }
        "versioned_composition_revision_history" => {
            let p = parts.decode(
                VersionedCompositionRevisionHistoryParams::from_request,
                VersionedCompositionRevisionHistoryParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let vo_id = parse_uuid(&p.versioned_object_uid, "versioned_object_uid")?;
            let resp = state
                .backend()
                .composition_revision_history_response(ehr_id, ferroehr::ids::VoId(vo_id))
                .await?;
            Ok(negotiate::read_rm::<RevisionHistory>(
                h,
                &base,
                None,
                &resp,
                "revision_history",
            ))
        }
        "versioned_composition_version_get_at_time" => {
            let p = parts.decode(
                VersionedCompositionVersionGetAtTimeParams::from_request,
                VersionedCompositionVersionGetAtTimeParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let vo_id = parse_uuid(&p.versioned_object_uid, "versioned_object_uid")?;
            let body = state
                .backend()
                .composition_version_at_time(ehr_id, ferroehr::ids::VoId(vo_id), p.version_at_time)
                .await?;
            // The VERSION envelope carries the same externalized media the bare
            // resource does, so it re-inlines on the same request.
            let body = super::expand_multimedia_if_requested(&state, q, body).await?;
            let resp = super::read_resp(&p.ehr_id, body);
            Ok(negotiate::read_rm::<Version<Composition>>(
                h,
                &base,
                None,
                &resp,
                VERSION_ROOT_TAG,
            ))
        }
        "versioned_composition_version_get_by_id" => {
            let p = parts.decode(
                VersionedCompositionVersionGetByIdParams::from_request,
                VersionedCompositionVersionGetByIdParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let vo_id = parse_uuid(&p.versioned_object_uid, "versioned_object_uid")?;
            let ovid = parse_version_uid(&p.version_uid)?;
            // The version_uid's object_id must match the VERSIONED_OBJECT
            // identifier the path names (overview `Resources.md` §Identifier
            // types; RM common version.adoc `Owner_id_valid`), so an incoherent
            // pair names nothing here and is a 404.
            if crate::overview::version_id::object_id_uuid(&ovid) != Some(vo_id) {
                return Err(RestError(ApiError::NotFound(format!(
                    "version {} in versioned object {}",
                    p.version_uid, p.versioned_object_uid
                ))));
            }
            let body = state
                .backend()
                .composition_version_envelope(ehr_id, ovid)
                .await?;
            // The VERSION envelope carries the same externalized media the bare
            // resource does, so it re-inlines on the same request.
            let body = super::expand_multimedia_if_requested(&state, q, body).await?;
            let resp = super::read_resp(&p.ehr_id, body);
            Ok(negotiate::read_rm::<Version<Composition>>(
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
