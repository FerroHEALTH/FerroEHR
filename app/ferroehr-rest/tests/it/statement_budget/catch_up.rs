// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The budget of an ITI-81 search on a token parameter that moves the search
//! index on the request (#6819): the token resolutions the search sends stay
//! in its nested count, and only the frontier round's are left out. No
//! openEHR spec governs the cost of a search: our own design.

use std::sync::atomic::Ordering;

use http::StatusCode;
use tracing::Instrument as _;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{
    FRONTIER_TOKENS, REQUEST_SPAN, RESOLVE, RoundTrips, background, calls, counted, drained, ledger,
};
use crate::access_evaluator_http::{ehr_with_composition, send};
use crate::common::{BASE, FHIR};

/// The reads, each followed by a search, of the window: a search catches the
/// index up unless the frontier round passed the read's entry first.
const ATTEMPTS: u64 = 5;

/// The statements nested in a token resolution: the writers' hold on the
/// search tokens and the token statement.
const INNER: [&str; 2] = [
    "SELECT pg_catalog.pg_advisory_xact_lock_shared",
    "WITH input AS ( SELECT i.rec, i.parameter",
];

/// A search on a token parameter that catches the index up keeps its token
/// statements in the request's nested count.
#[tokio::test]
async fn a_catch_up_on_the_request_stays_in_its_nested_count() {
    let round_trips = RoundTrips::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(round_trips.clone()));
    let (_db, pool, _svc, app) = Box::pin(super::served()).await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    let (ehr, composition) = ehr_with_composition(&app).await;
    drained(&pool).await;
    let evicted = testkit::statements::statement_window(&pool)
        .await
        .expect("the statement counts reset");
    let before = round_trips.1.load(Ordering::Relaxed);
    for _ in 0..ATTEMPTS {
        let (status, body) = send(
            &app,
            "GET",
            &format!("{BASE}/ehr/{ehr}/composition/{composition}"),
            "root",
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "the read: {body}");
        let (status, body) = send(
            &app,
            "GET",
            &format!("{FHIR}/AuditEvent?date=ge2026-01-01&entity-type=2"),
            "root",
            None,
            None,
        )
        .instrument(tracing::info_span!(REQUEST_SPAN))
        .await;
        assert_eq!(status, StatusCode::OK, "the search: {body}");
    }
    let requested = round_trips.1.load(Ordering::Relaxed) - before;
    drained(&pool).await;
    let statements = counted(&pool).await;
    testkit::statements::assert_statements_kept(&pool, evicted)
        .await
        .expect("no statement evicted while the test counted them");
    let nested: u64 = statements
        .iter()
        .filter(|s| !s.toplevel)
        .map(|s| s.calls)
        .sum();
    let resolutions = calls(&statements, RESOLVE);
    let inner: u64 = statements
        .iter()
        .filter(|s| !s.toplevel && INNER.iter().any(|head| s.query.starts_with(head)))
        .map(|s| s.calls)
        .sum();
    let report = format!(
        "{requested} of {resolutions} token resolutions sent by the searches, {nested} nested \
         statements, {inner} of them inside a resolution\n{}",
        ledger(&statements)
    );
    eprintln!("{report}");
    assert!(
        requested >= 1,
        "a search caught the index up on the request\n{report}"
    );
    assert!(
        resolutions >= requested,
        "the searches sent no more resolutions than the database ran\n{report}"
    );
    assert_eq!(
        inner,
        FRONTIER_TOKENS * resolutions,
        "a token resolution runs {FRONTIER_TOKENS} nested statements\n{report}"
    );
    let left_out = background(&statements, requested) - background(&statements, resolutions);
    assert_eq!(
        inner - left_out,
        FRONTIER_TOKENS * requested,
        "the searches' token statements stay in their nested count, {left_out} left out\n{report}"
    );
}
