// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-81 retrieval reads each chain's first records rather than the
//! whole log (#5646): for every filter, page size and offset it serves the
//! same total and the same page, record for record, as the newest-first read
//! of the union view it replaced, over a log of both chains whose records tie
//! on `recorded_at` within and across the chains.
//!
//! No openEHR spec governs audit storage: our own design. The search follows
//! IHE `RESTful` ATNA (ITI-81).

use jiff::Timestamp;

use ferroehr::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};
use ferroehr::system_log::store::chains::AuditChain;
use ferroehr::system_log::store::{AuditCriterion, AuditRecord, AuditSearchFilter, AuditStore};

/// The instants the records are written at: few, so most records tie.
const INSTANTS: [&str; 5] = [
    "2026-03-01T08:00:00Z",
    "2026-03-01T08:00:01Z",
    "2026-03-02T09:30:00Z",
    "2026-03-03T10:00:00Z",
    "2026-03-04T11:15:00Z",
];

/// The principals the records name.
const PRINCIPALS: [&str; 3] = ["alice", "bob", "carol"];

/// Record `n`: an access to an EHR, which the store keeps as an access-log
/// entry, on every other one, and a record of the system chain otherwise.
fn event(n: usize) -> AuditEvent {
    let class = if n.is_multiple_of(2) {
        ObjectClass::Ehr
    } else {
        ObjectClass::Demographic
    };
    let action = if n.is_multiple_of(3) {
        EventActionCode::Create
    } else {
        EventActionCode::Read
    };
    let mut e = AuditEvent::new(action, class, EventOutcome::Success);
    PRINCIPALS[n % PRINCIPALS.len()].clone_into(&mut e.user_id);
    e.timestamp = INSTANTS[(n * 7) % INSTANTS.len()]
        .parse()
        .expect("an instant");
    if n.is_multiple_of(2) {
        e.domain = AccessDomain::Ehr;
        e.ehr_id = Some(format!("7d44b88c-4199-4bad-97dc-{n:012x}"));
    }
    e
}

/// The records of batch `batch`, ten of them.
fn batch(batch: usize) -> Vec<AuditRecord> {
    (batch * 10..batch * 10 + 10)
        .map(|n| AuditRecord {
            event: event(n),
            subject: Some(format!("patient-{}", n % 4)),
            entry: None,
        })
        .collect()
}

/// One filter of the retrieval and the same filter as the `WHERE` of the
/// newest-first read it replaced.
struct Case {
    name: &'static str,
    filter: AuditSearchFilter,
    sql: &'static str,
}

fn cases() -> Vec<Case> {
    let from: Timestamp = "2026-03-01T08:00:01Z".parse().expect("an instant");
    let until: Timestamp = "2026-03-03T10:00:00Z".parse().expect("an instant");
    vec![
        Case {
            name: "every record",
            filter: AuditSearchFilter::default(),
            sql: "true",
        },
        Case {
            name: "a date from",
            filter: AuditSearchFilter {
                from: Some(from),
                ..AuditSearchFilter::default()
            },
            sql: "recorded_at >= '2026-03-01T08:00:01Z'",
        },
        Case {
            name: "a date window",
            filter: AuditSearchFilter {
                from: Some(from),
                until: Some(until),
                ..AuditSearchFilter::default()
            },
            sql: "recorded_at >= '2026-03-01T08:00:01Z' AND recorded_at < '2026-03-03T10:00:00Z'",
        },
        Case {
            name: "an agent",
            filter: AuditSearchFilter {
                criteria: vec![AuditCriterion::Agent(vec!["bob".to_owned()])],
                ..AuditSearchFilter::default()
            },
            sql: "principal = 'bob'",
        },
        Case {
            name: "two patients and an action",
            filter: AuditSearchFilter {
                criteria: vec![
                    AuditCriterion::Patient(vec!["patient-0".to_owned(), "patient-2".to_owned()]),
                    AuditCriterion::Action(vec!["R".to_owned()]),
                ],
                ..AuditSearchFilter::default()
            },
            sql: "patient_id IN ('patient-0', 'patient-2') AND action = 'R'",
        },
        Case {
            name: "an outcome and a date from",
            filter: AuditSearchFilter {
                from: Some(from),
                criteria: vec![AuditCriterion::Outcome(vec![0])],
                ..AuditSearchFilter::default()
            },
            sql: "recorded_at >= '2026-03-01T08:00:01Z' AND outcome = 0",
        },
        Case {
            name: "no match",
            filter: AuditSearchFilter {
                criteria: vec![AuditCriterion::Agent(vec!["nobody".to_owned()])],
                ..AuditSearchFilter::default()
            },
            sql: "principal = 'nobody'",
        },
    ]
}

/// The total and the positions of the page `where` selects, read as the
/// retrieval read them before #5646: the union view counted, and ordered
/// newest first with the page's limit and offset.
async fn reference(
    pool: &sqlx::PgPool,
    r#where: &str,
    count: i64,
    offset: i64,
) -> (i64, Vec<(String, i64)>) {
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM audit.audit_record WHERE {where}"
    )))
    .fetch_one(pool)
    .await
    .expect("the reference total");
    let page: Vec<(String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT chain, chain_seq FROM audit.audit_record WHERE {where} \
         ORDER BY recorded_at DESC, stored_at DESC, chain DESC, chain_seq DESC \
         LIMIT {count} OFFSET {offset}"
    )))
    .fetch_all(pool)
    .await
    .expect("the reference page");
    (total, page)
}

#[tokio::test]
async fn every_page_and_total_matches_the_read_of_the_union_view() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());
    for n in 0..12 {
        store
            .insert_batch(&batch(n), &[])
            .await
            .expect("the batch is stored");
    }
    let chains: Vec<(String, i64)> = sqlx::query_as(
        "SELECT chain, count(DISTINCT recorded_at) FROM audit.audit_record GROUP BY chain ORDER BY chain",
    )
    .fetch_all(&pool)
    .await
    .expect("the chains");
    assert_eq!(
        chains,
        [("access_log".to_owned(), 5), ("system".to_owned(), 5)],
        "both chains hold records, tied on five instants"
    );

    for case in cases() {
        for count in [1, 3, 7, 20, 120] {
            for offset in [0, 1, 4, 13, 59, 60, 61, 119, 500] {
                let filter = AuditSearchFilter {
                    count,
                    offset,
                    ..case.filter.clone()
                };
                let (total, documents) = store.search(&filter).await.expect("the search");
                let (want_total, want_page) = reference(&pool, case.sql, count, offset).await;
                let what = format!("{}, _count={count}, _offset={offset}", case.name);
                assert_eq!(total, want_total, "{what}: the total");
                let mut want = Vec::with_capacity(want_page.len());
                for (chain, seq) in &want_page {
                    let chain = AuditChain::named(chain).expect("a chain");
                    want.push(
                        store
                            .rendered(chain, *seq)
                            .await
                            .expect("the record renders")
                            .expect("the record exists"),
                    );
                }
                assert_eq!(documents, want, "{what}: the page");
            }
        }
    }
}
