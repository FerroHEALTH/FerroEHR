// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Microsoft Entra ID exaple client: every request shape its vendored
//! documents print, against the same generic provider the RFC suite uses, and
//! each deviation from RFC 7643 and RFC 7644 the provider tolerates, asserted
//! as reported by the layer.

use ferroehr_rest::scim::protocol::deviation::{self, Deviation};
use ferroehr_rest::scim::protocol::discovery::{
    AuthenticationScheme, Discovery, DiscoveryConfig, ExtensionConfig, FilterConfig,
    ResourceTypeConfig, SchemaSelection,
};
use ferroehr_rest::scim::protocol::filter::{self, FilterSupport, Logic, Term};
use ferroehr_rest::scim::protocol::list::{ListResponse, Projection};
use ferroehr_rest::scim::protocol::patch::{PatchOp, PatchRequest};
use ferroehr_rest::scim::protocol::path::AttrPath;
use ferroehr_rest::scim::protocol::resource::{Resource, ResourceSchema};
use ferroehr_rest::scim::protocol::schema::{
    AttributeDefinition, AttributeType, Mutability, Returned, Schema, Uniqueness, catalogue,
};
use ferroehr_rest::scim::protocol::urn;
use serde_json::{Value, json};

use super::source::{self, Doc, Fix};
use super::{group_schema, held, user_schema};

/// A user as the Entra client created it (its Create User request), held
/// under the id the vendored response assigns.
fn provisioned_user() -> Resource {
    let mut body = source::json(Doc::EntraEndpoint, 282..=303, &[]);
    body["id"] = json!("48af03ac28ad4fb88478");
    held(&body, &user_schema())
}

fn applied(body: &Value, resource: &Resource, schema: &ResourceSchema) -> (Value, Vec<Deviation>) {
    let request = PatchRequest::parse(body).unwrap_or_else(|error| panic!("{error}: {body}"));
    let outcome = request
        .apply(resource, schema)
        .unwrap_or_else(|error| panic!("{error}: {body}"));
    (outcome.resource().to_value(), request.deviations().to_vec())
}

/// Reads the filter of a markdown request line such as `*GET /Users?filter=x*`.
fn query_filter(line: &str) -> String {
    let (_, query) = line.split_once("filter=").expect("a filter query");
    query
        .trim_end_matches('*')
        .trim_end_matches(" HTTP/1.1")
        .to_owned()
}

#[test]
fn every_deviation_names_its_vendored_source() {
    for deviation in Deviation::ALL {
        let source = deviation.source();
        assert!(
            source.starts_with("docs/specs/microsoft-entra/"),
            "{deviation:?}: {source}"
        );
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(source.split_once(", ").map_or(source, |(p, _)| p));
        assert!(path.is_file(), "{deviation:?} cites {source}");
        assert!(!deviation.departs_from().is_empty(), "{deviation:?}");
    }
}

#[test]
fn the_create_user_request_is_accepted_with_its_readonly_meta_ignored() {
    let body = source::json(Doc::EntraEndpoint, 282..=303, &[]);
    let parsed = Resource::from_request(&body, &user_schema())
        .expect("the Entra Create User request is valid RFC 7644");
    assert_eq!(parsed.deviations(), []);
    let user = parsed.into_resource().to_value();
    assert_eq!(
        user["userName"],
        "Test_User_00aa00aa-bb11-cc22-dd33-44ee44ee44ee"
    );
    assert!(user.get("meta").is_none());
    assert!(
        user.get("roles").is_none(),
        "an empty roles list asserts nothing"
    );
    assert_eq!(
        user["schemas"],
        json!([urn::CORE_USER]),
        "no enterprise attribute is present"
    );
}

#[test]
fn the_reference_code_create_request_is_accepted_as_printed() {
    // The reference-code walkthrough sends nulls (unassigned, RFC 7643 §2.5)
    // and a misspelt enterprise URN that carries no attributes.
    let printed = source::lines(Doc::EntraEndpoint, 1229..=1253).join("\n");
    let body: Value = serde_json::from_str(&printed).expect("the walkthrough body is JSON");
    let parsed = Resource::from_request(&body, &user_schema()).expect("accepted");
    assert_eq!(parsed.deviations(), [Deviation::UnlistedSchemaUri]);
    let user = parsed.into_resource().to_value();
    assert_eq!(user["displayName"], "Joy Young");
    assert!(user.get("addresses").is_none());
    assert!(user.get("manager").is_none());
}

#[test]
fn the_query_filters_parse_and_pass_the_ferroehr_subset() {
    let support =
        FilterSupport::equality_conjunction(urn::CORE_USER, &["userName", "externalId", "id"])
            .expect("valid attribute names");
    for number in [405, 445] {
        let text = query_filter(&source::line(Doc::EntraEndpoint, number));
        support
            .check(&filter::parse(&text).expect("parses"))
            .unwrap_or_else(|e| panic!("{text}: {e}"));
    }
    // The Test Connection probe: "a user that doesn't exist, using a random
    // GUID as the matching property".
    let probe =
        filter::parse("externalId eq \"7b47a0f5-0f39-4a5e-b2a3-8a0a0c3a1f22\"").expect("parses");
    support.check(&probe).expect("admitted");
    let empty = serde_json::to_value(ListResponse::<Value>::new(vec![], 0, 1)).expect("serializes");
    let expected = source::json(Doc::EntraEndpoint, 451..=457, &[]);
    for key in ["schemas", "totalResults", "Resources", "startIndex"] {
        assert_eq!(empty[key], expected[key], "{key}");
    }
}

#[test]
fn the_group_query_filter_and_excluded_members_parse() {
    let line = source::line(Doc::EntraEndpoint, 748);
    let text = query_filter(&line);
    let group_support =
        FilterSupport::equality_conjunction(urn::CORE_GROUP, &["displayName", "externalId", "id"])
            .expect("valid attribute names");
    group_support
        .check(&filter::parse(&text).expect("parses"))
        .expect("admitted");
    let group = held(
        &source::json(Doc::Rfc7643, 2364..=2390, &[]),
        &group_schema(),
    );
    let projection = Projection::parse(None, Some("members")).expect("parses");
    assert!(
        projection
            .apply(&group, &group_schema())
            .get("members")
            .is_none()
    );
}

#[test]
fn the_value_path_sub_attribute_filter_is_read_as_a_value_filter() {
    // Microsoft asks that `emails[type eq "work"].value eq "..."` be usable as
    // a filter; Figure 1 has no `subAttr` after a `valuePath`.
    let line = source::line(Doc::EntraEndpoint, 207);
    let start = line.find("*emails[").expect("the example filter") + 1;
    let text = &line[start..start + line[start..].find('*').expect("closing emphasis")];
    let parsed = filter::parse(text).expect("tolerated");
    assert_eq!(
        parsed,
        filter::parse("emails[type eq \"work\" and value eq \"user@contoso.com\"]")
            .expect("parses")
    );
    assert!(matches!(parsed, Logic::Leaf(Term::ValuePath(..))));
    assert!(filter::parse("emails[type eq \"work\"].value pr").is_ok());
}

#[test]
fn a_capitalised_op_is_read_as_its_lower_case_operation() {
    let body = source::json(Doc::EntraEndpoint, 466..=480, &[]);
    let request = PatchRequest::parse(&body).expect("parses");
    assert_eq!(
        request
            .operations()
            .iter()
            .map(|o| o.op)
            .collect::<Vec<_>>(),
        [PatchOp::Replace, PatchOp::Replace]
    );
    assert_eq!(
        request.deviations(),
        [Deviation::OperationCase, Deviation::OperationCase]
    );
    let (user, _) = applied(&body, &provisioned_user(), &user_schema());
    let response = source::json(Doc::EntraEndpoint, 487..=508, &[]);
    assert_eq!(user["emails"], response["emails"]);
    assert_eq!(user["name"]["familyName"], response["name"]["familyName"]);
    let lower = json!({"schemas": [urn::PATCH_OP], "Operations": [{"op": "replace", "path": "title", "value": "x"}]});
    assert!(
        PatchRequest::parse(&lower)
            .expect("parses")
            .deviations()
            .is_empty(),
        "RFC 7644 spells op in lower case"
    );
}

#[test]
fn the_single_valued_and_disable_updates_apply() {
    let body = source::json(Doc::EntraEndpoint, 517..=524, &[]);
    let (user, _) = applied(&body, &provisioned_user(), &user_schema());
    assert_eq!(
        user["userName"],
        "5b50642d-79fc-4410-9e90-4c077cdd1a59@testuser.com"
    );
    let disable = source::json(Doc::EntraEndpoint, 589..=600, &[]);
    let (user, deviations) = applied(&disable, &provisioned_user(), &user_schema());
    assert_eq!(user["active"], false);
    assert_eq!(deviations, [Deviation::OperationCase]);
}

#[test]
fn a_replace_without_a_path_in_entra_case_applies() {
    let body = json!({"schemas": [urn::PATCH_OP], "Operations": [{"op": "Replace", "value": {"displayName": "Babs", "title": "Tour Guide"}}]});
    let (user, deviations) = applied(&body, &provisioned_user(), &user_schema());
    assert_eq!(
        (user["displayName"].clone(), user["title"].clone()),
        (json!("Babs"), json!("Tour Guide"))
    );
    assert_eq!(deviations, [Deviation::OperationCase]);
}

#[test]
fn the_create_group_request_tolerates_its_unlisted_schema() {
    let body = source::json(Doc::EntraEndpoint, 673..=680, &[]);
    let parsed = Resource::from_request(&body, &group_schema()).expect("accepted");
    assert_eq!(parsed.deviations(), [Deviation::UnlistedSchemaUri]);
    let group = parsed.into_resource().to_value();
    assert_eq!(group["schemas"], json!([urn::CORE_GROUP]));
    assert_eq!(group["displayName"], "displayName");
}

#[test]
fn group_updates_add_and_remove_members() {
    let group = held(
        &source::json(Doc::Rfc7643, 2364..=2390, &[]),
        &group_schema(),
    );
    let rename = source::json(Doc::EntraEndpoint, 782..=789, &[]);
    let (renamed, _) = applied(&rename, &group, &group_schema());
    assert_eq!(
        renamed["displayName"],
        "1879db59-3bdf-4490-ad68-ab880a269474updatedDisplayName"
    );

    let add = source::json(Doc::EntraEndpoint, 815..=825, &[]);
    let (added, _) = applied(&add, &group, &group_schema());
    let members = added["members"].as_array().expect("members");
    assert_eq!(members.len(), 3);
    assert_eq!(
        members[2],
        json!({"value": "f648f8d5ea4e4cd38e9c"}),
        "the null $ref is unassigned (RFC 7643 §2.5)"
    );

    let remove = source::json(Doc::EntraEndpoint, 849..=859, &[]);
    let request = PatchRequest::parse(&remove).expect("parses");
    assert_eq!(
        request.deviations(),
        [Deviation::OperationCase, Deviation::RemoveValueList]
    );
    let outcome = request
        .apply(&held(&added, &group_schema()), &group_schema())
        .expect("applies");
    assert_eq!(
        outcome.resource().to_value()["members"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "only the listed member is removed"
    );
    assert_eq!(
        outcome.resource().to_value()["members"],
        json!([
            {"value": "2819c223-7f76-453a-919d-413861904646", "$ref": "https://example.com/v2/Users/2819c223-7f76-453a-919d-413861904646", "display": "Babs Jensen"},
            {"value": "902c246b-6245-4190-8e05-00816be7344a", "$ref": "https://example.com/v2/Users/902c246b-6245-4190-8e05-00816be7344a", "display": "Mandy Pepperidge"}
        ])
    );
}

#[test]
fn a_remove_without_a_value_list_still_removes_every_value() {
    // RFC 7644 §3.5.2.2, for the generic client.
    let group = held(
        &source::json(Doc::Rfc7643, 2364..=2390, &[]),
        &group_schema(),
    );
    let body =
        json!({"schemas": [urn::PATCH_OP], "Operations": [{"op": "remove", "path": "members"}]});
    let (left, deviations) = applied(&body, &group, &group_schema());
    assert!(left.get("members").is_none());
    assert_eq!(deviations, []);
}

#[test]
fn the_role_requests_apply() {
    for lines in [219..=237, 365..=386] {
        Resource::from_request(
            &source::json(Doc::EntraMappings, lines.clone(), &[]),
            &user_schema(),
        )
        .unwrap_or_else(|e| panic!("{lines:?}: {e}"));
    }
    // The AppRoleAssignmentsComplex POST sends `roles[].displayName`, which
    // RFC 7643 §4.1.2 does not define; Microsoft has the operator add the
    // attribute to the application's schema first, so the RFC schema refuses it.
    let complex = source::json(Doc::EntraMappings, 288..=313, &[]);
    let error = Resource::from_request(&complex, &user_schema())
        .expect_err("displayName is not a roles sub-attribute");
    assert_eq!(
        error.scim_type(),
        Some(ferroehr_rest::scim::protocol::error::ScimType::InvalidValue)
    );
    assert!(error.detail().contains("displayName"), "{error}");
    for lines in [243..=258, 319..=337, 392..=416] {
        let body = source::json(Doc::EntraMappings, lines.clone(), &[]);
        let (user, _) = applied(&body, &provisioned_user(), &user_schema());
        assert!(
            user["roles"].as_array().is_some_and(|r| !r.is_empty()),
            "{lines:?}"
        );
    }
    // The multi-value example is a fragment (no id, no userName): its phone
    // numbers share one value under three types, which RFC 7643 §2.4 allows.
    let fragment = source::json(Doc::EntraMappings, 428..=446, &[]);
    let body = json!({"schemas": [urn::PATCH_OP], "Operations": [{"op": "replace", "path": "phoneNumbers", "value": fragment["phoneNumbers"]}]});
    let (user, _) = applied(&body, &provisioned_user(), &user_schema());
    assert_eq!(user["phoneNumbers"], fragment["phoneNumbers"]);
}

/// The provider with the custom extension `CustomExtensionName` Microsoft's
/// mapping article uses, holding one string attribute.
fn custom_extension_provider() -> Discovery {
    let urn = deviation::custom_extension_urn("CustomExtensionName", "User");
    let attribute = AttributeDefinition {
        name: "CustomAttribute".to_owned(),
        kind: AttributeType::String,
        sub_attributes: None,
        multi_valued: false,
        description: "A custom attribute.".to_owned(),
        required: false,
        canonical_values: None,
        case_exact: Some(false),
        mutability: Mutability::ReadWrite,
        returned: Returned::Default,
        uniqueness: Some(Uniqueness::None),
        reference_types: None,
    };
    let schema = Schema {
        schemas: None,
        id: urn,
        name: Some("CustomExtension".to_owned()),
        description: None,
        attributes: vec![attribute],
        meta: None,
    };
    Discovery::build(DiscoveryConfig {
        base_url: super::BASE_URL.to_owned(),
        documentation_uri: None,
        patch: true,
        etag: true,
        filter: FilterConfig {
            supported: true,
            max_results: 200,
        },
        authentication_schemes: vec![AuthenticationScheme::oauth_bearer_token(None)],
        resource_types: vec![ResourceTypeConfig {
            name: "User".to_owned(),
            endpoint: "/Users".to_owned(),
            description: Some("User Account".to_owned()),
            schema: SchemaSelection::all(catalogue::core_user()),
            extensions: vec![
                ExtensionConfig {
                    schema: SchemaSelection::all(catalogue::enterprise_user()),
                    required: false,
                },
                ExtensionConfig {
                    schema: SchemaSelection::all(schema),
                    required: false,
                },
            ],
        }],
    })
    .expect("a custom extension in the Entra form builds")
}

#[test]
fn the_custom_extension_urn_has_the_form_microsoft_documents() {
    assert_eq!(
        deviation::custom_extension_urn("ferrohealth", "User"),
        "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User"
    );
    let provider = custom_extension_provider();
    assert_eq!(provider.deviations(), [Deviation::UnregisteredExtensionUrn]);
    let schema = provider.resource_schema("User").expect("User");
    let path = AttrPath::parse(
        "urn:ietf:params:scim:schemas:extension:CustomExtensionName:2.0:User:CustomAttribute",
    )
    .expect("a path");
    assert!(
        schema.resolve(&path).is_some(),
        "the qualified attribute name resolves"
    );
}

#[test]
fn an_extension_container_without_its_schemas_entry_is_accepted() {
    // Microsoft: "The custom extension schema header is omitted in the example
    // because it isn't sent in requests from the Microsoft Entra SCIM client."
    let printed = source::json(Doc::EntraMappings, 157..=192, &[Fix::TrailingCommas]);
    let provider = custom_extension_provider();
    let schema = provider.resource_schema("User").expect("User");
    let parsed = Resource::from_request(&printed, schema).expect("accepted");
    assert_eq!(parsed.deviations(), [Deviation::SchemasOmitExtension]);
    let user = parsed.into_resource().to_value();
    let custom = "urn:ietf:params:scim:schemas:extension:CustomExtensionName:2.0:User";
    assert_eq!(user[custom]["CustomAttribute"], "701984");
    assert!(
        user["schemas"]
            .as_array()
            .expect("schemas")
            .iter()
            .any(|s| s == custom),
        "the provider lists it"
    );
    assert!(user.get("id").is_none());
}

#[test]
fn the_no_nulls_deviation_is_recorded_without_a_code_path() {
    // Microsoft: "the Microsoft Entra provisioning service doesn't send null
    // values to target systems", so a value cleared at the client never
    // reaches the provider; nothing in a request shows it.
    assert!(Deviation::ALL.contains(&Deviation::NoNulls));
    assert!(
        Deviation::NoNulls
            .source()
            .contains("customize-application-attributes.md")
    );
}

#[test]
fn an_array_for_the_singular_manager_is_refused() {
    // The reference-code walkthrough (Example 5) adds the enterprise `manager`
    // as a one-element array; `manager` is singular (RFC 7643 §4.3), and RFC
    // 7644 §3.8 encodes only a multi-valued attribute as an array.
    let printed = source::lines(Doc::EntraEndpoint, 1322..=1335).join("\n");
    let body: Value = serde_json::from_str(&printed).expect("the walkthrough body is JSON");
    let request = PatchRequest::parse(&body).expect("parses");
    let error = request
        .apply(&provisioned_user(), &user_schema())
        .expect_err("refused");
    assert_eq!(
        error.scim_type(),
        Some(ferroehr_rest::scim::protocol::error::ScimType::InvalidValue)
    );
    let object = json!({"schemas": [urn::PATCH_OP], "Operations": [{"op": "Add", "path": "manager",
        "value": {"$ref": "http://example.com/scim/Users/00aa00aa", "value": "00aa00aa"}}]});
    let (user, _) = applied(&object, &provisioned_user(), &user_schema());
    assert_eq!(
        user[urn::ENTERPRISE_USER]["manager"]["value"],
        "00aa00aa",
        "RFC 7644 §3.10: an unqualified extension name resolves"
    );
}
