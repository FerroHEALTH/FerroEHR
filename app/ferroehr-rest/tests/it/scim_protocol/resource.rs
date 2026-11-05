// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Resource bodies against their schema: the create (RFC 7644 §3.3) and
//! replace (§3.5.1) rules, the RFC 7643 §8 representations, the `ListResponse`
//! (§3.4.2), the `SearchRequest` (§3.4.3), pagination (§3.4.2.4) and the
//! `attributes` and `excludedAttributes` parameters (§3.4.2.5, §3.9).

use ferroehr_rest::scim::protocol::error::ScimType;
use ferroehr_rest::scim::protocol::list::{ListResponse, Page, Projection, SearchRequest};
use ferroehr_rest::scim::protocol::path::AttrPath;
use ferroehr_rest::scim::protocol::resource::Resource;
use ferroehr_rest::scim::protocol::urn;
use serde_json::{Value, json};

use super::filter::unwrap_certificate;
use super::source::{self, Doc, Fix};
use super::{group_schema, held, user_schema};

fn created(body: &Value) -> Resource {
    Resource::from_request(body, &user_schema())
        .unwrap_or_else(|error| panic!("{error}: {body}"))
        .into_resource()
}

fn refused(body: &Value) -> ScimType {
    let error =
        Resource::from_request(body, &user_schema()).expect_err("the body should be refused");
    assert_eq!(error.status(), http::StatusCode::BAD_REQUEST, "{error}");
    error.scim_type().expect("a 400 carries its scimType")
}

#[test]
fn the_create_example_is_accepted_as_asserted() {
    let body = source::json(Doc::Rfc7644, 639..=648, &[]);
    let user = created(&body).to_value();
    assert_eq!(user["userName"], "bjensen");
    assert_eq!(user["externalId"], "bjensen");
    assert_eq!(user["name"]["familyName"], "Jensen");
    assert_eq!(user["schemas"], json!([urn::CORE_USER]));
    assert!(
        user.get("id").is_none(),
        "the provider assigns id (RFC 7643 §3.1)"
    );
}

#[test]
fn readonly_attributes_of_a_create_are_ignored() {
    // RFC 7644 §3.3: "attributes whose mutability is "readOnly" ... SHALL be ignored".
    let response = source::json(Doc::Rfc7644, 689..=707, &[]);
    let user = created(&response).to_value();
    assert!(user.get("id").is_none());
    assert!(user.get("meta").is_none());
    assert_eq!(user["userName"], "bjensen");
}

#[test]
fn every_representation_example_is_valid_against_its_schema() {
    let user = user_schema();
    for (doc, lines) in [
        (Doc::Rfc7644, 689..=707),
        (Doc::Rfc7644, 760..=804),
        (Doc::Rfc7644, 1751..=1778),
        (Doc::Rfc7644, 4011..=4028),
        (Doc::Rfc7643, 1862..=1874),
    ] {
        held(&source::json(doc, lines, &[]), &user);
    }
    for lines in [1916..=2110, 2140..=2323] {
        let mut value = source::json(Doc::Rfc7643, lines, &[]);
        unwrap_certificate(&mut value);
        held(&value, &user);
    }
    held(
        &source::json(Doc::Rfc7643, 2364..=2390, &[]),
        &group_schema(),
    );
}

#[test]
fn a_certificate_wrapped_as_printed_is_not_base64() {
    // RFC 7643 §2.3.6: binary is base64 "as specified in Section 4 of
    // [RFC4648]", which admits no whitespace inside the encoding.
    let printed = source::json(Doc::Rfc7643, 1916..=2110, &[]);
    let error = Resource::from_representation(&printed, &user_schema())
        .expect_err("whitespace is not base64");
    assert_eq!(error.scim_type(), Some(ScimType::InvalidValue));
}

#[test]
fn a_put_replaces_the_asserted_attributes() {
    // RFC 7644 §3.5.1: the PUT example against the §3.4.1 User it retrieved.
    let existing = held(&source::json(Doc::Rfc7644, 760..=804, &[]), &user_schema());
    let request = source::json(Doc::Rfc7644, 1699..=1719, &[]);
    let replaced = existing
        .replace(&request, &user_schema(), &[])
        .expect("replaces")
        .into_resource()
        .to_value();
    let response = source::json(Doc::Rfc7644, 1751..=1778, &[]);
    for attribute in ["userName", "externalId", "name", "emails"] {
        assert_eq!(replaced[attribute], response[attribute], "{attribute}");
    }
    assert!(
        replaced.get("phoneNumbers").is_none(),
        "an omitted readWrite attribute is cleared"
    );
    assert!(
        replaced.get("roles").is_none(),
        "an empty array clears the attribute (RFC 7643 §2.5)"
    );
    assert_eq!(
        replaced["id"], "2819c223-7f76-453a-919d-413861904646",
        "readOnly values come from the resource"
    );
    assert_eq!(replaced["meta"], existing.to_value()["meta"]);
}

#[test]
fn a_put_keeps_what_the_caller_keeps_on_omission() {
    let existing = held(&source::json(Doc::Rfc7644, 760..=804, &[]), &user_schema());
    let request = json!({"schemas": [urn::CORE_USER], "userName": "bjensen"});
    let keep = [
        AttrPath::parse("externalId").expect("a path"),
        AttrPath::parse("phoneNumbers").expect("a path"),
    ];
    let replaced = existing
        .replace(&request, &user_schema(), &keep)
        .expect("replaces")
        .into_resource()
        .to_value();
    assert_eq!(replaced["externalId"], "bjensen");
    assert_eq!(
        replaced["phoneNumbers"],
        existing.to_value()["phoneNumbers"]
    );
    assert!(replaced.get("emails").is_none());
}

#[test]
fn the_structure_of_a_body_is_invalid_syntax() {
    for body in [
        json!("a string"),
        json!({"userName": "x"}),
        json!({"schemas": "urn:ietf:params:scim:schemas:core:2.0:User", "userName": "x"}),
        json!({"schemas": [], "userName": "x"}),
        json!({"schemas": [urn::ENTERPRISE_USER], "userName": "x"}),
        json!({"schemas": [urn::CORE_USER, urn::CORE_USER], "userName": "x"}),
        json!({"schemas": [urn::CORE_USER], "userName": "x", "USERNAME": "y"}),
    ] {
        assert_eq!(refused(&body), ScimType::InvalidSyntax, "{body}");
    }
}

#[test]
fn values_the_schema_does_not_admit_are_invalid_value() {
    for body in [
        json!({"schemas": [urn::CORE_USER]}),
        json!({"schemas": [urn::CORE_USER], "userName": 5}),
        json!({"schemas": [urn::CORE_USER], "userName": "x", "active": "yes"}),
        json!({"schemas": [urn::CORE_USER], "userName": "x", "emails": {"value": "a@b"}}),
        json!({"schemas": [urn::CORE_USER], "userName": "x", "emails": [{"value": "a@b", "label": "w"}]}),
        json!({"schemas": [urn::CORE_USER], "userName": "x", "emails": [{"value": "a@b", "primary": true}, {"value": "c@d", "primary": true}]}),
        json!({"schemas": [urn::CORE_USER], "userName": "x", "name": "Barbara"}),
        json!({"schemas": [urn::CORE_USER], "userName": "x", "shoeSize": 42}),
        json!({"schemas": [urn::CORE_USER], "userName": "x", "x509Certificates": [{"value": "not base64!"}]}),
    ] {
        assert_eq!(refused(&body), ScimType::InvalidValue, "{body}");
    }
    let error = Resource::from_request(
        &json!({"schemas": [urn::CORE_USER], "userName": "x", "shoeSize": 42}),
        &user_schema(),
    )
    .expect_err("refused");
    assert!(
        error.detail().contains("shoeSize"),
        "the refusal names the attribute: {error}"
    );
}

#[test]
fn attribute_names_are_matched_case_insensitively() {
    let user =
        created(&json!({"schemas": [urn::CORE_USER], "USERNAME": "x", "Name": {"GIVENNAME": "B"}}))
            .to_value();
    assert_eq!(user["userName"], "x");
    assert_eq!(user["name"]["givenName"], "B");
}

#[test]
fn an_unassigned_value_for_an_attribute_the_provider_lacks_asserts_nothing() {
    // RFC 7643 §2.5: unassigned, null and an empty array are one state.
    let user = created(
        &json!({"schemas": [urn::CORE_USER], "userName": "x", "shoeSize": null, "hobbies": []}),
    )
    .to_value();
    assert!(user.get("shoeSize").is_none());
    assert!(user.get("hobbies").is_none());
}

#[test]
fn a_list_response_serializes_as_figure_3() {
    let page = ListResponse::new(vec![json!({"id": "a"})], 100, 1);
    let value = serde_json::to_value(&page).expect("serializes");
    assert_eq!(value["schemas"], json!([urn::LIST_RESPONSE]));
    assert_eq!(value["totalResults"], 100);
    assert_eq!(value["itemsPerPage"], 1);
    assert_eq!(value["startIndex"], 1);
    assert_eq!(value["Resources"], json!([{"id": "a"}]));
    let empty = serde_json::to_value(ListResponse::<Value>::new(vec![], 0, 1)).expect("serializes");
    assert_eq!(empty["totalResults"], 0);
    assert_eq!(empty["Resources"], json!([]));
}

#[test]
fn every_list_response_example_reads_back() {
    for (lines, fixes) in [
        (862..=875, &[][..]),
        (1419..=1427, &[Fix::Elisions][..]),
        (1583..=1600, &[Fix::Elisions, Fix::TrailingCommas][..]),
        (4154..=4188, &[][..]),
    ] {
        let value = source::json(Doc::Rfc7644, lines.clone(), fixes);
        let list: ListResponse<Value> =
            serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{lines:?}: {e}"));
        assert_eq!(
            serde_json::to_value(&list).expect("serializes")["totalResults"],
            value["totalResults"]
        );
    }
}

#[test]
fn pagination_follows_table_6() {
    let page = Page::parse(None, None, 200).expect("parses");
    assert_eq!((page.start_index(), page.count()), (1, 200));
    let page = Page::parse(Some("0"), Some("-5"), 200).expect("parses");
    assert_eq!(
        (page.start_index(), page.count()),
        (1, 0),
        "below 1 is 1, a negative count is 0"
    );
    let page = Page::parse(Some("11"), Some("10"), 200).expect("parses");
    assert_eq!(
        (page.start_index(), page.offset(), page.count()),
        (11, 10, 10)
    );
    let page = Page::parse(Some("1"), Some("5000"), 200).expect("parses");
    assert_eq!(
        page.count(),
        200,
        "the provider MAY return fewer than count"
    );
    for (start, count) in [
        (Some("one"), None),
        (None, Some("1.5")),
        (Some("99999999999999999999999"), None),
    ] {
        let error = Page::parse(start, count, 200).expect_err("refused");
        assert_eq!(error.scim_type(), Some(ScimType::InvalidValue));
    }
}

#[test]
fn the_search_request_example_parses() {
    let body = source::json(Doc::Rfc7644, 1538..=1545, &[]);
    let search = SearchRequest::parse(&body).expect("parses");
    assert_eq!(search.filter.as_deref(), Some("displayName sw \"smith\""));
    assert_eq!(
        search.attributes,
        Some(vec!["displayName".to_owned(), "userName".to_owned()])
    );
    assert_eq!((search.start_index, search.count), (Some(1), Some(10)));
    let error = SearchRequest::parse(&json!({"filter": "x pr"})).expect_err("schemas is required");
    assert_eq!(error.scim_type(), Some(ScimType::InvalidSyntax));
}

#[test]
fn the_attributes_parameter_returns_the_minimum_set_plus_the_named() {
    // RFC 7644 §3.9: the example retrieves `attributes=userName` and gets
    // the representation printed below it.
    let user = held(&source::json(Doc::Rfc7644, 760..=804, &[]), &user_schema());
    let projection = Projection::parse(Some("userName"), None).expect("parses");
    let expected = source::json(Doc::Rfc7644, 3626..=3630, &[]);
    assert_eq!(projection.apply(&user, &user_schema()), expected);

    let projection = Projection::parse(Some("name.givenName,emails"), None).expect("parses");
    let projected = projection.apply(&user, &user_schema());
    assert_eq!(projected["name"], json!({"givenName": "Barbara"}));
    assert_eq!(projected["emails"], user.to_value()["emails"]);
    assert!(projected.get("meta").is_none());
}

#[test]
fn excluded_attributes_leave_the_always_returned() {
    let group = held(
        &source::json(Doc::Rfc7643, 2364..=2390, &[]),
        &group_schema(),
    );
    let projection = Projection::parse(None, Some("members,id")).expect("parses");
    let projected = projection.apply(&group, &group_schema());
    assert!(projected.get("members").is_none());
    assert_eq!(
        projected["id"], "e9e30dba-f08f-4109-8486-d5c6a331660a",
        "id is returned always (RFC 7643 §3.1)"
    );
    assert_eq!(projected["displayName"], "Tour Guides");
}

#[test]
fn never_returned_attributes_stay_out_of_every_projection() {
    let mut full = source::json(Doc::Rfc7643, 1916..=2110, &[]);
    unwrap_certificate(&mut full);
    let user = held(&full, &user_schema());
    for projection in [
        Projection::parse(None, None),
        Projection::parse(Some("password,userName"), None),
    ] {
        let projected = projection.expect("parses").apply(&user, &user_schema());
        assert!(
            projected.get("password").is_none(),
            "RFC 7643 §4.1.1: the password SHALL NOT be returnable"
        );
    }
    let error = Projection::parse(Some("userName"), Some("emails"))
        .expect_err("the two are mutually exclusive");
    assert_eq!(error.scim_type(), Some(ScimType::InvalidValue));
}
