// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals of the demographic routes on the wire: a versioned party or
//! relationship this server does not hold is a `404`, a malformed
//! `version_at_time` is a `400`, and a `time_validity` whose bounds are not
//! `DV_DATE` is a `422`.

use axum::Router;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common;

/// A versioned-object id no write ever creates.
const VO: &str = "7d3c2b1a-0f9e-4d8c-b7a6-5e4d3c2b1a0f";

async fn status_of(app: &Router, relative: &str) -> (StatusCode, String) {
    common::send_body(app, common::get(&common::full(relative))).await
}

#[tokio::test]
async fn an_unknown_versioned_party_is_404() {
    let (_db, app) = common::app(common::api_config(false)).await;
    for path in [
        format!("/demographic/versioned_party/{VO}"),
        format!("/demographic/versioned_party/{VO}/revision_history"),
        format!("/demographic/versioned_party/{VO}/version"),
    ] {
        let (status, body) = status_of(&app, &path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }
}

#[tokio::test]
async fn an_unknown_versioned_relationship_is_404() {
    let (_db, app) = common::app(common::api_config(false)).await;
    for path in [
        format!("/demographic/versioned_party_relationship/{VO}"),
        format!("/demographic/versioned_party_relationship/{VO}/revision_history"),
    ] {
        let (status, body) = status_of(&app, &path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }
}

#[tokio::test]
async fn a_malformed_version_at_time_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let path = format!("/demographic/person/{VO}?version_at_time=not-a-time");
    let (status, body) = status_of(&app, &path).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// A closed `DV_INTERVAL` whose two bounds are of `bound`.
fn interval(bound: &str, lower: &str, upper: &str) -> Value {
    json!({
        "_type": "DV_INTERVAL",
        "lower": { "_type": bound, "value": lower },
        "upper": { "_type": bound, "value": upper },
        "lower_unbounded": false, "upper_unbounded": false,
        "lower_included": true, "upper_included": true
    })
}

/// An `ITEM_TREE` of `node` holding one text `ELEMENT`.
fn tree(node: &str) -> Value {
    json!({
        "_type": "ITEM_TREE", "archetype_node_id": node,
        "name": { "_type": "DV_TEXT", "value": "structure" },
        "items": [{
            "_type": "ELEMENT", "archetype_node_id": "at0099",
            "name": { "_type": "DV_TEXT", "value": "text" },
            "value": { "_type": "DV_TEXT", "value": "x" }
        }]
    })
}

/// A `PARTY_REF` to the `PERSON` container `id`.
fn person_ref(id: &str) -> Value {
    json!({
        "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
        "id": { "_type": "HIER_OBJECT_ID", "value": id }
    })
}

/// An archetype root of `rm_type` and `archetype`, named `name`, with the
/// container id `uid`.
fn root(rm_type: &str, archetype: &str, name: &str, uid: &str) -> Value {
    json!({
        "_type": rm_type,
        "uid": { "_type": "HIER_OBJECT_ID", "value": uid },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name }
    })
}

/// Creates two PERSONs and returns their container ids: the parties a ROLE's
/// `performer` and a relationship's ends name (RM demographic `role.adoc`
/// `performer`, master02 §Party Relationships).
async fn held_persons(app: &Router) -> (String, String) {
    let mut ids = Vec::new();
    for name in ["Performer", "Partner"] {
        let mut person = root(
            "PERSON",
            "openEHR-DEMOGRAPHIC-PERSON.person.v1",
            name,
            &uuid::Uuid::now_v7().to_string(),
        );
        person["identities"] = json!([{
            "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" }, "details": tree("at0002")
        }]);
        let request = common::req("POST", common::full("/demographic/person"))
            .json(person.to_string())
            .build();
        let (status, headers, text) = common::send(app, request).await;
        assert_eq!(status, StatusCode::CREATED, "{name}: {text}");
        let uid = common::etag_uid(&headers);
        ids.push(uid.split("::").next().expect("a container id").to_owned());
    }
    let partner = ids.pop().expect("two persons");
    let performer = ids.pop().expect("two persons");
    (performer, partner)
}

/// Every demographic `time_validity` set to `validity`, each as the route
/// segment that takes the body, the body, and the attribute the refusal names;
/// the ROLEs are performed by `performer`, and the relationship runs from it to
/// `partner`.
fn validity_bodies(
    validity: &Value,
    (performer, partner): (&str, &str),
) -> Vec<(&'static str, Value, &'static str)> {
    let identities = json!([{
        "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
        "name": { "_type": "DV_TEXT", "value": "legal name" }, "details": tree("at0002")
    }]);
    let role = |uid: &str| {
        let mut role = root("ROLE", "openEHR-DEMOGRAPHIC-ROLE.role.v1", "clinician", uid);
        role["identities"] = identities.clone();
        role["performer"] = person_ref(performer);
        role
    };
    let mut role_validity = role("11111111-1111-4111-8111-111111111111");
    role_validity["time_validity"] = validity.clone();
    let mut capability = role("22222222-2222-4222-8222-222222222222");
    capability["capabilities"] = json!([{
        "_type": "CAPABILITY", "archetype_node_id": "at0010",
        "name": { "_type": "DV_TEXT", "value": "registration" },
        "credentials": tree("at0011"), "time_validity": validity
    }]);
    let mut person = root(
        "PERSON",
        "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "Jane Doe",
        "33333333-3333-4333-8333-333333333333",
    );
    person["identities"] = identities;
    person["contacts"] = json!([{
        "_type": "CONTACT", "archetype_node_id": "at0020",
        "name": { "_type": "DV_TEXT", "value": "home" },
        "addresses": [{
            "_type": "ADDRESS", "archetype_node_id": "at0021",
            "name": { "_type": "DV_TEXT", "value": "address" }, "details": tree("at0022")
        }],
        "time_validity": validity
    }]);
    let mut relationship = root(
        "PARTY_RELATIONSHIP",
        "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "parent-of",
        "44444444-4444-4444-8444-444444444444",
    );
    relationship["source"] = person_ref(performer);
    relationship["target"] = person_ref(partner);
    relationship["time_validity"] = validity.clone();
    vec![
        ("role", role_validity, "$.time_validity.lower"),
        ("role", capability, "$.capabilities[0].time_validity.lower"),
        ("person", person, "$.contacts[0].time_validity.lower"),
        ("party_relationship", relationship, "$.time_validity.lower"),
    ]
}

/// RM 1.1.0 types every demographic `time_validity` `DV_INTERVAL<DV_DATE>`
/// (§Attributes of `ROLE`, `CAPABILITY`, `CONTACT` and `PARTY_RELATIONSHIP` in
/// `RM/docs/UML/classes/org.openehr.rm.demographic.*.adoc`), while the ITS-JSON
/// 1.1.0 schemas refer to `DV_INTERVAL` with its parameter open, so a body
/// with `DV_DATE_TIME` bounds passes the schema and is refused as content
/// that parses but breaks the RM: `422` (ITS-REST `responses/422.yaml`),
/// naming the bound and `DV_DATE`. The same bodies with `DV_DATE` bounds
/// commit.
#[tokio::test]
async fn a_time_validity_with_date_time_bounds_is_422() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let (performer, partner) = held_persons(&app).await;
    let parties = (performer.as_str(), partner.as_str());
    let date_time = interval(
        "DV_DATE_TIME",
        "2026-01-01T08:00:00Z",
        "2026-12-31T17:00:00Z",
    );
    for (segment, body, path) in validity_bodies(&date_time, parties) {
        let request = common::req("POST", common::full(&format!("/demographic/{segment}")))
            .json(body.to_string())
            .build();
        let (status, _h, text) = common::send(&app, request).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{segment} {path}: {text}"
        );
        assert!(
            text.contains(path) && text.contains("expected _type \\\"DV_DATE\\\""),
            "{segment}: names {path} and DV_DATE: {text}"
        );
    }
    let date = interval("DV_DATE", "2026-01-01", "2026-12-31");
    for (segment, body, path) in validity_bodies(&date, parties) {
        let request = common::req("POST", common::full(&format!("/demographic/{segment}")))
            .json(body.to_string())
            .build();
        let (status, _h, text) = common::send(&app, request).await;
        assert_eq!(status, StatusCode::CREATED, "{segment} {path}: {text}");
    }
}

/// An openEHR code `code` of the `openehr` terminology, as `DV_CODED_TEXT`.
fn openehr_code(code: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT", "value": code,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": code }
    })
}

/// The `553|incomplete|` lane lifts only the lower bounds (RM common master06
/// §Incomplete Content: data "may not be wrong"), so a demographic
/// CONTRIBUTION committing a party as incomplete still refuses `DV_DATE_TIME`
/// bounds in its `time_validity`.
#[tokio::test]
async fn an_incomplete_contribution_still_refuses_date_time_bounds() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let (performer, partner) = held_persons(&app).await;
    let date_time = interval(
        "DV_DATE_TIME",
        "2026-01-01T08:00:00Z",
        "2026-12-31T17:00:00Z",
    );
    let parties = validity_bodies(&date_time, (&performer, &partner))
        .into_iter()
        .filter(|(segment, _body, _path)| *segment != "party_relationship");
    for (_segment, body, path) in parties {
        let audit = json!({
            "change_type": openehr_code("249"),
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "committer" }
        });
        let contribution = json!({
            "versions": [{
                "data": body,
                "lifecycle_state": openehr_code("553"),
                "commit_audit": audit
            }],
            "audit": audit
        });
        let request = common::req("POST", common::full("/demographic/contribution"))
            .json(contribution.to_string())
            .build();
        let (status, _h, text) = common::send(&app, request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path}: {text}");
        assert!(
            text.contains(path) && text.contains("expected _type \\\"DV_DATE\\\""),
            "names {path} and DV_DATE: {text}"
        );
    }
}
