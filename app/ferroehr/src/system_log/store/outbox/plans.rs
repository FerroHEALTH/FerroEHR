// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The delivery read reads a sink's window of the system log by its index,
//! however large the log (#6427). The audit runtime pool plans every
//! statement generically (#6392), so the generic plan is the one checked. No
//! openEHR spec governs the planning of a statement: our own design.

use sqlx::{PgConnection, Row};

use super::next_statement;

/// The system records the log holds.
const RECORDS: i64 = 200_000;

/// The sink's cursor and the delivery frontier: a window of 5,000 records.
const CURSOR: i64 = 190_000;
const FRONTIER: i64 = 195_000;

/// The records past the cursor the sink parked, the last of them an operator
/// asked to retry, and the records past the frontier a write holds.
const PARKED: i64 = 500;
const RETRIED: i64 = 20;
const HELD: i64 = 1_000;

/// The most pages one read may touch: the window's and the retried records',
/// never the log's.
const PAGES: u64 = 400;

/// The generic plan of the delivery read for the syslog sink, the pages it
/// touched, and the records it reads, each as whether retried and its
/// position.
async fn read(conn: &mut PgConnection) -> (String, u64, Vec<(bool, i64)>) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "PREPARE next_owed (text) AS {}",
        next_statement()
    )))
    .execute(&mut *conn)
    .await
    .expect("the read prepares");
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN (ANALYZE, BUFFERS, COSTS OFF, TIMING OFF, SUMMARY OFF) \
         EXECUTE next_owed ('syslog')",
    )
    .fetch_all(&mut *conn)
    .await
    .expect("the plan reads");
    sqlx::raw_sql("DEALLOCATE next_owed")
        .execute(&mut *conn)
        .await
        .expect("the read is deallocated");
    let rows = sqlx::query(sqlx::AssertSqlSafe(next_statement()))
        .bind("syslog")
        .fetch_all(&mut *conn)
        .await
        .expect("the read answers")
        .iter()
        .map(|row| (row.get::<bool, _>("retry"), row.get::<i64, _>("chain_seq")))
        .collect();
    let pages = plan
        .iter()
        .find_map(|line| line.trim().strip_prefix("Buffers: shared "))
        .map_or(0, |buffers| {
            buffers
                .split_whitespace()
                .filter_map(|part| part.split_once('='))
                .filter(|(kind, _)| matches!(*kind, "hit" | "read"))
                .map(|(_, count)| count.parse::<u64>().expect("a page count"))
                .sum()
        });
    (plan.join("\n"), pages, rows)
}

/// The plan reads the system log by its primary key and touches the
/// window's pages only.
fn assert_bounded(plan: &str, pages: u64, case: &str) {
    assert!(
        plan.contains("Scan using pk_audit_event on audit_event")
            && !plan.contains("Seq Scan on audit_event "),
        "{case}: the system log is read by its index:\n{plan}"
    );
    assert!(
        pages <= PAGES,
        "{case}: {pages} pages touched, at most {PAGES}:\n{plan}"
    );
}

/// Over a log of 200,000 system records, a sink's next record, a retried
/// parked record, and a sink at the frontier are each read through the
/// index range of its window, and the records read are the ones owed.
#[tokio::test]
async fn the_delivery_read_reads_the_window_by_its_index() {
    let db = testkit::db().await.expect("testkit database");
    let mut conn = crate::db::acquire::acquire(&db.pool())
        .await
        .expect("a connection");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "INSERT INTO audit.audit_render_policy (digest, policy) \
         VALUES (sha256(convert_to('{{}}', 'UTF8')), '{{}}'); \
         INSERT INTO audit.audit_event (id, recorded_at, stored_at, action, outcome, event_code, \
         operation, principal, patient_id, resource_class, resource_id, client_ip, request_id, \
         render_policy, search_tokens, content_digest) \
         SELECT gen_random_uuid(), now() - ({RECORDS} - g) * interval '1 second', now(), 'R', 0, \
         '110110', 'composition_get', 'user-' || g % 1000, 'patient-' || g % 50000, \
         'composition', md5(g::text) || '::ferroehr::1', '10.0.0.' || g % 250, \
         md5((g * 7)::text), sha256(convert_to('{{}}', 'UTF8')), ARRAY[1, 2, 3, g % 100], \
         sha256(int8send(g)) FROM generate_series(1, {RECORDS}) AS g; \
         INSERT INTO audit.audit_event_hold (seq, reserved_by) \
         SELECT seq, gen_random_uuid() FROM audit.audit_event WHERE seq > {RECORDS} - {HELD}; \
         INSERT INTO audit.delivery_cursor (sink, chain, through_id) \
         VALUES ('syslog', 'system', {CURSOR}), ('syslog', 'access_log', 0); \
         UPDATE audit.delivery_frontier SET through_id = {FRONTIER} WHERE chain = 'system'; \
         INSERT INTO audit.delivery_parked (sink, chain, record_id, reason) \
         SELECT 'syslog', 'system', g, 'refused' \
         FROM generate_series({CURSOR} + 1, {CURSOR} + {PARKED}) AS g"
    )))
    .execute(&mut *conn)
    .await
    .expect("the log is written");
    sqlx::raw_sql(
        "VACUUM ANALYZE audit.audit_event, audit.audit_event_hold, audit.delivery_parked, \
         audit.delivery_cursor, audit.delivery_frontier",
    )
    .execute(&mut *conn)
    .await
    .expect("the log is analyzed");
    sqlx::raw_sql("SET plan_cache_mode = force_generic_plan")
        .execute(&mut *conn)
        .await
        .expect("the generic plan is forced");

    let (plan, pages, rows) = read(&mut conn).await;
    assert_bounded(&plan, pages, "the next record");
    assert_eq!(
        rows,
        [(false, CURSOR + PARKED + 1)],
        "the first record not parked"
    );

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "UPDATE audit.delivery_parked SET retry = true \
         WHERE record_id > {CURSOR} + {PARKED} - {RETRIED}"
    )))
    .execute(&mut *conn)
    .await
    .expect("the parked records are retried");
    let (plan, pages, rows) = read(&mut conn).await;
    assert_bounded(&plan, pages, "a retried record");
    assert_eq!(
        rows,
        [
            (true, CURSOR + PARKED - RETRIED + 1),
            (false, CURSOR + PARKED + 1)
        ],
        "the oldest retried record, then the first record not parked"
    );

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "UPDATE audit.delivery_cursor SET through_id = {FRONTIER} \
         WHERE sink = 'syslog' AND chain = 'system'"
    )))
    .execute(&mut *conn)
    .await
    .expect("the cursor reaches the frontier");
    let (plan, pages, rows) = read(&mut conn).await;
    assert_bounded(&plan, pages, "a sink at the frontier");
    assert_eq!(
        rows,
        [(true, CURSOR + PARKED - RETRIED + 1)],
        "only the retried record"
    );
}
