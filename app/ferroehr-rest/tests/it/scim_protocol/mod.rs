// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SCIM 2.0 protocol layer (`ferroehr_rest::scim::protocol`) against the
//! examples of RFC 7643, RFC 7644 and Microsoft's Entra ID provisioning
//! documents, each read from its vendored text by [`source`].
//!
//! The suite runs the generic RFC 7644 client first (the RFC examples), then
//! the Entra ID example client's request shapes (`entra`), whose deviations the
//! provider tolerates as labelled inputs. No test here touches a database or a
//! route: the layer has neither.

mod discovery;
mod entra;
mod error;
mod etag;
mod filter;
mod filter_properties;
mod patch;
mod resource;
mod schema;
pub(crate) mod source;

use ferroehr_rest::scim::protocol::discovery::{
    AuthenticationScheme, Discovery, DiscoveryConfig, ExtensionConfig, FilterConfig,
    ResourceTypeConfig, SchemaSelection,
};
use ferroehr_rest::scim::protocol::resource::{Resource, ResourceSchema};
use ferroehr_rest::scim::protocol::schema::catalogue;
use serde_json::Value;

/// The base URL every example of RFC 7643 and RFC 7644 is served from.
pub(crate) const BASE_URL: &str = "https://example.com/v2";

/// A provider that serves every attribute RFC 7643 defines: the User with the
/// enterprise extension, and the Group.
pub(crate) fn rfc_provider() -> Discovery {
    rfc_provider_with(true)
}

/// [`rfc_provider`], with the enterprise extension `required` as given.
pub(crate) fn rfc_provider_with(enterprise_required: bool) -> Discovery {
    Discovery::build(DiscoveryConfig {
        base_url: BASE_URL.to_owned(),
        documentation_uri: None,
        patch: true,
        etag: true,
        filter: FilterConfig {
            supported: true,
            max_results: 200,
        },
        authentication_schemes: vec![AuthenticationScheme::oauth_bearer_token(None)],
        resource_types: vec![
            ResourceTypeConfig {
                name: "User".to_owned(),
                endpoint: "/Users".to_owned(),
                description: Some("User Account".to_owned()),
                schema: SchemaSelection::all(catalogue::core_user()),
                extensions: vec![ExtensionConfig {
                    schema: SchemaSelection::all(catalogue::enterprise_user()),
                    required: enterprise_required,
                }],
            },
            ResourceTypeConfig {
                name: "Group".to_owned(),
                endpoint: "/Groups".to_owned(),
                description: Some("Group".to_owned()),
                schema: SchemaSelection::all(catalogue::core_group()),
                extensions: vec![],
            },
        ],
    })
    .expect("the RFC 7643 provider should build")
}

/// The User resource schema of [`rfc_provider`], with the extension optional.
pub(crate) fn user_schema() -> ResourceSchema {
    rfc_provider_with(false)
        .resource_schema("User")
        .expect("the provider serves User")
        .clone()
}

/// The Group resource schema of [`rfc_provider`].
pub(crate) fn group_schema() -> ResourceSchema {
    rfc_provider()
        .resource_schema("Group")
        .expect("the provider serves Group")
        .clone()
}

/// Reads `value` as a representation the provider holds.
pub(crate) fn held(value: &Value, schema: &ResourceSchema) -> Resource {
    Resource::from_representation(value, schema).unwrap_or_else(|error| panic!("{error}: {value}"))
}
