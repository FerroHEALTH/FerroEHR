// SPDX-FileCopyrightText: Vernum Projecten B.V.
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
use openehr_its::rest::runtime::ApiError;

use ferroehr::service::query::request::{AqlQueryRequest, QueryOutcome};

use super::response::{self, QueryScope};
use crate::api::RequestParts;
use crate::overview::error::RestError;
use crate::params;
use crate::state::AppState;

/// Execute the two ad-hoc operations. The single wire `ehr_id` (query parameter
/// or `openehr-ehr-id` header — resolved for both methods by
/// [`response::ehr_id_from_request`], `Request.md` §About the `ehr_id`
/// parameter) is collected into the one-element [`AqlQueryRequest::ehr_ids`]
/// scope (the SM `List<UUID>` is realized as an extension; the conformant wire
/// binds one `ehr_id`).
pub(super) async fn execute(
    state: &AppState,
    op: &str,
    parts: &RequestParts,
    scope: &QueryScope,
) -> Result<QueryOutcome, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();

    match op {
        // GET /query/aql — `q` + paging/scope in the query string.
        "query_execute_adhoc_query" => {
            let p = QueryExecuteAdhocQueryParams::from_request(
                &parts.route_for(QueryExecuteAdhocQueryParams::PARAMS),
                q,
                h,
            )?;
            // Named parameter binds per Request.md §Query parameters (the
            // documented GET form); the JSON-object `query_parameters` stays
            // an accepted superset.
            let parameters = params::aql_binds(p.query_parameters)?;
            let request = scope.apply(AqlQueryRequest {
                ehr_ids: response::ehr_id_from_request(p.ehr_id, h)?
                    .into_iter()
                    .collect(),
                offset: p.offset,
                fetch: p.fetch,
                parameters,
                ..Default::default()
            });
            Ok(state.backend().execute_ad_hoc_query(p.q, request).await?)
        }
        // POST /query/aql — `AdhocQueryExecute` body (`q` + paging/params);
        // `ehr_id` still from the query string / header.
        "query_execute_adhoc_query_body" => {
            let body: AdhocQueryExecute = response::decode_body(h, &parts.body)?;
            // The docs-text SHOULD-list draws no GET/POST distinction — the
            // URL forms are accepted here too; a body-vs-URL conflict is a
            // 400 (the same rule the two `ehr_id` carriers follow).
            let offset = response::merge_body_and_url_i64(body.offset, q, "offset")?;
            let fetch = response::merge_body_and_url_i64(body.fetch, q, "fetch")?;
            let parameters = response::merge_body_and_url_parameters(
                body.query_parameters.unwrap_or_default(),
                q,
            )?;
            let request = scope.apply(AqlQueryRequest {
                ehr_ids: response::ehr_id_from_request(params::query_param(q, "ehr_id"), h)?
                    .into_iter()
                    .collect(),
                offset,
                fetch,
                parameters,
                ..Default::default()
            });
            Ok(state
                .backend()
                .execute_ad_hoc_query(body.q, request)
                .await?)
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted ad-hoc query operation: {other}"
        )))),
    }
}
