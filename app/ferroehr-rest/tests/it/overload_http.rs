// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Ingress overload-shedding tests, driven through the fully assembled router
//! over the **real** `FerroEhrService` (the scripted `Mock`/parking
//! hook is gone).
//!
//! No openEHR spec governs server overload — this is our own design (RFC 9110
//! §15.6.4 for the `503` status). These tests exercise the real router stack
//! (`build_with`): the family stack's overload bound (`server.
//! max_concurrent_requests`) over every FerroEHR route, never the health
//! routes.
//!
//! Reproducing real contention without the removed in-handler hook: a request
//! whose handler buffers a **never-ending request body**
//! (`futures::stream::pending`) parks inside the API permit (the body is read
//! while the concurrency permit is held), so a further request to the **same
//! route** is shed. (The parked requests and the shed probe target the same
//! operation, `POST …/composition`, whose handler reads the body.) The per-request timeout
//! (30 s) is far longer than the probe window, and the parked tasks are aborted
//! at the end of each test, so no permit leaks.
//!
//! * beyond the `max_concurrent_requests` bound, an API request is shed
//!   immediately with `503 Service Unavailable` + `Retry-After: 1` and the
//!   problem details;
//! * with the defaults the bound is on at 256: the request past it is shed,
//!   so what the server holds stays bounded;
//! * the health routes are outside the bound and are never shed, even while
//!   the bound is fully taken.
use std::num::NonZeroUsize;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use http::{Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use ferroehr::config::rest::ApiConfig;
use ferroehr_rest::config::AppConfig;
use ferrohealth_server::section::ServerSection;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
/// A syntactically valid EHR id (the dispatcher decodes it before the backend).
const EHR: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

/// Auth-disabled config with the concurrency bound `server` carries.
fn config(server: ServerSection) -> AppConfig {
    AppConfig {
        server,
        api: ApiConfig {
            base_path: BASE.to_owned(),
            openapi_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        auth: common::auth_off(),
        ..Default::default()
    }
}

/// The `[server]` section with the bound `max_in_flight`.
fn bounded(max_in_flight: usize) -> ServerSection {
    ServerSection {
        max_concurrent_requests: NonZeroUsize::new(max_in_flight).expect("a bound above zero"),
        ..ServerSection::default()
    }
}

async fn app(server: ServerSection) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::build_with(config(server), service).await)
}

/// A request whose handler parks holding its API permit: a composition create
/// whose body never ends, so the body-buffering extractor (`into_parts`) awaits
/// forever (well within the 30 s request timeout).
fn parking_request() -> Request<Body> {
    let never = futures::stream::pending::<Result<Bytes, std::io::Error>>();
    common::req("POST", format!("{BASE}/ehr/{EHR}/composition"))
        .json(Body::from_stream(never))
        .build()
}

/// A shed-probe on the **same route** as [`parking_request`]. When the route's
/// permits are exhausted this is shed at `poll_ready` (before its body is read),
/// so an empty body suffices and the `503` returns immediately.
fn probe_request() -> Request<Body> {
    common::req("POST", format!("{BASE}/ehr/{EHR}/composition"))
        .header("content-type", "application/json")
        .build()
}

/// Probe the (parked) route until a request is shed (`503`), giving the parked
/// handlers time to acquire their permits. Fails if no shed is observed within
/// ~5 s.
async fn probe_until_shed(app: &Router) -> Response<Body> {
    for _ in 0..250 {
        let resp = app
            .clone()
            .oneshot(probe_request())
            .await
            .expect("response");
        if resp.status() == StatusCode::SERVICE_UNAVAILABLE {
            return resp;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no request was shed within the probe window (permits never saturated)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_beyond_limit_are_shed_with_503() {
    let (_pg, app) = app(bounded(2)).await;

    // Two parked handlers occupy both permits of the composition route. Let them
    // acquire the permits *before* probing: `LoadShed` sheds a request that can't
    // immediately get a permit, so a probe racing the parked tasks could steal a
    // permit and get one of them shed instead of parked.
    let p1 = tokio::spawn(app.clone().oneshot(parking_request()));
    let p2 = tokio::spawn(app.clone().oneshot(parking_request()));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !p1.is_finished() && !p2.is_finished(),
        "both parking requests must be holding permits (not shed)"
    );

    // A further request to the same route finds no free permit and is shed.
    let resp = probe_until_shed(&app).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        resp.headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("problem details");
    // RFC 9457 problem details, FerroEHR's `unavailable` type.
    assert_eq!(body["status"], 503);
    assert!(body.get("detail").and_then(Value::as_str).is_some());

    // Release the parked handlers (aborting drops their futures → frees permits).
    p1.abort();
    p2.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_defaults_bound_the_server_at_256() {
    // A server started from the defaults holds at most 256 requests: the next
    // one is shed at once, so offered load past capacity cannot grow memory
    // (FerroHEALTH/FerroEHR#5743), and readiness still answers.
    let server = ServerSection::default();
    let bound = server.max_concurrent_requests.get();
    assert_eq!(256, bound, "the defaults carry the bound");
    let (_pg, app) = app(server).await;

    let parked: Vec<_> = (0..bound)
        .map(|_| tokio::spawn(app.clone().oneshot(parking_request())))
        .collect();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        parked.iter().all(|request| !request.is_finished()),
        "every parked request holds a slot (none shed)"
    );

    let shed = probe_until_shed(&app).await;
    assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        shed.headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    let readiness = app
        .clone()
        .oneshot(common::req("GET", "/health/readiness").build())
        .await
        .expect("response");
    assert_eq!(readiness.status(), StatusCode::OK, "readiness is never shed");

    for request in parked {
        request.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_health_routes_are_never_shed() {
    let (_pg, app) = app(bounded(1)).await;

    // Saturate the single permit of the composition route with one parked
    // handler; let it acquire the permit before probing (see the note in
    // `requests_beyond_limit_are_shed_with_503`).
    let parked = tokio::spawn(app.clone().oneshot(parking_request()));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !parked.is_finished(),
        "the parking request must hold the permit"
    );

    // A second request to that route is shed — the limit is genuinely saturated.
    let shed = probe_until_shed(&app).await;
    assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);

    // …but the health routes are outside the bound and answer normally, so an
    // orchestrator can always probe an overloaded server.
    for path in ["/health/readiness", "/health/liveness", "/health"] {
        let probe = common::req("GET", path).build();
        let answered = app.clone().oneshot(probe).await.expect("response");
        assert_eq!(answered.status(), StatusCode::OK, "{path}");
    }

    parked.abort();
}
