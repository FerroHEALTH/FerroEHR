// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The discovery documents FerroEHR's SCIM endpoint serves, built from
//! `[scim]` and the shared party mapping.
//!
//! RFC 7644 §4: `/Schemas` returns "all supported schemas", so the User
//! schema is the core User reduced to `userName`, `active` and the attributes
//! `[[authz.party_binding.person.attribute]]` maps, and FerroEHR's own
//! extension carries the binding key (`loginIdentifier`), the lifecycle state
//! of an incomplete User and every other extension attribute the mapping
//! names. The extension URN takes the form Microsoft Entra requires of a
//! custom attribute ([`crate::scim::protocol::deviation::custom_extension_urn`]). Which attributes
//! are served is our own design.

use ferroehr::config::binding::PartyBindingConfig;
use ferroehr::config::scim::ScimConfig;

use crate::scim::protocol::discovery::{
    AuthenticationScheme, Discovery, DiscoveryConfig, DiscoveryError, ExtensionConfig,
    FilterConfig, ResourceTypeConfig, SchemaSelection,
};
use crate::scim::protocol::path::AttrPath;
use crate::scim::protocol::schema::catalogue;
use crate::scim::protocol::schema::{
    AttributeDefinition, AttributeType, Mutability, Returned, Schema, Uniqueness,
};
use crate::scim::protocol::urn;

/// The extension attribute that feeds the binding key.
pub(crate) const LOGIN_IDENTIFIER: &str = "loginIdentifier";

/// The extension attribute naming an incomplete User's lifecycle state.
pub(crate) const LIFECYCLE_STATE: &str = "lifecycleState";

/// The extension attribute listing the template paths an incomplete User
/// lacks.
pub(crate) const MISSING_PATHS: &str = "missingPaths";

/// The URN of FerroEHR's User extension, in the form
/// [`crate::scim::protocol::deviation::custom_extension_urn`] builds.
pub(crate) const EXTENSION_URN: &str =
    "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User";

/// The discovery documents of `scim` over the person mapping of `binding`.
///
/// # Errors
/// [`DiscoveryError`] when a mapped SCIM attribute names no attribute of the
/// schema it sits in, or the base URL is not absolute.
pub(crate) fn discovery(
    scim: &ScimConfig,
    binding: &PartyBindingConfig,
) -> Result<Discovery, DiscoveryError> {
    let mut core = Vec::new();
    let mut enterprise = Vec::new();
    let mut extension = Vec::new();
    let ours = EXTENSION_URN;
    let intrinsic = ["userName", "active"].into_iter().map(str::to_owned);
    let mapped = binding
        .person
        .attribute
        .iter()
        .filter_map(|attribute| attribute.scim.clone());
    for scim_path in intrinsic.chain(mapped) {
        let Ok(parsed) = AttrPath::parse(&without_value_filter(&scim_path)) else {
            return Err(DiscoveryError::UnknownAttribute {
                schema: urn::CORE_USER.to_owned(),
                attribute: scim_path,
            });
        };
        match parsed.schema.as_deref() {
            Some(schema) if urn::same(schema, urn::ENTERPRISE_USER) => enterprise.push(parsed),
            Some(schema) if urn::same(schema, ours) => {
                extension.push(parsed.name.as_str().to_owned());
            }
            _ => core.push(parsed),
        }
    }
    let mut extensions = vec![ExtensionConfig {
        schema: SchemaSelection::all(ferrohealth_user(&extension)),
        required: false,
    }];
    if !enterprise.is_empty() {
        extensions.push(ExtensionConfig {
            schema: SchemaSelection::only(catalogue::enterprise_user(), enterprise),
            required: false,
        });
    }
    Discovery::build(DiscoveryConfig {
        base_url: scim.base_url().unwrap_or_default(),
        documentation_uri: None,
        // TODO(#5014): declare PATCH and entity tags once /Users serves them.
        patch: false,
        etag: false,
        filter: FilterConfig {
            supported: true,
            max_results: scim.max_results,
        },
        authentication_schemes: vec![AuthenticationScheme {
            primary: Some(true),
            ..AuthenticationScheme::oauth_bearer_token(None)
        }],
        resource_types: vec![ResourceTypeConfig {
            name: "User".to_owned(),
            endpoint: "/Users".to_owned(),
            description: Some(
                "A health professional, held as a demographic PERSON performing the \
                 professional ROLE"
                    .to_owned(),
            ),
            schema: SchemaSelection::only(catalogue::core_user(), core),
            extensions,
        }],
    })
}

/// `scim_path` with the RFC 7644 §3.4.2.2 `valuePath` filter of its
/// attribute removed: `emails[type eq "work"].value` reads `emails.value`.
fn without_value_filter(scim_path: &str) -> String {
    match (scim_path.find('['), scim_path.rfind(']')) {
        (Some(open), Some(close)) if open < close => {
            let head = scim_path.get(..open).unwrap_or_default();
            let tail = scim_path.get(close.saturating_add(1)..).unwrap_or_default();
            format!("{head}{tail}")
        }
        _ => scim_path.to_owned(),
    }
}

/// FerroEHR's User extension: the binding key, the lifecycle state and the
/// missing paths of an incomplete User, and each of `mapped`, another
/// extension attribute the mapping names.
fn ferrohealth_user(mapped: &[String]) -> Schema {
    let mut attributes = vec![
        AttributeDefinition {
            mutability: Mutability::Immutable,
            uniqueness: Some(Uniqueness::Server),
            ..string(
                LOGIN_IDENTIFIER,
                "The login identifier the deployment binds the professional's login by; set at \
                 creation and never changed (RFC 7643 §2.2).",
            )
        },
        AttributeDefinition {
            mutability: Mutability::ReadOnly,
            ..string(
                LIFECYCLE_STATE,
                "The openEHR lifecycle state of the User's PERSON when it is not complete.",
            )
        },
        AttributeDefinition {
            mutability: Mutability::ReadOnly,
            multi_valued: true,
            ..string(
                MISSING_PATHS,
                "The template paths an incomplete User's PERSON lacks.",
            )
        },
    ];
    for name in mapped {
        if !attributes.iter().any(|a| a.name.eq_ignore_ascii_case(name)) {
            attributes.push(string(name, "An attribute the deployment's mapping maps."));
        }
    }
    Schema {
        schemas: None,
        id: EXTENSION_URN.to_owned(),
        name: Some("FerroHEALTHUser".to_owned()),
        description: Some(
            "FerroEHR's User extension: the binding key and the lifecycle of the PERSON."
                .to_owned(),
        ),
        attributes,
        meta: None,
    }
}

/// A single-valued, optional, read-write string attribute, returned by
/// default, compared without regard to case (RFC 7643 §2.2 defaults).
fn string(name: &str, description: &str) -> AttributeDefinition {
    AttributeDefinition {
        name: name.to_owned(),
        kind: AttributeType::String,
        sub_attributes: None,
        multi_valued: false,
        description: description.to_owned(),
        required: false,
        canonical_values: None,
        case_exact: Some(false),
        mutability: Mutability::ReadWrite,
        returned: Returned::Default,
        uniqueness: Some(Uniqueness::None),
        reference_types: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroehr::config::binding::person::PersonAttribute;

    fn mapped(paths: &[&str]) -> PartyBindingConfig {
        let mut binding = PartyBindingConfig::default();
        binding.person.attribute = paths
            .iter()
            .map(|scim| PersonAttribute {
                scim: Some((*scim).to_owned()),
                ..PersonAttribute::default()
            })
            .collect();
        binding
    }

    fn enabled() -> ScimConfig {
        ScimConfig {
            enabled: true,
            public_base_url: Some("https://cdr.example.com".to_owned()),
            ..ScimConfig::default()
        }
    }

    #[test]
    fn the_user_schema_carries_exactly_the_mapped_attributes() {
        let binding = mapped(&[
            "name.givenName",
            "name.familyName",
            "emails[type eq \"work\"].value",
            "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier",
            "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:bigNumber",
        ]);
        let discovery = discovery(&enabled(), &binding).expect("the documents build");
        let user = discovery.schema(urn::CORE_USER).expect("the User schema");
        let names: Vec<&str> = user.attributes.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["userName", "name", "active", "emails"]);
        let ours = discovery.schema(EXTENSION_URN).expect("the extension");
        let names: Vec<&str> = ours.attributes.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                LOGIN_IDENTIFIER,
                LIFECYCLE_STATE,
                MISSING_PATHS,
                "bigNumber"
            ]
        );
        assert!(discovery.schema(urn::ENTERPRISE_USER).is_none());
        assert!(!discovery.service_provider_config().patch.supported);
    }

    #[test]
    fn the_extension_urn_has_the_custom_form() {
        assert_eq!(
            EXTENSION_URN,
            crate::scim::protocol::deviation::custom_extension_urn("ferrohealth", "User")
        );
    }

    #[test]
    fn an_enterprise_attribute_brings_its_extension() {
        let binding =
            mapped(&["urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:employeeNumber"]);
        let discovery = discovery(&enabled(), &binding).expect("the documents build");
        let enterprise = discovery
            .schema(urn::ENTERPRISE_USER)
            .expect("the Enterprise User extension");
        assert_eq!(enterprise.attributes.len(), 1);
    }

    #[test]
    fn an_unknown_core_attribute_is_refused() {
        let binding = mapped(&["favouriteColour"]);
        assert!(discovery(&enabled(), &binding).is_err());
    }
}
