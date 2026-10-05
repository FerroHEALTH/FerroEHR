// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the two ADMIN **extension** groups — the SM
//! activity report (`/admin/report/*`, `I_ADMIN_SERVICE` statistics) and the SM
//! archive pair (`/admin/archive/*`, `I_ADMIN_ARCHIVE`) — driven through the
//! assembled router over a real `FerroEhrService` on a real `PostgreSQL`.
//!
//! **No openEHR spec governs these routes** (the released Admin API is exactly
//! `admin_ehr_delete` + `admin_ehr_delete_all`); the operation SEMANTICS come
//! from `docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc` and
//! `i_admin_archive.adoc`, and the wire shape is our own design/extension. What
//! is asserted here is exactly what the CNF extension bindings drive:
//! `admin-activity-report` and `admin-archive` in
//! Veredictum's `artifacts/vocab/wire_surface.yaml`.
#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};

use crate::common;
use crate::common::BASE;
use tower::ServiceExt;

/// A syntactically valid id that is never created — the "unknown" probe.
const ABSENT: &str = "00000000-0000-0000-0000-000000000000";

async fn app(admin_enabled: bool) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (
        pg,
        common::router_with(common::api_config(admin_enabled), service),
    )
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    common::send_body(app, req).await
}

/// A bodyless `GET` of the [`BASE`]-relative `path`.
fn get(path: &str) -> Request<Body> {
    common::get(&format!("{BASE}{path}"))
}

/// A JSON `POST` of `body` to the [`BASE`]-relative `path`.
fn post_json(path: &str, body: &str) -> Request<Body> {
    common::post_json(&format!("{BASE}{path}"), body)
}

/// Create an EHR through the released wire and return its id.
async fn create_ehr(app: &Router) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .header("Prefer", "return=representation")
        .body(Body::empty())
        .expect("request");
    let (status, body) = send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).expect("EHR json");
    value["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned()
}

// ── the activity report (SM I_ADMIN_SERVICE statistics) ──────────────────────

/// The four reporting routes answer `200` with the SM return shape: a list of
/// CONTRIBUTION ids, and three bare `Integer` counts. An EHR created through
/// the released wire commits one CONTRIBUTION, so the counts are observable
/// rather than trivially zero.
#[tokio::test]
async fn the_activity_report_counts_what_the_released_wire_committed() {
    let (_pg, app) = app(true).await;

    let (status, body) = send(&app, get("/admin/report/contribution?a_service=Ehr")).await;
    assert_eq!(status, StatusCode::OK, "empty-server list: {body}");
    assert_eq!(body, "[]", "an empty server has committed nothing");

    let _ehr_id = create_ehr(&app).await;

    let (status, body) = send(&app, get("/admin/report/contribution?a_service=Ehr")).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<String> = serde_json::from_str(&body).expect("id list");
    assert_eq!(ids.len(), 1, "the EHR create committed one CONTRIBUTION");

    let (status, body) = send(&app, get("/admin/report/contribution/count?a_service=Ehr")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "1", "the count is the bare SM Integer");

    // COMPOSITIONs are EHR-scoped and none was committed, so both COMPOSITION
    // counters are zero — and a service that is not a versioned-content
    // service reports zero for everything.
    for path in [
        "/admin/report/versioned_composition/count?a_service=Ehr",
        "/admin/report/composition_version/count?a_service=Ehr",
        "/admin/report/contribution/count?a_service=Query",
    ] {
        let (status, body) = send(&app, get(path)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, "0", "{path}");
    }
}

/// The SM `Interval<Iso8601_date_time>` filter reaches the query: a range that
/// cannot contain the commit answers zero, an open-ended one containing it
/// answers one.
#[tokio::test]
async fn the_report_time_interval_filters_the_commit_window() {
    let (_pg, app) = app(true).await;
    let _ehr_id = create_ehr(&app).await;

    let (status, body) = send(
        &app,
        get("/admin/report/contribution/count?a_service=Ehr&time_interval=2000-01-01T00:00:00Z/2000-12-31T00:00:00Z"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "0", "the commit is outside the range");

    let (status, body) = send(
        &app,
        get("/admin/report/contribution/count?a_service=Ehr&time_interval=2000-01-01T00:00:00Z/"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "1", "an open upper bound contains the commit");
}

/// The two parameter refusals: `a_service` outside the SM `PLATFORM_SERVICE`
/// enumeration, and a `time_interval` that is not `<lower>/<upper>`.
#[tokio::test]
async fn the_report_refuses_an_unknown_service_and_a_malformed_interval() {
    let (_pg, app) = app(true).await;

    for path in [
        "/admin/report/contribution",
        "/admin/report/contribution?a_service=Terminology",
        "/admin/report/contribution?a_service=Ehr&time_interval=2020-01-01T00:00:00Z",
        "/admin/report/contribution?a_service=Ehr&time_interval=not-a-date/",
    ] {
        let (status, body) = send(&app, get(path)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
    }
}

/// A `time_interval` bounded on both sides with `lower > upper` is no
/// `Interval` at all — BASE
/// `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.interval.adoc`
/// §Invariants, `Limits_consistent`: `(not upper_unbounded and not
/// lower_unbounded) implies lower <= upper`. The SM parameter IS an
/// `Interval<Iso8601_date_time>` (`i_admin_service.adoc`), so the value
/// violates its own type and the call is refused rather than answered with the
/// empty result an inverted range would select. All four reporting routes share
/// the one boundary parser, so all four refuse.
#[tokio::test]
async fn the_report_refuses_an_interval_whose_lower_bound_is_after_its_upper() {
    let (_pg, app) = app(true).await;

    let inverted = "time_interval=2026-12-31T00:00:00Z/2020-01-01T00:00:00Z";
    for route in [
        "/admin/report/contribution",
        "/admin/report/contribution/count",
        "/admin/report/versioned_composition/count",
        "/admin/report/composition_version/count",
    ] {
        let path = format!("{route}?a_service=Ehr&{inverted}");
        let (status, body) = send(&app, get(&path)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
    }

    // The equal-bounds pair satisfies `lower <= upper` and is a legitimate
    // (closed, single-instant) interval — the refusal must not swallow it.
    let (status, body) = send(
        &app,
        get("/admin/report/contribution/count\
             ?a_service=Ehr&time_interval=2026-01-01T00:00:00Z/2026-01-01T00:00:00Z"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "equal bounds: {body}");
}

// ── the archive pair (SM I_ADMIN_ARCHIVE) ────────────────────────────────────

/// `archive_ehrs` marks the named EHRs archived and answers `204`; archival is
/// read-neutral, so the EHR stays retrievable through the released wire.
/// Re-archiving is idempotent.
#[tokio::test]
async fn archiving_an_ehr_is_read_neutral_and_idempotent() {
    let (_pg, app) = app(true).await;
    let ehr_id = create_ehr(&app).await;

    let body = format!(r#"{{"ehr_ids":["{ehr_id}"]}}"#);
    for _ in 0..2 {
        let (status, reply) = send(&app, post_json("/admin/archive/ehrs", &body)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "archive: {reply}");
    }

    let (status, _) = send(&app, get(&format!("/ehr/{ehr_id}"))).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "archival is a marker, not a deletion — the EHR must stay retrievable"
    );
}

/// The all-or-nothing refusals of both halves: an unknown id is `404` (SM
/// `ehr_id_does_not_exist` / `party_id_does_not_exist`) and a malformed one is
/// `400`, with nothing archived either way. A body that is not
/// `{ "<field>": [ … ] }` is `400` too.
#[tokio::test]
async fn the_archive_routes_refuse_unknown_malformed_and_shapeless_requests() {
    let (_pg, app) = app(true).await;

    for (path, field) in [
        ("/admin/archive/ehrs", "ehr_ids"),
        ("/admin/archive/parties", "party_ids"),
    ] {
        let (status, body) = send(
            &app,
            post_json(path, &format!(r#"{{"{field}":["{ABSENT}"]}}"#)),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path} unknown id: {body}");

        let (status, body) = send(
            &app,
            post_json(path, &format!(r#"{{"{field}":["not-a-uuid"]}}"#)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} malformed id: {body}"
        );

        let (status, body) = send(&app, post_json(path, r#"{"wrong":[]}"#)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} wrong shape: {body}"
        );

        // An EMPTY list archives nothing and succeeds — the SM parameter is a
        // list, and an empty one names no target to fail on.
        let (status, body) = send(&app, post_json(path, &format!(r#"{{"{field}":[]}}"#))).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{path} empty list: {body}");
    }
}

/// Create a demographic resource on `segment` and return its
/// `VERSIONED_OBJECT` uid (the `ETag` carries the `OBJECT_VERSION_ID`; its root
/// is the container).
async fn create_demographic(app: &Router, segment: &str, body: &serde_json::Value) -> String {
    let request = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/{segment}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "create {segment} must succeed"
    );
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned();
    etag.split("::")
        .next()
        .expect("versioned object uid")
        .to_owned()
}

/// A minimal demographic PARTY of `rm_type` (RM demographic `party.adoc`
/// §Invariants, `Identities_valid`: at least one `PARTY_IDENTITY`).
fn party_body(rm_type: &str, archetype: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "_type": rm_type,
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "label" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }]
    })
}

/// `archive_parties` selects PARTY version containers, so a
/// `PARTY_RELATIONSHIP`'s container id is refused exactly like an id that names
/// nothing: SM `i_admin_archive.adoc` §`archive_parties` takes `party_ids` and
/// declares `party_id_does_not_exist` as its only error, and RM demographic
/// `master02-demographic_package.adoc` puts every PARTY in "its own Version
/// container" (§Versioning Semantics) while relationships are stored "as part
/// of the data of the PARTY designated as the source" (§Party Relationships) —
/// a relationship travels with the parties the call selects and is never itself
/// a selectable party id. All-or-nothing: the party named alongside it stays
/// unarchived, and the same party archives fine on its own.
#[tokio::test]
async fn archiving_a_party_relationship_id_is_refused_like_an_unknown_party() {
    let (_pg, app) = app(true).await;

    let source = create_demographic(
        &app,
        "person",
        &party_body("PERSON", "openEHR-DEMOGRAPHIC-PERSON.person.v1", "Jane Doe"),
    )
    .await;
    let target = create_demographic(
        &app,
        "organisation",
        &party_body(
            "ORGANISATION",
            "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
            "General Hospital",
        ),
    )
    .await;
    // RM demographic master02 §Party Relationships: source/target are
    // "OBJECT_REFs containing HIER_OBJECT_IDs to denote the Version container
    // of a Party", so the refs name the two containers just created.
    let relationship = create_demographic(
        &app,
        "party_relationship",
        &serde_json::json!({
            "_type": "PARTY_RELATIONSHIP",
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
            "name": { "_type": "DV_TEXT", "value": "patient-of" },
            "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                        "id": { "_type": "HIER_OBJECT_ID", "value": source } },
            "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
                        "id": { "_type": "HIER_OBJECT_ID", "value": target } }
        }),
    )
    .await;

    let (status, body) = send(
        &app,
        post_json(
            "/admin/archive/parties",
            &format!(r#"{{"party_ids":["{relationship}"]}}"#),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a PARTY_RELATIONSHIP id names no Party: {body}"
    );

    // Alongside a real party the refusal still covers the whole request, and
    // that same party archives on its own — so the refusal is about the id's
    // kind, not about the call.
    let (status, body) = send(
        &app,
        post_json(
            "/admin/archive/parties",
            &format!(r#"{{"party_ids":["{source}","{relationship}"]}}"#),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "all-or-nothing: one relationship id refuses the whole set: {body}"
    );

    let (status, body) = send(
        &app,
        post_json(
            "/admin/archive/parties",
            &format!(r#"{{"party_ids":["{source}"]}}"#),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the party itself archives: {body}"
    );
}

// ── the restore pair (our own design — the SM declares no un-archive) ────────

/// Execute an ad-hoc AQL query and return how many rows it selected.
async fn aql_rows(app: &Router, aql: &str) -> usize {
    let request = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/query/aql"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::json!({ "q": aql }).to_string()))
        .expect("request");
    let (status, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK, "AQL {aql}: {body}");
    let result: serde_json::Value = serde_json::from_str(&body).expect("RESULT_SET json");
    result["rows"].as_array().expect("rows array").len()
}

/// How many rows of `relation` belong to `vo_id` (the archival move is physical,
/// so the primary/cold split is the only place it is observable).
///
/// Every `relation` argument in this file is a literal, so the interpolation
/// carries no caller data (`sqlx::AssertSqlSafe`).
async fn vo_rows(pool: &sqlx::PgPool, relation: &str, vo_id: &str) -> i64 {
    let vo = uuid::Uuid::parse_str(vo_id).expect("versioned object uuid");
    let sql = format!("SELECT count(*) FROM {relation} WHERE vo_id = $1");
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(vo)
        .fetch_one(pool)
        .await
        .expect("row count")
}

/// `POST /admin/archive/ehrs/restore` reverses the archival move: the EHR's
/// content returns to the primary tier, so it is AQL-visible again. Query
/// visibility is the operative difference — the engine reads the primary tier
/// only, while id-addressed reads are served from either tier and therefore
/// answer `200` throughout.
#[tokio::test]
async fn restoring_an_archived_ehr_makes_it_queryable_again() {
    let (_pg, app) = app(true).await;
    let ehr_id = create_ehr(&app).await;
    // Joins the EHR's current EHR_STATUS version, which archiving moves out of
    // the primary tier; `SELECT e/ehr_id/value` alone reads the `ehr` row, which
    // archival never touches, and would stay visible either way.
    let aql = "SELECT e/ehr_status/is_queryable FROM EHR e";
    let body = format!(r#"{{"ehr_ids":["{ehr_id}"]}}"#);

    assert_eq!(aql_rows(&app, aql).await, 1, "the fresh EHR is queryable");

    let (status, reply) = send(&app, post_json("/admin/archive/ehrs", &body)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "archive: {reply}");
    assert_eq!(
        aql_rows(&app, aql).await,
        0,
        "an archived EHR leaves the queryable store"
    );

    let (status, reply) = send(&app, post_json("/admin/archive/ehrs/restore", &body)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "restore: {reply}");
    assert_eq!(
        aql_rows(&app, aql).await,
        1,
        "the restored EHR is queryable again"
    );

    let (status, _) = send(&app, get(&format!("/ehr/{ehr_id}"))).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the id-addressed read answers 200 in both tiers"
    );
}

/// `POST /admin/archive/parties/restore` moves an archived party's versioned
/// object back to the primary tier, physically: the party's rows leave the cold
/// partition and reappear in the hot one.
#[tokio::test]
async fn restoring_an_archived_party_moves_its_rows_back_to_the_primary_tier() {
    let (pg, app) = app(true).await;
    let pool = pg.pool();

    let party = create_demographic(
        &app,
        "person",
        &party_body("PERSON", "openEHR-DEMOGRAPHIC-PERSON.person.v1", "Jane Doe"),
    )
    .await;
    let body = format!(r#"{{"party_ids":["{party}"]}}"#);
    let hot = vo_rows(&pool, "party.version_hot", &party).await;
    assert!(hot > 0, "the party has stored versions");

    let (status, reply) = send(&app, post_json("/admin/archive/parties", &body)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "archive: {reply}");
    assert_eq!(vo_rows(&pool, "party.version_hot", &party).await, 0);
    assert_eq!(vo_rows(&pool, "party.version_cold", &party).await, hot);

    let (status, reply) = send(&app, post_json("/admin/archive/parties/restore", &body)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "restore: {reply}");
    assert_eq!(vo_rows(&pool, "party.version_hot", &party).await, hot);
    assert_eq!(vo_rows(&pool, "party.version_cold", &party).await, 0);
    let markers = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM party.vo_head WHERE vo_id = $1 AND archived_at IS NOT NULL",
    )
    .bind(uuid::Uuid::parse_str(&party).expect("party uuid"))
    .fetch_one(&pool)
    .await
    .expect("marker count");
    assert_eq!(markers, 0, "restore drops the archive marker");
}

/// Restoring a record that was never archived is a no-op success on both
/// halves — the mirror of re-archiving an archived record. The existence check
/// still applies, so the id has to name a real record.
#[tokio::test]
async fn restoring_an_unarchived_record_succeeds_and_changes_nothing() {
    let (pg, app) = app(true).await;
    let pool = pg.pool();
    let ehr_id = create_ehr(&app).await;
    let party = create_demographic(
        &app,
        "person",
        &party_body("PERSON", "openEHR-DEMOGRAPHIC-PERSON.person.v1", "John Doe"),
    )
    .await;
    let hot = vo_rows(&pool, "party.version_hot", &party).await;

    for (path, body) in [
        (
            "/admin/archive/ehrs/restore",
            format!(r#"{{"ehr_ids":["{ehr_id}"]}}"#),
        ),
        (
            "/admin/archive/parties/restore",
            format!(r#"{{"party_ids":["{party}"]}}"#),
        ),
    ] {
        let (status, reply) = send(&app, post_json(path, &body)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{path}: {reply}");
    }

    assert_eq!(
        vo_rows(&pool, "party.version_hot", &party).await,
        hot,
        "an unarchived party is left exactly as it was"
    );
    assert_eq!(vo_rows(&pool, "party.version_cold", &party).await, 0);
    let (status, _) = send(&app, get(&format!("/ehr/{ehr_id}"))).await;
    assert_eq!(status, StatusCode::OK);
}

/// The restore halves carry the archive halves' refusals unchanged: an unknown
/// id is `404`, a malformed one `400`, a body of the wrong shape `400`, and an
/// empty list restores nothing and succeeds — all-or-nothing in every branch.
#[tokio::test]
async fn the_restore_routes_refuse_unknown_malformed_and_shapeless_requests() {
    let (_pg, app) = app(true).await;

    for (path, field) in [
        ("/admin/archive/ehrs/restore", "ehr_ids"),
        ("/admin/archive/parties/restore", "party_ids"),
    ] {
        let (status, body) = send(
            &app,
            post_json(path, &format!(r#"{{"{field}":["{ABSENT}"]}}"#)),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path} unknown id: {body}");

        let (status, body) = send(
            &app,
            post_json(path, &format!(r#"{{"{field}":["not-a-uuid"]}}"#)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} malformed id: {body}"
        );

        let (status, body) = send(&app, post_json(path, r#"{"wrong":[]}"#)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} wrong shape: {body}"
        );

        let (status, body) = send(&app, post_json(path, &format!(r#"{{"{field}":[]}}"#))).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{path} empty list: {body}");
    }
}

// ── the dump/load pair (SM I_ADMIN_DUMP_LOAD) ────────────────────────────────

/// A unique archive location for one test (the SM `file_sys_loc` parameter is
/// a location on the SERVER's file system, which in-process is this one).
fn archive_dir() -> String {
    std::env::temp_dir()
        .join(format!("ferroehr-cnf-dump-{}", uuid::Uuid::now_v7()))
        .to_string_lossy()
        .into_owned()
}

/// `export_ehrs` writes the archive and answers `200` with the SM
/// `List<DUMP_LOAD_FAIL_REPORT>` — EMPTY when every EHR was dumped. Requesting
/// the `zip` `COMPRESSION_FORMAT` member packs the same archive into one
/// container, and `load_ehrs` — which takes only `file_sys_loc` — reads either
/// container back.
#[tokio::test]
async fn dump_and_load_round_trip_in_both_containers() {
    for compression in [None, Some("zip")] {
        let (_pg, app) = app(true).await;
        let _ehr = create_ehr(&app).await;
        let dir = archive_dir();

        let mut spec = serde_json::json!({
            "file_sys_loc": dir,
            "logical_format": "openehr_canonical_json",
            "segment_split_size": 1024
        });
        if let Some(member) = compression {
            spec["compression_format"] = serde_json::Value::String(member.to_owned());
        }
        let (status, body) = send(&app, post_json("/admin/dump", &spec.to_string())).await;
        assert_eq!(status, StatusCode::OK, "dump {compression:?}: {body}");
        assert_eq!(body, "[]", "a clean dump reports no failures");

        // Loading the archive back into the SAME repository is the SM's own
        // documented duplicate case ("import EHRs with duplicate EHR ids will
        // fail"): the call succeeds and reports the entity, never a fatal.
        let (status, body) = send(
            &app,
            post_json("/admin/load", &format!(r#"{{"file_sys_loc":{dir:?}}}"#)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "load {compression:?}: {body}");
        let reports: serde_json::Value = serde_json::from_str(&body).expect("report json");
        let list = reports.as_array().expect("a JSON array");
        assert_eq!(list.len(), 1, "the one duplicate EHR is reported: {body}");
        assert_eq!(list[0]["entity_type"], "EHR");
        assert_eq!(list[0]["dump_status"], false);
        assert!(list[0]["error"].is_string(), "with an explanatory error");

        let _cleanup = std::fs::remove_dir_all(&dir);
    }
}

/// The request-shape refusals, each SM `precondition_violation` → `400`:
/// an absent/blank `file_sys_loc`, a value naming no enumeration member, a
/// non-integer `segment_split_size`, and `encoding` — whose `ENCODING_FORMAT`
/// enumeration (`encoding_format.adoc`) declares NO members, so no value is
/// representable.
#[tokio::test]
async fn dump_refuses_every_unrepresentable_request_shape() {
    let (_pg, app) = app(true).await;
    let dir = archive_dir();

    for (label, body) in [
        (
            "no location",
            serde_json::json!({ "segment_split_size": 1024 }),
        ),
        (
            "blank location",
            serde_json::json!({ "file_sys_loc": "   " }),
        ),
        (
            "unknown logical format",
            serde_json::json!({ "file_sys_loc": dir, "logical_format": "canonical_json" }),
        ),
        (
            "unknown compression format",
            serde_json::json!({ "file_sys_loc": dir, "compression_format": "gzip" }),
        ),
        (
            "encoding (empty enumeration)",
            serde_json::json!({ "file_sys_loc": dir, "encoding": "utf_8" }),
        ),
        (
            "non-integer split size",
            serde_json::json!({ "file_sys_loc": dir, "segment_split_size": "big" }),
        ),
        (
            "non-positive split size",
            serde_json::json!({ "file_sys_loc": dir, "segment_split_size": 0 }),
        ),
    ] {
        let (status, response) = send(&app, post_json("/admin/dump", &body.to_string())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {response}");
    }
    let _cleanup = std::fs::remove_dir_all(&dir);
}

/// EVERY declared enumeration member is served: both `EXPORT_FORMAT` members
/// (`export_format.adoc`: `openehr_canonical_xml`, `openehr_canonical_json`)
/// crossed with every `COMPRESSION_FORMAT` shape (`compression_format.adoc`:
/// absent, `zip`, `7z`). No member is refused, downgraded, or answered
/// `501` — the two enumerations are realized in full, and the format axis is
/// independent of the container axis.
#[tokio::test]
async fn dump_serves_every_declared_enumeration_member() {
    let (_pg, app) = app(true).await;
    let _ehr = create_ehr(&app).await;

    for logical in ["openehr_canonical_json", "openehr_canonical_xml"] {
        for compression in [None, Some("zip"), Some("7z")] {
            let dir = archive_dir();
            let mut spec = serde_json::json!({
                "file_sys_loc": dir,
                "logical_format": logical,
                "segment_split_size": 1024
            });
            if let Some(member) = compression {
                spec["compression_format"] = serde_json::Value::String(member.to_owned());
            }
            let (status, body) = send(&app, post_json("/admin/dump", &spec.to_string())).await;
            assert_eq!(status, StatusCode::OK, "{logical}/{compression:?}: {body}");
            assert_eq!(body, "[]", "a clean dump reports no failures");

            // The format-less `load_ehrs` reads it back whatever the export
            // asked for: every archived EHR is the SM's documented duplicate.
            let (status, body) = send(
                &app,
                post_json("/admin/load", &format!(r#"{{"file_sys_loc":{dir:?}}}"#)),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::OK,
                "load {logical}/{compression:?}: {body}"
            );
            let reports: serde_json::Value = serde_json::from_str(&body).expect("report json");
            let list = reports.as_array().expect("a JSON array");
            assert_eq!(list.len(), 1, "the one duplicate EHR is reported: {body}");
            assert_eq!(list[0]["dump_status"], false);

            let _cleanup = std::fs::remove_dir_all(&dir);
        }
    }
}

/// Every route of all three extension groups inherits the ADMIN group's config
/// gate: with `admin.enabled` off they answer `405` with an empty `Allow`,
/// before the backend is touched.
#[tokio::test]
async fn the_admin_gate_covers_the_extension_groups() {
    let (_pg, app) = app(false).await;

    let (status, _) = send(&app, get("/admin/report/contribution?a_service=Ehr")).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

    let (status, _) = send(&app, post_json("/admin/archive/ehrs", r#"{"ehr_ids":[]}"#)).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

    let (status, _) = send(
        &app,
        post_json("/admin/archive/ehrs/restore", r#"{"ehr_ids":[]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

    let (status, _) = send(
        &app,
        post_json("/admin/dump", r#"{"file_sys_loc":"/tmp/never"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

    let (status, _) = send(
        &app,
        post_json("/admin/load", r#"{"file_sys_loc":"/tmp/never"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}
