// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Microsoft Entra ID provisioning client against `/scim/v2/Groups`: the
//! "Group Operations" requests of Microsoft's "Develop a SCIM endpoint for
//! user provisioning to apps from Microsoft Entra ID", read from the vendored
//! document by line range, each answered as that document prints it. The
//! document's ids are its own, so the requests address the ids this provider
//! assigned.

use http::StatusCode;
use serde_json::{Value, json};

use super::{CLIENT, app, call, created};
use crate::scim_protocol::source::{self, Doc};

/// The target of the markdown request line `number`, such as
/// `*GET /Groups/40734ae655284ad3abcc?excludedAttributes=members HTTP/1.1*`,
/// with the document's id replaced by `id` and a filter percent-encoded.
fn request_path(number: usize, id: Option<&str>) -> String {
    let line = source::line(Doc::EntraEndpoint, number);
    let mut target = line
        .trim_matches('*')
        .trim_end_matches(" HTTP/1.1")
        .split_once(' ')
        .map(|(_, target)| target.trim().to_owned())
        .expect("a request line");
    if let Some(id) = id {
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        let parent = path.rsplit_once('/').map_or(path, |(parent, _)| parent);
        target = if query.is_empty() {
            format!("{parent}/{id}")
        } else {
            format!("{parent}/{id}?{query}")
        };
    }
    match target.split_once("filter=") {
        Some((head, filter)) => format!("{head}filter={}", urlencoding::encode(filter)),
        None => target,
    }
}

/// Asserts that `body` is the vendored Create Group request as this
/// provider serves it, with the members left out.
fn assert_group(body: &Value, display_name: &str) {
    assert_eq!(body["externalId"], "8aa1a0c0-c4c3-4bc0-b4a5-2ef676900159");
    assert_eq!(body["displayName"], display_name);
    assert_eq!(body["meta"]["resourceType"], "Group");
    assert_eq!(
        body["schemas"],
        json!(["urn:ietf:params:scim:schemas:core:2.0:Group"])
    );
}

/// Create Group, the duplicate create its SCIM Validator sends, Get Group
/// and Get Group by displayName with `excludedAttributes=members`, Update
/// Group of a non-member attribute, Add Members, Remove Members and Delete
/// Group, in the order the document lists them.
#[tokio::test]
async fn the_entra_group_operations_answer_as_documented() {
    let (_db, app) = app().await;
    let create = source::json(Doc::EntraEndpoint, 673..=680, &[]);
    let (status, _, body) = call(&app, "POST", "/Groups", CLIENT, Some(&create), &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_group(&body, "displayName");
    assert!(
        body["members"].as_array().is_none_or(Vec::is_empty),
        "Groups are created with an empty members list: {body}"
    );
    let id = body["id"].as_str().expect("an id").to_owned();

    let (status, _, body) = call(&app, "POST", "/Groups", CLIENT, Some(&create), &[]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["scimType"], "uniqueness");

    let (member, _) = created(
        &app,
        CLIENT,
        "member",
        "3c4d5e6f-7a8b-4c9d-8e0f-1a2b3c4d5e6f",
    )
    .await;
    let mut add = source::json(Doc::EntraEndpoint, 815..=825, &[]);
    add["Operations"][0]["value"][0]["value"] = json!(member);
    let (status, _, body) = call(
        &app,
        "PATCH",
        &request_path(812, Some(&id)),
        CLIENT,
        Some(&add),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, _, body) = call(
        &app,
        "GET",
        &request_path(724, Some(&id)),
        CLIENT,
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_group(&body, "displayName");
    assert_eq!(
        body.get("members"),
        None,
        "excludedAttributes=members: {body}"
    );

    let (status, _, body) = call(&app, "GET", &request_path(748, None), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totalResults"], 1);
    assert_eq!(body["Resources"][0]["id"], id.as_str());
    assert_group(&body["Resources"][0], "displayName");
    assert_eq!(body["Resources"][0].get("members"), None);

    let rename = source::json(Doc::EntraEndpoint, 782..=789, &[]);
    let (status, _, body) = call(
        &app,
        "PATCH",
        &request_path(779, Some(&id)),
        CLIENT,
        Some(&rename),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, _, body) = call(&app, "GET", &format!("/Groups/{id}"), CLIENT, None, &[]).await;
    assert_group(
        &body,
        "1879db59-3bdf-4490-ad68-ab880a269474updatedDisplayName",
    );
    assert_eq!(body["members"][0]["value"], member.as_str());

    let mut remove = source::json(Doc::EntraEndpoint, 849..=859, &[]);
    remove["Operations"][0]["value"][0]["value"] = json!(member);
    let (status, _, body) = call(
        &app,
        "PATCH",
        &format!("/Groups/{id}"),
        CLIENT,
        Some(&remove),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, _, body) = call(&app, "GET", &format!("/Groups/{id}"), CLIENT, None, &[]).await;
    assert!(
        body["members"].as_array().is_none_or(Vec::is_empty),
        "{body}"
    );

    let (status, _, body) = call(
        &app,
        "DELETE",
        &request_path(870, Some(&id)),
        CLIENT,
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, _, _) = call(&app, "GET", &format!("/Groups/{id}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Test Connection's random group probe and a filter on a random
/// `externalId` answer `404` and an empty `ListResponse`.
#[tokio::test]
async fn the_test_connection_group_probes_answer_as_documented() {
    let (_db, app) = app().await;
    let random = "5d0b8a27-0bd6-4a5c-a0f9-6c4b4e2e8f31";
    let probe = format!(
        "/Groups?filter={}",
        urlencoding::encode(&format!("externalId eq \"{random}\""))
    );
    let (status, _, body) = call(&app, "GET", &probe, CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totalResults"], 0);
    assert_eq!(body["Resources"], json!([]));
    let (status, _, body) =
        call(&app, "GET", &format!("/Groups/{random}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
