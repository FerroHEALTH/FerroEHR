// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The settings of a marked record at the wire (#6165): a gate-keeper who is
//! not the subject reads the `EHR_ACCESS` without its marking, a version
//! with no signature, and the response says so in `Redacted`; the
//! controller reads them whole (draft access control, The Decision
//! Procedure, Reading and Writing the Settings, item 3).

use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};

use crate::access_confidentiality_http::{marked, put_policy};
use crate::access_evaluator_http::{app_with, credential, directive, ehr_with_composition};
use crate::common;
use crate::common::EXT;

/// The status, the `Redacted` header and the body of `GET uri` as
/// `user`, declaring a treatment purpose.
async fn read(app: &axum::Router, uri: &str, user: &str) -> (StatusCode, Option<String>, Value) {
    let (status, headers, body) = common::send(
        app,
        Request::get(uri)
            .header(header::AUTHORIZATION, credential(user))
            .header(header::ACCEPT, "application/json")
            .header("x-purpose-of-use", "TREAT")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let redacted = headers
        .get("redacted")
        .map(|value| value.to_str().unwrap().to_owned());
    (
        status,
        redacted,
        serde_json::from_str(&body).unwrap_or(Value::Null),
    )
}

/// A record marked `R` whose settings name the nurse a gate-keeper, under a
/// rule that clears `R` for the nurse and the administrator (the holder of
/// `ADMIN`, the controller): the nurse reads
/// the settings without the marking, the latest version also without its
/// signature, each answer marked redacted; the administrator reads them
/// whole, with no such header.
#[tokio::test]
async fn a_gate_keeper_reads_the_settings_without_the_marking() {
    let mapping = crate::party_binding_http::mapping().with_admin_role("ADMIN");
    let (_db, pool, svc, app) = Box::pin(app_with(mapping)).await;
    let (ehr, _) = ehr_with_composition(&app).await;
    let nurse = common::basic_person(&pool, "nurse").await;
    let root = common::basic_person(&pool, "root").await;
    let mut settings = marked("R");
    settings["gate_keepers"] = common::access_settings("open", &[nurse])["gate_keepers"].clone();
    common::commit_access_settings(&svc, ehr.parse().expect("an EHR id"), settings).await;
    let applies_to: Vec<Value> = [nurse, root]
        .iter()
        .flat_map(|person| {
            crate::access_evaluator_http::naming(*person)
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    let level = json!({ "_type": "DV_CODED_TEXT", "value": "R",
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_Confidentiality" },
            "code_string": "R" } });
    let mut clearing = directive(
        "controller_rule",
        "permit",
        &json!({ "applies_to": applies_to,
                 "data": [{ "_type": "ACCESS_DATA_SCOPE", "confidentialities": [level] }] }),
    );
    clearing["name"] = json!({ "_type": "DV_TEXT", "value": "The ward reads R" });
    put_policy(&app, &json!({ "rules": [clearing] })).await;

    for (user, whole) in [("nurse", false), ("root", true)] {
        let (status, redacted, object) =
            read(&app, &format!("{EXT}/ehr/{ehr}/ehr_access"), user).await;
        assert_eq!(status, StatusCode::OK, "{user}: {object}");
        assert!(
            object["settings"].get("gate_keepers").is_some(),
            "{user}: {object}"
        );
        assert_eq!(
            object["settings"].get("marking").is_some(),
            whole,
            "{user}: the object's marking"
        );
        assert_eq!(
            redacted.as_deref(),
            (!whole).then_some("marking"),
            "{user}: the object's header"
        );
        let (status, redacted, version) = read(
            &app,
            &format!("{EXT}/ehr/{ehr}/versioned_ehr_access/version"),
            user,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{user}: {version}");
        assert_eq!(
            version["data"]["settings"].get("marking").is_some(),
            whole,
            "{user}: the version's marking"
        );
        assert_eq!(
            version.get("signature").is_some(),
            whole,
            "{user}: the version's signature"
        );
        assert_eq!(
            redacted.as_deref(),
            (!whole).then_some("marking"),
            "{user}: the version's header"
        );
    }
}
