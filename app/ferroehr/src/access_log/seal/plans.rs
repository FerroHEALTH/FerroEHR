// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit runtime pool plans every statement generically (#6392), so the
//! check for a seal past a window reads the last window from the index
//! whatever window it asks about. No openEHR spec governs the planning of a
//! statement: our own design.

use sqlx::PgConnection;

use super::{ACCESS_LOG, Log, SYSTEM};

/// The seal windows each log holds.
const WINDOWS: i64 = 4000;

/// The generic plan of `log`'s check for a seal past `window`, and its answer.
async fn more(conn: &mut PgConnection, log: &Log, window: i64) -> (String, bool) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "PREPARE more (bigint) AS {}",
        log.sql.more
    )))
    .execute(&mut *conn)
    .await
    .expect("the check prepares");
    let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN (COSTS OFF) EXECUTE more ({window})"
    )))
    .fetch_all(&mut *conn)
    .await
    .expect("the plan reads");
    sqlx::raw_sql("DEALLOCATE more")
        .execute(&mut *conn)
        .await
        .expect("the check is deallocated");
    let answer: bool = sqlx::query_scalar(log.sql.more)
        .bind(window)
        .fetch_one(&mut *conn)
        .await
        .expect("the check answers");
    (plan.join("\n"), answer)
}

/// Over thousands of seal windows, the generic plan of the check reads the
/// last window by a backward index-only scan, and the answer is the same as
/// asking whether any window follows.
#[tokio::test]
async fn the_check_for_a_later_seal_reads_the_last_window_from_the_index() {
    let db = testkit::db().await.expect("testkit database");
    let mut conn = crate::db::acquire::acquire(&db.pool())
        .await
        .expect("a connection");
    sqlx::raw_sql("SET plan_cache_mode = force_generic_plan")
        .execute(&mut *conn)
        .await
        .expect("the generic plan is forced");
    for (log, seal) in [
        (&ACCESS_LOG, "audit.access_log_seal"),
        (&SYSTEM, "audit.audit_event_seal"),
    ] {
        assert!(
            !more(&mut conn, log, 0).await.1,
            "no seal follows an empty log"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {seal} (window_no, first_id, last_id, entries, previous_digest, \
             digest, sealed_at) SELECT w, w * 10 - 9, w * 10, 10, sha256(int8send(w - 1)), \
             sha256(int8send(w)), now() FROM generate_series(1, {WINDOWS}) AS w; \
             ANALYZE {seal}"
        )))
        .execute(&mut *conn)
        .await
        .expect("the seal windows are written");
        for (window, follows) in [(1, true), (WINDOWS - 10, true), (WINDOWS, false)] {
            let (plan, answer) = more(&mut conn, log, window).await;
            assert!(
                plan.contains("Index Only Scan Backward") && !plan.contains("Seq Scan"),
                "{seal} past window {window}:\n{plan}"
            );
            assert_eq!(answer, follows, "{seal} past window {window}");
        }
    }
}
