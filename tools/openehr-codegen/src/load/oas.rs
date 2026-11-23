// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `OpenAPI` reader for the REST codegen: loads a vendored
//! `*-codegen.openapi.yaml` bundle and exposes its component schemas and each
//! operation with local `$ref`s resolved.

#![expect(
    clippy::disallowed_types,
    reason = "dev tooling over JSON artifacts (vendored BMM/OAS bundles, emitter reports) — not the \
              application (#1694)"
)]
use serde_json::Value;

/// A parsed `OpenAPI` document.
pub(crate) struct Oas {
    root: Value,
}

/// One HTTP operation (a method on a path).
#[expect(
    clippy::struct_field_names,
    reason = "`operation_id` mirrors the OAS `operationId` field name"
)]
pub(crate) struct Operation<'a> {
    pub method: &'a str,
    pub path: &'a str,
    /// `operationId`, e.g. `ehr_create` — the trait method name.
    pub operation_id: String,
    /// Resolved parameters (path/query/header).
    pub parameters: Vec<Param>,
    /// The request body schema (resolved `Value`) + whether it is required.
    pub request_body: Option<(Value, bool)>,
    /// The media types the request body declares, in document order; empty
    /// without a body.
    pub request_media: Vec<String>,
    /// Every documented response in ascending status order, `default` excluded.
    pub responses: Vec<Response>,
}

/// One documented response: status, referenced component, content schemas
/// verbatim and declared headers, in document order.
pub(crate) struct Response {
    pub status: http::StatusCode,
    /// The `#/components/responses/<name>` component the operation references,
    /// when it references one rather than declaring the response inline.
    pub component: Option<String>,
    pub media: Vec<(String, Value)>,
    pub headers: Vec<String>,
}

/// One resolved operation parameter.
pub(crate) struct Param {
    pub name: String,
    /// `path` | `query` | `header`.
    pub location: String,
    /// The `#/components/parameters/<name>` component the operation references,
    /// when it references one rather than declaring the parameter inline.
    pub component: Option<String>,
    pub required: bool,
    /// The declared `style`, when the parameter states one.
    pub style: Option<String>,
    /// The declared `explode`, when the parameter states one.
    pub explode: Option<bool>,
    /// The parameter's schema (resolved `Value`).
    pub schema: Value,
}

impl Oas {
    /// Parses an OAS YAML bundle.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or parsed.
    pub(crate) fn parse_file(path: &std::path::Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let root: Value =
            serde_norway::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
        Ok(Self { root })
    }

    /// A synthetic bundle carrying the first declarer's copy of each `keep` schema
    /// and the transitive `allOf` bases they compose, for the shared module.
    #[must_use]
    pub(crate) fn merged_schemas(
        bundles: &[(&str, Oas)],
        keep: &std::collections::BTreeSet<String>,
    ) -> Oas {
        let wanted = Self::base_closure(bundles, keep);
        let mut schemas = serde_json::Map::new();
        for (_, o) in bundles {
            for (name, schema) in o.schemas() {
                if wanted.contains(&name) && !schemas.contains_key(&name) {
                    schemas.insert(name, schema.clone());
                }
            }
        }
        Oas {
            root: serde_json::json!({ "components": { "schemas": schemas } }),
        }
    }

    /// The transitive `allOf`-base closure of `keep`, as a least fixpoint: a
    /// base may compose a base of its own.
    fn base_closure(
        bundles: &[(&str, Oas)],
        keep: &std::collections::BTreeSet<String>,
    ) -> std::collections::BTreeSet<String> {
        let mut wanted = keep.clone();
        while Self::add_allof_bases(bundles, &mut wanted) {}
        wanted
    }

    /// Adds every `allOf` base of the wanted schemas, returning whether `wanted`
    /// grew.
    fn add_allof_bases(
        bundles: &[(&str, Oas)],
        wanted: &mut std::collections::BTreeSet<String>,
    ) -> bool {
        let mut added = false;
        for (_, o) in bundles {
            for (name, schema) in o.schemas() {
                if !wanted.contains(&name) {
                    continue;
                }
                for base in Self::allof_bases(schema) {
                    added |= wanted.insert(base);
                }
            }
        }
        added
    }

    /// The component-schema names a schema's `allOf` members `$ref`.
    fn allof_bases(schema: &Value) -> Vec<String> {
        schema
            .get("allOf")
            .and_then(Value::as_array)
            .map(|members| members.iter().filter_map(Self::ref_name).collect())
            .unwrap_or_default()
    }

    /// The component schemas (`#/components/schemas`), in document order.
    #[must_use]
    pub(crate) fn schemas(&self) -> Vec<(String, &Value)> {
        self.root
            .pointer("/components/schemas")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v)).collect())
            .unwrap_or_default()
    }

    /// Resolve a local `$ref` (`#/components/...`) to the pointed-at value.
    #[must_use]
    /// Every `#/components/parameters` entry as `(component, in, description)`,
    /// in document order.
    pub(crate) fn parameter_components(&self) -> Vec<(String, String, String)> {
        let Some(params) = self
            .root
            .pointer("/components/parameters")
            .and_then(Value::as_object)
        else {
            return Vec::new();
        };
        params
            .iter()
            .map(|(component, p)| {
                let text = |key: &str| {
                    p.get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                (component.clone(), text("in"), text("description"))
            })
            .collect()
    }

    pub(crate) fn resolve<'a>(&'a self, value: &'a Value) -> &'a Value {
        let mut cur = value;
        // Follow chained single-level `$ref`s (parameters/responses → schema).
        while let Some(r) = cur.get("$ref").and_then(Value::as_str) {
            let ptr = r.trim_start_matches('#');
            match self.root.pointer(ptr) {
                Some(next) => cur = next,
                None => break,
            }
        }
        cur
    }

    /// The base name of a `$ref` (`#/components/schemas/Ehr` → `Ehr`), if this
    /// value is a direct ref.
    #[must_use]
    pub(crate) fn ref_name(value: &Value) -> Option<String> {
        value
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.rsplit('/').next())
            .map(str::to_string)
    }

    /// All operations across all paths.
    #[must_use]
    pub(crate) fn operations(&self) -> Vec<Operation<'_>> {
        // The Path Item methods (<https://spec.openapis.org/oas/v3.0.3#path-item-object>)
        // minus `trace`; the System API declares `OPTIONS /`.
        const METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head", "options"];
        let mut out = Vec::new();
        let Some(paths) = self.root.get("paths").and_then(Value::as_object) else {
            return out;
        };
        for (path, item) in paths {
            for method in METHODS {
                let Some(op) = item.get(method) else { continue };
                let operation_id = op
                    .get("operationId")
                    .and_then(Value::as_str)
                    .map_or_else(|| synth_op_id(method, path), str::to_string);
                let parameters = self.parse_params(op);
                let request_body = self.parse_request_body(op);
                let request_media = self.parse_request_media(op);
                let responses = self.parse_responses(op);
                out.push(Operation {
                    method,
                    path,
                    operation_id,
                    parameters,
                    request_body,
                    request_media,
                    responses,
                });
            }
        }
        out
    }

    fn parse_params(&self, op: &Value) -> Vec<Param> {
        let mut out = Vec::new();
        let Some(params) = op.get("parameters").and_then(Value::as_array) else {
            return out;
        };
        for p in params {
            let component = p
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|r| r.strip_prefix("#/components/parameters/"))
                .map(str::to_string);
            let p = self.resolve(p);
            let (Some(name), Some(location)) = (
                p.get("name").and_then(Value::as_str),
                p.get("in").and_then(Value::as_str),
            ) else {
                continue;
            };
            out.push(Param {
                name: name.to_string(),
                location: location.to_string(),
                component,
                required: p.get("required").and_then(Value::as_bool).unwrap_or(false),
                style: p.get("style").and_then(Value::as_str).map(str::to_string),
                explode: p.get("explode").and_then(Value::as_bool),
                // Kept verbatim, so a `$ref` keeps its name.
                schema: p.get("schema").cloned().unwrap_or(Value::Null),
            });
        }
        out
    }

    fn parse_request_body(&self, op: &Value) -> Option<(Value, bool)> {
        let rb = self.resolve(op.get("requestBody")?);
        let required = rb.get("required").and_then(Value::as_bool).unwrap_or(false);
        let schema = Self::first_json_schema(rb)?;
        Some((schema, required))
    }

    fn parse_request_media(&self, op: &Value) -> Vec<String> {
        op.get("requestBody")
            .map(|rb| self.resolve(rb))
            .and_then(|rb| rb.get("content")?.as_object())
            .map(|content| content.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Every response keyed by a numeric status, ascending, with the media
    /// types and headers it declares.
    fn parse_responses(&self, op: &Value) -> Vec<Response> {
        let Some(responses) = op.get("responses").and_then(Value::as_object) else {
            return Vec::new();
        };
        let mut out: Vec<Response> = responses
            .iter()
            .filter_map(|(code, resp)| {
                // NOTE: OAS 3.0 §Responses Object also keys a response by `default`, which is
                // not a status.
                let status = http::StatusCode::from_u16(code.parse::<u16>().ok()?).ok()?;
                let component = resp
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|r| r.strip_prefix("#/components/responses/"))
                    .map(str::to_string);
                let resp = self.resolve(resp);
                let media = resp
                    .get("content")
                    .and_then(Value::as_object)
                    .map(|content| {
                        content
                            .iter()
                            .filter_map(|(mt, mv)| Some((mt.clone(), mv.get("schema")?.clone())))
                            .collect()
                    })
                    .unwrap_or_default();
                let headers = resp
                    .get("headers")
                    .and_then(Value::as_object)
                    .map(|h| h.keys().cloned().collect())
                    .unwrap_or_default();
                Some(Response {
                    status,
                    component,
                    media,
                    headers,
                })
            })
            .collect();
        out.sort_by_key(|r| r.status);
        out
    }

    /// The `application/json` (or first) content schema of a request body or
    /// response, verbatim.
    fn first_json_schema(container: &Value) -> Option<Value> {
        let content = container.get("content")?.as_object()?;
        let media = content
            .get("application/json")
            .or_else(|| content.values().next())?;
        Some(media.get("schema")?.clone())
    }
}

/// Synthesize an operation id from method + path when the OAS omits one.
fn synth_op_id(method: &str, path: &str) -> String {
    let mut s = method.to_string();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        s.push('_');
        s.push_str(&seg.replace(['{', '}'], "").replace('-', "_"));
    }
    s
}
