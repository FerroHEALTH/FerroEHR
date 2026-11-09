// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `/scim/v2/Groups` (#5015) through the assembled router: the RFC 7643 §8.4
//! Group, the RFC 7644 §3.5.2.1 member `add` and §3.5.2.2 member `remove`
//! examples read from the vendored RFCs, each `GROUP` and `PERSON` version and
//! `PARTY_RELATIONSHIP` they leave, and the refusals. A Group is a demographic
//! `GROUP` whose members each hold an inline `PARTY_RELATIONSHIP` from their
//! `PERSON` to it (#5220), never a `ROLE`. The mapping is our own design.

use std::sync::Arc;

use axum::Router;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::binding::PartyBinding;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::party::{Outcome, PartyRef};
use ferrohealth_smart::resource::Caller;
use http::{StatusCode, header};
use serde_json::{Value, json};

use super::{
    CLIENT, GROUPS, LOGIN, OTHER, PERSONS, app, app_auditing, app_of, app_with_service, call,
    config, created, patch_op,
};
use crate::scim_protocol::source::{self, Doc};

/// The binding key of the `n`th User a test creates.
fn key(n: u32) -> String {
    format!("{n:08x}-7f76-453a-919d-413861904646")
}

/// Creates the `n`th User of `client`; its id.
async fn user(app: &Router, client: &str, n: u32) -> String {
    created(app, client, &format!("user{n}"), &key(n)).await.0
}

/// A Group create body named `name` with `members`.
fn group_body(name: &str, external_id: Option<&str>, members: &[&str]) -> Value {
    let mut body = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
        "displayName": name,
        "members": members.iter().map(|id| json!({ "value": id })).collect::<Vec<_>>(),
    });
    if let Some(external_id) = external_id {
        body["externalId"] = json!(external_id);
    }
    body
}

/// Creates the Group `name` of `client` with `members`; its id and entity
/// tag.
async fn group(app: &Router, client: &str, name: &str, members: &[&str]) -> (String, String) {
    let (status, headers, body) = call(
        app,
        "POST",
        "/Groups",
        client,
        Some(&group_body(name, None, members)),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().expect("an id").to_owned();
    let tag = headers[header::ETAG]
        .to_str()
        .expect("an entity tag")
        .to_owned();
    (id, tag)
}

/// The member ids the Group `id` of `client` serves, sorted.
async fn members(app: &Router, client: &str, id: &str) -> Vec<String> {
    let (status, _, body) = call(app, "GET", &format!("/Groups/{id}"), client, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut ids: Vec<String> = body["members"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|member| member["value"].as_str().expect("a value").to_owned())
        .collect();
    ids.sort();
    ids
}

/// `ids`, sorted.
fn sorted(ids: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    out.sort();
    out
}

/// Sends a PATCH of `operations` to the Group `id` of `client`; the status and the
/// entity tag.
async fn patch(app: &Router, client: &str, id: &str, operations: Value) -> (StatusCode, String) {
    let (status, headers, body) = call(
        app,
        "PATCH",
        &format!("/Groups/{id}"),
        client,
        Some(&patch_op(operations)),
        &[],
    )
    .await;
    let tag = headers
        .get(header::ETAG)
        .and_then(|tag| tag.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        status != StatusCode::OK,
        "a Group PATCH answers 204 or a refusal, never a body: {body}"
    );
    (status, tag)
}

/// Each stored version of the party `id`: its lifecycle state, its
/// CONTRIBUTION and its body (`null` for a deletion).
async fn versions(pool: &sqlx::PgPool, id: &str) -> Vec<(String, String, Value)> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT lifecycle_state, contribution_id::text, body FROM party.version \
         WHERE vo_id = $1::uuid ORDER BY sys_version",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .expect("read the versions");
    rows.into_iter()
        .map(|(state, contribution, body)| {
            let body = body.map_or(Value::Null, |body| {
                serde_json::from_str(&body).expect("a canonical body")
            });
            (state, contribution, body)
        })
        .collect()
}

/// The `GROUP`s the membership relationships of the `PERSON` `body` target,
/// sorted, or `None` when `relationships` is Void.
fn memberships(body: &Value) -> Option<Vec<String>> {
    let relationships = body.get("relationships")?.as_array()?;
    let mut out: Vec<String> = relationships
        .iter()
        .map(|relationship| {
            assert_eq!(relationship["_type"], "PARTY_RELATIONSHIP");
            assert_eq!(relationship["source"]["type"], "PERSON", "{relationship}");
            assert_eq!(relationship["target"]["type"], "GROUP", "{relationship}");
            relationship["target"]["id"]["value"]
                .as_str()
                .expect("a target")
                .to_owned()
        })
        .collect();
    out.sort();
    Some(out)
}

/// The entity tag the Group `id` of `client` serves.
async fn group_tag(app: &Router, client: &str, id: &str) -> String {
    let (status, headers, body) =
        call(app, "GET", &format!("/Groups/{id}"), client, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    headers[header::ETAG]
        .to_str()
        .expect("an entity tag")
        .to_owned()
}

/// The RFC 7643 §8.4 Group, created with two Users of the client, and the
/// RFC 7644 §3.5.2.1 add and §3.5.2.2 remove examples applied to it.
#[tokio::test]
async fn the_rfc_group_and_member_examples_answer_as_documented() {
    let (_db, app) = app().await;
    let babs = user(&app, CLIENT, 1).await;
    let mandy = user(&app, CLIENT, 2).await;
    let james = user(&app, CLIENT, 3).await;
    let mut tour_guides = source::json(Doc::Rfc7643, 2364..=2390, &[]);
    for (member, id) in [(0, &babs), (1, &mandy)] {
        tour_guides["members"][member]["value"] = json!(id);
        tour_guides["members"][member]["$ref"] =
            json!(format!("https://cdr.example.com/scim/v2/Users/{id}"));
    }
    let (status, headers, body) =
        call(&app, "POST", "/Groups", CLIENT, Some(&tour_guides), &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["displayName"], "Tour Guides");
    let id = body["id"].as_str().expect("an id").to_owned();
    assert_ne!(
        id, "e9e30dba-f08f-4109-8486-d5c6a331660a",
        "the id is the server's"
    );
    let location = format!("https://cdr.example.com/scim/v2/Groups/{id}");
    assert_eq!(headers[header::LOCATION], location.as_str());
    assert_eq!(body["meta"]["resourceType"], "Group");
    assert_eq!(body["meta"]["location"], location.as_str());
    assert_eq!(
        headers[header::ETAG],
        body["meta"]["version"].as_str().expect("a version")
    );
    for member in body["members"].as_array().expect("members") {
        let value = member["value"].as_str().expect("a value");
        assert_eq!(
            member["$ref"],
            format!("https://cdr.example.com/scim/v2/Users/{value}").as_str()
        );
        assert_eq!(member["type"], "User");
    }
    assert_eq!(members(&app, CLIENT, &id).await, sorted(&[&babs, &mandy]));

    let mut add = source::json(Doc::Rfc7644, 2033..=2049, &[]);
    add["Operations"][0]["value"][0]["value"] = json!(james);
    let (status, headers, body) = call(
        &app,
        "PATCH",
        &format!("/Groups/{id}"),
        CLIENT,
        Some(&add),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(
        headers.contains_key(header::ETAG),
        "RFC 7644 §3.5.2.1 answers the ETag"
    );
    assert_eq!(
        members(&app, CLIENT, &id).await,
        sorted(&[&babs, &mandy, &james])
    );

    let mut remove = source::json(Doc::Rfc7644, 2201..=2208, &[]);
    let elided = remove["Operations"][0]["path"]
        .as_str()
        .expect("a path")
        .replace("2819c223-7f76-...413861904646", &babs);
    remove["Operations"][0]["path"] = json!(elided);
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
    assert_eq!(members(&app, CLIENT, &id).await, sorted(&[&mandy, &james]));

    let remove_all = source::json(Doc::Rfc7644, 2219..=2224, &[]);
    let (status, _, body) = call(
        &app,
        "PATCH",
        &format!("/Groups/{id}"),
        CLIENT,
        Some(&remove_all),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(members(&app, CLIENT, &id).await, Vec::<String>::new());
}

/// Creating an empty Group, adding two members, removing one, renaming the
/// Group and deleting it each leave one `GROUP` version with the inline
/// memberships described, `relationships` Void when it holds none, and no
/// `ROLE` beyond the Users' own.
#[tokio::test]
async fn each_step_leaves_its_versions_with_the_memberships_and_no_role() {
    let (_db, pool, app) = app_auditing(false).await;
    let first = user(&app, CLIENT, 1).await;
    let second = user(&app, CLIENT, 2).await;
    let (id, created_tag) = group(&app, CLIENT, "Ward 3", &[]).await;

    let add = json!([{ "op": "add", "path": "members",
                       "value": [{ "value": first }, { "value": second }] }]);
    let (status, added_tag) = patch(&app, CLIENT, &id, add).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_ne!(
        added_tag, created_tag,
        "a membership change moves the Group's version"
    );

    let remove = json!([{ "op": "remove", "path": format!("members[value eq \"{first}\"]") }]);
    let (status, removed_tag) = patch(&app, CLIENT, &id, remove).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_ne!(removed_tag, added_tag);
    let rename = json!([{ "op": "replace", "path": "displayName", "value": "Ward 4" }]);
    assert_eq!(
        patch(&app, CLIENT, &id, rename).await.0,
        StatusCode::NO_CONTENT
    );
    let (status, _, body) = call(&app, "GET", &format!("/Groups/{id}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["displayName"], "Ward 4");
    assert_eq!(members(&app, CLIENT, &id).await, sorted(&[&second]));
    let (status, _, _) = call(&app, "DELETE", &format!("/Groups/{id}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The GROUP is versioned by its creation, its rename and its deletion
    // only, and never holds a membership.
    let group_versions = versions(&pool, &id).await;
    assert_eq!(group_versions.len(), 3, "create, rename, delete");
    assert_eq!(group_versions[0].2["_type"], "GROUP");
    for version in group_versions.iter().take(2) {
        assert!(version.2.get("relationships").is_none(), "{}", version.2);
    }
    assert_eq!(
        group_versions[2].2,
        Value::Null,
        "the delete stores no body"
    );
    assert_eq!(group_versions[2].0, "523");

    // Each member holds its membership: the add versions both PERSONs in one
    // CONTRIBUTION, the remove the first, the delete the second.
    let first_versions = versions(&pool, &first).await;
    let second_versions = versions(&pool, &second).await;
    let first_added = first_versions.iter().rev().nth(1).expect("the add");
    let first_removed = first_versions.last().expect("the remove");
    assert_eq!(memberships(&first_added.2), Some(vec![id.clone()]));
    assert_eq!(memberships(&first_removed.2), None);
    let second_added = second_versions.iter().rev().nth(1).expect("the add");
    let second_dropped = second_versions.last().expect("the delete's drop");
    assert_eq!(memberships(&second_added.2), Some(vec![id.clone()]));
    assert_eq!(memberships(&second_dropped.2), None);
    assert_eq!(first_added.1, second_added.1, "one CONTRIBUTION per change");
    assert_ne!(first_removed.1, first_added.1);
    assert_eq!(
        second_dropped.1, group_versions[2].1,
        "the Group's deletion drops its members' memberships in its CONTRIBUTION"
    );
    let roles: i64 =
        sqlx::query_scalar("SELECT count(DISTINCT vo_id) FROM party.version WHERE kind = 'ROLE'")
            .fetch_one(&pool)
            .await
            .expect("count the ROLEs");
    assert_eq!(
        roles, 2,
        "each User performs one ROLE, and no Group mints one"
    );
    for gone in ["GET", "PATCH", "DELETE"] {
        let body =
            (gone == "PATCH").then(|| patch_op(json!([{ "op": "remove", "path": "members" }])));
        let (status, _, _) = call(
            &app,
            gone,
            &format!("/Groups/{id}"),
            CLIENT,
            body.as_ref(),
            &[],
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{gone} after the delete (RFC 7644 §3.6)"
        );
    }
}

/// A membership change gives the Group and the member User a new entity
/// tag, since the User's `PERSON` holds the membership (RFC 7643 §4.1.2,
/// #5220); the User's body is unchanged, as it serves no `groups`, and
/// removing the last member leaves its `relationships` Void.
#[tokio::test]
async fn a_membership_changes_the_groups_tag_and_the_users() {
    let (_db, pool, app) = app_auditing(false).await;
    let member = user(&app, CLIENT, 1).await;
    let user_path = format!("/Users/{member}");
    let (_, before_headers, before) = call(&app, "GET", &user_path, CLIENT, None, &[]).await;
    let (id, tag) = group(&app, CLIENT, "Theatre", &[]).await;
    let add = json!([{ "op": "add", "path": "members", "value": [{ "value": member }] }]);
    let (_, added) = patch(&app, CLIENT, &id, add).await;
    assert_ne!(added, tag);
    let (_, after_headers, after) = call(&app, "GET", &user_path, CLIENT, None, &[]).await;
    assert_ne!(after_headers[header::ETAG], before_headers[header::ETAG]);
    let without_meta = |body: &Value| {
        let mut body = body.clone();
        body.as_object_mut().expect("a User").remove("meta");
        body
    };
    assert_eq!(
        without_meta(&after),
        without_meta(&before),
        "the User serves no groups attribute"
    );

    let (status, removed) = patch(
        &app,
        CLIENT,
        &id,
        json!([{ "op": "remove", "path": "members" }]),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_ne!(removed, added);
    let stored = versions(&pool, &member).await;
    assert_eq!(memberships(&stored.last().expect("a head").2), None);
}

/// Deleting a User deletes the memberships its `PERSON` holds with it, in
/// one CONTRIBUTION that versions no `GROUP`, and each Group's version
/// moves.
#[tokio::test]
async fn deleting_a_user_removes_its_memberships_in_the_same_contribution() {
    let (_db, pool, app) = app_auditing(false).await;
    let leaver = user(&app, CLIENT, 1).await;
    let stayer = user(&app, CLIENT, 2).await;
    let (shared, _) = group(&app, CLIENT, "Shared", &[&leaver, &stayer]).await;
    let (alone, _) = group(&app, CLIENT, "Alone", &[&leaver]).await;
    let tags = [
        group_tag(&app, CLIENT, &shared).await,
        group_tag(&app, CLIENT, &alone).await,
    ];
    let (status, _, body) = call(
        &app,
        "DELETE",
        &format!("/Users/{leaver}"),
        CLIENT,
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    for (group, tag) in [&shared, &alone].into_iter().zip(tags) {
        assert_eq!(
            versions(&pool, group).await.len(),
            1,
            "{group} keeps its version"
        );
        assert_ne!(
            group_tag(&app, CLIENT, group).await,
            tag,
            "{group}'s version moves"
        );
    }
    assert_eq!(members(&app, CLIENT, &shared).await, sorted(&[&stayer]));
    assert_eq!(members(&app, CLIENT, &alone).await, Vec::<String>::new());
}

/// A `remove` of `members` with a value list (Microsoft Entra ID) and one
/// with the RFC 7644 §3.5.2.2 value filter each remove only the members
/// they name; one with neither removes them all.
#[tokio::test]
async fn both_remove_forms_remove_only_the_named_members() {
    let (_db, app) = app().await;
    let ids = [
        user(&app, CLIENT, 1).await,
        user(&app, CLIENT, 2).await,
        user(&app, CLIENT, 3).await,
        user(&app, CLIENT, 4).await,
    ];
    let all: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (id, _) = group(&app, CLIENT, "Clinic", &all).await;

    let listed = json!([{ "op": "Remove", "path": "members", "value": [{ "value": ids[0] }] }]);
    assert_eq!(
        patch(&app, CLIENT, &id, listed).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(members(&app, CLIENT, &id).await, sorted(&all[1..]));

    let filtered = json!([{ "op": "remove",
                            "path": format!("members[value eq \"{}\"]", ids[1]) }]);
    assert_eq!(
        patch(&app, CLIENT, &id, filtered).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(members(&app, CLIENT, &id).await, sorted(&all[2..]));

    let everything = json!([{ "op": "remove", "path": "members" }]);
    assert_eq!(
        patch(&app, CLIENT, &id, everything).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(members(&app, CLIENT, &id).await, Vec::<String>::new());
}

/// A duplicate `displayName` (by its fold), a nested Group member, another
/// client's User as a member, an add past `max_group_members`, an `If-Match`
/// naming an older version, and another client's PATCH are refused with
/// `409`, `400`, `400`, `400`, `412` and `404`.
#[tokio::test]
async fn the_group_refusals_answer_their_status() {
    let mut capped = config();
    capped.scim.max_group_members = 2;
    let (_db, _pool, _service, app) = app_of(capped, false).await;
    let ours = [
        user(&app, CLIENT, 1).await,
        user(&app, CLIENT, 2).await,
        user(&app, CLIENT, 3).await,
    ];
    let theirs = user(&app, OTHER, 9).await;
    let (id, tag) = group(&app, CLIENT, "Ward 5", &[&ours[0]]).await;

    let (status, _, body) = call(
        &app,
        "POST",
        "/Groups",
        CLIENT,
        Some(&group_body("WARD 5", None, &[])),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["scimType"], "uniqueness");

    let (other_group, _) = group(&app, CLIENT, "Ward 6", &[]).await;
    for (member, why) in [
        (
            json!({ "value": other_group, "type": "Group" }),
            "a Group by its type",
        ),
        (json!({ "value": other_group }), "a Group by its id"),
        (
            json!({ "value": ours[1], "$ref": format!("https://cdr.example.com/scim/v2/Groups/{other_group}") }),
            "a Group by its $ref",
        ),
        (json!({ "value": theirs }), "another client's User"),
        (json!({ "value": "not-an-id" }), "no id"),
    ] {
        let add = json!([{ "op": "add", "path": "members", "value": [member] }]);
        let (status, _, body) = call(
            &app,
            "PATCH",
            &format!("/Groups/{id}"),
            CLIENT,
            Some(&patch_op(add)),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {body}");
        assert_eq!(body["scimType"], "invalidValue", "{why}");
    }

    let past = json!([{ "op": "add", "path": "members",
                        "value": [{ "value": ours[1] }, { "value": ours[2] }] }]);
    let (status, _, body) = call(
        &app,
        "PATCH",
        &format!("/Groups/{id}"),
        CLIENT,
        Some(&patch_op(past)),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["scimType"], "invalidValue");

    let add = json!([{ "op": "add", "path": "members", "value": [{ "value": ours[1] }] }]);
    let (status, _) = patch(&app, CLIENT, &id, add.clone()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = call(
        &app,
        "PATCH",
        &format!("/Groups/{id}"),
        CLIENT,
        Some(&patch_op(add)),
        &[("if-match", &tag)],
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);

    for method in ["GET", "PATCH", "PUT", "DELETE"] {
        let body = match method {
            "PATCH" => Some(patch_op(json!([{ "op": "remove", "path": "members" }]))),
            "PUT" => Some(group_body("Taken", None, &[])),
            _ => None,
        };
        let (status, _, _) = call(
            &app,
            method,
            &format!("/Groups/{id}"),
            OTHER,
            body.as_ref(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "another client's {method}");
    }
    assert_eq!(
        members(&app, CLIENT, &id).await,
        sorted(&[&ours[0], &ours[1]])
    );
}

/// A PUT replaces the `displayName` and the members, keeps an omitted
/// `externalId`, and answers `200` with the Group.
#[tokio::test]
async fn put_replaces_the_members_and_keeps_the_external_id() {
    let (_db, app) = app().await;
    let first = user(&app, CLIENT, 1).await;
    let second = user(&app, CLIENT, 2).await;
    let (status, _, body) = call(
        &app,
        "POST",
        "/Groups",
        CLIENT,
        Some(&group_body("Ward 7", Some("ward-7"), &[&first])),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().expect("an id").to_owned();
    let (status, _, body) = call(
        &app,
        "PUT",
        &format!("/Groups/{id}"),
        CLIENT,
        Some(&group_body("Ward 8", None, &[&second])),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["displayName"], "Ward 8");
    assert_eq!(body["externalId"], "ward-7");
    assert_eq!(members(&app, CLIENT, &id).await, sorted(&[&second]));
}

/// A person token naming two of the three Groups its `PERSON` belongs to
/// binds with exactly those two; a token with no group claim binds with
/// none.
#[tokio::test]
async fn a_token_naming_two_of_a_persons_groups_lists_exactly_those_two() {
    let (_db, service, app) = app_with_service().await;
    let member = user(&app, CLIENT, 1).await;
    let mut named = Vec::new();
    for (name, external_id) in [
        ("Ward", "ward"),
        ("Theatre", "theatre"),
        ("Clinic", "clinic"),
    ] {
        let (status, _, body) = call(
            &app,
            "POST",
            "/Groups",
            CLIENT,
            Some(&group_body(name, Some(external_id), &[&member])),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        if external_id != "theatre" {
            named.push(PartyRef::new(
                "demographic",
                "GROUP",
                body["id"].as_str().expect("an id"),
            ));
        }
    }
    let table = PartySection {
        issuer: PERSONS.to_owned(),
        identifier_issuer: LOGIN.to_owned(),
        identifier_type: "guid".to_owned(),
        group_claim: "groups".to_owned(),
        group_identifier_issuer: GROUPS.to_owned(),
        ..PartySection::default()
    };
    let profile = ferrohealth_smart::profile::Profile::default();
    let binding = PartyBinding::new(Arc::new(service.party_source("demographic")))
        .with_issuer(
            table,
            ferrohealth_smart::kind::IssuerKind::Persons,
            &profile,
        )
        .expect("the table binds");
    let resolve = |claims: serde_json::Map<String, Value>| {
        let caller = Caller::new(
            "subject".to_owned(),
            Some("app".to_owned()),
            "user/*.read",
            None,
            false,
            CallerKind::Person,
        )
        .with_issuer(PERSONS.to_owned())
        .with_person_key(Some(key(1)), None)
        .with_claims(claims);
        let binding = &binding;
        async move { binding.resolve(&caller).await.expect("the source answers") }
    };
    let mut claims = serde_json::Map::new();
    claims.insert("groups".to_owned(), json!(["ward", "clinic", "unknown"]));
    let Some(Outcome::Party(bound)) = resolve(claims).await else {
        panic!("the person binds");
    };
    let mut groups = bound.groups.clone();
    groups.sort();
    named.sort();
    assert_eq!(groups, named);
    let Some(Outcome::Party(bound)) = resolve(serde_json::Map::new()).await else {
        panic!("the person binds");
    };
    assert!(bound.groups.is_empty(), "{:?}", bound.groups);
}

/// Neither a standalone relationship container nor a `GROUP`'s own inline
/// relationship to a User makes a member: only the membership the member's
/// `PERSON` holds does (#5220). Deleting the User leaves an administrator's
/// `GROUP` with its one version.
#[tokio::test]
async fn standalone_and_group_held_relationships_never_count() {
    let (_db, pool, service, app) = app_of(config(), false).await;
    let leaver = user(&app, CLIENT, 1).await;
    let (owned, _) = group(&app, CLIENT, "Owned", &[]).await;
    let reference = |class: &str, id: &str| {
        json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": class,
                "id": { "_type": "HIER_OBJECT_ID", "value": id } })
    };
    let membership = |source: (&str, &str), target: (&str, &str)| {
        json!({
            "_type": "PARTY_RELATIONSHIP",
            "archetype_node_id": super::MEMBERSHIP,
            "name": { "_type": "DV_TEXT", "value": "Membership" },
            "source": reference(source.0, source.1),
            "target": reference(target.0, target.1),
        })
    };
    let standalone = openehr_its::json::from_canonical_value(&membership(
        ("PERSON", &leaver),
        ("GROUP", &owned),
    ))
    .expect("a PARTY_RELATIONSHIP");
    service
        .party_relationship_create(ferroehr::service::version_update::WriteRequest::new(
            standalone,
        ))
        .await
        .expect("the standalone relationship commits");
    assert_eq!(members(&app, CLIENT, &owned).await, Vec::<String>::new());

    let allocated = service.allocate_party_id();
    let admin = allocated.id().to_string();
    let mut body = versions(&pool, &owned).await[0].2.clone();
    let object = body.as_object_mut().expect("a GROUP");
    object.remove("uid");
    object.insert(
        "relationships".to_owned(),
        json!([membership(("GROUP", &admin), ("PERSON", &leaver))]),
    );
    body["identities"][0]["details"]["items"][0]["value"]["value"] = json!("Administrators' team");
    service
        .create_parties(
            vec![ferroehr::service::demographic::party_set::NewParty {
                id: allocated,
                kind: ferroehr::service::demographic::types::PartyKind::Group,
                body,
                lifecycle_state: None,
            }],
            "an administrator's GROUP",
        )
        .await
        .expect("the server writes the GROUP as an administrator");
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.binding_membership WHERE group_id = $1::uuid",
    )
    .bind(&admin)
    .fetch_one(&pool)
    .await
    .expect("count the rows");
    assert_eq!(rows, 0, "a GROUP's own relationship makes no member");

    let (status, _, body) = call(
        &app,
        "DELETE",
        &format!("/Users/{leaver}"),
        CLIENT,
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(versions(&pool, &admin).await.len(), 1);
}

/// Sets the `active` of the User `id` of `client`; the status.
async fn set_active(app: &Router, client: &str, id: &str, active: bool) -> StatusCode {
    let operations = json!([{ "op": "replace", "path": "active", "value": active }]);
    let (status, _, body) = call(
        app,
        "PATCH",
        &format!("/Users/{id}"),
        client,
        Some(&patch_op(operations)),
        &[],
    )
    .await;
    assert!(status.is_success(), "{status}: {body}");
    status
}

/// Microsoft Entra ID disables a User and then removes it from its Groups:
/// the deactivation drops the memberships its `PERSON` holds in the same
/// version, each Group's version moves, and the later remove finds nothing to
/// remove and answers success (RM common master06 §Version Lifecycle, #5220).
#[tokio::test]
async fn a_disabled_user_leaves_its_groups_and_the_later_remove_succeeds() {
    let (_db, pool, app) = app_auditing(false).await;
    let leaver = user(&app, CLIENT, 1).await;
    let stayer = user(&app, CLIENT, 2).await;
    let (id, _) = group(&app, CLIENT, "Ward 9", &[&leaver, &stayer]).await;
    let before = group_tag(&app, CLIENT, &id).await;
    set_active(&app, CLIENT, &leaver, false).await;
    let head = versions(&pool, &leaver).await;
    let deactivated = head.last().expect("the deactivation");
    assert_eq!(deactivated.0, "800");
    assert_eq!(
        memberships(&deactivated.2),
        None,
        "the deactivation drops the memberships"
    );
    assert_eq!(members(&app, CLIENT, &id).await, sorted(&[&stayer]));
    assert_ne!(
        group_tag(&app, CLIENT, &id).await,
        before,
        "the Group's version moves"
    );

    let remove = json!([{ "op": "remove", "path": format!("members[value eq \"{leaver}\"]") }]);
    assert_eq!(
        patch(&app, CLIENT, &id, remove).await.0,
        StatusCode::NO_CONTENT
    );
    let entra = json!([{ "op": "Remove", "path": "members", "value": [{ "value": leaver }] }]);
    assert_eq!(
        patch(&app, CLIENT, &id, entra).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(members(&app, CLIENT, &id).await, sorted(&[&stayer]));
    assert_eq!(
        versions(&pool, &leaver).await.len(),
        head.len(),
        "no version of the inactive PERSON"
    );
}

/// An add naming an inactive User is refused with `400 invalidValue`, the
/// cause named; a reactivated User belongs to no Group until its client adds
/// it again.
#[tokio::test]
async fn an_inactive_user_takes_no_membership_and_comes_back_without_one() {
    let (_db, _pool, app) = app_auditing(false).await;
    let member = user(&app, CLIENT, 1).await;
    let (id, _) = group(&app, CLIENT, "Ward 10", &[&member]).await;
    set_active(&app, CLIENT, &member, false).await;
    let add = json!([{ "op": "add", "path": "members", "value": [{ "value": member }] }]);
    let (status, _, body) = call(
        &app,
        "PATCH",
        &format!("/Groups/{id}"),
        CLIENT,
        Some(&patch_op(add.clone())),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["scimType"], "invalidValue", "{body}");
    assert!(
        body["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("reactivate the User first")),
        "{body}"
    );

    set_active(&app, CLIENT, &member, true).await;
    assert_eq!(
        members(&app, CLIENT, &id).await,
        Vec::<String>::new(),
        "a reactivated User comes back with no membership"
    );
    assert_eq!(
        patch(&app, CLIENT, &id, add).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(members(&app, CLIENT, &id).await, sorted(&[&member]));
}

/// Deleting a Group drops the membership of each active member in its
/// CONTRIBUTION and versions no inactive member, whose deactivation already
/// dropped it.
#[tokio::test]
async fn deleting_a_group_skips_its_inactive_members() {
    let (_db, pool, app) = app_auditing(false).await;
    let active = user(&app, CLIENT, 1).await;
    let inactive = user(&app, CLIENT, 2).await;
    let (id, _) = group(&app, CLIENT, "Ward 11", &[&active, &inactive]).await;
    set_active(&app, CLIENT, &inactive, false).await;
    let inactive_versions = versions(&pool, &inactive).await.len();
    let (status, _, body) = call(&app, "DELETE", &format!("/Groups/{id}"), CLIENT, None, &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let deleted = versions(&pool, &id).await;
    let active_head = versions(&pool, &active).await;
    let dropped = active_head.last().expect("the drop");
    assert_eq!(memberships(&dropped.2), None);
    assert_eq!(
        dropped.1,
        deleted.last().expect("the delete").1,
        "one CONTRIBUTION"
    );
    assert_eq!(versions(&pool, &inactive).await.len(), inactive_versions);
}
