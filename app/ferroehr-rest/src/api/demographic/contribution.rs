// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic CONTRIBUTION operations —
//! `operations/demographic_contribution_create.yaml`,
//! `demographic_contribution_get.yaml`. Canonical content negotiation; the
//! commit body is a `NewContribution` wrapper (`schemas/demographic/NewContribution.yaml`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the handler carries the canonical fragment the \
              negotiate seam produced once (stored-content serving / commit interior)"
)]

use axum::response::Response;
use http::StatusCode;

use openehr_its::rest::generated::demographic::ContributionGetParams;
use openehr_its::rest::runtime::ApiError;

use crate::api::RequestParts;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;
use ferroehr::service::response::ServiceResponse;

/// The `contribution_*` operations.
pub(super) async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let base = state.config().server.base_path.clone();

    match op {
        "contribution_create" => {
            // A CONTRIBUTION commit is a `NewContribution` wrapper DTO, JSON only.
            let body = negotiate::json_value(h, &parts.body)?;
            let resp = state
                .backend()
                .demographic_contribution_create(body)
                .await?;
            // 201_demographic_CONTRIBUTION + ETag(contribution_uid)/Location;
            // body per Prefer (oneOf[Contribution, Identifier]).
            Ok(write_shared(
                h,
                &base,
                "contribution",
                StatusCode::CREATED,
                StatusCode::CREATED,
                &resp,
            ))
        }
        "contribution_get" => {
            let p = parts.decode(
                ContributionGetParams::from_request,
                ContributionGetParams::PARAMS,
            )?;
            let uid = p.contribution_uid.clone();
            let resp = state
                .backend()
                .demographic_contribution_get(p.contribution_uid)
                .await?;
            // Weak ETag (the contribution uid — the same identity the 201's
            // ETag carries) + Last-Modified from `audit.time_committed`,
            // mirroring the EHR sibling's adjudicated reading of the
            // overview §"ETag and Last-Modified" SHOULD (a CONTRIBUTION is
            // immutable and uniquely identified; the released 200_CONTRIBUTION
            // declares neither header).
            let mut meta = ferroehr::service::response::ResourceMeta::new(String::new(), uid);
            if let Some(at) = resp
                .body
                .get("audit")
                .and_then(|a| a.get("time_committed"))
                .and_then(|t| t.get("value"))
                .and_then(serde_json::Value::as_str)
                .and_then(|raw| raw.parse::<jiff::Timestamp>().ok())
            {
                meta = meta.with_last_modified(at);
            }
            let mut out = negotiate::respond(h, StatusCode::OK, &resp.body);
            super::set_versioning_headers(&mut out, Some(&meta));
            Ok(out)
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted demographic contribution operation: {other}"
        )))),
    }
}

/// A create response for a JSON-only payload (CONTRIBUTION), honouring `Prefer`
/// and setting the demographic `ETag` + the creation `Location` (overview
/// §Location — `201 Created` is exactly the case it scopes the header to).
///
/// The body + `Preference-Applied` go through the shared
/// [`negotiate::write_negotiated`] seam: `return=representation` → the
/// CONTRIBUTION, `return=identifier` → the `{uid}` body at a `201`/`200`
/// (never `204`, §"Prefer only identifier"), otherwise the applied default
/// `return=minimal`.
fn write_shared(
    h: &http::HeaderMap,
    base: &str,
    segment: &str,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    resp: &ServiceResponse,
) -> Response {
    let mut out = negotiate::write_negotiated(
        h,
        minimal_status,
        repr_status,
        resp.meta.as_ref().map(|m| m.uid.as_str()),
        |status| negotiate::respond(h, status, &resp.body),
    );
    super::set_write_headers(&mut out, base, segment, resp.meta.as_ref());
    out
}
