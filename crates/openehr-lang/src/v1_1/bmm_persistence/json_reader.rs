// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The P_BMM schema reader for the JSON serialisation: `.bmm.json` text → the
//! same `P_BMM_SCHEMA` value the ODIN reader produces.
//!
//! `LANG/docs/bmm_persistence/master04-syntax.adoc` §Serialisation Formats names
//! JSON "the primary format produced and consumed by the `bmm-publisher`
//! tooling" and states that "Whatever the format, the same serialisation rules
//! apply": the subtype discriminator is a `"_type"` member where ODIN writes a
//! type marker, and the keyed collections are keyed by name. This module maps
//! the JSON text onto the ODIN data model by exactly those rules and hands the
//! tree to the ODIN reader's walk
//! ([`crate::v1_1::bmm_persistence::reader::read_schema`]), so both
//! serialisations share one strict reading: the attribute checks, the two
//! tolerance lists, and the stamping of `P_BMM_CLASS.source_schema_id` ("Set
//! during `BMM_SCHEMA` materialise") and `P_BMM_CLASS.uid` ("Assigned in
//! post-load processing"), neither of which a persisted file carries
//! (`LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_class.adoc`
//! §Attributes).
//!
//! The mapping, value by value:
//!
//! * an object carrying `_type` → a type-marked value over its other members;
//! * an object that is the value of a keyed collection ([`KEYED_COLLECTIONS`])
//!   → a keyed list, in document order;
//! * any other object → an attribute object;
//! * an array of scalars → a primitive list; an array holding an object or an
//!   array → a keyed list under the integer keys `1..n`, the ODIN form for a
//!   container of non-primitive values (`LANG/docs/odin/master05-content.adoc`
//!   §Container Objects);
//! * a `cardinality` object → an integer interval, its absent inclusion flags read
//!   per the `Interval` invariants;
//! * a string, number or boolean → the ODIN leaf of the same kind; `null` is
//!   refused, and so is a member name repeated within one object, as ODIN
//!   refuses a repeated attribute.
//!
//! The generated canonical-JSON codec on the `P_BMM_*` types is a different
//! thing: it (de)serialises the whole object, `uid` and `source_schema_id`
//! included, and is not a reader of persisted schema files.

use std::collections::BTreeSet;
use std::fmt;

use indexmap::IndexMap;
use serde::de::{Error as _, MapAccess, SeqAccess, Visitor};

use crate::v1_1::bmm_persistence::error::PBmmReadError;
use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use crate::v1_1::bmm_persistence::reader::{join_path, read_schema_tree};
use crate::v1_1::odin::{OdinInterval, OdinKey, OdinValue};

/// The P_BMM attributes whose JSON value is a collection keyed by name.
///
/// `master04-syntax.adoc` §Serialisation Formats: "Maps such as
/// `_class_definitions_`, `_properties_`, `_functions_`, function
/// `_parameters_`, and `_packages_` are keyed by name". The full set is every
/// attribute the P_BMM class docs type `Hash<…>` (`includes`, `packages`,
/// `constants`, `properties`, `functions`, `invariants`,
/// `generic_parameter_defs`, `aliases`, `parameters`, `pre_conditions`,
/// `post_conditions`), plus the class lists and `ancestor_defs`, which the
/// syntax chapter writes keyed (§Classes for Primitive Types, §Inheritance).
pub const KEYED_COLLECTIONS: &[&str] = &[
    "includes",
    "packages",
    "primitive_types",
    "class_definitions",
    "constants",
    "properties",
    "functions",
    "invariants",
    "generic_parameter_defs",
    "ancestor_defs",
    "aliases",
    "parameters",
    "pre_conditions",
    "post_conditions",
];

/// The member that carries the subtype discriminator.
const TYPE_MEMBER: &str = "_type";

/// The attribute whose JSON object form is an integer interval.
const CARDINALITY: &str = "cardinality";

/// Reads a P_BMM schema from its JSON serialisation.
///
/// The returned value is the one [`crate::v1_1::bmm_persistence::reader::read_schema`]
/// returns for the ODIN serialisation of the same schema: unresolved includes,
/// `None` in-memory caches, and `P_BMM_CLASS.source_schema_id`/`uid` stamped by
/// the shared walk.
///
/// # Errors
/// Returns [`PBmmReadError::Json`] when the text is not well-formed JSON or an
/// object repeats a member name, [`PBmmReadError::WrongValueShape`] for a
/// `null` or an out-of-range number, [`PBmmReadError::UnsupportedCardinality`]
/// for a `cardinality` object that is not a consistent integer interval, and
/// every schema-shape variant `read_schema` returns when the mapped tree does
/// not match the P_BMM class model.
pub fn read_schema_json(src: &str) -> Result<PBmmSchema, PBmmReadError> {
    let root: JsonNode = serde_json::from_str(src).map_err(|error| json_syntax(&error))?;
    let tree = to_odin("", &root, false)?;
    read_schema_tree(&tree)
}

/// The typed form of a JSON syntax error.
fn json_syntax(error: &serde_json::Error) -> PBmmReadError {
    PBmmReadError::Json {
        line: error.line(),
        column: error.column(),
        message: error.to_string(),
    }
}

/// One JSON value, object members kept in document order.
enum JsonNode {
    /// `null`.
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// A number.
    Number(serde_json::Number),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<JsonNode>),
    /// An object, its member names unique.
    Object(Vec<(String, JsonNode)>),
}

impl<'de> serde::Deserialize<'de> for JsonNode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NodeVisitor)
    }
}

/// The visitor building a [`JsonNode`].
struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = JsonNode;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<JsonNode, E> {
        Ok(JsonNode::Null)
    }

    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<JsonNode, E> {
        Ok(JsonNode::Bool(value))
    }

    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<JsonNode, E> {
        Ok(JsonNode::Number(value.into()))
    }

    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<JsonNode, E> {
        Ok(JsonNode::Number(value.into()))
    }

    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<JsonNode, E> {
        serde_json::Number::from_f64(value)
            .map(JsonNode::Number)
            .ok_or_else(|| E::custom("a non-finite number"))
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<JsonNode, E> {
        Ok(JsonNode::String(value.to_owned()))
    }

    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<JsonNode, E> {
        Ok(JsonNode::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsonNode, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(JsonNode::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonNode, A::Error> {
        let mut seen = BTreeSet::new();
        let mut members = Vec::new();
        while let Some(name) = map.next_key::<String>()? {
            if !seen.insert(name.clone()) {
                return Err(A::Error::custom(format_args!("duplicate member `{name}`")));
            }
            let value = map.next_value()?;
            members.push((name, value));
        }
        Ok(JsonNode::Object(members))
    }
}

/// The JSON value kind, for error text.
fn kind_name(node: &JsonNode) -> &'static str {
    match node {
        JsonNode::Null => "null",
        JsonNode::Bool(_) => "a boolean",
        JsonNode::Number(_) => "a number",
        JsonNode::String(_) => "a string",
        JsonNode::Array(_) => "an array",
        JsonNode::Object(_) => "an object",
    }
}

/// Maps one JSON value at `path` onto the ODIN data model; `keyed` marks the
/// value of a keyed-collection attribute.
fn to_odin(path: &str, node: &JsonNode, keyed: bool) -> Result<OdinValue, PBmmReadError> {
    match node {
        JsonNode::Null => Err(PBmmReadError::WrongValueShape {
            path: path.to_owned(),
            expected: "a value",
            found: kind_name(node),
        }),
        JsonNode::Bool(flag) => Ok(OdinValue::Boolean(*flag)),
        JsonNode::Number(number) => number_to_odin(path, number),
        JsonNode::String(text) => Ok(OdinValue::String(text.clone())),
        JsonNode::Array(items) => array_to_odin(path, items),
        JsonNode::Object(members) if keyed => members
            .iter()
            .map(|(key, value)| {
                let entry = to_odin(&join_path(path, key), value, false)?;
                Ok((OdinKey::String(key.clone()), entry))
            })
            .collect::<Result<_, PBmmReadError>>()
            .map(OdinValue::KeyedList),
        JsonNode::Object(members) => object_to_odin(path, members),
    }
}

/// A JSON number as an ODIN integer or real leaf.
fn number_to_odin(path: &str, number: &serde_json::Number) -> Result<OdinValue, PBmmReadError> {
    if let Some(integer) = number.as_i64() {
        return Ok(OdinValue::Integer(integer));
    }
    match number.as_f64() {
        Some(real) if !number.is_u64() => Ok(OdinValue::Real(real)),
        _ => Err(PBmmReadError::WrongValueShape {
            path: path.to_owned(),
            expected: "an integer within the 64-bit signed range or a real",
            found: "an out-of-range integer",
        }),
    }
}

/// A JSON array as an ODIN primitive list, or as an integer-keyed list when it
/// holds a non-primitive value.
fn array_to_odin(path: &str, items: &[JsonNode]) -> Result<OdinValue, PBmmReadError> {
    if items
        .iter()
        .any(|item| matches!(item, JsonNode::Object(_) | JsonNode::Array(_)))
    {
        return (1_i64..)
            .zip(items)
            .map(|(key, item)| {
                let entry = to_odin(&join_path(path, &key.to_string()), item, false)?;
                Ok((OdinKey::Integer(key), entry))
            })
            .collect::<Result<_, PBmmReadError>>()
            .map(OdinValue::KeyedList);
    }
    items
        .iter()
        .map(|item| to_odin(path, item, false))
        .collect::<Result<_, PBmmReadError>>()
        .map(OdinValue::List)
}

/// A JSON object as an ODIN attribute object, type-marked when it carries the
/// `_type` discriminator.
fn object_to_odin(path: &str, members: &[(String, JsonNode)]) -> Result<OdinValue, PBmmReadError> {
    let mut attributes = IndexMap::new();
    let mut marker = None;
    for (name, value) in members {
        if name == TYPE_MEMBER {
            let JsonNode::String(type_name) = value else {
                return Err(PBmmReadError::WrongValueShape {
                    path: join_path(path, name),
                    expected: "a type name",
                    found: kind_name(value),
                });
            };
            marker = Some(type_name.clone());
            continue;
        }
        let member_path = join_path(path, name);
        let mapped = match value {
            JsonNode::Object(limits) if name == CARDINALITY => cardinality(&member_path, limits)?,
            other => to_odin(
                &member_path,
                other,
                KEYED_COLLECTIONS.contains(&name.as_str()),
            )?,
        };
        attributes.insert(name.clone(), mapped);
    }
    let object = OdinValue::Object(attributes);
    Ok(match marker {
        Some(rm_type) => OdinValue::Typed {
            rm_type,
            value: Box::new(object),
        },
        None => object,
    })
}

/// A JSON `cardinality` object as an ODIN integer interval.
///
/// The members are the `Interval` attributes `lower`, `upper`,
/// `lower_unbounded`, `upper_unbounded`, `lower_included`, `upper_included`
/// (`BASE/docs/UML/classes/org.openehr.base.foundation_types.interval.adoc`
/// §Attributes); the published schemas write only the ones they need
/// (`{"lower": 1, "upper_unbounded": true}`). An absent `*_unbounded` reads as
/// "no limit written", and an absent `*_included` as the opposite of
/// `*_unbounded`, which is what the invariants `Lower_included_valid` and
/// `Upper_included_valid` leave for an unbounded side and what the ODIN form
/// `|>=1|` denotes for a bounded one. NOTE: no openEHR spec defines the JSON
/// interval members' defaults; this reading is our own design, chosen to match the ODIN range.
fn cardinality(path: &str, members: &[(String, JsonNode)]) -> Result<OdinValue, PBmmReadError> {
    let mut lower = None;
    let mut upper = None;
    let mut lower_unbounded = None;
    let mut upper_unbounded = None;
    let mut lower_included = None;
    let mut upper_included = None;
    for (name, value) in members {
        match name.as_str() {
            "lower" => lower = Some(interval_limit(path, value)?),
            "upper" => upper = Some(interval_limit(path, value)?),
            "lower_unbounded" => lower_unbounded = Some(interval_flag(path, value)?),
            "upper_unbounded" => upper_unbounded = Some(interval_flag(path, value)?),
            "lower_included" => lower_included = Some(interval_flag(path, value)?),
            "upper_included" => upper_included = Some(interval_flag(path, value)?),
            _ => {
                return Err(PBmmReadError::UnknownAttribute {
                    path: path.to_owned(),
                    attribute: name.clone(),
                });
            }
        }
    }
    let lower_unbounded = lower_unbounded.unwrap_or(lower.is_none());
    let upper_unbounded = upper_unbounded.unwrap_or(upper.is_none());
    let lower_included = lower_included.unwrap_or(!lower_unbounded);
    let upper_included = upper_included.unwrap_or(!upper_unbounded);
    let consistent = lower_unbounded != lower.is_some()
        && upper_unbounded != upper.is_some()
        && !(lower_unbounded && lower_included)
        && !(upper_unbounded && upper_included);
    if !consistent {
        return Err(PBmmReadError::UnsupportedCardinality {
            path: path.to_owned(),
        });
    }
    Ok(OdinValue::Interval(OdinInterval::Range {
        lower: lower.map(|limit| Box::new(OdinValue::Integer(limit))),
        lower_included,
        upper: upper.map(|limit| Box::new(OdinValue::Integer(limit))),
        upper_included,
    }))
}

/// One interval limit, an integer.
fn interval_limit(path: &str, value: &JsonNode) -> Result<i64, PBmmReadError> {
    match value {
        JsonNode::Number(number) => {
            number
                .as_i64()
                .ok_or_else(|| PBmmReadError::UnsupportedCardinality {
                    path: path.to_owned(),
                })
        }
        _ => Err(PBmmReadError::UnsupportedCardinality {
            path: path.to_owned(),
        }),
    }
}

/// One interval flag, a boolean.
fn interval_flag(path: &str, value: &JsonNode) -> Result<bool, PBmmReadError> {
    match value {
        JsonNode::Bool(flag) => Ok(*flag),
        _ => Err(PBmmReadError::UnsupportedCardinality {
            path: path.to_owned(),
        }),
    }
}
