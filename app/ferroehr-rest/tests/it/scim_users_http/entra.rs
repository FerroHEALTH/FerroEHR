// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Microsoft Entra ID provisioning client against `/scim/v2/Users`: the
//! "User Operations" requests of Microsoft's "Develop a SCIM endpoint for user
//! provisioning to apps from Microsoft Entra ID", read from the vendored
//! document by line range, and the probes its Test Connection sends. Each is
//! answered as that document prints it, within the attributes the mapping
//! carries.

use http::StatusCode;
use serde_json::{Value, json};

use super::{CLIENT, EXTENSION, app, call};
use crate::scim_protocol::source::{self, Doc};

/// The binding key the Entra client's attribute mapping sends for the
/// professional it creates.
const KEY: &str = "3c4d5e6f-7a8b-4c9d-8e0f-1a2b3c4d5e6f";

/// The query path of a markdown request line such as
/// `*GET /Users?filter=userName eq "x"*`, its filter percent-encoded.
fn request_path(number: usize) -> String {
    let line = source::line(Doc::EntraEndpoint, number);
    let target = line
        .trim_matches('*')
        .trim_end_matches(" HTTP/1.1")
        .split_once(' ')
        .map(|(_, target)| target.trim().to_owned())
        .expect("a request line");
    match target.split_once("filter=") {
        Some((head, filter)) => format!("{head}filter={}", urlencoding::encode(filter)),
        None => target,
    }
}

/// The Create User request (lines 282 to 303) as the Entra client sends it
/// to FerroEHR: with the binding key its attribute mapping adds in
/// FerroEHR's extension, whose URI it leaves out of `schemas`.
fn create_user() -> Value {
    let mut body = source::json(Doc::EntraEndpoint, 282..=303, &[]);
    body[EXTENSION] = json!({ "loginIdentifier": KEY });
    body
}

/// Asserts that `body` is the vendored User as FerroEHR maps it: the Create
/// User request's `externalId`, names, `active` and work email.
fn assert_created(body: &Value) {
    assert_eq!(
        body["userName"],
        "Test_User_00aa00aa-bb11-cc22-dd33-44ee44ee44ee"
    );
    assert_eq!(body["externalId"], "0a21f0f2-8d2a-4f8e-bf98-7363c4aed4ef");
    assert_eq!(body["name"]["formatted"], "givenName familyName");
    assert_eq!(body["name"]["givenName"], "givenName");
    assert_eq!(body["name"]["familyName"], "familyName");
    assert_eq!(body["active"], true);
    assert_eq!(
        body["emails"],
        json!([{
            "type": "work",
            "value": "Test_User_11bb11bb-cc22-dd33-ee44-55ff55ff55ff@testuser.com"
        }]),
        "{body}"
    );
}

/// Create User, the duplicate create its SCIM Validator sends, Get User, the
/// two Get User by query requests, the multi-valued and single-valued Update
/// User, Disable User and Delete User, in the order the document lists them.
#[tokio::test]
async fn the_entra_user_operations_answer_as_documented() {
    let (_db, app) = app().await;
    let (status, _, body) = call(&app, "POST", "/Users", CLIENT, Some(&create_user()), &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_created(&body);
    let id = body["id"].as_str().expect("an id").to_owned();
    let user = format!("/Users/{id}");

    let (status, _, body) = call(&app, "POST", "/Users", CLIENT, Some(&create_user()), &[]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["scimType"], "uniqueness");

    let (status, _, body) = call(&app, "GET", &user, CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_created(&body);

    let (status, _, body) = call(&app, "GET", &request_path(405), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totalResults"], 1);
    assert_eq!(body["Resources"][0]["id"], id.as_str());
    assert_created(&body["Resources"][0]);

    let (status, _, body) = call(&app, "GET", &request_path(445), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totalResults"], 0);
    assert_eq!(body["Resources"], json!([]));

    let multi = source::json(Doc::EntraEndpoint, 466..=480, &[]);
    let (status, _, body) = call(&app, "PATCH", &user, CLIENT, Some(&multi), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"]["familyName"], "updatedFamilyName");
    assert_eq!(body["name"]["givenName"], "givenName");
    assert_eq!(
        body["emails"],
        json!([{ "type": "work", "value": "updatedEmail@microsoft.com" }])
    );

    let single = source::json(Doc::EntraEndpoint, 517..=524, &[]);
    let (status, _, body) = call(&app, "PATCH", &user, CLIENT, Some(&single), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["userName"],
        "5b50642d-79fc-4410-9e90-4c077cdd1a59@testuser.com"
    );

    let disable = source::json(Doc::EntraEndpoint, 589..=600, &[]);
    let (status, _, body) = call(&app, "PATCH", &user, CLIENT, Some(&disable), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["active"], false);
    let joined = format!(
        "/Users?filter={}",
        urlencoding::encode("userName eq \"5b50642d-79fc-4410-9e90-4c077cdd1a59@testuser.com\"")
    );
    let (status, _, body) = call(&app, "GET", &joined, CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["Resources"][0]["active"], false,
        "a disabled User is still returned: {body}"
    );

    let (status, _, body) = call(&app, "DELETE", &user, CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, _, _) = call(&app, "GET", &user, CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Test Connection queries a User by a random GUID as the matching property
/// and expects `200` with an empty `ListResponse` (line 1434), and fetches a
/// random user and group (line 192), each of which this provider answers
/// `404` with an RFC 7644 §3.12 error.
#[tokio::test]
async fn the_test_connection_probes_answer_as_documented() {
    let (_db, app) = app().await;
    let random = "5d0b8a27-0bd6-4a5c-a0f9-6c4b4e2e8f31";
    let probe = format!(
        "/Users?filter={}",
        urlencoding::encode(&format!("externalId eq \"{random}\""))
    );
    let (status, _, body) = call(&app, "GET", &probe, CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["schemas"],
        json!(["urn:ietf:params:scim:api:messages:2.0:ListResponse"])
    );
    assert_eq!(body["totalResults"], 0);
    assert_eq!(body["Resources"], json!([]));

    for path in [format!("/Users/{random}"), format!("/Groups/{random}")] {
        let (status, _, body) = call(&app, "GET", &path, CLIENT, None, &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(
            body["schemas"],
            json!(["urn:ietf:params:scim:api:messages:2.0:Error"]),
            "{path}"
        );
        assert_eq!(body["status"], "404", "{path}");
    }
}
