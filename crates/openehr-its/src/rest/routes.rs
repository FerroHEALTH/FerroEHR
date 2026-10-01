// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The operation matcher over every API group's generated route table.
//!
//! [`lookup`] names the ITS-REST operation a request addresses — its group,
//! `operationId`, path template and path parameters — from the method and the
//! path alone, without reading the body. An intermediary that must pass a
//! request through byte-identical (a commit body, `ETag`, `Location`) matches
//! it here and forwards it unchanged; decoding and re-encoding through the
//! typed bodies cannot guarantee identical bytes.
//!
//! Matching follows the precedence of the usual HTTP routers: a path is
//! resolved first, then the method. Every segment of a template is either a
//! literal, compared byte for byte with the request's segment as received, or
//! a `{name}` parameter, which matches one non-empty segment. When several
//! templates match, the one with a literal at the first position where they
//! differ wins (`/query/aql` over `/query/{qualified_query_name}`). An RFC
//! 6570 query expansion (`{?name*}`) is not a path segment and is ignored.
//! The method must equal a declared one exactly: no openEHR spec governs
//! routing a `HEAD` to a `GET`, so neither does this matcher.

use http::Method;

use super::generated::{admin, definition, demographic, ehr, query, system};

/// One generated route table: `(method, path, operation_id)` per operation.
type RouteTable = &'static [(&'static str, &'static str, &'static str)];

/// Every API group's name and route table, in the order the groups are
/// generated.
const GROUPS: &[(&str, RouteTable)] = &[
    ("admin", admin::ROUTES),
    ("definition", definition::ROUTES),
    ("demographic", demographic::ROUTES),
    ("ehr", ehr::ROUTES),
    ("query", query::ROUTES),
    ("system", system::ROUTES),
];

/// What a request's method and path address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// One operation of one API group.
    Matched(RouteMatch),
    /// A resource the API defines, under a method it does not declare for it
    /// (`405`); `allowed` lists the declared methods, the `Allow` field values.
    MethodNotAllowed {
        /// The methods the resource declares, in route-table order.
        allowed: Vec<&'static str>,
    },
    /// A path no operation of the API names (`404`).
    NotFound,
}

/// The operation a request addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch {
    /// The API group the operation belongs to (`ehr`, `query`, …).
    pub group: &'static str,
    /// The operation's OAS `operationId`.
    pub operation_id: &'static str,
    /// The operation's path template, as the route table carries it.
    pub template: &'static str,
    /// The request method.
    pub method: Method,
    /// The path parameters, in path order.
    pub path_params: Vec<PathParam>,
}

impl RouteMatch {
    /// The path parameter `name`, when the template declares it.
    #[must_use]
    pub fn path_param(&self, name: &str) -> Option<&PathParam> {
        self.path_params.iter().find(|param| param.name == name)
    }
}

/// One path parameter of a matched request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathParam {
    /// The parameter name the template declares.
    pub name: &'static str,
    /// The segment as received, still percent-encoded — what a forwarded
    /// request sends on unchanged.
    pub raw: String,
}

impl PathParam {
    /// The segment percent-decoded (RFC 3986 §2.1), as the operation's
    /// parameter value.
    ///
    /// # Errors
    /// Returns the UTF-8 error when the decoded octets are not UTF-8 text.
    pub fn decoded(&self) -> Result<String, std::string::FromUtf8Error> {
        urlencoding::decode(&self.raw).map(std::borrow::Cow::into_owned)
    }
}

/// One segment of a path template.
#[derive(Clone, Copy)]
enum Segment<'a> {
    /// A segment that must equal the request's segment.
    Literal(&'a str),
    /// A `{name}` segment that captures the request's segment.
    Param(&'a str),
}

/// The segments of `template`, its RFC 6570 query expansion dropped.
fn template_segments(template: &str) -> Vec<Segment<'_>> {
    let path = template.split("{?").next().unwrap_or(template);
    path.strip_prefix('/')
        .unwrap_or(path)
        .split('/')
        .map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
                .map_or(Segment::Literal(segment), Segment::Param)
        })
        .collect()
}

/// The specificity of a matched template: `true` per literal segment, so a
/// lexicographic maximum prefers a literal at the first differing position.
fn specificity(segments: &[Segment<'_>]) -> Vec<bool> {
    segments
        .iter()
        .map(|segment| matches!(segment, Segment::Literal(_)))
        .collect()
}

/// Whether `segments` match the request's `parts`.
fn matches_path(segments: &[Segment<'_>], parts: &[&str]) -> bool {
    segments.len() == parts.len()
        && segments
            .iter()
            .zip(parts)
            .all(|(segment, part)| match segment {
                Segment::Literal(literal) => literal == part,
                Segment::Param(_) => !part.is_empty(),
            })
}

/// Names the operation `method` and `path` address.
///
/// `path` is relative to the API base — `/ehr/7d44…/composition/8849…::1`,
/// not `/openehr/v1/ehr/…` — and any query string after a `?` is ignored.
#[must_use]
pub fn lookup(method: &Method, path: &str) -> Lookup {
    let path = path.split('?').next().unwrap_or(path);
    let parts: Vec<&str> = path.strip_prefix('/').unwrap_or(path).split('/').collect();
    let candidates: Vec<Candidate> = GROUPS
        .iter()
        .flat_map(|&(group, routes)| {
            routes
                .iter()
                .map(move |&(method, template, operation_id)| Candidate {
                    group,
                    method,
                    template,
                    operation_id,
                    segments: template_segments(template),
                })
        })
        .filter(|candidate| matches_path(&candidate.segments, &parts))
        .collect();
    let Some(best) = candidates
        .iter()
        .map(|candidate| specificity(&candidate.segments))
        .max()
    else {
        return Lookup::NotFound;
    };
    // The routes of the one resource the path resolves to, every method.
    let resource: Vec<Candidate> = candidates
        .into_iter()
        .filter(|candidate| specificity(&candidate.segments) == best)
        .collect();
    let Some(chosen) = resource
        .iter()
        .find(|candidate| candidate.method == method.as_str())
    else {
        let mut allowed: Vec<&'static str> = Vec::new();
        for candidate in &resource {
            if !allowed.contains(&candidate.method) {
                allowed.push(candidate.method);
            }
        }
        return Lookup::MethodNotAllowed { allowed };
    };
    let path_params = chosen
        .segments
        .iter()
        .zip(&parts)
        .filter_map(|(segment, part)| match *segment {
            Segment::Param(name) => Some(PathParam {
                name,
                raw: (*part).to_owned(),
            }),
            Segment::Literal(_) => None,
        })
        .collect();
    Lookup::Matched(RouteMatch {
        group: chosen.group,
        operation_id: chosen.operation_id,
        template: chosen.template,
        method: method.clone(),
        path_params,
    })
}

/// One route of a route table, with its parsed template.
struct Candidate {
    group: &'static str,
    method: &'static str,
    template: &'static str,
    operation_id: &'static str,
    segments: Vec<Segment<'static>>,
}
