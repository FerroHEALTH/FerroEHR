// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A burst of first touches on a replica that has just started, with its
//! `EHR_ACCESS` listener connected and the access log on: every request is
//! answered, no request asks a pool for a second connection while it holds
//! one (debug builds refuse that, #5879), and the reads that fill the copies
//! are counted. No openEHR spec governs the copies or connection handling:
//! our own design.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::db::DbConfig;
use ferroehr::db::domain::{Domain, DomainPools};
use ferroehr::ids::EhrId;
use ferroehr::policy_register::listen::Listeners;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::sender::{ResolvedSubject, SubjectLookup, SubjectResolver, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{self, BASE, FHIR};

const USERS: &[(&str, &[&str])] = &[("root", &["ADMIN"]), ("nurse", &["USER"])];

/// The EHRs the burst reaches.
const EHRS: usize = 4;

/// The concurrent first requests of each kind on each EHR.
const READS: usize = 8;

/// The subject resolver the server wires: one read of the clinical pool.
fn resolver(pool: PgPool) -> SubjectResolver {
    Arc::new(move |ehr_id: String| {
        let pool = pool.clone();
        Box::pin(async move {
            let Ok(id) = uuid::Uuid::parse_str(&ehr_id) else {
                return SubjectLookup::NoSubject;
            };
            match ferroehr::storage::ehr_repo::ehr_subject_columns(&pool, EhrId(id)).await {
                Ok(Some((Some(id), namespace))) => {
                    SubjectLookup::Found(ResolvedSubject { id, namespace })
                }
                Ok(_) => SubjectLookup::NoSubject,
                Err(_) => SubjectLookup::Unavailable,
            }
        })
    })
}

/// A replica over pools of `size` connections each, its copies cold, its
/// `EHR_ACCESS` listener connected, and the access log on.
async fn replica(db: &testkit::TestDb, size: u32) -> Router {
    let mut settings = DbConfig::new(db.url());
    settings.max_connections = Some(size);
    settings.min_connections = Some(0);
    settings.connection_budget = 16;
    settings.acquire_timeout = ferrohealth_config::units::Duration::from_secs(10);
    let pools: DomainPools = ferroehr::db::connect_domains(&settings)
        .await
        .expect("the pools connect");
    let clinical = pools.get(Domain::Clinical).clone();
    let audit = pools.get(Domain::Audit).clone();
    let (sender, _handle) = start(
        common::local_audit(),
        Some(resolver(clinical)),
        audit.clone(),
    )
    .await
    .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&pools)
            .with_binding_mapping(crate::party_binding_http::mapping())
            .with_audit(sender)
            .with_audit_store(AuditStore::new(audit)),
    );
    Listeners::on(
        (*db.pool().connect_options()).clone(),
        &[Arc::clone(svc.ehr_access_listened())],
    )
    .spawn();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !svc.ehr_access_listened().is_listening() {
        assert!(
            std::time::Instant::now() < deadline,
            "the listener connects"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut config = common::api_config(true);
    common::with_basic_users(&mut config, &svc, &db.pool(), USERS).await;
    common::build_full(config, svc, None, Observability::default()).await
}

/// The calls `pg_stat_statements` counted of the statements starting with
/// `prefix` in the test's database.
async fn calls(pool: &PgPool, prefix: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT /* cold_start_burst_own */ COALESCE(sum(calls), 0)::bigint FROM \
         pg_stat_statements WHERE dbid = (SELECT oid FROM pg_database WHERE datname = \
         current_database()) AND toplevel AND regexp_replace(query, '\\s+', ' ', 'g') LIKE $1 || '%'",
    )
    .bind(prefix)
    .fetch_one(pool)
    .await
    .expect("pg_stat_statements reads")
}

/// The read that fills the per-EHR access copy.
const SETTINGS_READ: &str = "SELECT s.ehr_id IS NOT NULL AS has_settings";
/// The read that fills the access log's subject memo.
const SUBJECT_READ: &str = "SELECT subject_id, subject_namespace FROM ehr WHERE id = $1";

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

/// An EHR with one IPS composition, committed as the administrator through
/// `app`: the EHR id and the composition's version id.
async fn ehr_with_composition(app: &Router) -> (String, String) {
    let (status, body) =
        common::send_body(app, request("POST", &format!("{BASE}/ehr"), "root", None)).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let ehr = serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let composition = common::ips_canonical_composition().to_string();
    let (status, headers, body) = common::send(
        app,
        request(
            "POST",
            &format!("{BASE}/ehr/{ehr}/composition"),
            "root",
            Some(("application/json", composition)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    let uid = headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned();
    (ehr, uid)
}

/// The routes of the burst.
const KINDS: [&str; 5] = [
    "ehr read",
    "composition read",
    "composition commit",
    "single-EHR AQL",
    "access log read",
];

/// The request of kind `kind` on `ehr`, whose composition is `uid`.
fn touch(kind: usize, ehr: &str, uid: &str) -> Request<Body> {
    match kind {
        0 => request("GET", &format!("{BASE}/ehr/{ehr}"), "nurse", None),
        1 => request(
            "GET",
            &format!("{BASE}/ehr/{ehr}/composition/{uid}"),
            "nurse",
            None,
        ),
        2 => request(
            "POST",
            &format!("{BASE}/ehr/{ehr}/composition"),
            "nurse",
            Some((
                "application/json",
                common::ips_canonical_composition().to_string(),
            )),
        ),
        3 => request(
            "POST",
            &format!("{BASE}/query/aql"),
            "nurse",
            Some((
                "application/json",
                json!({
                    "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
                          WHERE e/ehr_id/value = $ehr",
                    "query_parameters": { "ehr": ehr },
                })
                .to_string(),
            )),
        ),
        _ => request(
            "GET",
            &format!("{FHIR}/AuditEvent?date=ge2026-01-01&_count=20"),
            "root",
            None,
        ),
    }
}

/// A burst of first requests of every kind on `EHRS` EHRs, on a replica with
/// two connections a pool, is answered without any request holding two
/// connections of one pool; the settings of each EHR are read at most once,
/// and no record asks for a subject its request read in the store.
#[tokio::test]
async fn a_burst_of_first_requests_reads_each_ehr_once() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect("pg_stat_statements: the testkit server loads it");
    let warm = replica(&db, 4).await;
    let (status, body) = common::send_body(
        &warm,
        request(
            "POST",
            &format!("{BASE}/definition/template/adl1.4"),
            "root",
            Some(("application/xml", common::ips_opt_xml())),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    let mut ehrs = Vec::new();
    for _ in 0..EHRS {
        let (ehr, uid) = ehr_with_composition(&warm).await;
        subject_moves(&pool, &ehr, &uuid::Uuid::now_v7().to_string()).await;
        ehrs.push((ehr, uid));
    }

    let app = replica(&db, 2).await;
    let evicted = testkit::statements::statement_window(&pool)
        .await
        .expect("the statement counts reset");
    let mut burst = tokio::task::JoinSet::new();
    for _ in 0..READS {
        for (ehr, uid) in &ehrs {
            for kind in 0..KINDS.len() {
                let app = app.clone();
                let request = touch(kind, ehr, uid);
                burst.spawn(async move {
                    let (status, body) = common::send_body(&app, request).await;
                    (kind, status, body)
                });
            }
        }
    }
    let answered = tokio::time::timeout(Duration::from_secs(60), burst.join_all())
        .await
        .expect("the burst is answered before the acquire timeouts");
    let failed: Vec<String> = answered
        .iter()
        .filter(|(_, status, _)| !status.is_success())
        .map(|(kind, status, body)| format!("{}: {status} {body}", KINDS[*kind]))
        .collect();
    assert!(
        failed.is_empty(),
        "every first request is answered: {failed:#?}"
    );
    let settings = calls(&pool, SETTINGS_READ).await;
    let subjects = calls(&pool, SUBJECT_READ).await;
    testkit::statements::assert_statements_kept(&pool, evicted)
        .await
        .expect("no statement evicted while the test counted them");
    eprintln!(
        "{} first requests on {EHRS} EHRs: {settings} settings reads, {subjects} subject reads",
        answered.len()
    );
    assert!(
        settings <= i64::try_from(EHRS).expect("a count"),
        "at most one settings read per EHR: {settings}"
    );
    assert_eq!(
        subjects, 0,
        "each record names the subject its request read in the store"
    );
}

/// The patients the access trail names for the records of `ehr`, oldest
/// first, polled until `expected` have landed.
async fn patients(pool: &PgPool, ehr: &str, expected: usize) -> Vec<Option<String>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let rows: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT patient_id FROM audit.access_trail WHERE ehr_id = $1 ORDER BY chain_seq",
        )
        .bind(ehr)
        .fetch_all(pool)
        .await
        .expect("the trail reads");
        if rows.len() >= expected {
            return rows;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {} of {expected} records landed",
            rows.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Moves the subject of `ehr` to `subject` in the row, as the database owner,
/// as a committed status leaves it, with no notification sent.
async fn subject_moves(pool: &PgPool, ehr: &str, subject: &str) {
    sqlx::query(
        "UPDATE clinical.ehr SET subject_id = $1, subject_namespace = 'demographic' \
         WHERE id = $2::uuid",
    )
    .bind(subject)
    .bind(ehr)
    .execute(pool)
    .await
    .expect("the subject moves");
}

/// A read's access record names the subject its own read found in the
/// store: after the subject moves with no notification, the replica's copy
/// still holds the old one, and no record of a later read names it (#5589).
#[tokio::test]
async fn a_read_names_the_subject_in_the_store_after_it_moves() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let warm = replica(&db, 4).await;
    let (status, body) =
        common::send_body(&warm, request("POST", &format!("{BASE}/ehr"), "root", None)).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let ehr = serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let first = uuid::Uuid::now_v7().to_string();
    let moved = uuid::Uuid::now_v7().to_string();
    subject_moves(&pool, &ehr, &first).await;

    let app = replica(&db, 2).await;
    let read = || request("GET", &format!("{BASE}/ehr/{ehr}"), "nurse", None);
    let (status, body) = common::send_body(&app, read()).await;
    assert_eq!(status, StatusCode::OK, "the first read: {body}");
    subject_moves(&pool, &ehr, &moved).await;
    let (stale, _) = common::send_body(&app, read()).await;
    let (status, body) = common::send_body(&app, read()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the read judged on the moved subject: {body}"
    );

    let named = patients(&pool, &ehr, 3).await;
    let named: Vec<&str> = named
        .iter()
        .rev()
        .take(3)
        .rev()
        .map(|p| p.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(
        named,
        [first.as_str(), moved.as_str(), moved.as_str()],
        "each read names the subject its read found (the second answered {stale})"
    );
}
