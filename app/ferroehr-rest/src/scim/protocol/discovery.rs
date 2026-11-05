// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The discovery documents of RFC 7644 §4, built from configuration:
//! `ServiceProviderConfig` (RFC 7643 §5), `ResourceTypes` (§6) and `Schemas`
//! (§7).
//!
//! A [`DiscoveryConfig`] names the resource types a provider serves, each
//! with a [`SchemaSelection`] of the attributes it maps; [`Discovery::build`]
//! checks it and builds every document once, so `/Schemas` advertises
//! exactly the selected attributes and every request is read against the
//! same [`ResourceSchema`]. Bulk, sort and password change are features this
//! layer does not implement, so the configuration has no switch for them and
//! the documents declare them unsupported.

use serde::{Deserialize, Serialize};

use crate::scim::protocol::deviation::{self, Deviation};
use crate::scim::protocol::error::ScimError;
use crate::scim::protocol::list::ListResponse;
use crate::scim::protocol::path::AttrPath;
use crate::scim::protocol::resource::ResourceSchema;
use crate::scim::protocol::schema::{AttributeDefinition, Meta, Schema};
use crate::scim::protocol::urn;

/// What a provider serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryConfig {
    /// The absolute base URL the endpoints sit under, without a trailing
    /// slash, such as `https://example.com/scim/v2`.
    pub base_url: String,
    /// A URL of the provider's documentation.
    pub documentation_uri: Option<String>,
    /// Whether PATCH is served.
    pub patch: bool,
    /// Whether resources carry entity tags.
    pub etag: bool,
    /// The filter capability.
    pub filter: FilterConfig,
    /// The authentication schemes, which `ServiceProviderConfig` lists.
    pub authentication_schemes: Vec<AuthenticationScheme>,
    /// The resource types.
    pub resource_types: Vec<ResourceTypeConfig>,
}

/// The filter capability (RFC 7643 §5, `filter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilterConfig {
    /// Whether filtering is supported.
    pub supported: bool,
    /// The most resources a response returns.
    pub max_results: u64,
}

/// One resource type a provider serves (RFC 7643 §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceTypeConfig {
    /// The resource type's name, such as `User`.
    pub name: String,
    /// The endpoint relative to the base URL, such as `/Users`.
    pub endpoint: String,
    /// The description.
    pub description: Option<String>,
    /// The base schema and the attributes the provider serves of it.
    pub schema: SchemaSelection,
    /// The schema extensions.
    pub extensions: Vec<ExtensionConfig>,
}

/// A schema extension of a resource type (RFC 7643 §6, `schemaExtensions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionConfig {
    /// The extension and the attributes the provider serves of it.
    pub schema: SchemaSelection,
    /// Whether a resource must carry the extension.
    pub required: bool,
}

/// A schema and the attributes a provider serves of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaSelection {
    schema: Schema,
    only: Option<Vec<AttrPath>>,
}

impl SchemaSelection {
    /// Selects every attribute of `schema`.
    #[must_use]
    pub const fn all(schema: Schema) -> Self {
        Self { schema, only: None }
    }

    /// Selects the attributes `paths` name of `schema`: an attribute whole,
    /// or one sub-attribute of a complex attribute.
    #[must_use]
    pub const fn only(schema: Schema, paths: Vec<AttrPath>) -> Self {
        Self {
            schema,
            only: Some(paths),
        }
    }

    /// Returns the schema with the selected attributes only, in the schema's order.
    fn selected(&self) -> Result<Schema, DiscoveryError> {
        let Some(paths) = &self.only else {
            return Ok(self.schema.clone());
        };
        for path in paths {
            let attribute = self.schema.attribute(path.name.as_str());
            let known = attribute.is_some_and(|a| {
                path.sub_attr
                    .as_ref()
                    .is_none_or(|sub| a.sub_attribute(sub.as_str()).is_some())
            });
            if !known
                || path
                    .schema
                    .as_deref()
                    .is_some_and(|schema| !urn::same(schema, &self.schema.id))
            {
                return Err(DiscoveryError::UnknownAttribute {
                    schema: self.schema.id.clone(),
                    attribute: path.to_string(),
                });
            }
        }
        let mut attributes = Vec::new();
        for attribute in &self.schema.attributes {
            let named: Vec<&AttrPath> = paths
                .iter()
                .filter(|path| path.name.matches(&attribute.name))
                .collect();
            if named.is_empty() {
                if attribute.required {
                    return Err(DiscoveryError::RequiredLeftOut {
                        schema: self.schema.id.clone(),
                        attribute: attribute.name.clone(),
                    });
                }
                continue;
            }
            attributes.push(narrowed(attribute, &named));
        }
        Ok(Schema {
            attributes,
            ..self.schema.clone()
        })
    }
}

/// Returns `attribute` with only the sub-attributes `named` selects, or whole
/// when one of them names it without a sub-attribute.
fn narrowed(attribute: &AttributeDefinition, named: &[&AttrPath]) -> AttributeDefinition {
    if named.iter().any(|path| path.sub_attr.is_none()) {
        return attribute.clone();
    }
    let sub_attributes = attribute.sub_attributes.as_ref().map(|subs| {
        subs.iter()
            .filter(|sub| {
                named.iter().any(|path| {
                    path.sub_attr
                        .as_ref()
                        .is_some_and(|name| name.matches(&sub.name))
                })
            })
            .cloned()
            .collect()
    });
    AttributeDefinition {
        sub_attributes,
        ..attribute.clone()
    }
}

/// An authentication scheme (RFC 7643 §5, `authenticationSchemes`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthenticationScheme {
    /// The scheme: `oauth`, `oauth2`, `oauthbearertoken`, `httpbasic` or `httpdigest`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The common name.
    pub name: String,
    /// The description.
    pub description: String,
    /// A URL of the scheme's specification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_uri: Option<String>,
    /// A URL of the scheme's usage documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_uri: Option<String>,
    /// Whether this is the preferred scheme (RFC 7643 §2.4; the §8.5 example sets it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<bool>,
}

impl AuthenticationScheme {
    /// Returns the OAuth bearer token scheme, as RFC 7643 §8.5 names it.
    #[must_use]
    pub fn oauth_bearer_token(documentation_uri: Option<String>) -> Self {
        Self {
            kind: "oauthbearertoken".to_owned(),
            name: "OAuth Bearer Token".to_owned(),
            description: "Authentication scheme using the OAuth Bearer Token Standard".to_owned(),
            spec_uri: Some("http://www.rfc-editor.org/info/rfc6750".to_owned()),
            documentation_uri,
            primary: None,
        }
    }
}

/// `supported` alone (RFC 7643 §5: `patch`, `changePassword`, `sort`, `etag`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Supported {
    /// Whether the feature is supported.
    pub supported: bool,
}

/// The bulk capability (RFC 7643 §5, `bulk`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BulkConfig {
    /// Whether bulk is supported.
    pub supported: bool,
    /// The most operations a request holds.
    pub max_operations: u64,
    /// The largest payload, in bytes.
    pub max_payload_size: u64,
}

/// The `ServiceProviderConfig` resource (RFC 7643 §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceProviderConfig {
    /// `urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig`.
    pub schemas: Vec<String>,
    /// A URL of the provider's documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_uri: Option<String>,
    /// PATCH.
    pub patch: Supported,
    /// Bulk.
    pub bulk: BulkConfig,
    /// Filtering.
    pub filter: FilterConfig,
    /// Password change.
    pub change_password: Supported,
    /// Sorting.
    pub sort: Supported,
    /// Entity tags.
    pub etag: Supported,
    /// The authentication schemes.
    pub authentication_schemes: Vec<AuthenticationScheme>,
    /// The resource metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// A schema extension of a `ResourceType` (RFC 7643 §6, `schemaExtensions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchemaExtension {
    /// The extension's URN.
    pub schema: String,
    /// Whether a resource must carry it.
    pub required: bool,
}

/// The `ResourceType` resource (RFC 7643 §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceType {
    /// `urn:ietf:params:scim:schemas:core:2.0:ResourceType`.
    pub schemas: Vec<String>,
    /// The resource type's id, its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The name.
    pub name: String,
    /// The endpoint relative to the base URL.
    pub endpoint: String,
    /// The description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The base schema's URN.
    pub schema: String,
    /// The schema extensions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_extensions: Option<Vec<SchemaExtension>>,
    /// The resource metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// A configuration the RFCs do not allow.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    /// The base URL is not an absolute `http` or `https` URL.
    #[error("the SCIM base URL '{0}' is not an absolute http(s) URL")]
    BaseUrl(String),
    /// Two resource types share a name or an endpoint.
    #[error("the resource type '{0}' is configured twice")]
    DuplicateResourceType(String),
    /// A selection names an attribute its schema does not define.
    #[error("the schema {schema} defines no attribute '{attribute}'")]
    UnknownAttribute {
        /// The schema.
        schema: String,
        /// The attribute named.
        attribute: String,
    },
    /// A selection leaves out a required attribute (RFC 7643 §7, `required`).
    #[error("the schema {schema} requires '{attribute}', which the selection leaves out")]
    RequiredLeftOut {
        /// The schema.
        schema: String,
        /// The required attribute.
        attribute: String,
    },
    /// An extension URN in the SCIM registry namespace is neither registered
    /// (RFC 7643 §10.4) nor of the custom extension form.
    #[error(
        "the extension URN {0} sits in urn:ietf:params:scim but is not registered (RFC 7643 §10.2.1)"
    )]
    UnregisteredUrn(String),
    /// Two schemas of one resource type share a URN.
    #[error("the schema {0} is configured twice")]
    DuplicateSchema(String),
}

/// The discovery documents and the resource schemas of one provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    service_provider_config: ServiceProviderConfig,
    resource_types: Vec<ResourceType>,
    schemas: Vec<Schema>,
    resource_schemas: Vec<ResourceSchema>,
    deviations: Vec<Deviation>,
}

impl Discovery {
    /// Checks `config` and builds every document.
    ///
    /// # Errors
    ///
    /// Returns [`DiscoveryError`] for a base URL that is not absolute, a
    /// resource type or schema configured twice, a selection naming an
    /// undefined attribute or leaving out a required one, and an extension
    /// URN in `urn:ietf:params:scim` that is neither registered nor of the
    /// form [`deviation::custom_extension_urn`] builds.
    pub fn build(config: DiscoveryConfig) -> Result<Self, DiscoveryError> {
        let base = config.base_url.trim_end_matches('/').to_owned();
        let absolute =
            url::Url::parse(&base).is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
        if !absolute {
            return Err(DiscoveryError::BaseUrl(config.base_url));
        }
        let mut resource_types = Vec::new();
        let mut resource_schemas: Vec<ResourceSchema> = Vec::new();
        let mut schemas: Vec<Schema> = Vec::new();
        let mut deviations = Vec::new();
        for resource_type in &config.resource_types {
            let taken = resource_types.iter().any(|existing: &ResourceType| {
                existing.name.eq_ignore_ascii_case(&resource_type.name)
                    || existing
                        .endpoint
                        .eq_ignore_ascii_case(&resource_type.endpoint)
            });
            if taken {
                return Err(DiscoveryError::DuplicateResourceType(
                    resource_type.name.clone(),
                ));
            }
            let base_schema = resource_type.schema.selected()?;
            check_urn(&base_schema.id, &mut deviations)?;
            let extensions = selected_extensions(resource_type, &base_schema, &mut deviations)?;
            resource_types.push(resource_type_document(
                resource_type,
                &base_schema,
                &extensions,
                &base,
            ));
            for schema in
                std::iter::once(&base_schema).chain(extensions.iter().map(|(schema, _)| schema))
            {
                if !schemas
                    .iter()
                    .any(|served| urn::same(&served.id, &schema.id))
                {
                    schemas.push(Schema {
                        schemas: Some(vec![urn::SCHEMA.to_owned()]),
                        meta: Some(meta("Schema", format!("{base}/Schemas/{}", schema.id))),
                        ..schema.clone()
                    });
                }
            }
            resource_schemas.push(ResourceSchema::new(
                &resource_type.name,
                base_schema,
                extensions,
            ));
        }
        let service_provider_config = ServiceProviderConfig {
            schemas: vec![urn::SERVICE_PROVIDER_CONFIG.to_owned()],
            documentation_uri: config.documentation_uri,
            patch: Supported {
                supported: config.patch,
            },
            bulk: BulkConfig {
                supported: false,
                max_operations: 0,
                max_payload_size: 0,
            },
            filter: config.filter,
            change_password: Supported { supported: false },
            sort: Supported { supported: false },
            etag: Supported {
                supported: config.etag,
            },
            authentication_schemes: config.authentication_schemes,
            meta: Some(meta(
                "ServiceProviderConfig",
                format!("{base}/ServiceProviderConfig"),
            )),
        };
        Ok(Self {
            service_provider_config,
            resource_types,
            schemas,
            resource_schemas,
            deviations,
        })
    }

    /// Returns the `ServiceProviderConfig` document.
    #[must_use]
    pub const fn service_provider_config(&self) -> &ServiceProviderConfig {
        &self.service_provider_config
    }

    /// Returns the `/ResourceTypes` list.
    #[must_use]
    pub fn resource_types(&self) -> ListResponse<&ResourceType> {
        let total = u64::try_from(self.resource_types.len()).unwrap_or(u64::MAX);
        ListResponse::new(self.resource_types.iter().collect(), total, 1)
    }

    /// Returns the resource type `id`, compared case-insensitively.
    #[must_use]
    pub fn resource_type(&self, id: &str) -> Option<&ResourceType> {
        self.resource_types
            .iter()
            .find(|resource_type| resource_type.name.eq_ignore_ascii_case(id))
    }

    /// Returns the `/Schemas` list: each schema once, in the order of the
    /// resource types.
    #[must_use]
    pub fn schemas(&self) -> ListResponse<&Schema> {
        let total = u64::try_from(self.schemas.len()).unwrap_or(u64::MAX);
        ListResponse::new(self.schemas.iter().collect(), total, 1)
    }

    /// Returns the schema whose URN is `id`, compared case-insensitively.
    #[must_use]
    pub fn schema(&self, id: &str) -> Option<&Schema> {
        self.schemas.iter().find(|schema| urn::same(&schema.id, id))
    }

    /// Returns the attributes requests of resource type `name` are read against.
    #[must_use]
    pub fn resource_schema(&self, name: &str) -> Option<&ResourceSchema> {
        self.resource_schemas
            .iter()
            .find(|schema| schema.resource_type().eq_ignore_ascii_case(name))
    }

    /// Returns the client deviations the configuration relies on.
    #[must_use]
    pub fn deviations(&self) -> &[Deviation] {
        &self.deviations
    }
}

/// Returns the refusal of a query on `/ResourceTypes` or `/Schemas` that
/// carries `filter`, or `None` when it carries none.
///
/// RFC 7644 §4: the other query parameters "SHALL be ignored", and "If a
/// "filter" is provided, the service provider SHOULD respond with HTTP status
/// code 403 (Forbidden) to ensure that clients cannot incorrectly assume that
/// any matching conditions specified in a filter are true".
#[must_use]
pub fn filter_refusal(filter: Option<&str>) -> Option<ScimError> {
    filter.map(|_filter| {
        ScimError::new(
            http::StatusCode::FORBIDDEN,
            "The discovery endpoints do not filter (RFC 7644 §4).",
        )
    })
}

/// Returns the selected extensions of `resource_type`, each with whether it
/// is required, refusing a URN the base schema or another extension holds.
fn selected_extensions(
    resource_type: &ResourceTypeConfig,
    base_schema: &Schema,
    deviations: &mut Vec<Deviation>,
) -> Result<Vec<(Schema, bool)>, DiscoveryError> {
    let mut extensions: Vec<(Schema, bool)> = Vec::new();
    for extension in &resource_type.extensions {
        let selected = extension.schema.selected()?;
        check_urn(&selected.id, deviations)?;
        let repeated = urn::same(&selected.id, &base_schema.id)
            || extensions
                .iter()
                .any(|(existing, _)| urn::same(&existing.id, &selected.id));
        if repeated {
            return Err(DiscoveryError::DuplicateSchema(selected.id));
        }
        extensions.push((selected, extension.required));
    }
    Ok(extensions)
}

/// Returns the `ResourceType` document of `resource_type` (RFC 7643 §6).
fn resource_type_document(
    resource_type: &ResourceTypeConfig,
    base_schema: &Schema,
    extensions: &[(Schema, bool)],
    base: &str,
) -> ResourceType {
    let schema_extensions = extensions
        .iter()
        .map(|(schema, required)| SchemaExtension {
            schema: schema.id.clone(),
            required: *required,
        })
        .collect::<Vec<_>>();
    ResourceType {
        schemas: vec![urn::RESOURCE_TYPE.to_owned()],
        id: Some(resource_type.name.clone()),
        name: resource_type.name.clone(),
        endpoint: resource_type.endpoint.clone(),
        description: resource_type.description.clone(),
        schema: base_schema.id.clone(),
        schema_extensions: (!schema_extensions.is_empty()).then_some(schema_extensions),
        meta: Some(meta(
            "ResourceType",
            format!("{base}/ResourceTypes/{}", resource_type.name),
        )),
    }
}

/// Refuses an unregistered URN in the SCIM registry namespace, and accepts
/// the custom extension form as [`Deviation::UnregisteredExtensionUrn`].
fn check_urn(id: &str, deviations: &mut Vec<Deviation>) -> Result<(), DiscoveryError> {
    let in_registry = id
        .get(..urn::REGISTRY_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(urn::REGISTRY_PREFIX));
    if !in_registry
        || urn::REGISTERED_SCHEMAS
            .iter()
            .any(|registered| urn::same(registered, id))
    {
        return Ok(());
    }
    if deviation::is_custom_extension_urn(id) {
        if !deviations.contains(&Deviation::UnregisteredExtensionUrn) {
            deviations.push(Deviation::UnregisteredExtensionUrn);
        }
        return Ok(());
    }
    Err(DiscoveryError::UnregisteredUrn(id.to_owned()))
}

fn meta(resource_type: &str, location: String) -> Meta {
    Meta {
        resource_type: Some(resource_type.to_owned()),
        location: Some(location),
        ..Meta::default()
    }
}
