// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A SCIM User's `roles` (RFC 7643 §4.1.2) as `ROLE` parties (#5625): each
//! value is one `ROLE` the User's `PERSON` performs, named by the role coded
//! in the terminology the client's `scim_provision` grant names, within the
//! roles that grant admits. A role the client removes is ended and no longer
//! decides the next request. RFC 7643 sets no vocabulary for `roles` and
//! RFC 7644 §2 leaves the authorization model out: our own design.

use ferroehr::service::FerroEhrService;
use http::StatusCode;
use openehr_rm::draft::access_control::terminology::ForeignCode;
use serde_json::{Value, json};

use super::{CLIENT, OTHER, ROLES, app_with_service, call, create_body, patch_op};

/// The roles of the caller facts the replica holds for the `PERSON` `id`.
async fn performed(service: &FerroEhrService, id: &str) -> Vec<ForeignCode> {
    service.grants().refresh().await.expect("the copy reads");
    service
        .grants()
        .facts_of(id.parse().expect("a UUID"), jiff::Timestamp::now())
        .roles()
        .to_vec()
}

/// The role `code` of [`ROLES`].
fn role(code: &str) -> ForeignCode {
    ForeignCode {
        terminology: ROLES.to_owned(),
        code: code.to_owned(),
    }
}

/// A create body holding `roles`.
fn with_roles(user_name: &str, key: &str, roles: &[&str]) -> Value {
    let mut body = create_body(user_name, user_name, key);
    body["roles"] = json!(
        roles
            .iter()
            .map(|r| json!({ "value": r }))
            .collect::<Vec<_>>()
    );
    body
}

/// A created role is a `ROLE` the User performs, served in its `roles` and
/// held by the caller facts; a PATCH that removes it ends it, so it decides
/// no further request, and a PUT replaces the set.
#[tokio::test]
async fn a_scim_role_is_a_role_the_user_performs_until_it_is_removed() {
    let (_db, service, app) = app_with_service().await;
    let (status, _, body) = call(
        &app,
        "POST",
        "/Users",
        CLIENT,
        Some(&with_roles(
            "nurse.ann",
            "1b6c9a2e-6a3d-4f7e-9a51-1f2b3c4d5e60",
            &["NURSE"],
        )),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["roles"], json!([{ "value": "NURSE" }]), "{body}");
    let id = body["id"].as_str().expect("an id").to_owned();
    assert_eq!(performed(&service, &id).await, vec![role("NURSE")]);

    let path = format!("/Users/{id}");
    let add =
        patch_op(json!([{ "op": "add", "path": "roles", "value": [{ "value": "PHYSICIAN" }] }]));
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&add), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["roles"],
        json!([{ "value": "NURSE" }, { "value": "PHYSICIAN" }]),
        "{body}"
    );
    assert_eq!(
        performed(&service, &id).await,
        vec![role("NURSE"), role("PHYSICIAN")]
    );

    let remove = patch_op(json!([{ "op": "remove", "path": "roles[value eq \"NURSE\"]" }]));
    let (status, _, body) = call(&app, "PATCH", &path, CLIENT, Some(&remove), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["roles"], json!([{ "value": "PHYSICIAN" }]), "{body}");
    assert_eq!(
        performed(&service, &id).await,
        vec![role("PHYSICIAN")],
        "the removed role decides the next request no more"
    );

    let (status, _, current) = call(&app, "GET", &path, CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    let mut replaced = current.clone();
    replaced.as_object_mut().expect("an object").remove("roles");
    let (status, _, body) = call(&app, "PUT", &path, CLIENT, Some(&replaced), &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.get("roles").is_none(),
        "a PUT without roles ends them: {body}"
    );
    assert_eq!(performed(&service, &id).await, Vec::new());
}

/// A role the client's grant does not admit, and any role of a client whose
/// grant names no role terminology, is refused `403`, and no User is made.
#[tokio::test]
async fn a_role_outside_the_clients_ceiling_is_403() {
    let (_db, _service, app) = app_with_service().await;
    let (status, _, body) = call(
        &app,
        "POST",
        "/Users",
        CLIENT,
        Some(&with_roles(
            "surgeon.sam",
            "2c7d0b3f-7b4e-4a8f-8b62-2a3c4d5e6f70",
            &["SURGEON"],
        )),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _, body) = call(
        &app,
        "POST",
        "/Users",
        OTHER,
        Some(&with_roles(
            "nurse.oli",
            "3d8e1c4a-8c5f-4b9a-9c73-3b4d5e6f7a80",
            &["NURSE"],
        )),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _, body) = call(
        &app,
        "GET",
        "/Users?filter=userName%20eq%20%22surgeon.sam%22",
        CLIENT,
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totalResults"], json!(0), "no User was made: {body}");
}
