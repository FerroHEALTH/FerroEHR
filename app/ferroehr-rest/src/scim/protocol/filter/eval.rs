// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A filter over a resource (RFC 7644 §3.4.2.2).
//!
//! A comparison reads the attribute's data type: strings by their
//! `caseExact` characteristic, numbers by value, `dateTime` chronologically,
//! booleans for equality only (Table 3). A multi-valued attribute matches
//! when any value does. An attribute the resource type does not define "SHALL
//! [be treated] as if there is no attribute value" (§3.4.2.1), and so does an
//! unassigned one (RFC 7643 §2.5), which is what `eq null` tests.

use std::cmp::Ordering;

use serde_json::Value;

use super::{AttrExp, CompValue, CompareOp, Logic, Term, Test, ValueFilter};
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::path::{AttrName, AttrPath};
use crate::scim::protocol::resource::value::instant;
use crate::scim::protocol::resource::{Resolved, Resource, ResourceSchema};
use crate::scim::protocol::schema::{AttributeDefinition, AttributeType};

impl Logic<Term> {
    /// Returns whether `resource` matches this filter.
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Logic::validate`].
    pub fn matches(&self, resource: &Resource, schema: &ResourceSchema) -> Result<bool, ScimError> {
        self.validate(schema)?;
        Ok(holds(self, &|term| term_matches(term, resource, schema)))
    }

    /// Refuses a comparison the attribute's data type does not support.
    ///
    /// # Errors
    ///
    /// Returns a `400` with `scimType` `invalidFilter`: "the specified
    /// attribute and filter comparison combination is not supported" (RFC
    /// 7644 Table 9), including ordering a boolean or binary attribute (Table
    /// 3), comparing a value of another type, comparing a complex attribute
    /// without naming a sub-attribute, and a value path over an attribute that
    /// is not complex.
    pub fn validate(&self, schema: &ResourceSchema) -> Result<(), ScimError> {
        each_leaf(self, &mut |term| match term {
            Term::Attr(expression) => {
                validate_attribute(&expression.path, &expression.test, schema)
            }
            Term::ValuePath(path, filter) => validate_value_path(path, filter, schema),
        })
    }
}

/// Returns whether `element`, one value of the multi-valued complex
/// `attribute`, matches `filter`.
pub(crate) fn value_matches(
    filter: &ValueFilter,
    attribute: &AttributeDefinition,
    element: &Value,
) -> bool {
    holds(filter, &|expression| {
        element_matches(element, attribute, expression)
    })
}

fn holds<T>(logic: &Logic<T>, leaf: &dyn Fn(&T) -> bool) -> bool {
    match logic {
        Logic::Leaf(term) => leaf(term),
        Logic::And(terms) => terms.iter().all(|term| holds(term, leaf)),
        Logic::Or(terms) => terms.iter().any(|term| holds(term, leaf)),
        Logic::Not(inner) => !holds(inner, leaf),
    }
}

pub(super) fn each_leaf<T>(
    logic: &Logic<T>,
    check: &mut dyn FnMut(&T) -> Result<(), ScimError>,
) -> Result<(), ScimError> {
    match logic {
        Logic::Leaf(term) => check(term),
        Logic::And(terms) | Logic::Or(terms) => {
            terms.iter().try_for_each(|term| each_leaf(term, check))
        }
        Logic::Not(inner) => each_leaf(inner, check),
    }
}

/// What a filter's attribute path names.
enum Target<'s> {
    /// The `schemas` attribute every resource carries (RFC 7644 §3.4.2.2, Figure 2).
    Schemas,
    /// An attribute of the resource type.
    Attribute(Resolved<'s>),
    /// Nothing the resource type defines.
    Undefined,
}

fn target<'s>(path: &AttrPath, schema: &'s ResourceSchema) -> Target<'s> {
    if path.schema.is_none() && path.sub_attr.is_none() && path.name.matches("schemas") {
        return Target::Schemas;
    }
    schema
        .resolve(path)
        .map_or(Target::Undefined, Target::Attribute)
}

/// Returns the definition a comparison reads and the sub-attribute it reads it
/// through: the sub-attribute the path names, else the `value` of a
/// multi-valued complex attribute, which RFC 7644 Figure 2 compares directly
/// (`emails co "example.com"`), else the attribute itself.
fn compared<'s>(resolved: &Resolved<'s>) -> (&'s AttributeDefinition, Option<&'s str>) {
    let value = || {
        resolved
            .attribute
            .sub_attribute("value")
            .filter(|_value| resolved.attribute.multi_valued)
    };
    match resolved.sub_attribute.or_else(value) {
        Some(sub) => (sub, Some(sub.name.as_str())),
        None => (resolved.attribute, None),
    }
}

fn validate_attribute(
    path: &AttrPath,
    test: &Test,
    schema: &ResourceSchema,
) -> Result<(), ScimError> {
    match target(path, schema) {
        Target::Undefined => Ok(()),
        Target::Schemas => check_test("schemas", AttributeType::String, test),
        Target::Attribute(resolved) => {
            check_test(&path.to_string(), compared(&resolved).0.kind, test)
        }
    }
}

fn validate_value_path(
    path: &AttrPath,
    filter: &ValueFilter,
    schema: &ResourceSchema,
) -> Result<(), ScimError> {
    let resolved = match target(path, schema) {
        Target::Undefined => return Ok(()),
        Target::Attribute(resolved)
            if resolved.sub_attribute.is_none() && resolved.attribute.is_complex() =>
        {
            resolved
        }
        Target::Schemas | Target::Attribute(_) => {
            return Err(ScimError::bad_request(
                ScimType::InvalidFilter,
                format!("The attribute '{path}' is not complex and takes no value filter."),
            ));
        }
    };
    each_leaf(filter, &mut |expression: &AttrExp<AttrName>| match resolved
        .attribute
        .sub_attribute(expression.path.as_str())
    {
        Some(sub) => check_test(&format!("{path}.{}", sub.name), sub.kind, &expression.test),
        None => Ok(()),
    })
}

/// Refuses a test the data type `kind` does not support (RFC 7644 Table 3).
fn check_test(label: &str, kind: AttributeType, test: &Test) -> Result<(), ScimError> {
    let Test::Compare(op, value) = test else {
        return Ok(());
    };
    let ordering = matches!(
        op,
        CompareOp::Gt | CompareOp::Ge | CompareOp::Lt | CompareOp::Le
    );
    let substring = matches!(op, CompareOp::Co | CompareOp::Sw | CompareOp::Ew);
    let supported = match (kind, value) {
        (_, CompValue::Null) => matches!(op, CompareOp::Eq | CompareOp::Ne),
        (AttributeType::String | AttributeType::Reference, CompValue::String(_)) => true,
        (AttributeType::Binary, CompValue::String(_)) => !ordering,
        (AttributeType::Boolean, CompValue::True | CompValue::False) => !ordering && !substring,
        (AttributeType::Integer | AttributeType::Decimal, CompValue::Number(_)) => !substring,
        (AttributeType::DateTime, CompValue::String(stamp)) => {
            !substring && instant(stamp).is_some()
        }
        _ => false,
    };
    if supported {
        Ok(())
    } else {
        Err(ScimError::bad_request(
            ScimType::InvalidFilter,
            format!(
                "The attribute '{label}' does not support the comparison '{} {value}'.",
                op.as_str()
            ),
        ))
    }
}

fn term_matches(term: &Term, resource: &Resource, schema: &ResourceSchema) -> bool {
    match term {
        Term::Attr(expression) => match target(&expression.path, schema) {
            Target::Undefined => test_values(&[], AttributeType::String, false, &expression.test),
            Target::Schemas => {
                let schemas: Vec<&Value> = resource
                    .attributes()
                    .get("schemas")
                    .and_then(Value::as_array)
                    .map(|s| s.iter().collect())
                    .unwrap_or_default();
                test_values(&schemas, AttributeType::String, false, &expression.test)
            }
            Target::Attribute(resolved) => {
                let (definition, through) = compared(&resolved);
                let values = attribute_values(resource, &resolved, through);
                test_values(
                    &values,
                    definition.kind,
                    definition.is_case_exact(),
                    &expression.test,
                )
            }
        },
        Term::ValuePath(path, filter) => match target(path, schema) {
            Target::Attribute(resolved) => {
                elements(resource.get(resolved.container, &resolved.attribute.name))
                    .into_iter()
                    .any(|element| value_matches(filter, resolved.attribute, element))
            }
            Target::Schemas | Target::Undefined => false,
        },
    }
}

/// Returns the values `resolved` names in `resource`, read through the
/// sub-attribute `through` of each value when there is one.
fn attribute_values<'r>(
    resource: &'r Resource,
    resolved: &Resolved<'_>,
    through: Option<&str>,
) -> Vec<&'r Value> {
    let values = elements(resource.get(resolved.container, &resolved.attribute.name));
    let Some(through) = through else {
        return values;
    };
    values
        .into_iter()
        .filter_map(|value| value.get(through))
        .flat_map(|value| elements(Some(value)))
        .collect()
}

/// Returns the values of an attribute's value: the items of an array, or the value itself.
fn elements(value: Option<&Value>) -> Vec<&Value> {
    match value {
        None => Vec::new(),
        Some(Value::Array(items)) => items.iter().collect(),
        Some(other) => vec![other],
    }
}

fn element_matches(
    element: &Value,
    attribute: &AttributeDefinition,
    expression: &AttrExp<AttrName>,
) -> bool {
    match attribute.sub_attribute(expression.path.as_str()) {
        None => test_values(&[], AttributeType::String, false, &expression.test),
        Some(sub) => {
            let values = elements(element.get(&sub.name));
            test_values(&values, sub.kind, sub.is_case_exact(), &expression.test)
        }
    }
}

fn test_values(values: &[&Value], kind: AttributeType, case_exact: bool, test: &Test) -> bool {
    match test {
        // NOTE: RFC 7644 Table 3, `pr`: "a non-empty or non-null value, or ... a non-empty node".
        Test::Present => values.iter().any(|value| is_present(value)),
        Test::Compare(CompareOp::Eq, CompValue::Null) => {
            !values.iter().any(|value| is_present(value))
        }
        Test::Compare(CompareOp::Ne, CompValue::Null) => {
            values.iter().any(|value| is_present(value))
        }
        Test::Compare(op, expected) => values
            .iter()
            .any(|value| compare(value, kind, case_exact, *op, expected)),
    }
}

fn is_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(members) => !members.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn compare(
    actual: &Value,
    kind: AttributeType,
    case_exact: bool,
    op: CompareOp,
    expected: &CompValue,
) -> bool {
    match (kind, expected, actual) {
        (
            AttributeType::String | AttributeType::Reference,
            CompValue::String(expected),
            Value::String(actual),
        ) => strings(actual, expected, case_exact, op),
        // NOTE: RFC 7643 §2.3.6: "A binary is case exact".
        (AttributeType::Binary, CompValue::String(expected), Value::String(actual)) => {
            strings(actual, expected, true, op)
        }
        (AttributeType::Boolean, CompValue::True | CompValue::False, Value::Bool(actual)) => {
            let equal = *actual == matches!(expected, CompValue::True);
            match op {
                CompareOp::Eq => equal,
                CompareOp::Ne => !equal,
                _ => false,
            }
        }
        (
            AttributeType::Integer | AttributeType::Decimal,
            CompValue::Number(expected),
            Value::Number(actual),
        ) => {
            let order = match (actual.as_i64(), expected.as_i64()) {
                (Some(actual), Some(expected)) => Some(actual.cmp(&expected)),
                _ => actual
                    .as_f64()
                    .zip(expected.as_f64())
                    .and_then(|(actual, expected)| actual.partial_cmp(&expected)),
            };
            ordered(op, order)
        }
        (AttributeType::DateTime, CompValue::String(expected), Value::String(actual)) => ordered(
            op,
            instant(actual)
                .zip(instant(expected))
                .map(|(actual, expected)| actual.cmp(&expected)),
        ),
        _ => false,
    }
}

fn strings(actual: &str, expected: &str, case_exact: bool, op: CompareOp) -> bool {
    let (actual, expected) = if case_exact {
        (actual.to_owned(), expected.to_owned())
    } else {
        (actual.to_lowercase(), expected.to_lowercase())
    };
    match op {
        CompareOp::Eq => actual == expected,
        CompareOp::Ne => actual != expected,
        CompareOp::Co => actual.contains(&expected),
        CompareOp::Sw => actual.starts_with(&expected),
        CompareOp::Ew => actual.ends_with(&expected),
        // NOTE: RFC 7644 Table 3: "For string attribute types, this is a lexicographical comparison".
        CompareOp::Gt => actual > expected,
        CompareOp::Ge => actual >= expected,
        CompareOp::Lt => actual < expected,
        CompareOp::Le => actual <= expected,
    }
}

fn ordered(op: CompareOp, order: Option<Ordering>) -> bool {
    let Some(order) = order else {
        return false;
    };
    match op {
        CompareOp::Eq => order == Ordering::Equal,
        CompareOp::Ne => order != Ordering::Equal,
        CompareOp::Gt => order == Ordering::Greater,
        CompareOp::Ge => order != Ordering::Less,
        CompareOp::Lt => order == Ordering::Less,
        CompareOp::Le => order != Ordering::Greater,
        CompareOp::Co | CompareOp::Sw | CompareOp::Ew => false,
    }
}
