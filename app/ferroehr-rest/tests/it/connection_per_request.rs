// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One pool checkout per request on the common read paths.
//!
//! The request scope (`ferroehr::db::acquire::request_scope`) keeps the
//! connection a request checked out between its statements, so the access
//! checks and the read share it and the request queues for the pool once.
//! No openEHR spec governs connection handling: our own design.

use axum::Router;
use axum::body::Body;
use ferroehr::db::acquire::pool_checkouts;
use http::{Request, StatusCode};

use crate::common;
use crate::common::BASE;

/// The fewest pool checkouts one of three identical `GET`s of `uri`, the read
/// `read` names, took: a background task of the process can check out beside
/// a request, never in all three windows at once.
async fn checkouts_of(app: &Router, read: &str, uri: &str) -> u64 {
    let mut fewest = u64::MAX;
    for _ in 0..3 {
        let before = pool_checkouts();
        let (status, body) = common::send_body(app, common::get(uri)).await;
        let after = pool_checkouts();
        assert_eq!(status, StatusCode::OK, "{read}: {body}");
        fewest = fewest.min(after - before);
    }
    fewest
}

#[tokio::test]
async fn the_common_reads_check_out_one_connection() {
    let (_db, app) = common::test_router().await;
    let upload = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header("content-type", "application/xml")
        .body(Body::from(common::ips_opt_xml()))
        .expect("request");
    let (status, body) = common::send_body(&app, upload).await;
    assert_eq!(status, StatusCode::CREATED, "IPS OPT upload: {body}");
    let ehr_id = common::create_ehr(&app).await;
    let (status, headers, body) = common::send(
        &app,
        common::post_json(
            &format!("{BASE}/ehr/{ehr_id}/composition"),
            &common::ips_canonical_composition().to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "composition create: {body}");
    let uid = common::etag_uid(&headers);

    for (read, uri) in [
        ("EHR", format!("{BASE}/ehr/{ehr_id}")),
        ("EHR_STATUS", format!("{BASE}/ehr/{ehr_id}/ehr_status")),
        (
            "COMPOSITION",
            format!("{BASE}/ehr/{ehr_id}/composition/{uid}"),
        ),
    ] {
        assert_eq!(
            checkouts_of(&app, read, &uri).await,
            1,
            "pool checkouts of a read of the {read}"
        );
    }
}
