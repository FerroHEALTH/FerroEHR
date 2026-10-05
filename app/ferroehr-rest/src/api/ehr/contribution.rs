// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `CONTRIBUTION` resource.
//!
//! Spec: `docs/specs/openehr/ITS-REST/specifications/docs/ehr/` (CONTRIBUTION) +
//! `specifications/operations/{contribution_create,contribution_get}.yaml`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::Response;
use http::StatusCode;

use openehr_its::rest::generated::ehr::{ContributionCreateParams, ContributionGetParams};
use openehr_its::rest::runtime::ApiError;

use crate::api::RequestParts;
use crate::negotiate::{AppliedPreference, WireFormat};
use crate::overview::error::RestError;
use crate::overview::version_id::{parse_ehr_id, parse_uuid};
use crate::state::AppState;
use crate::{negotiate, params};

/// The representations a CONTRIBUTION endpoint negotiates.
///
/// The envelope is always canonical JSON and the Simplified types select the
/// inner `versions[i].data` form ("the CONTRIBUTION envelope itself remains
/// canonical JSON", `contribution_create.yaml` §Simplified Formats). XML is not
/// offered: the published XSDs declare no global CONTRIBUTION document element,
/// so an XML `Accept` "cannot fulfill this aspect of the request" and takes the
/// 406 MUST of overview `Resources.md` §XML Format.
const CONTRIBUTION_FORMATS: &[WireFormat] = &[
    WireFormat::CanonicalJson,
    WireFormat::Flat,
    WireFormat::Structured,
];

#[expect(
    clippy::too_many_lines,
    reason = "create + get, each with an envelope and a simplified-format branch: \
              a flat match keeps every operation's wire behaviour in one place"
)]
pub(super) async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let ok = StatusCode::OK;
    let created = StatusCode::CREATED;
    let base = state.config().server.base_path.clone();

    match op {
        "contribution_create" => {
            let p = parts.decode(
                ContributionCreateParams::from_request,
                ContributionCreateParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            // TODO(#3550): commit through the typed `NewContribution`; the raw
            // body stays while a member's `commit_audit` may inherit `committer`
            // from the CONTRIBUTION audit (RM common master06 §Committal), which
            // the generated `UpdateAudit` requires.
            // NOTE: a CONTRIBUTION commit is a wrapper DTO, not a single
            // canonical RM value with a defined canonical-XML shape — so it is
            // accepted as JSON only.
            let body = match negotiate::content_type_format(h) {
                Some(WireFormat::CanonicalJson) => negotiate::json_value(h, &parts.body)?,
                Some(fmt @ (WireFormat::Flat | WireFormat::Structured)) => {
                    crate::formats::dispatch::contribution_from_simplified(
                        &state,
                        h,
                        &parts.body,
                        fmt,
                    )
                    .await?
                }
                _ => {
                    return Err(RestError(ApiError::UnsupportedMediaType(
                        "a CONTRIBUTION is committed as application/json, \
                         application/openehr.wt.flat+json, or \
                         application/openehr.wt.structured+json"
                            .to_owned(),
                    )));
                }
            };
            // Under `return=minimal` the response is headers-only, so the
            // composite body and its post-commit re-read are skipped
            // (`Requests_and_responses` §Representation details).
            let want_repr = negotiate::prefers_representation(h);
            let resp = state
                .backend()
                .ehr_contribution_commit(ehr_id, body, want_repr)
                .await?;
            // 201_CONTRIBUTION: ETag(contribution_uid) + Location; body per
            // Prefer, in the negotiated inner form on a Simplified `Accept`.
            match negotiate::resolve_accept(h, CONTRIBUTION_FORMATS, WireFormat::CanonicalJson) {
                Some(fmt @ (WireFormat::Flat | WireFormat::Structured)) if want_repr => {
                    let mut out = crate::formats::dispatch::contribution_to_simplified(
                        &state, created, &resp.body, fmt,
                    )
                    .await?;
                    if let Some(meta) = &resp.meta {
                        negotiate::set_resource_headers(
                            &mut out,
                            &base,
                            Some("contribution"),
                            meta,
                        );
                    }
                    // This branch is `want_repr` only, so the applied
                    // preference is representation (§Representation details
                    // negotiation).
                    negotiate::set_preference_applied(&mut out, AppliedPreference::Representation);
                    Ok(out)
                }
                _ => Ok(negotiate::write_json(
                    h,
                    &base,
                    created,
                    created,
                    Some("contribution"),
                    &resp,
                )),
            }
        }
        "contribution_get" => {
            let p = parts.decode(
                ContributionGetParams::from_request,
                ContributionGetParams::PARAMS,
            )?;
            let ehr_id = parse_ehr_id(&p.ehr_id)?;
            let cid = parse_uuid(&p.contribution_uid, "contribution id")?;
            // `Prefer: resolve_refs` (Requests_and_responses §Representation
            // details negotiation): versions as full ORIGINAL_VERSIONs.
            let body = if negotiate::prefers_resolve_refs(h) {
                state
                    .backend()
                    .get_contribution_resolved(ehr_id, cid)
                    .await?
            } else {
                state.backend().get_contribution(ehr_id, cid).await?
            };
            // A weak `ETag` on the contribution uid plus `Last-Modified` from
            // `audit.time_committed`: the released 200_CONTRIBUTION declares
            // neither, so this is our reading of the reach of the SHOULD in
            // overview §"ETag and Last-Modified".
            let meta = {
                let mut m = ferroehr::service::response::ResourceMeta::new(
                    p.ehr_id.clone(),
                    cid.to_string(),
                );
                if let Some(at) = body
                    .get("audit")
                    .and_then(|a| a.get("time_committed"))
                    .and_then(|t| t.get("value"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(|raw| raw.parse::<jiff::Timestamp>().ok())
                {
                    m = m.with_last_modified(at);
                }
                m
            };
            // The envelope stays canonical JSON; a Simplified `Accept`
            // serializes each `versions[i].data` COMPOSITION into the requested
            // inner form (`contribution_get.yaml` §Simplified Formats), and a
            // non-COMPOSITION inner payload is a 406.
            let mut out =
                match negotiate::resolve_accept(h, CONTRIBUTION_FORMATS, WireFormat::CanonicalJson)
                {
                    Some(fmt @ (WireFormat::Flat | WireFormat::Structured)) => {
                        crate::formats::dispatch::contribution_to_simplified(&state, ok, &body, fmt)
                            .await?
                    }
                    _ => negotiate::respond(h, ok, &body),
                };
            negotiate::set_versioning_headers(&mut out, &meta);
            Ok(out)
        }
        "contribution_list" => {
            // NOTE: no openEHR spec governs this — our own extension; the
            // contract defines only the by-uid CONTRIBUTION GET.
            let ehr_id_raw = parts.path.get("ehr_id").ok_or_else(|| {
                RestError(ApiError::BadRequest(
                    "missing path parameter 'ehr_id'".to_owned(),
                ))
            })?;
            let ehr_id = parse_ehr_id(ehr_id_raw)?;
            // `offset` defaults to 0 (negative → 0); `fetch` defaults to 20 and is
            // capped at 100 (a non-positive value falls back to the default).
            let offset = params::query_param(q, "offset")
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|o| *o >= 0)
                .unwrap_or(0);
            let fetch = params::query_param(q, "fetch")
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|f| *f > 0)
                .unwrap_or(20)
                .min(100);
            let body = state
                .backend()
                .ehr_contribution_list_page(ehr_id, offset, fetch)
                .await?;
            // A JSON-only DTO (no spec-typed canonical-XML shape) → `respond`.
            Ok(negotiate::respond(h, ok, &body))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted ehr operation: {other}"
        )))),
    }
}
