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
/// position. The statement reaches `PREPARE` as a bound value.
async fn read(conn: &mut PgConnection) -> (String, u64, Vec<(bool, i64)>) {
    sqlx::query("SELECT set_config('ferroehr.plan_test', $1, false)")
        .bind(next_statement())
        .execute(&mut *conn)
        .await
        .expect("the read is handed over");
    sqlx::raw_sql(
        "DO $$ BEGIN EXECUTE 'PREPARE next_owed (text) AS ' \
         || current_setting('ferroehr.plan_test'); END $$",
    )
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
    let seed: [(&str, &[i64]); 6] = [
        (
            "INSERT INTO audit.audit_render_policy (digest, policy) \
             VALUES (sha256(convert_to('{}', 'UTF8')), '{}')",
            &[],
        ),
        (
            "WITH own_xid AS MATERIALIZED (SELECT pg_catalog.pg_current_xact_id() AS taken) \
             INSERT INTO audit.audit_event (id, recorded_at, stored_at, action, outcome, \
             event_code, operation, principal, patient_id, resource_class, resource_id, \
             client_ip, request_id, render_policy, search_tokens, content_digest) \
             SELECT gen_random_uuid(), now() - ($1 - g) * interval '1 second', now(), 'R', 0, \
             '110110', 'composition_get', 'user-' || g % 1000, 'patient-' || g % 50000, \
             'composition', md5(g::text) || '::ferroehr::1', '10.0.0.' || g % 250, \
             md5((g * 7)::text), sha256(convert_to('{}', 'UTF8')), ARRAY[1, 2, 3, g % 100], \
             sha256(int8send(g)) FROM own_xid CROSS JOIN generate_series(1, $1) AS g",
            &[RECORDS],
        ),
        (
            "INSERT INTO audit.audit_event_hold (seq, reserved_by) \
             SELECT seq, gen_random_uuid() FROM audit.audit_event WHERE seq > $1",
            &[RECORDS - HELD],
        ),
        (
            "INSERT INTO audit.delivery_cursor (sink, chain, through_id) \
             VALUES ('syslog', 'system', $1), ('syslog', 'access_log', 0)",
            &[CURSOR],
        ),
        (
            "UPDATE audit.delivery_frontier SET through_id = $1 WHERE chain = 'system'",
            &[FRONTIER],
        ),
        (
            "INSERT INTO audit.delivery_parked (sink, chain, record_id, reason) \
             SELECT 'syslog', 'system', g, 'refused' FROM generate_series($1, $2) AS g",
            &[CURSOR + 1, CURSOR + PARKED],
        ),
    ];
    for (statement, values) in seed {
        let mut query = sqlx::query(statement);
        for value in values {
            query = query.bind(*value);
        }
        query.execute(&mut *conn).await.expect("the log is written");
    }
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

    sqlx::query("UPDATE audit.delivery_parked SET retry = true WHERE record_id > $1")
        .bind(CURSOR + PARKED - RETRIED)
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

    sqlx::query(
        "UPDATE audit.delivery_cursor SET through_id = $1 \
         WHERE sink = 'syslog' AND chain = 'system'",
    )
    .bind(FRONTIER)
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

/// The access-log entries the log holds, the last of them past the search
/// index's cursor.
const ENTRIES: i64 = 200_000;
const INDEXED: i64 = 198_000;

/// Writes an access log of [`ENTRIES`] entries, those up to [`INDEXED`] in
/// the search index, and a syslog sink owed all of it.
async fn seed_access_log(conn: &mut PgConnection) {
    let seed: [(&str, &[i64]); 6] = [
        (
            "INSERT INTO audit.audit_render_policy (digest, policy) \
             VALUES (sha256(convert_to('{}', 'UTF8')), '{}')",
            &[],
        ),
        (
            "WITH own_xid AS MATERIALIZED (SELECT pg_catalog.pg_current_xact_id() AS taken) \
             INSERT INTO audit.access_log_entry (id, record_type, uid, time_recorded, content, \
             content_digest, render_policy_id, event_action, event_outcome, principal, \
             patient_id) OVERRIDING SYSTEM VALUE \
             SELECT g, 'ACCESS_LOG_ENTRY', gen_random_uuid(), \
             now() - ($1 - g) * interval '1 second', '{}', sha256(int8send(g)), \
             (SELECT id FROM audit.audit_render_policy), 'R', 0, 'user-' || g % 50, \
             'patient-' || g % 500 FROM own_xid CROSS JOIN generate_series(1, $1) AS g",
            &[ENTRIES],
        ),
        (
            "INSERT INTO audit.access_log_index (entry_id, time_recorded, render_policy_id, \
             principal, patient_id, search_tokens, search_keys) \
             SELECT id, time_recorded, render_policy_id, principal, patient_id, '{}', '{}' \
             FROM audit.access_log_entry WHERE id <= $1",
            &[INDEXED],
        ),
        (
            "UPDATE audit.access_log_index_state SET last_id = $1",
            &[INDEXED],
        ),
        (
            "INSERT INTO audit.delivery_cursor (sink, chain, through_id) \
             VALUES ('syslog', 'system', 0), ('syslog', 'access_log', 0)",
            &[],
        ),
        (
            "UPDATE audit.delivery_frontier SET through_id = $1 WHERE chain = 'access_log'",
            &[ENTRIES],
        ),
    ];
    for (statement, values) in seed {
        let mut query = sqlx::query(statement);
        for value in values {
            query = query.bind(*value);
        }
        query.execute(&mut *conn).await.expect("the log is written");
    }
    sqlx::raw_sql(
        "VACUUM ANALYZE audit.access_log_entry, audit.access_log_index, \
         audit.access_log_index_state, audit.delivery_cursor, audit.delivery_frontier",
    )
    .execute(&mut *conn)
    .await
    .expect("the log is analyzed");
}

/// Over an access log of 200,000 entries, its last 2,000 past the search
/// index's cursor, a sink's next entry is read through the key range of its
/// window, with the window in the indexed entries, across the cursor and
/// past it, and the entry read is the one owed (#6385).
#[tokio::test]
async fn the_delivery_read_reads_the_access_log_window_by_its_key() {
    let db = testkit::db().await.expect("testkit database");
    let mut conn = crate::db::acquire::acquire(&db.pool())
        .await
        .expect("a connection");
    seed_access_log(&mut conn).await;
    sqlx::raw_sql("SET plan_cache_mode = force_generic_plan")
        .execute(&mut *conn)
        .await
        .expect("the generic plan is forced");
    for cursor in [0, INDEXED - 1_000, INDEXED, ENTRIES - 1] {
        sqlx::query(
            "UPDATE audit.delivery_cursor SET through_id = $1 \
             WHERE sink = 'syslog' AND chain = 'access_log'",
        )
        .bind(cursor)
        .execute(&mut *conn)
        .await
        .expect("the cursor stands");
        let (plan, pages, rows) = read(&mut conn).await;
        assert!(
            !plan.contains("Gather")
                && !plan.contains("Seq Scan on access_log_entry ")
                && !plan.contains("Seq Scan on access_log_index "),
            "cursor {cursor}: the window is read by its key:\n{plan}"
        );
        assert!(
            pages <= PAGES,
            "cursor {cursor}: {pages} pages touched, at most {PAGES}:\n{plan}"
        );
        assert_eq!(
            rows,
            [(false, cursor + 1)],
            "cursor {cursor}: the next entry"
        );
    }
}

/// Over the same access log, the person's trail reads the entries past the
/// search index's cursor as one bounded key range in one process, with no
/// sort of that window, whether the cursor is caught up or behind (#6385).
#[cfg(feature = "fhir")]
#[tokio::test]
async fn the_person_trail_reads_the_open_window_by_its_key() {
    use sea_query::PostgresQueryBuilder;

    use crate::system_log::search::condition::search_condition;
    use crate::system_log::search::resolve_criteria;
    use crate::system_log::store::{AuditCriterion, AuditSearchFilter, newest_first};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let mut conn = crate::db::acquire::acquire(&pool)
        .await
        .expect("a connection");
    seed_access_log(&mut conn).await;
    let filter = AuditSearchFilter {
        criteria: vec![AuditCriterion::Patient(vec!["patient-7".to_owned()])],
        ..AuditSearchFilter::default()
    };
    let resolved = resolve_criteria(&pool, &filter)
        .await
        .expect("the criteria resolve")
        .expect("no token criterion");
    let trail = newest_first(search_condition(&filter, &resolved)).to_string(PostgresQueryBuilder);
    sqlx::query("SELECT set_config('ferroehr.plan_test', $1, false)")
        .bind(trail)
        .execute(&mut *conn)
        .await
        .expect("the trail is handed over");
    sqlx::raw_sql(
        "DO $$ BEGIN EXECUTE 'PREPARE trail AS ' || current_setting('ferroehr.plan_test'); END $$",
    )
    .execute(&mut *conn)
    .await
    .expect("the trail prepares");
    for cursor in [INDEXED, ENTRIES] {
        sqlx::query("UPDATE audit.access_log_index_state SET last_id = $1")
            .bind(cursor)
            .execute(&mut *conn)
            .await
            .expect("the cursor stands");
        sqlx::query(
            "INSERT INTO audit.access_log_index (entry_id, time_recorded, render_policy_id, \
             principal, patient_id, search_tokens, search_keys) \
             SELECT id, time_recorded, render_policy_id, principal, patient_id, '{}', '{}' \
             FROM audit.access_log_entry WHERE id <= $1 ON CONFLICT DO NOTHING",
        )
        .bind(cursor)
        .execute(&mut *conn)
        .await
        .expect("the index holds the entries up to the cursor");
        let plan: Vec<String> = sqlx::query_scalar(
            "EXPLAIN (ANALYZE, BUFFERS, COSTS OFF, TIMING OFF, SUMMARY OFF) EXECUTE trail",
        )
        .fetch_all(&mut *conn)
        .await
        .expect("the plan reads");
        let plan = plan.join("\n");
        let lines: Vec<&str> = plan.lines().collect();
        let window = lines.windows(2).any(|pair| {
            pair[0].contains("->  Index Scan using pk_access_log_entry on access_log_entry")
                && pair[1].contains("(id > (InitPlan")
                && pair[1].contains("(id <= (InitPlan")
        });
        assert!(
            window && plan.matches("Sort Key").count() == 1,
            "cursor {cursor}: the open window is one bounded key range, sorted only as the \
             trail:\n{plan}"
        );
        let rows = plan
            .lines()
            .next()
            .and_then(|top| top.split("rows=").nth(1))
            .and_then(|rows| rows.split_whitespace().next())
            .unwrap_or_default()
            .to_owned();
        assert_eq!(
            rows, "400.00",
            "cursor {cursor}: the patient's 400 entries:\n{plan}"
        );
    }
}
