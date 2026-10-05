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
//! fingerprint under a `PostgreSQL` advisory lock and stamped complete as its
//! database comment, read via `shobj_description` because a connection to a
//! template blocks cloning (`PostgreSQL` docs § CREATE DATABASE). Clones are
//! named `ferroehr_tk_<secs>_<rand>` and swept by [`sweep_stale`], which every
//! test process runs once before it hands out a database.

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
pub mod json_literals;

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

/// Clones younger than this grace window are never swept.
///
/// A running test's clone can momentarily have no session on it (the pools run
/// `min_connections = 0`), so connectedness cannot tell live from leaked; the
/// name-embedded creation time can.
const SWEEP_GRACE: Duration = Duration::from_mins(30);

/// Failures of the harness itself (server acquisition, template build,
/// cloning) — always broken test infrastructure, never application behaviour.
#[derive(Debug, thiserror::Error)]
pub enum TestkitError {
    /// Starting or adopting the reusable `PostgreSQL` container failed.
    #[error("testkit: container: {0}")]
    Container(#[from] testcontainers::TestcontainersError),
    /// A query/connection against the server failed.
    #[error("testkit: database: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// Pool construction or migration via the platform library failed.
    #[error("testkit: platform db: {0}")]
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
}

/// One fresh, fully migrated database for one test: a clone of the migrated
/// template.
///
/// Dropping the guard best-effort-drops the database; the init sweep reclaims
/// anything a killed process left behind.
#[derive(Debug)]
pub struct TestDb {
    pool: PgPool,
    url: String,
    name: String,
    admin_url: String,
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
        // Best-effort: `Drop` cannot await and the process may exit first; the
        // init sweep is the durable backstop.
        //
        // NOTE: `WITH (FORCE)` is correct here and wrong in the sweep — it
        // terminates our own clone's connections, never another process's.
        let admin_url = self.admin_url.clone();
        let name = self.name.clone();
        drop(std::thread::Builder::new().spawn(move || {
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
        }));
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
async fn provision(server: &str, template: Option<&str>) -> Result<TestDb, TestkitError> {
    const POOL_DEADLINE: Duration = Duration::from_secs(15);
    let name = fresh_name();
    let mut admin = connect_ready(server).await?;
    if let Some(template) = template {
        create_clone(&mut admin, &name, template).await?;
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
    Ok(TestDb {
        pool,
        url,
        name,
        admin_url: server.to_owned(),
    })
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
    Ok(format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
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
    let stamp = format!("ferroehr-testkit fingerprint={fingerprint} complete");

    let mut admin = connect_ready(admin_url).await?;

    if template_complete(&mut admin, &template, &stamp).await? {
        drop(admin.close().await);
        return Ok(template);
    }

    // The lock is session-scoped: hold this connection until the stamp lands.
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(TEMPLATE_LOCK_KEY)
        .execute(&mut admin)
        .await?;
    let outcome = build_template(&mut admin, admin_url, &template, &stamp).await;
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

/// Whether `template` exists and carries the completion stamp.
async fn template_complete(
    admin: &mut PgConnection,
    template: &str,
    stamp: &str,
) -> Result<bool, TestkitError> {
    // `shobj_description` is NULL for a template with no comment yet, so the
    // scalar is doubly optional: NULL means "not complete", never an error.
    let complete: Option<Option<bool>> = sqlx::query_scalar(
        "SELECT shobj_description(oid, 'pg_database') = $2 \
         FROM pg_database WHERE datname = $1",
    )
    .bind(template)
    .bind(stamp)
    .fetch_optional(admin)
    .await?;
    Ok(complete.flatten() == Some(true))
}

/// Builds the template under the already-held advisory lock: re-check, drop
/// any incomplete carcass, create, migrate, then stamp the database comment.
async fn build_template(
    admin: &mut PgConnection,
    admin_url: &str,
    template: &str,
    stamp: &str,
) -> Result<(), TestkitError> {
    if template_complete(admin, template, stamp).await? {
        return Ok(());
    }
    let drop_sql = format!("DROP DATABASE IF EXISTS {template} WITH (FORCE)");
    sqlx::raw_sql(sqlx::AssertSqlSafe(drop_sql))
        .execute(&mut *admin)
        .await?;
    let create_sql = format!("CREATE DATABASE {template}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(create_sql))
        .execute(&mut *admin)
        .await?;

    // Close the pool fully: a lingering connection to the template blocks
    // every clone.
    let template_url = clone_url(admin_url, template);
    let pool = db::connect(&DbConfig::new(template_url)).await?;
    let migrated = db::run_migrations(&pool).await;
    pool.close().await;
    migrated?;

    let comment_sql = format!("COMMENT ON DATABASE {template} IS '{stamp}'");
    sqlx::raw_sql(sqlx::AssertSqlSafe(comment_sql))
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

/// Clones the template, retrying the transient lock conflict concurrent clones
/// of one template can hit.
async fn create_clone(
    admin: &mut PgConnection,
    name: &str,
    template: &str,
) -> Result<(), TestkitError> {
    const DEADLINE: Duration = Duration::from_mins(1);
    let start = Instant::now();
    loop {
        // Default WAL_LOG strategy: no forced checkpoint per clone
        // (PostgreSQL docs § CREATE DATABASE, `STRATEGY`).
        let sql = format!("CREATE DATABASE {name} TEMPLATE {template}");
        match sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut *admin)
            .await
        {
            Ok(_) => return Ok(()),
            Err(error) => {
                if start.elapsed() >= DEADLINE {
                    return Err(TestkitError::Sqlx(error));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
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
///
/// # Errors
///
/// Returns [`TestkitError`] when the shared server cannot be acquired or
/// connected to. Individual drops never fail the call — an in-use database is
/// counted as [`SweepReport::in_use`] and any other failure is logged.
pub async fn sweep_stale() -> Result<SweepReport, TestkitError> {
    let server = server().await?;
    let mut admin = connect_ready(server).await?;
    let report = sweep(&mut admin).await;
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

/// Drops every stale harness database — clones older than [`SWEEP_GRACE`], and
/// templates other than the current migration fingerprint's — plus the stale
/// harness roles.
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

    let listed: Result<Vec<String>, sqlx::Error> = sqlx::query_scalar(
        "SELECT datname FROM pg_database WHERE datname LIKE $1 AND NOT datistemplate",
    )
    .bind(format!("{DB_PREFIX}%"))
    .fetch_all(&mut *admin)
    .await;
    let names = match listed {
        Ok(names) => names,
        Err(error) => {
            tracing::warn!("testkit sweep: cannot list harness databases: {error}");
            Vec::new()
        }
    };

    for name in names {
        if !stale(&name, &current_template, now) {
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
    }

    // Roles are cluster-global: tests name login roles `<clone-db>_<suffix>`
    // so the same staleness parse applies.
    let listed_roles: Result<Vec<String>, sqlx::Error> =
        sqlx::query_scalar("SELECT rolname FROM pg_roles WHERE rolname LIKE $1")
            .bind(format!("{DB_PREFIX}%"))
            .fetch_all(&mut *admin)
            .await;
    for role in listed_roles.unwrap_or_default() {
        if !stale(&role, &current_template, now) {
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

/// Whether a harness database is stale: an outdated template (any
/// `…tpl_…` other than the current one) or a clone whose name-embedded
/// creation time is older than [`SWEEP_GRACE`].
fn stale(name: &str, current_template: &str, now_secs: u64) -> bool {
    let Some(rest) = name.strip_prefix(DB_PREFIX) else {
        return false;
    };
    if let Some(_fingerprint) = rest.strip_prefix("tpl_") {
        return name != current_template;
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
        DB_PREFIX, SQLSTATE_OBJECT_IN_USE, SWEEP_GRACE, clone_url, fresh_name, probe_docker,
        redacted, refusal_is_benign, stale,
    };

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
        assert!(!stale(current, current, now));
        assert!(stale("ferroehr_tk_tpl_old0", current, now));
        let young = format!("{DB_PREFIX}{:x}_aaaa", now - 10);
        assert!(!stale(&young, current, now));
        let old = format!("{DB_PREFIX}{:x}_aaaa", now - SWEEP_GRACE.as_secs() - 1);
        assert!(stale(&old, current, now));
        assert!(!stale("unrelated_db", current, now));
    }

    /// The live template is excluded from the sweep at every age: it carries no
    /// creation time in its name and must survive arbitrarily long runs, since
    /// every clone is made from it.
    #[test]
    fn stale_never_sweeps_the_live_template() {
        let current = "ferroehr_tk_tpl_deadbeef";
        for now in [0u64, 0x1000_0000, u64::MAX] {
            assert!(
                !stale(current, current, now),
                "live template swept at {now}"
            );
        }
        // A template built from any OTHER migration fingerprint is stale.
        assert!(stale("ferroehr_tk_tpl_0", current, 0));
        assert!(stale("ferroehr_tk_tpl_feedface", current, 0));
    }

    /// A clone name whose embedded creation time does not parse is never swept:
    /// an unknown age is not evidence of staleness.
    #[test]
    fn stale_spares_unparseable_clone_names() {
        let current = "ferroehr_tk_tpl_abcd";
        let now = 0x1000_0000u64;
        assert!(!stale("ferroehr_tk_notahexnumber_x", current, now));
        assert!(!stale("ferroehr_tk_nounderscore", current, now));
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
