// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One pool checkout per request on the common read paths.
//!
//! The request scope (`ferroehr::db::acquire::request_scope`) keeps the
//! connection a request checked out between its statements, so the access
//! checks and the read share it and the request queues for the pool once.
//! No openEHR spec governs connection handling: our own design.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use ferroehr::db::domain::{Domain, DomainPools};
use ferroehr::service::FerroEhrService;
use http::StatusCode;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use crate::common;
use crate::common::BASE;

/// A lazy pool serving `domain` that counts in `checkouts` every connection it
/// hands out, from the idle queue (`before_acquire`) or freshly opened
/// (`after_connect`).
fn counted(options: &PgConnectOptions, domain: Domain, checkouts: &Arc<AtomicU64>) -> PgPool {
    let opened = Arc::clone(checkouts);
    let reused = Arc::clone(checkouts);
    PgPoolOptions::new()
        .min_connections(0)
        .test_before_acquire(false)
        .after_connect(move |conn, _meta| {
            opened.fetch_add(1, Ordering::Relaxed);
            Box::pin(async move {
                sqlx::query(domain.search_path())
                    .execute(conn)
                    .await
                    .map(drop)
            })
        })
        .before_acquire(move |_conn, _meta| {
            reused.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Ok(true) })
        })
        .connect_lazy_with(options.clone())
}

/// The fewest pool checkouts one of three identical `GET`s of `uri`, the read
/// `read` names, took: a background task of the process can check out beside
/// a request, never in all three windows at once.
async fn checkouts_of(app: &Router, checkouts: &AtomicU64, read: &str, uri: &str) -> u64 {
    let mut fewest = u64::MAX;
    for _ in 0..3 {
        let before = checkouts.load(Ordering::Relaxed);
        let (status, body) = common::send_body(app, common::get(uri)).await;
        let after = checkouts.load(Ordering::Relaxed);
        assert_eq!(status, StatusCode::OK, "{read}: {body}");
        fewest = fewest.min(after - before);
    }
    fewest
}

#[tokio::test]
async fn the_common_reads_check_out_one_connection() {
    let db = common::test_db().await;
    let options = PgConnectOptions::from_str(db.url()).expect("the test DSN parses");
    let checkouts = Arc::new(AtomicU64::new(0));
    let pools = DomainPools {
        clinical: counted(&options, Domain::Clinical, &checkouts),
        party: counted(&options, Domain::Party, &checkouts),
        audit: counted(&options, Domain::Audit, &checkouts),
    };
    let app = common::unauthenticated_router(Arc::new(FerroEhrService::new(&pools))).await;
    let upload = common::req("POST", format!("{BASE}/definition/template/adl1.4"))
        .header("content-type", "application/xml")
        .body(common::ips_opt_xml())
        .build();
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
            checkouts_of(&app, &checkouts, read, &uri).await,
            1,
            "pool checkouts of a read of the {read}"
        );
    }
}
