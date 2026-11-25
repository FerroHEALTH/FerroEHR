// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The query surfaces' shared server API: ad-hoc AQL execution, BFF-local
//! validation, and stored-query CRUD over the ITS-REST Definition/Query APIs.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use crate::bff::ehrs::ResultPage;
use crate::error::ViewerError;
use crate::locale::server::ServerMsg;
use crate::locale::{tr_served, tr_served_with};

/// A stored-query listing row (ITS-REST Definition API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredQueryRow {
    /// Qualified name (`namespace::name`).
    pub name: String,
    /// Version (semver text).
    pub version: String,
    /// The stored AQL text (empty in list responses that omit it).
    pub query: String,
    /// Saved timestamp as reported.
    pub saved: String,
}

/// Run ad-hoc AQL via `POST query/aql` with parameter bindings (a JSON
/// object as text), one page at `offset`.
///
/// A query that carries its own AQL window is sent bare. The spec bars only
/// `fetch` beside an AQL top; withholding `offset` as well is the viewer's own
/// conservative choice, so such a query reaches the CDR exactly as written.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Rejected`] when `parameters_json` is not a JSON object.
pub async fn run_aql(
    // The AQL to execute.
    aql: String,
    // Parameter bindings as a JSON object text; empty means none.
    parameters_json: String,
    // First row of the page to return.
    offset: u32,
) -> Result<ResultPage, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let parameters: serde_json::Value = if parameters_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&parameters_json).map_err(|e| {
            ViewerError::rejected(tr_served(ServerMsg::ParametersMustJsonObject3), e)
        })?
    };
    if !parameters.is_object() {
        return Err(ViewerError::Invalid(tr_served(
            ServerMsg::ParametersMustJsonObject,
        )));
    }
    let url = cdr.rest_v1("query/aql");
    // NOTE: `fetch` "cannot be combined with AQL-top" —
    // `docs/specs/openehr/ITS-REST/specifications/docs/query/Request.md`
    // §Common Headers and Query Parameters.
    let has_own_window = crate::aql_text::carries_own_window(&aql);
    let body = if has_own_window {
        serde_json::json!({ "q": aql, "query_parameters": parameters }).to_string()
    } else {
        crate::bff::ehrs::aql_request_body(&aql, &parameters, offset, crate::bff::PAGE_SIZE)
    };
    let response = cdr.post(&url).body("application/json", body).ok().await?;
    crate::bff::ehrs::parse_result_set(&response.body, offset)
}

/// List the CDR's stored queries (`GET definition/query`).
///
/// NOTE: no openEHR spec governs this — our own design/extension. The released
/// Definition API declares only `GET /definition/query/{qualified_query_name}`,
/// whose wildcard is a PATH segment
/// (`docs/specs/openehr/ITS-REST/specifications/operations/definition_query_list.yaml`),
/// so the segment-less listing the viewer reads is the CDR's own route.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Failed`] on an unparseable body.
pub async fn list_stored_queries() -> Result<Vec<StoredQueryRow>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.rest_v1("definition/query");
    let response = cdr.get(&url).send().await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        // A CDR with no stored queries may 404 the listing; treat as empty.
        return Ok(Vec::new());
    }
    let body = response.ok()?.body;
    let doc: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| ViewerError::failed(tr_served(ServerMsg::StoredQueryListJson), e))?;
    // The list shape nests under "versions" per the Definition API; parse
    // defensively (array at root or under "versions").
    let items = doc
        .get("versions")
        .and_then(serde_json::Value::as_array)
        .or_else(|| doc.as_array())
        .cloned()
        .unwrap_or_default();
    let text = |item: &serde_json::Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| item.get(*k).and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_owned()
    };
    Ok(items
        .iter()
        .map(|item| StoredQueryRow {
            name: text(item, &["name", "qualified_query_name"]),
            version: text(item, &["version"]),
            query: text(item, &["query", "q"]),
            saved: text(item, &["saved", "timestamp"]),
        })
        .collect())
}

/// Fetch one stored query's AQL text
/// (`GET definition/query/{name}/{version}`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); CDR errors normalized
/// (a `404` for an unknown name included).
pub async fn fetch_stored_query(
    // The qualified stored-query name.
    name: String,
    // The version to read; a partial pattern resolves by prefix.
    version: String,
) -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.rest_v1(&format!(
        "definition/query/{}/{}",
        urlencoding::encode(&name),
        urlencoding::encode(&version)
    ));
    let body = cdr.get(&url).ok().await?.body;
    let doc: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| ViewerError::failed(tr_served(ServerMsg::StoredQueryJson), e))?;
    Ok(doc
        .get("query")
        .or_else(|| doc.get("q"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&body)
        .to_owned())
}

/// Stores a query, with the AQL as `text/plain`, at `version` or at the
/// server-assigned version when `version` is absent.
///
/// - `version` present: `PUT definition/query/{name}/{version}`. The
///   (`name`, `version`) pair identifies a stored query (ITS-REST
///   `docs/query/Qualified_query_name.md`) and an existing one answers `409`,
///   so the save screens propose a bumped version
///   ([`next_minor`](crate::query_namespace::next_minor)).
/// - `version` absent: `PUT definition/query/{name}`, which replaces the
///   query stored at the server's version.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] for a bad name, a non-triple version, or
/// unparseable AQL; CDR errors normalized (notably `409` for an existing
/// `(name, version)` pair).
pub async fn store_query(
    // The qualified name to file the definition under.
    name: String,
    // The concrete `major.minor.patch` version; the CDR assigns one when absent.
    version: Option<String>,
    // The AQL text to store.
    aql: String,
) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    if name.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            ServerMsg::StoredQueryNeedsName,
        )));
    }
    // An empty field means "no version" — the unversioned store — so the
    // screens can bind a plain text input without an Option dance.
    let version = version
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    if let Some(version) = version.as_deref()
        && !crate::query_namespace::is_full_semver(version)
    {
        return Err(ViewerError::Invalid(tr_served_with(
            ServerMsg::NotVersionStoreStoredQueryVersion,
            &[("version", (version).to_string().into())],
        )));
    }
    openehr_query::parser::parse_str(&aql)
        .map_err(|e| ViewerError::Invalid(crate::error::error_chain(&e)))?;
    let path = match version.as_deref() {
        Some(version) => format!(
            "definition/query/{}/{}",
            urlencoding::encode(&name),
            urlencoding::encode(version)
        ),
        None => format!("definition/query/{}", urlencoding::encode(&name)),
    };
    let url = cdr.rest_v1(&path);
    cdr.put(&url).body("text/plain", aql).ok().await?;
    Ok(())
}

/// Executes a stored query with parameter bindings and returns one page of
/// its `RESULT_SET`.
///
/// An empty `version` runs the latest version; a full or partial one is the
/// path segment (ITS-REST `docs/query/Qualified_query_name.md`), composed with
/// [`resolve_version`](crate::query_namespace::resolve_version).
/// `parameters_json` is the `query_parameters` body, as ITS-REST
/// `docs/query/Request.md` §GET vs POST recommends, with unprefixed names from
/// [`placeholders`](crate::aql_text::placeholders). `paged` false sends no
/// `fetch` or `offset`, for a definition with its own window.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Rejected`] when `parameters_json` is not a JSON object; CDR
/// errors normalized via [`CdrResponse::ok`](crate::cdr::CdrResponse::ok)
/// (notably `404` when no stored version matches the resolution form);
/// [`ViewerError::Failed`] when the result set is not valid JSON.
pub async fn run_stored_query(
    // The qualified stored-query name.
    name: String,
    // The version to run; a partial pattern resolves by prefix.
    version: String,
    // Parameter bindings as a JSON object text; empty means none.
    parameters_json: String,
    // First row of the page to return.
    offset: u32,
    // Whether the request owns the row window.
    paged: bool,
) -> Result<ResultPage, ViewerError> {
    let cdr = crate::bff::cdr()?;
    if name.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            ServerMsg::NoStoredQueryRunOpenOne,
        )));
    }
    let parameters: serde_json::Value = if parameters_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&parameters_json).map_err(|e| {
            ViewerError::rejected(tr_served(ServerMsg::ParametersMustJsonObject4), e)
        })?
    };
    if !parameters.is_object() {
        return Err(ViewerError::Invalid(tr_served(
            ServerMsg::ParametersMustJsonObject,
        )));
    }
    // Name and version are separate path segments, each percent-encoded via
    // `urlencoding`.
    let version = version.trim();
    let path = if version.is_empty() {
        format!("query/{}", urlencoding::encode(&name))
    } else {
        format!(
            "query/{}/{}",
            urlencoding::encode(&name),
            urlencoding::encode(version)
        )
    };
    let url = cdr.rest_v1(&path);
    let body = if paged {
        serde_json::json!({
            "query_parameters": parameters,
            "fetch": crate::bff::PAGE_SIZE,
            "offset": offset,
        })
    } else {
        serde_json::json!({ "query_parameters": parameters })
    };
    let response = cdr
        .post(&url)
        .body("application/json", body.to_string())
        .ok()
        .await?;
    crate::bff::ehrs::parse_result_set(&response.body, if paged { offset } else { 0 })
}

/// Run one stored query (`GET query/{name}/{version}`) and return its match
/// count — the dashboard namespace-tile primitive.
///
/// A `SELECT COUNT(*)` query answers one 1×1 numeric cell, and that cell IS the
/// count. Every other query answers rows, and the count is how many rows came
/// back on ONE page: the request sends no `fetch`, so the window is the stored
/// definition's own AQL top, or the CDR's default page, whose size "depends on
/// the implementation" (`docs/specs/openehr/ITS-REST/specifications/docs/query/Request.md`
/// §Common Headers and Query Parameters). No `fetch` is added deliberately: the
/// parameter "cannot be combined with AQL-top", and a viewer-sized window would
/// cap every tile at the table page size instead of reporting a magnitude.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Failed`] on an unparseable result.
pub async fn run_stored_count(
    // The qualified stored-query name.
    name: String,
    // The version to run.
    version: String,
) -> Result<i64, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.rest_v1(&format!(
        "query/{}/{}",
        urlencoding::encode(&name),
        urlencoding::encode(&version)
    ));
    let body = cdr.get(&url).ok().await?.body;
    let doc: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| ViewerError::failed(tr_served(ServerMsg::StoredQueryResultJson), e))?;
    let rows = doc
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    // A single 1x1 numeric cell (COUNT-shaped query) IS the count.
    if rows.len() == 1
        && let Some(cell) = rows
            .first()
            .and_then(serde_json::Value::as_array)
            .filter(|row| row.len() == 1)
            .and_then(|row| row.first())
            .and_then(serde_json::Value::as_i64)
    {
        return Ok(cell);
    }
    Ok(i64::try_from(rows.len()).unwrap_or(i64::MAX))
}
