// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the handlers of `/Users` and `/Groups` share: the resource schema,
//! the base URL, the page bound and the filter a resource type supports, the
//! query parameters of a read and a query, and the access record each
//! response carries.
//!
//! A query is always a `ListResponse`, an empty one with `200` when nothing
//! matches (RFC 7644 §3.4.2), and a filter outside the supported subset is
//! `400` `invalidFilter` (§3.4.2.2). No openEHR spec governs SCIM: our own
//! design over RFC 7644.

use axum::response::Response;
use ferroehr::ids::VoId;
use ferroehr::system_log::event::AccessDomain;
use http::{HeaderValue, StatusCode, header};
use serde::Deserialize;
use serde_json::Value;

use crate::scim::endpoint::scim_json;
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::filter::{
    self, CompValue, CompareOp, Filter, FilterSupport, Logic, Term, Test,
};
use crate::scim::protocol::list::{ListResponse, Page, Projection};
use crate::scim::protocol::resource::{Parsed, Resource, ResourceSchema};
use crate::system_log::middleware::{AuditObject, AuditOpId};

/// The query of `GET /Users` and `GET /Groups` (RFC 7644 §3.4.2).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListQuery {
    filter: Option<String>,
    start_index: Option<String>,
    count: Option<String>,
    attributes: Option<String>,
    excluded_attributes: Option<String>,
}

/// The attribute projection of a read by id (RFC 7644 §3.9).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadQuery {
    attributes: Option<String>,
    excluded_attributes: Option<String>,
}

impl ReadQuery {
    /// The projection the query asks for.
    ///
    /// # Errors
    /// The [`Projection::parse`] refusals.
    pub(super) fn projection(&self) -> Result<Projection, ScimError> {
        Projection::parse(
            self.attributes.as_deref(),
            self.excluded_attributes.as_deref(),
        )
    }
}

/// What a query asks for: its page, its projection and its checked filter,
/// `None` when it carries none.
pub(super) type Asked = (Page, Projection, Option<Filter>);

/// The schema and the query bounds of one resource type.
pub(super) struct Listing {
    /// The resource type's schema.
    pub(super) schema: ResourceSchema,
    /// The SCIM base URL.
    pub(super) base_url: String,
    max_results: u64,
    filter: FilterSupport,
}

impl Listing {
    /// The listing of the resource type `schema` under `scim`, filtered by
    /// `eq` and `and` over the `attributes` of its base schema `urn`.
    ///
    /// # Errors
    /// The refusal naming an attribute the filter subset cannot name.
    pub(super) fn new(
        schema: ResourceSchema,
        scim: &ferroehr::config::scim::ScimConfig,
        urn: &str,
        attributes: &[&str],
    ) -> Result<Self, String> {
        Ok(Self {
            schema,
            base_url: scim.base_url().unwrap_or_default(),
            max_results: scim.filter_max_results,
            filter: FilterSupport::equality_conjunction(urn, attributes)
                .map_err(|error| error.to_string())?,
        })
    }

    /// The resource a create request's `body` carries (RFC 7644 §3.3).
    ///
    /// # Errors
    /// A `400` `invalidSyntax` when it is not JSON, and the
    /// [`Resource::from_request`] refusals.
    pub(super) fn requested(&self, body: &[u8]) -> Result<Parsed, ScimError> {
        Resource::from_request(&json_body(body)?, &self.schema)
    }

    /// What the query `query` asks for.
    ///
    /// # Errors
    /// The refusals of a malformed page or projection, and the `400`
    /// `invalidFilter` of a filter outside the supported subset.
    pub(super) fn asked(&self, query: &ListQuery) -> Result<Asked, ScimError> {
        let page = Page::parse(
            query.start_index.as_deref(),
            query.count.as_deref(),
            self.max_results,
        )?;
        let projection = Projection::parse(
            query.attributes.as_deref(),
            query.excluded_attributes.as_deref(),
        )?;
        let parsed = query.filter.as_deref().map(filter::parse).transpose()?;
        if let Some(parsed) = &parsed {
            self.filter.check(parsed)?;
        }
        Ok((page, projection, parsed))
    }

    /// The full representation `full` of a `kind` under `projection`, or
    /// `full` itself, logged, when it does not meet the schema.
    pub(super) fn projected(&self, full: Value, projection: &Projection, kind: &str) -> Value {
        match Resource::from_representation(&full, &self.schema) {
            Ok(resource) => projection.apply(&resource, &self.schema),
            Err(error) => {
                tracing::error!(%error, kind, "a SCIM representation does not meet its schema");
                full
            }
        }
    }

    /// `response` with the `Location` of the resource `id` at `endpoint`.
    pub(super) fn located(&self, mut response: Response, endpoint: &str, id: VoId) -> Response {
        if let Ok(location) = HeaderValue::from_str(&format!("{}/{endpoint}/{id}", self.base_url)) {
            response.headers_mut().insert(header::LOCATION, location);
        }
        response
    }
}

/// The `ListResponse` of `resources`, one page of `total` from `page`, with
/// the access record of `op` naming the one resource `ids` holds.
pub(super) fn listed(
    resources: Vec<Value>,
    (total, page): (u64, &Page),
    op: &'static str,
    ids: &[VoId],
) -> Response {
    let only = match ids {
        [one] => Some(*one),
        _ => None,
    };
    let count = u64::try_from(resources.len()).unwrap_or(u64::MAX);
    let response = scim_json(
        StatusCode::OK,
        &ListResponse::new(resources, total, page.start_index()),
    );
    audited(response, op, only, Some(count))
}

/// The JSON object `body` holds.
///
/// # Errors
/// A `400` `invalidSyntax` when it is not JSON.
pub(super) fn json_body(body: &[u8]) -> Result<Value, ScimError> {
    serde_json::from_slice(body).map_err(|_error| {
        ScimError::bad_request(ScimType::InvalidSyntax, "The body is not a JSON object.")
    })
}

/// The `eq` comparisons of the supported filter `parsed`, each its attribute
/// name and its string value.
///
/// # Errors
/// A `400` `invalidFilter` when it compares with a value that is not a
/// string.
pub(super) fn equalities(parsed: &Filter) -> Result<Vec<(&str, &str)>, ScimError> {
    let mut leaves = Vec::new();
    collect(parsed, &mut leaves);
    leaves
        .into_iter()
        .map(|(name, value)| {
            value.map(|value| (name, value)).ok_or_else(|| {
                ScimError::bad_request(
                    ScimType::InvalidFilter,
                    format!("'{name}' is compared with a string."),
                )
            })
        })
        .collect()
}

/// Pushes each `eq` comparison of `parsed` to `leaves`.
fn collect<'a>(parsed: &'a Filter, leaves: &mut Vec<(&'a str, Option<&'a str>)>) {
    match parsed {
        Logic::Leaf(Term::Attr(expression)) => {
            let value = match &expression.test {
                Test::Compare(CompareOp::Eq, CompValue::String(value)) => Some(value.as_str()),
                _ => None,
            };
            leaves.push((expression.path.name.as_str(), value));
        }
        Logic::And(parts) => {
            for part in parts {
                collect(part, leaves);
            }
        }
        // NOTE: `FilterSupport::check` admitted only `eq` and `and`.
        Logic::Leaf(Term::ValuePath(..)) | Logic::Or(_) | Logic::Not(_) => {}
    }
}

/// `response` with the access record's operation and object: the `PERSON`
/// or `GROUP` `party`, and the number of resources a query served.
pub(super) fn audited(
    mut response: Response,
    op: &'static str,
    party: Option<VoId>,
    result_count: Option<u64>,
) -> Response {
    response.extensions_mut().insert(AuditOpId(op));
    response.extensions_mut().insert(AuditObject {
        uid: party.map(|party| party.to_string()),
        result_count,
        domain: Some(AccessDomain::Demographic),
        ..AuditObject::default()
    });
    response
}
