// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The error response of RFC 7644 §3.12: its body, the `status` string and
//! every `scimType` of Table 9.

use ferroehr_rest::scim::protocol::error::{ErrorBody, ScimError, ScimType};
use http::StatusCode;

use super::source::{self, Doc, Fix};

#[test]
fn every_scim_type_of_table_9_is_spelled_as_the_table() {
    let mut keywords: Vec<String> = Vec::new();
    for line in source::lines(Doc::Rfc7644, 3821..=3910) {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if let Some(first) = cells
            .get(1)
            .filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_alphabetic()))
        {
            keywords.push((*first).to_owned());
        }
    }
    assert_eq!(keywords.len(), 10, "{keywords:?}");
    let ours: Vec<&str> = ScimType::ALL.iter().map(|t| t.as_str()).collect();
    assert_eq!(ours, keywords);
    for scim_type in ScimType::ALL {
        assert_eq!(
            serde_json::to_value(scim_type).expect("serializes"),
            scim_type.as_str()
        );
    }
}

#[test]
fn the_not_found_example_is_what_the_provider_writes() {
    let printed = source::json(Doc::Rfc7644, 2673..=2677, &[]);
    let ours = ScimError::new(
        StatusCode::NOT_FOUND,
        "Resource 2819c223-7f76-453a-919d-413861904646 not found",
    );
    assert_eq!(
        serde_json::to_value(ours.body()).expect("serializes"),
        printed
    );
    assert_eq!(
        source::json(Doc::Rfc7644, 3931..=3935, &[]),
        printed,
        "§3.12 prints the same body"
    );
}

#[test]
fn the_mutability_example_is_what_the_provider_writes() {
    // RFC 7644 §3.12 prints this body without the comma after `scimType`.
    let printed = source::json(
        Doc::Rfc7644,
        3941..=3946,
        &[Fix::Replace(
            r#""scimType":"mutability""#,
            r#""scimType":"mutability","#,
        )],
    );
    let ours = ScimError::bad_request(ScimType::Mutability, "Attribute 'id' is readOnly");
    assert_eq!(
        serde_json::to_value(ours.body()).expect("serializes"),
        printed
    );
}

#[test]
fn an_error_body_reads_back_into_its_error() {
    let printed = source::json(
        Doc::Rfc7644,
        3941..=3946,
        &[Fix::Replace(
            r#""scimType":"mutability""#,
            r#""scimType":"mutability","#,
        )],
    );
    let body: ErrorBody = serde_json::from_value(printed).expect("reads");
    let error = ScimError::try_from(body).expect("a valid error body");
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.scim_type(), Some(ScimType::Mutability));
    assert_eq!(error.detail(), "Attribute 'id' is readOnly");

    let entra = source::json(Doc::EntraEndpoint, 392..=398, &[]);
    let body: ErrorBody = serde_json::from_value(entra).expect("reads");
    assert_eq!(
        ScimError::try_from(body).expect("valid").status(),
        StatusCode::NOT_FOUND
    );

    for bad in [
        serde_json::json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:Error"], "status": "four hundred"}),
        serde_json::json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:ListResponse"], "status": "400"}),
        serde_json::json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:Error"], "status": "404", "scimType": "invalidValue"}),
    ] {
        let body: ErrorBody = serde_json::from_value(bad.clone()).expect("reads as a body");
        assert!(ScimError::try_from(body).is_err(), "{bad}");
    }
}

#[test]
fn the_status_is_a_json_string_and_scim_type_is_only_for_its_statuses() {
    let error = ScimError::bad_request(
        ScimType::InvalidFilter,
        "The operator 'regex' is not supported.",
    );
    let body = serde_json::to_value(error.body()).expect("serializes");
    assert_eq!(body["status"], "400");
    assert_eq!(body["scimType"], "invalidFilter");
    assert_eq!(
        body["schemas"],
        serde_json::json!(["urn:ietf:params:scim:api:messages:2.0:Error"])
    );

    // RFC 7644 §3.3: a duplicate "MUST return HTTP status code 409 (Conflict)
    // with a "scimType" error code of "uniqueness"".
    let conflict = ScimError::uniqueness("userName 'bjensen' is in use");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(conflict.scim_type(), Some(ScimType::Uniqueness));

    let gone =
        serde_json::to_value(ScimError::new(StatusCode::PRECONDITION_FAILED, "changed").body())
            .expect("serializes");
    assert!(
        gone.get("scimType").is_none(),
        "no null is written for an absent scimType"
    );
}
