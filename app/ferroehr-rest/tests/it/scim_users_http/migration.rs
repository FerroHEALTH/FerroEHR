// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The first provisioning cycle of the Microsoft Entra ID client over a
//! deployment migrating to party binding (#5009): its "Get User by query"
//! then "Update User" requests, read from the vendored "Develop a SCIM
//! endpoint for user provisioning to apps from Microsoft Entra ID", find the
//! `PERSON`s that already carry the binding key and `userName`, write their
//! `externalId`, and create no duplicate. A prepared `PERSON` with no
//! professional `ROLE` is no User until `party-binding adopt` gives it one.
//! The migration and the adoption are our own design; the `externalId` is
//! the provider's to issue (RFC 7643 §3.1).

use ferroehr::ids::VoId;
use ferroehr::party_binding::check::{Administrator, CheckInput};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::adopt::AdoptInput;
use ferroehr::service::demographic::party_set::NewParty;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::versioning::committer::SYSTEM_COMMITTER_NAME;
use http::StatusCode;
use serde_json::{Value, json};

use super::{CLIENT, KEY_PATH, LOGIN, NAME_PATH, PERSON, ROLE, app_with_service, call};
use crate::scim_protocol::source::{self, Doc};

/// The `userName` of the vendored "Get User by query" request (line 405).
const USER_NAME: &str = "Test_User_00aa00aa-bb11-cc22-dd33-44ee44ee44ee";
/// The `externalId` the identity provider issues and writes.
const EXTERNAL_ID: &str = "0a21f0f2-8d2a-4f8e-bf98-7363c4aed4ef";
/// The binding key the existing `PERSON` carries.
const KEY: &str = "7d8e9f0a-1b2c-4d3e-8f4a-5b6c7d8e9f0a";
/// The `PARTY_REF.namespace` of the mapping.
const NAMESPACE: &str = "demographic";

/// The vendored "Get User by query" request (line 405), its filter
/// percent-encoded.
fn get_user_by_query() -> String {
    let line = source::line(Doc::EntraEndpoint, 405);
    let target = line
        .trim_matches('*')
        .split_once(' ')
        .map(|(_, target)| target.trim().to_owned())
        .expect("a request line");
    let (head, filter) = target.split_once("filter=").expect("a filter");
    format!("{head}filter={}", urlencoding::encode(filter))
}

/// The vendored "Update User [Single-valued properties]" request (lines 517
/// to 524) replacing `externalId` instead of `userName`.
fn update_external_id() -> Value {
    let mut body = source::json(Doc::EntraEndpoint, 517..=524, &[]);
    body["Operations"][0]["path"] = json!("externalId");
    body["Operations"][0]["value"] = json!(EXTERNAL_ID);
    body
}

/// The `PERSON` an earlier system holds: the binding key and the
/// `userName`, no `externalId`.
async fn existing_body(svc: &FerroEhrService) -> Value {
    let identifier = |kind: &str, id: &str| json!({ "_type": "DV_IDENTIFIER", "issuer": LOGIN, "id": id, "type": kind });
    svc.mapped_party_body(
        None,
        PERSON,
        &[
            (KEY_PATH.to_owned(), identifier("guid", KEY)),
            (KEY_PATH.to_owned(), identifier("scim_user_name", USER_NAME)),
            (
                NAME_PATH.to_owned(),
                json!({ "_type": "DV_TEXT", "value": "givenName familyName" }),
            ),
        ],
    )
    .await
    .expect("the body maps")
}

/// Commits the existing `PERSON` as a migration does, under the system
/// committer, with its professional `ROLE` when `professional`.
async fn migrated(svc: &FerroEhrService, professional: bool) -> VoId {
    let mut body = existing_body(svc).await;
    let person = svc.allocate_party_id();
    let id = person.id();
    let mut parties = Vec::new();
    if professional {
        let role = svc.allocate_party_id();
        body["roles"] = json!([{
            "_type": "PARTY_REF", "namespace": NAMESPACE, "type": "ROLE",
            "id": { "_type": "HIER_OBJECT_ID", "value": role.id().to_string() }
        }]);
        parties.push(NewParty {
            body: svc
                .professional_role_body(ROLE, id, NAMESPACE)
                .await
                .expect("the ROLE maps"),
            id: role,
            kind: PartyKind::Role,
            lifecycle_state: None,
        });
    }
    parties.insert(
        0,
        NewParty {
            id: person,
            kind: PartyKind::Person,
            body,
            lifecycle_state: None,
        },
    );
    svc.create_parties(parties, "a migration from an earlier system", None)
        .await
        .expect("the migration commits");
    id
}

/// One provisioning cycle of the Entra client: "Get User by query", then
/// "Update User" on the User it found. Returns the User's `id`.
async fn provisioning_cycle(app: &axum::Router) -> String {
    let (status, _, body) = call(app, "GET", &get_user_by_query(), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["totalResults"], 1,
        "the existing PERSON is found: {body}"
    );
    let id = body["Resources"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    assert_eq!(body["Resources"][0]["externalId"], Value::Null, "{body}");

    let user = format!("/Users/{id}");
    let (status, _, body) = call(
        app,
        "PATCH",
        &user,
        CLIENT,
        Some(&update_external_id()),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["externalId"], EXTERNAL_ID);
    assert_eq!(body["userName"], USER_NAME);
    id
}

/// Asserts the provider holds one User of `USER_NAME`, the `PERSON` `id`,
/// found by `userName` and by `externalId`.
async fn assert_one_user(app: &axum::Router, id: &str) {
    let (_, _, body) = call(app, "GET", &get_user_by_query(), CLIENT, None, &[]).await;
    assert_eq!(body["totalResults"], 1, "no duplicate: {body}");
    assert_eq!(body["Resources"][0]["id"], id);
    let by_external_id = format!(
        "/Users?filter={}",
        urlencoding::encode(&format!("externalId eq \"{EXTERNAL_ID}\""))
    );
    let (_, _, body) = call(app, "GET", &by_external_id, CLIENT, None, &[]).await;
    assert_eq!(body["totalResults"], 1, "{body}");
    assert_eq!(body["Resources"][0]["id"], id);
    let (status, _, body) = call(app, "GET", "/Users", CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totalResults"], 1, "one User in all: {body}");
}

#[tokio::test]
async fn a_provisioning_cycle_updates_an_existing_person_and_creates_no_duplicate() {
    let (_db, svc, app) = app_with_service().await;
    let existing = migrated(&svc, true).await;

    let id = provisioning_cycle(&app).await;

    assert_eq!(id, existing.to_string(), "the existing PERSON is the User");
    assert_one_user(&app, &id).await;
}

#[tokio::test]
async fn an_adopted_person_is_updated_by_the_cycle_and_never_duplicated() {
    let (_db, svc, app) = app_with_service().await;
    let administrator = VoId(uuid::Uuid::now_v7());
    let prepared = migrated(&svc, false).await;
    let (_, _, body) = call(&app, "GET", &get_user_by_query(), CLIENT, None, &[]).await;
    assert_eq!(
        body["totalResults"], 0,
        "a prepared PERSON with no ROLE is no User, so the provider would create one: {body}"
    );

    let report = svc
        .adopt_persons(&AdoptInput {
            check: CheckInput {
                key_types: [(LOGIN.to_owned(), "guid".to_owned())].into(),
                administrators: vec![Administrator::Party(administrator.0)],
                migrations: vec![SYSTEM_COMMITTER_NAME.to_owned()],
                today: ferroehr::support::today_utc().to_string(),
                ..CheckInput::default()
            },
            parties: vec![prepared],
            namespace: NAMESPACE.to_owned(),
        })
        .await
        .expect("the adoption commits");
    assert_eq!(report.adopted.len(), 1, "{report:?}");

    let id = provisioning_cycle(&app).await;

    assert_eq!(id, prepared.to_string(), "the adopted PERSON is the User");
    assert_one_user(&app, &id).await;
}
