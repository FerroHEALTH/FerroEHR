// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SCIM schema model (RFC 7643 §2.2, §2.3, §7) and the `meta` attribute
//! (§3.1).
//!
//! [`Schema`] and [`AttributeDefinition`] are the wire form of the `Schema`
//! resource: they read the RFC 7643 §8.7 representations without loss and are
//! what `/Schemas` serves. A characteristic the RFC prints is `Some`; one it
//! omits is `None`, read through the §2.2 defaults by the accessors.
//! [`catalogue`] holds the User, Group and Enterprise User schemas.

pub mod catalogue;

use serde::{Deserialize, Serialize};

/// An attribute's data type (RFC 7643 §2.3, Table 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttributeType {
    /// A JSON string of Unicode characters (§2.3.1).
    String,
    /// `true` or `false` (§2.3.2).
    Boolean,
    /// A real number (§2.3.3).
    Decimal,
    /// A whole number (§2.3.4).
    Integer,
    /// An `xsd:dateTime` string (§2.3.5).
    DateTime,
    /// A base64 string (§2.3.6).
    Binary,
    /// A URI string (§2.3.7).
    Reference,
    /// An object of sub-attributes (§2.3.8).
    Complex,
}

/// When an attribute's value may be (re)defined (RFC 7643 §7, `mutability`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mutability {
    /// The attribute SHALL NOT be modified.
    ReadOnly,
    /// The attribute MAY be updated and read at any time.
    ReadWrite,
    /// The attribute MAY be defined at creation or replacement and SHALL NOT be updated.
    Immutable,
    /// The attribute MAY be updated and SHALL NOT be returned.
    WriteOnly,
}

/// When an attribute is returned (RFC 7643 §7, `returned`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Returned {
    /// Always, whatever the `attributes` parameter names.
    Always,
    /// Never.
    Never,
    /// By default, unless the `attributes` parameter leaves it out.
    Default,
    /// Only when the `attributes` parameter names it.
    Request,
}

/// How a provider enforces uniqueness of a value (RFC 7643 §7, `uniqueness`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Uniqueness {
    /// Not unique in any way.
    None,
    /// Unique within the provider.
    Server,
    /// Globally unique.
    Global,
}

/// An attribute definition, one entry of a schema's `attributes` (RFC 7643 §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttributeDefinition {
    /// The attribute's name.
    pub name: String,
    /// The attribute's data type.
    #[serde(rename = "type")]
    pub kind: AttributeType,
    /// The sub-attributes of a complex attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_attributes: Option<Vec<AttributeDefinition>>,
    /// Whether the attribute holds several values.
    pub multi_valued: bool,
    /// The human-readable description.
    pub description: String,
    /// Whether the attribute is required.
    pub required: bool,
    /// The suggested canonical values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_values: Option<Vec<String>>,
    /// Whether a string value is case-sensitive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_exact: Option<bool>,
    /// When the value may be (re)defined.
    pub mutability: Mutability,
    /// When the value is returned.
    pub returned: Returned,
    /// How uniqueness is enforced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uniqueness: Option<Uniqueness>,
    /// The resource types a reference may point at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_types: Option<Vec<String>>,
}

impl AttributeDefinition {
    /// Returns the sub-attribute `name`, compared case-insensitively (RFC 7643 §2.1).
    #[must_use]
    pub fn sub_attribute(&self, name: &str) -> Option<&Self> {
        self.sub_attributes
            .as_deref()
            .and_then(|subs| subs.iter().find(|sub| sub.name.eq_ignore_ascii_case(name)))
    }

    /// Returns whether string values compare case-sensitively.
    #[must_use]
    pub fn is_case_exact(&self) -> bool {
        // NOTE: RFC 7643 §2.2: an omitted "caseExact" is "false" (i.e., case-insensitive).
        self.case_exact.unwrap_or(false)
    }

    /// Returns whether the attribute is complex.
    #[must_use]
    pub fn is_complex(&self) -> bool {
        self.kind == AttributeType::Complex
    }
}

/// A schema definition, the `Schema` resource of RFC 7643 §7.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Schema {
    /// The resource schema, `urn:ietf:params:scim:schemas:core:2.0:Schema`,
    /// on a served document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schemas: Option<Vec<String>>,
    /// The schema URI.
    pub id: String,
    /// The human-readable name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The attribute definitions.
    pub attributes: Vec<AttributeDefinition>,
    /// The resource metadata of a served document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

impl Schema {
    /// Returns the attribute `name`, compared case-insensitively (RFC 7643 §2.1).
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&AttributeDefinition> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case(name))
    }
}

/// The `meta` attribute of a resource (RFC 7643 §3.1), every sub-attribute of
/// which the provider assigns.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Meta {
    /// The name of the resource's type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    /// When the resource was added.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// When the resource was last updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    /// The URI of the resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The version, equal to the `ETag` header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}
