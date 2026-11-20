// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A record the controller marks restricted or very restricted, at the wire
//! (#6057): the `403 refused-confidentiality` that names neither level nor
//! reason, the `404` of a concealed level, a query and the look-up by
//! subject that leave the record out with counts and pages agreeing, the
//! patient's own read, the emergency, and the level in the access log
//! (draft access control, The Decision Procedure, step 6 and Refusals).

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};

use crate::access_evaluator_http::{app, credential, ehr_with_composition, queried, refusal, send};
use crate::common;
use crate::common::{BASE, EXT, TEST_SUBJECTS};

/// Open settings whose marking is `level`.
pub(crate) fn marked(level: &str) -> Value {
    let mut settings = common::directive_settings(&[]);
    settings["marking"] = json!({ "_type": "CONFIDENTIALITY_MARKING",
        "confidentiality": { "_type": "DV_CODED_TEXT", "value": level,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_Confidentiality" },
                "code_string": level } },
        "set_by": { "_type": "PARTY_IDENTIFIED", "name": "privacy officer" },
        "set_at": { "_type": "DV_DATE_TIME", "value": "2026-11-19T10:00:00Z" },
        "reason": { "_type": "DV_TEXT", "value": "public figure" } });
    settings
}

/// Stores the access policy `members` as the controller.
pub(crate) async fn put_policy(app: &Router, members: &Value) {
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
    if let (Some(policy), Some(members)) = (policy.as_object_mut(), members.as_object()) {
        policy.extend(members.clone());
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

/// The first column of each row `q` returns to `user`, with `extra` request
/// members.
async fn column(app: &Router, user: &str, q: &str, extra: &Value) -> Vec<Value> {
    let mut request = json!({ "q": q });
    if let (Some(request), Some(extra)) = (request.as_object_mut(), extra.as_object()) {
        request.extend(extra.clone());
    }
    let (status, body) = send(
        app,
        "POST",
        &format!("{BASE}/query/aql"),
        user,
        None,
        Some(("application/json", request.to_string())),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "query: {body}");
    serde_json::from_str::<Value>(&body).expect("RESULT_SET")["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row[0].clone())
        .collect()
}

/// A record marked `R` is refused to a professional with the code alone, on
/// the EHR and its data, and a population query leaves it out with `COUNT`
/// and every page agreeing; the other record is unchanged.
#[tokio::test]
async fn a_marked_record_is_refused_and_left_out_of_every_query() {
    let (_db, _pool, svc, app) = Box::pin(app()).await;
    let (marked_ehr, marked_vo) = ehr_with_composition(&app).await;
    let (_, open_vo) = ehr_with_composition(&app).await;
    common::commit_access_settings(&svc, marked_ehr.parse().expect("an EHR id"), marked("R")).await;
    for uri in [
        format!("{BASE}/ehr/{marked_ehr}"),
        format!("{BASE}/ehr/{marked_ehr}/composition/{marked_vo}"),
    ] {
        let (status, body) = send(&app, "GET", &uri, "nurse", None, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {body}");
        assert_eq!(refusal(&body).0, json!("refused-confidentiality"), "{body}");
        assert!(!body.contains("public figure"), "no reason: {body}");
        assert!(!body.contains("restricted"), "no level: {body}");
    }
    assert_eq!(
        queried(&app, "nurse", None).await,
        vec![open_vo.clone()],
        "the query"
    );
    let count = "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c";
    assert_eq!(
        column(&app, "nurse", count, &json!({})).await,
        vec![json!(1)]
    );
    let uids = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";
    let first = column(&app, "nurse", uids, &json!({ "offset": 0, "fetch": 1 })).await;
    assert_eq!(first.len(), 1, "the first page: {first:?}");
    assert!(
        column(&app, "nurse", uids, &json!({ "offset": 1, "fetch": 1 }))
            .await
            .is_empty(),
        "no second page"
    );
    assert_eq!(
        column(&app, "root", count, &json!({})).await,
        vec![json!(1)],
        "with no rule naming a level, the subject alone reads it, the administrator included"
    );
    let (status, body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{marked_ehr}/composition/{marked_vo}"),
        "drb",
        Some("ETREAT"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the emergency lifts it: {body}");
}

/// A policy rule that clears `R` for a treatment purpose admits the nurse
/// who declares it; a level the policy conceals answers `404`, as an
/// unknown record.
#[tokio::test]
async fn a_permit_rule_clears_a_level_and_a_concealed_level_is_absent() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (restricted, restricted_vo) = ehr_with_composition(&app).await;
    let (very, very_vo) = ehr_with_composition(&app).await;
    let nurse = common::basic_person(&pool, "nurse").await;
    common::commit_access_settings(&svc, restricted.parse().expect("an EHR id"), marked("R")).await;
    common::commit_access_settings(&svc, very.parse().expect("an EHR id"), marked("V")).await;
    let level = |code: &str| {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_Confidentiality" },
                "code_string": code } })
    };
    let mut clearing = crate::access_evaluator_http::directive(
        "controller_rule",
        "permit",
        &json!({ "applies_to": crate::access_evaluator_http::naming(nurse),
                 "purposes": [{ "_type": "DV_CODED_TEXT", "value": "TREAT",
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_ActReason" },
                        "code_string": "TREAT" } }],
                 "data": [{ "_type": "ACCESS_DATA_SCOPE", "confidentialities": [level("R")] }] }),
    );
    clearing["name"] = json!({ "_type": "DV_TEXT", "value": "The nurse reads R in treatment" });
    put_policy(
        &app,
        &json!({ "rules": [clearing], "concealed_confidentialities": [level("V")] }),
    )
    .await;
    let read = |ehr: &str, vo: &str| format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(
        &app,
        "GET",
        &read(&restricted, &restricted_vo),
        "nurse",
        Some("TREAT"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the rule clears R: {body}");
    let (status, body) = send(
        &app,
        "GET",
        &read(&restricted, &restricted_vo),
        "nurse",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no purpose: {body}");
    let (status, body) = send(
        &app,
        "GET",
        &read(&very, &very_vo),
        "nurse",
        Some("TREAT"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "V is concealed: {body}");
    assert!(!body.contains("refused-"), "{body}");
    assert_eq!(
        queried(&app, "nurse", Some("TREAT")).await,
        vec![restricted_vo],
        "the query reads R and leaves V out"
    );
}

/// The look-up of an EHR by its subject leaves a marked record out, while
/// its subject reads it, and the controller's commit and each refusal are
/// access-log entries naming the level.
#[tokio::test]
async fn the_look_up_leaves_a_marked_record_out_and_the_log_names_the_level() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/ehr"),
        "root",
        None,
        Some(("application/json", common::subject_status("vip-1"))),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let ehr = serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let lookup = format!("{BASE}/ehr?subject_id=vip-1&subject_namespace={TEST_SUBJECTS}");
    let (status, body) = send(&app, "GET", &lookup, "nurse", None, None).await;
    assert_eq!(status, StatusCode::OK, "found before the marking: {body}");
    common::commit_access_settings(&svc, ehr.parse().expect("an EHR id"), marked("V")).await;
    let (status, body) = send(&app, "GET", &lookup, "nurse", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "left out: {body}");
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
    let labelled = |content: &str| {
        let entry: Value = serde_json::from_str(content).expect("an entry");
        entry["security_labels"][0]["defining_code"]["code_string"] == json!("V")
    };
    let refused = crate::access_log_http::entries(&pool, 1, |stored| {
        stored.ehr_id.as_deref() == Some(ehr.as_str())
            && stored.result.as_deref() == Some("refused")
            && stored.content.contains("refused-confidentiality")
    })
    .await;
    assert!(
        refused.iter().all(|stored| labelled(&stored.content)),
        "each refusal carries the level: {refused:?}"
    );
    assert!(
        refused
            .iter()
            .any(|stored| stored.content.contains("the marking V")),
        "the refusal names the marking's level: {refused:?}"
    );
    let set = crate::access_log_http::entries(&pool, 1, |stored| {
        stored.ehr_id.as_deref() == Some(ehr.as_str())
            && stored.content.contains("ehr-access:ehr-wide")
            && labelled(&stored.content)
    })
    .await;
    assert!(!set.is_empty(), "the controller's commit names the level");
}
