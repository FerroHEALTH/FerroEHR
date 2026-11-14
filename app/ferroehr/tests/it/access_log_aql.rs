// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL over the access log (#5131) against a real `PostgreSQL` 18:
//! `ACCESS_LOG_ENTRY` as a top-level class, read by a reviewer across EHRs
//! and by the person in the subject view of their own EHR, refused to anyone
//! else and inside an `EHR` containment, paged as any query is, and served by
//! the `(ehr_id, time_recorded)` index for the person's lookup by time.
//!
//! No openEHR spec governs AQL over the access log: our own design.

use serde_json::{Value, json};
use sqlx::{Connection, Executor as _};

use ferroehr::access_log::store::AccessLogStore;
use ferroehr::access_log::view::{LogReader, LogStanding};
use ferroehr::aql::access_log::sql::{LogScope, statement};
use ferroehr::aql::ir::{ParamValue, Params};
use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::system_log::store::AuditStore;
use openehr_query::parser::parse_str;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

use crate::access_log_store::example;
use crate::fixtures::create_ehr;

/// The service over a fresh database with its audit store on, two EHRs, and
/// three entries in the log of the first and two in the log of the second.
struct Logged {
    db: testkit::TestDb,
    svc: FerroEhrService,
    first: EhrId,
    second: EhrId,
}

/// The proposal's example entry, number `n`, about `ehr`, recorded at minute
/// `n` of the day.
fn entry(n: u32, ehr: EhrId) -> AccessLogEntry {
    let mut value = example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["ehr_id"]["value"] = json!(ehr.to_string());
    value["time_recorded"]["value"] = json!(format!("2026-11-07T09:{n:02}:00Z"));
    serde_json::from_value(value).expect("the example entry reads")
}

async fn logged() -> Logged {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit_store(AuditStore::new(pool.clone()));
    let first = create_ehr(&svc).await;
    let second = create_ehr(&svc).await;
    let entries = vec![
        entry(1, first),
        entry(2, second),
        entry(3, first),
        entry(4, second),
        entry(5, first),
    ];
    AccessLogStore::new(pool.clone())
        .append(entries)
        .await
        .expect("the entries append");
    Logged {
        db,
        svc,
        first,
        second,
    }
}

/// A request for `standing`, with `fetch` and `offset`.
fn request(standing: LogStanding) -> AqlQueryRequest {
    AqlQueryRequest {
        log_standing: standing,
        ..AqlQueryRequest::default()
    }
}

/// The rows of the query `aql` for `request`.
async fn rows(svc: &FerroEhrService, aql: &str, request: &AqlQueryRequest) -> Vec<Value> {
    let outcome = svc
        .execute_aql(aql, None, request)
        .await
        .unwrap_or_else(|error| panic!("{aql}: {error}"));
    let rows = outcome.result_set["rows"]
        .as_array()
        .cloned()
        .expect("rows");
    assert_eq!(
        outcome.served_rows,
        u64::try_from(rows.len()).expect("a count"),
        "the served rows are counted where they are"
    );
    assert!(outcome.log_read.is_some(), "the query read the log");
    rows
}

/// A reviewer reads every EHR's entries in the full view, newest first by
/// default, narrowed by a path predicate and paged by `fetch` and `offset`.
#[tokio::test]
async fn a_reviewer_queries_across_ehrs() {
    let fx = logged().await;
    let reviewer = request(LogStanding::Reviewer);
    let all = rows(
        &fx.svc,
        "SELECT l/ehr_id/value, l/link/sequence, l/accessor/credential/subject \
         FROM ACCESS_LOG_ENTRY l",
        &reviewer,
    )
    .await;
    assert_eq!(all.len(), 5, "{all:?}");
    let ehrs: Vec<&Value> = all.iter().map(|row| &row[0]).collect();
    assert!(ehrs.contains(&&json!(fx.first.to_string())), "{all:?}");
    assert!(ehrs.contains(&&json!(fx.second.to_string())), "{all:?}");
    assert_eq!(all[0][1], json!(5), "newest first: {all:?}");
    assert_eq!(all[0][2], json!("a1b2c3"), "the full view: {all:?}");

    let second = rows(
        &fx.svc,
        "SELECT l/uid/value FROM ACCESS_LOG_ENTRY l WHERE l/ehr_id/value = $ehr \
         ORDER BY l/time_recorded/value ASC",
        &AqlQueryRequest {
            parameters: [("ehr".to_owned(), json!(fx.second.to_string()))].into(),
            ..request(LogStanding::Reviewer)
        },
    )
    .await;
    assert_eq!(
        second,
        vec![
            json!(["0193f1a2-7c1e-7b4a-9f00-000000000002"]),
            json!(["0193f1a2-7c1e-7b4a-9f00-000000000004"]),
        ]
    );

    let page = rows(
        &fx.svc,
        "SELECT l/link/sequence FROM ACCESS_LOG_ENTRY l ORDER BY l/link/sequence ASC",
        &AqlQueryRequest {
            fetch: Some(2),
            offset: Some(1),
            ..request(LogStanding::Reviewer)
        },
    )
    .await;
    assert_eq!(page, vec![json!([2]), json!([3])]);

    let versions = rows(
        &fx.svc,
        "SELECT l/items/version_id/value FROM ACCESS_LOG_ENTRY l \
         WHERE l/items/rm_type = 'COMPOSITION' AND l/is_emergency = false LIMIT 1",
        &reviewer,
    )
    .await;
    assert_eq!(
        versions,
        vec![json!([[
            "8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1"
        ]])]
    );
}

/// The person reads only the entries of their own EHR, in the subject view: a
/// withheld path reads as absent in a column and in a condition.
#[tokio::test]
async fn the_person_queries_their_own_redacted_entries() {
    let fx = logged().await;
    let person = request(LogStanding::Person(fx.first));
    let own = rows(
        &fx.svc,
        "SELECT l, l/ehr_id/value, l/accessor/credential/subject, l/channel \
         FROM ACCESS_LOG_ENTRY l",
        &person,
    )
    .await;
    assert_eq!(own.len(), 3, "{own:?}");
    for row in &own {
        assert_eq!(row[1], json!(fx.first.to_string()), "{row}");
        assert_eq!(row[2], Value::Null, "the credential is withheld: {row}");
        assert_eq!(row[3], Value::Null, "the channel is withheld: {row}");
        assert!(row[0].get("link").is_none(), "the subject view: {row}");
        assert!(row[0]["accessor"].get("credential").is_none(), "{row}");
        assert!(row[0].get("time_recorded").is_some(), "{row}");
    }
    let probed = rows(
        &fx.svc,
        "SELECT l/uid/value FROM ACCESS_LOG_ENTRY l \
         WHERE l/accessor/credential/subject = 'a1b2c3' OR EXISTS l/channel",
        &person,
    )
    .await;
    assert!(
        probed.is_empty(),
        "a withheld path tests nothing: {probed:?}"
    );

    let elsewhere = fx
        .svc
        .execute_aql(
            "SELECT l FROM ACCESS_LOG_ENTRY l",
            None,
            &AqlQueryRequest {
                ehr_ids: vec![fx.second],
                ..request(LogStanding::Person(fx.first))
            },
        )
        .await
        .expect_err("another EHR's log");
    assert!(
        matches!(elsewhere, ServiceError::NotFound(_)),
        "{elsewhere}"
    );
}

/// A caller who is neither the person nor a reviewer is refused, and a query
/// that puts the log inside an EHR containment is refused for everyone.
#[tokio::test]
async fn a_refused_caller_and_a_containment_are_refused() {
    let fx = logged().await;
    let refused = fx
        .svc
        .execute_aql(
            "SELECT l FROM ACCESS_LOG_ENTRY l",
            None,
            &request(LogStanding::Refused),
        )
        .await
        .expect_err("refused");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused}");

    for aql in [
        "SELECT l FROM EHR e CONTAINS ACCESS_LOG_ENTRY l",
        "SELECT l FROM ACCESS_LOG_ENTRY l CONTAINS EHR e",
    ] {
        let error = fx
            .svc
            .execute_aql(aql, None, &request(LogStanding::Reviewer))
            .await
            .expect_err(aql);
        assert!(
            matches!(error, ServiceError::BadRequest(_)),
            "{aql}: {error}"
        );
        assert!(
            error
                .to_string()
                .contains("never appears inside an EHR containment"),
            "{aql}: {error}"
        );
    }
}

/// The person's lookup of their entries by time reads the `(ehr_id,
/// time_recorded)` index, in the order the index keeps.
#[tokio::test]
async fn the_persons_lookup_by_time_reads_the_ehr_index() {
    let fx = logged().await;
    let query = parse_str(
        "SELECT l FROM ACCESS_LOG_ENTRY l WHERE l/time_recorded/value >= $from \
         AND l/time_recorded/value < '2026-11-08T00:00:00Z'",
    )
    .expect("parses");
    let plan = ferroehr::aql::access_log::plan(&query)
        .expect("plans")
        .expect("a log query");
    let params = Params::default().with("from", ParamValue::Str("2026-11-07T00:00:00Z".to_owned()));
    let ehrs = [fx.first];
    let scope = LogScope {
        reader: LogReader::Person,
        ehrs: &ehrs,
        limit: Some(100),
        offset: None,
    };
    let (sql, values) = statement(&plan, &params, &scope).expect("builds");

    let pool = fx.db.pool();
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    tx.execute("SET LOCAL enable_seqscan = off")
        .await
        .expect("discourage sequential scans");
    let explained: Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("EXPLAIN (FORMAT JSON) {sql}")),
        values,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("explain");
    tx.rollback().await.expect("rollback");

    let mut scans = Vec::new();
    collect_scans(&explained[0]["Plan"], &mut scans);
    assert!(
        scans.iter().any(|(node, index)| node.contains("Index")
            && index.as_deref() == Some("ix_access_log_record_ehr")),
        "the lookup reads ix_access_log_record_ehr: {scans:?}\n{explained:#}"
    );
    assert!(
        scans.iter().all(|(node, _)| node != "Seq Scan"),
        "no sequential scan: {scans:?}"
    );
}

/// Every scan node of `plan` with the index it reads.
fn collect_scans(plan: &Value, into: &mut Vec<(String, Option<String>)>) {
    if let Some(node) = plan["Node Type"].as_str()
        && node.contains("Scan")
    {
        into.push((
            node.to_owned(),
            plan["Index Name"].as_str().map(str::to_owned),
        ));
    }
    for child in plan["Plans"].as_array().into_iter().flatten() {
        collect_scans(child, into);
    }
}
