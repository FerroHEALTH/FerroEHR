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

/// Another fingerprint's template stamped complete moments ago survives the
/// sweep, while one stamped long ago is reclaimed: suites built from different
/// migration fingerprints on one server stop rebuilding each other's
/// templates on every process start.
///
/// Cluster-global like the sweep gate above; both stand-ins carry unique names
/// inside the template namespace, and the young one is dropped at the end.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "non-key randomness: a v4 suffix keeping the stand-in template names \
              unique across parallel test processes on the shared server"
)]
async fn sweep_spares_a_young_foreign_template() {
    let db = testkit::db().await.expect("clone");
    let pool = db.pool();
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM now())::bigint")
        .fetch_one(&pool)
        .await
        .expect("server clock");

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let young_fp = format!("young{suffix}");
    let old_fp = format!("old{suffix}");
    let stand_ins = [(young_fp.as_str(), now), (old_fp.as_str(), 1)];
    for (fingerprint, built) in stand_ins {
        let name = format!("ferroehr_tk_tpl_{fingerprint}");
        // Two statements: `CREATE DATABASE` refuses to run inside the implicit
        // transaction a multi-statement simple query opens.
        let create = format!("CREATE DATABASE {name}");
        let stamp = format!(
            "COMMENT ON DATABASE {name} IS \
             'ferroehr-testkit fingerprint={fingerprint} complete at={built}'"
        );
        for sql in [create, stamp] {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(&pool)
                .await
                .expect("create a foreign-template stand-in");
        }
    }

    testkit::sweep_stale().await.expect("sweep");

    let probe = |name: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT datname FROM pg_database WHERE datname = $1")
                .bind(name)
                .fetch_all(&pool)
                .await
                .expect("catalog probe after the sweep")
        }
    };
    let young = format!("ferroehr_tk_tpl_{young_fp}");
    let old = format!("ferroehr_tk_tpl_{old_fp}");
    let young_after = probe(young.clone()).await;
    let old_after = probe(old.clone()).await;
    let cleanup = format!("DROP DATABASE IF EXISTS {young}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(cleanup))
        .execute(&pool)
        .await
        .expect("drop the young stand-in");

    assert_eq!(
        young_after,
        vec![young.clone()],
        "sweep dropped the young foreign template {young}"
    );
    assert!(
        old_after.is_empty(),
        "sweep left the old foreign template {old} behind"
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

/// A template dropped mid-run, as the startup sweep of a suite built from
/// another migration fingerprint drops it on a shared server, is rebuilt, and
/// the next clone is fully migrated: a clone is made only from a stamped
/// template, never from one being rebuilt.
///
/// Cluster-global like the sweep gate above: it removes the shared template,
/// which every concurrent process then rebuilds through the same path.
#[tokio::test]
async fn a_dropped_template_is_rebuilt_before_the_next_clone() {
    let first = testkit::db().await.expect("first clone");
    let template = format!("ferroehr_tk_tpl_{}", ferroehr::db::migration_fingerprint());
    let drop_template = format!("DROP DATABASE IF EXISTS {template} WITH (FORCE)");
    sqlx::raw_sql(sqlx::AssertSqlSafe(drop_template))
        .execute(&first.pool())
        .await
        .expect("drop the template as a foreign sweep does");

    let second = testkit::db()
        .await
        .expect("a clone after the template was dropped");
    ferroehr::db::verify_migrations(&second.pool())
        .await
        .expect("the clone carries every embedded migration");
    let restored: Vec<String> =
        sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname = $1")
            .bind(template.as_str())
            .fetch_all(&second.pool())
            .await
            .expect("catalog probe for the template");
    assert_eq!(restored, vec![template], "the template was not rebuilt");
}

/// Whether database `name` exists, read through `pool` (the catalog is
/// cluster-global).
#[expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this helper; a \
              panicking probe is how the calling test fails (the Rust Book ch11)"
)]
async fn exists(pool: &sqlx::PgPool, name: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("catalog probe")
}

/// A clone is gone the moment its guard drops, as when a passing test
/// returns (#3924).
#[tokio::test]
async fn a_passing_tests_clone_is_dropped_with_its_guard() {
    let probe = testkit::db().await.expect("probe clone");
    let db = testkit::db().await.expect("clone");
    let name = db.name().to_owned();
    assert!(
        exists(&probe.pool(), &name).await,
        "the clone {name} was never created"
    );
    drop(db);
    assert!(
        !exists(&probe.pool(), &name).await,
        "dropping the guard left {name} behind"
    );
}

/// A clone is gone once a test that holds it panics, since the unwind drops
/// the guard (#3924).
#[tokio::test]
async fn a_failing_tests_clone_is_dropped_by_the_unwind() {
    let probe = testkit::db().await.expect("probe clone");
    let (named, name) = std::sync::mpsc::channel();
    // A test of its own, on its own runtime, as nextest runs one.
    let failing = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let db = testkit::db().await.expect("clone");
            named
                .send(db.name().to_owned())
                .expect("send the clone's name");
            panic!("a failing test holding {}", db.name());
        });
    });
    assert!(failing.join().is_err(), "the stand-in test must fail");
    let name = name.recv().expect("the clone's name");
    assert!(
        !exists(&probe.pool(), &name).await,
        "the failing test left {name} behind"
    );
}

/// A clone whose process was killed, so its lease is free, is swept however
/// young it is, while a clone whose lease is held survives (#3924).
///
/// Cluster-global like the sweep gate above; the stand-in carries a unique,
/// freshly stamped name inside the harness's prefix.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "non-key randomness: a v4 suffix keeping the stand-in database name \
              unique across parallel test processes on the shared server"
)]
async fn a_killed_runs_young_clone_is_swept_by_its_free_lease() {
    let db = testkit::db().await.expect("clone");
    let pool = db.pool();
    let now = u64::try_from(jiff::Timestamp::now().as_second()).expect("a post-1970 clock");
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let killed = format!("ferroehr_tk_{now:x}_{}", suffix.get(..12).expect("a uuid"));
    let create = format!("CREATE DATABASE {killed}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(create))
        .execute(&pool)
        .await
        .expect("create the killed run's clone");

    testkit::sweep_stale().await.expect("sweep");

    assert!(
        !exists(&pool, &killed).await,
        "the sweep left the killed run's clone {killed} behind"
    );
    assert!(
        exists(&pool, db.name()).await,
        "the sweep dropped {}, whose lease is held",
        db.name()
    );
}

/// The sweep never drops a database a live session holds the lease of: a
/// sweeper loops while eight threads, each a session of its own like a test
/// process, take clones of a template that was just dropped, so one of them
/// rebuilds it through a clone-shaped staging database under the sweep
/// (#3924: a sweep once dropped that staging database mid-build, and the
/// rename failed with SQLSTATE 3D000).
///
/// Cluster-global like the template test above: it drops the shared template.
#[test]
fn the_sweep_spares_every_leased_clone_and_staging_database_under_load() {
    const WORKERS: usize = 8;
    const CLONES_EACH: usize = 4;
    let runtime = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    };
    runtime().block_on(async {
        let probe = testkit::db().await.expect("probe clone");
        let template = format!("ferroehr_tk_tpl_{}", ferroehr::db::migration_fingerprint());
        let drop_template = format!("DROP DATABASE IF EXISTS {template} WITH (FORCE)");
        sqlx::raw_sql(sqlx::AssertSqlSafe(drop_template))
            .execute(&probe.pool())
            .await
            .expect("drop the template so the workers rebuild it");
    });

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sweeper = {
        let stop = std::sync::Arc::clone(&stop);
        std::thread::spawn(move || {
            runtime().block_on(async {
                let mut sweeps = 0_u32;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    testkit::sweep_stale().await.expect("sweep");
                    sweeps += 1;
                }
                sweeps
            })
        })
    };
    let workers: Vec<_> = (0..WORKERS)
        .map(|_| {
            std::thread::spawn(move || {
                runtime().block_on(async {
                    for _ in 0..CLONES_EACH {
                        let db = testkit::db().await.expect("a clone under the sweep");
                        ferroehr::db::verify_migrations(&db.pool())
                            .await
                            .expect("the clone is whole while its lease is held");
                    }
                });
            })
        })
        .collect();
    let failed = workers
        .into_iter()
        .map(std::thread::JoinHandle::join)
        .filter(Result::is_err)
        .count();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let sweeps = sweeper.join().expect("the sweeper ends");
    assert_eq!(failed, 0, "{failed} worker(s) lost a clone to the sweep");
    assert!(sweeps > 0, "the sweep never ran beside the workers");
}
