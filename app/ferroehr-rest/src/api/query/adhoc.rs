// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Ad-hoc AQL execution — `GET`/`POST /query/aql` (`query_execute_adhoc_query`
//! + `_body`).
//!
//! An ad-hoc query supplies the AQL text directly (`Query_types.md` §Ad-hoc
//! (non-stored) queries; `schemas/query/AdhocQueryExecute.yaml`: `q` required,
//! plus `offset`/`fetch`/`query_parameters`). The `q` and the paging/scope
//! parameters arrive either in the query string (`GET`) or the JSON body
//! (`POST`, `AdhocQueryExecute`); this module normalizes both into an
//! [`AqlQueryRequest`] and calls the [`QueryService`] ad-hoc seam.
//!
//! [`QueryService`]: ferroehr::service::FerroEhrService

use openehr_its::rest::generated::query::{AdhocQueryExecute, QueryExecuteAdhocQueryParams};

use ferroehr::service::query::request::{AqlQueryRequest, QueryOutcome};

use super::response::{self, QueryScope};
use crate::api::RequestParts;
use crate::overview::error::RestError;
use crate::params;
use crate::state::AppState;

/// The `query_execute_adhoc_query` operation: GET /query/aql — `q` + paging/scope in the query string.
///
/// The single wire `ehr_id` (query parameter
/// or `openehr-ehr-id` header — resolved for both methods by
/// [`response::ehr_scope`], `Request.md` §About the `ehr_id`
/// parameter) is collected into the one-element [`AqlQueryRequest::ehr_ids`]
/// scope (a list scope is our own extension; the conformant wire binds one
/// `ehr_id`).
pub(super) async fn get(
    state: &AppState,
    parts: &RequestParts,
    scope: &QueryScope,
) -> Result<QueryOutcome, RestError> {
    let h = &parts.headers;
    let _q = parts.query.as_deref();
    let p = parts.decode(
        QueryExecuteAdhocQueryParams::from_request,
        QueryExecuteAdhocQueryParams::PARAMS,
    )?;
    // Named parameter binds per Request.md §Query parameters (the
    // documented GET form); the JSON-object `query_parameters` stays
    // an accepted superset.
    let parameters = params::aql_binds(p.query_parameters)?;
    let request = scope.apply(AqlQueryRequest {
        ehr_ids: response::ehr_scope(p.ehr_id, h)?,
        offset: p.offset,
        fetch: p.fetch,
        parameters,
        ..Default::default()
    });
    Ok(state.backend().execute_aql(&p.q, None, &request).await?)
}

/// The `query_execute_adhoc_query_body` operation: POST /query/aql — `AdhocQueryExecute` body (`q` + paging/params); `ehr_id` still from the query string / header.
pub(super) async fn post(
    state: &AppState,
    parts: &RequestParts,
    scope: &QueryScope,
) -> Result<QueryOutcome, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let body: AdhocQueryExecute = response::decode_body(h, &parts.body)?;
    // The docs-text SHOULD-list draws no GET/POST distinction — the
    // URL forms are accepted here too; a body-vs-URL conflict is a
    // 400 (the same rule the two `ehr_id` carriers follow).
    let offset = response::merge_body_and_url_i64(body.offset, q, "offset")?;
    let fetch = response::merge_body_and_url_i64(body.fetch, q, "fetch")?;
    let parameters =
        response::merge_body_and_url_parameters(body.query_parameters.unwrap_or_default(), q)?;
    let request = scope.apply(AqlQueryRequest {
        ehr_ids: response::ehr_scope(params::query_param(q, "ehr_id"), h)?,
        offset,
        fetch,
        parameters,
        ..Default::default()
    });
    Ok(state.backend().execute_aql(&body.q, None, &request).await?)
}
