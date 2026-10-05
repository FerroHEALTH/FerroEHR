// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the `query` API group — ad-hoc + stored AQL execution
//! (ITS-REST 1.1.0 QUERY API). This module is the operation match: it runs the
//! ABAC pre-filter (`extensions::abac`), routes each `operationId` to the
//! ad-hoc ([`super::adhoc`]), stored ([`super::stored`]) or cohort
//! ([`super::cohort`]) execution path, runs
//! the ABAC post-check, and renders the assembled `RESULT_SET` (with its
//! spec-mandated `ETag`) via [`super::response`].
//!
//! Spec: the six released operations, their parameter lists, and their
//! `RESULT_SET` response are `query-codegen.openapi.yaml` +
//! `docs/query/{Request,Response}.md`. The seventh, `query_execute_cohort`
//! ([`super::cohort`]), is our own extension — no openEHR spec governs it.
//! The request-normalization + response-rendering shared by both paths lives in
//! [`super::response`].

use axum::response::{IntoResponse, Response};

use openehr_its::rest::runtime::ApiError;

use crate::api::{BoxResponse, RequestParts};
use crate::overview::error::RestError;

use super::response::{self, QueryScope};
use super::{adhoc, cohort, stored};
use crate::state::AppState;

pub(crate) fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        run(state, op, parts)
            .await
            .unwrap_or_else(IntoResponse::into_response)
    })
}

async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    // ABAC: the patient subject-scope pre-filter + collection flag. A
    // missing configured patient claim is a ready 403. The PEP entry points
    // (`extensions::access::pep::{query_pre,query_post}`) are `pub(crate)`, so
    // this cross-module dispatcher calls them directly.
    let (subject_scope, collect) = match crate::extensions::access::pep::query_pre(&state, op) {
        Ok(prep) => prep,
        Err(deny) => return Ok(deny),
    };
    // EHR_ACCESS: the per-EHR decision the path routes apply, held over every
    // EHR the query could reach (RM ehr `ehr_access.adoc` §`EHR_ACCESS` Class).
    let ehr_access = match crate::extensions::access::ehr_access::query_scope(&state).await {
        Ok(ehr_access) => ehr_access,
        Err(deny) => return Ok(*deny),
    };
    let scope = QueryScope {
        subject_scope,
        collect,
        ehr_access,
    };

    let outcome = match op {
        "query_execute_adhoc_query" | "query_execute_adhoc_query_body" => {
            adhoc::execute(&state, op, &parts, &scope).await?
        }
        "query_execute_stored_query"
        | "query_execute_stored_query_body"
        | "query_execute_stored_query_version"
        | "query_execute_stored_query_version_body" => {
            stored::execute(&state, op, &parts, &scope).await?
        }
        // The cohort extension: the same ABAC pre-filter, post-check, rendering
        // and access extensions as the released operations, over a scope
        // resolved across the pseudonymisation boundary rather than supplied.
        "query_execute_cohort" => {
            let outcome = cohort::execute(&state, &parts, &scope).await?;
            let suppressed = outcome.suppressed;
            let mut query = outcome.query;
            if suppressed {
                // A suppressed response discloses no EHR, so the access record
                // must name none: the served-EHR extension is what the ATNA
                // layer writes one access per served record from.
                query.served_ehrs.clear();
            }
            query
        }
        other => {
            return Err(RestError(ApiError::Internal(format!(
                "unrouted query operation: {other}"
            ))));
        }
    };

    // ABAC query post-check: PDP fan-out over the touched template set.
    if let Err(deny) = crate::extensions::access::pep::query_post(&state, op, &outcome).await {
        return Ok(deny);
    }

    let mut resp = response::respond_result_set(&parts.headers, &outcome.result_set);
    // The access log records one access per EHR the statement served, plus the
    // statement's own operation record. Both ride the response extensions the
    // ATNA middleware reads, so the caller identity stays in one place.
    resp.extensions_mut()
        .insert(crate::system_log::middleware::AuditObject {
            ehr_id: None,
            uid: None,
            result_count: Some(outcome.served_rows),
            domain: None,
            origins: outcome.served_origins.clone(),
            origin_count: (!outcome.served_origins.is_empty()).then_some(outcome.origin_count),
        });
    if !outcome.served_ehrs.is_empty() {
        resp.extensions_mut()
            .insert(crate::system_log::middleware::AuditServedEhrs(
                outcome.served_ehrs,
            ));
    }
    Ok(resp)
}
