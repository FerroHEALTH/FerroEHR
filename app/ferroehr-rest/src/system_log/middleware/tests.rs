// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit middleware's classification, outcome and object helpers.

use super::*;

#[test]
fn outcome_codes_track_http_status() {
    assert_eq!(outcome_from_status(StatusCode::OK), EventOutcome::Success);
    assert_eq!(
        outcome_from_status(StatusCode::NO_CONTENT),
        EventOutcome::Success
    );
    // 304 Not Modified is a successful conditional read (RFC 9110 §15.4.5);
    // redirection is not a failed action.
    assert_eq!(
        outcome_from_status(StatusCode::NOT_MODIFIED),
        EventOutcome::Success
    );
    assert_eq!(
        outcome_from_status(StatusCode::BAD_REQUEST),
        EventOutcome::MinorFailure
    );
    assert_eq!(
        outcome_from_status(StatusCode::UNAUTHORIZED),
        EventOutcome::MinorFailure
    );
    assert_eq!(
        outcome_from_status(StatusCode::FORBIDDEN),
        EventOutcome::MinorFailure
    );
    assert_eq!(
        outcome_from_status(StatusCode::NOT_FOUND),
        EventOutcome::MinorFailure
    );
    assert_eq!(
        outcome_from_status(StatusCode::INTERNAL_SERVER_ERROR),
        EventOutcome::SeriousFailure
    );
    assert_eq!(
        outcome_from_status(StatusCode::SERVICE_UNAVAILABLE),
        EventOutcome::SeriousFailure
    );
}

/// The EHR id `route_ehr_id` reads for `path` once `routes` matched it.
async fn routed_ehr_id(templates: &[&str], path: &str) -> Option<String> {
    use tower::ServiceExt as _;
    let mut app = axum::Router::new();
    for template in templates {
        app = app.route(template, axum::routing::any(|| async {}));
    }
    let app = app.layer(axum::middleware::from_fn(
        |req: Request, next: Next| async move {
            let (req, found) = route_ehr_id(req).await;
            let mut resp = next.run(req).await;
            if let Some(found) = found {
                resp.headers_mut()
                    .insert("x-ehr", HeaderValue::from_str(&found).unwrap());
            }
            resp
        },
    ));
    let resp = app
        .oneshot(Request::get(path).body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{path} should match a route");
    resp.headers()
        .get("x-ehr")
        .map(|v| v.to_str().unwrap().to_owned())
}

#[tokio::test]
async fn ehr_id_from_the_matched_route() {
    let routes = [
        "/ehr",
        "/ehr/{ehr_id}",
        "/ehr/{ehr_id}/composition/{uid_based_id}",
        "/ehr/{ehr_id}/ehr_status",
        "/admin/ehr/all",
        "/admin/ehr/{ehr_id}",
        "/message/export/{ehr_id}",
        "/message/import/{ehr_id}",
        "/definition/template/adl1.4/{template_id}",
    ];
    let some = |id: &str| Some(id.to_owned());
    assert_eq!(
        routed_ehr_id(&routes, "/ehr/abc-123/composition/x").await,
        some("abc-123")
    );
    assert_eq!(
        routed_ehr_id(&routes, "/ehr/abc-123").await,
        some("abc-123")
    );
    assert_eq!(
        routed_ehr_id(&routes, "/ehr/abc-123/ehr_status").await,
        some("abc-123")
    );
    // Create has no id parameter.
    assert_eq!(routed_ehr_id(&routes, "/ehr").await, None);
    // Admin delete carries the id; the literal `all` route names none.
    assert_eq!(routed_ehr_id(&routes, "/admin/ehr/zzz").await, some("zzz"));
    assert_eq!(routed_ehr_id(&routes, "/admin/ehr/all").await, None);
    // The extension routes name the EHR after a segment other than `ehr`.
    for path in ["/message/export/e-1", "/message/import/e-1"] {
        assert_eq!(routed_ehr_id(&routes, path).await, some("e-1"), "{path}");
    }
    assert_eq!(
        routed_ehr_id(&routes, "/definition/template/adl1.4/t").await,
        None
    );
}

#[test]
fn object_id_from_template_paths() {
    assert_eq!(
        object_id_from_path(
            "definition_template_adl1.4_get",
            "/x/definition/template/adl1.4/vital_signs.v1"
        ),
        Some("vital_signs.v1".to_owned())
    );
    // Upload/list have no id segment.
    assert_eq!(
        object_id_from_path(
            "definition_template_adl1.4_upload",
            "/x/definition/template/adl1.4"
        ),
        None
    );
    assert_eq!(
        object_id_from_path(
            "definition_template_adl2_version_get",
            "/x/definition/template/adl2/t.v1/1.0.0"
        ),
        Some("t.v1".to_owned())
    );
    // The ADL 2 conversion names the template it reads.
    assert_eq!(
        object_id_from_path(
            "definition_template_adl14_convert",
            "/x/definition/template/adl1.4/vital_signs.v1/adl2"
        ),
        Some("vital_signs.v1".to_owned())
    );
}

#[test]
fn object_id_from_query_paths() {
    // Stored query execution: the qualified name is the search criteria.
    assert_eq!(
        object_id_from_path("query_execute_stored_query", "/x/query/eu.ferroehr::q1"),
        Some("eu.ferroehr::q1".to_owned())
    );
    assert_eq!(
        object_id_from_path("definition_query_store.yaml", "/x/definition/query/org::q2"),
        Some("org::q2".to_owned())
    );
    // Ad-hoc queries stay UNKNOWN (no extraction), and so does the cohort
    // extension: `cohort` is a route segment, not a stored-query name.
    assert_eq!(
        object_id_from_path("query_execute_adhoc_query", "/x/query/aql"),
        None
    );
    assert_eq!(
        object_id_from_path("query_execute_cohort", "/x/query/cohort"),
        None
    );
}

#[test]
fn object_id_from_demographic_paths() {
    assert_eq!(
        object_id_from_path("person_get", "/x/demographic/person/p-1"),
        Some("p-1".to_owned())
    );
    assert_eq!(
        object_id_from_path("role_tags_delete", "/x/demographic/role/r-9/tags/k"),
        Some("r-9".to_owned())
    );
    assert_eq!(
        object_id_from_path("versioned_party_get", "/x/demographic/versioned_party/vp-3"),
        Some("vp-3".to_owned())
    );
    // Create has no id segment.
    assert_eq!(
        object_id_from_path("person_create", "/x/demographic/person"),
        None
    );
    // The physical party delete names the party it removed.
    assert_eq!(
        object_id_from_path("admin_party_delete", "/x/admin/party/p-7"),
        Some("p-7".to_owned())
    );
}
