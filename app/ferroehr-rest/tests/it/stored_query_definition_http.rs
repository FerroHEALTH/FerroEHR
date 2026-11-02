// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the stored-query DEFINITION wire (group-11 audit):
//! the version-less `PUT /definition/query/{name}`
//! (whose `Location` names exactly the version the store wrote — never a
//! neighbouring one, `headers/Location_Query.yaml`) and the versioned
//! `PUT …/{name}/{version}` (exact `major.minor.patch` required — the
//! `{major}`/`{major}.{minor}` prefix forms of `parameters/path/version.yaml`
//! are READ-side resolution patterns, and a non-numeric segment would poison
//! the surface's SEMVER ordering; register-adjudicated `400`).
#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};

use ferroehr::config::rest::RestConfig;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::AuthSection;

use crate::common;
use crate::common::send;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const AQL: &str = "SELECT e/ehr_id/value FROM EHR e";

fn config() -> AppConfig {
    AppConfig {
        server: common::server(),
        rest: RestConfig {
            base_path: BASE.to_owned(),
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            cors_permissive: false,
            ..Default::default()
        },
        auth: AuthSection {
            enabled: false,
            basic: None,
            ..AuthSection::default()
        },
        ..Default::default()
    }
}

async fn app() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(config(), service).await)
}

fn put(uri: String) -> Request<Body> {
    common::req("PUT", uri)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(AQL)
        .build()
}

fn location(h: &header::HeaderMap) -> &str {
    h.get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location header")
}

// ── the version-less store's Location names the WRITTEN version ─────────────

#[tokio::test]
async fn versionless_store_location_names_the_default_slot() {
    let (_pg, app) = app().await;
    let (status, headers, body) =
        send(&app, put(format!("{BASE}/definition/query/org.test::a"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        location(&headers),
        format!("{BASE}/definition/query/org.test::a/1.0.0"),
        "the version-less store writes the 1.0.0 default slot and Location \
         names exactly that resource (headers/Location_Query.yaml)"
    );
}

#[tokio::test]
async fn versionless_store_location_ignores_higher_stored_versions() {
    let (_pg, app) = app().await;
    // A higher version exists first…
    let (status, _h, body) = send(
        &app,
        put(format!("{BASE}/definition/query/org.test::b/2.0.0")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // …then the version-less store writes 1.0.0: Location must name 1.0.0,
    // never the neighbouring 2.0.0 this PUT did not touch.
    let (status, headers, body) =
        send(&app, put(format!("{BASE}/definition/query/org.test::b"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        location(&headers),
        format!("{BASE}/definition/query/org.test::b/1.0.0"),
        "Location indicates the URL of the Stored Query resource this PUT \
         stored (headers/Location_Query.yaml), not the highest version"
    );
}

#[tokio::test]
async fn versionless_store_updates_in_place() {
    let (_pg, app) = app().await;
    let uri = format!("{BASE}/definition/query/org.test::c");
    let (status, _h, body) = send(&app, put(uri.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // "Stores a new query, or updates an existing query on the system"
    // (definition_query_store.yaml) — the second PUT is an update, not a 409.
    let (status, headers, body) = send(&app, put(uri)).await;
    assert_eq!(status, StatusCode::OK, "update-in-place: {body}");
    assert!(location(&headers).ends_with("/1.0.0"));
}

// ── the versioned store requires an exact numeric SEMVER triple ─────────────

#[tokio::test]
async fn versioned_store_accepts_exact_semver_and_conflicts_on_duplicate() {
    let (_pg, app) = app().await;
    let uri = format!("{BASE}/definition/query/org.test::d/1.2.3");
    let (status, headers, body) = send(&app, put(uri.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        location(&headers),
        format!("{BASE}/definition/query/org.test::d/1.2.3")
    );
    // "409 Conflict is returned when a query with the given
    // qualified_query_name and version already exists on the server"
    // (responses/409_StoredQuery_version.yaml).
    let (status, _h, body) = send(&app, put(uri)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn versioned_store_rejects_non_exact_versions() {
    let (_pg, app) = app().await;
    // The prefix forms are read-side resolution patterns
    // (parameters/path/version.yaml) that resolve to nothing writable on a
    // store, and the released identifier format is three numeric parts
    // ("SEMVER style (i.e. `major.minor.patch`)",
    // docs/query/Qualified_query_name.md). Every other token is the 400 the
    // docs text assigns to a client error no other 4xx fits
    // (docs/overview/Requests_and_responses.md §"HTTP status codes").
    for bad in ["1", "1.0", "1.0.0-rc.1", "latest", "1.0.x", "1..0"] {
        let (status, _h, body) = send(
            &app,
            put(format!("{BASE}/definition/query/org.test::e/{bad}")),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`{bad}` is not an exact major.minor.patch: {body}"
        );
    }
}

#[tokio::test]
async fn version_get_resolves_prefixes_to_highest() {
    let (_pg, app) = app().await;
    for v in ["1.0.0", "1.0.9", "1.2.0"] {
        let (status, _h, body) = send(
            &app,
            put(format!("{BASE}/definition/query/org.test::f/{v}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    // "a pattern as partial prefix … the highest (latest) version matching
    // the prefix will be considered" (parameters/path/version.yaml).
    for (sel, expect) in [("1", "1.2.0"), ("1.0", "1.0.9"), ("1.0.0", "1.0.0")] {
        let get = common::req("GET", format!("{BASE}/definition/query/org.test::f/{sel}"))
            .accept("application/json")
            .build();
        let (status, _h, body) = send(&app, get).await;
        assert_eq!(status, StatusCode::OK, "{sel}: {body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v.get("version").and_then(|x| x.as_str()),
            Some(expect),
            "selector `{sel}` resolves to `{expect}`: {body}"
        );
    }
}

#[tokio::test]
async fn version_reads_refuse_a_malformed_version_before_any_lookup() {
    let (_pg, app) = app().await;
    let (status, _h, body) = send(
        &app,
        put(format!("{BASE}/definition/query/org.test::v/1.2.0")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The definition read, the stored-query GET execution and its POST form all
    // take `parameters/path/version.yaml`.
    let reads = |version: &str| {
        let get = |uri: String| common::req("GET", uri).accept("application/json").build();
        [
            get(format!("{BASE}/definition/query/org.test::v/{version}")),
            get(format!("{BASE}/query/org.test::v/{version}")),
            common::req("POST", format!("{BASE}/query/org.test::v/{version}"))
                .accept("application/json")
                .json("{}")
                .build(),
        ]
    };
    // No SEMVER version and no `{major}` / `{major}.{minor}` prefix: a
    // syntactically invalid parameter (responses/400.yaml, 400_Query.yaml).
    for bad in ["v1.x", "1.x", "01", "1.02", "1.2.0.0"] {
        for req in reads(bad) {
            let uri = req.uri().to_string();
            let (status, _h, body) = send(&app, req).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        }
    }
    // The exact version and its prefixes resolve; a well-formed version the
    // store does not hold is the 404 of 404_Query_version.yaml.
    for (version, expect) in [
        ("1.2.0", StatusCode::OK),
        ("1.2", StatusCode::OK),
        ("1", StatusCode::OK),
        ("2", StatusCode::NOT_FOUND),
        ("1.3", StatusCode::NOT_FOUND),
        ("1.2.1", StatusCode::NOT_FOUND),
    ] {
        for req in reads(version) {
            let uri = req.uri().to_string();
            let (status, _h, body) = send(&app, req).await;
            assert_eq!(status, expect, "{uri}: {body}");
        }
    }
}

// ── a non-text/plain declared body type is 415, never a parse-time 400 ──────

#[tokio::test]
async fn stores_refuse_a_declared_foreign_body_type_with_415() {
    let (_pg, app) = app().await;
    // Both store forms declare text/plain as the single body type; a payload
    // declaring another media type cannot be processed as it (`Resources.md`
    // §format rules: the service "MUST respond with HTTP status code 415
    // Unsupported Media Type").
    for uri in [
        format!("{BASE}/definition/query/org.test::g"),
        format!("{BASE}/definition/query/org.test::g/1.0.0"),
    ] {
        let req = common::req("PUT", uri.clone()).json(AQL).build();
        let (status, _h, body) = send(&app, req).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{uri}: {body}");
    }
}

#[tokio::test]
async fn stores_accept_an_absent_content_type() {
    let (_pg, app) = app().await;
    // An absent Content-Type declares nothing to refuse (the header is a
    // client MAY) — the operation's own body type applies.
    let req = common::req("PUT", format!("{BASE}/definition/query/org.test::h/1.0.0"))
        .body(AQL)
        .build();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

// ── a store runs the analysis an execution runs ─────────────────────────────

fn put_text(uri: String, aql: &str) -> Request<Body> {
    common::req("PUT", uri)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(aql.to_owned())
        .build()
}

#[tokio::test]
async fn store_accepts_a_query_the_engine_plans() {
    let (_pg, app) = app().await;
    // Parameters (a temporal bound among them) are bound at execution; the
    // store analyses the text without them.
    let aql = "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
               WHERE c/context/start_time/value > $since AND e/ehr_id/value = $ehr_id \
               LIMIT 10";
    for uri in [
        format!("{BASE}/definition/query/org.test::plans"),
        format!("{BASE}/definition/query/org.test::plans_v/1.0.0"),
    ] {
        let (status, _h, body) = send(&app, put_text(uri.clone(), aql)).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    }
}

#[tokio::test]
async fn store_refuses_a_query_the_engine_refuses_and_stores_nothing() {
    let (_pg, app) = app().await;
    // Reading a variable declared under NOT CONTAINS is refused at SQL
    // generation (QUERY master03 §Containment); the store refuses it with the
    // operation's only error status (definition_query_store.yaml: `400`).
    let aql = "SELECT o/name/value FROM EHR e CONTAINS COMPOSITION c \
               NOT CONTAINS OBSERVATION o";
    for name in ["org.test::refused", "org.test::refused_v/1.0.0"] {
        let uri = format!("{BASE}/definition/query/{name}");
        let (status, _h, body) = send(&app, put_text(uri.clone(), aql)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
    }
    for name in ["org.test::refused", "org.test::refused_v"] {
        let get = common::req("GET", format!("{BASE}/definition/query/{name}"))
            .accept("application/json")
            .build();
        // definition_query_list declares `200` only: nothing stored is `[]`.
        let (status, _h, body) = send(&app, get).await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
        assert_eq!(body.trim(), "[]", "{name} was stored: {body}");
    }
}

/// A stored query text nested beyond the parser's depth bound is refused at
/// store time with the operation's `400` (`definition_query_store.yaml`), so no
/// execution of a stored query ever meets one (#4284).
#[tokio::test]
async fn store_refuses_a_query_nested_beyond_the_depth_bound() {
    let (_pg, app) = app().await;
    let aql = format!(
        "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE {}c/name/value = 'x'",
        "NOT ".repeat(20_000)
    );
    for name in ["org.test::deep", "org.test::deep_v/1.0.0"] {
        let uri = format!("{BASE}/definition/query/{name}");
        let (status, _h, body) = send(&app, put_text(uri.clone(), &aql)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
    }
}
