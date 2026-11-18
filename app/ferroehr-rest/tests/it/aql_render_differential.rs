// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL cells the server renders in Rust are byte-identical to the cells
//! PostgreSQL's jsonb projection served before them (#6004).
//!
//! A debug build executes every AQL statement a second time with its
//! projection built as jsonb in SQL and asserts that each row's cells are the
//! same bytes (`ferroehr::aql::exec`). This suite drives that check over the
//! capacity fixtures, the lab result and the International Patient Summary,
//! through every kind of projected path the emitter writes: object and scalar
//! leaves, the members a node row keeps in its columns, compact markers,
//! typed ELEMENT values, missing members, lists, fragment filters, VERSION
//! and EHR fields, aggregates, functions and literals. No openEHR spec
//! governs the rendering mechanics: our own design.

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};

use crate::common::{self, BASE};

const USERS: &[(&str, &[&str])] = &[("root", &["ADMIN"]), ("nurse", &["USER"])];

const LAB: &str = "COMPOSITION c[openEHR-EHR-COMPOSITION.report-result.v1] \
                   CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.laboratory_test_result.v1] \
                   CONTAINS CLUSTER a[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1]";

/// The projections the suite sends, each `FROM EHR e CONTAINS <from>` and
/// scoped to one EHR.
const CASES: &[(&str, &str)] = &[
    // The composition root: synthesized uid, header members, compact markers.
    (
        "c/uid/value, c/uid, c/name, c/name/value, c/archetype_node_id, \
         c/archetype_details, c/archetype_details/template_id/value, \
         c/archetype_details/rm_version, c/language, c/language/code_string, \
         c/territory, c/category, c/category/defining_code/code_string, c/composer, \
         c/composer/name, c/context, c/context/start_time, c/context/start_time/value, \
         c/context/setting, c/context/health_care_facility, c/links, c/links/target/value, \
         c/feeder_audit, c/feeder_audit/originating_system_item_ids",
        "COMPOSITION c",
    ),
    // An entry below the root: its own fragment, origin and time markers,
    // the subject, a missing member.
    (
        "o/name, o/name/value, o/archetype_node_id, o/language, o/encoding, \
         o/encoding/code_string, o/subject, o/data/origin, o/data/origin/value, \
         o/data/events/time, o/data/events/time/value, o/protocol, o/uid/value, \
         o/guideline_id, o/workflow_id/id/value",
        LAB,
    ),
    // ELEMENT values from the typed value columns and from the fragment.
    (
        "a/items/value, a/items/value/magnitude, a/items/value/units, a/items/value/value, \
         a/items/value/defining_code/code_string, \
         a/items/value/defining_code/terminology_id/value, a/items/name, a/items/null_flavour, \
         a/items/value/precision, a/items/value/normal_range",
        LAB,
    ),
    // A filter on a fragment step, true, false and on a missing member.
    (
        "c/category/defining_code[code_string='433']/terminology_id/value, \
         c/category/defining_code[code_string='999']/terminology_id/value, \
         c/territory[code_string!='NL']/code_string, \
         c/links[type/value='nothing']/target/value",
        "COMPOSITION c",
    ),
    // Every IPS node of a few classes: lists, nested clusters, coded values.
    (
        "x/name/value, x/archetype_node_id, x/data/events/time/value, x/data/origin, \
         x/subject, x/language",
        "COMPOSITION c CONTAINS OBSERVATION x",
    ),
    (
        "ev/name/value, ev/data, ev/protocol, ev/archetype_details",
        "COMPOSITION c CONTAINS EVALUATION ev",
    ),
    (
        "el/value, el/name, el/value/value, el/value/magnitude, el/value/units, \
         el/value/defining_code, el/value/symbol, el/value/id, el/value/mappings, \
         el/value/mappings/target/code_string",
        "COMPOSITION c CONTAINS ELEMENT el",
    ),
    // VERSION and EHR fields: text, timestamptz, jsonb columns.
    (
        "v/uid/value, v/commit_audit/time_committed, v/commit_audit/time_committed/value, \
         v/commit_audit/committer, v/commit_audit/change_type/value, \
         v/commit_audit/description, v/lifecycle_state/value, v/lifecycle_state/defining_code/code_string, v/commit_audit/change_type/defining_code/terminology_id/value, \
         v/contribution/id/value, e/ehr_id/value, e/time_created, e/time_created/value, \
         e/system_id/value, e/ehr_status/subject, e/ehr_status/is_queryable, \
         e/ehr_status/is_modifiable, e/ehr_status/name/value",
        "VERSION v CONTAINS COMPOSITION c",
    ),
    // Aggregates over numbers, text and instants.
    (
        "COUNT(*), COUNT(el/value/magnitude), COUNT(DISTINCT el/name/value), \
         MAX(el/value/magnitude), MIN(el/value/magnitude), SUM(el/value/magnitude), \
         AVG(el/value/magnitude), MIN(c/context/start_time/value), \
         MAX(c/context/start_time/value), MAX(el/name/value)",
        "COMPOSITION c CONTAINS ELEMENT el",
    ),
    // Functions and literals.
    (
        "LENGTH(c/name/value), CONCAT(c/name/value, '-', c/language/code_string), \
         SUBSTRING(c/name/value, 2, 3), POSITION('a', c/name/value), \
         CONTAINS(c/name/value, 'a'), 1, 1.0, 1.5, 0.1, 100000000000000000000.0, 'text', \
         true, false",
        "COMPOSITION c",
    ),
    (
        "ABS(a/items/value/magnitude), ROUND(a/items/value/magnitude, 1), \
         CEIL(a/items/value/magnitude), FLOOR(a/items/value/magnitude), \
         MOD(a/items/value/magnitude, 7)",
        LAB,
    ),
];

/// `method uri` as `user`, with an optional body of the content type given.
fn request(method: &str, uri: &str, user: &str, body: Option<(&str, String)>) -> Request<Body> {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, common::basic_credential(user))
        .header(header::ACCEPT, "application/json")
        .header("Prefer", "return=representation");
    match body {
        Some((kind, body)) => req
            .header(header::CONTENT_TYPE, kind)
            .body(Body::from(body)),
        None => req.body(Body::empty()),
    }
    .expect("a request")
}

/// Sends `req` and requires `want`.
async fn send(app: &Router, req: Request<Body>, want: StatusCode, what: &str) -> String {
    let (status, body) = common::send_body(app, req).await;
    assert_eq!(status, want, "{what}: {body}");
    body
}

/// A new EHR with `composition` committed in it.
async fn ehr_with(app: &Router, composition: String) -> String {
    let body = send(
        app,
        request("POST", &format!("{BASE}/ehr"), "root", None),
        StatusCode::CREATED,
        "EHR create",
    )
    .await;
    let ehr = serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    send(
        app,
        request(
            "POST",
            &format!("{BASE}/ehr/{ehr}/composition"),
            "root",
            Some(("application/json", composition)),
        ),
        StatusCode::CREATED,
        "commit",
    )
    .await;
    ehr
}

#[tokio::test]
#[cfg_attr(
    not(debug_assertions),
    ignore = "the differential check of the rendering runs in a debug build"
)]
async fn every_projected_path_renders_the_cells_the_jsonb_projection_served() {
    let db = testkit::db().await.expect("testkit database");
    let svc = std::sync::Arc::new(common::binding_service(&db.pool()));
    let mut config = common::api_config(true);
    common::with_basic_users(&mut config, &svc, &db.pool(), USERS).await;
    let app = common::build_full(
        config,
        svc,
        None,
        ferroehr_rest::extensions::management::Observability::default(),
    )
    .await;
    let root = env!("CARGO_MANIFEST_DIR");
    let lab_opt = std::fs::read_to_string(format!(
        "{root}/../../corpus/templates/ckm/generic-lab-test-result.opt"
    ))
    .expect("the lab OPT");
    let lab: Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{root}/../../corpus/templates/ckm/generic-lab-test-result.example.json"
        ))
        .expect("the lab composition"),
    )
    .expect("lab json");
    for opt in [lab_opt, common::ips_opt_xml()] {
        send(
            &app,
            request(
                "POST",
                &format!("{BASE}/definition/template/adl1.4"),
                "root",
                Some(("application/xml", opt)),
            ),
            StatusCode::CREATED,
            "OPT upload",
        )
        .await;
    }
    let ehrs = [
        ehr_with(&app, lab.to_string()).await,
        ehr_with(&app, common::ips_canonical_composition().to_string()).await,
    ];
    let mut served = 0usize;
    let mut refused = Vec::new();
    for ehr in &ehrs {
        for (select, from) in CASES {
            let q =
                format!("SELECT {select} FROM EHR e CONTAINS {from} WHERE e/ehr_id/value = $ehr");
            let (status, body) = common::send_body(
                &app,
                request(
                    "POST",
                    &format!("{BASE}/query/aql"),
                    "nurse",
                    Some((
                        "application/json",
                        json!({ "q": q, "query_parameters": { "ehr": ehr } }).to_string(),
                    )),
                ),
            )
            .await;
            if status != StatusCode::OK {
                refused.push(format!("{status} {q}: {body}"));
                continue;
            }
            served += serde_json::from_str::<Value>(&body).expect("a RESULT_SET")["rows"]
                .as_array()
                .map_or(0, Vec::len);
        }
    }
    assert!(
        refused.is_empty(),
        "every case answers:\n{}",
        refused.join("\n")
    );
    assert!(served > 0, "the cases served rows to compare");
}
