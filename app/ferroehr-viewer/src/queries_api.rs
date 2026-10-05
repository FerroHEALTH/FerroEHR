// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The query surfaces' shared server API: ad-hoc AQL execution, BFF-local
//! validation, and stored-query CRUD over the ITS-REST Definition/Query APIs.
//!
//! One call is an extension rather than a released operation: the unfiltered
//! stored-query listing ([`list_stored_queries`]), which flags itself.
//!
//! The viewer persists nothing of its own here (nor anywhere): a stored
//! query's grouping is derived from the namespace in its qualified name
//! (`crate::query_namespace`), so it lives in the CDR and reads identically
//! for every API client.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use leptos::server;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;
use crate::pages::ehrs::ResultPage;

/// A stored-query listing row (ITS-REST Definition API).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Validate AQL text BFF-locally through the real grammar — no CDR round
/// trip. Returns the parser diagnostic on failure.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a session;
/// [`ViewerError::Invalid`] carrying the parse diagnostic.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn validate_aql(
    /// The AQL text to parse.
    aql: String,
) -> Result<(), ViewerError> {
    crate::session::require_session().await?;
    openehr_query::parser::parse_str(&aql)
        .map(|_| ())
        .map_err(|e| ViewerError::Invalid(e.to_string()))
}

/// Run ad-hoc AQL via `POST query/aql` with parameter bindings (a JSON
/// object as text), one page at `offset`.
///
/// A query that carries its own AQL window is sent bare. The spec bars only
/// `fetch` beside an AQL top; withholding `offset` as well is the viewer's own
/// conservative choice, so such a query reaches the CDR exactly as written.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a session;
/// [`ViewerError::Invalid`] when `parameters_json` is not a JSON object;
/// CDR errors normalized via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn run_aql(
    /// The AQL to execute.
    aql: String,
    /// Parameter bindings as a JSON object text; empty means none.
    parameters_json: String,
    /// First row of the page to return.
    offset: u32,
) -> Result<ResultPage, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let parameters: serde_json::Value = if parameters_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&parameters_json)
            .map_err(|e| ViewerError::Invalid(format!("parameters must be a JSON object: {e}")))?
    };
    if !parameters.is_object() {
        return Err(ViewerError::Invalid(
            "parameters must be a JSON object".to_owned(),
        ));
    }
    let url = state.cdr.rest_v1("query/aql");
    // NOTE: `fetch` "cannot be combined with AQL-top" —
    // `docs/specs/openehr/ITS-REST/specifications/docs/query/Request.md`
    // §Common Headers and Query Parameters.
    let has_own_window = crate::aql_text::carries_own_window(&aql);
    let body = if has_own_window {
        serde_json::json!({ "q": aql, "query_parameters": parameters }).to_string()
    } else {
        crate::pages::ehrs::aql_request_body(&aql, &parameters, offset)
    };
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    crate::pages::ehrs::parse_result_set(&response.body, offset)
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
/// [`ViewerError::Unauthenticated`] without a session; CDR errors
/// normalized; [`ViewerError::Internal`] on an unparseable body.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_stored_queries() -> Result<Vec<StoredQueryRow>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let url = state.cdr.rest_v1("definition/query");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        // A CDR with no stored queries may 404 the listing; treat as empty.
        return Ok(Vec::new());
    }
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let doc: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| ViewerError::Internal(format!("stored-query list JSON: {e}")))?;
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
/// [`ViewerError::Unauthenticated`] without a session; CDR errors
/// normalized (a `404` for an unknown name included).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_stored_query(
    /// The qualified stored-query name.
    name: String,
    /// The version to read; a partial pattern resolves by prefix.
    version: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let url = state.cdr.rest_v1(&format!(
        "definition/query/{}/{}",
        urlencoding::encode(&name),
        urlencoding::encode(&version)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let doc: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| ViewerError::Internal(format!("stored query JSON: {e}")))?;
    Ok(doc
        .get("query")
        .or_else(|| doc.get("q"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&body)
        .to_owned())
}

/// Store a query at an explicit version, or at the server-assigned version
/// when `version` is absent — the two ITS-REST store operations, with the AQL
/// as `text/plain`:
///
/// - `version` present → `PUT definition/query/{name}/{version}`, storing the
///   query at that one version. A stored query is identified by its qualified
///   name AND its version (ITS-REST docs text,
///   `docs/specs/openehr/ITS-REST/specifications/docs/query/Qualified_query_name.md`
///   §Qualified query name), and that pair is IMMUTABLE here: an existing one
///   answers `409`, never an overwrite — which is why the save screens propose
///   a bumped version when re-saving a loaded query
///   ([`next_minor`](crate::query_namespace::next_minor)).
/// - `version` absent → `PUT definition/query/{name}`: the server owns the
///   version, and a query already stored at it is REPLACED.
///
/// `name` is the qualified query name — `[{namespace}::]{query-name}`, the
/// namespace optional (same file). The save screens compose it from their
/// namespace + name fields with [`qualify`](crate::query_namespace::qualify).
/// Name and version are separate path segments, each percent-encoded via
/// `urlencoding`. The AQL is validated BFF-locally first, and an explicit
/// version must be a concrete `major.minor.patch`
/// ([`is_full_semver`](crate::query_namespace::is_full_semver)) — a partial
/// pattern is the READ form, not something to file a definition under.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a session;
/// [`ViewerError::Invalid`] for a bad name, a non-triple version, or
/// unparseable AQL; CDR errors normalized (notably `409` for an existing
/// `(name, version)` pair).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn store_query(
    /// The qualified name to file the definition under.
    name: String,
    /// The concrete `major.minor.patch` version; the CDR assigns one when absent.
    version: Option<String>,
    /// The AQL text to store.
    aql: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    if name.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "the stored query needs a name".to_owned(),
        ));
    }
    // An empty field means "no version" — the unversioned store — so the
    // screens can bind a plain text input without an Option dance.
    let version = version
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    if let Some(version) = version.as_deref()
        && !crate::query_namespace::is_full_semver(version)
    {
        return Err(ViewerError::Invalid(format!(
            "`{version}` is not a version to store at: a stored-query version is \
             `major.minor.patch` (for example `1.0.0`). A shorter pattern like \
             `1` or `1.0` selects the latest matching version when READING a \
             query; leave the field empty to let the server assign the version."
        )));
    }
    openehr_query::parser::parse_str(&aql).map_err(|e| ViewerError::Invalid(e.to_string()))?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let path = match version.as_deref() {
        Some(version) => format!(
            "definition/query/{}/{}",
            urlencoding::encode(&name),
            urlencoding::encode(version)
        ),
        None => format!("definition/query/{}", urlencoding::encode(&name)),
    };
    let url = state.cdr.rest_v1(&path);
    let response = state
        .cdr
        .put_text(&session.credential, &url, "text/plain", aql)
        .await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

/// Execute a STORED query with parameter bindings and return one page of its
/// `RESULT_SET`.
///
/// The two ITS-REST stored-query execution operations are addressed by whether a
/// `version` is supplied — which is the spec's version-resolution contract, not a
/// viewer convention. `version` is the path segment to send, EMPTY meaning "send
/// none" (a plain `String` rather than an `Option` so nothing about the request
/// depends on how an optional field survives the server-fn encoding):
///
/// - `version` empty → `POST query/{qualified_query_name}` — "when `version` is
///   not supplied at all, the system must use the latest `version`".
/// - `version` non-empty → `POST query/{qualified_query_name}/{version}`, where
///   the segment is either a complete `major.minor.patch` or a partial
///   `{major}` / `{major}.{minor}` pattern: "When only a partial `version`
///   pattern is supplied … the latest query version matching supplied prefix
///   will be used."
///
/// (Both quotes: ITS-REST `specifications/docs/query/Qualified_query_name.md`
/// §Qualified query name. The viewer composes the segment with
/// [`resolve_version`](crate::query_namespace::resolve_version).)
///
/// `parameters_json` is the `query_parameters` object — the request body carries
/// the bindings rather than the URL: "we recommend clients using the `POST`
/// method instead of `GET`" (ITS-REST `specifications/docs/query/Request.md`
/// §GET vs POST). The names are unprefixed (`temperature`, not `$temperature`)
/// per that document's §Common Headers and Query Parameters, and the viewer
/// derives them with [`placeholders`](crate::aql_text::placeholders).
///
/// `paged` says whether the REQUEST owns the row window: `true` sends
/// `fetch`/`offset`, `false` sends neither because the stored definition carries
/// its own AQL `LIMIT`/`TOP` and the two windows cannot be combined (`fetch`
/// "cannot be combined with AQL-top", same section). The caller reads that from
/// the definition it already loaded
/// ([`carries_own_window`](crate::aql_text::carries_own_window)).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a session;
/// [`ViewerError::Invalid`] when `parameters_json` is not a JSON object; CDR
/// errors normalized via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success)
/// (notably `404` when no stored version matches the resolution form);
/// [`ViewerError::Internal`] when the result set is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn run_stored_query(
    /// The qualified stored-query name.
    name: String,
    /// The version to run; a partial pattern resolves by prefix.
    version: String,
    /// Parameter bindings as a JSON object text; empty means none.
    parameters_json: String,
    /// First row of the page to return.
    offset: u32,
    /// Whether the request owns the row window (see above).
    paged: bool,
) -> Result<ResultPage, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    if name.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "no stored query to run — open one from the stored-query list".to_owned(),
        ));
    }
    let parameters: serde_json::Value = if parameters_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&parameters_json)
            .map_err(|e| ViewerError::Invalid(format!("parameters must be a JSON object: {e}")))?
    };
    if !parameters.is_object() {
        return Err(ViewerError::Invalid(
            "parameters must be a JSON object".to_owned(),
        ));
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
    let url = state.cdr.rest_v1(&path);
    let body = if paged {
        serde_json::json!({
            "query_parameters": parameters,
            "fetch": crate::components::data_table::PAGE_SIZE,
            "offset": offset,
        })
    } else {
        serde_json::json!({ "query_parameters": parameters })
    };
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            body.to_string(),
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    crate::pages::ehrs::parse_result_set(&response.body, if paged { offset } else { 0 })
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
/// [`ViewerError::Unauthenticated`] without a session; CDR errors
/// normalized; [`ViewerError::Internal`] on an unparseable result.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn run_stored_count(
    /// The qualified stored-query name.
    name: String,
    /// The version to run.
    version: String,
) -> Result<i64, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let url = state.cdr.rest_v1(&format!(
        "query/{}/{}",
        urlencoding::encode(&name),
        urlencoding::encode(&version)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let doc: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| ViewerError::Internal(format!("stored-query result JSON: {e}")))?;
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
