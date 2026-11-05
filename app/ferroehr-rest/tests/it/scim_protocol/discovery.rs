// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The discovery documents of RFC 7644 §4 built from configuration:
//! `ServiceProviderConfig` (RFC 7643 §5), `ResourceTypes` (§6) and `Schemas`
//! (§7), against the RFC 7643 §8.5 and §8.6 and RFC 7644 Figure 9 examples.

use ferroehr_rest::scim::protocol::discovery::{
    self, AuthenticationScheme, Discovery, DiscoveryConfig, DiscoveryError, ExtensionConfig,
    FilterConfig, ResourceType, ResourceTypeConfig, SchemaSelection, ServiceProviderConfig,
};
use ferroehr_rest::scim::protocol::path::AttrPath;
use ferroehr_rest::scim::protocol::schema::catalogue;
use http::StatusCode;
use serde_json::{Value, json};

use super::source::{self, Doc};
use super::{BASE_URL, rfc_provider};

fn config(resource_types: Vec<ResourceTypeConfig>) -> DiscoveryConfig {
    DiscoveryConfig {
        base_url: BASE_URL.to_owned(),
        documentation_uri: None,
        patch: true,
        etag: true,
        filter: FilterConfig {
            supported: true,
            max_results: 200,
        },
        authentication_schemes: vec![AuthenticationScheme::oauth_bearer_token(None)],
        resource_types,
    }
}

fn user_with(schema: SchemaSelection, extensions: Vec<ExtensionConfig>) -> ResourceTypeConfig {
    ResourceTypeConfig {
        name: "User".to_owned(),
        endpoint: "/Users".to_owned(),
        description: Some("User Account".to_owned()),
        schema,
        extensions,
    }
}

fn paths(names: &[&str]) -> Vec<AttrPath> {
    names
        .iter()
        .map(|n| AttrPath::parse(n).expect("a path"))
        .collect()
}

fn has_null(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.iter().any(has_null),
        Value::Object(members) => members.values().any(has_null),
        _ => false,
    }
}

#[test]
fn the_service_provider_config_declares_what_the_provider_does() {
    // #5014: patch and etag supported, filter with maxResults, bulk, sort and
    // changePassword unsupported, and the OAuth bearer token scheme.
    let document =
        serde_json::to_value(rfc_provider().service_provider_config()).expect("serializes");
    assert_eq!(
        document,
        json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
            "patch": {"supported": true},
            "bulk": {"supported": false, "maxOperations": 0, "maxPayloadSize": 0},
            "filter": {"supported": true, "maxResults": 200},
            "changePassword": {"supported": false},
            "sort": {"supported": false},
            "etag": {"supported": true},
            "authenticationSchemes": [{
                "type": "oauthbearertoken",
                "name": "OAuth Bearer Token",
                "description": "Authentication scheme using the OAuth Bearer Token Standard",
                "specUri": "http://www.rfc-editor.org/info/rfc6750"
            }],
            "meta": {"resourceType": "ServiceProviderConfig", "location": "https://example.com/v2/ServiceProviderConfig"}
        })
    );
}

#[test]
fn the_service_provider_config_example_reads_back_unchanged() {
    let printed = source::json(Doc::Rfc7643, 2420..=2487, &[]);
    let config: ServiceProviderConfig =
        serde_json::from_value(printed.clone()).expect("reads §8.5");
    assert_eq!(serde_json::to_value(&config).expect("serializes"), printed);
}

#[test]
fn the_resource_types_are_the_section_8_6_example() {
    let printed = source::json(Doc::Rfc7643, 2532..=2562, &[]);
    let provider = rfc_provider();
    let ours: Vec<&ResourceType> = provider.resource_types().resources;
    assert_eq!(serde_json::to_value(&ours).expect("serializes"), printed);
}

#[test]
fn the_resource_types_list_is_figure_9() {
    let printed = source::json(Doc::Rfc7644, 4154..=4188, &[]);
    let ours = serde_json::to_value(rfc_provider().resource_types()).expect("serializes");
    assert_eq!(ours["Resources"], printed["Resources"]);
    assert_eq!(ours["totalResults"], printed["totalResults"]);
    assert_eq!(ours["schemas"], printed["schemas"]);
    assert_eq!(
        rfc_provider().resource_type("User").expect("User").name,
        "User"
    );
    assert!(rfc_provider().resource_type("Device").is_none());
}

#[test]
fn the_schemas_list_carries_each_schema_once_with_its_location() {
    let schemas = serde_json::to_value(rfc_provider().schemas()).expect("serializes");
    let ids: Vec<&str> = schemas["Resources"]
        .as_array()
        .expect("resources")
        .iter()
        .map(|s| s["id"].as_str().expect("id"))
        .collect();
    assert_eq!(
        ids,
        [
            "urn:ietf:params:scim:schemas:core:2.0:User",
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User",
            "urn:ietf:params:scim:schemas:core:2.0:Group",
        ]
    );
    let user = &schemas["Resources"][0];
    assert_eq!(
        user["schemas"],
        json!(["urn:ietf:params:scim:schemas:core:2.0:Schema"])
    );
    assert_eq!(
        user["meta"],
        json!({"resourceType": "Schema", "location": "https://example.com/v2/Schemas/urn:ietf:params:scim:schemas:core:2.0:User"})
    );
    let one = rfc_provider();
    let found = one
        .schema("URN:IETF:PARAMS:SCIM:SCHEMAS:CORE:2.0:GROUP")
        .expect("schema ids compare case-insensitively");
    assert_eq!(found.id, "urn:ietf:params:scim:schemas:core:2.0:Group");
}

#[test]
fn no_discovery_document_writes_a_null() {
    // Microsoft, /Schemas: "If a value isn't present, don't send null values."
    let provider = rfc_provider();
    for document in [
        serde_json::to_value(provider.service_provider_config()).expect("serializes"),
        serde_json::to_value(provider.resource_types()).expect("serializes"),
        serde_json::to_value(provider.schemas()).expect("serializes"),
    ] {
        assert!(!has_null(&document), "{document}");
    }
}

#[test]
fn a_selection_advertises_exactly_the_selected_attributes() {
    let selection = SchemaSelection::only(
        catalogue::core_user(),
        paths(&[
            "userName",
            "name.givenName",
            "name.familyName",
            "emails",
            "active",
        ]),
    );
    let provider = Discovery::build(config(vec![user_with(selection, vec![])])).expect("builds");
    let user = provider
        .schema("urn:ietf:params:scim:schemas:core:2.0:User")
        .expect("User");
    let names: Vec<&str> = user.attributes.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        ["userName", "name", "active", "emails"],
        "the catalogue order is kept"
    );
    let name = user
        .attributes
        .iter()
        .find(|a| a.name == "name")
        .expect("name");
    let subs: Vec<&str> = name
        .sub_attributes
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(subs, ["familyName", "givenName"]);
    assert!(
        provider
            .resource_schema("User")
            .expect("User")
            .resolve(&AttrPath::parse("title").expect("a path"))
            .is_none()
    );
}

#[test]
fn a_configuration_the_rfcs_forbid_is_refused() {
    for (config, expected) in [
        (
            config(vec![user_with(
                SchemaSelection::only(catalogue::core_user(), paths(&["shoeSize"])),
                vec![],
            )]),
            "shoeSize",
        ),
        (
            config(vec![user_with(
                SchemaSelection::only(catalogue::core_user(), paths(&["name"])),
                vec![],
            )]),
            "userName",
        ),
        (
            config(vec![
                user_with(SchemaSelection::all(catalogue::core_user()), vec![]),
                user_with(SchemaSelection::all(catalogue::core_user()), vec![]),
            ]),
            "User",
        ),
        (
            DiscoveryConfig {
                base_url: "/relative".to_owned(),
                ..config(vec![])
            },
            "/relative",
        ),
    ] {
        let error: DiscoveryError = Discovery::build(config).expect_err("refused");
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn an_extension_in_the_registered_namespace_must_be_registered_or_follow_the_extension_form() {
    let mut bad = catalogue::enterprise_user();
    bad.id = "urn:ietf:params:scim:schemas:core:2.0:Badge".to_owned();
    let refused = Discovery::build(config(vec![user_with(
        SchemaSelection::all(catalogue::core_user()),
        vec![ExtensionConfig {
            schema: SchemaSelection::all(bad),
            required: false,
        }],
    )]));
    assert!(
        refused
            .expect_err("an unregistered core URN is refused")
            .to_string()
            .contains("Badge")
    );
}

#[test]
fn a_filter_on_a_discovery_endpoint_is_forbidden() {
    // RFC 7644 §4: "If a "filter" is provided, the service provider SHOULD
    // respond with HTTP status code 403 (Forbidden)".
    let refusal = discovery::filter_refusal(Some("name eq \"User\"")).expect("a filter is refused");
    assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
    assert!(
        discovery::filter_refusal(None).is_none(),
        "a query without a filter is served"
    );
}
