// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The PATCH operations applied to a resource (RFC 7644 §3.5.2.1 to §3.5.2.3).
//!
//! Every operation works on the canonical form of [`Resource`]: a value is
//! checked against its attribute definition before it is written, a
//! multi-valued value is identified by its `value` and `type` sub-attributes
//! (RFC 7643 §2.4), and a value written with `primary` true clears `primary`
//! on the others (RFC 7644 §3.5.2: the server SHALL "automatically set
//! "primary" to "false" for any other values").

use serde_json::{Map, Value};

use super::{PatchOp, PatchOperation, PatchOutcome, PatchRequest};
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::filter::eval::value_matches;
use crate::scim::protocol::filter::{AttrExp, CompValue, CompareOp, Logic, Test, ValueFilter};
use crate::scim::protocol::path::{AttrPath, PatchPath};
use crate::scim::protocol::resource::value::{self, Origin};
use crate::scim::protocol::resource::{Container, Resolved, Resource, ResourceSchema, body};
use crate::scim::protocol::schema::{AttributeDefinition, Mutability, Schema};

impl PatchRequest {
    /// Applies the operations in order to a copy of `resource`.
    ///
    /// # Errors
    ///
    /// Returns, and leaves `resource` as it was (§3.5.2: "the original SCIM
    /// resource MUST be restored"), a `400` with `invalidPath` for a path the
    /// resource type does not define; `noTarget` for a `replace` whose value
    /// filter selects nothing (§3.5.2.3) and an `add` whose filter selects
    /// nothing and is not an equality to build the value from; `mutability`
    /// for a change of a `readOnly` or `immutable` value, or a removal of a
    /// required one (§3.5.2.2); `invalidValue` for a value the attribute does
    /// not admit.
    pub fn apply(
        &self,
        resource: &Resource,
        schema: &ResourceSchema,
    ) -> Result<PatchOutcome, ScimError> {
        let mut attributes = resource.attributes().clone();
        for operation in &self.operations {
            apply_operation(&mut attributes, operation, schema)?;
        }
        body::settle_schemas(&mut attributes, schema);
        body::require(&attributes, schema)?;
        let changed = &attributes != resource.attributes();
        Ok(PatchOutcome {
            resource: Resource::from_canonical(attributes),
            changed,
        })
    }
}

type Attributes = Map<String, Value>;

fn apply_operation(
    attributes: &mut Attributes,
    operation: &PatchOperation,
    schema: &ResourceSchema,
) -> Result<(), ScimError> {
    let value = operation.value.as_ref();
    match &operation.path {
        None => {
            let Some(Value::Object(members)) = value else {
                return Err(ScimError::bad_request(
                    ScimType::InvalidValue,
                    "An operation without a 'path' takes an object of attributes (RFC 7644 §3.5.2.1).",
                ));
            };
            members.iter().try_for_each(|(key, member)| {
                apply_member(attributes, operation.op, schema, key, member)
            })
        }
        Some(PatchPath::Attribute(path)) => {
            if let Some(extension) = schema.extension_named(path) {
                return apply_extension(attributes, operation.op, schema, extension, value);
            }
            let resolved = resolve(schema, path)?;
            match resolved.sub_attribute {
                None => attribute_operation(attributes, operation.op, resolved, value),
                Some(sub) => {
                    sub_attribute_operation(attributes, operation.op, resolved, sub, value)
                }
            }
        }
        Some(PatchPath::Values {
            attr,
            filter,
            sub_attr,
        }) => {
            let resolved = resolve(schema, attr)?;
            let attribute = resolved.attribute;
            if resolved.sub_attribute.is_some()
                || !attribute.multi_valued
                || !attribute.is_complex()
            {
                return Err(ScimError::bad_request(
                    ScimType::InvalidPath,
                    format!("'{attr}' is not a multi-valued complex attribute."),
                ));
            }
            let sub = match sub_attr {
                Some(name) => Some(attribute.sub_attribute(name.as_str()).ok_or_else(|| {
                    ScimError::bad_request(
                        ScimType::InvalidPath,
                        format!("'{attr}' has no sub-attribute '{name}'."),
                    )
                })?),
                None => None,
            };
            values_operation(attributes, operation.op, resolved, filter, sub, value)
        }
    }
}

/// Applies one member of a path-less `add` or `replace` value: an attribute,
/// its fully qualified name, or an extension container.
fn apply_member(
    attributes: &mut Attributes,
    op: PatchOp,
    schema: &ResourceSchema,
    key: &str,
    member: &Value,
) -> Result<(), ScimError> {
    if let Some(extension) = schema.extension(key) {
        return apply_extension(attributes, op, schema, extension, Some(member));
    }
    // NOTE: a key that is no attribute path is legitimately undefined, refused below unless unassigned.
    let resolved = AttrPath::parse(key)
        .ok()
        .and_then(|path| schema.resolve(&path));
    match resolved {
        Some(
            resolved @ Resolved {
                sub_attribute: None,
                ..
            },
        ) => attribute_operation(attributes, op, resolved, Some(member)),
        Some(
            resolved @ Resolved {
                sub_attribute: Some(sub),
                ..
            },
        ) => sub_attribute_operation(attributes, op, resolved, sub, Some(member)),
        // NOTE: RFC 7643 §2.5: an unassigned value asserts the state an undefined attribute already has.
        None if value::is_unassigned(member) => Ok(()),
        None => Err(ScimError::bad_request(
            ScimType::InvalidValue,
            format!(
                "Attribute '{key}' is not defined for {}.",
                schema.resource_type()
            ),
        )),
    }
}

/// Applies an operation whose target is a whole extension container.
fn apply_extension(
    attributes: &mut Attributes,
    op: PatchOp,
    schema: &ResourceSchema,
    extension: &Schema,
    value: Option<&Value>,
) -> Result<(), ScimError> {
    if op == PatchOp::Remove {
        let required = schema
            .extensions()
            .iter()
            .any(|(s, required)| *required && s.id == extension.id);
        if required {
            return Err(ScimError::bad_request(
                ScimType::Mutability,
                format!("The extension {} is required.", extension.id),
            ));
        }
        attributes.remove(&extension.id);
        return Ok(());
    }
    let Some(Value::Object(members)) = value else {
        return Err(ScimError::bad_request(
            ScimType::InvalidValue,
            format!(
                "The extension {} takes an object of its attributes.",
                extension.id
            ),
        ));
    };
    for (name, member) in members {
        let attribute = extension.attribute(name).ok_or_else(|| {
            ScimError::bad_request(
                ScimType::InvalidValue,
                format!("Attribute '{name}' is not defined for {}.", extension.id),
            )
        })?;
        let resolved = Resolved {
            container: Container::Extension(&extension.id),
            attribute,
            sub_attribute: None,
        };
        attribute_operation(attributes, op, resolved, Some(member))?;
    }
    Ok(())
}

fn attribute_operation(
    attributes: &mut Attributes,
    op: PatchOp,
    resolved: Resolved<'_>,
    value: Option<&Value>,
) -> Result<(), ScimError> {
    let attribute = resolved.attribute;
    writable(attribute)?;
    let current = body::get(attributes, resolved.container, &attribute.name).cloned();
    if op == PatchOp::Remove {
        let Some(current) = current else {
            return Ok(());
        };
        removable(attribute)?;
        let next = match value {
            // NOTE: Deviation::RemoveValueList: a remove with a value list removes only the listed values.
            Some(listed) if attribute.multi_valued => without_listed(current, listed, attribute),
            Some(_) => {
                return Err(ScimError::bad_request(
                    ScimType::InvalidValue,
                    format!("A remove of '{}' carries no value.", attribute.name),
                ));
            }
            None => None,
        };
        body::set(attributes, resolved.container, &attribute.name, next);
        return Ok(());
    }
    let incoming = match value {
        // NOTE: RFC 7644 §3.5.2.1: the value "MAY be a quoted value, or ... a JSON object", so one value stands for a list of one.
        Some(single) if attribute.multi_valued && !single.is_array() && !single.is_null() => {
            value::normalize(
                attribute,
                &Value::Array(vec![single.clone()]),
                Origin::Request,
                &attribute.name,
            )?
        }
        Some(value) => value::normalize(attribute, value, Origin::Request, &attribute.name)?,
        None => None,
    };
    let Some(incoming) = incoming else {
        // NOTE: RFC 7643 §2.5: assigning null or an empty array makes the attribute unassigned.
        if op == PatchOp::Replace && current.is_some() {
            removable(attribute)?;
            body::set(attributes, resolved.container, &attribute.name, None);
        }
        return Ok(());
    };
    let next = match (op, attribute.multi_valued) {
        (PatchOp::Add, true) => with_added(current.clone(), incoming, attribute),
        (_, false) if attribute.is_complex() => merged(current.as_ref(), incoming, attribute)?,
        _ => incoming,
    };
    if attribute.mutability == Mutability::Immutable
        && current.as_ref().is_some_and(|current| *current != next)
    {
        return Err(ScimError::bad_request(
            ScimType::Mutability,
            format!("Attribute '{}' is immutable.", attribute.name),
        ));
    }
    body::set(attributes, resolved.container, &attribute.name, Some(next));
    Ok(())
}

/// Applies an operation on one sub-attribute of a singular complex attribute.
fn sub_attribute_operation(
    attributes: &mut Attributes,
    op: PatchOp,
    resolved: Resolved<'_>,
    sub: &AttributeDefinition,
    value: Option<&Value>,
) -> Result<(), ScimError> {
    let attribute = resolved.attribute;
    if attribute.multi_valued {
        return Err(ScimError::bad_request(
            ScimType::InvalidPath,
            format!(
                "'{}.{}' names a sub-attribute of a multi-valued attribute; select its values with a filter.",
                attribute.name, sub.name
            ),
        ));
    }
    writable(attribute)?;
    writable(sub)?;
    let mut object = match body::get(attributes, resolved.container, &attribute.name) {
        Some(Value::Object(object)) => object.clone(),
        _ => Map::new(),
    };
    let incoming = match (op, value) {
        (PatchOp::Remove, _) | (_, None) => None,
        (_, Some(value)) => value::normalize(
            sub,
            value,
            Origin::Request,
            &format!("{}.{}", attribute.name, sub.name),
        )?,
    };
    write_sub(&mut object, sub, incoming, op)?;
    store(
        attributes,
        resolved,
        (!object.is_empty()).then_some(Value::Object(object)),
    )
}

/// Applies an operation on the values of a multi-valued complex attribute a
/// value filter selects, or on one sub-attribute of each.
fn values_operation(
    attributes: &mut Attributes,
    op: PatchOp,
    resolved: Resolved<'_>,
    filter: &ValueFilter,
    sub: Option<&AttributeDefinition>,
    value: Option<&Value>,
) -> Result<(), ScimError> {
    let attribute = resolved.attribute;
    writable(attribute)?;
    if let Some(sub) = sub {
        writable(sub)?;
    }
    let mut items = match body::get(attributes, resolved.container, &attribute.name) {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    let selected: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| value_matches(filter, attribute, item))
        .map(|(index, _)| index)
        .collect();
    let no_target = || {
        ScimError::bad_request(
            ScimType::NoTarget,
            format!("The filter on '{}' selects no value.", attribute.name),
        )
    };
    let element = |value: Option<&Value>| match (sub, value) {
        (_, None) => Ok(None),
        (Some(sub), Some(value)) => value::normalize(
            sub,
            value,
            Origin::Request,
            &format!("{}.{}", attribute.name, sub.name),
        ),
        (None, Some(value)) => value::single(attribute, value, Origin::Request, &attribute.name),
    };
    match op {
        PatchOp::Remove => {
            // NOTE: RFC 7644 §3.5.2.2: a filter that selects nothing leaves the resource as it was.
            for &index in selected.iter().rev() {
                match (sub, items.get_mut(index).and_then(Value::as_object_mut)) {
                    (Some(sub), Some(object)) => write_sub(object, sub, None, op)?,
                    _ => {
                        items.remove(index);
                    }
                }
            }
            items.retain(|item| item.as_object().is_none_or(|object| !object.is_empty()));
        }
        PatchOp::Replace | PatchOp::Add if selected.is_empty() => {
            if op == PatchOp::Replace {
                return Err(no_target());
            }
            let mut record = equality_record(filter, attribute).ok_or_else(no_target)?;
            let written = element(value)?;
            match (sub, written) {
                (Some(sub), Some(value)) => {
                    record.insert(sub.name.clone(), value);
                }
                (None, Some(Value::Object(members))) => record.extend(members),
                _ => {}
            }
            items.push(Value::Object(record));
            let last = items.len().saturating_sub(1);
            settle_primary(&mut items, &[last]);
        }
        PatchOp::Replace | PatchOp::Add => {
            let written = element(value)?;
            for &index in &selected {
                let Some(object) = items.get_mut(index).and_then(Value::as_object_mut) else {
                    continue;
                };
                match (sub, &written) {
                    (Some(sub), written) => write_sub(object, sub, written.clone(), op)?,
                    (None, Some(Value::Object(members))) if op == PatchOp::Replace => {
                        object.clone_from(members);
                    }
                    (None, Some(Value::Object(members))) => object.extend(members.clone()),
                    (None, _) => {}
                }
            }
            settle_primary(&mut items, &selected);
        }
    }
    store(
        attributes,
        resolved,
        (!items.is_empty()).then_some(Value::Array(items)),
    )
}

/// Writes the new value of `resolved`'s attribute, refusing to leave a
/// required or `immutable` attribute unassigned (§3.5.2.2).
fn store(
    attributes: &mut Attributes,
    resolved: Resolved<'_>,
    value: Option<Value>,
) -> Result<(), ScimError> {
    let name = &resolved.attribute.name;
    if value.is_none() && body::get(attributes, resolved.container, name).is_some() {
        removable(resolved.attribute)?;
    }
    body::set(attributes, resolved.container, name, value);
    Ok(())
}

/// Writes, or with `None` removes, the sub-attribute `sub` of `object`.
fn write_sub(
    object: &mut Map<String, Value>,
    sub: &AttributeDefinition,
    value: Option<Value>,
    op: PatchOp,
) -> Result<(), ScimError> {
    let current = object.get(&sub.name);
    let changes = match &value {
        Some(value) => current != Some(value),
        None => current.is_some(),
    };
    if changes && sub.mutability == Mutability::Immutable && current.is_some() {
        return Err(ScimError::bad_request(
            ScimType::Mutability,
            format!("Sub-attribute '{}' is immutable.", sub.name),
        ));
    }
    match value {
        Some(value) => {
            object.insert(sub.name.clone(), value);
        }
        None if op == PatchOp::Remove || current.is_some() => {
            if current.is_some() {
                removable(sub)?;
            }
            object.remove(&sub.name);
        }
        None => {}
    }
    Ok(())
}

/// Returns the sub-attributes of `current` with those of `incoming` set: a
/// complex value "replaces any existing values or adds where an attribute did
/// not previously exist", leaving the others unchanged (§3.5.2.3).
fn merged(
    current: Option<&Value>,
    incoming: Value,
    attribute: &AttributeDefinition,
) -> Result<Value, ScimError> {
    let mut object = match current {
        Some(Value::Object(object)) => object.clone(),
        _ => Map::new(),
    };
    let Value::Object(members) = incoming else {
        return Ok(incoming);
    };
    for (name, value) in members {
        let sub = attribute.sub_attribute(&name).ok_or_else(|| {
            ScimError::bad_request(
                ScimType::InvalidValue,
                format!("Sub-attribute '{name}' is not defined."),
            )
        })?;
        write_sub(&mut object, sub, Some(value), PatchOp::Replace)?;
    }
    Ok(Value::Object(object))
}

/// Returns the values of `current` followed by each value of `incoming` it
/// does not already hold (§3.5.2.1: "no changes SHOULD be made" for a value
/// already present).
fn with_added(current: Option<Value>, incoming: Value, attribute: &AttributeDefinition) -> Value {
    let mut items = match current {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    let Value::Array(added) = incoming else {
        return Value::Array(items);
    };
    for value in added {
        if !items.iter().any(|held| same_value(held, &value, attribute)) {
            items.push(value);
            let last = items.len().saturating_sub(1);
            settle_primary(&mut items, &[last]);
        }
    }
    Value::Array(items)
}

/// Returns `current` without the values `listed` names, or `None` when none
/// remain.
fn without_listed(
    current: Value,
    listed: &Value,
    attribute: &AttributeDefinition,
) -> Option<Value> {
    let listed: Vec<&Value> = match listed {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    let Value::Array(mut items) = current else {
        return Some(current);
    };
    items.retain(|held| {
        !listed
            .iter()
            .any(|value| same_value(held, value, attribute))
    });
    (!items.is_empty()).then_some(Value::Array(items))
}

/// Returns whether two values of a multi-valued attribute are the same value:
/// equal `value` and `type` sub-attributes for a complex attribute with a
/// `value` (RFC 7643 §2.4, "the same (type, value) combination"), otherwise
/// equal values, each compared by its `caseExact` characteristic.
fn same_value(held: &Value, other: &Value, attribute: &AttributeDefinition) -> bool {
    let Some(value_sub) = attribute.sub_attribute("value") else {
        return equal(held, other, attribute.is_case_exact());
    };
    let sub = |item: &Value, name: &str| {
        item.as_object().and_then(|object| {
            object
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        })
    };
    let same_type = match (sub(held, "type"), sub(other, "type")) {
        (Some(left), Some(right)) => equal(&left, &right, false),
        (None, None) => true,
        // NOTE: a listed value without a `type` names the value whatever its type, as a member removal sends it.
        (Some(_), None) | (None, Some(_)) => sub(other, "type").is_none(),
    };
    match (sub(held, "value"), sub(other, "value")) {
        (Some(left), Some(right)) => same_type && equal(&left, &right, value_sub.is_case_exact()),
        _ => false,
    }
}

fn equal(left: &Value, right: &Value, case_exact: bool) -> bool {
    match (left, right) {
        (Value::String(left), Value::String(right)) if !case_exact => {
            left.to_lowercase() == right.to_lowercase()
        }
        _ => left == right,
    }
}

/// Clears `primary` on every value but those at `written` that carry it.
fn settle_primary(items: &mut [Value], written: &[usize]) {
    if !written
        .iter()
        .any(|&index| items.get(index).is_some_and(value::is_primary))
    {
        return;
    }
    for (index, item) in items.iter_mut().enumerate() {
        if !written.contains(&index)
            && value::is_primary(item)
            && let Some(object) = item.as_object_mut()
        {
            object.insert("primary".to_owned(), Value::Bool(false));
        }
    }
}

/// Returns the value an `add` builds when its filter selects nothing: the
/// filter must be equalities joined by `and`, and their values are the new
/// value's sub-attributes. RFC 7644 §3.5.2.1 says "If the target location
/// does not exist, the attribute and value are added"; reading the filter's
/// equalities as that location is our own design.
fn equality_record(
    filter: &ValueFilter,
    attribute: &AttributeDefinition,
) -> Option<Map<String, Value>> {
    let leaves: Vec<&AttrExp<_>> = match filter {
        Logic::Leaf(leaf) => vec![leaf],
        Logic::And(terms) => terms
            .iter()
            .map(|term| {
                if let Logic::Leaf(leaf) = term {
                    Some(leaf)
                } else {
                    None
                }
            })
            .collect::<Option<_>>()?,
        Logic::Or(_) | Logic::Not(_) => return None,
    };
    let mut record = Map::new();
    for leaf in leaves {
        let Test::Compare(CompareOp::Eq, value) = &leaf.test else {
            return None;
        };
        let sub = attribute.sub_attribute(leaf.path.as_str())?;
        let value = match value {
            CompValue::String(text) => Value::String(text.clone()),
            CompValue::True => Value::Bool(true),
            CompValue::False => Value::Bool(false),
            CompValue::Number(number) => Value::Number(number.clone()),
            CompValue::Null => return None,
        };
        record.insert(sub.name.clone(), value);
    }
    Some(record)
}

/// Refuses a change of a `readOnly` attribute (§3.5.2: "a client MUST NOT
/// modify an attribute that has mutability "readOnly"").
fn writable(attribute: &AttributeDefinition) -> Result<(), ScimError> {
    if attribute.mutability == Mutability::ReadOnly {
        return Err(ScimError::bad_request(
            ScimType::Mutability,
            format!("Attribute '{}' is readOnly.", attribute.name),
        ));
    }
    Ok(())
}

/// Refuses the removal of a required or `immutable` value (§3.5.2.2: an
/// attribute that "becomes unassigned and is defined as a required attribute
/// or a read-only attribute" is a `mutability` failure).
fn removable(attribute: &AttributeDefinition) -> Result<(), ScimError> {
    if attribute.required {
        return Err(ScimError::bad_request(
            ScimType::Mutability,
            format!("Attribute '{}' is required.", attribute.name),
        ));
    }
    if attribute.mutability == Mutability::Immutable {
        return Err(ScimError::bad_request(
            ScimType::Mutability,
            format!("Attribute '{}' is immutable.", attribute.name),
        ));
    }
    Ok(())
}

fn resolve<'s>(schema: &'s ResourceSchema, path: &AttrPath) -> Result<Resolved<'s>, ScimError> {
    schema.resolve(path).ok_or_else(|| {
        ScimError::bad_request(
            ScimType::InvalidPath,
            format!(
                "The attribute '{path}' is not defined for {}.",
                schema.resource_type()
            ),
        )
    })
}
