// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `RESULT_SET` assembly (ITS-REST 1.1.0
//! `schemas/query/ResultSet`) and AQL parameter substitution
//! (QUERY `master03-syntax.adoc` §Parameters).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL result rows are arbitrary \
              projections by specification (QUERY 1.1)"
)]

use openehr_its::rest::stored_query::QualifiedQueryName;
use std::sync::LazyLock;

use jiff::Timestamp;
use openehr_its::rest::generated::query::{ResultSetColumn, ResultSetMetadata};
use regex::Regex;
use serde_json::Value;

use crate::aql::cell::Cell;
use crate::aql::exec::QueryResult;
use crate::aql::ir::{ParamValue, Params};
use crate::error::ServiceError;
use crate::service::query::request::AqlQueryRequest;

/// The `RESULT_SET` `_schema_version` this server emits — "the version of the
/// specification defining the serialized object" (ITS-REST
/// `schemas/query/ResultSet`), i.e. the implemented ITS-REST release, read
/// from the `openehr-its` crate version so it can never lag a pin bump.
const RESULT_SET_SCHEMA_VERSION: &str = openehr_its::SPEC_VERSION;

/// The `RESULT_SET.meta._type` discriminator — "the type of the serialized
/// result object" (ITS-REST `schemas/query/ResultSetMetadata`, whose own
/// example gives `_type: RESULTSET`).
const RESULT_SET_META_TYPE: &str = "RESULTSET";

/// Build the typed [`Params`] from the request's `query_parameters` map
/// (values arrive as JSON scalars; complex values degrade to their JSON
/// text — the documented widening on [`AqlQueryRequest::parameters`]).
pub(super) fn build_params(request: &AqlQueryRequest) -> Params {
    let mut params = Params::default();
    for (name, value) in &request.parameters {
        params.insert(name.clone(), param_value(value));
    }
    params
}

/// One JSON parameter value as a typed [`ParamValue`]: scalars map directly;
/// a number outside `i64`/`f64` and any array/object degrade to their JSON
/// text.
fn param_value(value: &Value) -> ParamValue {
    match value {
        Value::Null => ParamValue::Null,
        Value::Bool(b) => ParamValue::Bool(*b),
        Value::Number(n) => n
            .as_i64()
            .map(ParamValue::Int)
            .or_else(|| n.as_f64().map(ParamValue::Real))
            .unwrap_or_else(|| ParamValue::Str(n.to_string())),
        Value::String(s) => ParamValue::Str(s.clone()),
        other => ParamValue::Str(other.to_string()),
    }
}

/// The `$name` parameter-reference token in an AQL query (QUERY `master03`
/// §Parameters/Syntax l.106: `$` followed by an identifier).
static PARAM_REF: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "the pattern is a fixed literal in this file; inspecting it \
                  proves it compiles, so the Err arm is unreachable and a \
                  typed error would have no caller"
    )]
    Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)")
        .expect("the parameter-reference pattern should always compile")
});

/// Render the executed AQL: substitute each bound `$parameter` with its value.
///
/// `RESULT_SET.meta._executed_aql` is "the executed AQL" — the query after
/// parameter binding (QUERY `master03` §Parameters NOTE l.113; ITS-REST
/// `schemas/query/ResultSetMeta`). A `Str` becomes a single-quoted AQL string
/// literal (embedded `'` doubled), `Int`/`Real`/`Bool` render as their
/// literal form, `Null` as `NULL`; a `$name` with no binding is left verbatim
/// (the engine already rejects an unbound parameter at planning time).
pub(super) fn substitute_params(aql: &str, params: &Params) -> String {
    if params.is_empty() {
        // Nothing to substitute — skip the regex scan of the query text.
        return aql.to_owned();
    }
    PARAM_REF
        .replace_all(aql, |caps: &regex::Captures<'_>| {
            match params.get(&caps[1]) {
                Some(value) => render_param(value),
                None => caps[0].to_owned(),
            }
        })
        .into_owned()
}

/// Render one bound parameter as an AQL literal (see [`substitute_params`]).
fn render_param(value: &ParamValue) -> String {
    match value {
        ParamValue::Int(n) => n.to_string(),
        ParamValue::Real(r) => r.to_string(),
        ParamValue::Bool(b) => b.to_string(),
        ParamValue::Null => "NULL".to_owned(),
        ParamValue::Str(s) => format!("'{}'", s.replace('\'', "''")),
    }
}

/// Assemble the ITS-REST 1.1.0 `RESULT_SET` document
/// (`schemas/query/ResultSet`: `meta` + `q` + `columns[]` + `rows[][]`). `q`
/// is the query as submitted; `executed` is the parameter-substituted text
/// for `_executed_aql`; `name` (a stored query's qualified name) is emitted
/// only when present.
///
/// The envelope is held as the generated ITS-REST contract types
/// ([`ResultSetMetadata`] / [`ResultSetColumn`], emitted from the vendored
/// `query` OAS) and the rows as the cells the executor rendered, so the
/// document is written once ([`ResultSetDoc::write`]) and the served shape is
/// the contract's by construction.
///
/// NOTE: no `id` is emitted. The released ITS-REST
/// `ResultSet` schema (`crates/openehr-its/vendor/rest-oas/query-*.openapi.yaml`
/// §`components.schemas.ResultSet`) declares exactly
/// `meta`/`name`/`q`/`columns`/`rows` with `required: [rows]` and **no**
/// `additionalProperties`, so on this wire an `id` key would be an undeclared
/// property on a closed object schema.
pub(super) fn result_set_json(
    aql: &str,
    executed: &str,
    name: Option<&QualifiedQueryName>,
    result: QueryResult,
) -> ResultSetDoc {
    let columns: Vec<ResultSetColumn> = result
        .columns
        .iter()
        .map(|c| {
            // NOTE: `RESULT_SET_COLUMN.archetype_id [0..1]` is optional and
            // omitted — the OAS declares only name + path.
            ResultSetColumn {
                name: c.name.clone(),
                path: c.path.clone(),
                additional_properties: std::collections::BTreeMap::new(),
            }
        })
        .collect();
    ResultSetDoc {
        meta: ResultSetMetadata {
            // `_href` is "URL of the executed query (only for GET endpoint)"
            // and this assembler is method-agnostic, so it is left absent.
            _href: None,
            _type: Some(RESULT_SET_META_TYPE.to_owned()),
            _schema_version: Some(RESULT_SET_SCHEMA_VERSION.to_owned()),
            _created: Some(Timestamp::now().to_string()),
            // `_generator` is a debugging aid ("some identifier of the
            // application that generated the result"); nothing depends on it
            // and it is left absent rather than fabricated.
            _generator: None,
            _executed_aql: Some(executed.to_owned()),
            // The OAS declares `ResultSetMetadata` `additionalProperties: true`;
            // only an extension such as the cohort facts fills it.
            additional_properties: std::collections::BTreeMap::new(),
        },
        name: name.cloned(),
        q: aql.to_owned(),
        columns,
        rows: result.rows,
    }
}

/// A `RESULT_SET` document: its envelope and its rendered rows, written once
/// ([`Self::write`]).
#[derive(Debug, Clone)]
pub struct ResultSetDoc {
    meta: ResultSetMetadata,
    name: Option<QualifiedQueryName>,
    q: String,
    columns: Vec<ResultSetColumn>,
    rows: Vec<Vec<Cell>>,
}

/// A written `RESULT_SET`: the JSON body and its weak `ETag`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenResultSet {
    /// The `application/json` body.
    pub body: String,
    /// The weak `ETag`, `W/"…"`.
    pub etag: String,
}

impl Default for ResultSetDoc {
    fn default() -> Self {
        result_set_json("", "", None, QueryResult::of_rows(Vec::new(), Vec::new()))
    }
}

impl ResultSetDoc {
    /// The rendered rows.
    #[must_use]
    pub fn rows(&self) -> &[Vec<Cell>] {
        &self.rows
    }

    /// Drops every row, keeping the envelope (small-cell suppression).
    pub fn clear_rows(&mut self) {
        self.rows.clear();
    }

    /// Adds the metadata extension `key` (`ResultSetMetadata`
    /// `additionalProperties: true`).
    pub fn insert_meta(&mut self, key: &str, value: Value) {
        self.meta
            .additional_properties
            .insert(key.to_owned(), value);
    }

    /// Writes the document and its `ETag`.
    ///
    /// The keys come in the contract's order, every optional property left
    /// `None` omitted (no ITS-REST component schema sets `nullable: true`,
    /// <https://spec.openapis.org/oas/v3.0.3#schema-object>); a `rows` cell
    /// is a genuine `null` where an AQL leaf is unset.
    ///
    /// The `ETag` (`headers/ETag_RESULT_SET.yaml`, weak) is the first 128
    /// bits of the SHA-256 of the bytes as written, except the `meta` object
    /// that carries the per-response `_created` stamp, for which only the
    /// written `_executed_aql` counts: an `ETag` "changes as soon as the
    /// resource changes" (`Requests_and_responses.md` §`ETag` and
    /// Last-Modified), and two queries that return the same rows are not the
    /// same result set.
    ///
    /// # Errors
    /// [`ResultSetError::Serialise`] when an envelope part does not
    /// serialise.
    pub fn write(&self) -> Result<WrittenResultSet, ResultSetError> {
        use sha2::Digest as _;
        crate::telemetry::conversion::record("result_set_to_text");
        let meta = serde_json::to_string(&self.meta).map_err(ResultSetError::Serialise)?;
        let executed =
            serde_json::to_string(&self.meta._executed_aql).map_err(ResultSetError::Serialise)?;
        let mut rest = String::new();
        if let Some(name) = &self.name {
            rest.push_str("\"name\":");
            rest.push_str(&serde_json::to_string(name).map_err(ResultSetError::Serialise)?);
            rest.push(',');
        }
        rest.push_str("\"q\":");
        rest.push_str(&serde_json::to_string(&self.q).map_err(ResultSetError::Serialise)?);
        rest.push_str(",\"columns\":");
        rest.push_str(&serde_json::to_string(&self.columns).map_err(ResultSetError::Serialise)?);
        rest.push_str(",\"rows\":[");
        for (i, row) in self.rows.iter().enumerate() {
            if i > 0 {
                rest.push(',');
            }
            rest.push('[');
            for (j, cell) in row.iter().enumerate() {
                if j > 0 {
                    rest.push(',');
                }
                rest.push_str(cell.json());
            }
            rest.push(']');
        }
        rest.push(']');
        let mut hasher = sha2::Sha256::new();
        hasher.update(executed.as_bytes());
        hasher.update(b"\n");
        hasher.update(rest.as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let (half, _) = digest.split_at(16);
        let mut id = [0u8; 16];
        id.copy_from_slice(half);
        let mut body = String::with_capacity(meta.len() + rest.len() + 11);
        body.push_str("{\"meta\":");
        body.push_str(&meta);
        body.push(',');
        body.push_str(&rest);
        body.push('}');
        Ok(WrittenResultSet {
            body,
            etag: format!("W/\"{}\"", uuid::Uuid::from_bytes(id)),
        })
    }

    /// The document as a JSON value, as it is written.
    ///
    /// # Errors
    /// As [`Self::write`].
    pub fn to_value(&self) -> Result<Value, ResultSetError> {
        serde_json::from_str(&self.write()?.body).map_err(ResultSetError::Serialise)
    }
}

/// A `RESULT_SET` document that could not be assembled.
#[derive(Debug, thiserror::Error)]
pub enum ResultSetError {
    /// The envelope did not serialise.
    #[error("the RESULT_SET envelope did not serialise")]
    Serialise(#[source] serde_json::Error),
}

impl From<ResultSetError> for ServiceError {
    fn from(error: ResultSetError) -> Self {
        ServiceError::internal("assemble the RESULT_SET", error)
    }
}

/// An empty `RESULT_SET` for `aql`: the envelope, no columns, no rows.
///
/// What a cohort query answers when its cohort resolves to no EHR; the query is
/// never planned then, so no column metadata exists.
#[must_use]
pub(crate) fn empty_result_set(aql: &str) -> ResultSetDoc {
    result_set_json(aql, aql, None, QueryResult::of_rows(Vec::new(), Vec::new()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::aql::exec::ColumnMeta;

    fn doc(executed: &str, created: &str, rows: Vec<Vec<Cell>>) -> ResultSetDoc {
        let mut doc = result_set_json(
            "SELECT c/uid/value FROM COMPOSITION c",
            executed,
            None,
            QueryResult::of_rows(
                vec![ColumnMeta {
                    name: "#0".to_owned(),
                    path: Some("/uid/value".to_owned()),
                }],
                rows,
            ),
        );
        doc.meta._created = Some(created.to_owned());
        doc
    }

    fn uid_rows() -> Vec<Vec<Cell>> {
        vec![
            vec![Cell::of_str("8849182c::ferroehr::1")],
            vec![Cell::null()],
        ]
    }

    #[test]
    fn the_document_is_written_in_the_contract_shape() {
        let written = doc("SELECT 1", "2026-08-03T09:00:00Z", uid_rows())
            .write()
            .expect("the envelope serialises");
        let value: Value = serde_json::from_str(&written.body).expect("valid JSON");
        assert_eq!(
            value,
            json!({
                "meta": {
                    "_type": "RESULTSET",
                    "_schema_version": RESULT_SET_SCHEMA_VERSION,
                    "_created": "2026-08-03T09:00:00Z",
                    "_executed_aql": "SELECT 1",
                },
                "q": "SELECT c/uid/value FROM COMPOSITION c",
                "columns": [{"name": "#0", "path": "/uid/value"}],
                "rows": [["8849182c::ferroehr::1"], [null]],
            })
        );
        assert!(written.body.starts_with(r#"{"meta":{"_type":"RESULTSET","#));
    }

    /// Pins a known `RESULT_SET` to its exact `ETag`, so a change to what the
    /// digest covers fails loudly: a served `ETag` must never change for an
    /// unchanged `RESULT_SET`. The expected value is the first 128 bits of the
    /// SHA-256 of the written `_executed_aql`, a newline and the rest of the
    /// body, computed independently with `openssl dgst -sha256`.
    #[test]
    fn the_etag_is_pinned_to_a_known_vector() {
        let written = doc("SELECT 1", "2026-08-03T09:00:00Z", uid_rows())
            .write()
            .expect("the envelope serialises");
        assert_eq!(written.etag, "W/\"031e878b-e6c5-a4c0-402c-2dcf3d188358\"");
    }

    #[test]
    fn the_etag_is_weak_stable_and_ignores_the_created_stamp() {
        let one = doc("SELECT 1", "2026-08-03T09:00:00Z", uid_rows())
            .write()
            .unwrap();
        let two = doc("SELECT 1", "2026-08-03T11:30:42.117Z", uid_rows())
            .write()
            .unwrap();
        assert!(one.etag.starts_with("W/\""), "weak ETag: {}", one.etag);
        assert_eq!(one.etag, two.etag);
        assert_ne!(one.body, two.body);
    }

    #[test]
    fn the_etag_changes_with_the_rows_and_the_executed_query() {
        let base = doc("SELECT 1", "2026-08-03T09:00:00Z", uid_rows())
            .write()
            .unwrap();
        let no_rows = doc("SELECT 1", "2026-08-03T09:00:00Z", Vec::new())
            .write()
            .unwrap();
        let other = doc("SELECT 2", "2026-08-03T09:00:00Z", uid_rows())
            .write()
            .unwrap();
        assert_ne!(base.etag, no_rows.etag);
        assert_ne!(base.etag, other.etag);
    }

    #[test]
    fn a_metadata_extension_is_written_into_meta() {
        let mut doc = doc("SELECT 1", "2026-08-03T09:00:00Z", uid_rows());
        doc.insert_meta("cohort", json!({"size": 3}));
        doc.clear_rows();
        let value = doc.to_value().unwrap();
        assert_eq!(value["meta"]["cohort"], json!({"size": 3}));
        assert_eq!(value["rows"], json!([]));
    }
}
