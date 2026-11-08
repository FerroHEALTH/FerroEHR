// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stored-query execution — `GET`/`POST /query/{qualified_query_name}` (latest)
//! and `.../{version}` (explicit version).
//!
//! A stored query is referenced by its qualified name
//! (`[{namespace}::]{query-name}`; reserved name `aql`) with an optional SEMVER
//! `version` (`Qualified_query_name.md`; `Query_types.md` §Stored queries). The
//! definition service holds the AQL; execution supplies only the paging/scope
//! window and `$parameter` binds — in the query string (`GET`) or the JSON body
//! (`POST`, `schemas/query/Query.yaml`: `offset`/`fetch`/`query_parameters`, no
//! `q`). This module normalizes both into an [`AqlQueryRequest`] and calls the
//! [`QueryService`] stored seam (`version = None` → latest).
//!
//! [`QueryService`]: ferroehr::service::FerroEhrService

use http::HeaderMap;

use openehr_its::rest::generated::query::{
    Query as StoredQueryBody, QueryExecuteStoredQueryParams, QueryExecuteStoredQueryVersionParams,
};
use openehr_its::rest::runtime::ApiError;

use ferroehr::service::query::request::{AqlQueryRequest, QueryOutcome};

use super::response::{self, QueryScope};
use crate::api::RequestParts;
use crate::overview::error::RestError;
use crate::params;
use crate::state::AppState;

/// The `query_execute_stored_query` operation: GET /query/{name} — latest version; paging/scope in the query string.
///
/// The single wire `ehr_id` (query parameter or `openehr-ehr-id`
/// header — resolved for all four by [`response::ehr_scope`],
/// `Request.md` §About the `ehr_id` parameter) is collected into the
/// one-element [`AqlQueryRequest::ehr_ids`] scope.
pub(super) async fn get(
    state: &AppState,
    parts: &RequestParts,
    scope: &QueryScope,
) -> Result<QueryOutcome, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        QueryExecuteStoredQueryParams::from_request,
        QueryExecuteStoredQueryParams::PARAMS,
    )?;
    // Named parameter binds per Request.md §Query parameters — the
    // documented GET form; the JSON-object `query_parameters` stays
    // an accepted superset.
    let parameters = params::aql_binds(p.query_parameters)?;
    let request = scope.apply(AqlQueryRequest {
        ehr_ids: response::ehr_scope(p.ehr_id, h)?,
        offset: p.offset,
        fetch: p.fetch,
        parameters,
        ..Default::default()
    });
    Ok(state
        .backend()
        .execute_stored_query(p.qualified_query_name, None, request)
        .await?)
}

/// The `query_execute_stored_query_body` operation: POST /query/{name} — latest version; `Query` body.
pub(super) async fn post(
    state: &AppState,
    parts: &RequestParts,
    scope: &QueryScope,
) -> Result<QueryOutcome, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let name = typed_segment(parts, "qualified_query_name")?;
    let request = scope.apply(stored_body_request(q, h, &parts.body)?);
    Ok(state
        .backend()
        .execute_stored_query(name, None, request)
        .await?)
}

/// The `query_execute_stored_query_version` operation: GET /query/{name}/{version} — explicit SEMVER version.
pub(super) async fn version_get(
    state: &AppState,
    parts: &RequestParts,
    scope: &QueryScope,
) -> Result<QueryOutcome, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        QueryExecuteStoredQueryVersionParams::from_request,
        QueryExecuteStoredQueryVersionParams::PARAMS,
    )?;
    let parameters = params::aql_binds(p.query_parameters)?;
    let request = scope.apply(AqlQueryRequest {
        ehr_ids: response::ehr_scope(p.ehr_id, h)?,
        offset: p.offset,
        fetch: p.fetch,
        parameters,
        ..Default::default()
    });
    Ok(state
        .backend()
        .execute_stored_query(p.qualified_query_name, Some(p.version), request)
        .await?)
}

/// The `query_execute_stored_query_version_body` operation: POST /query/{name}/{version} — explicit version; `Query` body.
pub(super) async fn version_post(
    state: &AppState,
    parts: &RequestParts,
    scope: &QueryScope,
) -> Result<QueryOutcome, RestError> {
    let h = &parts.headers;
    let q = parts.query.as_deref();
    let name = typed_segment(parts, "qualified_query_name")?;
    let version = typed_segment(parts, "version")?;
    let request = scope.apply(stored_body_request(q, h, &parts.body)?);
    Ok(state
        .backend()
        .execute_stored_query(name, Some(version), request)
        .await?)
}

/// The path parameter `key` read by its ITS-REST grammar
/// (`openehr_its::rest::stored_query`), a value outside it a `400`.
fn typed_segment<T>(parts: &RequestParts, key: &str) -> Result<T, RestError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    response::path_segment(parts, key)?
        .parse()
        .map_err(|refusal| {
            RestError::from(ApiError::BadRequest(format!(
                "path parameter `{key}`: {refusal}"
            )))
        })
}

/// Build the request for a stored-query `POST` body (`Query` schema: `offset`,
/// `fetch`, `query_parameters`); `ehr_id` comes from the query string or header.
fn stored_body_request(
    q: Option<&str>,
    h: &HeaderMap,
    body: &bytes::Bytes,
) -> Result<AqlQueryRequest, RestError> {
    // All three body members are OPTIONAL (`Request.md` §Common Headers:
    // offset defaults 0, fetch is implementation-default — the released OAS
    // required-list loses this real conflict to the docs text): `{}` executes a parameterless
    // stored query.
    let parsed: StoredQueryBody = response::decode_body(h, body)?;
    // The docs-text SHOULD-list draws no GET/POST distinction ("All query
    // execution requests SHOULD support at least the following parameters"),
    // so the URL forms are accepted on the POSTs too; a value carried in BOTH
    // places must agree — a conflict is a 400 (the same rule the `ehr_id`
    // adjudicated).
    let offset = response::merge_body_and_url_i64(parsed.offset, q, "offset")?;
    let fetch = response::merge_body_and_url_i64(parsed.fetch, q, "fetch")?;
    let parameters =
        response::merge_body_and_url_parameters(parsed.query_parameters.unwrap_or_default(), q)?;
    Ok(AqlQueryRequest {
        ehr_ids: response::ehr_scope(params::query_param(q, "ehr_id"), h)?,
        offset,
        fetch,
        parameters,
        ..Default::default()
    })
}
