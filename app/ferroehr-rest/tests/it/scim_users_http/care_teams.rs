// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Care teams of SCIM-provisioned professionals, through their real callers
//! (#5617, #5640): the ADT system writing `member_of` on a SCIM User's
//! `PERSON` through the Demographic API, and a SCIM client creating a
//! care-team Group with FerroEHR's Group extension (`careTeam`).
//!
//! The party protection leaves a version that changes only the care
//! relationships to the `write_care_relationship` grant, held in the CDR. A
//! care-team Group's `GROUP` is named by the coded `care_team` and each
//! membership by the coded `member_of` (RM demographic `PARTY.type()` and
//! `PARTY_RELATIONSHIP.type()` are the `name`), written under the same grant. A Group change
//! compares what the care relationships say, never their position. No openEHR
//! spec gives a relationship an access meaning or governs who writes a party:
//! our own design (BASE Architecture Overview §The Problem of Roles puts the
//! care deliverer with the care delivery environment; RFC 7643 §4.2 leaves a
//! Group's meaning to the provider).

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use http::StatusCode;
use serde_json::{Value, json};

use super::default_definitions::{seeded, zero_config};
use super::{CLIENT, LOGIN, MACHINES, OTHER, OTHER_LOGIN, SCOPE, call, created, patch_op};
use crate::common::{self, BASE};
use crate::policy_fixture::{capability, grant, scope};

/// The URN of FerroEHR's Group extension.
const GROUP_EXTENSION: &str = "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:Group";
/// The client id of the hospital's ADT system.
const ADT: &str = "adt";

/// The router over a fresh database holding the default definitions, where
/// [`CLIENT`] provisions under [`LOGIN`] and holds `write_care_relationship`,
/// [`OTHER`] provisions under [`OTHER_LOGIN`] without it, and [`ADT`] reaches
/// the Demographic API (`access_demographic`) and holds
/// `write_care_relationship`.
async fn hospital() -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let config = zero_config();
    let (db, pool) = common::migrated_pool().await;
    let service = Arc::new(seeded(&config, &pool).await);
    let provisioning = |binding_issuer: &str| {
        capability(
            "scim_provision",
            &[
                scope("binding_issuer", binding_issuer),
                scope("token_scope", SCOPE),
            ],
        )
    };
    let care = capability("write_care_relationship", &[]);
    grant(
        &service,
        MACHINES,
        CLIENT,
        &[provisioning(LOGIN), care.clone()],
    )
    .await;
    grant(&service, MACHINES, OTHER, &[provisioning(OTHER_LOGIN)]).await;
    let demographic = capability("access_demographic", &[]);
    grant(&service, MACHINES, ADT, &[demographic, care]).await;
    let app = common::router_with(config, Arc::clone(&service)).await;
    (db, service, app)
}

/// A client-credentials token of `client` for the Demographic API.
fn system_token(client: &str) -> String {
    let exp = jiff::Timestamp::now().as_second() + 3600;
    common::bearer(
        &json!({ "iss": MACHINES, "aud": super::AUDIENCE, "exp": exp,
        "sub": client, "client_id": client,
        "scope": "system/composition-*.cruds system/aql-*.s system/template-*.r" }),
    )
}

/// The status, `ETag` and body of `GET /demographic/person/{id}` as `client`.
async fn read_person(app: &Router, client: &str, id: &str) -> (StatusCode, String, Value) {
    let request = common::req("GET", format!("{BASE}/demographic/person/{id}"))
        .auth(&system_token(client))
        .accept("application/json")
        .build();
    let (status, headers, body) = common::send(app, request).await;
    let etag = headers
        .get(http::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let etag = etag
        .strip_prefix("W/")
        .unwrap_or(etag)
        .trim_matches('"')
        .to_owned();
    (
        status,
        etag,
        serde_json::from_str(&body).unwrap_or(Value::Null),
    )
}

/// The status and body of the `PUT` of `body` as the version after
/// `version` of the `PERSON` `id`, as `client`.
async fn write_person(
    app: &Router,
    client: &str,
    (id, version): (&str, &str),
    body: &Value,
) -> (StatusCode, String) {
    let request = common::req("PUT", format!("{BASE}/demographic/person/{id}"))
        .auth(&system_token(client))
        .accept("application/json")
        .if_match(format!("\"{version}\""))
        .json(body.to_string())
        .build();
    let (status, _, body) = common::send(app, request).await;
    (status, body)
}

/// A `DV_CODED_TEXT` of FerroEHR's terminology.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" },
            "code_string": code } })
}

/// A `member_of` care relationship of the default archetype from the
/// `PERSON` `member` to the `GROUP` `team`, valid from 2026-01-01.
fn member_of(member: &str, team: &str) -> Value {
    let reference = |class: &str, id: &str| {
        json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": class,
                "id": { "_type": "HIER_OBJECT_ID", "value": id } })
    };
    let archetype = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.member_of.v1";
    json!({
        "_type": "PARTY_RELATIONSHIP", "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "Care team membership" },
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "id2",
            "name": { "_type": "DV_TEXT", "value": "Details" },
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "id3",
                "name": { "_type": "DV_TEXT", "value": "Care relationship code" },
                "value": term("member_of") }] },
        "time_validity": { "_type": "DV_INTERVAL",
            "lower": { "_type": "DV_DATE", "value": "2026-01-01" },
            "lower_unbounded": false, "upper_unbounded": true,
            "lower_included": true, "upper_included": false },
        "source": reference("PERSON", member),
        "target": reference("GROUP", team)
    })
}

/// The SCIM Group `name` of `client` with `members`, a care team when
/// `care_team`; the status and body.
async fn group(
    app: &Router,
    client: &str,
    name: &str,
    members: &[&str],
    care_team: bool,
) -> (StatusCode, Value) {
    let body = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group", GROUP_EXTENSION],
        "displayName": name,
        "members": members.iter().map(|id| json!({ "value": id })).collect::<Vec<_>>(),
        GROUP_EXTENSION: { "careTeam": care_team },
    });
    let (status, _, created) = call(app, "POST", "/Groups", client, Some(&body), &[]).await;
    (status, created)
}

/// The id of a created SCIM Group.
fn id_of(created: &Value) -> String {
    created["id"].as_str().expect("an id").to_owned()
}

/// The `relationships` of the `PERSON` `id` the ADT system reads.
async fn relationships(app: &Router, id: &str) -> Vec<Value> {
    let (status, _, body) = read_person(app, ADT, id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["relationships"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Whether `relationship` carries `ferroehr-operations::member_of`.
fn carries_member_of(relationship: &Value) -> bool {
    relationship
        .to_string()
        .contains("\"code_string\":\"member_of\"")
}

/// The ADT system rosters a SCIM User on a team: a version of the User's
/// `PERSON` that adds only a `member_of` relationship commits under its
/// `write_care_relationship` grant, and a version that also changes anything
/// else stays refused by the party protection.
#[tokio::test]
async fn the_adt_system_rosters_a_scim_user_and_changes_nothing_else() {
    let (_db, _service, app) = hospital().await;
    let (user, _) = created(&app, CLIENT, "bram", "bram-login").await;
    let (status, ward) = group(&app, CLIENT, "Ward 3", &[], false).await;
    assert_eq!(status, StatusCode::CREATED, "{ward}");
    let ward = id_of(&ward);

    let (status, version, person) = read_person(&app, ADT, &user).await;
    assert_eq!(status, StatusCode::OK, "{person}");
    let mut renamed = person.clone();
    renamed["relationships"] = json!([member_of(&user, &ward)]);
    renamed["identities"][0]["details"]["items"][0]["value"]["value"] = json!("Someone else");
    let (status, refused) = write_person(&app, ADT, (&user, &version), &renamed).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(
        refused.contains("protected by the party binding"),
        "{refused}"
    );

    let mut rostered = person.clone();
    rostered["relationships"] = json!([member_of(&user, &ward)]);
    let (status, answer) = write_person(&app, ADT, (&user, &version), &rostered).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{answer}");
    let held = relationships(&app, &user).await;
    assert_eq!(held.len(), 1, "{held:?}");
    assert!(carries_member_of(&held[0]), "{held:?}");

    // The SCIM client still owns the User: a rename through SCIM keeps the roster.
    let rename = patch_op(json!([{ "op": "replace", "path": "name.formatted", "value": "Bram" }]));
    let (status, _, renamed) = call(
        &app,
        "PATCH",
        &format!("/Users/{user}"),
        CLIENT,
        Some(&rename),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(relationships(&app, &user).await.len(), 1);
}

/// A caller reaching the Demographic API without the grant is refused the
/// same roster.
#[tokio::test]
async fn a_roster_without_the_grant_is_refused() {
    let (_db, service, app) = hospital().await;
    grant(
        &service,
        MACHINES,
        "lab",
        &[capability("access_demographic", &[])],
    )
    .await;
    let (user, _) = created(&app, CLIENT, "anna", "anna-login").await;
    let (_, ward) = group(&app, CLIENT, "Cardiology", &[], false).await;
    let (_, version, mut person) = read_person(&app, ADT, &user).await;
    person["relationships"] = json!([member_of(&user, &id_of(&ward))]);
    let (status, refused) = write_person(&app, "lab", (&user, &version), &person).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(relationships(&app, &user).await, Vec::<Value>::new());
}

/// A care-team Group: its creation and each membership need the client's
/// `write_care_relationship` grant; its members' memberships carry
/// `member_of`; `careTeam` is served and never changes; and removing a member
/// removes the care relationship.
#[tokio::test]
async fn a_care_team_group_writes_member_of_under_the_grant() {
    let (_db, _service, app) = hospital().await;
    let (user, _) = created(&app, CLIENT, "carla", "carla-login").await;
    let (status, team) = group(&app, CLIENT, "Psychiatry", &[&user], true).await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    assert_eq!(team[GROUP_EXTENSION]["careTeam"], json!(true), "{team}");
    let team = id_of(&team);
    let held = relationships(&app, &user).await;
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(
        held[0]["name"]["defining_code"]["code_string"], "member_of",
        "the membership is named by the coded member_of: {held:?}"
    );
    let request = common::req("GET", format!("{BASE}/demographic/group/{team}"))
        .auth(&system_token(ADT))
        .accept("application/json")
        .build();
    let (status, _, read) = common::send(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    let read: Value = serde_json::from_str(&read).expect("a GROUP");
    assert_eq!(
        read["name"]["defining_code"]["code_string"], "care_team",
        "the GROUP is named by the coded care_team: {read}"
    );

    let flip = patch_op(json!([{ "op": "replace",
        "path": format!("{GROUP_EXTENSION}:careTeam"), "value": false }]));
    let (status, _, refused) = call(
        &app,
        "PATCH",
        &format!("/Groups/{team}"),
        CLIENT,
        Some(&flip),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["scimType"], "mutability", "{refused}");

    let remove = patch_op(json!([{ "op": "remove",
        "path": format!("members[value eq \"{user}\"]") }]));
    let (status, _, removed) = call(
        &app,
        "PATCH",
        &format!("/Groups/{team}"),
        CLIENT,
        Some(&remove),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{removed}");
    assert_eq!(relationships(&app, &user).await, Vec::<Value>::new());

    let (status, _, plain) = call(&app, "GET", &format!("/Groups/{team}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{plain}");
    assert_eq!(plain[GROUP_EXTENSION]["careTeam"], json!(true), "{plain}");
}

/// A client without the grant creates no care-team Group, with or without
/// members, and its plain Groups write memberships with no care relationship.
#[tokio::test]
async fn a_care_team_group_needs_the_grant() {
    let (_db, _service, app) = hospital().await;
    let (user, _) = created(&app, OTHER, "dirk", "dirk-login").await;
    for members in [&[][..], &[user.as_str()][..]] {
        let (status, refused) = group(&app, OTHER, "Emergency", members, true).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert!(
            refused.to_string().contains("write_care_relationship"),
            "{refused}"
        );
    }
    let (status, plain) = group(&app, OTHER, "Emergency", &[&user], false).await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    assert!(plain.get(GROUP_EXTENSION).is_none(), "{plain}");
    let held = relationships(&app, &user).await;
    assert_eq!(held.len(), 1, "{held:?}");
    assert!(!carries_member_of(&held[0]), "{held:?}");
}

/// A SCIM client without the grant changes the plain Groups of a User the
/// ADT system rostered (#5640): removing a membership held before the roster
/// moves the roster's position and changes no care relationship, and the
/// roster stays.
#[tokio::test]
async fn a_group_change_leaves_the_adt_roster_in_place() {
    let (_db, _service, app) = hospital().await;
    let (user, _) = created(&app, OTHER, "gina", "gina-login").await;
    let (_, first) = group(&app, OTHER, "Night shift", &[&user], false).await;
    let first = id_of(&first);
    let (_, ward) = group(&app, OTHER, "Ward 3", &[], false).await;
    let ward = id_of(&ward);

    let (_, version, mut person) = read_person(&app, ADT, &user).await;
    let mut held = person["relationships"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    held.push(member_of(&user, &ward));
    person["relationships"] = Value::Array(held);
    let (status, answer) = write_person(&app, ADT, (&user, &version), &person).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{answer}");
    assert_eq!(relationships(&app, &user).await.len(), 2);

    let remove = patch_op(json!([{ "op": "remove",
        "path": format!("members[value eq \"{user}\"]") }]));
    let (status, _, removed) = call(
        &app,
        "PATCH",
        &format!("/Groups/{first}"),
        OTHER,
        Some(&remove),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{removed}");
    let (_, again) = group(&app, OTHER, "Day shift", &[&user], false).await;
    assert!(again["id"].is_string(), "{again}");
    let held = relationships(&app, &user).await;
    assert_eq!(held.len(), 2, "{held:?}");
    assert_eq!(
        held.iter().filter(|r| carries_member_of(r)).count(),
        1,
        "{held:?}"
    );
}
