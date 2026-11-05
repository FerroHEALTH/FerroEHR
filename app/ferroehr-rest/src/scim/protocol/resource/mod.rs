// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A SCIM resource against the schema of its resource type.
//!
//! [`ResourceSchema`] is one resource type's attributes: the common
//! attributes (RFC 7643 §3.1), the base schema and the schema extensions
//! (§3.3), and resolves an attribute path against them (RFC 7644 §3.10).
//! [`Resource`] is a body checked against it, in one canonical form: every
//! name in its schema's spelling, extension attributes inside the container
//! named by the extension URN, `schemas` listing exactly the base schema and
//! the extensions present, and no unassigned value. The provider builds one
//! from a create body ([`Resource::from_request`], RFC 7644 §3.3), a replace
//! body ([`Resource::replace`], §3.5.1) or its own representation
//! ([`Resource::from_representation`]).

pub(crate) mod body;
pub(crate) mod value;

use std::fmt;

use serde_json::{Map, Value};

use crate::scim::protocol::deviation::Deviation;
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::path::AttrPath;
use crate::scim::protocol::schema::{AttributeDefinition, Mutability, Schema, catalogue};
use crate::scim::protocol::urn;

/// The attributes of one resource type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSchema {
    resource_type: String,
    common: Vec<AttributeDefinition>,
    base: Schema,
    extensions: Vec<(Schema, bool)>,
}

/// Where an attribute's value sits in a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container<'a> {
    /// At the top level: a common or a base schema attribute.
    Core,
    /// Inside the container keyed by this extension URN.
    Extension(&'a str),
}

/// An attribute path resolved against a [`ResourceSchema`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved<'a> {
    /// Where the attribute's value sits.
    pub container: Container<'a>,
    /// The attribute.
    pub attribute: &'a AttributeDefinition,
    /// The sub-attribute the path names, of a complex attribute.
    pub sub_attribute: Option<&'a AttributeDefinition>,
}

impl ResourceSchema {
    /// Creates the schema of resource type `resource_type` from its base
    /// schema and its extensions, each with whether it is required.
    #[must_use]
    pub fn new(resource_type: &str, base: Schema, extensions: Vec<(Schema, bool)>) -> Self {
        Self {
            resource_type: resource_type.to_owned(),
            common: catalogue::common_attributes(),
            base,
            extensions,
        }
    }

    /// Returns the name of the resource type.
    #[must_use]
    pub fn resource_type(&self) -> &str {
        &self.resource_type
    }

    /// Returns the base schema.
    #[must_use]
    pub const fn base(&self) -> &Schema {
        &self.base
    }

    /// Returns the extensions, each with whether it is required.
    #[must_use]
    pub fn extensions(&self) -> &[(Schema, bool)] {
        &self.extensions
    }

    /// Returns the extension whose URN is `id`, compared case-insensitively.
    #[must_use]
    pub fn extension(&self, id: &str) -> Option<&Schema> {
        self.extensions
            .iter()
            .map(|(schema, _)| schema)
            .find(|schema| urn::same(&schema.id, id))
    }

    /// Returns the common attributes (RFC 7643 §3.1).
    #[must_use]
    pub fn common(&self) -> &[AttributeDefinition] {
        &self.common
    }

    /// Resolves `path` against the resource type's attributes.
    ///
    /// A path qualified by the base schema URN names a common or base
    /// attribute, one qualified by an extension URN an attribute of that
    /// extension. An unqualified name is looked up in the common and base
    /// attributes, then in the extensions, where RFC 7644 §3.10 lets a client
    /// omit the URN of an extended attribute it "SHOULD fully qualify"; a name
    /// two extensions define resolves to nothing.
    #[must_use]
    pub fn resolve(&self, path: &AttrPath) -> Option<Resolved<'_>> {
        let name = path.name.as_str();
        let (container, attribute) = match &path.schema {
            Some(schema) if urn::same(schema, &self.base.id) => {
                (Container::Core, self.core_attribute(name)?)
            }
            Some(schema) => {
                let extension = self.extension(schema)?;
                (
                    Container::Extension(&extension.id),
                    extension.attribute(name)?,
                )
            }
            None => {
                if let Some(attribute) = self.core_attribute(name) {
                    (Container::Core, attribute)
                } else {
                    let mut found = self
                        .extensions
                        .iter()
                        .filter_map(|(schema, _)| Some((schema, schema.attribute(name)?)));
                    let (extension, attribute) = found.next()?;
                    if found.next().is_some() {
                        return None;
                    }
                    (Container::Extension(&extension.id), attribute)
                }
            }
        };
        let sub_attribute = match &path.sub_attr {
            Some(sub) => Some(attribute.sub_attribute(sub.as_str())?),
            None => None,
        };
        Some(Resolved {
            container,
            attribute,
            sub_attribute,
        })
    }

    /// Returns the extension a path names whole, as a PATCH path or a
    /// projection may: the path is the extension URN itself.
    #[must_use]
    pub fn extension_named(&self, path: &AttrPath) -> Option<&Schema> {
        let schema = path.schema.as_deref()?;
        if path.sub_attr.is_some() {
            return None;
        }
        self.extension(&format!("{schema}:{}", path.name))
    }

    fn core_attribute(&self, name: &str) -> Option<&AttributeDefinition> {
        self.common
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case(name))
            .or_else(|| self.base.attribute(name))
    }

    /// Returns the attributes of `container`.
    fn attributes_of<'a>(
        &'a self,
        container: Container<'_>,
    ) -> Box<dyn Iterator<Item = &'a AttributeDefinition> + 'a> {
        match container {
            Container::Core => Box::new(self.common.iter().chain(self.base.attributes.iter())),
            Container::Extension(id) => Box::new(
                self.extension(id)
                    .into_iter()
                    .flat_map(|schema| schema.attributes.iter()),
            ),
        }
    }
}

/// A resource in canonical form, checked against its [`ResourceSchema`].
#[derive(Clone, PartialEq, Eq)]
pub struct Resource(Map<String, Value>);

/// A resource read from a request, with the client deviations it needed.
#[derive(Clone, PartialEq, Eq)]
pub struct Parsed {
    resource: Resource,
    deviations: Vec<Deviation>,
}

impl Parsed {
    /// Returns the resource.
    #[must_use]
    pub const fn resource(&self) -> &Resource {
        &self.resource
    }

    /// Returns the resource, consuming the outcome.
    #[must_use]
    pub fn into_resource(self) -> Resource {
        self.resource
    }

    /// Returns the client deviations the request needed, in the order met.
    #[must_use]
    pub fn deviations(&self) -> &[Deviation] {
        &self.deviations
    }
}

impl Resource {
    /// Reads a create body (RFC 7644 §3.3).
    ///
    /// `readOnly` attributes are ignored, an unassigned value of an attribute
    /// the schema lacks asserts nothing (RFC 7643 §2.5), and every required
    /// attribute must be present.
    ///
    /// # Errors
    ///
    /// Returns a `400` with `invalidSyntax` when the body is not an object, its
    /// `schemas` is missing, empty, repeats a URI or lacks the base schema, or
    /// an attribute appears twice; with `invalidValue` when an attribute is
    /// not defined, a value has the wrong type, or a required attribute is missing.
    pub fn from_request(body: &Value, schema: &ResourceSchema) -> Result<Parsed, ScimError> {
        let read = body::read(body, schema, value::Origin::Request)?;
        body::require(&read.attributes, schema)?;
        Ok(Parsed {
            resource: Self(read.attributes),
            deviations: read.deviations,
        })
    }

    /// Reads a representation the provider holds, with its `readOnly`
    /// attributes; it must carry its `id` (RFC 7643 §3.1).
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Resource::from_request`], and `invalidValue`
    /// when the representation has no `id`.
    pub fn from_representation(body: &Value, schema: &ResourceSchema) -> Result<Self, ScimError> {
        let read = body::read(body, schema, value::Origin::Representation)?;
        body::require(&read.attributes, schema)?;
        if read
            .attributes
            .get("id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(ScimError::bad_request(
                ScimType::InvalidValue,
                "A representation carries a non-empty 'id'.",
            ));
        }
        Ok(Self(read.attributes))
    }

    /// Replaces this resource's attributes with a PUT body (RFC 7644 §3.5.1).
    ///
    /// A `readWrite` or `writeOnly` attribute the body omits is cleared, as
    /// the section allows, except the attributes in `keep_when_omitted`, which
    /// keep their value unless the body names them. A `readOnly` value comes
    /// from this resource, and an `immutable` one keeps its value unless the
    /// body sends the same.
    ///
    /// # Errors
    ///
    /// Returns the refusals of [`Resource::from_request`], and a `400` with
    /// `mutability` when the body changes an `immutable` value.
    pub fn replace(
        &self,
        body: &Value,
        schema: &ResourceSchema,
        keep_when_omitted: &[AttrPath],
    ) -> Result<Parsed, ScimError> {
        let read = body::read(body, schema, value::Origin::Request)?;
        let mut attributes = read.attributes;
        for container in std::iter::once(Container::Core).chain(
            schema
                .extensions
                .iter()
                .map(|(s, _)| Container::Extension(&s.id)),
        ) {
            for attribute in schema.attributes_of(container) {
                let current = self.get(container, &attribute.name);
                let kept = keep_when_omitted.iter().any(|path| {
                    schema.resolve(path).is_some_and(|r| {
                        r.container == container && r.attribute.name == attribute.name
                    })
                }) && !read
                    .asserted
                    .iter()
                    .any(|(c, name)| *c == container_key(container) && *name == attribute.name);
                let keep = match attribute.mutability {
                    Mutability::ReadOnly => true,
                    Mutability::Immutable => {
                        let incoming = body::get(&attributes, container, &attribute.name);
                        if let (Some(current), Some(incoming)) = (current, incoming)
                            && current != incoming
                        {
                            return Err(ScimError::bad_request(
                                ScimType::Mutability,
                                format!("Attribute '{}' is immutable.", attribute.name),
                            ));
                        }
                        incoming.is_none()
                    }
                    Mutability::ReadWrite | Mutability::WriteOnly => kept,
                };
                if keep {
                    body::set(
                        &mut attributes,
                        container,
                        &attribute.name,
                        current.cloned(),
                    );
                }
            }
        }
        body::settle_schemas(&mut attributes, schema);
        body::require(&attributes, schema)?;
        Ok(Parsed {
            resource: Self(attributes),
            deviations: read.deviations,
        })
    }

    /// Returns the value of attribute `name` in `container`.
    #[must_use]
    pub fn get(&self, container: Container<'_>, name: &str) -> Option<&Value> {
        body::get(&self.0, container, name)
    }

    /// Returns the attributes, `schemas` first.
    #[must_use]
    pub const fn attributes(&self) -> &Map<String, Value> {
        &self.0
    }

    /// Returns the resource as a JSON value.
    #[must_use]
    pub fn to_value(&self) -> Value {
        Value::Object(self.0.clone())
    }

    /// Returns the resource as a JSON value, consuming it.
    #[must_use]
    pub fn into_value(self) -> Value {
        Value::Object(self.0)
    }

    /// Wraps attributes the PATCH engine has already brought into canonical form.
    pub(crate) const fn from_canonical(attributes: Map<String, Value>) -> Self {
        Self(attributes)
    }
}

/// The key a container's attributes sit under: `None` at the top level.
pub(crate) fn container_key(container: Container<'_>) -> Option<String> {
    match container {
        Container::Core => None,
        Container::Extension(id) => Some(id.to_owned()),
    }
}

// NOTE: a resource holds personal data, so its Debug output lists attribute names only.
impl fmt::Debug for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resource")
            .field("attributes", &self.0.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl fmt::Debug for Parsed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Parsed")
            .field("resource", &self.resource)
            .field("deviations", &self.deviations)
            .finish()
    }
}
