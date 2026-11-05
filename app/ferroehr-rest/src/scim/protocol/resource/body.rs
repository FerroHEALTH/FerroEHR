// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A resource body's members against its schema: `schemas` (RFC 7643 §3),
//! the attributes at the top level and in each extension container (§3.3),
//! and the fully qualified attribute names RFC 7644 §3.10 allows in every
//! operation.

use serde_json::{Map, Value};

use super::value::{self, Origin};
use super::{Container, ResourceSchema, container_key};
use crate::scim::protocol::deviation::Deviation;
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::path::AttrPath;
use crate::scim::protocol::schema::Mutability;
use crate::scim::protocol::urn;

/// A body read into canonical form.
pub(super) struct Read {
    /// The attributes, `schemas` first.
    pub(super) attributes: Map<String, Value>,
    /// The client deviations the body needed.
    pub(super) deviations: Vec<Deviation>,
    /// Every attribute the body named, by container key and name, assigned
    /// or not.
    pub(super) asserted: Vec<(Option<String>, String)>,
}

/// Reads `body` against `schema`.
pub(super) fn read(
    body: &Value,
    schema: &ResourceSchema,
    origin: Origin,
) -> Result<Read, ScimError> {
    let Value::Object(members) = body else {
        return Err(ScimError::bad_request(
            ScimType::InvalidSyntax,
            "A resource body is a JSON object.",
        ));
    };
    let mut read = Read {
        attributes: Map::new(),
        deviations: Vec::new(),
        asserted: Vec::new(),
    };
    let listed = listed_extensions(members, schema, &mut read.deviations)?;
    for (key, member) in members {
        if key.eq_ignore_ascii_case("schemas") {
            continue;
        }
        if let Some(extension) = schema.extension(key) {
            let id = extension.id.as_str();
            let Value::Object(inner) = member else {
                if value::is_unassigned(member) {
                    continue;
                }
                return Err(ScimError::bad_request(
                    ScimType::InvalidValue,
                    format!("The extension '{key}' takes an object of its attributes."),
                ));
            };
            if !listed.iter().any(|listed| listed == id)
                && !read.deviations.contains(&Deviation::SchemasOmitExtension)
            {
                read.deviations.push(Deviation::SchemasOmitExtension);
            }
            for (name, value) in inner {
                place(
                    &mut read,
                    schema,
                    Container::Extension(id),
                    name,
                    value,
                    origin,
                )?;
            }
            continue;
        }
        // NOTE: a key that is no attribute path is legitimately not of the qualified form; it is read as a name.
        let qualified = AttrPath::parse(key)
            .ok()
            .filter(|path| path.schema.is_some() && path.sub_attr.is_none());
        match qualified.as_ref().and_then(|path| schema.resolve(path)) {
            Some(resolved) => {
                let name = resolved.attribute.name.clone();
                place(&mut read, schema, resolved.container, &name, member, origin)?;
            }
            None => place(&mut read, schema, Container::Core, key, member, origin)?,
        }
    }
    settle_schemas(&mut read.attributes, schema);
    Ok(read)
}

/// Checks `schemas` and returns the extension URNs it lists, in their schema
/// spelling; a URI the resource type does not define is ignored as
/// [`Deviation::UnlistedSchemaUri`], since an attribute under it is refused
/// as undefined anyway.
fn listed_extensions(
    members: &Map<String, Value>,
    schema: &ResourceSchema,
    deviations: &mut Vec<Deviation>,
) -> Result<Vec<String>, ScimError> {
    let mut entries = members
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("schemas"));
    let Some((_, value)) = entries.next() else {
        return Err(ScimError::bad_request(
            ScimType::InvalidSyntax,
            "A resource body carries 'schemas' (RFC 7643 §3).",
        ));
    };
    if entries.next().is_some() {
        return Err(ScimError::bad_request(
            ScimType::InvalidSyntax,
            "'schemas' appears twice.",
        ));
    }
    let Some(items) = value.as_array().filter(|items| !items.is_empty()) else {
        return Err(ScimError::bad_request(
            ScimType::InvalidSyntax,
            "'schemas' is a non-empty array of schema URIs.",
        ));
    };
    let mut seen: Vec<&str> = Vec::with_capacity(items.len());
    for item in items {
        let uri = item.as_str().ok_or_else(|| {
            ScimError::bad_request(
                ScimType::InvalidSyntax,
                "'schemas' is a non-empty array of schema URIs.",
            )
        })?;
        if seen.iter().any(|other| urn::same(other, uri)) {
            return Err(ScimError::bad_request(
                ScimType::InvalidSyntax,
                format!("'{uri}' appears twice in 'schemas' (RFC 7643 §3)."),
            ));
        }
        seen.push(uri);
    }
    if !seen.iter().any(|uri| urn::same(uri, &schema.base().id)) {
        return Err(ScimError::bad_request(
            ScimType::InvalidSyntax,
            format!(
                "'schemas' lacks the base schema {} (RFC 7643 §3.3).",
                schema.base().id
            ),
        ));
    }
    let mut listed = Vec::new();
    for uri in seen
        .into_iter()
        .filter(|uri| !urn::same(uri, &schema.base().id))
    {
        match schema.extension(uri) {
            Some(extension) => listed.push(extension.id.clone()),
            None if !deviations.contains(&Deviation::UnlistedSchemaUri) => {
                deviations.push(Deviation::UnlistedSchemaUri);
            }
            None => {}
        }
    }
    Ok(listed)
}

/// Checks the member `name` of `container` and puts it in canonical form.
fn place(
    read: &mut Read,
    schema: &ResourceSchema,
    container: Container<'_>,
    name: &str,
    member: &Value,
    origin: Origin,
) -> Result<(), ScimError> {
    let Some(attribute) = schema
        .attributes_of(container)
        .find(|attribute| attribute.name.eq_ignore_ascii_case(name))
    else {
        // NOTE: RFC 7643 §2.5: an unassigned value asserts the state an undefined attribute already has.
        if value::is_unassigned(member) {
            return Ok(());
        }
        return Err(ScimError::bad_request(
            ScimType::InvalidValue,
            format!(
                "Attribute '{name}' is not defined for {}.",
                schema.resource_type()
            ),
        ));
    };
    let key = (container_key(container), attribute.name.clone());
    if read.asserted.contains(&key) {
        return Err(ScimError::bad_request(
            ScimType::InvalidSyntax,
            format!("Attribute '{}' appears twice.", attribute.name),
        ));
    }
    read.asserted.push(key);
    if origin == Origin::Request && attribute.mutability == Mutability::ReadOnly {
        return Ok(());
    }
    if let Some(value) = value::normalize(attribute, member, origin, &attribute.name)? {
        set(
            &mut read.attributes,
            container,
            &attribute.name,
            Some(value),
        );
    }
    Ok(())
}

/// Refuses a resource that lacks a required attribute or a required
/// extension (RFC 7643 §6, `schemaExtensions.required`).
pub(crate) fn require(
    attributes: &Map<String, Value>,
    schema: &ResourceSchema,
) -> Result<(), ScimError> {
    let mut containers = vec![(Container::Core, true)];
    for (extension, required) in schema.extensions() {
        let present = attributes.contains_key(&extension.id);
        if *required && !present {
            return Err(ScimError::bad_request(
                ScimType::InvalidValue,
                format!("The extension {} is required.", extension.id),
            ));
        }
        containers.push((Container::Extension(&extension.id), present));
    }
    for (container, _) in containers.into_iter().filter(|(_, present)| *present) {
        if let Some(missing) = schema.attributes_of(container).find(|attribute| {
            attribute.required && get(attributes, container, &attribute.name).is_none()
        }) {
            return Err(ScimError::bad_request(
                ScimType::InvalidValue,
                format!("Attribute '{}' is required.", missing.name),
            ));
        }
    }
    Ok(())
}

/// Returns the value of attribute `name` in `container`.
pub(crate) fn get<'a>(
    attributes: &'a Map<String, Value>,
    container: Container<'_>,
    name: &str,
) -> Option<&'a Value> {
    match container {
        Container::Core => attributes.get(name),
        Container::Extension(id) => attributes.get(id)?.as_object()?.get(name),
    }
}

/// Sets, or with `None` removes, attribute `name` in `container`; an
/// extension container left empty is removed.
pub(crate) fn set(
    attributes: &mut Map<String, Value>,
    container: Container<'_>,
    name: &str,
    value: Option<Value>,
) {
    match (container, value) {
        (Container::Core, Some(value)) => {
            attributes.insert(name.to_owned(), value);
        }
        (Container::Core, None) => {
            attributes.remove(name);
        }
        (Container::Extension(id), Some(value)) => {
            let entry = attributes
                .entry(id.to_owned())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(inner) = entry {
                inner.insert(name.to_owned(), value);
            }
        }
        (Container::Extension(id), None) => {
            let emptied = attributes
                .get_mut(id)
                .and_then(Value::as_object_mut)
                .is_some_and(|inner| {
                    inner.remove(name);
                    inner.is_empty()
                });
            if emptied {
                attributes.remove(id);
            }
        }
    }
}

/// Puts `schemas` first, listing the base schema and each extension whose
/// container holds a value (RFC 7643 §3: the URIs "that define the attributes
/// present").
pub(crate) fn settle_schemas(attributes: &mut Map<String, Value>, schema: &ResourceSchema) {
    let mut schemas = vec![Value::String(schema.base().id.clone())];
    for (extension, _) in schema.extensions() {
        if attributes
            .get(&extension.id)
            .and_then(Value::as_object)
            .is_some_and(|inner| !inner.is_empty())
        {
            schemas.push(Value::String(extension.id.clone()));
        }
    }
    let mut settled = Map::with_capacity(attributes.len() + 1);
    settled.insert("schemas".to_owned(), Value::Array(schemas));
    for (key, value) in std::mem::take(attributes) {
        if key != "schemas" {
            settled.insert(key, value);
        }
    }
    *attributes = settled;
}
