// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access evaluator on a write and on an EHR the store does not hold,
//! over the assembled router and a real `PostgreSQL` 18.
//!
//! A write the subject's directives refuse is refused before any version is
//! stored, judged on the content it carries as a read of that content would
//! be (RM EHR IM §EHR Access; BASE Architecture Overview §Access Control,
//! #5626), apart from a restriction or an opt-out, which restrict who sees
//! data and never its recording (EHDS Art. 8; owner ruling on #5621). An EHR id that names no EHR answers `404` before any step refuses,
//! exactly as a restricted EHR does, so a restriction stays invisible (EHDS
//! Art. 8, #5623). Both are our own design.

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};

use crate::access_evaluator_http::{
    app, credential, directive, ehr_with_composition, naming, refusal, send,
};
use crate::common::{self, BASE, EXT};

/// A coded term of the access control groups.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID",
                    "value": ferroehr::party_binding::policy_mark::TERMINOLOGY },
                "code_string": code } })
}

/// The number of compositions `ehr` holds, as the administrator counts them.
async fn compositions(app: &Router, ehr: &str) -> u64 {
    let q = json!({
        "q": format!("SELECT COUNT(c/uid/value) FROM EHR e[ehr_id/value='{ehr}'] CONTAINS COMPOSITION c")
    });
    let (status, body) = send(
        app,
        "POST",
        &format!("{BASE}/query/aql"),
        "root",
        None,
        Some(("application/json", q.to_string())),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "count: {body}");
    serde_json::from_str::<Value>(&body).expect("RESULT_SET")["rows"][0][0]
        .as_u64()
        .expect("a count")
}

/// `user`'s commit of the IPS composition to `ehr`.
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

/// The nurse's write of a composition a consent deny covers by its template
/// is refused `403 refused-consent-denied` before anything is stored: the EHR
/// still holds the one composition, and the access log records the refusal
/// with its code.
#[tokio::test]
async fn a_write_a_directive_refuses_is_never_stored() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (ehr, _) = ehr_with_composition(&app).await;
    let nurse = common::basic_person(&pool, "nurse").await;
    let template = json!([{ "_type": "ACCESS_DATA_SCOPE",
                            "templates": ["International Patient Summary"] }]);
    let deny = directive(
        "consent",
        "deny",
        &json!({ "applies_to": naming(nurse), "operations": [term("write")], "data": template }),
    );
    common::commit_access_settings(
        &svc,
        ehr.parse().expect("an EHR id"),
        common::directive_settings(&[deny]),
    )
    .await;
    let (status, body) = commit(&app, &ehr, "nurse").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (member, detail) = refusal(&body);
    assert_eq!(
        member,
        json!("refused-consent-denied"),
        "a write refusal carries the refusal member a read refusal carries: {body}"
    );
    assert!(detail.starts_with("refused-consent-denied"), "{body}");
    assert_eq!(compositions(&app, &ehr).await, 1, "nothing was stored");
    let refused = crate::access_log_http::entries(&pool, 1, |stored| {
        stored.result.as_deref() == Some("refused")
    })
    .await;
    let entry: Value = serde_json::from_str(&refused[0].content).expect("a canonical entry");
    assert_eq!(
        entry["outcome"]["reason"]["defining_code"]["code_string"], "refused-consent-denied",
        "{entry}"
    );
}

/// A restriction covering a template restricts who reads it, never its
/// recording: the nurse's write in the restricted category is stored and
/// answered `201` with its representation, built from the version her request
/// committed; her next read of it is refused with the `404` of absent data,
/// and the access log holds exactly the create and the refused read (EHDS
/// Art. 8; owner ruling on #5621).
#[tokio::test]
async fn a_write_in_a_restricted_category_is_stored_and_not_read_back() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (ehr, _) = ehr_with_composition(&app).await;
    let nurse = common::basic_person(&pool, "nurse").await;
    let mut restriction = common::access_directive("restriction", &[]);
    restriction["data"] = json!([{ "_type": "ACCESS_DATA_SCOPE",
                                   "templates": ["International Patient Summary"] }]);
    restriction["applies_to"] = naming(nurse);
    restriction["emergency_override"] = json!(false);
    common::commit_access_settings(
        &svc,
        ehr.parse().expect("an EHR id"),
        common::directive_settings(&[restriction]),
    )
    .await;
    let (status, body) = commit(&app, &ehr, "nurse").await;
    assert_eq!(status, StatusCode::CREATED, "the write is stored: {body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).expect("the representation")["_type"],
        "COMPOSITION",
        "{body}"
    );
    assert_eq!(compositions(&app, &ehr).await, 2, "the version was stored");
    let vo = serde_json::from_str::<Value>(&body).expect("the composition")["uid"]["value"]
        .as_str()
        .and_then(|uid| uid.split("::").next())
        .expect("the composition uid")
        .to_owned();
    let read = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    let (status, body) = send(&app, "GET", &read, "nurse", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(!body.contains("refused-"), "{body}");
    let refused = crate::access_log_http::entries(&pool, 1, |stored| {
        stored.result.as_deref() == Some("refused")
    })
    .await;
    assert_eq!(refused.len(), 1, "one refused read");
    let naming_it =
        crate::access_log_http::entries(&pool, 1, |stored| stored.content.contains(&vo)).await;
    let actions: Vec<(Option<&str>, Option<&str>)> = naming_it
        .iter()
        .map(|stored| (stored.action.as_deref(), stored.result.as_deref()))
        .filter(|(_, result)| *result != Some("refused"))
        .collect();
    assert_eq!(
        actions,
        vec![(Some("create"), Some("granted"))],
        "the response is recorded as the create, and no read of it but the refused one"
    );
    let refused = &refused[0];
    let entry: Value = serde_json::from_str(&refused.content).expect("a canonical entry");
    assert_eq!(
        entry["outcome"]["reason"]["defining_code"]["code_string"], "refused-restricted",
        "{entry}"
    );
}

/// `body` less what differs between any two answers, the problem's
/// `instance` and the EHR id `ehr` it names.
fn unique(body: &str, ehr: &str) -> String {
    let mut problem: Value = serde_json::from_str(body).expect("a problem body");
    problem
        .as_object_mut()
        .map(|problem| problem.remove("instance"));
    problem.to_string().replace(ehr, "{ehr_id}")
}

/// Under a policy that requires a care relationship, an EHR id that names no
/// EHR answers the nurse `404` before step 5 refuses, and a restricted EHR
/// answers her the same: the two answers have one status and one body but
/// for the EHR id each names. An EHR she has no care relationship with and
/// no restriction answers `403 refused-no-care-relationship`.
#[tokio::test]
async fn an_unknown_ehr_answers_as_a_restricted_one() {
    let (_db, pool, svc, app) = Box::pin(app()).await;
    let (restricted, _) = ehr_with_composition(&app).await;
    let (uncared, _) = ehr_with_composition(&app).await;
    let mut restriction = common::access_directive("restriction", &[]);
    restriction["applies_to"] = naming(common::basic_person(&pool, "nurse").await);
    common::commit_access_settings(
        &svc,
        restricted.parse().expect("an EHR id"),
        common::directive_settings(&[restriction]),
    )
    .await;
    let archetype = "openEHR-EHR-ACCESS_POLICY.controller.v1";
    let policy = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Hospital access policy" },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0" },
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
        "care_relationship_required": true,
        "purpose_required": false
    });
    let (status, _, body) = common::send(
        &app,
        Request::put(format!("{EXT}/admin/access_policy"))
            .header(header::AUTHORIZATION, credential("root"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_NONE_MATCH, "*")
            .body(Body::from(policy.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the policy: {body}");
    let unknown = "0f0e0d0c-0b0a-4908-8706-050403020100";
    let read = |ehr: &str| format!("{BASE}/ehr/{ehr}");
    let (status, uncared_body) = send(&app, "GET", &read(&uncared), "nurse", None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{uncared_body}");
    assert_eq!(
        refusal(&uncared_body).0,
        json!("refused-no-care-relationship"),
        "{uncared_body}"
    );
    let (unknown_status, unknown_body) =
        send(&app, "GET", &read(unknown), "nurse", None, None).await;
    let (restricted_status, restricted_body) =
        send(&app, "GET", &read(&restricted), "nurse", None, None).await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND, "{unknown_body}");
    assert_eq!(restricted_status, unknown_status, "{restricted_body}");
    assert_eq!(
        unique(&restricted_body, &restricted),
        unique(&unknown_body, unknown),
        "the restricted EHR answers byte for byte as the unknown one"
    );
}
