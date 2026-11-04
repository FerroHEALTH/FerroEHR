// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `VERSIONED_PARTY` reads — `operations/versioned_party_get.yaml`,
//! `versioned_party_revision_history.yaml`,
//! `versioned_party_version_get_at_time.yaml`,
//! `versioned_party_version_get_by_id.yaml`. Canonical content negotiation
//! (`Accept_canonical`/`ContentType_canonical`).
//!
//! Every read here is a `GET` on a `VERSION` / `VERSIONED_OBJECT` resource, so
//! each carries the weak `ETag` (and `Last-Modified` where the served body
//! exposes a commit audit) the ITS-REST overview
//! `Requests_and_responses.md` §"`ETag` and Last-Modified" asks for — and NO
//! `Location`, which §Location restricts to creation/redirect responses and
//! §"Deprecated headers" deprecates on `GET`.

use axum::response::Response;

use openehr_its::rest::generated::demographic::{
    VersionedPartyGetParams, VersionedPartyVersionGetAtTimeParams,
    VersionedPartyVersionGetByIdParams,
};

use ferroehr::versioning::object_version_id::parse_version_uid;

use crate::api::RequestParts;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The `versioned_party_get` operation.
pub(crate) async fn get(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedPartyGetParams::from_request,
        VersionedPartyGetParams::PARAMS,
    )?;
    let resp = state
        .backend()
        .versioned_party_get(super::object_of(&p.versioned_object_uid)?)
        .await?;
    // ETag from VERSIONED_OBJECT.uid.value and Last-Modified from the
    // newest version's commit instant, both via the service metadata
    // seam (overview §"ETag and Last-Modified": VERSIONED_OBJECT is
    // named outright; the instant is the version spine's, never
    // scraped from the container body, which exposes no commit
    // audit — the EHR-side container reads' pattern).
    Ok(super::respond_versioned(h, &resp))
}

/// The `versioned_party_revision_history` operation.
pub(crate) async fn revision_history(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedPartyGetParams::from_request,
        VersionedPartyGetParams::PARAMS,
    )?;
    let vo = p.versioned_object_uid.clone();
    let resp = state
        .backend()
        .versioned_party_revision_history(super::object_of(&vo)?)
        .await?;
    // ETag from the addressed VERSIONED_OBJECT (a REVISION_HISTORY has
    // no uid of its own); Last-Modified from the most recent item.
    Ok(super::read_versioned(h, &vo, &resp.body))
}

/// The `versioned_party_version_get_at_time` operation.
pub(crate) async fn version_get_at_time(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedPartyVersionGetAtTimeParams::from_request,
        VersionedPartyVersionGetAtTimeParams::PARAMS,
    )?;
    // 200_VERSION_at_time analogue: the served VERSION's ETag +
    // Last-Modified (its commit instant rides the response metadata).
    let resp = state
        .backend()
        .versioned_party_version_get_at_time(
            super::object_of(&p.versioned_object_uid)?,
            p.version_at_time,
        )
        .await?;
    Ok(super::respond_versioned(h, &resp))
}

/// The `versioned_party_version_get_by_id` operation.
pub(crate) async fn version_get_by_id(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        VersionedPartyVersionGetByIdParams::from_request,
        VersionedPartyVersionGetByIdParams::PARAMS,
    )?;
    let vo = p.versioned_object_uid.clone();
    let resp = state
        .backend()
        .versioned_party_version_get_by_id(
            super::object_of(&vo)?,
            parse_version_uid(&p.version_uid)?.1,
        )
        .await?;
    // ETag from VERSION.uid.value; Last-Modified from the served
    // ORIGINAL_VERSION's commit_audit.time_committed.
    Ok(super::read_versioned(h, &vo, &resp.body))
}
