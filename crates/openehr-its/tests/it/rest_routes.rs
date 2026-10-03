// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::panic_in_result_fn,
    reason = "test assertions in the Book's Result-returning shape (.claude/rules/testing.md \
              §Test shapes)"
)]
//! Tests for the operation matcher over the generated route tables
//! (`rest::routes::lookup`).
//!
//! The route tables are the vendored OAS `paths`
//! (`crates/openehr-its/vendor/rest-oas/<group>-codegen.openapi.yaml`); the
//! matcher's precedence (a literal segment over a parameter), its path
//! parameter decoding (RFC 3986 §2.1) and the declared-parameter table each
//! match carries — held to the generated param structs — are pinned here.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use http::Method;
use openehr_its::rest::generated::{admin, definition, demographic, ehr, query, system};
use openehr_its::rest::routes::{
    Lookup, Param, ParamKind, ParamLocation, PathParam, RouteMatch, lookup,
};
use serde::de::{DeserializeOwned, Visitor};

/// A test's plumbing error: every fallible step propagates with `?`.
type TestResult = Result<(), Box<dyn Error>>;

/// One generated route table.
type RouteTable = &'static [(&'static str, &'static str, &'static str)];

/// One generated parameter table, index-aligned with its route table.
type ParamTable = &'static [&'static [Param]];

/// Every generated API group with its route table and parameter table.
const GROUPS: &[(&str, RouteTable, ParamTable)] = &[
    ("admin", admin::ROUTES, admin::ROUTE_PARAMS),
    ("definition", definition::ROUTES, definition::ROUTE_PARAMS),
    (
        "demographic",
        demographic::ROUTES,
        demographic::ROUTE_PARAMS,
    ),
    ("ehr", ehr::ROUTES, ehr::ROUTE_PARAMS),
    ("query", query::ROUTES, query::ROUTE_PARAMS),
    ("system", system::ROUTES, system::ROUTE_PARAMS),
];

/// A concrete path for `template`: every `{name}` segment filled with a sample
/// that no literal segment spells, the query expansion dropped.
fn concrete(template: &str) -> String {
    let path = template.split("{?").next().unwrap_or(template);
    path.split('/')
        .map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
                .map_or_else(|| segment.to_owned(), |name| format!("sample-{name}"))
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The parameter names `template` declares, in path order.
fn declared_params(template: &str) -> Vec<String> {
    let path = template.split("{?").next().unwrap_or(template);
    path.split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
        .map(str::to_owned)
        .collect()
}

/// The match for `method` and `path`, failing the test on anything else.
fn matched(method: &Method, path: &str) -> Result<RouteMatch, Box<dyn Error>> {
    match lookup(method, path) {
        Lookup::Matched(found) => Ok(found),
        other => Err(format!("{method} {path} matched nothing: {other:?}").into()),
    }
}

/// The groups listed here are every group the generator emits, so the
/// exhaustive test below reaches every route table.
#[test]
fn the_group_list_is_every_generated_group() -> TestResult {
    let mod_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rest/generated/mod.rs");
    let generated: Vec<String> = std::fs::read_to_string(mod_rs)?
        .lines()
        .filter_map(|line| line.strip_prefix("pub mod ")?.strip_suffix(';'))
        .filter(|name| *name != "common")
        .map(str::to_owned)
        .collect();
    let listed: Vec<String> = GROUPS
        .iter()
        .map(|(name, _, _)| (*name).to_owned())
        .collect();
    assert_eq!(listed, generated);
    Ok(())
}

/// A concrete path for every route of every group matches back to that
/// route's operation, with its path parameters named in order.
#[test]
fn every_route_matches_back_to_its_operation() -> TestResult {
    let mut checked = 0_usize;
    for (group, routes, params) in GROUPS {
        assert_eq!(routes.len(), params.len(), "{group}");
        for ((method, template, operation_id), declared) in routes.iter().zip(*params) {
            let method = Method::from_bytes(method.as_bytes())?;
            let path = concrete(template);
            let found = matched(&method, &path)?;
            assert_eq!(
                (found.group, found.operation_id, found.template),
                (*group, *operation_id, *template),
                "{method} {path}"
            );
            assert_eq!(found.method, method);
            let names: Vec<String> = found
                .path_params
                .iter()
                .map(|param| param.name.to_owned())
                .collect();
            assert_eq!(names, declared_params(template), "{method} {template}");
            assert_eq!(found.params, *declared, "{method} {template}");
            let mut path_declared: Vec<String> = declared
                .iter()
                .filter(|param| param.location == ParamLocation::Path)
                .map(|param| param.name.to_owned())
                .collect();
            path_declared.sort();
            let mut in_template = declared_params(template);
            in_template.sort();
            assert_eq!(path_declared, in_template, "{method} {template}");
            for param in &found.path_params {
                assert_eq!(param.raw, format!("sample-{}", param.name));
            }
            checked += 1;
        }
    }
    let total: usize = GROUPS.iter().map(|(_, routes, _)| routes.len()).sum();
    assert_eq!(checked, total);
    Ok(())
}

/// A literal segment wins over a parameter at the same position.
#[test]
fn a_literal_segment_beats_a_parameter() -> TestResult {
    assert_eq!(
        matched(&Method::GET, "/query/aql")?.operation_id,
        "query_execute_adhoc_query"
    );
    assert_eq!(
        matched(&Method::GET, "/query/org.example::vitals")?.operation_id,
        "query_execute_stored_query"
    );
    assert_eq!(
        matched(&Method::DELETE, "/admin/ehr/all")?.operation_id,
        "admin_ehr_delete_all"
    );
    assert_eq!(
        matched(&Method::DELETE, "/admin/ehr/7d44b88c")?.operation_id,
        "admin_ehr_delete"
    );
    assert_eq!(
        matched(&Method::GET, "/definition/template/adl2/vitals/example")?.operation_id,
        "definition_template_adl2_example_get"
    );
    assert_eq!(
        matched(&Method::GET, "/definition/template/adl2/vitals/1.0.0")?.operation_id,
        "definition_template_adl2_version_get"
    );
    Ok(())
}

/// A resource the API defines answers `405` with its declared methods under
/// any other method; a path no operation names answers `404`.
#[test]
fn an_undeclared_method_is_405_and_an_unknown_path_404() {
    assert_eq!(
        lookup(&Method::DELETE, "/query/aql"),
        Lookup::MethodNotAllowed {
            allowed: vec!["GET", "POST"]
        }
    );
    assert_eq!(
        lookup(&Method::PATCH, "/ehr/7d44b88c"),
        Lookup::MethodNotAllowed {
            allowed: vec!["GET", "PUT"]
        }
    );
    assert_eq!(
        lookup(&Method::HEAD, "/ehr/7d44b88c"),
        Lookup::MethodNotAllowed {
            allowed: vec!["GET", "PUT"]
        },
        "a HEAD is matched only where the OAS declares one"
    );
    for path in ["/nothing", "/ehr/", "/ehr//composition", "/query/aql/x/y"] {
        assert_eq!(lookup(&Method::GET, path), Lookup::NotFound, "{path:?}");
    }
}

/// A path parameter keeps its segment as received and decodes it on
/// request; a query string is not part of the path.
#[test]
fn path_parameters_keep_their_raw_form_and_decode() -> TestResult {
    let found = matched(
        &Method::GET,
        "/ehr/7d44b88c/composition/8849182c%3A%3Acdr.example.org%3A%3A1?version_at_time=x",
    )?;
    assert_eq!(found.operation_id, "composition_get");
    let uid = found
        .path_param("uid_based_id")
        .ok_or("the template declares uid_based_id")?;
    assert_eq!(uid.raw, "8849182c%3A%3Acdr.example.org%3A%3A1");
    assert_eq!(uid.decoded()?, "8849182c::cdr.example.org::1");
    assert!(found.path_param("version_uid").is_none());
    let not_utf8 = PathParam {
        name: "ehr_id",
        raw: "%FF%FE".to_owned(),
    };
    assert!(not_utf8.decoded().is_err());
    Ok(())
}

/// One field of a generated param struct, as the generated source declares it.
#[derive(Debug, PartialEq, Eq)]
struct Field {
    /// The wire name: the `#[serde(rename)]` value, else the field identifier.
    wire: String,
    /// The location the field's doc line names (`path`, `query`, `header`).
    location: String,
    /// Whether the field is not an `Option`.
    required: bool,
    /// Whether the field (inside any `Option`) is a `Vec`.
    list: bool,
}

/// The field a `pub ident: Ty,` declaration closes, with its pending rename and
/// doc-line location.
fn field(
    decl: &str,
    rename: Option<String>,
    location: Option<String>,
) -> Result<Field, Box<dyn Error>> {
    let (ident, ty) = decl
        .strip_suffix(',')
        .and_then(|decl| decl.split_once(": "))
        .ok_or_else(|| format!("not a field declaration: {decl:?}"))?;
    let inner = ty
        .strip_prefix("Option<")
        .and_then(|ty| ty.strip_suffix('>'));
    Ok(Field {
        wire: rename.unwrap_or_else(|| ident.trim_start_matches("r#").to_owned()),
        location: location.ok_or_else(|| format!("no location doc line on {ident}"))?,
        required: inner.is_none(),
        list: inner.unwrap_or(ty).starts_with("Vec<"),
    })
}

/// Every param struct of a generated group source, keyed by the operation id
/// its doc summary names, with its fields in declaration order.
fn param_structs(source: &str) -> Result<BTreeMap<String, Vec<Field>>, Box<dyn Error>> {
    let mut out = BTreeMap::new();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        let Some(operation_id) = line
            .strip_prefix("/// Parameters for `")
            .and_then(|rest| rest.strip_suffix("` (path/query/header)."))
        else {
            continue;
        };
        let mut fields = Vec::new();
        let mut rename = None;
        let mut location = None;
        for line in lines.by_ref() {
            let line = line.trim();
            if line == "}" {
                break;
            }
            if let Some(value) = line
                .strip_prefix("#[serde(rename = \"")
                .and_then(|rest| rest.strip_suffix("\")]"))
            {
                rename = Some(value.to_owned());
            } else if let Some(doc) = line.strip_prefix("/// `")
                && location.is_none()
            {
                location = doc
                    .rsplit_once("` (")
                    .and_then(|(_, rest)| rest.strip_suffix(')'))
                    .map(str::to_owned);
            } else if let Some(decl) = line.strip_prefix("pub ")
                && !decl.starts_with("struct ")
            {
                fields.push(field(decl, rename.take(), location.take())?);
            }
        }
        if out.insert(operation_id.to_owned(), fields).is_some() {
            return Err(format!("two param structs for {operation_id}").into());
        }
    }
    Ok(out)
}

/// The table row of every operation equals the param struct the same emitter
/// pass wrote for it: the same wire names, locations, requiredness and list
/// shape, in field order — and an operation without a struct has an empty row.
#[test]
fn every_route_params_row_equals_its_param_struct() -> TestResult {
    let generated = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rest/generated");
    let mut compared = 0_usize;
    for (group, routes, params) in GROUPS {
        let source = std::fs::read_to_string(generated.join(format!("{group}.rs")))?;
        let mut structs = param_structs(&source)?;
        for ((_, _, operation_id), declared) in routes.iter().zip(*params) {
            let fields = structs.remove(*operation_id).unwrap_or_default();
            let row: Vec<Field> = declared
                .iter()
                .map(|param| Field {
                    wire: param.name.to_owned(),
                    location: match param.location {
                        ParamLocation::Path => "path",
                        ParamLocation::Query => "query",
                        ParamLocation::Header => "header",
                        ParamLocation::Cookie => "cookie",
                    }
                    .to_owned(),
                    required: param.required,
                    list: matches!(param.kind, ParamKind::Array(_)),
                })
                .collect();
            assert_eq!(row, fields, "{group} {operation_id}");
            compared += row.len();
        }
        assert!(
            structs.is_empty(),
            "{group}: param structs with no route: {:?}",
            structs.keys().collect::<Vec<_>>()
        );
    }
    assert!(compared > 0, "the reader found no param struct fields");
    Ok(())
}

/// What the field-name probe deserializer reports.
#[derive(Debug)]
enum Probe {
    /// The field names a derived `Deserialize` asked `deserialize_struct` for.
    Fields(&'static [&'static str]),
    /// Anything else the type asked for.
    Other(String),
}

impl fmt::Display for Probe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fields(fields) => write!(f, "struct fields {fields:?}"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl Error for Probe {}

impl serde::de::Error for Probe {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Other(msg.to_string())
    }
}

/// A deserializer that answers only by reporting the field names a derived
/// struct `Deserialize` passes to `deserialize_struct` — its wire names.
struct FieldProbe;

impl<'de> serde::Deserializer<'de> for FieldProbe {
    type Error = Probe;

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Probe> {
        Err(Probe::Other("not a struct".to_owned()))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Probe> {
        Err(Probe::Fields(fields))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

/// The wire field names serde's derive gives `T`.
fn serde_fields<T: DeserializeOwned>() -> Result<Vec<&'static str>, Box<dyn Error>> {
    match T::deserialize(FieldProbe) {
        Err(Probe::Fields(fields)) => Ok(fields.to_vec()),
        Err(other) => Err(other.into()),
        Ok(_) => Err("the probe never yields a value".into()),
    }
}

/// The source reader above agrees with serde's own derive on the wire names of
/// param structs that rename, use a raw identifier and carry docs-text headers,
/// and each struct's `PARAMS` names those same fields.
#[test]
fn the_source_reader_agrees_with_serde_and_params() -> TestResult {
    let cases: [(&str, &str, Vec<&'static str>, &[Param]); 4] = [
        (
            "ehr",
            "ehr_create",
            serde_fields::<ehr::EhrCreateParams>()?,
            ehr::EhrCreateParams::PARAMS,
        ),
        (
            "ehr",
            "ehr_status_update",
            serde_fields::<ehr::EhrStatusUpdateParams>()?,
            ehr::EhrStatusUpdateParams::PARAMS,
        ),
        (
            "query",
            "query_execute_adhoc_query",
            serde_fields::<query::QueryExecuteAdhocQueryParams>()?,
            query::QueryExecuteAdhocQueryParams::PARAMS,
        ),
        (
            "definition",
            "definition_template_adl1.4_example_get",
            serde_fields::<definition::DefinitionTemplateAdl14ExampleGetParams>()?,
            definition::DefinitionTemplateAdl14ExampleGetParams::PARAMS,
        ),
    ];
    let generated = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rest/generated");
    for (group, operation_id, serde_names, params) in cases {
        let source = std::fs::read_to_string(generated.join(format!("{group}.rs")))?;
        let structs = param_structs(&source)?;
        let fields = structs
            .get(operation_id)
            .ok_or_else(|| format!("no param struct for {operation_id}"))?;
        let read: Vec<&str> = fields.iter().map(|field| field.wire.as_str()).collect();
        assert_eq!(read, serde_names, "{operation_id}");
        let table: Vec<&str> = params.iter().map(|param| param.name).collect();
        assert_eq!(table, serde_names, "{operation_id}");
    }
    Ok(())
}

/// A match names the declared query parameters and headers, with the kinds
/// their schemas state: headers case-insensitively, and an undeclared query
/// key belonging to the exploded `query_parameters` object.
#[test]
fn a_match_names_its_declared_query_parameters_and_headers() -> TestResult {
    let adhoc = matched(&Method::GET, "/query/aql")?;
    let q = adhoc.query_param("q").ok_or("q is declared")?;
    assert_eq!(
        (q.location, q.required, q.kind),
        (ParamLocation::Query, true, ParamKind::Text)
    );
    assert_eq!(
        adhoc.query_param("ehr_id").map(|param| param.kind),
        Some(ParamKind::Uuid)
    );
    assert_eq!(
        adhoc.query_param("offset").map(|param| param.kind),
        Some(ParamKind::Integer)
    );
    assert!(adhoc.query_param("Q").is_none(), "query names are exact");
    assert_eq!(
        adhoc.query_key("ehr_id").map(|param| param.name),
        Some("ehr_id")
    );
    let members = adhoc
        .query_key("systolic_bp")
        .ok_or("an undeclared key is a query_parameters member")?;
    assert_eq!(
        (members.name, members.explode, members.kind),
        ("query_parameters", true, ParamKind::Object)
    );
    let accept = adhoc.header_param("accept").ok_or("Accept is declared")?;
    assert_eq!(accept.name, "Accept");
    assert_eq!(accept.kind, ParamKind::Enum(&["application/json"]));
    assert!(adhoc.header_param("X-Forwarded-For").is_none());

    let get = matched(&Method::GET, "/ehr/7d44b88c/composition/8849182c::sys::1")?;
    assert_eq!(get.operation_id, "composition_get");
    assert_eq!(
        get.query_param("version_at_time").map(|param| param.kind),
        Some(ParamKind::DateTime)
    );
    assert!(
        get.query_key("anything").is_none(),
        "no object to collect it"
    );

    let update = matched(&Method::PUT, "/ehr/7d44b88c/ehr_status")?;
    assert_eq!(update.operation_id, "ehr_status_update");
    let if_match = update
        .header_param("if-match")
        .ok_or("If-Match is declared")?;
    assert!(if_match.required);
    let prefer = update.header_param("PREFER").ok_or("Prefer is declared")?;
    assert_eq!(
        prefer.kind,
        ParamKind::Enum(&[
            "return=representation",
            "return=minimal",
            "return=identifier"
        ])
    );
    let tags = update
        .header_param("openehr-item-tag")
        .ok_or("openehr-item-tag is declared")?;
    assert_eq!(
        (tags.explode, tags.kind),
        (true, ParamKind::Array(&ParamKind::Object))
    );
    let audit = update
        .header_param("openEHR-AUDIT-DETAILS")
        .ok_or("the docs text defines openehr-audit-details")?;
    assert_eq!(audit.kind, ParamKind::Array(&ParamKind::Text));
    Ok(())
}

/// Every EHR-area and demographic path parameter that carries an openEHR
/// identifier names its BASE class, and the two `uid_based_id` variants keep
/// their distinct classes per operation (OAS `uid_based_id`,
/// `uid_based_id_as_version_uid`, `uid_based_id_as_versioned_object_uid`).
#[test]
fn path_parameters_name_their_identifier_class() {
    use openehr_its::rest::routes::{IdentifierClass, Param, ParamLocation};
    type Group = (
        &'static [(&'static str, &'static str, &'static str)],
        &'static [&'static [Param]],
    );
    let groups: [Group; 6] = [
        (admin::ROUTES, admin::ROUTE_PARAMS),
        (definition::ROUTES, definition::ROUTE_PARAMS),
        (demographic::ROUTES, demographic::ROUTE_PARAMS),
        (ehr::ROUTES, ehr::ROUTE_PARAMS),
        (query::ROUTES, query::ROUTE_PARAMS),
        (system::ROUTES, system::ROUTE_PARAMS),
    ];
    let mut uid_based = BTreeMap::new();
    for (routes, params) in groups {
        for ((_, _, operation), row) in routes.iter().zip(params.iter()) {
            for p in row.iter().filter(|p| p.location == ParamLocation::Path) {
                let expected = match p.name {
                    "ehr_id" | "versioned_object_uid" | "contribution_uid" => {
                        Some(IdentifierClass::HierObject)
                    }
                    "version_uid" => Some(IdentifierClass::ObjectVersion),
                    "uid_based_id" => {
                        uid_based.insert(*operation, p.identifier);
                        continue;
                    }
                    _ => None,
                };
                assert_eq!(p.identifier, expected, "{operation}: {}", p.name);
            }
            for p in row.iter().filter(|p| p.location != ParamLocation::Path) {
                assert_eq!(p.identifier, None, "{operation}: {}", p.name);
            }
        }
    }
    for (operation, class) in [
        ("composition_get", IdentifierClass::UidBased),
        ("composition_update", IdentifierClass::HierObject),
        ("composition_delete", IdentifierClass::ObjectVersion),
        ("composition_tags_get", IdentifierClass::UidBased),
        ("person_get", IdentifierClass::UidBased),
        ("person_update", IdentifierClass::HierObject),
        ("person_delete", IdentifierClass::ObjectVersion),
    ] {
        assert_eq!(uid_based.get(operation), Some(&Some(class)), "{operation}");
    }
    assert_eq!(IdentifierClass::ObjectVersion.as_str(), "OBJECT_VERSION_ID");
}

/// `from_request` decodes an AQL query string as the generated router does:
/// the declared keys into their fields, every other pair into
/// `query_parameters` with its value read as JSON when it parses (ITS-REST
/// `query/` `query_parameters`, OAS `style: form, explode: true`), and `+`
/// kept as a literal plus (RFC 3986 §2.1).
#[test]
fn from_request_decodes_the_adhoc_query_string() {
    let m = matched(&Method::GET, "/query/aql").unwrap();
    let params = query::QueryExecuteAdhocQueryParams::from_request(
        &m,
        Some("q=SELECT+1&offset=1&fetch=2&uid=x&n=3"),
        &http::HeaderMap::new(),
    )
    .unwrap();
    assert_eq!(params.q, "SELECT+1");
    assert_eq!(params.offset, Some(1));
    assert_eq!(params.fetch, Some(2));
    let members = params.query_parameters.unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(members.get("uid"), Some(&serde_json::json!("x")));
    assert_eq!(members.get("n"), Some(&serde_json::json!(3)));
}

/// A repeated member and a missing required parameter are refused with the
/// `400` the generated handler returns, naming the parameter.
#[test]
fn from_request_refuses_as_the_router_does() {
    let m = matched(&Method::GET, "/query/aql").unwrap();
    let refused = |query: &str| match query::QueryExecuteAdhocQueryParams::from_request(
        &m,
        Some(query),
        &http::HeaderMap::new(),
    ) {
        Err(openehr_its::rest::runtime::ApiError::BadRequest(message)) => message,
        other => panic!("{query}: expected a 400, got {other:?}"),
    };
    assert_eq!(
        refused("q=a&uid=x&uid=y"),
        "the member `uid` of the query parameter `query_parameters` is given more than once"
    );
    assert_eq!(
        refused("offset=1"),
        "the required query parameter `q` is missing"
    );
    assert_eq!(
        refused("q=a&offset=one"),
        "the query parameter `offset` is not a valid value: invalid digit found in string"
    );
}

/// The stored-query operations decode their path parameters (percent-decoded)
/// and query string, and `definition_query_store` its path, query and
/// headers.
#[test]
fn from_request_covers_the_stored_query_operations() {
    let mut headers = http::HeaderMap::new();
    headers.insert("accept", http::HeaderValue::from_static("application/json"));
    let m = matched(&Method::GET, "/query/org.example%3A%3Avitals").unwrap();
    let p = query::QueryExecuteStoredQueryParams::from_request(&m, Some("ehr_id=e1&n=2"), &headers)
        .unwrap();
    assert_eq!(p.qualified_query_name, "org.example::vitals");
    assert_eq!(p.ehr_id.as_deref(), Some("e1"));
    assert_eq!(p.accept.as_deref(), Some("application/json"));
    assert_eq!(
        p.query_parameters.unwrap().get("n"),
        Some(&serde_json::json!(2))
    );
    let m = matched(&Method::GET, "/query/org.example::vitals/1.0.2").unwrap();
    let p = query::QueryExecuteStoredQueryVersionParams::from_request(&m, None, &headers).unwrap();
    assert_eq!(p.qualified_query_name, "org.example::vitals");
    assert_eq!(p.version, "1.0.2");
    assert!(p.query_parameters.is_none());
    let m = matched(&Method::PUT, "/definition/query/org.example::vitals").unwrap();
    let p = definition::DefinitionQueryStoreYamlParams::from_request(
        &m,
        Some("query_type=AQL"),
        &headers,
    )
    .unwrap();
    assert_eq!(p.qualified_query_name, "org.example::vitals");
    assert_eq!(p.query_type.as_deref(), Some("AQL"));
}
