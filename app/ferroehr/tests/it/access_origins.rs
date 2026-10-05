// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log records the origin of the data it served (#3212).
//!
//! EHDS Annex II 3.2(e) asks for "the origin or origins of data". openEHR
//! carries provenance on the content: `FEEDER_AUDIT.originating_system_audit`
//! names "the IT system owned by the organisation legally responsible for
//! handling the data, and at which the data were previously created" (RM
//! common, `FEEDER_AUDIT_DETAILS.system_id`), on any `LOCATABLE`. A body with
//! none was created here, and its origin is this server.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use serde_json::{Value, json};
use sqlx::Row;

use crate::fixtures::{composition, uv};

/// A `FEEDER_AUDIT` whose originating system is `system_id`.
fn feeder_audit(system_id: &str) -> Value {
    json!({
        "_type": "FEEDER_AUDIT",
        "originating_system_audit": {
            "_type": "FEEDER_AUDIT_DETAILS",
            "system_id": system_id
        }
    })
}

/// A `SECTION` (a `LOCATABLE` with no mandatory content) stamped with a
/// `FEEDER_AUDIT` from `system_id`.
fn section_from(name: &str, system_id: &str) -> Value {
    json!({
        "_type": "SECTION",
        "archetype_node_id": "openEHR-EHR-SECTION.adhoc.v1",
        "name": { "_type": "DV_TEXT", "value": name },
        "feeder_audit": feeder_audit(system_id)
    })
}

async fn setup() -> (testkit::TestDb, FerroEhrService, EhrId) {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr");
    (db, svc, ehr_id)
}

/// Commit `body` and return the versioned object id and the creating system.
async fn commit(svc: &FerroEhrService, ehr_id: EhrId, body: &Value) -> (VoId, String) {
    let created = svc
        .create_composition(ehr_id, uv(body, "249", None))
        .await
        .expect("commit the composition");
    (created.vo_id, created.creating_system_id)
}

/// The origins the latest read of `vo` serves, from the response metadata.
async fn served_origins(svc: &FerroEhrService, ehr_id: EhrId, vo: VoId) -> Vec<String> {
    svc.composition_latest_response(ehr_id, vo)
        .await
        .expect("the latest read")
        .meta
        .expect("a served version carries metadata")
        .origins
}

/// The commit-time stamp of the current version of `vo`.
async fn stamped_origins(db: &testkit::TestDb, vo: VoId) -> Option<Value> {
    sqlx::query(
        "SELECT v.origins FROM version v JOIN vo_head h ON h.vo_id = v.vo_id \
         AND h.trunk_head_sys_version = v.sys_version WHERE v.vo_id = $1",
    )
    .bind(vo.0)
    .fetch_one(&db.pool())
    .await
    .expect("the current version row")
    .get("origins")
}

/// A body carrying no `FEEDER_AUDIT` was created through this API: its one
/// origin is this server, and the read says so.
#[tokio::test]
async fn a_body_without_feeder_audit_has_this_server_as_its_origin() {
    let (db, svc, ehr_id) = setup().await;
    let (vo, creating_system) = commit(&svc, ehr_id, &composition("home grown")).await;

    assert_eq!(
        stamped_origins(&db, vo).await,
        Some(json!([creating_system])),
        "the commit stamps the creating system as the sole origin"
    );
    assert_eq!(served_origins(&svc, ehr_id, vo).await, [creating_system]);
}

/// Every `FEEDER_AUDIT` anywhere in the body counts once: the composition's
/// own, one on a content item, and a duplicate that collapses.
#[tokio::test]
async fn feeder_audits_anywhere_in_the_body_count_once_each() {
    let (db, svc, ehr_id) = setup().await;
    let mut body = composition("imported");
    body["feeder_audit"] = feeder_audit("lab.example");
    body["content"] = json!([
        section_from("imaging", "pacs.example"),
        section_from("lab again", "lab.example"),
    ]);
    let (vo, _) = commit(&svc, ehr_id, &body).await;

    assert_eq!(
        stamped_origins(&db, vo).await,
        Some(json!(["lab.example", "pacs.example"])),
        "distinct, sorted, and never this server when the body names its origins"
    );
    assert_eq!(
        served_origins(&svc, ehr_id, vo).await,
        ["lab.example", "pacs.example"]
    );
}

/// An AQL page records the union of the origins over the version rows it
/// served, with the true distinct count beside the set.
#[tokio::test]
async fn an_aql_page_records_the_union_of_the_origins_it_served() {
    let (_db, svc, ehr_id) = setup().await;
    let (_, creating_system) = commit(&svc, ehr_id, &composition("home grown")).await;
    let mut imported = composition("imported");
    imported["feeder_audit"] = feeder_audit("lab.example");
    commit(&svc, ehr_id, &imported).await;

    let outcome = svc
        .execute_ad_hoc_query(
            format!(
                "SELECT c/uid/value FROM EHR e[ehr_id/value='{ehr_id}'] CONTAINS COMPOSITION c"
            ),
            AqlQueryRequest::default(),
        )
        .await
        .expect("the query runs");
    let mut expected = vec![creating_system, "lab.example".to_owned()];
    expected.sort();
    assert_eq!(outcome.served_origins, expected, "{}", outcome.result_set);
    assert_eq!(outcome.origin_count, 2);

    // A leaf projection over one root still knows which versions it served.
    let one = svc
        .execute_ad_hoc_query(
            format!(
                "SELECT c/name/value FROM EHR e[ehr_id/value='{ehr_id}'] CONTAINS COMPOSITION c[name/value='imported']"
            ),
            AqlQueryRequest::default(),
        )
        .await
        .expect("the query runs");
    assert_eq!(one.served_origins, ["lab.example"]);
    assert_eq!(one.origin_count, 1);
}

/// A row nothing stamped (pre-column, or a verbatim archive load) is assessed
/// at read from the body in hand; the SQL aggregate an AQL page uses has no
/// body to walk and reports the creating system, which is the honest answer
/// the migration documents rather than a claim about content it did not read.
#[tokio::test]
async fn an_unstamped_row_is_assessed_from_the_body_at_read() {
    let (db, svc, ehr_id) = setup().await;
    let mut imported = composition("imported");
    imported["feeder_audit"] = feeder_audit("lab.example");
    let (vo, creating_system) = commit(&svc, ehr_id, &imported).await;

    sqlx::query("UPDATE version SET origins = NULL WHERE vo_id = $1")
        .bind(vo.0)
        .execute(&db.pool())
        .await
        .expect("unstamp the row");
    assert_eq!(stamped_origins(&db, vo).await, None);

    assert_eq!(
        served_origins(&svc, ehr_id, vo).await,
        ["lab.example"],
        "the point read walks the body it serves"
    );
    let outcome = svc
        .execute_ad_hoc_query(
            format!(
                "SELECT c/uid/value FROM EHR e[ehr_id/value='{ehr_id}'] CONTAINS COMPOSITION c"
            ),
            AqlQueryRequest::default(),
        )
        .await
        .expect("the query runs");
    assert_eq!(
        outcome.served_origins,
        [creating_system],
        "the aggregate falls back to the creating system for an unstamped row"
    );
}
