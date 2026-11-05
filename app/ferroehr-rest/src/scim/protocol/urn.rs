// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The URNs of the SCIM schemas (RFC 7643 §10.4) and messages (RFC 7644 §8.2).

/// The core User schema (RFC 7643 §4.1).
pub const CORE_USER: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
/// The core Group schema (RFC 7643 §4.2).
pub const CORE_GROUP: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
/// The Enterprise User extension (RFC 7643 §4.3).
pub const ENTERPRISE_USER: &str = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User";
/// The service provider configuration schema (RFC 7643 §5).
pub const SERVICE_PROVIDER_CONFIG: &str =
    "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig";
/// The resource type schema (RFC 7643 §6).
pub const RESOURCE_TYPE: &str = "urn:ietf:params:scim:schemas:core:2.0:ResourceType";
/// The schema of a schema definition (RFC 7643 §7).
pub const SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Schema";
/// The list response message (RFC 7644 §3.4.2).
pub const LIST_RESPONSE: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
/// The search request message (RFC 7644 §3.4.3).
pub const SEARCH_REQUEST: &str = "urn:ietf:params:scim:api:messages:2.0:SearchRequest";
/// The PATCH request message (RFC 7644 §3.5.2).
pub const PATCH_OP: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
/// The error response message (RFC 7644 §3.12).
pub const ERROR: &str = "urn:ietf:params:scim:api:messages:2.0:Error";

/// The prefix of every URN the SCIM registry assigns (RFC 7643 §10.2).
pub const REGISTRY_PREFIX: &str = "urn:ietf:params:scim:";

/// The schema URNs RFC 7643 §10.4 registers.
pub const REGISTERED_SCHEMAS: [&str; 6] = [
    CORE_USER,
    CORE_GROUP,
    ENTERPRISE_USER,
    SERVICE_PROVIDER_CONFIG,
    RESOURCE_TYPE,
    SCHEMA,
];

/// Returns whether two schema URIs name the same schema.
///
/// RFC 7644 §3.10: "All facets (URN, attribute, and sub-attribute name) of the
/// fully encoded attribute name are case insensitive."
#[must_use]
pub fn same(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}
