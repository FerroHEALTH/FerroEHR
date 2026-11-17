// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An AQL statement shape is a named statement, prepared once on a
//! connection and executed again from there, while text built for one
//! request goes unnamed (#5884). No openEHR spec governs statement
//! preparation: our own design.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use ferroehr::db::DbConfig;
use ferroehr::db::domain::{Domain, DomainPools};
use ferroehr::service::FerroEhrService;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{self, BASE};

const USERS: &[(&str, &[&str])] = &[("root", &["ADMIN"]), ("nurse", &["USER"])];

/// The executions of one query the test sends.
const RUNS: i64 = 8;

/// The requests of a key before it is bound from its shape: the first
/// remembers the key, the second derives the shape (`aql::sql::shape`).
const UNSHAPED: i64 = 2;

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

/// A replica over pools of one connection each, and its clinical pool.
async fn replica(db: &testkit::TestDb) -> (Router, PgPool) {
    let mut settings = DbConfig::new(db.url());
    settings.max_connections = Some(1);
    settings.min_connections = Some(0);
    settings.connection_budget = 8;
    let pools: DomainPools = ferroehr::db::connect_domains(&settings)
        .await
        .expect("the pools connect");
    let clinical = pools.get(Domain::Clinical).clone();
    let svc = Arc::new(
        FerroEhrService::new(&pools).with_binding_mapping(crate::party_binding_http::mapping()),
    );
    let mut config = common::api_config(true);
    common::with_basic_users(&mut config, &svc, &db.pool(), USERS).await;
    let app = common::build_full(config, svc, None, Observability::default()).await;
    (app, clinical)
}

/// A single-EHR query sent `RUNS` times on one connection is one named
/// statement, executed by every request bound from its shape.
#[tokio::test]
async fn a_statement_shape_is_prepared_once_and_reused() {
    let db = testkit::db().await.expect("testkit database");
    let (app, clinical) = replica(&db).await;
    let (status, body) = common::send_body(
        &app,
        request(
            "POST",
            &format!("{BASE}/definition/template/adl1.4"),
            "root",
            Some(("application/xml", common::ips_opt_xml())),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    let (status, body) =
        common::send_body(&app, request("POST", &format!("{BASE}/ehr"), "root", None)).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let ehr = serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let (status, body) = common::send_body(
        &app,
        request(
            "POST",
            &format!("{BASE}/ehr/{ehr}/composition"),
            "root",
            Some((
                "application/json",
                common::ips_canonical_composition().to_string(),
            )),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    let query = json!({
        "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = $ehr",
        "query_parameters": { "ehr": ehr },
    })
    .to_string();
    for run in 0..RUNS {
        let (status, body) = common::send_body(
            &app,
            request(
                "POST",
                &format!("{BASE}/query/aql"),
                "nurse",
                Some(("application/json", query.clone())),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "run {run}: {body}");
        let rows = serde_json::from_str::<Value>(&body).expect("a RESULT_SET")["rows"]
            .as_array()
            .map_or(0, Vec::len);
        assert_eq!(rows, 1, "run {run}: {body}");
    }
    let mut conn = ferroehr::db::acquire::acquire(&clinical)
        .await
        .expect("the one clinical connection");
    let prepared: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT generic_plans, custom_plans FROM pg_prepared_statements \
         WHERE statement LIKE '%access_ehr_0%' AND statement NOT LIKE '%pg_prepared%'",
    )
    .fetch_all(&mut *conn)
    .await
    .expect("pg_prepared_statements reads");
    let held: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(total_bytes), 0)::bigint FROM pg_backend_memory_contexts \
         WHERE name IN ('CachedPlan', 'CachedPlanSource', 'CachedPlanQuery') \
         AND ident LIKE '%access_ehr_0%'",
    )
    .fetch_one(&mut *conn)
    .await
    .expect("pg_backend_memory_contexts reads");
    eprintln!(
        "the AQL statement: {prepared:?} (generic, custom plans); its plans hold {held} bytes"
    );
    assert_eq!(
        prepared.len(),
        1,
        "the shape is one named statement on the connection: {prepared:?}"
    );
    let executions: i64 = prepared
        .iter()
        .map(|(generic, custom)| generic + custom)
        .sum();
    assert_eq!(
        executions,
        RUNS - UNSHAPED,
        "every request bound from the shape executes the one statement"
    );
}
