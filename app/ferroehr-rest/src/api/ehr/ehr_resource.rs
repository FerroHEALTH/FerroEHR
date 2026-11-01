// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR` resource + EHR-level item tags.
//!
//! Spec: `docs/specs/openehr/ITS-REST/specifications/docs/ehr/` (EHR API) +
//! `specifications/operations/{ehr_get_by_subject,ehr_create,
//! ehr_create_with_id,ehr_get_by_id,ehr_tags_get}.yaml`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::Response;
use http::StatusCode;
use serde_json::Value;

use openehr_its::rest::generated::ehr::{
    EhrCreateParams, EhrCreateWithIdParams, EhrGetByIdParams, EhrGetBySubjectParams,
    EhrTagsGetParams,
};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::{Ehr, EhrStatus};

use ferroehr::ids::EhrId;
use ferroehr::service::response::{ResourceMeta, ServiceResponse};

use crate::api::RequestParts;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::overview::version_id::parse_ehr_id;
use crate::state::AppState;

pub(super) async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let ok = StatusCode::OK;
    // The configured base path, for building `Location` URLs.
    let base = state.config().rest.base_path.clone();

    // EHR / EHR_STATUS have no Simplified-Formats mapping (they are not
    // templated) — a simplified Content-Type/Accept is rejected uniformly.
    crate::formats::dispatch::guard_non_templated(h)?;

    match op {
        "ehr_get_by_subject" => {
            let p = parts.decode(
                EhrGetBySubjectParams::from_request,
                EhrGetBySubjectParams::PARAMS,
            )?;
            let read = state
                .backend()
                .ehr_read_for_subject(&p.subject_id, &p.subject_namespace)
                .await?;
            // 200_EHR: the weak ETag off `EHR.ehr_id.value` — the ITS-REST
            // overview's own example source for a resource with a unique
            // state identifier (Requests_and_responses.md §"ETag and
            // Last-Modified": the value "is usually taken from e.g.
            // VERSIONED_OBJECT.uid.value, VERSION.uid.value,
            // EHR.ehr_id.value"). No Location on a GET (§Location).
            Ok(ehr_read_response(h, ok, &read))
        }
        "ehr_create" => {
            let _p = parts.decode(EhrCreateParams::from_request, EhrCreateParams::PARAMS)?;
            let status = negotiate::optional_rm_value::<EhrStatus>(h, &parts.body)?;
            // The service returns the created EHR's own resource metadata
            // (ehr_id + creation instant) — the write path never rebuilds a
            // metadata-less envelope, so `Last-Modified` survives to the wire.
            let committal = create_committal(h)?;
            let (ehr_id, meta) = state
                .backend()
                .create_ehr_meta(status, committal.as_ref())
                .await?;
            ehr_write_response(&state, h, &base, ehr_id, meta).await
        }
        "ehr_create_with_id" => {
            let p = parts.decode(
                EhrCreateWithIdParams::from_request,
                EhrCreateWithIdParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let status = negotiate::optional_rm_value::<EhrStatus>(h, &parts.body)?;
            let committal = create_committal(h)?;
            let meta = state
                .backend()
                .create_ehr_with_id_meta(ehr_id, status, committal.as_ref())
                .await?;
            ehr_write_response(&state, h, &base, ehr_id, meta).await
        }
        "ehr_get_by_id" => {
            let p = parts.decode(EhrGetByIdParams::from_request, EhrGetByIdParams::PARAMS)?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let read = state.backend().ehr_read(ehr_id).await?;
            Ok(ehr_read_response(h, ok, &read))
        }
        "ehr_tags_get" => {
            let p = parts.decode(EhrTagsGetParams::from_request, EhrTagsGetParams::PARAMS)?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let tags = state
                .backend()
                .ehr_tags_get(
                    ehr_id,
                    p.tag_key.as_deref(),
                    p.tag_value.as_deref(),
                    p.tag_target_path.as_deref(),
                )
                .await?;
            Ok(negotiate::respond(
                h,
                ok,
                &openehr_its::json::to_canonical_value(&tags),
            ))
        }
        other => Err(RestError::from(ApiError::Internal(format!(
            "unrouted ehr operation: {other}"
        )))),
    }
}

/// The committal metadata of an EHR create, when the request carried the
/// `openehr-version` / `openehr-audit-details` headers.
///
/// EHR creation is a commit on change-controlled content: "the result should
/// be a root EHR object, an EHR Status object, and an EHR Access object …
/// created and committed in a Contribution" (RM ehr `master04-ehr_package.adoc`
/// §EHR Creation), and the `EHR_STATUS` is one of the change-controlled
/// resources the merge rule names. So the ITS-REST MUST applies to `POST /ehr`
/// and `PUT /ehr/{ehr_id}` exactly as it does to a COMPOSITION write
/// (`Requests_and_responses.md` §"openehr-version and openehr-audit-details":
/// the headers MUST be accepted on `PUT`, `POST` and `DELETE`, and "whatever
/// is provided it MUST be merged with the default VERSION and
/// `VERSION.audit_details` attributes on commit runtime").
///
/// The merge starts from the authenticated principal as the committer, so an
/// unsupplied `committer` keeps the server default instead of being
/// overwritten.
///
/// # Errors
/// [`ApiError::BadRequest`] when a committal header carries a malformed
/// identifier.
fn create_committal(
    headers: &http::HeaderMap,
) -> Result<Option<ferroehr::service::version_update::Committal>, ApiError> {
    crate::overview::committal::committal_commit(headers, super::committer_proxy())
}

/// Render an EHR read (`200_EHR`) with the weak `ETag` carrying
/// `EHR.ehr_id.value` and the `Last-Modified` of the latest change to the
/// served EHR: ITS-REST overview §"`ETag` and Last-Modified", "Both `ETag` and
/// `Last-Modified` SHOULD be included in responses for VERSION,
/// `VERSIONED_OBJECT`, or other resources that have versioning or unique state
/// identifiers". The EHR's creating commit dates it until a folder hierarchy
/// joins or leaves `EHR.folders`.
fn ehr_read_response(h: &http::HeaderMap, status: StatusCode, read: &ServiceResponse) -> Response {
    let mut out = negotiate::respond_rm::<Ehr>(h, status, &read.body, super::EHR_ROOT_TAG);
    if let Some(id) = read
        .body
        .get("ehr_id")
        .and_then(|i| i.get("value"))
        .and_then(Value::as_str)
    {
        negotiate::set_etag(&mut out, id);
    }
    if let Some(at) = read.meta.as_ref().and_then(|meta| meta.last_modified) {
        negotiate::set_last_modified(&mut out, at);
    }
    out
}

/// Render an EHR create response (`201_EHR)`: `ETag(ehr_id)` +
/// `Last-Modified` + `Location`, with the RM `EHR` body only on
/// `Prefer: return=representation`.
///
/// `meta` comes straight from the service create (the `ehr_id` plus the
/// creating CONTRIBUTION's commit instant), so the `Last-Modified` the
/// ITS-REST overview §"`ETag` and Last-Modified" asks for is not lost between
/// the commit and the wire.
async fn ehr_write_response(
    state: &AppState,
    h: &http::HeaderMap,
    base: &str,
    ehr_id: EhrId,
    meta: ResourceMeta,
) -> Result<Response, RestError> {
    let body = if negotiate::prefers_representation(h) {
        // `ehr_created_object` serves the just-committed EHR body from the
        // create-time stash (built from the commit results), avoiding the
        // `ehr_summary` re-read; it falls back to a full read on a stash miss.
        state.backend().ehr_created_object(ehr_id).await?
    } else {
        Value::Null
    };
    let resp = ServiceResponse::new(body, meta);
    Ok(negotiate::write_rm::<Ehr>(
        h,
        base,
        StatusCode::CREATED,
        StatusCode::CREATED,
        None,
        &resp,
        super::EHR_ROOT_TAG,
    ))
}
