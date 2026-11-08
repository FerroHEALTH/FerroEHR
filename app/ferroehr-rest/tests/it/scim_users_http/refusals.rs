// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals of `/scim/v2/Users` that no other module of this suite
//! pins: a `PERSON` that is no User of the client, a leaver's `userName`, a
//! changed binding key and an attribute the mapping does not carry, each with
//! its RFC 7644 §3.12 status and `scimType`. Which `PERSON` is a client's
//! User is our own design.

use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use http::StatusCode;
use serde_json::{Value, json};

use super::{
    CLIENT, EXTENSION, LOGIN, OTHER, app, app_with_service, call, create_body, created, patch_op,
};

/// Commits `body` as a new `PERSON` outside SCIM, as an administrator does,
/// and returns its id.
async fn committed(svc: &FerroEhrService, body: &Value) -> String {
    let write = WriteRequest::new(
        openehr_its::json::from_canonical_value(body).expect("the PERSON body decodes"),
    );
    let response = Box::pin(svc.party_create(PartyKind::Person, write))
        .await
        .expect("the PERSON commits");
    let uid = response.meta.expect("a version").uid;
    uid.split("::").next().expect("a container id").to_owned()
}

/// The `PERSON` body of the User `id`, without its version and its `roles`,
/// every identifier of it renamed by `suffix`.
async fn person_like(svc: &FerroEhrService, id: &str, suffix: &str) -> Value {
    let vo = id.parse().map(ferroehr::ids::VoId).expect("a User id");
    let user = svc
        .provisioned_user(LOGIN, vo)
        .await
        .expect("reads")
        .expect("found");
    let mut body = user.body;
    let object = body.as_object_mut().expect("a PERSON");
    object.remove("uid");
    object.remove("roles");
    let items = body
        .pointer_mut("/details/items")
        .and_then(Value::as_array_mut)
        .expect("the identifiers");
    for item in items {
        let value = item.pointer_mut("/value/id").expect("a DV_IDENTIFIER id");
        let renamed = format!("{}{suffix}", value.as_str().expect("a string"));
        *value = json!(renamed);
    }
    body
}

/// Another client's User, a `PERSON` carrying the client's key but performing
/// no professional `ROLE`, and a patient's `PERSON` carrying no key are no
/// Users of the client: each is `404` to a read, a `PATCH` and a `DELETE`,
/// and none is listed.
#[tokio::test]
async fn a_person_that_is_no_user_of_the_client_is_404() {
    let (_db, svc, app) = app_with_service().await;
    let (theirs, _) = created(
        &app,
        OTHER,
        "theirs",
        "5a6b7c8d-9e0f-4a1b-8c2d-3e4f5a6b7c8d",
    )
    .await;
    let (mine, _) = created(&app, CLIENT, "mine", "6b7c8d9e-0f1a-4b2c-9d3e-4f5a6b7c8d9e").await;
    let roleless = committed(&svc, &person_like(&svc, &mine, "0").await).await;
    let mut patient = person_like(&svc, &mine, "1").await;
    patient.as_object_mut().expect("a PERSON").remove("details");
    let patient = committed(&svc, &patient).await;

    let replace = patch_op(json!([{ "op": "replace", "path": "name.formatted", "value": "x" }]));
    for (what, client, id) in [
        ("another client's User", CLIENT, &theirs),
        ("a PERSON without a professional ROLE", CLIENT, &roleless),
        ("a patient's PERSON", CLIENT, &patient),
        ("another client's User", OTHER, &mine),
    ] {
        let path = format!("/Users/{id}");
        for (method, body) in [("GET", None), ("PATCH", Some(&replace)), ("DELETE", None)] {
            let (status, _, answer) = call(&app, method, &path, client, body, &[]).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{what}, {method}: {answer}");
            assert_eq!(answer["status"], "404", "{what}, {method}");
        }
    }
    let (_, _, list) = call(&app, "GET", "/Users", CLIENT, None, &[]).await;
    assert_eq!(
        list["totalResults"], 1,
        "only the client's own User: {list}"
    );
    assert_eq!(list["Resources"][0]["id"], mine.as_str());
}

/// A reused `userName` never binds a new person to a leaver's `PERSON`, a
/// `userName` taken under another key is a conflict, and the binding key
/// never changes (RFC 7643 §2.2).
#[tokio::test]
async fn a_leavers_user_name_and_a_changed_key_are_refused() {
    let (_db, app) = app().await;
    let (leaver, _) = created(
        &app,
        CLIENT,
        "leaver@example.org",
        "7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f",
    )
    .await;
    let disable = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let path = format!("/Users/{leaver}");
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&disable), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let joiner = create_body(
        "leaver@example.org",
        "joiner",
        "8d9e0f1a-2b3c-4d4e-9f5a-6b7c8d9e0f1a",
    );
    let (status, _, body) = call(&app, "POST", "/Users", CLIENT, Some(&joiner), &[]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["scimType"], "uniqueness");
    let (_, _, body) = call(&app, "GET", &path, CLIENT, None, &[]).await;
    assert_eq!(body["active"], false, "the leaver stays inactive: {body}");

    let enable = patch_op(json!([{ "op": "replace", "path": "active", "value": true }]));
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&enable), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, body) = call(&app, "POST", "/Users", CLIENT, Some(&joiner), &[]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["scimType"], "uniqueness");

    let rekey = patch_op(json!([{
        "op": "replace",
        "path": format!("{EXTENSION}:loginIdentifier"),
        "value": "9e0f1a2b-3c4d-4e5f-8a6b-7c8d9e0f1a2b"
    }]));
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&rekey), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["scimType"], "mutability");
    let (_, _, body) = call(&app, "GET", &path, CLIENT, None, &[]).await;
    assert_eq!(
        body[EXTENSION]["loginIdentifier"],
        "7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f"
    );
}

/// An attribute the mapping does not carry is refused in a create and in a
/// `PATCH`, never dropped in silence.
#[tokio::test]
async fn an_unmapped_attribute_is_refused() {
    let (_db, app) = app().await;
    let mut body = create_body("titled", "titled", "0f1a2b3c-4d5e-4f6a-9b7c-8d9e0f1a2b3c");
    body["title"] = json!("Dr");
    let (status, _, answer) = call(&app, "POST", "/Users", CLIENT, Some(&body), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(answer["scimType"], "invalidValue", "{answer}");

    let (id, _) = created(
        &app,
        CLIENT,
        "untitled",
        "1a2b3c4d-5e6f-4a7b-8c8d-9e0f1a2b3c4d",
    )
    .await;
    let titled = patch_op(json!([{ "op": "add", "path": "title", "value": "Dr" }]));
    let (status, _, answer) = call(
        &app,
        "PATCH",
        &format!("/Users/{id}"),
        CLIENT,
        Some(&titled),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(answer["scimType"], "invalidPath", "{answer}");
}
