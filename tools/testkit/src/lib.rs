// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared test-database harness for every DB-backed test in the workspace.
//!
//! One `PostgreSQL` 18 server, one migrated template database per migration
//! fingerprint, one `CREATE DATABASE … TEMPLATE …` clone per test. No openEHR
//! spec governs test infrastructure — our own design/extension.
//!
//! The server is `FERROEHR_TEST_PG_URL` when set (a DSN whose role may
//! `CREATE DATABASE`), otherwise the reusable named container
//! `ferroehr-testkit-pg18` (`postgres:18`), which is left running across runs
//! and reclaimed with `docker rm -f ferroehr-testkit-pg18`.
//!
//! The template (`ferroehr_tk_tpl_<fingerprint>`) is built once per
//! fingerprint under a `PostgreSQL` advisory lock, migrated under a staging
//! name and renamed into place, and stamped complete as its database comment,
//! read via `shobj_description` because a connection to a template blocks
//! cloning (`PostgreSQL` docs § CREATE DATABASE); the stamp carries the build
//! time, so another fingerprint's sweep spares a template younger than
//! `TEMPLATE_GRACE`. A clone is made under the
//! same lock in shared mode and only from a stamped template. Clones are
//! named `ferroehr_tk_<secs>_<rand>` and swept by [`sweep_stale`], which every
//! test process runs once before it hands out a database. A clone is leased
//! to the process that made it by a session-level advisory lock held on its
//! own connection for the clone's life, so the sweep drops a clone whose
//! lease is free (its process ended, however it ended) and never one whose
//! lease is held (`PostgreSQL` docs § Advisory Locks).

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
pub mod json_literals;
pub mod syslog_tls;

use std::time::{Duration, Instant};

use sqlx::{Connection, PgConnection, PgPool};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt, ReuseDirective};
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;

use ferroehr::db;
use ferroehr::db::DbConfig;

/// Prefix for every database the harness creates (templates + clones), so a
/// sweep can never touch anything else.
const DB_PREFIX: &str = "ferroehr_tk_";

/// The fixed name of the reusable local `PostgreSQL` container.
const CONTAINER_NAME: &str = "ferroehr-testkit-pg18";

/// Explicit shared-memory (`/dev/shm`) size for the reusable container, in
/// bytes — one gibibyte.
///
/// The image default is 64 MB, which the cumulative-statistics area and
/// parallel-query segments exhaust on a server carrying many databases; every
/// dynamic allocation then fails (<https://hub.docker.com/_/postgres>,
/// "set shared memory limit when using docker compose"; `PostgreSQL` docs
/// § Resource Consumption). The size applies at container CREATION only, so a
/// running 64 MB container must be reclaimed for a change to take effect.
const SHM_SIZE_BYTES: u64 = 1024 * 1024 * 1024;

/// `SQLSTATE` 55006 `object_in_use` — what `PostgreSQL` reports when
/// `DROP DATABASE` is refused because sessions are still connected
/// (`PostgreSQL` docs § DROP DATABASE; § Appendix A "`PostgreSQL` Error
/// Codes", class 55 "Object Not In Prerequisite State").
const SQLSTATE_OBJECT_IN_USE: &str = "55006";

/// Environment variable naming an externally provided server (CI, local dev).
const ENV_URL: &str = "FERROEHR_TEST_PG_URL";

/// Advisory-lock key serializing template builds across test processes.
const TEMPLATE_LOCK_KEY: i64 = 0x0EB2_7E57_0001;

/// Advisory-lock key electing the single startup sweeper.
const SWEEP_LOCK_KEY: i64 = 0x0EB2_7E57_0002;

/// Clones younger than this grace window are never swept, for a clone name
/// that carries no lease key.
///
/// A running test's clone can momentarily have no session on it (the pools run
/// `min_connections = 0`), so connectedness cannot tell live from leaked; the
/// lease can, and for a name without one the name-embedded creation time can.
const SWEEP_GRACE: Duration = Duration::from_mins(30);

/// The first key of every clone lease, a two-key session-level advisory lock
/// whose second key comes from the clone's name ([`lease_key`]).
const CLONE_LEASE_CLASS: i32 = 0x0EB2_7E57;

/// Templates of another migration fingerprint stamped complete less than this
/// long ago are never swept.
///
/// Suites built from different fingerprints share one server; without the
/// grace each one's startup sweep drops the other's template, which then
/// rebuilds on its next clone.
const TEMPLATE_GRACE: Duration = Duration::from_mins(30);

/// The leading text of every template completion stamp.
const STAMP_PREFIX: &str = "ferroehr-testkit fingerprint=";

/// Failures of the harness itself (server acquisition, template build,
/// cloning) — always broken test infrastructure, never application behaviour.
#[derive(Debug, thiserror::Error)]
pub enum TestkitError {
    /// Starting or adopting the reusable `PostgreSQL` container failed.
    #[error("testkit: starting or adopting the container failed")]
    Container(#[from] testcontainers::TestcontainersError),
    /// A query/connection against the server failed.
    #[error("testkit: a query or connection failed")]
    Sqlx(#[from] sqlx::Error),
    /// Pool construction or migration via the platform library failed.
    #[error("testkit: building or migrating the platform pool failed")]
    Db(#[from] db::DbError),
    /// The server did not accept connections within the readiness deadline.
    #[error("testkit: server at {url} not ready within {seconds}s: {last}")]
    NotReady {
        /// Redacted server location (host:port/db).
        url: String,
        /// Deadline that elapsed.
        seconds: u64,
        /// The last connection error observed.
        last: String,
    },
    /// No database is available on this box: `FERROEHR_TEST_PG_URL` is unset
    /// and no Docker daemon answered, so container-backed tests cannot run
    /// here. The message carries the probe result and the way out; a broken
    /// setup with a REACHABLE daemon still fails through the other variants.
    #[error("testkit: {0}")]
    DockerUnavailable(String),
    /// The migrated template stayed absent or incomplete through every rebuild
    /// within the deadline, so no clone could be made from it.
    #[error("testkit: template {template} not available for cloning within {seconds}s")]
    TemplateUnavailable {
        /// The template database name.
        template: String,
        /// Deadline that elapsed.
        seconds: u64,
    },
    /// The container's published host has no IPv4 address to reach the
    /// IPv4-published port on.
    #[error("testkit: the container host {host} resolves to no IPv4 address: {reason}")]
    NoIpv4Host {
        /// The host the container runtime reported.
        host: String,
        /// Why no IPv4 address came back.
        reason: String,
    },
}

/// One fresh, fully migrated database for one test: a clone of the migrated
/// template.
///
/// Dropping the guard drops the database before the drop returns, so a test
/// that passes and one that panics both leave nothing behind; the startup
/// sweep reclaims what a killed process left, by its free lease.
#[derive(Debug)]
pub struct TestDb {
    pool: PgPool,
    url: String,
    name: String,
    admin_url: String,
    /// The connection holding the clone's lease; closing it frees the lease.
    lease: Option<PgConnection>,
}

impl TestDb {
    /// A handle to the clone's connection pool (cheaply cloneable).
    #[must_use]
    pub fn pool(&self) -> PgPool {
        self.pool.clone()
    }

    /// The clone's DSN, for call sites that build their own configuration.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The database name (unique per call).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // `Drop` cannot await, so the drop runs on a thread of its own, and is
        // joined: a detached thread dies with the test process, which nextest
        // ends as soon as the test returns, and its clone leaked (#3924).
        //
        // NOTE: `WITH (FORCE)` is correct here and wrong in the sweep — it
        // terminates our own clone's connections, never another process's.
        let admin_url = self.admin_url.clone();
        let name = self.name.clone();
        let dropper = std::thread::Builder::new().spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                rt.block_on(async {
                    if let Ok(mut conn) = PgConnection::connect(&admin_url).await {
                        let sql = format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)");
                        drop(
                            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                                .execute(&mut conn)
                                .await,
                        );
                        drop(conn.close().await);
                    }
                });
            }
        });
        if let Ok(dropper) = dropper {
            drop(dropper.join());
        }
        drop(self.lease.take());
    }
}

/// Returns a fresh, fully migrated database on the shared test server.
///
/// Call it once per independent database a test needs.
///
/// # Errors
///
/// Returns [`TestkitError`] when the server cannot be acquired, the template
/// cannot be built, or the clone/pool fails.
pub async fn db() -> Result<TestDb, TestkitError> {
    let server = server().await?;
    let template = TEMPLATE.get_or_try_init(|| ensure_template(server)).await?;
    provision(server, Some(template)).await
}

/// Returns a fresh, pristine (non-migrated) database on the shared test server,
/// for harnesses that lay down their own DDL.
///
/// # Errors
///
/// Returns [`TestkitError`] when the server cannot be acquired or the
/// database/pool creation fails.
pub async fn empty_db() -> Result<TestDb, TestkitError> {
    let server = server().await?;
    provision(server, None).await
}

/// Creates a uniquely named database (a clone of `template`, or empty) and
/// pools it.
///
/// A clone is verified against the migrations this build embeds before it is
/// handed out, so a database that is not fully migrated fails here, named as
/// harness breakage, rather than inside the test that received it.
async fn provision(server: &str, template: Option<&str>) -> Result<TestDb, TestkitError> {
    const POOL_DEADLINE: Duration = Duration::from_secs(15);
    let name = fresh_name();
    // The lease is taken before the clone exists, so no sweep ever sees the
    // clone without its owner.
    let mut lease = connect_ready(server).await?;
    if let Some(key) = lease_key(&name) {
        sqlx::query("SELECT pg_advisory_lock($1, $2)")
            .bind(CLONE_LEASE_CLASS)
            .bind(key)
            .execute(&mut lease)
            .await?;
    }
    let mut admin = connect_ready(server).await?;
    if let Some(template) = template {
        create_clone(&mut admin, server, &name, template).await?;
    } else {
        let sql = format!("CREATE DATABASE {name}");
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut admin)
            .await?;
    }
    drop(admin.close().await);

    let url = clone_url(server, &name);
    // Every clone pool shares one server started with `max_connections=200`.
    let mut config = DbConfig::new(url.clone());
    config.max_connections = 10;
    config.min_connections = 0;
    // Retry ESTABLISHMENT only: under a full-parallel nextest run the shared
    // server's accept path is briefly saturable, which is load, not a defect.
    let start = Instant::now();
    let pool = loop {
        match db::connect(&config).await {
            Ok(pool) => break pool,
            Err(error) => {
                if start.elapsed() >= POOL_DEADLINE {
                    return Err(TestkitError::from(error));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let db = TestDb {
        pool,
        url,
        name,
        admin_url: server.to_owned(),
        lease: Some(lease),
    };
    if template.is_some() {
        db::verify_migrations(&db.pool).await?;
    }
    Ok(db)
}

// ---------------------------------------------------------------------------
// Server acquisition
// ---------------------------------------------------------------------------

/// The admin DSN of the shared server, resolved once per test process.
static SERVER: OnceCell<String> = OnceCell::const_new();

/// The reusable container guard, kept only so the handle lives for the process
/// (`ReuseDirective::Always` means dropping it never stops the container).
static CONTAINER: OnceCell<ContainerAsync<Postgres>> = OnceCell::const_new();

/// The template database name, ensured once per test process (and, via the
/// advisory lock inside, once per server across processes).
static TEMPLATE: OnceCell<String> = OnceCell::const_new();

/// The shared server's admin DSN, acquired once per test process, including
/// the readiness wait and the startup sweep.
async fn server() -> Result<&'static str, TestkitError> {
    SERVER
        .get_or_try_init(|| async {
            let url = resolve_server_url().await?;
            let mut admin = connect_ready(&url).await?;
            startup_sweep(&mut admin).await;
            drop(admin.close().await);
            Ok(url)
        })
        .await
        .map(String::as_str)
}

/// The admin DSN of the externally provided server, or of the reusable
/// container (started or adopted).
#[expect(
    clippy::disallowed_methods,
    reason = "`FERROEHR_TEST_PG_URL` and `DOCKER_HOST` are the harness's OWN \
              environment contracts (CI hands it the workflow server; Docker's \
              own variable names the daemon); testkit is test tooling and must \
              not depend on the server's config tree, which is what that ban \
              protects"
)]
async fn resolve_server_url() -> Result<String, TestkitError> {
    if let Ok(url) = std::env::var(ENV_URL) {
        return Ok(url);
    }
    probe_docker(std::env::var("DOCKER_HOST").ok().as_deref())?;
    let container = CONTAINER.get_or_try_init(start_container).await?;
    let host = container.get_host().await?.to_string();
    let port = container.get_host_port_ipv4(5432).await?;
    let address = ipv4_address(&host, port).await?;
    Ok(format!("postgres://postgres:postgres@{address}/postgres"))
}

/// Returns the IPv4 socket address `host` reaches the IPv4-published `port` on.
///
/// The DSN names this address rather than the host, which is usually
/// `localhost`: a name that resolves to `::1` first makes every connection try
/// an IPv6 port nothing listens on, and since a published port lies in the
/// ephemeral range, the kernel now and then picks that very port as the
/// attempt's source port, so the connection opens onto itself (RFC 9293 §3.5,
/// simultaneous open) and the client reads its own `SSLRequest` back as a
/// `0x00` reply.
async fn ipv4_address(host: &str, port: u16) -> Result<std::net::SocketAddr, TestkitError> {
    let no_ipv4 = |reason: String| TestkitError::NoIpv4Host {
        host: host.to_owned(),
        reason,
    };
    tokio::net::lookup_host(format!("{host}:{port}"))
        .await
        .map_err(|e| no_ipv4(e.to_string()))?
        .find(std::net::SocketAddr::is_ipv4)
        .ok_or_else(|| no_ipv4("the lookup returned only IPv6 addresses".to_owned()))
}

/// Starts — or adopts — the reusable named `PostgreSQL` 18 container.
///
/// Concurrent first boots race on the fixed name; the losers retry and the
/// reuse lookup then finds the winner's container.
/// Fails fast, with the way out in the message, when no Docker daemon can
/// answer — BEFORE testcontainers burns its retry ladder on connection
/// refusals.
///
/// The live failure class this closes (#3009): `/var/run/docker.sock` on a
/// developer box can be a dead symlink to another runtime's socket (a podman
/// machine that is not running) while Docker Desktop serves its real socket
/// at `~/.docker/run/docker.sock`; testcontainers then reports a bare
/// `ConnectionRefused` per test. `std::env::set_var` is unsafe in edition
/// 2024 and this workspace forbids `unsafe`, so the harness cannot switch
/// sockets itself — it names the exact `DOCKER_HOST` to run with instead.
///
/// An explicit `DOCKER_HOST` is trusted as given: a TCP endpoint is not
/// probed here (testcontainers owns that connection), and a Unix endpoint is
/// probed so a dead override still gets the fast, named failure.
fn probe_docker(docker_host: Option<&str>) -> Result<(), TestkitError> {
    fn answers(path: &std::path::Path) -> bool {
        std::os::unix::net::UnixStream::connect(path).is_ok()
    }
    if let Some(host) = docker_host {
        if let Some(path) = host.strip_prefix("unix://") {
            if answers(std::path::Path::new(path)) {
                return Ok(());
            }
            return Err(TestkitError::DockerUnavailable(format!(
                "DOCKER_HOST is set to {host}, but nothing answers on that socket — start \
                 that daemon, unset DOCKER_HOST, or export {ENV_URL} to use an external \
                 PostgreSQL; container-backed tests cannot run until one of those holds"
            )));
        }
        return Ok(());
    }
    let default = std::path::PathBuf::from("/var/run/docker.sock");
    if answers(&default) {
        return Ok(());
    }
    let desktop = std::env::home_dir().map(|h| h.join(".docker/run/docker.sock"));
    if let Some(desktop) = desktop.filter(|p| answers(p)) {
        return Err(TestkitError::DockerUnavailable(format!(
            "/var/run/docker.sock answers nothing (often a dead symlink to another \
             runtime's socket), but a live Docker Desktop daemon answered at \
             {socket} — run the tests with DOCKER_HOST=unix://{socket} (or export \
             {ENV_URL} to use an external PostgreSQL)",
            socket = desktop.display()
        )));
    }
    Err(TestkitError::DockerUnavailable(format!(
        "no Docker daemon reachable (probed /var/run/docker.sock and the Docker \
         Desktop socket under $HOME/.docker/run/) and {ENV_URL} is unset — \
         container-backed tests are not exercised on this box; start Docker or \
         export {ENV_URL}"
    )))
}

async fn start_container() -> Result<ContainerAsync<Postgres>, TestkitError> {
    let mut last = None;
    for attempt in 0..5u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
        }
        let request = Postgres::default()
            .with_tag("18")
            // PostgreSQL docs § "Non-Durable Settings": the server is disposable.
            .with_cmd([
                "postgres",
                "-c",
                "fsync=off",
                "-c",
                "synchronous_commit=off",
                "-c",
                "full_page_writes=off",
                "-c",
                "max_connections=200",
            ])
            .with_shm_size(SHM_SIZE_BYTES)
            .with_container_name(CONTAINER_NAME)
            .with_reuse(ReuseDirective::Always);
        match request.start().await {
            Ok(container) => return Ok(container),
            Err(error) => last = Some(error),
        }
    }
    match last {
        Some(error) => Err(TestkitError::Container(error)),
        // Unreachable: five attempts either returned or set `last`.
        None => Err(TestkitError::NotReady {
            url: CONTAINER_NAME.to_owned(),
            seconds: 0,
            last: "container start never attempted".to_owned(),
        }),
    }
}

/// Connects to the admin database, retrying while the server finishes booting.
async fn connect_ready(admin_url: &str) -> Result<PgConnection, TestkitError> {
    const DEADLINE: Duration = Duration::from_mins(1);
    let start = Instant::now();
    loop {
        match PgConnection::connect(admin_url).await {
            Ok(conn) => return Ok(conn),
            Err(error) => {
                if start.elapsed() >= DEADLINE {
                    return Err(TestkitError::NotReady {
                        url: redacted(admin_url),
                        seconds: DEADLINE.as_secs(),
                        last: error.to_string(),
                    });
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

// ---------------------------------------------------------------------------
// Template database
// ---------------------------------------------------------------------------

/// Returns the migrated template database for the current migration
/// fingerprint, building it under an advisory lock when absent.
async fn ensure_template(admin_url: &str) -> Result<String, TestkitError> {
    let fingerprint = db::migration_fingerprint();
    let template = format!("{DB_PREFIX}tpl_{fingerprint}");

    let mut admin = connect_ready(admin_url).await?;

    if template_complete(&mut admin, &template, &fingerprint).await? {
        drop(admin.close().await);
        return Ok(template);
    }

    // The lock is session-scoped: hold this connection until the stamp lands.
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(TEMPLATE_LOCK_KEY)
        .execute(&mut admin)
        .await?;
    let outcome = build_template(&mut admin, admin_url, &template, &fingerprint).await;
    drop(
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(TEMPLATE_LOCK_KEY)
            .execute(&mut admin)
            .await,
    );
    drop(admin.close().await);
    outcome?;
    Ok(template)
}

/// The completion stamp a template of migration `fingerprint` carries as its
/// database comment, recording `built_secs` (Unix seconds) as its build time.
fn template_stamp(fingerprint: &str, built_secs: u64) -> String {
    format!("{STAMP_PREFIX}{fingerprint} complete at={built_secs}")
}

/// A parsed completion stamp.
#[derive(Debug, PartialEq, Eq)]
struct TemplateStamp<'a> {
    /// The migration fingerprint the template was built from.
    fingerprint: &'a str,
    /// When the template was stamped complete (Unix seconds); `None` for a
    /// stamp written before the build time was recorded.
    built_secs: Option<u64>,
}

/// Parses a template's database comment as a completion stamp.
///
/// Returns `None` for anything that is not a stamp; a stamp without a build
/// time still reads as complete, with an unknown age.
fn parse_stamp(comment: &str) -> Option<TemplateStamp<'_>> {
    let rest = comment.strip_prefix(STAMP_PREFIX)?;
    let (fingerprint, tail) = rest.split_once(' ')?;
    if tail == "complete" {
        return Some(TemplateStamp {
            fingerprint,
            built_secs: None,
        });
    }
    let built = tail.strip_prefix("complete at=")?.parse().ok()?;
    Some(TemplateStamp {
        fingerprint,
        built_secs: Some(built),
    })
}

/// Whether `template` exists and carries the completion stamp of
/// `fingerprint`.
async fn template_complete(
    admin: &mut PgConnection,
    template: &str,
    fingerprint: &str,
) -> Result<bool, sqlx::Error> {
    // `shobj_description` is NULL for a template with no comment yet, so the
    // scalar is doubly optional: NULL means "not complete", never an error.
    let comment: Option<Option<String>> = sqlx::query_scalar(
        "SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = $1",
    )
    .bind(template)
    .fetch_optional(admin)
    .await?;
    Ok(comment
        .flatten()
        .as_deref()
        .and_then(parse_stamp)
        .is_some_and(|stamp| stamp.fingerprint == fingerprint))
}

/// Builds the template of migration `fingerprint` under the already-held
/// advisory lock: re-check, drop any incomplete carcass, migrate a staging
/// database, rename it to the template, then stamp the database comment with
/// the fingerprint and the build time.
///
/// The template name therefore only ever names a fully migrated database. The
/// staging database carries a clone-shaped name and its lease is held on
/// `admin` until it is renamed, so a concurrent process's sweep spares it
/// instead of dropping it mid-migration; another fingerprint's suite on the same server spares THIS template for
/// `TEMPLATE_GRACE` after the stamp and sweeps it as stale after that, and
/// nothing it drops can be half built.
async fn build_template(
    admin: &mut PgConnection,
    admin_url: &str,
    template: &str,
    fingerprint: &str,
) -> Result<(), TestkitError> {
    if template_complete(admin, template, fingerprint).await? {
        return Ok(());
    }
    let drop_sql = format!("DROP DATABASE IF EXISTS {template} WITH (FORCE)");
    sqlx::raw_sql(sqlx::AssertSqlSafe(drop_sql))
        .execute(&mut *admin)
        .await?;
    let staging = fresh_name();
    let lease = lease_key(&staging);
    if let Some(key) = lease {
        sqlx::query("SELECT pg_advisory_lock($1, $2)")
            .bind(CLONE_LEASE_CLASS)
            .bind(key)
            .execute(&mut *admin)
            .await?;
    }
    let create_sql = format!("CREATE DATABASE {staging}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(create_sql))
        .execute(&mut *admin)
        .await?;
    let built = migrate_and_rename(admin, admin_url, &staging, template).await;
    if let Some(key) = lease {
        drop(
            sqlx::query("SELECT pg_advisory_unlock($1, $2)")
                .bind(CLONE_LEASE_CLASS)
                .bind(key)
                .execute(&mut *admin)
                .await,
        );
    }
    if built.is_err() {
        let sql = format!("DROP DATABASE IF EXISTS {staging} WITH (FORCE)");
        drop(
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(&mut *admin)
                .await,
        );
    }
    built?;

    let stamp = template_stamp(fingerprint, now_secs());
    let comment_sql = format!("COMMENT ON DATABASE {template} IS '{stamp}'");
    sqlx::raw_sql(sqlx::AssertSqlSafe(comment_sql))
        .execute(&mut *admin)
        .await?;
    Ok(())
}

/// Migrates the `staging` database, closes every connection to it, and renames
/// it to `template`.
///
/// The pool is closed fully first: `ALTER DATABASE … RENAME` and every later
/// clone refuse a database other sessions are connected to (`PostgreSQL` docs
/// § ALTER DATABASE, § CREATE DATABASE).
async fn migrate_and_rename(
    admin: &mut PgConnection,
    admin_url: &str,
    staging: &str,
    template: &str,
) -> Result<(), TestkitError> {
    let pool = db::connect(&DbConfig::new(clone_url(admin_url, staging))).await?;
    let migrated = db::run_migrations(&pool).await;
    pool.close().await;
    migrated?;
    let rename_sql = format!("ALTER DATABASE {staging} RENAME TO {template}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(rename_sql))
        .execute(&mut *admin)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Clones
// ---------------------------------------------------------------------------

/// A unique clone name embedding its creation time (hex seconds) for the
/// sweep: `ferroehr_tk_<secs-hex>_<rand>`.
#[expect(
    clippy::disallowed_methods,
    reason = "non-key randomness: a v4 suffix that makes a clone's database name \
              unique across concurrent test processes — no index locality to gain, \
              so the uuidv7 key rule does not apply"
)]
fn fresh_name() -> String {
    let secs = now_secs();
    let rand = uuid::Uuid::new_v4().simple().to_string();
    let short = rand.get(..12).unwrap_or(rand.as_str());
    format!("{DB_PREFIX}{secs:x}_{short}")
}

/// The second key of the lease on clone `name`: the first eight hex digits of
/// its random suffix, or `None` for a name [`fresh_name`] did not make.
fn lease_key(name: &str) -> Option<i32> {
    let rest = name.strip_prefix(DB_PREFIX)?;
    if rest.starts_with("tpl_") {
        return None;
    }
    let (_, suffix) = rest.split_once('_')?;
    let digits = suffix.get(..8)?;
    let key = u32::from_str_radix(digits, 16).ok()?;
    Some(i32::from_be_bytes(key.to_be_bytes()))
}

/// Clones the template, only ever from a complete one.
///
/// Each attempt holds the template advisory lock in SHARED mode, so no build
/// (which holds it exclusively) runs between the completion check and the
/// clone. A template that is gone or unstamped (another fingerprint's sweep
/// drops this one as stale) is rebuilt through [`ensure_template`] with the
/// shared lock released; any other refusal, such as the transient conflict
/// concurrent clones of one template can hit, is retried. Both stop at the
/// deadline.
async fn create_clone(
    admin: &mut PgConnection,
    admin_url: &str,
    name: &str,
    template: &str,
) -> Result<(), TestkitError> {
    const DEADLINE: Duration = Duration::from_mins(1);
    let fingerprint = db::migration_fingerprint();
    let start = Instant::now();
    loop {
        sqlx::query("SELECT pg_advisory_lock_shared($1)")
            .bind(TEMPLATE_LOCK_KEY)
            .execute(&mut *admin)
            .await?;
        let attempt = clone_if_complete(admin, name, template, &fingerprint).await;
        sqlx::query("SELECT pg_advisory_unlock_shared($1)")
            .bind(TEMPLATE_LOCK_KEY)
            .execute(&mut *admin)
            .await?;
        let expired = start.elapsed() >= DEADLINE;
        match attempt {
            Ok(CloneAttempt::Cloned) => return Ok(()),
            Ok(CloneAttempt::TemplateIncomplete) if expired => {
                return Err(TestkitError::TemplateUnavailable {
                    template: template.to_owned(),
                    seconds: DEADLINE.as_secs(),
                });
            }
            Ok(CloneAttempt::TemplateIncomplete) => {
                ensure_template(admin_url).await?;
            }
            Err(error) if expired => return Err(TestkitError::Sqlx(error)),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// What one clone attempt under the shared template lock found.
enum CloneAttempt {
    /// The clone was created from the complete template.
    Cloned,
    /// The template is absent or not stamped complete; nothing was created.
    TemplateIncomplete,
}

/// Clones `template` as `name` when it carries the completion stamp of
/// `fingerprint`.
async fn clone_if_complete(
    admin: &mut PgConnection,
    name: &str,
    template: &str,
    fingerprint: &str,
) -> Result<CloneAttempt, sqlx::Error> {
    if !template_complete(admin, template, fingerprint).await? {
        return Ok(CloneAttempt::TemplateIncomplete);
    }
    // Default WAL_LOG strategy: no forced checkpoint per clone (PostgreSQL docs
    // § CREATE DATABASE, `STRATEGY`).
    let sql = format!("CREATE DATABASE {name} TEMPLATE {template}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(&mut *admin)
        .await?;
    Ok(CloneAttempt::Cloned)
}

/// The DSN for database `name` on the server `admin_url` points at.
fn clone_url(admin_url: &str, name: &str) -> String {
    match admin_url.rfind('/') {
        Some(cut) => {
            let base = admin_url.get(..cut).unwrap_or(admin_url);
            format!("{base}/{name}")
        }
        None => format!("{admin_url}/{name}"),
    }
}

/// `host:port/db` with credentials stripped, for error messages.
fn redacted(url: &str) -> String {
    url.rsplit('@').next().unwrap_or("<unparseable>").to_owned()
}

/// Wall-clock seconds since the Unix epoch — the time base embedded in every
/// clone name and read back by the sweep.
///
/// A negative timestamp (a clock set before 1970) floors at 0 so a clone name
/// always parses back.
fn now_secs() -> u64 {
    u64::try_from(jiff::Timestamp::now().as_second()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Sweep
// ---------------------------------------------------------------------------

/// What one sweep pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepReport {
    /// Stale databases the pass selected for dropping.
    pub candidates: usize,
    /// Databases actually dropped.
    pub dropped: usize,
    /// Databases another test process was still connected to, so the
    /// force-free drop was refused and the database left alone.
    pub in_use: usize,
}

/// Reclaims stale harness databases (and stale harness roles) now.
///
/// Safe to call at any time: the drops are force-free, so a database another
/// process is connected to is skipped rather than torn out from under it.
/// The pass waits for the sweep lock, so it never runs beside another
/// process's sweep: a concurrent sweeper that took a free clone's lease
/// would otherwise make this pass skip the clone and return before the other
/// pass dropped it.
///
/// # Errors
///
/// Returns [`TestkitError`] when the shared server cannot be acquired or
/// connected to, or the sweep lock cannot be taken. Individual drops never
/// fail the call — an in-use database is counted as [`SweepReport::in_use`]
/// and any other failure is logged.
pub async fn sweep_stale() -> Result<SweepReport, TestkitError> {
    let server = server().await?;
    let mut admin = connect_ready(server).await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(SWEEP_LOCK_KEY)
        .execute(&mut admin)
        .await?;
    let report = sweep(&mut admin).await;
    drop(
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(SWEEP_LOCK_KEY)
            .execute(&mut admin)
            .await,
    );
    drop(admin.close().await);
    Ok(report)
}

/// The once-per-process sweep run while the server is acquired, elected by an
/// advisory try-lock so only one process of a parallel run sweeps.
async fn startup_sweep(admin: &mut PgConnection) {
    let elected: Result<Option<bool>, sqlx::Error> =
        sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(SWEEP_LOCK_KEY)
            .fetch_optional(&mut *admin)
            .await;
    if !matches!(elected, Ok(Some(true))) {
        return;
    }

    let report = sweep(&mut *admin).await;
    let candidates = report.candidates;
    let dropped = report.dropped;
    let in_use = report.in_use;
    tracing::info!(
        "testkit startup sweep: {candidates} stale database(s), {dropped} dropped, {in_use} in use"
    );

    drop(
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(SWEEP_LOCK_KEY)
            .execute(&mut *admin)
            .await,
    );
}

/// Drops every stale harness database (clones older than [`SWEEP_GRACE`], and
/// templates of another migration fingerprint stamped longer than
/// `TEMPLATE_GRACE` ago or not stamped at all) plus the stale harness roles.
///
/// The drops are force-free, so a database another process is connected to is
/// refused with [`SQLSTATE_OBJECT_IN_USE`] and skipped as benign (`PostgreSQL`
/// docs § DROP DATABASE). Every failure is logged and swallowed: the sweep is
/// hygiene, not correctness.
async fn sweep(admin: &mut PgConnection) -> SweepReport {
    let mut report = SweepReport {
        candidates: 0,
        dropped: 0,
        in_use: 0,
    };
    // Derived, not passed in: the sweep runs before the template is ensured.
    let current_template = format!("{DB_PREFIX}tpl_{}", db::migration_fingerprint());
    let now = now_secs();

    let listed: Result<Vec<(String, Option<String>)>, sqlx::Error> = sqlx::query_as(
        "SELECT datname, shobj_description(oid, 'pg_database') FROM pg_database \
         WHERE datname LIKE $1 AND NOT datistemplate",
    )
    .bind(format!("{DB_PREFIX}%"))
    .fetch_all(&mut *admin)
    .await;
    let databases = match listed {
        Ok(databases) => databases,
        Err(error) => {
            tracing::warn!("testkit sweep: cannot list harness databases: {error}");
            Vec::new()
        }
    };

    for (name, comment) in databases {
        let lease = lease_key(&name);
        let candidate = match lease {
            Some(key) => lease_free(admin, key).await,
            None => stale(&name, comment.as_deref(), &current_template, now),
        };
        if !candidate {
            continue;
        }
        report.candidates += 1;
        // No `WITH (FORCE)`: a clone with live sessions belongs to a running
        // test process.
        let sql = format!("DROP DATABASE IF EXISTS {name}");
        match sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut *admin)
            .await
        {
            Ok(_) => report.dropped += 1,
            Err(error) => {
                if refusal_is_benign(sqlstate(&error).as_deref()) {
                    report.in_use += 1;
                    tracing::debug!("testkit sweep: {name} is in use, skipped");
                } else {
                    tracing::warn!("testkit sweep: dropping {name} failed: {error}");
                }
            }
        }
        if let Some(key) = lease {
            drop(
                sqlx::query("SELECT pg_advisory_unlock($1, $2)")
                    .bind(CLONE_LEASE_CLASS)
                    .bind(key)
                    .execute(&mut *admin)
                    .await,
            );
        }
    }

    // Roles are cluster-global: tests name login roles `<clone-db>_<suffix>`
    // so the same staleness parse applies.
    let listed_roles: Result<Vec<String>, sqlx::Error> =
        sqlx::query_scalar("SELECT rolname FROM pg_roles WHERE rolname LIKE $1")
            .bind(format!("{DB_PREFIX}%"))
            .fetch_all(&mut *admin)
            .await;
    for role in listed_roles.unwrap_or_default() {
        if !stale(&role, None, &current_template, now) {
            continue;
        }
        let sql = format!("DROP ROLE IF EXISTS {role}");
        if let Err(error) = sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut *admin)
            .await
        {
            tracing::debug!("testkit sweep: dropping role {role} failed: {error}");
        }
    }

    report
}

/// Whether the lease `key` names is free, taking it when it is: the process
/// that made the clone has ended. The caller releases it after the drop.
async fn lease_free(admin: &mut PgConnection, key: i32) -> bool {
    let taken: Result<bool, sqlx::Error> =
        sqlx::query_scalar("SELECT pg_try_advisory_lock($1, $2)")
            .bind(CLONE_LEASE_CLASS)
            .bind(key)
            .fetch_one(&mut *admin)
            .await;
    match taken {
        Ok(taken) => taken,
        Err(error) => {
            tracing::warn!("testkit sweep: reading a clone lease failed: {error}");
            false
        }
    }
}

/// The `SQLSTATE` a failure carries, when it was reported by the server at all
/// (a transport or pool failure carries none).
fn sqlstate(error: &sqlx::Error) -> Option<String> {
    let code = sqlx::error::DatabaseError::code(error.as_database_error()?)?;
    Some(code.into_owned())
}

/// Whether a refused sweep drop is the benign "another test process is still
/// connected" outcome rather than a broken harness.
fn refusal_is_benign(code: Option<&str>) -> bool {
    code == Some(SQLSTATE_OBJECT_IN_USE)
}

/// Whether harness database `name`, carrying database comment `comment`, is
/// stale at `now_secs`.
///
/// A template other than `current_template` is stale unless its completion
/// stamp records a build time younger than `TEMPLATE_GRACE`; an unstamped
/// template, or one whose stamp carries no build time, has no known age and
/// is stale. A clone is stale when its name-embedded creation time is older
/// than [`SWEEP_GRACE`]. The live template and names outside the harness
/// prefix are never stale.
fn stale(name: &str, comment: Option<&str>, current_template: &str, now_secs: u64) -> bool {
    let Some(rest) = name.strip_prefix(DB_PREFIX) else {
        return false;
    };
    if rest.starts_with("tpl_") {
        if name == current_template {
            return false;
        }
        let built = comment
            .and_then(parse_stamp)
            .and_then(|stamp| stamp.built_secs);
        return built
            .is_none_or(|built| now_secs.saturating_sub(built) >= TEMPLATE_GRACE.as_secs());
    }
    let Some((secs_hex, _)) = rest.split_once('_') else {
        return false;
    };
    let Ok(created) = u64::from_str_radix(secs_hex, 16) else {
        return false;
    };
    now_secs.saturating_sub(created) >= SWEEP_GRACE.as_secs()
}

#[cfg(test)]
mod tests {
    use super::{
        DB_PREFIX, SQLSTATE_OBJECT_IN_USE, SWEEP_GRACE, TEMPLATE_GRACE, TemplateStamp,
        TestkitError, clone_url, fresh_name, ipv4_address, parse_stamp, probe_docker, redacted,
        refusal_is_benign, stale, template_stamp,
    };

    /// `localhost` resolves to `::1` before `127.0.0.1` on common hosts; the DSN
    /// must name the IPv4 loopback, never an IPv6 port nothing listens on.
    #[tokio::test]
    async fn the_server_address_is_ipv4_even_for_localhost() {
        let address = ipv4_address("localhost", 5432)
            .await
            .expect("localhost resolves");
        assert_eq!(address.to_string(), "127.0.0.1:5432");
        let literal = ipv4_address("127.0.0.1", 6543).await.expect("a literal");
        assert_eq!(literal.to_string(), "127.0.0.1:6543");
        assert!(matches!(
            ipv4_address("::1", 5432).await,
            Err(TestkitError::NoIpv4Host { .. })
        ));
    }

    #[test]
    fn clone_url_replaces_database_segment() {
        assert_eq!(
            clone_url("postgres://u:p@h:5432/postgres", "ferroehr_tk_x"),
            "postgres://u:p@h:5432/ferroehr_tk_x"
        );
    }

    #[test]
    fn fresh_names_are_unique_and_prefixed() {
        let a = fresh_name();
        let b = fresh_name();
        assert_ne!(a, b);
        assert!(a.starts_with(DB_PREFIX));
    }

    #[test]
    fn redaction_strips_credentials() {
        assert_eq!(redacted("postgres://u:secret@h:5432/db"), "h:5432/db");
    }

    #[test]
    fn stale_spares_current_template_and_young_clones() {
        let now = 0x1000_0000u64;
        let current = "ferroehr_tk_tpl_abcd";
        assert!(!stale(current, None, current, now));
        assert!(stale("ferroehr_tk_tpl_old0", None, current, now));
        let young = format!("{DB_PREFIX}{:x}_aaaa", now - 10);
        assert!(!stale(&young, None, current, now));
        let old = format!("{DB_PREFIX}{:x}_aaaa", now - SWEEP_GRACE.as_secs() - 1);
        assert!(stale(&old, None, current, now));
        assert!(!stale("unrelated_db", None, current, now));
    }

    /// The live template is excluded from the sweep at every age: it carries no
    /// creation time in its name and must survive arbitrarily long runs, since
    /// every clone is made from it.
    #[test]
    fn stale_never_sweeps_the_live_template() {
        let current = "ferroehr_tk_tpl_deadbeef";
        for now in [0u64, 0x1000_0000, u64::MAX] {
            assert!(
                !stale(current, None, current, now),
                "live template swept at {now}"
            );
        }
        // A template built from any OTHER migration fingerprint is stale.
        assert!(stale("ferroehr_tk_tpl_0", None, current, 0));
        assert!(stale("ferroehr_tk_tpl_feedface", None, current, 0));
    }

    /// Another fingerprint's template survives the sweep while its stamp is
    /// younger than the grace period, and is stale once it is older, or when
    /// it carries no stamp or a stamp without a build time.
    #[test]
    fn stale_spares_a_young_foreign_template() {
        let current = "ferroehr_tk_tpl_deadbeef";
        let foreign = "ferroehr_tk_tpl_feedface";
        let now = 0x1000_0000u64;
        let young = template_stamp("feedface", now - 10);
        assert!(!stale(foreign, Some(&young), current, now));
        let old = template_stamp("feedface", now - TEMPLATE_GRACE.as_secs());
        assert!(stale(foreign, Some(&old), current, now));
        assert!(stale(foreign, None, current, now));
        let undated = "ferroehr-testkit fingerprint=feedface complete";
        assert!(stale(foreign, Some(undated), current, now));
        assert!(stale(foreign, Some("not a stamp"), current, now));
    }

    /// The stamp round-trips, and the undated form still reads as complete.
    #[test]
    fn stamps_parse_back() {
        assert_eq!(
            parse_stamp(&template_stamp("abc", 42)),
            Some(TemplateStamp {
                fingerprint: "abc",
                built_secs: Some(42),
            })
        );
        assert_eq!(
            parse_stamp("ferroehr-testkit fingerprint=abc complete"),
            Some(TemplateStamp {
                fingerprint: "abc",
                built_secs: None,
            })
        );
        assert_eq!(parse_stamp("ferroehr-testkit fingerprint=abc"), None);
        assert_eq!(
            parse_stamp("ferroehr-testkit fingerprint=abc complete at=x"),
            None
        );
    }

    /// A clone name whose embedded creation time does not parse is never swept:
    /// an unknown age is not evidence of staleness.
    #[test]
    fn stale_spares_unparseable_clone_names() {
        let current = "ferroehr_tk_tpl_abcd";
        let now = 0x1000_0000u64;
        assert!(!stale("ferroehr_tk_notahexnumber_x", None, current, now));
        assert!(!stale("ferroehr_tk_nounderscore", None, current, now));
    }

    /// `PostgreSQL` refuses a force-free `DROP DATABASE` on a database with
    /// live sessions with SQLSTATE 55006 (`object_in_use`) — for the sweep that
    /// is a benign "another test process owns this clone", not a harness
    /// failure. Any other SQLSTATE, or none at all, is not benign.
    #[test]
    fn only_object_in_use_is_a_benign_sweep_refusal() {
        assert_eq!(SQLSTATE_OBJECT_IN_USE, "55006");
        assert!(refusal_is_benign(Some(SQLSTATE_OBJECT_IN_USE)));
        assert!(!refusal_is_benign(Some("55000"))); // object_not_in_prerequisite_state
        assert!(!refusal_is_benign(Some("42501"))); // insufficient_privilege
        assert!(!refusal_is_benign(Some("3D000"))); // invalid_catalog_name
        assert!(!refusal_is_benign(None)); // transport/pool failure
    }

    #[test]
    fn probe_trusts_a_tcp_docker_host_without_probing() {
        assert!(probe_docker(Some("tcp://127.0.0.1:2375")).is_ok());
    }

    #[test]
    fn probe_accepts_a_unix_docker_host_that_answers() {
        let dir = std::env::temp_dir().join(format!("testkit-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let sock = dir.join("live.sock");
        drop(std::fs::remove_file(&sock));
        let listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind scratch socket");
        let host = format!("unix://{}", sock.display());
        assert!(probe_docker(Some(&host)).is_ok());
        drop(listener);
        drop(std::fs::remove_file(&sock));
    }

    #[test]
    fn probe_names_a_dead_unix_docker_host() {
        let err = probe_docker(Some("unix:///nonexistent/testkit-probe.sock"))
            .expect_err("a dead override must fail fast");
        let text = err.to_string();
        assert!(text.contains("DOCKER_HOST"), "{text}");
        assert!(text.contains("FERROEHR_TEST_PG_URL"), "{text}");
    }
}
