// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! PATCH (RFC 7644 §3.5.2): every example request of §3.5.2.1 to §3.5.2.3,
//! applied to the RFC 7643 §8 representations, and the refusals the section
//! names.

use ferroehr_rest::scim::protocol::error::ScimType;
use ferroehr_rest::scim::protocol::patch::{PatchOp, PatchRequest};
use ferroehr_rest::scim::protocol::path::PatchPath;
use ferroehr_rest::scim::protocol::resource::Resource;
use serde_json::{Value, json};

use super::filter::unwrap_certificate;
use super::source::{self, Doc, Fix};
use super::{group_schema, held, user_schema};

const BABS: &str = "2819c223-7f76-453a-919d-413861904646";
const MANDY: &str = "902c246b-6245-4190-8e05-00816be7344a";

fn group() -> Resource {
    held(
        &source::json(Doc::Rfc7643, 2364..=2390, &[]),
        &group_schema(),
    )
}

fn minimal_user() -> Resource {
    held(
        &source::json(Doc::Rfc7643, 1862..=1874, &[]),
        &user_schema(),
    )
}

fn full_user() -> Resource {
    let mut full = source::json(Doc::Rfc7643, 1916..=2110, &[]);
    unwrap_certificate(&mut full);
    held(&full, &user_schema())
}

fn member_values(resource: &Resource) -> Vec<String> {
    resource.to_value()["members"]
        .as_array()
        .map(|members| {
            members
                .iter()
                .map(|m| m["value"].as_str().expect("a member value").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn patch(body: &Value) -> PatchRequest {
    PatchRequest::parse(body).unwrap_or_else(|error| panic!("{error}: {body}"))
}

fn refused(
    body: &Value,
    resource: &Resource,
    schema: &ferroehr_rest::scim::protocol::resource::ResourceSchema,
) -> ScimType {
    let request = PatchRequest::parse(body);
    let error = match request {
        Ok(request) => request
            .apply(resource, schema)
            .expect_err("the PATCH should be refused"),
        Err(error) => error,
    };
    assert_eq!(error.status(), http::StatusCode::BAD_REQUEST, "{error}");
    error.scim_type().expect("a 400 carries its scimType")
}

#[test]
fn figure_6_adds_a_member_already_present_without_a_change() {
    // RFC 7644 §3.5.2 Figure 6, with its "... + additional operations" elided.
    let body = source::json(
        Doc::Rfc7644,
        1818..=1835,
        &[Fix::Elisions, Fix::TrailingCommas],
    );
    let request = patch(&body);
    assert_eq!(request.operations().len(), 1);
    assert_eq!(request.operations()[0].op, PatchOp::Add);
    let outcome = request.apply(&group(), &group_schema()).expect("applies");
    assert!(
        !outcome.changed(),
        "§3.5.2.1: the group already holds the value"
    );
    assert_eq!(member_values(outcome.resource()), [BABS, MANDY]);
}

#[test]
fn adding_a_member_already_present_changes_nothing() {
    // RFC 7644 §3.5.2.1: "If the user was already a member of this group, no
    // changes should be made to the resource".
    let body = source::json(Doc::Rfc7644, 2033..=2049, &[]);
    let outcome = patch(&body)
        .apply(&group(), &group_schema())
        .expect("applies");
    assert!(!outcome.changed());
    assert_eq!(member_values(outcome.resource()), [BABS, MANDY]);
}

#[test]
fn adding_without_a_path_adds_to_the_resource_itself() {
    // RFC 7644 §3.5.2.1 prints this request one closing brace short.
    let body = source::json(
        Doc::Rfc7644,
        2100..=2114,
        &[Fix::Replace(
            r#""nickname":"Babs""#,
            r#""nickname":"Babs"}"#,
        )],
    );
    let outcome = patch(&body)
        .apply(&minimal_user(), &user_schema())
        .expect("applies");
    assert!(outcome.changed());
    let user = outcome.resource().to_value();
    assert_eq!(
        user["emails"],
        json!([{"value": "babs@jensen.org", "type": "home"}])
    );
    assert_eq!(
        user["nickName"], "Babs",
        "attribute names are case-insensitive (RFC 7643 §2.1)"
    );
    assert!(user.get("nickname").is_none());

    let again = patch(&body)
        .apply(&full_user(), &user_schema())
        .expect("applies");
    assert!(!again.changed(), "the full user already holds both values");
}

#[test]
fn removing_a_member_by_value_filter() {
    let body = source::json(Doc::Rfc7644, 2201..=2208, &[]);
    let request = patch(&body);
    assert!(matches!(
        request.operations()[0].path,
        Some(PatchPath::Values { .. })
    ));
    // The printed id is elided ("2819c223-7f76-...413861904646"), so it names
    // no member: §3.5.2.2, "no changes should be made".
    let outcome = request.apply(&group(), &group_schema()).expect("applies");
    assert!(!outcome.changed());

    let full = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "remove", "path": format!("members[value eq \"{BABS}\"]")}]});
    let outcome = patch(&full)
        .apply(&group(), &group_schema())
        .expect("applies");
    assert!(outcome.changed());
    assert_eq!(member_values(outcome.resource()), [MANDY]);
}

#[test]
fn removing_every_member_leaves_the_attribute_unassigned() {
    let body = source::json(Doc::Rfc7644, 2219..=2224, &[]);
    let outcome = patch(&body)
        .apply(&group(), &group_schema())
        .expect("applies");
    assert!(outcome.changed());
    assert!(
        outcome.resource().to_value().get("members").is_none(),
        "§3.5.2.2: SHALL be considered unassigned"
    );
}

#[test]
fn removing_matching_records_of_a_complex_multi_valued_attribute() {
    let body = source::json(Doc::Rfc7644, 2229..=2236, &[]);
    let outcome = patch(&body)
        .apply(&full_user(), &user_schema())
        .expect("applies");
    assert_eq!(
        outcome.resource().to_value()["emails"],
        json!([{"value": "babs@jensen.org", "type": "home"}])
    );
}

#[test]
fn removing_and_adding_a_member_in_one_request() {
    // The remove path prints no space between `eq` and its value.
    let body = source::json(Doc::Rfc7644, 2257..=2278, &[]);
    let outcome = patch(&body)
        .apply(&group(), &group_schema())
        .expect("applies");
    assert_eq!(
        member_values(outcome.resource()),
        [BABS, MANDY, "08e1d05d...473d93df9210"]
    );
}

#[test]
fn replacing_every_member_by_remove_then_add() {
    let body = source::json(Doc::Rfc7644, 2314..=2339, &[]);
    let outcome = patch(&body)
        .apply(&group(), &group_schema())
        .expect("applies");
    assert_eq!(
        member_values(outcome.resource()),
        [BABS, "08e1d05d-121c-4561-8b96-473d93df9210"]
    );
}

#[test]
fn replacing_every_member_in_one_operation() {
    let body = source::json(Doc::Rfc7644, 2426..=2447, &[]);
    let outcome = patch(&body)
        .apply(&group(), &group_schema())
        .expect("applies");
    assert_eq!(
        member_values(outcome.resource()),
        ["2819c223...413861904646", "08e1d05d...473d93df9210"]
    );
}

#[test]
fn replacing_the_work_address_resets_the_other_primary_values() {
    let body = source::json(Doc::Rfc7644, 2483..=2502, &[]);
    let outcome = patch(&body)
        .apply(&full_user(), &user_schema())
        .expect("applies");
    let addresses = outcome.resource().to_value()["addresses"].clone();
    let work: Vec<&Value> = addresses
        .as_array()
        .expect("addresses")
        .iter()
        .filter(|a| a["type"] == "work")
        .collect();
    assert_eq!(work.len(), 1);
    assert_eq!(work[0]["streetAddress"], "911 Universal City Plaza");
    assert_eq!(work[0]["country"], "US");
    assert_eq!(work[0]["primary"], true);
    let primaries = addresses
        .as_array()
        .expect("addresses")
        .iter()
        .filter(|a| a["primary"] == true)
        .count();
    assert_eq!(
        primaries, 1,
        "RFC 7643 §2.4: the primary value true MUST appear no more than once"
    );
}

#[test]
fn replacing_a_sub_attribute_of_the_matching_records() {
    let body = source::json(Doc::Rfc7644, 2538..=2546, &[]);
    let outcome = patch(&body)
        .apply(&full_user(), &user_schema())
        .expect("applies");
    let addresses = outcome.resource().to_value()["addresses"].clone();
    assert_eq!(addresses[0]["streetAddress"], "1010 Broadway Ave");
    assert_eq!(
        addresses[0]["locality"], "Hollywood",
        "the other sub-attributes are left unchanged"
    );
    assert_eq!(addresses[1]["streetAddress"], "456 Hollywood Blvd");
}

#[test]
fn replacing_without_a_path_replaces_the_named_attributes() {
    // RFC 7644 §3.5.2.3 prints this request one closing brace short.
    let body = source::json(
        Doc::Rfc7644,
        2594..=2613,
        &[Fix::Replace(
            r#""nickname":"Babs""#,
            r#""nickname":"Babs"}"#,
        )],
    );
    let outcome = patch(&body)
        .apply(&full_user(), &user_schema())
        .expect("applies");
    let user = outcome.resource().to_value();
    assert_eq!(
        user["emails"],
        json!([{"value": "bjensen@example.com", "type": "work", "primary": true}, {"value": "babs@jensen.org", "type": "home"}])
    );
    assert_eq!(user["nickName"], "Babs");
    assert_eq!(
        user["title"], "Tour Guide",
        "§3.5.2.3: other attributes are unaffected"
    );
}

#[test]
fn a_remove_without_a_path_is_no_target() {
    let body = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [{"op": "remove"}]});
    assert_eq!(
        refused(&body, &group(), &group_schema()),
        ScimType::NoTarget
    );
}

#[test]
fn a_replace_whose_filter_matches_nothing_is_no_target() {
    let body = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "replace", "path": "addresses[type eq \"other\"].locality", "value": "Elsewhere"}]});
    assert_eq!(
        refused(&body, &full_user(), &user_schema()),
        ScimType::NoTarget
    );
}

#[test]
fn an_add_whose_equality_filter_matches_nothing_adds_the_record() {
    let body = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "add", "path": "emails[type eq \"work\"].value", "value": "bjensen@example.com"}]});
    let outcome = patch(&body)
        .apply(&minimal_user(), &user_schema())
        .expect("applies");
    assert_eq!(
        outcome.resource().to_value()["emails"],
        json!([{"type": "work", "value": "bjensen@example.com"}])
    );
}

#[test]
fn readonly_required_and_immutable_targets_are_mutability() {
    let user = full_user();
    let schema = user_schema();
    for operation in [
        json!({"op": "replace", "path": "id", "value": "other"}),
        json!({"op": "replace", "path": "meta.created", "value": "2020-01-01T00:00:00Z"}),
        json!({"op": "add", "path": "groups", "value": [{"value": "x"}]}),
        json!({"op": "remove", "path": "userName"}),
    ] {
        let body = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [operation]});
        assert_eq!(
            refused(&body, &user, &schema),
            ScimType::Mutability,
            "{body}"
        );
    }
    let body = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "replace", "path": format!("members[value eq \"{BABS}\"].value"), "value": "x"}]});
    assert_eq!(
        refused(&body, &group(), &group_schema()),
        ScimType::Mutability,
        "RFC 7643 §4.2: member sub-attributes are immutable"
    );
}

#[test]
fn malformed_requests_are_refused_with_their_scim_type() {
    let user = minimal_user();
    let schema = user_schema();
    for (body, expected) in [
        (
            json!({"Operations": [{"op": "add", "path": "title", "value": "x"}]}),
            ScimType::InvalidSyntax,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"]}),
            ScimType::InvalidSyntax,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": []}),
            ScimType::InvalidSyntax,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [{"op": "move", "path": "title"}]}),
            ScimType::InvalidSyntax,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [{"path": "title", "value": "x"}]}),
            ScimType::InvalidSyntax,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [{"op": "add", "path": "title"}]}),
            ScimType::InvalidValue,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [{"op": "add", "path": "title[", "value": "x"}]}),
            ScimType::InvalidPath,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [{"op": "add", "path": "emails[type zz \"w\"]", "value": "x"}]}),
            ScimType::InvalidFilter,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [{"op": "add", "path": "title", "value": 5}]}),
            ScimType::InvalidValue,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [{"op": "add", "path": "noSuchAttribute", "value": "x"}]}),
            ScimType::InvalidPath,
        ),
        (
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": [{"op": "add", "path": "title", "value": "x"}, {"op": "remove"}]}),
            ScimType::NoTarget,
        ),
    ] {
        assert_eq!(refused(&body, &user, &schema), expected, "{body}");
    }
}

#[test]
fn a_failing_operation_leaves_the_resource_as_it_was() {
    // RFC 7644 §3.5.2: "A PATCH request, regardless of the number of
    // operations, SHALL be treated as atomic."
    let user = minimal_user();
    let before = user.to_value();
    let body = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "replace", "path": "title", "value": "x"}, {"op": "replace", "path": "id", "value": "y"}]});
    patch(&body)
        .apply(&user, &user_schema())
        .expect_err("the second operation is refused");
    assert_eq!(user.to_value(), before);
}

#[test]
fn an_extension_attribute_added_by_its_qualified_name_extends_schemas() {
    // RFC 7644 §3.5.2: adding the attribute by its fully qualified name
    // "automatically adds the value ... to the resource's "schemas" attribute".
    let body = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "add", "path": "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:employeeNumber", "value": "701984"}]});
    let outcome = patch(&body)
        .apply(&minimal_user(), &user_schema())
        .expect("applies");
    let user = outcome.resource().to_value();
    assert_eq!(
        user["urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"]["employeeNumber"],
        "701984"
    );
    assert_eq!(
        user["schemas"],
        json!([
            "urn:ietf:params:scim:schemas:core:2.0:User",
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
        ])
    );
    let body = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "replace", "value": {"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:costCenter": "4130"}}]});
    let outcome = patch(&body)
        .apply(&minimal_user(), &user_schema())
        .expect("applies");
    assert_eq!(
        outcome.resource().to_value()["urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"]
            ["costCenter"],
        "4130"
    );
}

#[test]
fn a_new_primary_value_resets_the_others() {
    let body = json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "add", "path": "emails", "value": [{"value": "new@example.com", "type": "other", "primary": true}]}]});
    let outcome = patch(&body)
        .apply(&full_user(), &user_schema())
        .expect("applies");
    let emails = outcome.resource().to_value()["emails"].clone();
    assert_eq!(emails[0]["primary"], false);
    assert_eq!(emails[2]["primary"], true);
}

#[test]
fn every_figure_8_path_parses() {
    let text = source::joined(Doc::Rfc7644, 1872..=1882);
    let paths: Vec<String> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let member: Value =
                serde_json::from_str(&format!("{{{}}}", line.trim())).expect("a path member");
            member["path"].as_str().expect("a path string").to_owned()
        })
        .collect();
    assert_eq!(paths.len(), 5);
    for path in &paths {
        PatchPath::parse(path).unwrap_or_else(|error| panic!("{path}: {error}"));
    }
}
