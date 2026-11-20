// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login of a record's person (#6106): a `persons` caller whose one
//! `PERSON` carries no professional and no representative marker, which the
//! shared binding answers `wrong_marker`. FerroEHR admits it as the person
//! themself (EHDS Art. 3): it finds and reads the EHRs whose
//! `EHR_STATUS.subject` names that `PERSON`, as their subject, and nothing
//! else, with no role and no professional right. No openEHR spec governs
//! resolving a login to a party: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::party_binding::source::BindingSource;
use ferrohealth_smart::party::source::{Lookup, PartyRecord, PartySource, Query, SourceError};
use ferrohealth_smart::party::{Markers, PartyRef, PartyState};
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{service, token};
use crate::representation_http::{STRANGER, ehr, router};

/// The `PERSON` every login of [`Patient`] binds.
const PERSON: &str = "4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f7a";

/// A party source that binds every login to one `PERSON` holding neither
/// marker: a patient's.
struct Patient;

impl BindingSource for Patient {}

impl PartySource for Patient {
    fn lookup<'a>(
        &'a self,
        _query: &'a Query<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Lookup, SourceError>> + Send + 'a>> {
        let record = PartyRecord::new(
            PartyRef::new("demographic", "PERSON", PERSON),
            PartyState::Active,
            Markers::default(),
        );
        Box::pin(async move { Ok(Lookup::Parties(vec![record])) })
    }
}

/// `method` on `path` under the API base as `credential`, with `body`.
async fn send(
    app: &Router,
    (method, path): (&str, &str),
    credential: &str,
    body: Option<&Value>,
) -> (StatusCode, String) {
    let req = common::req(method, format!("{BASE}{path}"))
        .auth(credential)
        .accept("application/json");
    let req = match body {
        Some(body) => req.json(body.to_string()).build(),
        None => req.build(),
    };
    common::send_body(app, req).await
}

/// The lookup of the EHR whose subject is the `PERSON` `person`.
fn lookup(person: &str) -> String {
    format!("/ehr?subject_id={person}&subject_namespace=demographic")
}

#[tokio::test]
async fn a_patients_own_login_finds_and_reads_their_own_ehr() {
    let (_db, svc) = service().await;
    let own = ehr(&svc, Some(PERSON)).await;
    let app = router(Arc::clone(&svc), Arc::new(Patient)).await;
    let credential = token("eva-login", &json!({}));

    let (code, body) = send(&app, ("GET", &lookup(PERSON)), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "the lookup of her own EHR: {body}");
    let found: Value = serde_json::from_str(&body).expect("an EHR");
    assert_eq!(found["ehr_id"]["value"], json!(own), "{body}");
    let (code, body) = send(&app, ("GET", &format!("/ehr/{own}")), &credential, None).await;
    assert_eq!(code, StatusCode::OK, "her own EHR: {body}");
}

#[tokio::test]
async fn a_patients_own_login_holds_no_professional_right() {
    let (_db, svc) = service().await;
    ehr(&svc, Some(PERSON)).await;
    let stranger = ehr(&svc, Some(STRANGER)).await;
    let app = router(Arc::clone(&svc), Arc::new(Patient)).await;
    let credential = token("eva-login", &json!({ "roles": ["CLINICIAN", "ADMIN"] }));

    let (code, body) = send(&app, ("GET", &lookup(STRANGER)), &credential, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND, "another person's EHR: {body}");
    let (code, body) = send(
        &app,
        ("GET", &format!("/ehr/{stranger}")),
        &credential,
        None,
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "another EHR by id: {body}");
    assert!(body.contains("refused-not-subject"), "{body}");
    let aql = json!({ "q": "SELECT e/ehr_id/value FROM EHR e" });
    for (method, path, body) in [("POST", "/ehr", None), ("POST", "/query/aql", Some(&aql))] {
        let (code, answer) = send(&app, (method, path), &credential, body).await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{method} {path}: {answer}");
        assert!(
            answer.contains("refused-not-subject"),
            "{method} {path}: {answer}"
        );
    }
}
