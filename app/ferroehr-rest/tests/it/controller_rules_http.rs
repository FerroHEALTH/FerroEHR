// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's rules at the wire (#5621, #5624, #5627): a rule of the
//! `ACCESS_POLICY` names a role, which the caller holds by performing a
//! `ROLE` whose coded `name` is that role (RM Demographic IM `PARTY.type()`),
//! and decides as step 5 of the access evaluator for every EHR; a named
//! datum is judged before any step about its EHR alone. EHDS Art. 11(3) asks
//! for rules by category of professional and of data. The rules, their step
//! and the reading of a role are our own design.

use axum::Router;
use axum::body::Body;
use ferroehr::service::FerroEhrService;
use http::{Request, StatusCode, header};
use openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS;
use serde_json::{Value, json};

use crate::access_evaluator_http::{app, credential, ehr_with_composition, queried, refusal, send};
use crate::common;
use crate::common::{BASE, EXT};

/// The jurisdiction's role code system.
const ROLES: &str = "urn:example:roles";
/// The HL7 v3 `ActCode` sensitivity code system.
const ACT_CODE: &str = "2.16.840.1.113883.5.25";
/// The template of the IPS composition the tests commit.
const IPS: &str = "International Patient Summary";

/// A coded value of `terminology`.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// A term of the access control groups.
fn term(code: &str) -> Value {
    coded(TERMINOLOGY_ID_ACCESS, code)
}

/// An `access effect`: a code of HL7 FHIR `consent-provision-type`.
fn provision(code: &str) -> Value {
    coded(
        openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_CONSENT_PROVISION_TYPE,
        code,
    )
}

/// A rule of `effect` with `extra` members.
fn rule(effect: &str, extra: &Value) -> Value {
    let mut r = json!({ "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "Rule" }, "archetype_node_id": "id15",
        "kind": term("controller_rule"), "effect": provision(effect), "emergency_override": false });
    if let (Some(r), Some(extra)) = (r.as_object_mut(), extra.as_object()) {
        r.extend(extra.clone());
    }
    r
}

/// An actor scope naming the role `role` of [`ROLES`].
fn role(role: &str) -> Value {
    json!({ "_type": "ACCESS_ACTOR_SCOPE", "role": coded(ROLES, role) })
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

/// The Basic user `user` performs a `ROLE` named the role `code` of [`ROLES`].
async fn performs(svc: &FerroEhrService, pool: &sqlx::PgPool, user: &str, code: &str) {
    let person = common::bound_person(pool, common::BASIC_LOGIN, user)
        .await
        .expect("the user's PERSON");
    crate::system_grants::performs_role(
        svc,
        ("PERSON", ferroehr::ids::VoId(person)),
        &coded(ROLES, code),
    )
    .await;
    svc.grants().refresh().await.expect("the copy reads");
}

/// Commits the IPS composition into `ehr` as `user`; the status and body.
async fn commit(app: &Router, ehr: &str, user: &str) -> (StatusCode, String) {
    send(
        app,
        "POST",
        &format!("{BASE}/ehr/{ehr}/composition"),
        user,
        None,
        Some((
            "application/json",
            common::ips_canonical_composition().to_string(),
        )),
    )
    .await
}

/// A nurse may not commit the template her role's rule denies, on any EHR,
/// whatever the patient's directives permit; another professional commits it,
/// and the nurse still reads it.
#[tokio::test]
async fn a_rule_refuses_the_nurse_the_template_her_role_may_not_write() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    performs(&svc, &pool, "nurse", "NURSE").await;
    let nurse = common::bound_person(&pool, common::BASIC_LOGIN, "nurse")
        .await
        .expect("the nurse's PERSON");
    let mut settings = common::access_settings("open", &[]);
    settings["directives"] = json!([crate::access_evaluator_http::directive(
        "consent",
        "permit",
        &crate::access_evaluator_http::naming(nurse),
    )]);
    common::commit_access_settings(&svc, ehr.parse().expect("an EHR id"), settings).await;
    put_policy(
        &app,
        &json!({ "rules": [rule("deny", &json!({ "applies_to": [role("NURSE")],
            "operations": [term("write")],
            "data": [{ "_type": "ACCESS_DATA_SCOPE", "templates": [IPS] }] }))] }),
    )
    .await;
    let (status, body) = commit(&app, &ehr, "nurse").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body).0, json!("refused-policy-denied"), "{body}");
    let (status, body) = commit(&app, &ehr, "psychiatrist").await;
    assert_eq!(status, StatusCode::CREATED, "another professional: {body}");
    let read = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::OK, "the nurse reads: {body}");
}

/// A classified datum is refused for its sensitivity before the policy's
/// care relationship is asked, and a permit rule naming the caller's role
/// clears it (#5627).
#[tokio::test]
async fn a_named_datum_is_judged_before_its_ehr_and_a_rule_clears_a_role() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    performs(&svc, &pool, "psychiatrist", "PSYCHIATRIST").await;
    let sensitive = json!([coded(ACT_CODE, "PSY")]);
    put_policy(
        &app,
        &json!({
            "classifications": [{ "_type": "ACCESS_CLASSIFICATION",
                "name": { "_type": "DV_TEXT", "value": "Psychiatric notes" },
                "archetype_node_id": "id2",
                "data": { "_type": "ACCESS_DATA_SCOPE", "templates": [IPS] },
                "sensitivities": sensitive }],
            "care_relationship_required": true,
            "rules": [rule("permit", &json!({ "applies_to": [role("PSYCHIATRIST")],
                "data": [{ "_type": "ACCESS_DATA_SCOPE", "sensitivities": sensitive }] }))]
        }),
    )
    .await;
    let read = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body).0, json!("refused-sensitivity"), "{body}");
    let (status, body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{ehr}"),
        "nurse",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        refusal(&body).0,
        json!("refused-no-care-relationship"),
        "the EHR itself is judged by the EHR's steps: {body}"
    );
    let (status, body) = send(&app, "GET", &read, "psychiatrist", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        refusal(&body).0,
        json!("refused-no-care-relationship"),
        "cleared, the psychiatrist meets the care step next: {body}"
    );
    assert!(queried(&app, "nurse", None).await.is_empty(), "the query");
}
