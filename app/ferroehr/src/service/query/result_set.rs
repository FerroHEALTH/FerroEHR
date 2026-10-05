// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `RESULT_SET` assembly (`result_set.adoc`; ITS-REST 1.1.0
//! `schemas/query/ResultSet`) and AQL parameter substitution
//! (QUERY `master03-syntax.adoc` §Parameters).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL result rows are arbitrary \
              projections by specification (QUERY 1.1)"
)]

use std::sync::LazyLock;

use jiff::Timestamp;
use openehr_its::rest::generated::query::{ResultSet, ResultSetColumn, ResultSetMetadata};
use regex::Regex;
use serde_json::Value;

use crate::aql::exec::QueryResult;
use crate::aql::ir::{ParamValue, Params};
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
    let mut params = Params::new();
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
/// The document is built as the generated ITS-REST contract types
/// ([`ResultSet`] / [`ResultSetMetadata`] / [`ResultSetColumn`], emitted from
/// the vendored `query` OAS) and serialized from them, so the served shape is
/// the contract's shape by construction — a contract change breaks this
/// function rather than silently diverging from a hand-written literal.
///
/// NOTE: no `id` is emitted. The SM `RESULT_SET` makes `id` mandatory
/// (`SM/docs/UML/classes/result_set.adoc`), but the released ITS-REST
/// `ResultSet` schema (`crates/openehr-its/vendor/rest-oas/query-*.openapi.yaml`
/// §`components.schemas.ResultSet`) declares exactly
/// `meta`/`name`/`q`/`columns`/`rows` with `required: [rows]` and **no**
/// `additionalProperties`, so on this wire an `id` key would be an undeclared
/// property on a closed object schema.
pub(super) fn result_set_json(
    aql: &str,
    executed: &str,
    name: Option<&str>,
    result: QueryResult,
) -> Value {
    let columns: Vec<ResultSetColumn> = result
        .columns
        .iter()
        .map(|c| {
            // NOTE: `RESULT_SET_COLUMN.archetype_id [0..1]` is optional and
            // omitted — the SM flags it "check on whether needed"
            // (`result_set_column.adoc`) and the OAS declares only name + path.
            ResultSetColumn {
                name: c.name.clone(),
                path: c.path.clone(),
                additional_properties: std::collections::BTreeMap::new(),
            }
        })
        .collect();

    let set = ResultSet {
        meta: Some(ResultSetMetadata {
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
            // The OAS declares `ResultSetMetadata` `additionalProperties: true`,
            // but this server publishes no metadata extension, so the map stays
            // empty and serializes to nothing.
            additional_properties: std::collections::BTreeMap::new(),
        }),
        name: name.map(ToOwned::to_owned),
        q: Some(aql.to_owned()),
        columns: Some(columns),
        // The envelope serializes with EMPTY rows; the real rows are MOVED
        // into the serialized map below, so a page of documents is never
        // deep-copied by `to_value`.
        rows: Vec::new(),
        additional_properties: std::collections::BTreeMap::new(),
    };

    // Every optional property the DTOs leave `None` is OMITTED, not rendered as
    // `null`: the generated contract types carry
    // `#[serde(skip_serializing_if = "Option::is_none")]` on each optional
    // property, because no ITS-REST component schema sets `nullable: true` and
    // an OpenAPI 3.0 `type: string` property does not admit `null`
    // (<https://spec.openapis.org/oas/v3.0.3#schema-object>). `rows` cells are
    // untouched — an unset AQL leaf is a genuine `null` there.
    let mut envelope = serde_json::to_value(&set).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut envelope {
        map.insert(
            "rows".to_owned(),
            Value::Array(result.rows.into_iter().map(Value::Array).collect()),
        );
    }
    envelope
}

/// An empty `RESULT_SET` for `aql`: the envelope, no columns, no rows.
///
/// What a cohort query answers when the cohort resolves to no EHR at all. The
/// query is never planned in that case — an unscoped execution would be a
/// population query, the opposite of what was asked — so no column metadata
/// exists to report, and the honest document is the empty one.
pub(crate) fn empty_result_set(aql: &str) -> Value {
    result_set_json(
        aql,
        aql,
        None,
        QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            served_ehrs: Vec::new(),
            served_origins: Vec::new(),
            origin_count: 0,
        },
    )
}
