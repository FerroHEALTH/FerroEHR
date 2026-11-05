// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! SCIM query responses and parameters.
//!
//! The `ListResponse` message (RFC 7644 §3.4.2), the `SearchRequest` message
//! (§3.4.3), pagination (§3.4.2.4) and the `attributes` and
//! `excludedAttributes` projection (§3.4.2.5, §3.9).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::message;
use crate::scim::protocol::path::AttrPath;
use crate::scim::protocol::resource::{Container, Resource, ResourceSchema};
use crate::scim::protocol::schema::{AttributeDefinition, Returned};
use crate::scim::protocol::urn;

/// The `ListResponse` message (RFC 7644 §3.4.2, Figure 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListResponse<T> {
    /// The message schema, `urn:ietf:params:scim:api:messages:2.0:ListResponse`.
    pub schemas: Vec<String>,
    /// The number of results the query matched, which may exceed the page.
    pub total_results: u64,
    /// The number of resources in this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_per_page: Option<u64>,
    /// The 1-based index of the first resource in this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_index: Option<u64>,
    /// The resources of this page.
    #[serde(rename = "Resources")]
    pub resources: Vec<T>,
}

impl<T> ListResponse<T> {
    /// Creates the page of `resources` that starts at `start_index` of
    /// `total_results` results.
    ///
    /// The page always carries `itemsPerPage`, `startIndex` and `Resources`,
    /// an empty array when nothing matched; a query that matches nothing
    /// "SHALL return success (HTTP status code 200) with "totalResults" set to
    /// a value of 0".
    #[must_use]
    pub fn new(resources: Vec<T>, total_results: u64, start_index: u64) -> Self {
        Self {
            schemas: vec![urn::LIST_RESPONSE.to_owned()],
            total_results,
            items_per_page: Some(u64::try_from(resources.len()).unwrap_or(u64::MAX)),
            start_index: Some(start_index),
            resources,
        }
    }
}

/// The page a query asks for (RFC 7644 §3.4.2.4, Table 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    start_index: u64,
    count: u64,
}

impl Page {
    /// Reads the `startIndex` and `count` query parameters against the
    /// provider's `maxResults`.
    ///
    /// A `startIndex` below 1 "SHALL be interpreted as 1", a negative `count`
    /// "as "0"", an absent `count` is `max_results`, and a larger one is
    /// lowered to it, since the provider "MAY return fewer results".
    ///
    /// # Errors
    ///
    /// Returns a `400` with `invalidValue` when a parameter is not an integer.
    pub fn parse(
        start_index: Option<&str>,
        count: Option<&str>,
        max_results: u64,
    ) -> Result<Self, ScimError> {
        let integer = |name: &str, text: &str| {
            text.trim().parse::<i64>().map_err(|_error| {
                ScimError::bad_request(
                    ScimType::InvalidValue,
                    format!("'{name}' is an integer (RFC 7644 §3.4.2.4)."),
                )
            })
        };
        let start_index = match start_index {
            Some(text) => u64::try_from(integer("startIndex", text)?.max(1)).unwrap_or(1),
            None => 1,
        };
        let count = match count {
            Some(text) => u64::try_from(integer("count", text)?.max(0))
                .unwrap_or(0)
                .min(max_results),
            None => max_results,
        };
        Ok(Self { start_index, count })
    }

    /// Returns the 1-based index of the first result.
    #[must_use]
    pub const fn start_index(&self) -> u64 {
        self.start_index
    }

    /// Returns how many results precede the page.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.start_index.saturating_sub(1)
    }

    /// Returns the most results the page holds.
    #[must_use]
    pub const fn count(&self) -> u64 {
        self.count
    }
}

/// The order a query sorts by (RFC 7644 §3.4.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortOrder {
    /// Ascending, the default.
    Ascending,
    /// Descending.
    Descending,
}

/// The `SearchRequest` message (RFC 7644 §3.4.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchRequest {
    /// The attributes to return.
    pub attributes: Option<Vec<String>>,
    /// The attributes to leave out.
    pub excluded_attributes: Option<Vec<String>>,
    /// The filter.
    pub filter: Option<String>,
    /// The attribute to sort by.
    pub sort_by: Option<String>,
    /// The sort order.
    pub sort_order: Option<SortOrder>,
    /// The 1-based index of the first result.
    pub start_index: Option<i64>,
    /// The most results to return.
    pub count: Option<i64>,
}

impl SearchRequest {
    /// Reads a `SearchRequest` body.
    ///
    /// # Errors
    ///
    /// Returns a `400` with `invalidSyntax` when the body is not a
    /// `SearchRequest` message or a member has the wrong JSON type.
    pub fn parse(body: &Value) -> Result<Self, ScimError> {
        let Value::Object(object) = body else {
            return Err(ScimError::bad_request(
                ScimType::InvalidSyntax,
                "A SearchRequest is a JSON object.",
            ));
        };
        let names = [
            "schemas",
            "attributes",
            "excludedAttributes",
            "filter",
            "sortBy",
            "sortOrder",
            "startIndex",
            "count",
        ];
        let [
            schemas,
            attributes,
            excluded,
            filter,
            sort_by,
            sort_order,
            start_index,
            count,
        ] = message::members(object, names, "SearchRequest")?;
        message::require_schema(schemas, urn::SEARCH_REQUEST, "SearchRequest")?;
        let strings =
            |value: Option<&Value>, name: &str| -> Result<Option<Vec<String>>, ScimError> {
                value
                    .map(|value| {
                        value
                            .as_array()
                            .and_then(|items| {
                                items
                                    .iter()
                                    .map(|item| item.as_str().map(str::to_owned))
                                    .collect::<Option<Vec<_>>>()
                            })
                            .ok_or_else(|| {
                                ScimError::bad_request(
                                    ScimType::InvalidSyntax,
                                    format!("'{name}' is an array of attribute names."),
                                )
                            })
                    })
                    .transpose()
            };
        let text = |value: Option<&Value>, name: &str| -> Result<Option<String>, ScimError> {
            value
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        ScimError::bad_request(
                            ScimType::InvalidSyntax,
                            format!("'{name}' is a string."),
                        )
                    })
                })
                .transpose()
        };
        let integer = |value: Option<&Value>, name: &str| -> Result<Option<i64>, ScimError> {
            value
                .map(|value| {
                    value.as_i64().ok_or_else(|| {
                        ScimError::bad_request(
                            ScimType::InvalidSyntax,
                            format!("'{name}' is an integer."),
                        )
                    })
                })
                .transpose()
        };
        let sort_order = sort_order
            .map(|value| {
                serde_json::from_value::<SortOrder>(value.clone()).map_err(|_error| {
                    ScimError::bad_request(
                        ScimType::InvalidSyntax,
                        "'sortOrder' is \"ascending\" or \"descending\".",
                    )
                })
            })
            .transpose()?;
        Ok(Self {
            attributes: strings(attributes, "attributes")?,
            excluded_attributes: strings(excluded, "excludedAttributes")?,
            filter: text(filter, "filter")?,
            sort_by: text(sort_by, "sortBy")?,
            sort_order,
            start_index: integer(start_index, "startIndex")?,
            count: integer(count, "count")?,
        })
    }
}

/// Which attributes a returned resource carries (RFC 7644 §3.9).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Projection {
    attributes: Option<Vec<AttrPath>>,
    excluded: Vec<AttrPath>,
}

impl Projection {
    /// Reads the comma-separated `attributes` and `excludedAttributes` query
    /// parameters.
    ///
    /// # Errors
    ///
    /// Returns a `400` with `invalidValue` when both are given, which §3.9
    /// calls "mutually exclusive", or an entry is not an attribute path.
    pub fn parse(
        attributes: Option<&str>,
        excluded_attributes: Option<&str>,
    ) -> Result<Self, ScimError> {
        let split = |text: &str| {
            text.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        Self::from_lists(
            attributes.map(split).as_deref(),
            excluded_attributes.map(split).as_deref(),
        )
    }

    /// Reads the `attributes` and `excludedAttributes` lists of a `SearchRequest`.
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Projection::parse`].
    pub fn from_lists(
        attributes: Option<&[String]>,
        excluded_attributes: Option<&[String]>,
    ) -> Result<Self, ScimError> {
        let paths = |names: &[String]| {
            names
                .iter()
                .map(|name| {
                    AttrPath::parse(name).map_err(|error| {
                        ScimError::bad_request(ScimType::InvalidValue, error.to_string())
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        };
        match (attributes, excluded_attributes) {
            (Some(_), Some(_)) => Err(ScimError::bad_request(
                ScimType::InvalidValue,
                "'attributes' and 'excludedAttributes' are mutually exclusive (RFC 7644 §3.9).",
            )),
            (Some(attributes), None) => Ok(Self {
                attributes: Some(paths(attributes)?),
                excluded: Vec::new(),
            }),
            (None, Some(excluded)) => Ok(Self {
                attributes: None,
                excluded: paths(excluded)?,
            }),
            (None, None) => Ok(Self::default()),
        }
    }

    /// Returns `resource` with the attributes this projection returns:
    /// always `schemas` and every `returned: always` attribute, never a
    /// `returned: never` one, and the `default` and `request` attributes as
    /// the parameters name them.
    #[must_use]
    pub fn apply(&self, resource: &Resource, schema: &ResourceSchema) -> Value {
        let mut out = Map::new();
        for (key, value) in resource.attributes() {
            if key == "schemas" {
                out.insert(key.clone(), value.clone());
            } else if let Some(extension) = schema.extension(key) {
                let container = Container::Extension(&extension.id);
                let inner: Map<String, Value> = value
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(name, value)| {
                        let attribute = extension.attribute(name)?;
                        Some((
                            name.clone(),
                            self.project(schema, container, attribute, value)?,
                        ))
                    })
                    .collect();
                if !inner.is_empty() {
                    out.insert(key.clone(), Value::Object(inner));
                }
            } else if let Some(attribute) = schema
                .common()
                .iter()
                .chain(&schema.base().attributes)
                .find(|a| a.name == *key)
                && let Some(value) = self.project(schema, Container::Core, attribute, value)
            {
                out.insert(key.clone(), value);
            }
        }
        Value::Object(out)
    }

    /// Returns the part of `value` this projection keeps of `attribute`.
    fn project(
        &self,
        schema: &ResourceSchema,
        container: Container<'_>,
        attribute: &AttributeDefinition,
        value: &Value,
    ) -> Option<Value> {
        // A path names the attribute when it is its extension's URN or resolves
        // to it; with `sub` it must name the whole attribute or that sub-attribute.
        let names = |path: &AttrPath, sub: Option<&str>| {
            if schema
                .extension_named(path)
                .is_some_and(|extension| Container::Extension(&extension.id) == container)
            {
                return true;
            }
            schema.resolve(path).is_some_and(|resolved| {
                resolved.container == container
                    && resolved.attribute.name == attribute.name
                    && match (resolved.sub_attribute, sub) {
                        (None, _) => true,
                        (Some(resolved), Some(sub)) => resolved.name == sub,
                        (Some(_), None) => false,
                    }
            })
        };
        let touches = |list: &[AttrPath]| {
            list.iter().any(|path| {
                names(path, None)
                    || schema.resolve(path).is_some_and(|r| {
                        r.container == container && r.attribute.name == attribute.name
                    })
            })
        };
        let keep = match (attribute.returned, &self.attributes) {
            (Returned::Never, _) | (Returned::Request, None) => false,
            (Returned::Always, _) => true,
            (Returned::Default | Returned::Request, Some(list)) => touches(list),
            (Returned::Default, None) => !self.excluded.iter().any(|path| names(path, None)),
        };
        if !keep {
            return None;
        }
        let Some(subs) = attribute.sub_attributes.as_deref() else {
            return Some(value.clone());
        };
        let keep_sub = |sub: &AttributeDefinition| match (sub.returned, &self.attributes) {
            (Returned::Never, _) | (Returned::Request, None) => false,
            (Returned::Always, _) => true,
            (_, Some(list)) => list.iter().any(|path| names(path, Some(&sub.name))),
            (_, None) => !self
                .excluded
                .iter()
                .any(|path| names(path, Some(&sub.name))),
        };
        let trim = |item: &Value| -> Option<Value> {
            let object: Map<String, Value> = item
                .as_object()?
                .iter()
                .filter(|(name, _)| {
                    subs.iter()
                        .find(|sub| sub.name == **name)
                        .is_some_and(keep_sub)
                })
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            (!object.is_empty()).then_some(Value::Object(object))
        };
        match value {
            Value::Array(items) => {
                let items: Vec<Value> = items.iter().filter_map(trim).collect();
                (!items.is_empty()).then_some(Value::Array(items))
            }
            other => trim(other),
        }
    }
}
