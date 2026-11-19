// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A commit body in which an object writes a member name twice is refused
//! `400` on every route that takes one (#6043).
//!
//! RFC 8259 §4 leaves the behaviour of software that receives a repeated name
//! "unpredictable", and the canonical form signatures are computed over has
//! none (RFC 8785 §3.1, on I-JSON: RFC 7493 §2.3). A parsed JSON value holds
//! the last occurrence and hides the first, so a route that read its body
//! through one committed what its client had not unambiguously said. ITS-REST
//! overview `Requests_and_responses.md` §HTTP status codes gives `400` to
//! content that "could not be parsed or is invalid".

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};

use crate::common::{create_ehr, full, send, test_router};

/// A `DV_TEXT` member `name`, written twice.
const NAME_TWICE: &str =
    r#""name":{"_type":"DV_TEXT","value":"a"},"name":{"_type":"DV_TEXT","value":"b"}"#;

/// A commit audit of a creation by a named committer.
const AUDIT: &str = r#"{"_type":"UPDATE_AUDIT","change_type":{"value":"creation","defining_code":{"terminology_id":{"value":"openehr"},"code_string":"249"}},"committer":{"_type":"PARTY_IDENTIFIED","name":"Dr Jones"}}"#;

/// The `lifecycle_state` of a complete version.
const COMPLETE: &str = r#"{"value":"complete","defining_code":{"terminology_id":{"value":"openehr"},"code_string":"532"}}"#;

/// Sends `body` as `media` and asserts the `400` that names `repeated`.
async fn refused(
    app: &Router,
    method: &str,
    path: &str,
    media: &str,
    body: String,
    repeated: &str,
) {
    let request = Request::builder()
        .method(method)
        .uri(full(path))
        .header(header::CONTENT_TYPE, media)
        .header(
            header::IF_MATCH,
            "\"8849182c-82ad-4088-a07f-48ead4180515::local::1\"",
        )
        .header("openehr-template-id", "any")
        .body(Body::from(body.clone()))
        .expect("the request builds");
    let (status, _, text) = send(app, request).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{method} {path} {body}: {text}"
    );
    assert!(
        text.contains(&format!("duplicate field `{repeated}`")),
        "{method} {path}: the refusal names the repeated member: {text}"
    );
}

/// The CONTRIBUTION bodies that repeat a name: in the envelope, in the
/// CONTRIBUTION audit, in a member's audit and inside a member's `data`.
fn contributions() -> Vec<(String, &'static str)> {
    let member = |data: &str, audit: &str| {
        format!(r#"{{"lifecycle_state":{COMPLETE},"commit_audit":{audit},"data":{data}}}"#)
    };
    let composition = r#"{"_type":"COMPOSITION"}"#;
    let twice_named = AUDIT.replace(
        r#""name":"Dr Jones""#,
        r#""name":"Dr Jones","name":"Dr Smith""#,
    );
    vec![
        (
            format!(
                r#"{{"versions":[],"versions":[{}],"audit":{AUDIT}}}"#,
                member(composition, AUDIT)
            ),
            "versions",
        ),
        (
            format!(
                r#"{{"versions":[{}],"audit":{twice_named}}}"#,
                member(composition, AUDIT)
            ),
            "name",
        ),
        (
            format!(
                r#"{{"versions":[{}],"audit":{AUDIT}}}"#,
                member(composition, &twice_named)
            ),
            "name",
        ),
        (
            format!(
                r#"{{"versions":[{}],"audit":{AUDIT}}}"#,
                member(&format!(r#"{{"_type":"COMPOSITION",{NAME_TWICE}}}"#), AUDIT)
            ),
            "name",
        ),
    ]
}

#[tokio::test]
async fn a_contribution_with_a_repeated_member_name_is_refused() {
    let (_db, app) = test_router().await;
    let ehr = create_ehr(&app).await;
    for (body, repeated) in contributions() {
        for path in [
            format!("/ehr/{ehr}/contribution"),
            "/demographic/contribution".to_owned(),
        ] {
            refused(
                &app,
                "POST",
                &path,
                "application/json",
                body.clone(),
                repeated,
            )
            .await;
        }
        for media in [
            "application/openehr.wt.flat+json",
            "application/openehr.wt.structured+json",
        ] {
            let path = format!("/ehr/{ehr}/contribution");
            refused(&app, "POST", &path, media, body.clone(), repeated).await;
        }
    }
}

#[tokio::test]
async fn a_direct_commit_with_a_repeated_member_name_is_refused() {
    let (_db, app) = test_router().await;
    let ehr = create_ehr(&app).await;
    for (method, path, class) in [
        ("POST", format!("/ehr/{ehr}/composition"), "COMPOSITION"),
        ("PUT", format!("/ehr/{ehr}/ehr_status"), "EHR_STATUS"),
        ("POST", format!("/ehr/{ehr}/directory"), "FOLDER"),
        ("POST", "/demographic/person".to_owned(), "PERSON"),
    ] {
        let body = format!(r#"{{"_type":"{class}",{NAME_TWICE}}}"#);
        refused(&app, method, &path, "application/json", body, "name").await;
    }
}
