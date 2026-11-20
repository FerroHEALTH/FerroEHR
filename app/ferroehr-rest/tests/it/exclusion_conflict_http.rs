// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's exclusion and the conflict refusals at the wire (#6077;
//! draft access control decision procedure, steps 0 and 5): an exclusion
//! keeps the named caller out of one record, answered forbidden without the
//! exclusion or its reason, and the access log names it; only the controller
//! reads and writes it. A professional performing a `ROLE` is refused its
//! own record and a relative's, in a read and in a query, unless a conflict
//! rule of the policy allows it. Our own design over that procedure.

use axum::Router;
use axum::body::Body;
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use http::{Request, StatusCode, header};
use openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::access_evaluator_http::{
    app, credential, directive, ehr_with_composition, naming, queried, refusal, send,
};
use crate::common;
use crate::common::{BASE, EXT};

/// The jurisdiction's role code system.
const ROLES: &str = "urn:example:roles";
/// The reason the controller gives for the exclusion, which no answer names.
const REASON: &str = "Former partner of the patient";

/// A coded value of `terminology`.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// The exclusion of the `PERSON` `person`, its reason in its name.
fn exclusion(person: uuid::Uuid) -> Value {
    let mut excluded = directive(
        "exclusion",
        "deny",
        &json!({ "applies_to": naming(person) }),
    );
    excluded["name"] = json!({ "_type": "DV_TEXT", "value": REASON });
    excluded
}

/// Open settings naming `keepers` gate-keepers, with `exclusions`.
fn settings(keepers: &[uuid::Uuid], exclusions: &[Value]) -> Value {
    let mut settings = common::access_settings("open", keepers);
    if !exclusions.is_empty() {
        settings["exclusions"] = json!(exclusions);
    }
    settings
}

/// The Basic user `user` performs a `ROLE` named the role `code` of [`ROLES`].
async fn performs(svc: &FerroEhrService, pool: &PgPool, user: &str, code: &str) {
    let person = common::basic_person(pool, user).await;
    crate::system_grants::performs_role(
        svc,
        ("PERSON", ferroehr::ids::VoId(person)),
        &coded(ROLES, code),
    )
    .await;
    svc.grants().refresh().await.expect("the copy reads");
}

/// The EHR of the `PERSON` `person`, with one IPS composition, as the
/// administrator; the EHR and the composition's versioned-object id.
async fn ehr_of(app: &Router, person: uuid::Uuid) -> (String, String) {
    let status = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" } },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": { "_type": "PARTY_SELF", "external_ref": {
            "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": person.to_string() } } },
        "is_queryable": true,
        "is_modifiable": true
    });
    let (code, body) = send(
        app,
        "POST",
        &format!("{BASE}/ehr"),
        "root",
        None,
        Some(("application/json", status.to_string())),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "EHR create: {body}");
    let ehr = serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let (code, body) = send(
        app,
        "POST",
        &format!("{BASE}/ehr/{ehr}/composition"),
        "root",
        None,
        Some((
            "application/json",
            common::ips_canonical_composition().to_string(),
        )),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "commit: {body}");
    let vo = serde_json::from_str::<Value>(&body).expect("a composition")["uid"]["value"]
        .as_str()
        .expect("uid")
        .split("::")
        .next()
        .expect("object id")
        .to_owned();
    (ehr, vo)
}

/// Commits, as the server, the kinship `code` of the openEHR `subject
/// relationship` group between the `PERSON`s `source` and `target`.
async fn related(svc: &FerroEhrService, source: uuid::Uuid, target: uuid::Uuid, code: &str) {
    let person = |id: uuid::Uuid| {
        json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() } })
    };
    let archetype = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.kinship.v0";
    let body = json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "name": coded(TERMINOLOGY_ID_ACCESS, "related_to"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "details" },
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0002",
                        "name": coded(TERMINOLOGY_ID_ACCESS, "kinship"),
                        "value": coded("openehr", code) }] },
        "source": person(source),
        "target": person(target)
    });
    let held = openehr_its::json::from_canonical_value(&body).expect("a PARTY_RELATIONSHIP");
    let mut write = WriteRequest::new(held);
    write.lifecycle_state = Some("532".to_owned());
    Box::pin(svc.party_relationship_create(write))
        .await
        .expect("the kinship commits");
    svc.grants().refresh().await.expect("the copy reads");
}

/// Writes the controller's first policy with `members` as the administrator.
async fn put_policy(app: &Router, members: &Value) {
    let archetype = "openEHR-EHR-ACCESS_POLICY.controller.v1";
    let mut policy = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Hospital access policy" },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0" },
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
        "care_relationship_required": false,
        "purpose_required": false
    });
    if let (Some(p), Some(members)) = (policy.as_object_mut(), members.as_object()) {
        p.extend(members.clone());
    }
    let (status, _, body) = common::send(
        app,
        Request::put(format!("{EXT}/admin/access_policy"))
            .header(header::AUTHORIZATION, credential("root"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_NONE_MATCH, "*")
            .body(Body::from(policy.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the policy: {body}");
}

/// The current `EHR_ACCESS` of `ehr_id` as the controller reads it: the
/// server, outside any request.
async fn controller_read(svc: &FerroEhrService, ehr_id: ferroehr::ids::EhrId) -> String {
    svc.ehr_object(
        ehr_id,
        ferroehr::service::ehr::object::EhrObject::Access,
        None,
    )
    .await
    .expect("the controller reads the EHR_ACCESS")
    .body
    .to_string()
}

/// The canonical entry of the first stored access-log entry of `ehr` that
/// `keep` selects.
async fn entry_of(pool: &PgPool, ehr: &str, keep: impl Fn(&Value) -> bool) -> Value {
    let read = |stored: &crate::access_log_http::Stored| {
        serde_json::from_str::<Value>(&stored.content).expect("a canonical entry")
    };
    let kept = crate::access_log_http::entries(pool, 1, |stored| {
        stored.ehr_id.as_deref() == Some(ehr) && keep(&read(stored))
    })
    .await;
    read(&kept[0])
}

/// An exclusion refuses the nurse it names as forbidden, with the refusal
/// code and neither the exclusion nor its reason, in a read and a query; the
/// access log names the exclusion; the psychiatrist reads the record.
#[tokio::test]
async fn an_excluded_caller_is_refused_without_a_reason_and_the_log_names_the_exclusion() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let nurse = common::basic_person(&pool, "nurse").await;
    let ehr_id = ehr.parse().expect("an EHR id");
    common::commit_access_settings(&svc, ehr_id, settings(&[], &[exclusion(nurse)])).await;
    let read = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body).0, json!("refused-excluded"), "{body}");
    assert!(!body.contains(REASON), "the answer names no reason: {body}");
    assert!(queried(&app, "nurse", None).await.is_empty(), "the query");
    let (status, body) = send(&app, "GET", &read, "psychiatrist", None, None).await;
    assert_eq!(status, StatusCode::OK, "anyone else: {body}");
    let refused = entry_of(&pool, &ehr, |entry| {
        entry["outcome"]["reason"]["defining_code"]["code_string"] == "refused-excluded"
    })
    .await;
    let step = refused["outcome"]["description"]["value"]
        .as_str()
        .unwrap_or_default();
    assert!(
        step.starts_with("step 0 (exclusion): exclusion 1 of EHR_ACCESS "),
        "{refused}"
    );
}

/// The controller (the server, as an administrator is) reads the
/// exclusions; a gate-keeper reads the settings
/// without them, and a version a gate-keeper commits carries them forward
/// unchanged, while one that changes them is refused whole.
#[tokio::test]
async fn only_the_controller_reads_and_writes_the_exclusions() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (ehr, _) = ehr_with_composition(&app).await;
    let (nurse, psychiatrist) = (
        common::basic_person(&pool, "nurse").await,
        common::basic_person(&pool, "psychiatrist").await,
    );
    let ehr_id = ehr.parse().expect("an EHR id");
    common::commit_access_settings(&svc, ehr_id, settings(&[nurse], &[exclusion(psychiatrist)]))
        .await;
    let current = format!("{EXT}/ehr/{ehr}/ehr_access");
    let controller = controller_read(&svc, ehr_id).await;
    assert!(
        controller.contains("\"exclusions\""),
        "the controller reads them: {controller}"
    );
    let (status, body) = send(&app, "GET", &current, "nurse", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.contains("exclusions"),
        "a gate-keeper reads none: {body}"
    );
    assert!(!body.contains(REASON), "{body}");

    let (status, ehr_json) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{ehr}"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ehr_json}");
    let ehr_json: Value = serde_json::from_str(&ehr_json).expect("an EHR");
    let contribution = |scheme: &Value, preceding: u32| {
        crate::ehr_access_gate::access_contribution_over(&ehr_json, scheme, preceding, "nurse")
            .to_string()
    };
    let path = format!("{BASE}/ehr/{ehr}/contribution");
    let changed = settings(&[nurse], &[exclusion(nurse)]);
    let (status, body) = send(
        &app,
        "POST",
        &path,
        "nurse",
        None,
        Some(("application/json", contribution(&changed, 2))),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a gate-keeper changes no exclusion: {body}"
    );
    let (status, body) = send(
        &app,
        "POST",
        &path,
        "nurse",
        None,
        Some((
            "application/json",
            contribution(&settings(&[nurse], &[]), 2),
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the gate-keeper's version: {body}"
    );
    let controller = controller_read(&svc, ehr_id).await;
    assert!(
        controller.contains(REASON),
        "the exclusion is carried forward: {controller}"
    );
    let read = format!("{BASE}/ehr/{ehr}");
    let (status, body) = send(&app, "GET", &read, "psychiatrist", None, None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "it still binds the psychiatrist: {body}"
    );
}

/// A professional reads its own record as the record's person until it
/// performs a professional `ROLE`; then the read and the query are refused
/// `refused-conflict` under the default rule, and a conflict rule naming it
/// admits it, marked in the access log.
#[tokio::test]
async fn staff_reading_their_own_record_as_a_professional_are_refused_conflict() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let nurse = common::basic_person(&pool, "nurse").await;
    let (own, vo) = ehr_of(&app, nurse).await;
    let (_, other) = ehr_with_composition(&app).await;
    let read = format!("{BASE}/ehr/{own}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::OK, "the subject path: {body}");

    performs(&svc, &pool, "nurse", "NURSE").await;
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body).0, json!("refused-conflict"), "{body}");
    let rows = queried(&app, "nurse", None).await;
    assert!(
        !rows.contains(&vo),
        "the query leaves her record out: {rows:?}"
    );
    assert!(rows.contains(&other), "and reads the others: {rows:?}");

    put_policy(
        &app,
        &json!({ "conflict_rules": [{ "_type": "ACCESS_CONFLICT_RULE",
            "conflict": coded(TERMINOLOGY_ID_ACCESS, "self"),
            "applies_to": naming(nurse) }] }),
    )
    .await;
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::OK, "the rule allows it: {body}");
    assert!(queried(&app, "nurse", None).await.contains(&vo));
    let allowed = entry_of(&pool, &own, |entry| {
        entry["authorisations"].as_array().is_some_and(|grounds| {
            grounds
                .iter()
                .any(|g| g["kind"]["value"] == "conflict allowance")
        })
    })
    .await;
    assert_eq!(
        allowed["outcome"]["result"]["value"], "granted",
        "{allowed}"
    );
}

/// A professional related to the subject is refused the record in a read
/// and in a query under the conflict rule; a conflict rule of that kinship
/// admits it, and kinship never admits anyone.
#[tokio::test]
async fn a_professional_related_to_the_subject_is_refused_in_a_read_and_in_aql() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (nurse, psychiatrist) = (
        common::basic_person(&pool, "nurse").await,
        common::basic_person(&pool, "psychiatrist").await,
    );
    let (mother, vo) = ehr_of(&app, nurse).await;
    let (_, other) = ehr_with_composition(&app).await;
    performs(&svc, &pool, "psychiatrist", "PSYCHIATRIST").await;
    related(&svc, psychiatrist, nurse, "254").await;
    let read = format!("{BASE}/ehr/{mother}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "psychiatrist", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body).0, json!("refused-conflict"), "{body}");
    let rows = queried(&app, "psychiatrist", None).await;
    assert!(
        !rows.contains(&vo),
        "the query leaves the relative out: {rows:?}"
    );
    assert!(rows.contains(&other), "{rows:?}");

    put_policy(
        &app,
        &json!({ "conflict_rules": [{ "_type": "ACCESS_CONFLICT_RULE",
            "conflict": coded(TERMINOLOGY_ID_ACCESS, "related"),
            "kinships": [coded("openehr", "254")] }] }),
    )
    .await;
    let (status, body) = send(&app, "GET", &read, "psychiatrist", None, None).await;
    assert_eq!(status, StatusCode::OK, "the rule of that kinship: {body}");
    assert!(queried(&app, "psychiatrist", None).await.contains(&vo));
}
