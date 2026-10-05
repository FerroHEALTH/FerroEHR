// SPDX-FileCopyrightText: Cadasto B.V.
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
//!
//! A match also names the operation's declared parameters ([`Param`]) — path,
//! query and header — from the generated `ROUTE_PARAMS` table, which is each
//! param struct's own `PARAMS`: the parameters the vendored OAS
//! (`vendor/rest-oas/<group>-codegen.openapi.yaml`) declares, plus the request
//! headers the ITS-REST docs text defines beyond them. An intermediary passes
//! on exactly those; no openEHR spec governs forwarding, so which undeclared
//! input it refuses is its own design.

use http::Method;

use super::generated::{admin, definition, demographic, ehr, query, system};

/// One generated route table: `(method, path, operation_id)` per operation.
type RouteTable = &'static [(&'static str, &'static str, &'static str)];

/// One generated parameter table, index-aligned with its route table.
type ParamTable = &'static [&'static [Param]];

/// One generated request-media table, index-aligned with its route table.
type MediaTable = &'static [&'static [&'static str]];

/// Every API group's name, route table and parameter table, in the order the
/// groups are generated.
const GROUPS: &[(&str, RouteTable, ParamTable, MediaTable)] = &[
    (
        "admin",
        admin::ROUTES,
        admin::ROUTE_PARAMS,
        admin::ROUTE_REQUEST_MEDIA,
    ),
    (
        "definition",
        definition::ROUTES,
        definition::ROUTE_PARAMS,
        definition::ROUTE_REQUEST_MEDIA,
    ),
    (
        "demographic",
        demographic::ROUTES,
        demographic::ROUTE_PARAMS,
        demographic::ROUTE_REQUEST_MEDIA,
    ),
    (
        "ehr",
        ehr::ROUTES,
        ehr::ROUTE_PARAMS,
        ehr::ROUTE_REQUEST_MEDIA,
    ),
    (
        "query",
        query::ROUTES,
        query::ROUTE_PARAMS,
        query::ROUTE_REQUEST_MEDIA,
    ),
    (
        "system",
        system::ROUTES,
        system::ROUTE_PARAMS,
        system::ROUTE_REQUEST_MEDIA,
    ),
];

/// One parameter an operation declares, as its OAS Parameter Object states it.
///
/// The fields follow OAS 3.0.3 §Parameter Object
/// (<https://spec.openapis.org/oas/v3.0.3#parameter-object>): `name` is spelled
/// as the OAS spells it, `required` is the stated value (absent means
/// `false`), and `explode` is the stated value or the style's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    /// The parameter name, spelled as the OAS spells it.
    pub name: &'static str,
    /// Where the parameter travels.
    pub location: ParamLocation,
    /// Whether the operation requires the parameter.
    pub required: bool,
    /// Whether an array or object value spreads over several pairs: under the
    /// `form` style each member of an exploded object is a query key of its own.
    pub explode: bool,
    /// The value shape the parameter's schema states.
    pub kind: ParamKind,
    /// The openEHR identifier class a path parameter carries, which the OAS
    /// states only in the parameter's description (the RM attribute the value
    /// is "taken from"); `None` for a value that is no openEHR identifier and
    /// for every non-path parameter.
    pub identifier: Option<IdentifierClass>,
}

/// An openEHR identifier class from BASE `base_types.identification`, the
/// form a path parameter's value takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdentifierClass {
    /// `HIER_OBJECT_ID`: the `uid` of a versioned object, an EHR or a
    /// contribution.
    HierObject,
    /// `OBJECT_VERSION_ID`: the `uid` of a `VERSION`
    /// (`object_id::creating_system_id::version_tree_id`).
    ObjectVersion,
    /// `UID_BASED_ID`: either of the two above.
    UidBased,
}

impl IdentifierClass {
    /// The BASE class name (`"HIER_OBJECT_ID"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HierObject => "HIER_OBJECT_ID",
            Self::ObjectVersion => "OBJECT_VERSION_ID",
            Self::UidBased => "UID_BASED_ID",
        }
    }
}

/// Where a parameter travels (OAS 3.0.3 §Parameter Locations).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamLocation {
    /// A `{name}` segment of the path template.
    Path,
    /// A query-string parameter.
    Query,
    /// A request header field.
    Header,
    /// A cookie.
    Cookie,
}

/// The value shape a parameter's schema states: its `enum`, `type`, `format`
/// and `items`, the `$ref`s resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// A schema `enum`: the listed values, in document order, as wire text.
    Enum(&'static [&'static str]),
    /// `type: string` without a `format`.
    Text,
    /// `type: string`, `format: uuid`.
    Uuid,
    /// `type: string`, `format: date`.
    Date,
    /// `type: string`, `format: date-time` (the bundles also spell it
    /// `datetime`).
    DateTime,
    /// `type: string` with a `format` none of the kinds above names.
    Formatted(&'static str),
    /// `type: integer`, whatever its `format`.
    Integer,
    /// `type: number`, whatever its `format`.
    Number,
    /// `type: boolean`.
    Boolean,
    /// `type: object`.
    Object,
    /// `type: array`, whose items have the inner kind.
    Array(&'static ParamKind),
    /// A schema that states no `type`.
    Unspecified,
}

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
    /// Every parameter the operation declares — path, query and header — in
    /// declaration order.
    pub params: &'static [Param],
    /// The media types the operation's request body is declared in (the OAS
    /// `requestBody.content` keys, `text/plain` for a stored query), empty when
    /// it takes no body. A forwarding intermediary sets `Content-Type` from
    /// them where the operation declares no `Content-Type` parameter.
    pub request_media: &'static [&'static str],
}

impl RouteMatch {
    /// The path parameter `name`, when the template declares it.
    #[must_use]
    pub fn path_param(&self, name: &str) -> Option<&PathParam> {
        self.path_params.iter().find(|param| param.name == name)
    }

    /// The declared query parameter `name`, compared byte for byte.
    #[must_use]
    pub fn query_param(&self, name: &str) -> Option<&'static Param> {
        self.declared(ParamLocation::Query)
            .find(|param| param.name == name)
    }

    /// The declared request header `name`, compared case-insensitively: field
    /// names are case-insensitive (RFC 9110 §5.1).
    #[must_use]
    pub fn header_param(&self, name: &str) -> Option<&'static Param> {
        self.declared(ParamLocation::Header)
            .find(|param| param.name.eq_ignore_ascii_case(name))
    }

    /// The declared parameter a query key `key` belongs to.
    ///
    /// That is the query parameter named `key`, else the exploded `form`-style
    /// object parameter, whose members each travel as a query key of their own
    /// (OAS 3.0.3 §Style Examples) — `query_parameters` on the AQL operations.
    #[must_use]
    pub fn query_key(&self, key: &str) -> Option<&'static Param> {
        self.query_param(key).or_else(|| {
            self.declared(ParamLocation::Query)
                .find(|param| param.explode && param.kind == ParamKind::Object)
        })
    }

    /// The declared parameters at `location`, in declaration order.
    fn declared(&self, location: ParamLocation) -> impl Iterator<Item = &'static Param> {
        self.params
            .iter()
            .filter(move |param| param.location == location)
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
        .flat_map(|&(group, routes, params, media)| {
            // The generated module asserts `ROUTE_PARAMS.len() == ROUTES.len()`
            // and the same of `ROUTE_REQUEST_MEDIA` at compile time, so the
            // zip pairs every route with its rows.
            routes.iter().zip(params).zip(media).map(
                move |((&(method, template, operation_id), &params), &request_media)| Candidate {
                    group,
                    method,
                    template,
                    operation_id,
                    params,
                    request_media,
                    segments: template_segments(template),
                },
            )
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
        params: chosen.params,
        request_media: chosen.request_media,
    })
}

/// One route of a route table, with its parsed template.
struct Candidate {
    group: &'static str,
    method: &'static str,
    template: &'static str,
    operation_id: &'static str,
    params: &'static [Param],
    request_media: &'static [&'static str],
    segments: Vec<Segment<'static>>,
}
