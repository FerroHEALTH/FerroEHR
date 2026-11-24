// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The budget of an `EHR_ACCESS` settings write (#6222): the round trips the
//! commit sends, the statements `PostgreSQL` runs nested in them, the
//! statements that find or store the settings class, and the write-ahead log
//! the commit writes, for a write whose settings make a class no EHR made
//! before and for one whose class is stored.
//!
//! The class is found or stored by one `INSERT ... ON CONFLICT DO SELECT`
//! (PostgreSQL 19, `INSERT`), where a first write took a read and an insert.
//! The WAL figure is what `statement_budget::wal` counts, with the chain heads
//! left out. No openEHR spec governs the cost of a write: our own design.

use std::fmt::Write as _;
use std::sync::atomic::Ordering;

use serde_json::Value;
use sqlx::PgPool;
use tracing::Instrument as _;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{CHAINER_RELEASE, REQUEST_SPAN, RoundTrips, calls, counted, drained};
use crate::access_evaluator_http::ehr_with_composition;
use crate::common;

/// The writes of each kind, each on an EHR of its own.
const WRITES: u64 = 5;

/// The ceilings of one kind of write, per write.
struct Budget {
    name: &'static str,
    round_trips: u64,
    nested: u64,
    class_statements: u64,
    wal: u64,
}

/// The ceilings: the figures measured on PostgreSQL 19.0 over two runs, with
/// 3% on the WAL bytes and none on the counts. A write of a new class sent 16
/// round trips, two of them for its class, before the class took one
/// statement; the WAL bytes did not move (12,073 to 12,075 and 9,791 to 9,797
/// per write before and after). The access-log write taking no row lock on a
/// shared part (#6241) wrote 108 and 111 bytes fewer per write, measured
/// beside the merge base, and each WAL ceiling fell by as much. The entry
/// written once by a plain insert (#6242) wrote 1,121 and 1,118 bytes fewer
/// and ran 4 and 5 nested statements fewer per write, measured beside the
/// merge base, and each ceiling fell by as much. The entry's search columns
/// copied off the request's commit (#6319) wrote 747 and 750 bytes fewer per
/// write, measured beside the merge base, and each WAL ceiling fell by as
/// much.
const BUDGETS: [Budget; 2] = [
    Budget {
        name: "a new class",
        round_trips: 15,
        nested: 5,
        class_statements: 1,
        wal: 9986,
    },
    Budget {
        name: "a stored class",
        round_trips: 15,
        nested: 1,
        class_statements: 1,
        wal: 7551,
    },
];

/// What the writes of one kind measured, per write.
#[derive(Debug, Default)]
struct Measured {
    round_trips: u64,
    nested: u64,
    class_statements: u64,
    wal: u64,
    ledger: String,
}

/// The settings of the `k`th class: `k` restrictions of objects the EHR does
/// not hold, so each `k` makes a class of its own.
fn settings(k: usize) -> Value {
    let objects: Vec<String> = (0..k).map(|_| uuid::Uuid::now_v7().to_string()).collect();
    let directives: Vec<Value> = objects
        .iter()
        .map(|object| common::access_directive("restriction", &[object.as_str()]))
        .collect();
    common::directive_settings(&directives)
}

/// Whether `relation` is a seal or owing state, which a chainer pass writes
/// once whatever it seals.
fn head_relation(relation: &str) -> bool {
    super::wal::pass_relation(relation)
}

/// Waits until the frontier round indexed every access-log entry with its
/// search tokens, which it stores apart from the writes (#6322).
async fn indexed(pool: &PgPool) {
    for _ in 0..800 {
        let behind: bool = sqlx::query_scalar(
            "SELECT /* statement_budget_own */ EXISTS (SELECT 1 FROM audit.access_log_entry e \
              WHERE e.id > (SELECT x.last_id FROM audit.access_log_index_state x \
                             WHERE x.singleton))",
        )
        .fetch_one(pool)
        .await
        .expect("the index cursor reads");
        if !behind {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the frontier round did not index the entries");
}

/// Commits the settings of class `k + 1` on the `k`th EHR of `ehrs` and
/// measures the writes.
async fn measure(
    pool: &PgPool,
    svc: &ferroehr::service::FerroEhrService,
    round_trips: &RoundTrips,
    ehrs: &[ferroehr::ids::EhrId],
) -> Measured {
    drained(pool).await;
    indexed(pool).await;
    let evicted = testkit::statements::statement_window(pool)
        .await
        .expect("the statement counts reset");
    let from = super::wal::insert_lsn(pool).await;
    let sent_before = round_trips.0.load(Ordering::Relaxed);
    for (k, ehr) in ehrs.iter().enumerate() {
        common::commit_access_settings(svc, *ehr, settings(k + 1))
            .instrument(tracing::info_span!(REQUEST_SPAN))
            .await;
    }
    let sent = round_trips.0.load(Ordering::Relaxed) - sent_before;
    drained(pool).await;
    indexed(pool).await;
    let to = super::wal::insert_lsn(pool).await;
    super::wal::flushed(pool, &to).await;
    let statements = counted(pool).await;
    testkit::statements::assert_statements_kept(pool, evicted)
        .await
        .expect("no statement evicted while the test counted them");
    let nested: u64 = statements
        .iter()
        .filter(|s| !s.toplevel)
        .map(|s| s.calls)
        .sum();
    let chainer = calls(&statements, "SELECT audit.audit_intake_release") * CHAINER_RELEASE;
    let mut out = Measured {
        round_trips: sent.div_euclid(WRITES),
        nested: nested.saturating_sub(chainer).div_euclid(WRITES),
        class_statements: statements
            .iter()
            .filter(|s| s.toplevel && s.query.contains("access_class"))
            .map(|s| s.calls)
            .sum::<u64>()
            .div_euclid(WRITES),
        ..Measured::default()
    };
    let mut wal = 0;
    for (relation, bytes, records) in super::wal::by_relation(pool, &from, &to).await {
        let head = head_relation(&relation);
        if !head {
            wal += bytes;
        }
        writeln!(
            out.ledger,
            "  {bytes:>8} B {records:>5}  {relation}{}",
            if head {
                "  (a chain head, left out)"
            } else {
                ""
            }
        )
        .unwrap();
    }
    out.wal = wal.div_euclid(WRITES);
    writeln!(
        out.ledger,
        "{nested} nested, {chainer} of them the chainer's"
    )
    .unwrap();
    for s in &statements {
        let level = if s.toplevel { "top" } else { "nested" };
        let text: String = s.query.chars().take(140).collect();
        writeln!(out.ledger, "  {level:>6} {:>4}  {text}", s.calls).unwrap();
    }
    out
}

/// A settings write stays within its budget, whether its class is new or
/// stored, and finds or stores the class in one statement.
#[tokio::test]
async fn an_ehr_settings_write_stays_within_its_budget() {
    let round_trips = RoundTrips::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(round_trips.clone()));
    let (_db, pool, svc, app) = Box::pin(super::served()).await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_walinspect")
        .execute(&pool)
        .await
        .expect("pg_walinspect: a contrib module the server ships");
    super::wal::kept(&pool).await;
    let mut new = Vec::new();
    let mut stored = Vec::new();
    for _ in 0..WRITES {
        new.push(
            ehr_with_composition(&app)
                .await
                .0
                .parse()
                .expect("an EHR id"),
        );
        stored.push(
            ehr_with_composition(&app)
                .await
                .0
                .parse()
                .expect("an EHR id"),
        );
    }
    let mut report = String::new();
    let mut over = Vec::new();
    for (budget, ehrs) in BUDGETS.iter().zip([new, stored]) {
        let m = measure(&pool, &svc, &round_trips, &ehrs).await;
        for (what, measured, ceiling) in [
            ("round trips", m.round_trips, budget.round_trips),
            ("nested statements", m.nested, budget.nested),
            (
                "class statements",
                m.class_statements,
                budget.class_statements,
            ),
            ("WAL bytes", m.wal, budget.wal),
        ] {
            if measured > ceiling {
                over.push(format!("{}: {what} {measured} (<= {ceiling})", budget.name));
            }
        }
        writeln!(
            report,
            "{}: {} round trips, {} nested, {} class statements, {} WAL B per write\n{}",
            budget.name, m.round_trips, m.nested, m.class_statements, m.wal, m.ledger
        )
        .unwrap();
    }
    let classes: i64 = sqlx::query_scalar("SELECT count(*) FROM access_class")
        .fetch_one(&pool)
        .await
        .expect("the classes count");
    assert_eq!(
        classes,
        i64::try_from(WRITES).expect("a count"),
        "the first writes stored a class each and the second ones stored none"
    );
    eprintln!("{report}");
    assert!(
        over.is_empty(),
        "a settings write exceeds its budget: {over:?}\n{report}"
    );
}
