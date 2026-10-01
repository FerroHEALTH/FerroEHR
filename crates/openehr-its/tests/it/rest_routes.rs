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
//! matcher's precedence (a literal segment over a parameter) and its path
//! parameter decoding (RFC 3986 §2.1) are pinned here.

use std::error::Error;
use std::path::Path;

use http::Method;
use openehr_its::rest::generated::{admin, definition, demographic, ehr, query, system};
use openehr_its::rest::routes::{Lookup, PathParam, RouteMatch, lookup};

/// A test's plumbing error: every fallible step propagates with `?`.
type TestResult = Result<(), Box<dyn Error>>;

/// One generated route table.
type RouteTable = &'static [(&'static str, &'static str, &'static str)];

/// Every generated API group with its route table.
const GROUPS: &[(&str, RouteTable)] = &[
    ("admin", admin::ROUTES),
    ("definition", definition::ROUTES),
    ("demographic", demographic::ROUTES),
    ("ehr", ehr::ROUTES),
    ("query", query::ROUTES),
    ("system", system::ROUTES),
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
    let listed: Vec<String> = GROUPS.iter().map(|(name, _)| (*name).to_owned()).collect();
    assert_eq!(listed, generated);
    Ok(())
}

/// A concrete path for every route of every group matches back to that
/// route's operation, with its path parameters named in order.
#[test]
fn every_route_matches_back_to_its_operation() -> TestResult {
    let mut checked = 0_usize;
    for (group, routes) in GROUPS {
        for (method, template, operation_id) in *routes {
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
            for param in &found.path_params {
                assert_eq!(param.raw, format!("sample-{}", param.name));
            }
            checked += 1;
        }
    }
    let total: usize = GROUPS.iter().map(|(_, routes)| routes.len()).sum();
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
