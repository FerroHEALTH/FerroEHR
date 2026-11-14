// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The record's person and their representative, signing in through the
//! patient portal with tokens that carry no role, end to end over the
//! assembled router with the RBAC gate on, on a real `PostgreSQL` 18.
//!
//! Their right comes from who they are (EHDS Art. 3 and 4(2)), so the role
//! gate lets them through and the access evaluator decides (#5633). A login
//! acting for the person is a representative whichever issuer it signed in
//! through, held to the `delegation` directives of the person's
//! `EHR_ACCESS` (#5613). The competent person writes their own `EHR_ACCESS`
//! while no gate-keeper is named (BASE Architecture Overview §Access
//! Control, #5636). Which caller passes the role gate is our own design.

use http::StatusCode;
use serde_json::{Value, json};

use crate::access_log_reads_http::{Fixture, fixture, get, root, stored};
use crate::common::{self, BASE};
use crate::portal_representatives_http::{CHILD_LOGIN, PARENT, token};

/// The child's own portal token, with no role.
fn patient(ehr: &str) -> String {
    token(CHILD_LOGIN, ehr, &json!({}))
}

/// The parent's portal token under their own login, naming the child's EHR,
/// with no role.
fn parent(ehr: &str) -> String {
    token(PARENT, ehr, &json!({}))
}

/// The status and body of `method uri` as `credential`, with `body`.
async fn send(
    fx: &Fixture,
    method: &str,
    uri: &str,
    credential: &str,
    body: Option<&Value>,
) -> (StatusCode, String) {
    let req = common::req(method, uri)
        .auth(credential)
        .accept("application/json")
        .prefer_representation();
    let req = match body {
        Some(body) => req.json(body.to_string()),
        None => req,
    };
    common::send_body(&fx.app, req.build()).await
}

/// The IPS template uploaded and one IPS composition committed to the
/// child's EHR, as the administrator; the composition's versioned-object id.
async fn composition(fx: &Fixture) -> String {
    let (status, _, body) = common::send(
        &fx.app,
        common::req("POST", format!("{BASE}/definition/template/adl1.4"))
            .auth(&root())
            .header(http::header::CONTENT_TYPE, "application/xml")
            .body(common::ips_opt_xml())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    let (status, headers, body) = common::send(
        &fx.app,
        common::req("POST", format!("{BASE}/ehr/{}/composition", fx.ehr))
            .auth(&root())
            .json(common::ips_canonical_composition().to_string())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    headers
        .get(http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .split("::")
        .next()
        .expect("object id")
        .to_owned()
}

/// The `refusal` member of a problem body.
fn refusal(body: &str) -> Value {
    serde_json::from_str::<Value>(body).unwrap_or(Value::Null)["refusal"].clone()
}

/// A coded term of the access control groups.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID",
                    "value": ferroehr::party_binding::policy_mark::TERMINOLOGY },
                "code_string": code } })
}

/// A portal token with no role passes the RBAC gate for the person and for
/// the parent acting for them, and the evaluator alone holds each to the
/// child's EHR: another EHR is refused with the step-0 code, never the role
/// gate's reason.
#[tokio::test]
async fn a_portal_token_with_no_role_reaches_the_evaluator() {
    let fx = fixture().await;
    for (who, credential) in [
        ("the person", patient(&fx.ehr)),
        ("the parent", parent(&fx.ehr)),
    ] {
        let (status, body) = send(
            &fx,
            "GET",
            &format!("{BASE}/ehr/{}", fx.ehr),
            &credential,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{who}: {body}");
    }
    let (status, body) = send(
        &fx,
        "GET",
        &format!("{BASE}/ehr/{}", fx.elsewhere),
        &patient(&fx.ehr),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body), json!("refused-not-subject"), "{body}");
    assert!(!body.contains("at least one role"), "{body}");
}

/// The parent, signing in to the patient portal under their own login, is a
/// representative: a `delegation` permit naming them that covers another
/// template narrows them to it, so the child's IPS composition is refused
/// with `refused-consent-denied` and the refused entry records that code.
/// The child is still the subject and reads it, recorded as the
/// subject of care.
#[tokio::test]
async fn a_portal_representative_takes_the_delegation_directives() {
    let fx = fixture().await;
    let vo = composition(&fx).await;
    let delegation = json!({
        "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "delegation" },
        "archetype_node_id": "at0001",
        "kind": term("delegation"),
        "effect": term("permit"),
        "emergency_override": false,
        "applies_to": [{
            "party": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                       "id": { "_type": "HIER_OBJECT_ID", "value": fx.parent.to_string() } },
            "relationship": term("representative")
        }],
        "data": [{ "_type": "ACCESS_DATA_SCOPE", "templates": ["Vaccination record"] }]
    });
    common::commit_access_settings(
        &fx.svc,
        fx.ehr.parse().expect("an EHR id"),
        common::directive_settings(&[delegation]),
    )
    .await;
    let read = format!("{BASE}/ehr/{}/composition/{vo}", fx.ehr);
    let (status, body) = send(&fx, "GET", &read, &parent(&fx.ehr), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(refusal(&body), json!("refused-consent-denied"), "{body}");
    let refused = stored(&fx.pool, 1, |entry| {
        entry["outcome"]["result"]["value"] == json!("refused")
            && entry["accessor"]["kind"]["value"] == json!("representative")
    })
    .await;
    let entry = refused.last().expect("the refused entry");
    assert_eq!(
        entry["outcome"]["reason"]["defining_code"]["code_string"], "refused-consent-denied",
        "{entry}"
    );

    let (status, body) = send(&fx, "GET", &read, &patient(&fx.ehr), None).await;
    assert_eq!(status, StatusCode::OK, "the child reads it: {body}");
    stored(&fx.pool, 1, |entry| {
        entry["accessor"]["kind"]["value"] == json!("subject of care")
            && entry["items"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item["rm_type"] == json!("COMPOSITION"))
            })
    })
    .await;
}

/// The competent person commits their own first directives with a
/// CONTRIBUTION while no gate-keeper is named; once those settings name a
/// gate-keeper who is not the person, the person's next write is refused.
#[tokio::test]
async fn the_person_writes_their_own_directives_until_a_gate_keeper_is_named() {
    let fx = fixture().await;
    let (status, ehr) = get(&fx.app, &format!("{BASE}/ehr/{}", fx.ehr), &root()).await;
    assert_eq!(status, StatusCode::OK, "{ehr}");
    // NOTE: RM common master04 §PARTY_SELF, the person's CONTRIBUTION names its committer
    // as the record's person.
    let contribution = |settings: &Value, preceding: u32| {
        let mut body = crate::ehr_access_gate::access_contribution_over(
            &ehr,
            settings,
            preceding,
            CHILD_LOGIN,
        );
        for pointer in ["/audit", "/versions/0/commit_audit"] {
            if let Some(audit) = body.pointer_mut(pointer).and_then(Value::as_object_mut) {
                audit.insert("committer".to_owned(), json!({ "_type": "PARTY_SELF" }));
            }
        }
        body
    };
    let uri = format!("{BASE}/ehr/{}/contribution", fx.ehr);
    let own = common::access_settings("restricted", &[]);
    let (status, body) = send(
        &fx,
        "POST",
        &uri,
        &patient(&fx.ehr),
        Some(&contribution(&own, 1)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the person's own: {body}");
    let guarded = common::access_settings("restricted", &[fx.parent.0]);
    let (status, body) = send(
        &fx,
        "POST",
        &uri,
        &patient(&fx.ehr),
        Some(&contribution(&guarded, 2)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the person names a gate-keeper: {body}"
    );
    let (status, body) = send(
        &fx,
        "POST",
        &uri,
        &patient(&fx.ehr),
        Some(&contribution(&own, 3)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "named gate-keepers without the person take their write away: {body}"
    );
}
