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

/// Sweeps until database `name` is gone, read through `pool`, and says
/// whether it went within [`RECLAIM_DEADLINE`]. A clone's lease and
/// sessions end a moment after their sockets close, so one sweep may still
/// find the clone leased or in use.
#[expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this helper; a \
              panicking sweep is how the calling test fails (the Rust Book ch11)"
)]
async fn reclaimed(pool: &sqlx::PgPool, name: &str) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < RECLAIM_DEADLINE {
        testkit::sweep_stale().await.expect("sweep");
        if !exists(pool, name).await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

/// How long the sweeps that follow a clone's end may take to drop it.
const RECLAIM_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);

/// A dropped guard's clone is dropped by the next sweep, as when a passing
/// test returns.
#[tokio::test]
async fn a_dropped_guards_clone_is_reclaimed_by_the_next_sweep() {
    let probe = testkit::db().await.expect("probe clone");
    let db = testkit::db().await.expect("clone");
    let name = db.name().to_owned();
    assert!(
        exists(&probe.pool(), &name).await,
        "the clone {name} was never created"
    );
    drop(db);
    assert!(
        reclaimed(&probe.pool(), &name).await,
        "the sweeps left the dropped guard's clone {name} behind"
    );
}

/// A failing test's clone is dropped by the next sweep, since the unwind
/// drops the guard.
#[tokio::test]
async fn a_failing_tests_clone_is_reclaimed_by_the_next_sweep() {
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
        reclaimed(&probe.pool(), &name).await,
        "the sweeps left the failing test's clone {name} behind"
    );
}

/// Names the child half of
/// [`a_process_that_ends_without_its_drop_has_its_clone_reclaimed`] and asks
/// it to run.
const ENDED_CHILD: &str = "FERROEHR_TESTKIT_ENDED_CHILD";

/// The text after which the child half names its clone, on the line libtest
/// also writes the test's name to.
const CLONE_LINE: &str = "testkit-clone=";

/// The child half: a process that takes a clone, names it, and ends without
/// dropping its guard, as a killed test process does.
#[test]
#[ignore = "the child half of a_process_that_ends_without_its_drop_has_its_clone_reclaimed, run in its own process"]
#[expect(
    clippy::disallowed_methods,
    reason = "the parent asks the child to run in the environment of the process it spawns"
)]
#[expect(
    clippy::mem_forget,
    reason = "the child ends with its guard never dropped, as a killed process does"
)]
fn ended_child() {
    if std::env::var(ENDED_CHILD).is_err() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let db = runtime.block_on(testkit::db()).expect("clone");
    println!("{CLONE_LINE}{}", db.name());
    std::mem::forget(db);
}

/// A process that ends without dropping its guard has its clone dropped by
/// the next sweep, by its lease, which the end of the process frees.
#[tokio::test]
async fn a_process_that_ends_without_its_drop_has_its_clone_reclaimed() {
    let probe = testkit::db().await.expect("probe clone");
    let exe = std::env::current_exe().expect("the test binary");
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(exe)
            .args([
                "harness::ended_child",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ENDED_CHILD, "1")
            .output()
    })
    .await
    .expect("the child's waiter")
    .expect("the child runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the child failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let name = stdout
        .lines()
        .find_map(|line| line.split_once(CLONE_LINE).map(|(_, name)| name.trim()))
        .expect("the child names its clone")
        .to_owned();
    assert!(
        reclaimed(&probe.pool(), &name).await,
        "the sweeps left the ended process's clone {name} behind"
    );
}

/// Opens a session on the server and database of `url` and stops when the
/// server asks for the password, so its backend waits in client
/// authentication until the returned stream closes (`PostgreSQL` docs
/// § Frontend/Backend Protocol, "Start-up").
#[expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this helper; a \
              panicking probe is how the calling test fails (the Rust Book ch11)"
)]
async fn waiting_in_authentication(url: &str) -> tokio::net::TcpStream {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    const PROTOCOL_3_0: i32 = 196_608;
    const AUTHENTICATION: u8 = b'R';
    let options: sqlx::postgres::PgConnectOptions = url.parse().expect("the clone's DSN");
    let database = options.get_database().expect("the DSN names a database");
    let mut body = PROTOCOL_3_0.to_be_bytes().to_vec();
    for field in ["user", options.get_username(), "database", database] {
        body.extend_from_slice(field.as_bytes());
        body.push(0);
    }
    body.push(0);
    let length = i32::try_from(body.len() + 4).expect("a short start-up message");
    let mut stream = tokio::net::TcpStream::connect((options.get_host(), options.get_port()))
        .await
        .expect("a connection to the server");
    stream
        .write_all(&length.to_be_bytes())
        .await
        .expect("the start-up length");
    stream.write_all(&body).await.expect("the start-up message");
    let tag = stream.read_u8().await.expect("the server's answer");
    let _length = stream.read_i32().await.expect("the answer's length");
    let request = stream.read_i32().await.expect("the authentication request");
    assert_eq!(
        tag, AUTHENTICATION,
        "the server answers with an authentication request"
    );
    assert_ne!(request, 0, "the server asks for a password");
    stream
}

/// Dropping a guard returns at once while a session of the same server waits
/// in client authentication, as a connection of a test's runtime does when
/// the test returns mid-handshake: a drop that ran `DROP DATABASE` would wait
/// for that backend until the server's `authentication_timeout` (#6367).
#[tokio::test]
async fn a_guard_drops_at_once_while_a_session_waits_in_authentication() {
    let db = testkit::db().await.expect("clone");
    let waiting = waiting_in_authentication(db.url()).await;
    let start = std::time::Instant::now();
    drop(db);
    let took = start.elapsed();
    drop(waiting);
    assert!(
        took < std::time::Duration::from_secs(15),
        "dropping the guard took {took:?} while a session waited in authentication"
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
