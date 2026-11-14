// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `VERSIONED_EHR_STATUS` container (the versioned-object view of `EHR_STATUS`),
//! whose four reads the `EHR_ACCESS` extension serves the same way for
//! `VERSIONED_EHR_ACCESS` ([`super::ehr_access`]).
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
use serde::de::DeserializeOwned;

use openehr_its::rest::generated::ehr::{
    VersionedEhrStatusGetParams, VersionedEhrStatusRevisionHistoryParams,
    VersionedEhrStatusVersionGetAtTimeParams, VersionedEhrStatusVersionGetByIdParams,
};
use openehr_its::xml::runtime::ToXml;
use openehr_rm::prelude::{RevisionHistory, Version};

use crate::api::RequestParts;
use crate::api::ehr::{EhrObjectType, REVISION_HISTORY_ROOT_TAG, VERSION_ROOT_TAG};
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::{parse_ehr_id, parse_version_uid};
use crate::state::AppState;

/// The `versioned_ehr_status_get` operation, for the versioned object of `T`.
pub(crate) async fn get<T: EhrObjectType>(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedEhrStatusGetParams::from_request,
        VersionedEhrStatusGetParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let resp = state
        .backend()
        .versioned_ehr_object(ehr_id, T::OBJECT)
        .await?;
    Ok(negotiate::resource::read_rm::<T::Versioned>(
        h,
        &resp,
        T::VERSIONED_ROOT_TAG,
    ))
}

/// The `versioned_ehr_status_revision_history` operation, for the versioned
/// object of `T`.
pub(crate) async fn revision_history<T: EhrObjectType>(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedEhrStatusRevisionHistoryParams::from_request,
        VersionedEhrStatusRevisionHistoryParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let resp = state
        .backend()
        .ehr_object_revision_history(ehr_id, T::OBJECT)
        .await?;
    Ok(negotiate::resource::read_rm::<RevisionHistory>(
        h,
        &resp,
        REVISION_HISTORY_ROOT_TAG,
    ))
}

/// The `versioned_ehr_status_version_get_at_time` operation, for the
/// versioned object of `T`.
pub(crate) async fn version_get_at_time<T: EhrObjectType>(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError>
where
    Version<T>: DeserializeOwned + ToXml,
{
    let h = &parts.headers;
    let q = parts.query.as_deref();
    super::refuse_unmapped_xml::<T>(h)?;
    let p = parts.decode(
        VersionedEhrStatusVersionGetAtTimeParams::from_request,
        VersionedEhrStatusVersionGetAtTimeParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let mut resp = state
        .backend()
        .ehr_object_version_at_time(ehr_id, T::OBJECT, p.version_at_time)
        .await?;
    // The envelope carries the same externalized media the bare
    // resource does, so it re-inlines on the same request.
    resp.body = super::expand_multimedia_if_requested(&state, q, resp.body).await?;
    Ok(negotiate::resource::read_rm::<Version<T>>(
        h,
        &resp,
        VERSION_ROOT_TAG,
    ))
}

/// The `versioned_ehr_status_version_get_by_id` operation, for the versioned
/// object of `T`.
pub(crate) async fn version_get_by_id<T: EhrObjectType>(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError>
where
    Version<T>: DeserializeOwned + ToXml,
{
    let h = &parts.headers;
    let q = parts.query.as_deref();
    super::refuse_unmapped_xml::<T>(h)?;
    let p = parts.decode(
        VersionedEhrStatusVersionGetByIdParams::from_request,
        VersionedEhrStatusVersionGetByIdParams::PARAMS,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let (vo_id, version) = super::version_components(&parse_version_uid(&p.version_uid)?)?;
    let mut resp = state
        .backend()
        .ehr_object_version_envelope(ehr_id, T::OBJECT, ferroehr::ids::VoId(vo_id), &version)
        .await?;
    // Full-identity check as on the ehr_status by-version read
    // (`Resources.md` §Identifier types; BASE master05 case rule).
    super::ensure_served_version(&p.version_uid, &resp.body)?;
    // The VERSION envelope carries the same externalized media the bare
    // resource does, so it re-inlines on the same request.
    resp.body = super::expand_multimedia_if_requested(&state, q, resp.body).await?;
    Ok(negotiate::resource::read_rm::<Version<T>>(
        h,
        &resp,
        VERSION_ROOT_TAG,
    ))
}
