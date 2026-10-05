// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Live gate for the harness itself: server acquisition, template build,
//! and per-test cloning against a real `PostgreSQL` 18.

/// Two clones from one process: unique databases, both fully migrated
/// (the `clinical.node` table exists and is queryable), independent state.
#[tokio::test]
async fn clones_are_unique_and_fully_migrated() {
    let first = testkit::db().await.expect("first clone");
    let second = testkit::db().await.expect("second clone");
    assert_ne!(first.name(), second.name(), "clones must be distinct");

    for db in [&first, &second] {
        let migrated: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'clinical' AND table_name = 'node')",
        )
        .fetch_one(&db.pool())
        .await
        .expect("schema probe");
        assert!(migrated, "clone {} is not migrated", db.name());
    }

    // Writes stay isolated per clone: an EHR row in one never shows in the
    // other (both tables start empty — the template carries schema only).
    sqlx::query(
        "INSERT INTO clinical.ehr (id, system_id, time_created) \
         VALUES (gen_random_uuid(), 'testkit', now())",
    )
    .execute(&first.pool())
    .await
    .expect("insert into first");
    let counts: (i64, i64) = (
        sqlx::query_scalar("SELECT count(*) FROM clinical.ehr")
            .fetch_one(&first.pool())
            .await
            .expect("count first"),
        sqlx::query_scalar("SELECT count(*) FROM clinical.ehr")
            .fetch_one(&second.pool())
            .await
            .expect("count second"),
    );
    assert_eq!(counts, (1, 0), "clone state leaked between databases");
}

/// The sweep reclaims a leaked clone: a database in the harness's own clone
/// namespace whose name-embedded creation time is ancient and which nothing is
/// connected to. This is the regression gate for clone accumulation — thousands
/// of leaked clones inflate the server's cumulative-statistics area until
/// dynamic shared memory is exhausted and every DB-backed test fails with
/// `could not resize shared memory segment ...: No space left on device`.
///
/// Deliberately cluster-global (unlike every other test on the shared server):
/// the harness's own gate is the one place that may assert on server-wide
/// state. The stand-in database carries a unique name inside the harness's
/// prefix, so parallel test processes cannot collide on it.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "non-key randomness: a v4 suffix keeping the stand-in database name \
              unique across parallel test processes on the shared server"
)]
async fn sweep_reclaims_ancient_leaked_clones() {
    let db = testkit::db().await.expect("clone");
    let pool = db.pool();

    // `ferroehr_tk_<secs-hex>_<rand>` with a 1970 creation stamp: exactly what a
    // clone looks like after the owning process was killed before its cleanup
    // landed. `CREATE DATABASE` is cluster-global, so the clone's own
    // connection can create it.
    let leaked = format!("ferroehr_tk_1_leaked{}", uuid::Uuid::new_v4().simple());
    let create = format!("CREATE DATABASE {leaked}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(create))
        .execute(&pool)
        .await
        .expect("create the leaked-clone stand-in");

    testkit::sweep_stale().await.expect("sweep");

    let after: Vec<String> =
        sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname = $1")
            .bind(leaked.as_str())
            .fetch_all(&pool)
            .await
            .expect("catalog probe after the sweep");
    assert!(
        after.is_empty(),
        "sweep left the ancient clone {leaked} behind"
    );

    // The clone this test runs on is young and connected — the same sweep must
    // never touch it.
    let own: Vec<String> = sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname = $1")
        .bind(db.name())
        .fetch_all(&pool)
        .await
        .expect("catalog probe for the running test's own clone");
    assert_eq!(
        own,
        vec![db.name().to_owned()],
        "sweep dropped the running test's own clone"
    );
}

/// The migration fingerprint is stable within a build — the template cache
/// key must not wobble between calls.
#[test]
fn fingerprint_is_stable() {
    assert_eq!(
        ferroehr::db::migration_fingerprint(),
        ferroehr::db::migration_fingerprint()
    );
}
