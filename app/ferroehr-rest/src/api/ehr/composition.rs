// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `COMPOSITION` resource + its item tags.
//!
//! Spec: `docs/specs/openehr/ITS-REST/specifications/docs/ehr/` (COMPOSITION) +
//! `specifications/operations/{composition_create,composition_get,
//! composition_update,composition_delete,composition_tags_get,
//! composition_tags_update,composition_tags_delete}.yaml`.
//!
//! The FLAT/STRUCTURED converters live in `crate::formats::dispatch` (the
//! Simplified-Formats wire adapter) and are called by their full path.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::Response;
use http::StatusCode;
use serde_json::Value;

use openehr_base::prelude::UidBasedId;

use openehr_its::rest::generated::ehr::{
    CompositionCreateParams, CompositionDeleteParams, CompositionGetParams,
    CompositionTagsDeleteParams, CompositionTagsGetParams, CompositionTagsUpdateParams,
    CompositionUpdateParams,
};
use openehr_its::rest::runtime::ApiError;
use openehr_rm::prelude::Composition;

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::response::{RawServiceResponse, ReadBody, ResourceMeta, ServiceResponse};
use ferroehr::versioning::change::Committed;

use ferroehr::versioning::object_version_id::parse_uid_based_id;

use crate::api::RequestParts;
use crate::api::item_tags;
use crate::negotiate::{AppliedPreference, WireFormat};
use crate::overview::error::RestError;
use crate::overview::version_id::{
    object_id_uuid, parse_ehr_id, parse_version_uid, require_if_match,
};
use crate::state::AppState;
use crate::{negotiate, params};

/// The representations a COMPOSITION endpoint negotiates: canonical JSON/XML
/// plus the two Simplified data-instance forms (`Accept_LOCATABLE` /
/// `ContentType_LOCATABLE`). The mapping tables (`simplified_formats/master05`)
/// govern COMPOSITION content, so all four are supported here.
const COMPOSITION_FORMATS: &[WireFormat] = &[
    WireFormat::CanonicalJson,
    WireFormat::CanonicalXml,
    WireFormat::Flat,
    WireFormat::Structured,
];

/// Decode a COMPOSITION request body per its `Content-Type`: canonical JSON/XML
/// through the RM decoder, FLAT/STRUCTURED through the Simplified-Formats
/// adapter. A `Content-Type` outside `ContentType_LOCATABLE` (e.g. the
/// Web Template media type, a deprecated `…schema+json`, or an unknown type) is
/// `415` (Resources.md §Simplified Formats MUST).
async fn decode_composition_body(
    state: &AppState,
    h: &http::HeaderMap,
    body: &bytes::Bytes,
) -> Result<Composition, RestError> {
    match negotiate::content_type_format(h) {
        Some(WireFormat::CanonicalJson | WireFormat::CanonicalXml) => {
            Ok(negotiate::rm_value::<Composition>(h, body)?)
        }
        // The converted fragment re-enters through the same strict reader as a
        // canonical body: one decode, one refusal class.
        Some(WireFormat::Flat) => typed_composition(
            &crate::formats::dispatch::composition_from_flat(state, h, body).await?,
        ),
        Some(WireFormat::Structured) => typed_composition(
            &crate::formats::dispatch::composition_from_structured(state, h, body).await?,
        ),
        _ => Err(RestError(ApiError::UnsupportedMediaType(
            "a COMPOSITION is committed as application/json, application/xml, \
             application/openehr.wt.flat+json, or application/openehr.wt.structured+json"
                .to_owned(),
        ))),
    }
}

/// Re-types a converted Simplified-Formats body as the RM `COMPOSITION` the
/// commit seam takes.
fn typed_composition(value: &Value) -> Result<Composition, RestError> {
    // NOTE: a post-conversion strict-reader refusal is the template-mediated
    // 422 class (master04 §Validation; `responses/422.yaml`).
    openehr_its::json::from_canonical_value(value).map_err(|e| {
        let detail = e.to_string();
        let hint = if detail.contains("invalid type") {
            " (value does not match the expected data type for the field)"
        } else {
            ""
        };
        RestError(ApiError::Unprocessable(format!(
            "the simplified-format body did not convert to a valid COMPOSITION: {detail}{hint}"
        )))
    })
}

pub(super) async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    match op {
        "composition_create" => create(state, parts).await,
        "composition_get" => get(state, parts).await,
        "composition_update" => Box::pin(update(state, parts)).await,
        "composition_delete" => delete(state, parts).await,
        "composition_tags_get" => tags_get(state, parts).await,
        "composition_tags_update" => tags_update(state, parts).await,
        "composition_tags_delete" => tags_delete(state, parts).await,
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted ehr operation: {other}"
        )))),
    }
}

/// `composition_create` — commit a new COMPOSITION into an EHR.
///
/// # Errors
/// The parameter, body-decode, commit and item-tag rejections the operation
/// declares.
async fn create(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let created = StatusCode::CREATED;
    let base = state.config().server.base_path.clone();
    let p = CompositionCreateParams::from_request(
        &parts.route_for(CompositionCreateParams::PARAMS),
        q,
        h,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let body = decode_composition_body(&state, h, &parts.body).await?;
    let uv = super::mk_update_version(
        h,
        body,
        super::CHANGE_CREATION,
        "COMPOSITION creation",
        None,
    )?;
    // Tags are judged before the commit, so a defective tag refuses while
    // nothing is durable; the write itself stays post-commit.
    let pending_tags = item_tags::pending(h)?;
    let committed = state
        .backend()
        .create_composition(ehr_id, uv)
        .await
        .map_err(|e| RestError::from(ApiError::from(e)))?;
    let uid = committed.version_uid();
    let stored_tags = item_tags::persist(
        &state,
        item_tags::TagTarget::EhrContent {
            ehr_id,
            target_type: "COMPOSITION",
        },
        &uid,
        pending_tags,
    )
    .await?;
    let meta = commit_meta(ehr_id, uid, &committed);
    let mut resp =
        composition_write_response(&state, h, &base, ehr_id, meta, created, created).await?;
    stored_tags.echo(&mut resp);
    Ok(resp)
}

/// `composition_get` — serve a COMPOSITION at a version, an instant, or its
/// latest.
///
/// # Errors
/// The parameter and read rejections the operation declares, plus a `406` for
/// an unfulfillable `Accept`.
async fn get(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let no_content = StatusCode::NO_CONTENT;
    let base = state.config().server.base_path.clone();
    let p =
        CompositionGetParams::from_request(&parts.route_for(CompositionGetParams::PARAMS), q, h)?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let uid = parse_uid_based_id(&p.uid_based_id)?;
    // A bare COMPOSITION carries no `commit_audit`, so `Last-Modified`
    // (Requests_and_responses.md §"ETag and Last-Modified") comes from the
    // version row the service reads alongside the body.
    let read = if let Some(ovid) = uid.version {
        state
            .backend()
            .composition_at_version_response(ehr_id, ovid)
            .await?
    } else if p.version_at_time.is_some() {
        state
            .backend()
            .composition_at_time_response(ehr_id, uid.vo_id, p.version_at_time)
            .await?
    } else {
        state
            .backend()
            .composition_latest_response(ehr_id, uid.vo_id)
            .await?
    };
    let RawServiceResponse { body, meta } = read;
    // A plain JSON accept with no multimedia expansion serves the stored
    // bytes verbatim (the body is uid-stamped at commit); the rest parse.
    let expand = params::query_param(q, "expand_multimedia").as_deref() == Some("true");
    let accept = negotiate::resolve_accept(h, COMPOSITION_FORMATS, WireFormat::CanonicalJson);
    let mut body = match body {
        ReadBody::RawJson(text) if !expand && accept == Some(WireFormat::CanonicalJson) => {
            let mut out = negotiate::raw_json_body(ok, text);
            if let Some(meta) = &meta {
                negotiate::set_versioning_headers(&mut out, meta);
            }
            return Ok(out);
        }
        other => other.into_value().map_err(|e| {
            RestError::from(crate::overview::error::internal_fault(
                "re-parse the stored composition body",
                &e,
            ))
        })?,
    };
    // A deleted version resolves to a null body → 204 No Content
    // (composition_get.yaml `204_because_deleted*`).
    if body.is_null() {
        return Ok(negotiate::empty(no_content));
    }
    body = super::expand_multimedia_if_requested(&state, q, body).await?;
    // "The `ETag` value is independent of its resource serialization format
    // (JSON/XML)" (§"ETag and Last-Modified"), so the simplified
    // representations carry the version-identity headers too.
    match accept {
        Some(WireFormat::Flat) => {
            let mut out =
                crate::formats::dispatch::composition_flat_response(&state, ok, &body).await?;
            if let Some(meta) = &meta {
                negotiate::set_versioning_headers(&mut out, meta);
            }
            return Ok(out);
        }
        Some(WireFormat::Structured) => {
            let mut out =
                crate::formats::dispatch::composition_structured_response(&state, ok, &body)
                    .await?;
            if let Some(meta) = &meta {
                negotiate::set_versioning_headers(&mut out, meta);
            }
            return Ok(out);
        }
        _ => {}
    }
    // `ETag` and `Last-Modified` both SHOULD accompany a resource with a
    // unique state identifier (§"ETag and Last-Modified").
    let resp = ServiceResponse { body, meta };
    Ok(negotiate::read_rm::<Composition>(
        h,
        &base,
        Some("composition"),
        &resp,
        "composition",
    ))
}

/// `composition_update` — commit a new version of a COMPOSITION.
///
/// # Errors
/// The parameter, body-decode, precondition and commit rejections the
/// operation declares.
async fn update(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let no_content = StatusCode::NO_CONTENT;
    let base = state.config().server.base_path.clone();
    let p = CompositionUpdateParams::from_request(
        &parts.route_for(CompositionUpdateParams::PARAMS),
        q,
        h,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let uid = parse_uid_based_id(&p.uid_based_id)?;
    let body = decode_composition_body(&state, h, &parts.body).await?;
    // A body `uid` must name the same versioned object as the path.
    // NOTE: OAS-grounded (docs text silent) with no assigned status; the
    // fitting released row is 422 (Requests_and_responses.md §HTTP status codes).
    if let Some(body_uid) = body.uid.as_ref() {
        // A body `uid` names its versioned object through the
        // `OBJECT_VERSION_ID.object_id` (BASE `base_types` §Functions); a
        // HIER_OBJECT_ID names no VERSION, so it fails the comparison.
        let body_vo = match body_uid {
            UidBasedId::ObjectVersionId(ovid) => object_id_uuid(ovid),
            UidBasedId::HierObjectId(_) => None,
        };
        if body_vo != Some(uid.vo_id.0) {
            return Err(ApiError::Unprocessable(format!(
                "the body COMPOSITION.uid {:?} does not identify the \
                         versioned object addressed by the request path ({})",
                body_uid.value(),
                uid.vo_id
            ))
            .into());
        }
    }
    let uv = super::mk_update_version(
        h,
        body,
        super::CHANGE_MODIFICATION,
        "COMPOSITION update",
        Some(require_if_match(&p.if_match)?),
    )?;
    let pending_tags = item_tags::pending(h)?;
    match state
        .backend()
        .update_composition(ehr_id, uid.vo_id, uv)
        .await
    {
        Ok(committed) => {
            let new_uid = committed.version_uid();
            let stored_tags = item_tags::persist(
                &state,
                item_tags::TagTarget::EhrContent {
                    ehr_id,
                    target_type: "COMPOSITION",
                },
                &new_uid,
                pending_tags,
            )
            .await?;
            let meta = commit_meta(ehr_id, new_uid, &committed);
            // The minimal-preference update is 204: "If no response body is
            // returned, the service SHOULD use `204 No Content`" (§"Prefer
            // minimal…"); create stays 201-only (composition_create.yaml).
            let mut resp =
                composition_write_response(&state, h, &base, ehr_id, meta, no_content, ok).await?;
            stored_tags.echo(&mut resp);
            Ok(resp)
        }
        Err(e @ ferroehr::service::error::ServiceError::VersionConflict(_)) => {
            let meta = state
                .backend()
                .composition_latest_meta(ehr_id, uid.vo_id)
                .await
                .ok()
                .flatten();
            Ok(negotiate::error_with_meta(
                ApiError::from(e),
                &base,
                Some("composition"),
                meta.as_ref(),
            ))
        }
        Err(e) => Err(RestError::from(ApiError::from(e))),
    }
}

/// `composition_delete` — commit a `523|deleted|` version of a COMPOSITION.
///
/// # Errors
/// The parameter, precondition and commit rejections the operation declares.
async fn delete(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let base = state.config().server.base_path.clone();
    let p = CompositionDeleteParams::from_request(
        &parts.route_for(CompositionDeleteParams::PARAMS),
        q,
        h,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    // composition_delete.yaml: the uid_based_id MUST be an OBJECT_VERSION_ID
    // (the preceding_version_uid to delete); a bare HIER_OBJECT_ID → 400.
    let ovid = parse_version_uid(&p.uid_based_id)?;
    let vo_id = object_id_uuid(&ovid).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "OBJECT_VERSION_ID object_id is not a UUID: {}",
            p.uid_based_id
        ))
    })?;
    // The operation declares no `If-Match`, but a received one is honoured
    // (overview §"If-Match and accidental overwrites"); the service
    // evaluates it after its own pre-checks (RFC 9110 §13.2.1).
    let volunteered_if_match = match h.get("if-match").and_then(|v| v.to_str().ok()) {
        Some(raw) => Some(require_if_match(raw)?),
        None => None,
    };
    // A DELETE commits a `523|deleted|` version, so the committal headers
    // apply here too (overview §"openehr-version and openehr-audit-details").
    let update_audit =
        crate::overview::committal::committal_audit_for_delete(h, super::committer_proxy())?;
    match state
        .backend()
        .delete_composition(
            ehr_id,
            &ovid,
            volunteered_if_match.as_ref(),
            update_audit.as_ref(),
        )
        .await
    {
        Ok(committed) => {
            // A logical delete commits a `523|deleted|` VERSION, so its
            // commit instant is the resource's last modification
            // (RM common master06 §Logical Deletion).
            let uid = committed.version_uid();
            let resp = ServiceResponse::deleted(commit_meta(ehr_id, uid, &committed));
            Ok(negotiate::deleted_with_headers(
                &base,
                Some("composition"),
                &resp,
            ))
        }
        // The 409 and the volunteered-If-Match 412 both carry the latest
        // `version_uid` in `ETag` (overview §"If-Match and accidental
        // overwrites").
        Err(
            e @ (ferroehr::service::error::ServiceError::Conflict(_)
            | ferroehr::service::error::ServiceError::VersionConflict(_)),
        ) => {
            let meta = state
                .backend()
                .composition_latest_meta(ehr_id, VoId(vo_id))
                .await
                .ok()
                .flatten();
            Ok(negotiate::error_with_meta(
                ApiError::from(e),
                &base,
                Some("composition"),
                meta.as_ref(),
            ))
        }
        Err(e) => Err(RestError::from(ApiError::from(e))),
    }
}

/// `composition_tags_get` — serve the item tags of a COMPOSITION.
///
/// # Errors
/// The parameter and read rejections the operation declares.
async fn tags_get(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let p = CompositionTagsGetParams::from_request(
        &parts.route_for(CompositionTagsGetParams::PARAMS),
        q,
        h,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let tags = state
        .backend()
        .target_tags_get(ehr_id, p.uid_based_id, "COMPOSITION")
        .await?;
    Ok(negotiate::respond(
        h,
        ok,
        &openehr_its::json::to_canonical_value(&tags),
    ))
}

/// `composition_tags_update` — replace the item tags of a COMPOSITION.
///
/// # Errors
/// The parameter rejections the operation declares, plus the body rejections
/// [`crate::api::item_tags::write_body`] states.
async fn tags_update(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let no_content = StatusCode::NO_CONTENT;
    let p = CompositionTagsUpdateParams::from_request(
        &parts.route_for(CompositionTagsUpdateParams::PARAMS),
        q,
        h,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    let body = item_tags::write_body(h, &parts.body)?;
    let tags = state
        .backend()
        .target_tags_replace(ehr_id, p.uid_based_id, "COMPOSITION", body)
        .await?;
    // composition_tags_update.yaml: 200 on `Prefer: return=representation`,
    // 204 when `Prefer` is missing or `return=minimal` (overview §Prefer).
    Ok(negotiate::write_collection(
        h,
        no_content,
        ok,
        &openehr_its::json::to_canonical_value(&tags),
    ))
}

/// `composition_tags_delete` — remove one item tag of a COMPOSITION.
///
/// # Errors
/// The parameter and delete rejections the operation declares.
async fn tags_delete(state: AppState, parts: RequestParts) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let no_content = StatusCode::NO_CONTENT;
    let p = CompositionTagsDeleteParams::from_request(
        &parts.route_for(CompositionTagsDeleteParams::PARAMS),
        q,
        h,
    )?;
    let ehr_id = parse_ehr_id(&p.ehr_id)?;
    state
        .backend()
        .target_tag_delete(ehr_id, p.uid_based_id, "COMPOSITION", p.key)
        .await?;
    Ok(negotiate::empty(no_content))
}

/// Returns the committed version's [`ResourceMeta`]: its `OBJECT_VERSION_ID`
/// and the commit instant ITS-REST derives `Last-Modified` from
/// (`Requests_and_responses.md` §"`ETag` and Last-Modified").
fn commit_meta(ehr_id: EhrId, uid: String, committed: &Committed) -> ResourceMeta {
    ResourceMeta::new(ehr_id.to_string(), uid).with_last_modified(committed.time_committed)
}

/// Render a COMPOSITION create/update response: FLAT/STRUCTURED interop bodies
/// when requested (always the representation), else the canonical
/// `ETag`/`Last-Modified`/`Location` + `Prefer` write response.
async fn composition_write_response(
    state: &AppState,
    h: &http::HeaderMap,
    base: &str,
    ehr_id: EhrId,
    meta: ResourceMeta,
    minimal: StatusCode,
    repr: StatusCode,
) -> Result<Response, RestError> {
    if let Some(fmt @ (WireFormat::Flat | WireFormat::Structured)) =
        negotiate::resolve_accept(h, COMPOSITION_FORMATS, WireFormat::CanonicalJson)
    {
        let ovid = parse_version_uid(&meta.uid)?;
        let body = state
            .backend()
            .get_composition_at_version(ehr_id, ovid)
            .await?;
        let mut out = if fmt == WireFormat::Structured {
            crate::formats::dispatch::composition_structured_response(state, repr, &body).await?
        } else {
            crate::formats::dispatch::composition_flat_response(state, repr, &body).await?
        };
        // `Location` is required on a `201` regardless of body form
        // (RFC 7231 §6.3.2), so the simplified representations get the same
        // version-identity headers.
        negotiate::set_resource_headers(&mut out, base, Some("composition"), &meta);
        // NOTE: a Simplified-Formats commit returns the body in the negotiated
        // form (`Accept` decides, not `Prefer`), so the applied preference is
        // representation (§Representation details negotiation).
        negotiate::set_preference_applied(&mut out, AppliedPreference::Representation);
        return Ok(out);
    }
    let body = if negotiate::prefers_representation(h) {
        let ovid = parse_version_uid(&meta.uid)?;
        state
            .backend()
            .get_composition_at_version(ehr_id, ovid)
            .await?
    } else {
        Value::Null
    };
    let resp = ServiceResponse::new(body, meta);
    Ok(negotiate::write_rm::<Composition>(
        h,
        base,
        minimal,
        repr,
        Some("composition"),
        &resp,
        "composition",
    ))
}
