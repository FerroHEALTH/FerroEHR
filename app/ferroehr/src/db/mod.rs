// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `PostgreSQL` bootstrap: connection settings, pool construction, and the
//! per-schema migration sequence.
//!
//! No openEHR spec governs the persistence mechanism; the storage substrate is
//! our own PG18-native design. This module is the single place the rest of the
//! crate obtains a database handle. [`DbConfig`] (the `[db]` config section)
//! carries the shared DSN and the pool tuning; [`domain::StorageConfig`] (the
//! `[storage]` section) carries one DSN per storage domain, each defaulting to
//! the shared one. [`connect_domains`] opens the four pools
//! ([`domain::DomainPools`]) and [`prepare`] brings each database they reach to
//! the state this build requires — on the migration DSN
//! ([`DbConfig::migrate_dsn`]) for every domain that reaches the same database
//! it does, a credential a deployment may name because preparation spans every
//! schema of a database while each runtime credential holds one domain. The domains differ in the
//! `search_path` their connections carry, so one set of storage functions
//! serves them all. [`verify_domain_isolation`] is the boot gate that refuses
//! to serve when the runtime roles can read across those boundaries, when two
//! separately-configured domains turn out to authenticate as one role, or —
//! under `deployment_profile = "production"` — when a domain role is missing
//! altogether. The `sea-query` identifier vocabulary for the live schema lives
//! in [`iden`]. This is the defining module for the whole bootstrap surface,
//! with no re-exports.

pub mod acquire;
pub mod domain;
pub mod iden;

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};

use crate::config::deployment::DeploymentProfile;
use crate::db::acquire::acquire;
use crate::db::domain::{Domain, DomainLayout, DomainPools, StorageConfig};
use ferrohealth_secret::secret_url::SecretUrl;

// ── Settings — the `[db]` config section ─────────────────────────────────────

/// The zero-config dev DSN (matches the compose dev stack). Production MUST
/// override it; the boot path warns prominently when
/// [`DbConfig::is_dev_default`] holds.
pub const DEFAULT_URL: &str = "postgres://ferroehr:ferroehr@localhost:5432/ferroehr";

/// What the server does about schema migrations when it boots.
///
/// Migrations are DDL, so a self-migrating server must authenticate as a role
/// that can execute DDL — and a role that can rewrite the schema is a role an
/// application-level SQL flaw can rewrite the schema with. This setting is how
/// a deployment opts out of that: the schema is applied out of band by the
/// migrator role, and the server connects as the DML-only `ferroehr_clinical` role.
///
/// No openEHR spec governs migration mechanics or database roles — our own
/// design/extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MigrationMode {
    /// Apply the embedded migrations at boot, then verify (the default, and
    /// what makes an empty configuration boot against an empty database).
    #[default]
    Apply,
    /// Never issue DDL: verify that the database already carries exactly this
    /// build's migrations, and refuse to serve when it does not.
    Verify,
}

/// Connection settings for the application `PostgreSQL` database — the `[db]`
/// section of the one config tree ([`crate::config::FerroEhrConfig`]), with no
/// loader of its own.
///
/// No openEHR spec governs persistence — our own design. The DSN is a
/// [`SecretUrl`]: its embedded credentials are redacted from every rendering
/// (`Debug`, `/management/env`, `config check`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DbConfig {
    /// `PostgreSQL` connection DSN (`postgres://user:pass@host:port/db`).
    /// Credentials are redacted from every rendering ([`SecretUrl`]).
    pub url: SecretUrl,
    /// Path to a file holding the DSN, read at boot in place of [`Self::url`].
    ///
    /// The route for a deployment that mounts its database credential as a file
    /// rather than passing it as an environment value, which is readable through
    /// `/proc/<pid>/environ` and inherited by every child process. Setting both
    /// this and a non-default `url` is a boot error.
    pub url_file: Option<PathBuf>,
    /// DSN that schema preparation ([`prepare`]) authenticates as; unset (the
    /// default) reuses [`Self::url`].
    ///
    /// Preparing the schema spans EVERY schema, whatever [`Self::migrate`]
    /// says: `apply` issues the DDL of all five migration sets, and `verify`
    /// reads all five `_sqlx_migrations` bookkeeping tables. A runtime
    /// credential that holds one pseudonymisation domain can do neither, so
    /// under the separated-role posture this key is what names the credential
    /// that can, while the pools stay narrow. It is used for that one boot
    /// step on a connection that is closed again — no pool is ever held on it,
    /// and no request is ever served through it. No openEHR spec governs
    /// migration mechanics or database roles — our own design/extension.
    pub migrate_url: Option<SecretUrl>,
    /// Path to a file holding [`Self::migrate_url`], read at boot in place of
    /// it — the mounted-secret route, as [`Self::url_file`] is for the
    /// clinical DSN. Setting both is a boot error.
    pub migrate_url_file: Option<PathBuf>,
    /// Upper bound of the connection pool.
    pub max_connections: u32,
    /// Connections the pool keeps open when idle (avoids cold reopen +
    /// `SET search_path` churn under variable load).
    pub min_connections: u32,
    /// Seconds to wait for a free connection before failing.
    pub acquire_timeout_secs: u64,
    /// `statement_timeout` applied to every pooled connection, in
    /// milliseconds; `0` leaves the server default (usually unlimited).
    ///
    /// This is the backstop the request timeout cannot be. A request over the
    /// HTTP timeout is answered `408` by dropping the handler future, which does
    /// not cancel the statement PostgreSQL is running
    /// (<https://www.postgresql.org/docs/18/runtime-config-client.html>), so
    /// without this a handful of expensive queries can hold every pooled
    /// connection while their clients have already been given up on.
    ///
    /// Set ABOVE the AQL engine's own budget
    /// ([`crate::service::query::config::QueryConfig::timeout_ms`]) so the
    /// engine's typed refusal fires first and this only catches what the engine
    /// does not govern. No openEHR spec governs it — our own design.
    pub statement_timeout_ms: u64,
    /// Whether the server applies its embedded migrations at boot
    /// ([`MigrationMode`]).
    ///
    /// `apply` (the default) keeps a fresh checkout and a fresh database
    /// working with no configuration at all. `verify` is the least-privilege
    /// production posture: the DSN may then authenticate as a role with no DDL
    /// rights, and the server refuses to boot against a database that is not
    /// already migrated to exactly this build.
    pub migrate: MigrationMode,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            url: SecretUrl::new(DEFAULT_URL),
            url_file: None,
            migrate_url: None,
            migrate_url_file: None,
            // Deliberate defaults: 20 max (10 hard-capped realistic write
            // concurrency ×2), 2 min (no cold reopen churn at idle).
            max_connections: 20,
            min_connections: 2,
            acquire_timeout_secs: 30,
            // Twice the engine's own 30 s budget, so the engine refuses first
            // and this remains a backstop rather than the primary control.
            statement_timeout_ms: 60_000,
            migrate: MigrationMode::Apply,
        }
    }
}

impl DbConfig {
    /// Settings for `url` with defaults for everything else.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: SecretUrl::new(url.into()),
            ..Self::default()
        }
    }

    /// Whether the DSN is the built-in dev default (no operator override). The
    /// boot path logs a prominent warning in this case so a production
    /// deployment never silently runs against the dev database.
    #[must_use]
    pub fn is_dev_default(&self) -> bool {
        self.url.expose() == DEFAULT_URL
    }

    /// The DSN schema preparation connects with: [`Self::migrate_url`] when
    /// the deployment names a credential for it, else [`Self::url`].
    #[must_use]
    pub fn migrate_dsn(&self) -> &str {
        self.migrate_url
            .as_ref()
            .map_or_else(|| self.url.expose(), SecretUrl::expose)
    }

    /// Whether schema preparation authenticates as its own database role
    /// (a distinct DSN), rather than as the clinical runtime one.
    #[must_use]
    pub fn migrator_is_separated(&self) -> bool {
        self.migrate_url.is_some()
    }
}

// ── Error ────────────────────────────────────────────────────────────────────

/// Errors produced by the persistence foundation.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// A driver/pool/query error from `sqlx`, carried as the source and kept
    /// out of this variant's own message.
    #[error("database operation failed")]
    Sqlx(#[from] sqlx::Error),

    /// No connection could be obtained from the pool; the pool's error is the
    /// source.
    #[error(transparent)]
    Acquire(#[from] acquire::AcquireError),

    /// A schema migration failed to apply; the migrator's error is the source.
    #[error("applying the schema migrations failed")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    /// The database does not carry exactly this build's migrations, and
    /// [`MigrationMode::Verify`] forbids applying them; the divergence found is
    /// the source.
    #[error(
        "the database is not migrated to this build. \
         [db].migrate is `verify`, so this server will not issue DDL: apply the \
         migrations out of band with the migrator role, then start it again"
    )]
    SchemaNotReady(#[source] SchemaMismatch),

    /// The credential preparing the schema cannot read a migration set's
    /// bookkeeping at all, so the migration state is unknown rather than
    /// wrong.
    #[error(
        "database role `{role}` cannot read the migration state of schema `{schema}`: \
         preparing the schema reads every schema's `_sqlx_migrations` bookkeeping table, and \
         this credential holds no privilege on `{schema}`. Under the separated-domain posture \
         no runtime credential ever can, because each holds one domain: point `[db] \
         migrate_url` (or `migrate_url_file`) at the credential that prepares the schema — \
         unset, it falls back to `[db] url` — and, for a domain in a database of its own, \
         `[storage.migrate.<domain>] url` at that database's migration credential"
    )]
    SchemaUnreadable {
        /// The schema whose migration bookkeeping could not be read.
        schema: String,
        /// The database role the refused connection authenticates as.
        role: String,
        /// `PostgreSQL`'s own refusal (`SQLSTATE` 42501).
        #[source]
        source: sqlx::Error,
    },

    /// The database was created by a release older than the storage rewrite.
    #[error(
        "this database predates the storage rewrite: schema `{schema}` carries its own \
         migration bookkeeping, which only a release before the rewrite wrote. The storage \
         schema was rewritten and the new migration sets replace the old ones outright, so \
         there is nothing to upgrade in place: this server will not create its schemas beside \
         the old ones and serve an empty repository while the existing content sits \
         unreachable in the same database. Dump anything worth keeping, recreate the \
         database, and start this server against it"
    )]
    FirstGenerationDatabase {
        /// The first-generation schema whose migration bookkeeping was found.
        schema: &'static str,
    },

    /// A runtime role can read a relation belonging to a pseudonymisation
    /// domain it does not own.
    #[error(
        "the pseudonymisation boundary is not enforced by the database: role `{role}` can reach \
         the {kind} `{relation}`, which belongs to another domain. The clinical record, the \
         identity of its subject, and the map between them must not be reachable by one \
         credential (GDPR Art. 4(5) and \
         Art. 32(1)(a); no openEHR spec governs database roles — our own design). Remedy: \
         `REVOKE ALL ON {relation} FROM {role}` — and, for a function, `REVOKE EXECUTE ON \
         FUNCTION {relation} FROM PUBLIC`, since PUBLIC holds EXECUTE by default \
         (https://www.postgresql.org/docs/18/sql-grant.html)"
    )]
    DomainIsolationBreached {
        /// The runtime role holding the privilege it must not hold.
        role: String,
        /// What kind of object it reaches (`table`, `view`, `sequence`, …).
        kind: String,
        /// The schema-qualified object it reaches.
        relation: String,
    },

    /// Two domains a deployment placed on different DSNs authenticate as one
    /// database role, so the separation it configured does not exist.
    #[error(
        "the `{domain}` and `{other}` domains are configured on separate DSNs but both \
         authenticate as database role `{role}`, so one credential still holds both domains and \
         the separation is only apparent. Give each domain a login role of its own — a member of \
         that domain's runtime role and of no other (GDPR Art. 4(5) and Art. 32(1)(a); no openEHR \
         spec governs database roles — our own design)"
    )]
    DomainRoleShared {
        /// One of the two domains.
        domain: Domain,
        /// The other.
        other: Domain,
        /// The role both sessions authenticate as.
        role: String,
    },

    /// A domain's runtime role does not exist, under a profile that requires
    /// the grants to be real.
    #[error(
        "database role `{role}` does not exist, so the `{domain}` domain's grants separate \
         nothing and this check has nothing to measure. deployment_profile = \"production\" \
         requires the runtime roles to exist: the migrations create them only when the migrator \
         holds CREATEROLE, so provision them (CREATE ROLE {role} NOLOGIN NOINHERIT) and grant \
         each pool's login role membership in exactly one of them"
    )]
    DomainRoleMissing {
        /// The absent role.
        role: String,
        /// The domain it serves.
        domain: Domain,
    },

    /// A runtime role can write a migration bookkeeping table.
    #[error(
        "database role `{role}` can write the migration bookkeeping table `{relation}`, which \
         `[db] migrate = \"verify\"` compares against this build's migrations: a runtime \
         credential able to rewrite it can make an unmigrated or altered database pass that \
         check. Remedy: `REVOKE ALL ON {relation} FROM {role}; GRANT SELECT ON {relation} TO \
         {role}` (no openEHR spec governs database roles — our own design)"
    )]
    BookkeepingWritable {
        /// The runtime role holding a write privilege on the bookkeeping.
        role: String,
        /// The schema-qualified bookkeeping table.
        relation: String,
    },

    /// A domain's `[storage.migrate.<domain>]` credential reaches a different
    /// database from the one the domain is served from.
    #[error(
        "`[storage.migrate.{domain}] url` reaches a different database from `[storage.{domain}] \
         url`: the migration credential prepares the database the {domain} domain lives in, so it \
         must reach the same one. Point both at that database"
    )]
    DomainMigratorElsewhere {
        /// The domain whose two credentials disagree.
        domain: Domain,
    },
}

/// How a database's recorded migration state differs from the one this binary
/// embeds.
///
/// Each variant names a distinct operational situation, so a caller can branch
/// on it rather than match a message: an unmigrated database, a database behind
/// the binary, a database ahead of it, and a database whose bookkeeping is
/// damaged.
#[derive(Debug, thiserror::Error)]
pub enum SchemaMismatch {
    /// The schema carries no `_sqlx_migrations` table at all.
    #[error("schema `{schema}` has never been migrated")]
    NeverMigrated {
        /// The `PostgreSQL` schema that carries the migration set.
        schema: String,
    },

    /// Migrations this binary embeds are absent from the database.
    #[error(
        "schema `{schema}` is missing migration(s) {versions:?} — the database is older than this build"
    )]
    Missing {
        /// The `PostgreSQL` schema that carries the migration set.
        schema: String,
        /// The absent migration versions, ascending.
        versions: Vec<i64>,
    },

    /// A migration is recorded as having failed partway through.
    #[error("schema `{schema}` records migration {version} as failed")]
    Failed {
        /// The `PostgreSQL` schema that carries the migration set.
        schema: String,
        /// The failed migration's version.
        version: i64,
    },

    /// A migration was applied from source text this binary does not carry.
    #[error("schema `{schema}` applied migration {version} from different source text")]
    ChecksumMismatch {
        /// The `PostgreSQL` schema that carries the migration set.
        schema: String,
        /// The diverging migration's version.
        version: i64,
    },

    /// The database carries migrations this binary does not know about.
    #[error(
        "schema `{schema}` carries migration(s) {versions:?} — the database is newer than this build"
    )]
    Ahead {
        /// The `PostgreSQL` schema that carries the migration set.
        schema: String,
        /// The unknown migration versions, ascending.
        versions: Vec<i64>,
    },
}

// ── Pool ─────────────────────────────────────────────────────────────────────

/// Everything a freshly-opened physical connection needs before it serves a
/// query: the domain's search path and the statement-timeout backstop.
///
/// One implementation for every pool, so a domain cannot drop a setting.
/// Dropping the timeout in particular silently disarms the DB-side
/// runaway-query guard, and a broken control must never look like a policy
/// outcome.
async fn open_session(
    conn: &mut PgConnection,
    search_path: &'static str,
    statement_timeout: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(search_path).execute(&mut *conn).await?;
    if let Some(statement_timeout) = statement_timeout {
        // A session-level SET on the physical connection, surviving every
        // checkout, where `SET LOCAL` would last one transaction.
        // `AssertSqlSafe` is audited: PostgreSQL's `SET` takes no bind
        // placeholder and the value is a `u64` from our own configuration,
        // never client input.
        sqlx::query(sqlx::AssertSqlSafe(statement_timeout.to_owned()))
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// The pool options for one domain: sizing + acquire timeout from settings,
/// the domain's session setup on every physical connection, and no per-checkout
/// liveness ping. Connection retirement stays on the `sqlx` defaults (an idle
/// reap plus a bounded lifetime — infinite-lived connections are discouraged by
/// the driver, so we do not disable them).
fn pool_options(settings: &DbConfig, domain: Domain) -> PgPoolOptions {
    // Rendered once here rather than per connection. The value is an integer
    // from our own configuration, never client input, and it is bound as a
    // literal because PostgreSQL's `SET` takes no parameter placeholder.
    let statement_timeout = (settings.statement_timeout_ms > 0)
        .then(|| format!("SET statement_timeout = {}", settings.statement_timeout_ms));
    let search_path = domain.search_path();
    PgPoolOptions::new()
        .max_connections(settings.max_connections)
        .min_connections(settings.min_connections)
        .acquire_timeout(Duration::from_secs(settings.acquire_timeout_secs))
        // No liveness ping per checkout: `test_before_acquire` (default true)
        // pings each connection before handing it out, one round trip per
        // acquisition (https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.PoolOptions.html#method.test_before_acquire).
        // A connection that broke while idle therefore fails its first real
        // statement, and that statement's error reaches the caller: sqlx does
        // not retry it.
        .test_before_acquire(false)
        .after_connect(move |conn, _meta| {
            // Cloned per call: `after_connect` takes an `Fn`, so the captured
            // value cannot be moved out of it.
            let statement_timeout = statement_timeout.clone();
            Box::pin(
                async move { open_session(conn, search_path, statement_timeout.as_deref()).await },
            )
        })
}

/// Create one domain's connection pool.
///
/// Every physical connection is initialized with that domain's search path
/// ([`Domain::search_path`]) so queries can use unqualified table names, as the
/// schema expects, and with no other domain's schema on it — a query issued
/// here against another domain's relation fails to resolve rather than quietly
/// crossing the pseudonymisation boundary. The DSN is
/// `[storage.<domain>].url` when the deployment gives the domain one, and
/// `[db].url` otherwise: the schema separation is unconditional, the credential
/// separation is the deployment's choice.
///
/// # Errors
///
/// Returns [`DbError::Sqlx`] when the DSN does not parse as a `PostgreSQL`
/// URL, the initial connection fails (unreachable host, refused
/// authentication, unknown database), or the session-setup statements fail on
/// that first connection.
pub async fn connect_domain(
    settings: &DbConfig,
    storage: &StorageConfig,
    domain: Domain,
) -> Result<PgPool, DbError> {
    let pool = pool_options(settings, domain)
        .connect(storage.dsn(domain, settings))
        .await?;
    Ok(pool)
}

/// Create the clinical pool on the shared `[db].url`, for a caller that holds
/// only a [`DbConfig`].
///
/// [`connect_domain`] over a default [`StorageConfig`] — the co-located
/// deployment's clinical pool. The test harness and the operator checks use it;
/// the serving path uses [`connect_domains`], which honours every
/// `[storage.<domain>]` DSN.
///
/// # Errors
///
/// Whatever [`connect_domain`] returns.
pub async fn connect(settings: &DbConfig) -> Result<PgPool, DbError> {
    connect_domain(settings, &StorageConfig::default(), Domain::Clinical).await
}

/// Create all four domain pools.
///
/// The boot path's one call: every domain gets its own pool, its own
/// `search_path` and — where the deployment named one — its own credential.
///
/// # Errors
///
/// Whatever [`connect_domain`] returns, for the first domain that cannot
/// connect.
pub async fn connect_domains(
    settings: &DbConfig,
    storage: &StorageConfig,
) -> Result<DomainPools, DbError> {
    Ok(DomainPools {
        clinical: connect_domain(settings, storage, Domain::Clinical).await?,
        party: connect_domain(settings, storage, Domain::Party).await?,
        linkage: connect_domain(settings, storage, Domain::Linkage).await?,
        audit: connect_domain(settings, storage, Domain::Audit).await?,
    })
}

/// A pool for `domain` over the DSN an existing pool already holds, for a
/// caller that has a [`PgPool`] and no configuration.
///
/// This is what lets [`crate::service::FerroEhrService::new`] stay synchronous
/// and infallible while still routing each chapter at its own schema:
/// `PgPool::connect_options` hands back the connect options the pool was built
/// from, and `PgPoolOptions::connect_lazy_with` builds a pool from them with no
/// I/O at all.
///
/// It carries the pool defaults rather than the deployment's `[db]` tuning,
/// which it cannot see, and it opens no connection until one is asked for
/// (`min_connections(0)`, so constructing a service costs nothing). A
/// deployment that tunes the pool or separates the runtime roles supplies its
/// own pools through [`crate::service::FerroEhrService::with_demographic_pool`]
/// and its siblings instead.
#[must_use]
pub fn domain_pool_from(pool: &PgPool, domain: Domain) -> PgPool {
    let defaults = DbConfig::default();
    let options = pool.connect_options();
    pool_options(&defaults, domain)
        .min_connections(0)
        .connect_lazy_with(PgConnectOptions::clone(&options))
}

// ── Migrations ───────────────────────────────────────────────────────────────

/// The `ext` schema: the runtime roles, our openEHR support functions
/// (`openehr_magnitude` and its ISO-8601 helpers) and the deployment posture.
/// Runs first, so every later set finds its roles and helpers in place.
static EXT_MIGRATOR: Migrator = sqlx::migrate!("migrations/ext");

/// The `clinical` schema — the clinical pseudonymisation domain: the EHR-owned
/// versioned objects, the append-only `version` store with its `vo_head`, the
/// decomposed `node` table, and the supporting relations. No openEHR spec
/// governs the physical schema; the change-control semantics it realizes are
/// RM common `master06-change_control_package.adoc`.
static CLINICAL_MIGRATOR: Migrator = sqlx::migrate!("migrations/clinical");

/// The `party` schema — the demographic pseudonymisation domain: PARTY
/// versioned objects and their change control, physically separated from the
/// clinical schema so no runtime role reads both (GDPR Art. 4(5) and
/// Art. 32(1)(a); no openEHR spec governs storage layout — our own design).
/// Its change-control and node relations are rendered from the same DDL
/// template as the clinical ones, so the two cannot drift. Runs after
/// `clinical`, because its grants revoke each domain's roles from the other's
/// schema and both must exist.
static PARTY_MIGRATOR: Migrator = sqlx::migrate!("migrations/party");

/// The `linkage` schema — the linkage pseudonymisation domain: the map from a
/// party to the EHR whose subject it is, the additional information that
/// re-joins a pseudonymised record to a person (GDPR Art. 4(5) and
/// Art. 32(1)(a); no openEHR spec governs storage layout — our own design).
/// Runs after `party`: its grants revoke the clinical and party roles from
/// this schema and this schema's role from theirs, so both sets of relations
/// must already exist.
static LINKAGE_MIGRATOR: Migrator = sqlx::migrate!("migrations/linkage");

/// The `audit` schema — the local IHE ATNA Audit Record Repository (the
/// `audit_event` table and its tamper chain). Strictly outside the EHR content
/// (BASE `architecture_overview/master07-security.adoc` §Access logging:
/// in-system access logs, never part of the EHR proper).
static AUDIT_MIGRATOR: Migrator = sqlx::migrate!("migrations/audit");

/// The migrator for one domain's set, paired with the schema that carries its
/// `_sqlx_migrations` bookkeeping table.
const fn domain_migrator(domain: Domain) -> (&'static str, &'static Migrator) {
    match domain {
        Domain::Clinical => ("clinical", &CLINICAL_MIGRATOR),
        Domain::Party => ("party", &PARTY_MIGRATOR),
        Domain::Linkage => ("linkage", &LINKAGE_MIGRATOR),
        Domain::Audit => ("audit", &AUDIT_MIGRATOR),
    }
}

/// One schema of the first storage generation, and how to recognise its
/// bookkeeping.
#[derive(Debug, Clone, Copy)]
struct FirstGenerationSet {
    /// The schema the first generation's migration set ran in.
    schema: &'static str,
    /// The description sqlx recorded for that set's version 1, when this build
    /// also owns a set of the same name; `None` when the schema itself is
    /// first-generation and any bookkeeping in it is the signature.
    ///
    /// sqlx derives the description from the file name after the version
    /// number, with underscores replaced by spaces, so `0001_baseline.sql`
    /// records `baseline`.
    first_description: Option<&'static str>,
}

/// The first storage generation's migration sets, which this build does not
/// migrate and cannot read.
///
/// The rewrite is greenfield: the new sets replace the old ones outright,
/// nothing is upgraded in place, and a database created by an earlier release
/// is refused at boot rather than half-adopted.
///
/// Three of the five schema NAMES survive the rewrite (`ext`, `linkage`,
/// `audit`), so for those the signature cannot be the bookkeeping's existence —
/// it is which migration ran FIRST. Version 1 is the one row a set can never
/// lack, and its description names the file: the first generation opened `ext`
/// with `0001_openehr_functions.sql` where this one opens it with
/// `0001_schema_and_roles.sql`, and opened `linkage`/`audit` with
/// `0001_baseline.sql` where this one opens them with `0001_schema_and_role`
/// and `0001_schema_and_roles`. For `ehr` and `demographic`, which this build
/// owns no set for, any bookkeeping at all is the signature — a bare schema of
/// the same name is not, because `CREATE SCHEMA` is cheap and someone may have
/// made one, while the bookkeeping table is written only by a migrator that ran
/// there.
///
/// Without this, a first-generation `ext`, `linkage` or `audit` set would reach
/// its own migrator and fail on a checksum mismatch — an error about a hash
/// where the operator needs the remedy.
const FIRST_GENERATION_SETS: &[FirstGenerationSet] = &[
    FirstGenerationSet {
        schema: "ehr",
        first_description: None,
    },
    FirstGenerationSet {
        schema: "demographic",
        first_description: None,
    },
    FirstGenerationSet {
        schema: "ext",
        first_description: Some("openehr functions"),
    },
    FirstGenerationSet {
        schema: "linkage",
        first_description: Some("baseline"),
    },
    FirstGenerationSet {
        schema: "audit",
        first_description: Some("baseline"),
    },
];

/// The extension the schema needs, installed by the bootstrap.
///
/// `btree_gist` is the only one: it backs `linkage.subject_ehr`'s temporal
/// `UNIQUE (... WITHOUT OVERLAPS)`, which PostgreSQL enforces as a `GiST` index
/// over its operator classes
/// (<https://www.postgresql.org/docs/18/sql-createtable.html>), and installing
/// it here is what lets a plain PostgreSQL 18 database serve with nothing
/// preinstalled.
const BOOTSTRAP_EXTENSION: &str = "CREATE EXTENSION IF NOT EXISTS btree_gist WITH SCHEMA ext";

/// The schemas a database's preparation creates, in [`Domain::ALL`] order:
/// each set's own, and each schema a set's statements name
/// ([`Domain::names_schemas_of`]).
fn bootstrap_schemas(sets: &[Domain]) -> Vec<Domain> {
    let mut schemas: Vec<Domain> = sets.to_vec();
    for set in sets {
        for named in set.names_schemas_of() {
            if !schemas.contains(named) {
                schemas.push(*named);
            }
        }
    }
    schemas.sort_unstable();
    schemas
}

/// Bootstrap done outside the migrations, for a database where `sets` apply:
/// the `ext` schema, the schemas [`bootstrap_schemas`] names, and
/// [`BOOTSTRAP_EXTENSION`].
///
/// Every test proves it: `testkit` builds its template databases by running
/// exactly this path, over every set, against a fresh database.
fn bootstrap_statements(sets: &[Domain]) -> Vec<String> {
    let mut statements = vec!["CREATE SCHEMA IF NOT EXISTS ext".to_owned()];
    statements.extend(
        bootstrap_schemas(sets)
            .into_iter()
            .map(|domain| format!("CREATE SCHEMA IF NOT EXISTS {}", domain.schema())),
    );
    statements.push(BOOTSTRAP_EXTENSION.to_owned());
    statements
}

/// A stable fingerprint of the complete embedded migration state: the
/// bootstrap statements plus every migrator's (version, checksum) sequence,
/// in application order.
///
/// Two builds with identical migrations produce the same value; any migration
/// change produces a new one. Test infrastructure keys its migrated template
/// databases on this (no openEHR spec governs test infrastructure — our own
/// design).
#[must_use]
pub fn migration_fingerprint() -> String {
    // FNV-1a over the bootstrap text + each migration's version/checksum —
    // collision-resistant enough for a cache key with a handful of live
    // values, with no hashing dependency.
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    for statement in bootstrap_statements(&Domain::ALL) {
        eat(statement.as_bytes());
    }
    for migrator in std::iter::once(&EXT_MIGRATOR)
        .chain(Domain::ALL.iter().map(|domain| domain_migrator(*domain).1))
    {
        for migration in migrator.iter() {
            eat(&migration.version.to_le_bytes());
            eat(&migration.checksum);
        }
    }
    format!("{hash:016x}")
}

/// Bootstrap schemas/extensions and apply every migration set on one
/// connection — the single-database sequence.
///
/// Each migrator runs on a connection whose `search_path` starts with its
/// target schema, so the unqualified DDL and that set's `_sqlx_migrations`
/// bookkeeping table land in the right schema (five independent bookkeeping
/// tables, one per set). Safe to call repeatedly: an already-applied migration
/// is skipped, and its recorded checksum is validated against the embedded
/// source.
///
/// The sequence runs on a connection **detached** from the pool and closed
/// afterwards: the migrators mutate the session `search_path`, and a
/// mid-sequence failure must never return a connection with a non-standard
/// search path to the pool.
///
/// # Errors
///
/// Returns [`DbError::Acquire`] when no connection can be acquired within the
/// acquire timeout, [`DbError::Sqlx`] when a bootstrap/`search_path` statement
/// fails (e.g. the
/// role lacks `CREATE` on the database, or `btree_gist` is unavailable), and
/// [`DbError::Migrate`] when a migration fails to apply or an
/// already-applied migration fails checksum validation against the embedded
/// source.
pub async fn run_migrations(pool: &PgPool) -> Result<(), DbError> {
    let mut conn = acquire(pool).await?.detach();
    let outcome = apply_migrations(&mut conn, &Domain::ALL).await;
    close_quietly(conn).await;
    outcome
}

/// Open the one-shot connection one database's schema preparation runs on.
///
/// Preparation is a boot step rather than a serving path, so it takes a single
/// connection that is closed again instead of a pool held for the process
/// lifetime, and it carries none of the pools' session setup: no domain
/// `search_path` (the sequence sets its own per set, and the bookkeeping reads
/// are schema-qualified) and no `statement_timeout`, which is the backstop for
/// request-serving statements and would cancel a long migration partway
/// (<https://www.postgresql.org/docs/18/runtime-config-client.html>).
async fn migration_connection(dsn: &str) -> Result<PgConnection, DbError> {
    let conn = PgConnection::connect(dsn).await?;
    Ok(conn)
}

/// The credential one database is prepared on, best first.
///
/// The order is the preference when several domains of one database name
/// different credentials: the migration DSN of `[db]`, then a migration
/// credential named for the database in `[storage.migrate.<domain>]`, and a
/// runtime DSN only when the deployment named neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PreparationCredential {
    /// [`DbConfig::migrate_dsn`], for the database it reaches.
    Migrator,
    /// `[storage.migrate.<domain>].url`, for the database that domain lives in.
    DomainMigrator,
    /// A relocated domain's own runtime DSN.
    Runtime,
}

/// One database's share of the preparation: the credential to connect on, and
/// the domains that live there.
#[derive(Debug)]
struct PreparationGroup {
    /// The DSN this database is prepared with.
    dsn: String,
    /// Which kind of credential [`Self::dsn`] is.
    credential: PreparationCredential,
    /// The domains resident in it, in [`Domain::ALL`] order.
    domains: Vec<Domain>,
}

impl PreparationGroup {
    /// The domain sets `apply` runs in this database, in [`Domain::ALL`] order:
    /// each resident domain's, plus each companion set a resident one needs
    /// ([`Domain::prepares_with`]).
    ///
    /// `verify` compares the resident sets only: a companion set serves no
    /// pool, and the resident set that needs it could not have been applied
    /// without it.
    fn sets(&self) -> Vec<Domain> {
        let mut sets = self.domains.clone();
        for domain in &self.domains {
            if let Some(required) = domain.prepares_with()
                && !sets.contains(&required)
            {
                sets.push(required);
            }
        }
        sets.sort_unstable();
        sets
    }
}

/// The database a connection reaches: its cluster and the database inside it.
///
/// Read from the server rather than compared as DSN text, for the reason
/// [`cluster_identity`] gives: two DSNs can reach one database through
/// different host names, a proxy or a pooler, and two DSNs that differ only in
/// their credential always do. The database name completes the cluster
/// identifier, because two domains may be relocated to two databases of one
/// cluster.
async fn database_identity(conn: &mut PgConnection) -> Result<(String, String), DbError> {
    let identity: (String, String) = sqlx::query_as(
        "SELECT system_identifier::text, current_database()::text FROM pg_control_system()",
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(identity)
}

/// The database `dsn` reaches, read once per distinct DSN text.
async fn database_of(
    dsn: &str,
    seen: &mut Vec<(String, (String, String))>,
) -> Result<(String, String), DbError> {
    if let Some((_, identity)) = seen.iter().find(|(known, _)| known == dsn) {
        return Ok(identity.clone());
    }
    let mut conn = migration_connection(dsn).await?;
    let identity = database_identity(&mut conn).await;
    close_quietly(conn).await;
    let identity = identity?;
    seen.push((dsn.to_owned(), identity.clone()));
    Ok(identity)
}

/// Which credential prepares which domains, one group per database.
///
/// A domain's candidate credential is `[storage.migrate.<domain>].url` when the
/// deployment names one, its own runtime DSN when it is relocated without one,
/// and the migration DSN ([`DbConfig::migrate_dsn`]) otherwise. Candidates are
/// then grouped by the DATABASE they reach, never by DSN text, and each
/// database is prepared on the best credential among its domains'
/// ([`PreparationCredential`]): the migration DSN prepares every domain in the
/// database it reaches, which is the whole of the separated-credential posture
/// on one database.
///
/// With nothing relocated and no domain migrator named, the plan is the one
/// database the migration DSN reaches, and no connection is opened.
///
/// # Errors
///
/// [`DbError::DomainMigratorElsewhere`] when a domain's migration credential
/// and its runtime DSN reach different databases, and [`DbError::Sqlx`] when a
/// connection or an identity read fails.
async fn preparation_plan(
    settings: &DbConfig,
    storage: &StorageConfig,
) -> Result<Vec<PreparationGroup>, DbError> {
    let migration_dsn = settings.migrate_dsn();
    let candidates: Vec<(Domain, &str, PreparationCredential)> = Domain::ALL
        .into_iter()
        .map(|domain| {
            if let Some(dsn) = storage.migrate_dsn(domain) {
                (domain, dsn, PreparationCredential::DomainMigrator)
            } else if storage.is_separated(domain) {
                (
                    domain,
                    storage.dsn(domain, settings),
                    PreparationCredential::Runtime,
                )
            } else {
                (domain, migration_dsn, PreparationCredential::Migrator)
            }
        })
        .collect();
    if candidates
        .iter()
        .all(|(_, _, credential)| *credential == PreparationCredential::Migrator)
    {
        return Ok(vec![PreparationGroup {
            dsn: migration_dsn.to_owned(),
            credential: PreparationCredential::Migrator,
            domains: Domain::ALL.to_vec(),
        }]);
    }

    let mut seen = Vec::new();
    let migration_database = database_of(migration_dsn, &mut seen).await?;
    let mut plan: Vec<((String, String), PreparationGroup)> = Vec::new();
    for (domain, candidate, kind) in candidates {
        let database = database_of(candidate, &mut seen).await?;
        if kind == PreparationCredential::DomainMigrator && storage.is_separated(domain) {
            let served = database_of(storage.dsn(domain, settings), &mut seen).await?;
            if served != database {
                return Err(DbError::DomainMigratorElsewhere { domain });
            }
        }
        let (dsn, credential) = if database == migration_database {
            (migration_dsn, PreparationCredential::Migrator)
        } else {
            (candidate, kind)
        };
        if let Some((_, group)) = plan.iter_mut().find(|(known, _)| *known == database) {
            group.domains.push(domain);
            if credential < group.credential {
                dsn.clone_into(&mut group.dsn);
                group.credential = credential;
            }
        } else {
            plan.push((
                database,
                PreparationGroup {
                    dsn: dsn.to_owned(),
                    credential,
                    domains: vec![domain],
                },
            ));
        }
    }
    let mut plan: Vec<PreparationGroup> = plan.into_iter().map(|(_, group)| group).collect();
    plan.sort_by(|a, b| a.domains.cmp(&b.domains));
    Ok(plan)
}

/// Applies the embedded migrations, one connection per database the domains
/// reach.
///
/// The `ext` set is applied in every such database, then each resident
/// domain's set and any companion set one of them needs
/// ([`Domain::prepares_with`]), on the credential the plan chose for that
/// database: the migration DSN where it reaches it, a domain's
/// `[storage.migrate.<domain>]` credential elsewhere, and a relocated domain's
/// runtime DSN when neither is named.
///
/// # Errors
///
/// [`DbError::DomainMigratorElsewhere`] when a domain's migration credential
/// and its runtime DSN reach different databases, [`DbError::Sqlx`] when a DSN
/// does not parse, a connection fails, or a bootstrap statement is refused (a
/// credential without `CREATE` on the database or on a schema), and
/// [`DbError::Migrate`] when a migration fails to apply or an already-applied
/// one fails checksum validation.
pub async fn apply_schema(settings: &DbConfig, storage: &StorageConfig) -> Result<(), DbError> {
    for group in preparation_plan(settings, storage).await? {
        let mut conn = migration_connection(&group.dsn).await?;
        let outcome = apply_migrations(&mut conn, &group.sets()).await;
        close_quietly(conn).await;
        outcome?;
    }
    Ok(())
}

/// Verifies the recorded migration state on every database the domains reach.
///
/// The bookkeeping comparison over one connection per database, on the
/// credential [`apply_schema`] would prepare it with: the check reads `ext`'s
/// and each resident set's `_sqlx_migrations` table, which no single-domain
/// runtime credential can do. Issues no DDL, so the credentials it names need
/// read access and nothing more.
///
/// # Errors
///
/// [`DbError::SchemaNotReady`] naming the first divergence found,
/// [`DbError::SchemaUnreadable`] when a credential cannot read a set's
/// bookkeeping at all, [`DbError::DomainMigratorElsewhere`] when a domain's
/// migration credential and its runtime DSN reach different databases, or
/// [`DbError::Sqlx`] when a connection or the read fails for any other reason.
pub async fn verify_schema(settings: &DbConfig, storage: &StorageConfig) -> Result<(), DbError> {
    for group in preparation_plan(settings, storage).await? {
        let mut conn = migration_connection(&group.dsn).await?;
        let outcome = verify_recorded_state(&mut conn, &group.domains).await;
        close_quietly(conn).await;
        outcome?;
    }
    Ok(())
}

/// One migration set's recorded level beside the level this build embeds.
///
/// No openEHR spec governs migration bookkeeping — our own design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetLevel {
    /// The schema the set runs in, which also holds its bookkeeping table.
    pub schema: &'static str,
    /// The highest version the bookkeeping records as applied successfully;
    /// `None` when the schema has never been migrated.
    pub applied: Option<i64>,
    /// The highest version this build embeds; `None` for a set that embeds no
    /// migration.
    pub embedded: Option<i64>,
}

/// The migration levels of one database the domains reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatabaseLevels {
    /// The domains resident in this database.
    pub domains: Vec<Domain>,
    /// The `ext` set, then each resident domain's set.
    pub sets: Vec<SetLevel>,
}

/// Reads the recorded migration level of every set in every database the
/// domains reach, issuing no DDL.
///
/// The read [`verify_schema`] does, summarized rather than judged: it connects
/// on the same credential per database as [`apply_schema`] (the migration DSN
/// in the database it reaches, a `[storage.migrate.<domain>]` credential or a
/// relocated domain's own DSN elsewhere), so the credentials it names need read
/// access to the bookkeeping and nothing more.
///
/// # Errors
///
/// [`DbError::SchemaUnreadable`] when a credential cannot read a set's
/// bookkeeping, [`DbError::DomainMigratorElsewhere`] when a domain's migration
/// credential and its runtime DSN reach different databases, or
/// [`DbError::Sqlx`] when a connection or a read fails for any other reason.
pub async fn schema_levels(
    settings: &DbConfig,
    storage: &StorageConfig,
) -> Result<Vec<DatabaseLevels>, DbError> {
    let mut databases = Vec::new();
    for group in preparation_plan(settings, storage).await? {
        let mut conn = migration_connection(&group.dsn).await?;
        let outcome = recorded_levels(&mut conn, &group.domains).await;
        close_quietly(conn).await;
        databases.push(DatabaseLevels {
            domains: group.domains,
            sets: outcome?,
        });
    }
    Ok(databases)
}

/// The `ext` set's level, then each named domain's, on one connection.
async fn recorded_levels(
    conn: &mut PgConnection,
    domains: &[Domain],
) -> Result<Vec<SetLevel>, DbError> {
    let mut sets = vec![set_level(&mut *conn, "ext", &EXT_MIGRATOR).await?];
    for domain in domains {
        let (schema, migrator) = domain_migrator(*domain);
        sets.push(set_level(&mut *conn, schema, migrator).await?);
    }
    Ok(sets)
}

/// One set's highest successfully applied version beside its embedded one.
async fn set_level(
    conn: &mut PgConnection,
    schema: &'static str,
    migrator: &Migrator,
) -> Result<SetLevel, DbError> {
    let embedded = migrator.iter().map(|migration| migration.version).max();
    // The schema name is one of the `domain_migrator` literals, never input,
    // and both statements classify a privilege refusal as `verify_set` does.
    let bookkeeping = format!("{schema}._sqlx_migrations");
    let present: bool = match sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(&bookkeeping)
        .fetch_one(&mut *conn)
        .await
    {
        Ok(present) => present,
        Err(error) => return Err(unreadable_bookkeeping(&mut *conn, schema, error).await),
    };
    if !present {
        return Ok(SetLevel {
            schema,
            applied: None,
            embedded,
        });
    }
    let query = format!("SELECT max(version) FROM {bookkeeping} WHERE success");
    let applied: Option<i64> = match sqlx::query_scalar(sqlx::AssertSqlSafe(query))
        .fetch_one(&mut *conn)
        .await
    {
        Ok(applied) => applied,
        Err(error) => return Err(unreadable_bookkeeping(&mut *conn, schema, error).await),
    };
    Ok(SetLevel {
        schema,
        applied,
        embedded,
    })
}

/// Reads what the database layer can measure about a deployment's databases:
/// the cluster each domain pool reached, and the domains whose database schema
/// preparation reaches on a runtime credential.
///
/// The facts [`crate::config::deployment::DeploymentPosture::evaluate`] reads
/// beside the configuration. No openEHR spec governs deployment posture — our
/// own design/extension.
///
/// # Errors
///
/// [`DbError::Sqlx`] when a pool cannot read its cluster identity or a
/// migration connection cannot be opened, and whatever
/// [`domains_prepared_on_a_runtime_credential`] returns.
pub async fn database_facts(
    settings: &DbConfig,
    storage: &StorageConfig,
    pools: &DomainPools,
) -> Result<crate::config::deployment::DatabaseFacts, DbError> {
    let clusters = crate::config::deployment::ClusterIdentities {
        clinical: Some(cluster_identity(&pools.clinical).await?),
        party: Some(cluster_identity(&pools.party).await?),
        linkage: Some(cluster_identity(&pools.linkage).await?),
        audit: Some(cluster_identity(&pools.audit).await?),
    };
    Ok(crate::config::deployment::DatabaseFacts {
        clusters,
        prepared_on_runtime_credential: domains_prepared_on_a_runtime_credential(settings, storage)
            .await?,
    })
}

/// Close a detached connection, reporting a failure to close as a trace event
/// rather than as the operation's outcome: the work is already done, and a
/// failed close must not mask its result.
async fn close_quietly(conn: PgConnection) {
    if let Err(error) = conn.close().await {
        tracing::debug!(%error, "closing the migration connection failed");
    }
}

/// Brings every database the domains reach to the state this build requires, as
/// [`DbConfig::migrate`] directs, and then proves the pseudonymisation
/// boundary holds.
///
/// The boot-path entry point — call it rather than the halves it composes
/// wherever the operator's configuration should decide. [`MigrationMode::Apply`]
/// applies the embedded migrations ([`apply_schema`]);
/// [`MigrationMode::Verify`] issues no DDL and only checks
/// ([`verify_schema`]).
///
/// **Preparation and the boot gate deliberately authenticate as different
/// credentials, and that is the whole point of the split.** Preparation spans
/// every schema of a database — the DDL of each resident set under `apply`,
/// their `_sqlx_migrations` tables under `verify` — so it runs on the
/// migration DSN ([`DbConfig::migrate_dsn`]) for every domain that reaches the
/// same database it does.
/// [`verify_domain_isolation`] runs on the runtime pools instead, because it
/// exists to measure what THOSE credentials can reach: it reads `pg_catalog`
/// and the `has_*_privilege` functions, which any role may call
/// (<https://www.postgresql.org/docs/18/functions-info.html>), so running it
/// on the migration credential would not fail — it would silently measure a
/// role that holds every domain by design, and the boot gate would stop
/// saying anything about the roles that serve requests.
///
/// # Errors
///
/// In `apply` mode, whatever [`apply_schema`] returns. In `verify` mode,
/// [`DbError::SchemaNotReady`] when a database does not carry exactly this
/// build's migrations, [`DbError::SchemaUnreadable`] when the migration
/// credential cannot read a set's bookkeeping at all, or [`DbError::Sqlx`]
/// when the check itself cannot run. In both modes, whatever
/// [`verify_domain_isolation`] returns.
pub async fn prepare(
    settings: &DbConfig,
    storage: &StorageConfig,
    pools: &DomainPools,
    profile: DeploymentProfile,
) -> Result<(), DbError> {
    match settings.migrate {
        MigrationMode::Apply => apply_schema(settings, storage).await?,
        MigrationMode::Verify => {
            tracing::info!(
                "[db].migrate is `verify`: this server issues no DDL and requires an \
                 already-migrated database"
            );
            verify_schema(settings, storage).await?;
        }
    }
    verify_domain_isolation(pools, &storage.layout(settings), profile).await
}

/// Verifies, without issuing any DDL, that the database carries exactly the
/// migrations this binary embeds.
///
/// Read-only by construction: it reads each set's `_sqlx_migrations`
/// bookkeeping table and compares versions, success flags and checksums
/// against the embedded sources. That makes it usable both as the boot gate for
/// [`MigrationMode::Verify`] and as an operator check against a running
/// database.
///
/// # Errors
///
/// [`DbError::SchemaNotReady`] naming the first divergence found,
/// [`DbError::SchemaUnreadable`] when this pool's credential cannot read a
/// set's bookkeeping at all, [`DbError::Acquire`] when no connection can be
/// acquired, or [`DbError::Sqlx`] when the bookkeeping read fails for any other
/// reason.
pub async fn verify_migrations(pool: &PgPool) -> Result<(), DbError> {
    let mut conn = acquire(pool).await?;
    verify_recorded_state(&mut conn, &Domain::ALL).await
}

/// Compare the `ext` set and each named domain's set against their embedded
/// sources, on one connection.
///
/// The shared body of [`verify_migrations`] (a pooled connection) and
/// [`verify_schema`] (the one-shot migration connections), so the boot gate and
/// the operator check cannot drift apart.
async fn verify_recorded_state(conn: &mut PgConnection, domains: &[Domain]) -> Result<(), DbError> {
    verify_set(&mut *conn, "ext", &EXT_MIGRATOR).await?;
    for domain in domains {
        let (schema, migrator) = domain_migrator(*domain);
        verify_set(&mut *conn, schema, migrator).await?;
    }
    Ok(())
}

/// Refuses to serve when the database's grants do not hold the domain
/// boundaries the deployment claims.
///
/// Four refusals: a runtime role that can read a pseudonymisation domain it
/// does not own, a runtime role that can write a migration bookkeeping table,
/// two separately-configured domains that authenticate as one role, and —
/// under [`DeploymentProfile::Production`] — a domain role that does not exist.
/// The grant sweep runs once in every DATABASE a domain pool reaches, read from
/// the server (`pg_control_system()` and `current_database()`), because grants
/// are per database and a relocated domain's database carries its own.
///
/// The separation of the clinical record, the identity of its subject, and the
/// map between them is a property of the DATABASE's grants, not of the
/// application's routing: code that reaches for the wrong schema is a bug this
/// server can fix, while a role that can read two of the three domains defeats
/// the separation no matter how correct the code is (GDPR Art. 4(5) and
/// Art. 32(1)(a),
/// <https://eur-lex.europa.eu/eli/reg/2016/679/oj>; EDPB Guidelines 01/2025
/// require the separation to hold against internal actors). So the grants are
/// checked at boot, and a breach is a refusal rather than a warning.
///
/// Every object kind a read could go through is covered: tables, partitioned
/// tables, foreign tables, views and materialized views (`SELECT`), sequences
/// (`SELECT`/`USAGE`) and functions (`EXECUTE` — which PUBLIC holds by default,
/// PostgreSQL 18 `GRANT` §Notes,
/// <https://www.postgresql.org/docs/18/sql-grant.html>).
///
/// Two domains on ONE configured DSN are not a breach here — one DSN is one
/// credential by construction, and that is the co-located posture the
/// deployment profile reports as `shared_credential`. What is a breach is two
/// domains the operator configured SEPARATELY that nevertheless authenticate as
/// the same role: the deployment believes it made a separation it did not, and
/// a control that cannot decide must never look like a policy outcome.
///
/// A role that does not exist is skipped with a warning under
/// [`DeploymentProfile::Sandbox`] and refused under
/// [`DeploymentProfile::Production`]: role provisioning is a deployment step,
/// and the migrations create the roles only when the migrator holds
/// `CREATEROLE` (dev, compose and the test harness run without it), so a
/// sandbox proves what it can see — but a production deployment whose domain
/// roles are absent has no grants to separate anything with, and this check
/// would otherwise pass by having nothing to measure.
///
/// # Errors
///
/// [`DbError::DomainIsolationBreached`] naming the role, the object kind and
/// the schema-qualified object it can reach, [`DbError::BookkeepingWritable`]
/// naming the role and the bookkeeping table, [`DbError::DomainRoleShared`] when
/// two separately-configured domains authenticate as one role,
/// [`DbError::DomainRoleMissing`] under `production`, or [`DbError::Sqlx`] when
/// a catalog read itself fails.
pub async fn verify_domain_isolation(
    pools: &DomainPools,
    layout: &DomainLayout,
    profile: DeploymentProfile,
) -> Result<(), DbError> {
    let mut swept: Vec<(String, String)> = Vec::new();
    for domain in Domain::ALL {
        let pool = pools.get(domain);
        let database: (String, String) = sqlx::query_as(
            "SELECT system_identifier::text, current_database()::text FROM pg_control_system()",
        )
        .fetch_one(&mut *acquire(pool).await?)
        .await?;
        if swept.contains(&database) {
            continue;
        }
        swept.push(database);
        verify_role_barriers(pool, profile).await?;
    }
    verify_configured_separation(pools, layout).await
}

/// The grant sweep over the database `pool` reaches: every domain role against
/// every schema it must not reach, and against every migration bookkeeping
/// table it must not write.
///
/// Any pool of that database serves: the check reads `pg_catalog` and the
/// `has_*_privilege` functions, which answer about any role from any session.
async fn verify_role_barriers(pool: &PgPool, profile: DeploymentProfile) -> Result<(), DbError> {
    let bookkeeping_schemas: Vec<String> = std::iter::once("ext")
        .chain(Domain::ALL.into_iter().map(Domain::schema))
        .map(str::to_owned)
        .collect();
    for domain in Domain::ALL {
        let forbidden: Vec<String> = domain
            .barred_from()
            .iter()
            .map(|schema| (*schema).to_owned())
            .collect();
        for role in domain.roles() {
            // The existence probe runs first and separately:
            // `has_table_privilege` raises `undefined_object` for a role that
            // does not exist
            // (<https://www.postgresql.org/docs/18/functions-info.html>), so it
            // can never be evaluated for one, not even under a WHERE that would
            // discard the row.
            let present: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)")
                    .bind(role)
                    .fetch_one(&mut *acquire(pool).await?)
                    .await?;
            if !present {
                if profile == DeploymentProfile::Production {
                    return Err(DbError::DomainRoleMissing {
                        role: (*role).to_owned(),
                        domain,
                    });
                }
                tracing::warn!(
                    role = *role,
                    %domain,
                    "the domain role does not exist, so this database enforces no grant \
                     separation for that domain; provision the runtime roles (the migrations \
                     create them only with CREATEROLE). deployment_profile = \"production\" \
                     refuses to boot in this state"
                );
                continue;
            }
            // The bookkeeping is what `verify` compares against the embedded
            // sources, so a runtime role able to rewrite it could make an
            // unmigrated or altered database pass.
            let writable: Option<String> = sqlx::query_scalar(
                "SELECT n.nspname || '._sqlx_migrations'
                 FROM pg_namespace n
                 JOIN pg_class c ON c.relnamespace = n.oid AND c.relname = '_sqlx_migrations'
                 WHERE n.nspname = ANY($2)
                   AND has_table_privilege($1, c.oid, 'INSERT,UPDATE,DELETE,TRUNCATE')
                 ORDER BY 1
                 LIMIT 1",
            )
            .bind(role)
            .bind(&bookkeeping_schemas)
            .fetch_optional(&mut *acquire(pool).await?)
            .await?;
            if let Some(relation) = writable {
                return Err(DbError::BookkeepingWritable {
                    role: (*role).to_owned(),
                    relation,
                });
            }
            if forbidden.is_empty() {
                continue;
            }
            let breach: Option<(String, String)> = sqlx::query_as(
                "SELECT kind, relation FROM (
                     SELECT CASE c.relkind
                                WHEN 'S' THEN 'sequence'
                                WHEN 'v' THEN 'view'
                                WHEN 'm' THEN 'materialized view'
                                WHEN 'f' THEN 'foreign table'
                                ELSE 'table'
                            END AS kind,
                            n.nspname || '.' || c.relname AS relation
                     FROM pg_namespace n
                     JOIN pg_class c ON c.relnamespace = n.oid
                     WHERE n.nspname = ANY($2)
                       AND c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
                       AND CASE WHEN c.relkind = 'S'
                                THEN has_sequence_privilege($1, c.oid, 'SELECT,USAGE')
                                ELSE has_table_privilege($1, c.oid, 'SELECT')
                           END
                     UNION ALL
                     SELECT 'function', n.nspname || '.' || p.proname
                     FROM pg_namespace n
                     JOIN pg_proc p ON p.pronamespace = n.oid
                     WHERE n.nspname = ANY($2)
                       AND has_function_privilege($1, p.oid, 'EXECUTE')
                 ) reachable
                 ORDER BY relation
                 LIMIT 1",
            )
            .bind(role)
            .bind(&forbidden)
            .fetch_optional(&mut *acquire(pool).await?)
            .await?;
            if let Some((kind, relation)) = breach {
                return Err(DbError::DomainIsolationBreached {
                    role: (*role).to_owned(),
                    kind,
                    relation,
                });
            }
        }
    }
    Ok(())
}

/// The credential check: two domains the operator placed on DIFFERENT DSNs must
/// not authenticate as the same database role.
///
/// `current_user` is read from each pool rather than parsed out of the DSN: a
/// DSN may take its user from the environment, a service file or a `.pgpass`
/// entry, and what matters is the role the session actually holds
/// (<https://www.postgresql.org/docs/18/functions-info.html>).
///
/// Only the three pseudonymisation domains are compared. The audit repository
/// is written by the clinical credential by design, so a deployment that moves
/// it to a database of its own and keeps that role there has made no mistake.
async fn verify_configured_separation(
    pools: &DomainPools,
    layout: &DomainLayout,
) -> Result<(), DbError> {
    let mut roles: Vec<(Domain, String)> = Vec::new();
    for domain in Domain::ALL {
        if !domain.is_pseudonymisation_domain() {
            continue;
        }
        let role: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(&mut *acquire(pools.get(domain)).await?)
            .await?;
        roles.push((domain, role));
    }
    for (index, (domain, role)) in roles.iter().enumerate() {
        for (other, other_role) in roles.iter().skip(index + 1) {
            if role != other_role {
                continue;
            }
            if layout
                .placement(*domain)
                .shares_dsn_with(layout.placement(*other))
            {
                continue;
            }
            return Err(DbError::DomainRoleShared {
                domain: *domain,
                other: *other,
                role: role.clone(),
            });
        }
    }
    Ok(())
}

/// `SQLSTATE` 42501 `insufficient_privilege` — what `PostgreSQL` reports for a
/// refused read, whether the missing grant is on the relation or on its schema
/// (`PostgreSQL` docs § Appendix A "`PostgreSQL` Error Codes", class 42,
/// <https://www.postgresql.org/docs/18/errcodes-appendix.html>).
const SQLSTATE_INSUFFICIENT_PRIVILEGE: &str = "42501";

/// Whether `PostgreSQL` refused this statement for lack of privilege, rather
/// than failing for any other reason.
fn is_insufficient_privilege(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(refusal)
            if refusal.code().as_deref() == Some(SQLSTATE_INSUFFICIENT_PRIVILEGE)
    )
}

/// The database role this session authenticates as, for an error that must
/// name the credential and not only the schema.
///
/// A failure to read it renders as `unknown`: this runs only while another
/// error is already being reported, and replacing that error with this one
/// would hide the boot failure the operator has to act on.
async fn current_role(conn: &mut PgConnection) -> String {
    sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut *conn)
        .await
        .unwrap_or_else(|_| "unknown".to_owned())
}

/// Classify a failed bookkeeping read: a privilege refusal becomes
/// [`DbError::SchemaUnreadable`], naming the schema and the credential;
/// anything else stays the driver error it was.
///
/// A bare 42501 about `_sqlx_migrations` is unactionable — it names a table an
/// operator has never heard of and no credential at all — and the separated
/// posture reaches it on the very first set.
async fn unreadable_bookkeeping(
    conn: &mut PgConnection,
    schema: &str,
    error: sqlx::Error,
) -> DbError {
    if !is_insufficient_privilege(&error) {
        return DbError::Sqlx(error);
    }
    DbError::SchemaUnreadable {
        schema: schema.to_owned(),
        role: current_role(&mut *conn).await,
        source: error,
    }
}

/// Compare one migration set's bookkeeping table against its embedded source.
async fn verify_set(
    conn: &mut PgConnection,
    schema: &str,
    migrator: &Migrator,
) -> Result<(), DbError> {
    // The schema name is one of the literals in `MIGRATION_SETS`, never
    // input: `to_regclass` answers NULL for a missing relation rather than
    // failing (PostgreSQL 18 docs, "System Information Functions") — but it
    // still raises 42501 for a schema this credential may not enter, which is
    // why both statements here classify their failure.
    let bookkeeping = format!("{schema}._sqlx_migrations");
    let present: bool = match sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(&bookkeeping)
        .fetch_one(&mut *conn)
        .await
    {
        Ok(present) => present,
        Err(error) => return Err(unreadable_bookkeeping(&mut *conn, schema, error).await),
    };
    if !present {
        return Err(DbError::SchemaNotReady(SchemaMismatch::NeverMigrated {
            schema: schema.to_owned(),
        }));
    }

    let query = format!("SELECT version, success, checksum FROM {bookkeeping} ORDER BY version");
    let applied: Vec<(i64, bool, Vec<u8>)> = match sqlx::query_as(sqlx::AssertSqlSafe(query))
        .fetch_all(&mut *conn)
        .await
    {
        Ok(applied) => applied,
        Err(error) => return Err(unreadable_bookkeeping(&mut *conn, schema, error).await),
    };

    let mut unknown: Vec<i64> = applied.iter().map(|(version, _, _)| *version).collect();
    let mut missing: Vec<i64> = Vec::new();
    for embedded in migrator.iter() {
        let Some((version, success, checksum)) = applied
            .iter()
            .find(|(version, _, _)| *version == embedded.version)
        else {
            missing.push(embedded.version);
            continue;
        };
        unknown.retain(|known| known != version);
        if !success {
            return Err(DbError::SchemaNotReady(SchemaMismatch::Failed {
                schema: schema.to_owned(),
                version: *version,
            }));
        }
        if checksum.as_slice() != embedded.checksum.as_ref() {
            return Err(DbError::SchemaNotReady(SchemaMismatch::ChecksumMismatch {
                schema: schema.to_owned(),
                version: *version,
            }));
        }
    }
    if !missing.is_empty() {
        return Err(DbError::SchemaNotReady(SchemaMismatch::Missing {
            schema: schema.to_owned(),
            versions: missing,
        }));
    }
    if !unknown.is_empty() {
        return Err(DbError::SchemaNotReady(SchemaMismatch::Ahead {
            schema: schema.to_owned(),
            versions: unknown,
        }));
    }
    Ok(())
}

/// Runs the bootstrap, the `ext` set and each named set on one dedicated
/// connection.
///
/// `ext` first, unconditionally: every later set finds its roles, helper
/// functions and posture table in place, and a database that carries only one
/// relocated domain still gets the helpers that domain's storage code calls.
/// Already-applied sets are no-ops, which is what makes "the `ext` set is
/// applied per database by whichever domain reaches it first" true without any
/// bookkeeping of our own.
///
/// A schema a set only NAMES ([`Domain::names_schemas_of`]) and which this run
/// had to create is dropped again afterwards, so a relocated database ends
/// holding `ext` and its own sets' schemas and nothing else. The drop is
/// `RESTRICT` (<https://www.postgresql.org/docs/18/sql-dropschema.html>): a
/// schema that came to hold anything stays, and the failure is reported.
async fn apply_migrations(conn: &mut PgConnection, domains: &[Domain]) -> Result<(), DbError> {
    guard_first_generation_database(&mut *conn).await?;
    let named_only: Vec<&'static str> = bootstrap_schemas(domains)
        .into_iter()
        .filter(|domain| !domains.contains(domain))
        .map(Domain::schema)
        .collect();
    let preexisting = present_schemas(&mut *conn, &named_only).await?;
    for statement in bootstrap_statements(domains) {
        // Each statement is built from the `Domain::schema` literals, never
        // input; DDL takes no bind placeholder.
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&mut *conn)
            .await?;
    }

    sqlx::query("SET search_path TO ext")
        .execute(&mut *conn)
        .await?;
    EXT_MIGRATOR.run(&mut *conn).await?;

    for domain in domains {
        let (schema, migrator) = domain_migrator(*domain);
        // The schema name is one of the literals in `domain_migrator`, never
        // input; PostgreSQL's SET takes no bind placeholder.
        let search_path = format!("SET search_path TO {schema}, ext");
        sqlx::query(sqlx::AssertSqlSafe(search_path))
            .execute(&mut *conn)
            .await?;
        migrator.run(&mut *conn).await?;
    }
    for schema in named_only {
        if preexisting.contains(&schema) {
            continue;
        }
        // The name is a `Domain::schema` literal, never input.
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema}")))
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Refuse a database created by a release older than the storage rewrite.
///
/// The rewrite is greenfield: the migration sets replace the first
/// generation's outright, so there is nothing to upgrade and nothing to adopt.
/// Left to itself the sequence would create `clinical` and `party` beside the
/// old `ehr` and `demographic` schemas and serve an empty repository, with the
/// operator's data sitting untouched and unreachable in the same database.
/// That is the one outcome worse than refusing.
///
/// The signature is [`FIRST_GENERATION_SETS`]: bookkeeping in a schema this
/// build owns no set for, or bookkeeping whose version 1 names the file the
/// first generation opened that schema with.
///
/// `to_regclass` answers `NULL` for a missing relation instead of failing
/// (<https://www.postgresql.org/docs/18/functions-info.html>), so the presence
/// probe is one statement over a fresh database and an old one alike.
async fn guard_first_generation_database(conn: &mut PgConnection) -> Result<(), DbError> {
    for set in FIRST_GENERATION_SETS {
        let present: bool =
            sqlx::query_scalar("SELECT to_regclass($1 || '._sqlx_migrations') IS NOT NULL")
                .bind(set.schema)
                .fetch_one(&mut *conn)
                .await?;
        if !present {
            continue;
        }
        let Some(signature) = set.first_description else {
            // This build owns no set of that name, so the bookkeeping can only
            // be the first generation's.
            return Err(DbError::FirstGenerationDatabase { schema: set.schema });
        };
        // The schema name survives the rewrite, so which migration ran FIRST is
        // what separates the generations. `set.schema` is one of the literals
        // in `FIRST_GENERATION_SETS` above, never input.
        let sql = format!(
            "SELECT description FROM {}._sqlx_migrations WHERE version = 1",
            set.schema
        );
        let first: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_optional(&mut *conn)
            .await?;
        if first.as_deref() == Some(signature) {
            return Err(DbError::FirstGenerationDatabase { schema: set.schema });
        }
    }
    Ok(())
}

// ── Erasure ──────────────────────────────────────────────────────────────────

/// One database an erasure reaches, and the schemas of this build in it.
///
/// No openEHR spec governs this — our own design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatabaseErasure {
    /// The database's name, as the server reports it (`current_database()`).
    pub database: String,
    /// The domains resident in this database.
    pub domains: Vec<Domain>,
    /// The schemas of this build present in it: before an erasure, the ones it
    /// would drop; after one, the ones it dropped.
    pub schemas: Vec<&'static str>,
}

/// Every database the domains reach, with the schemas of this build each one
/// holds, and the stored instance id.
///
/// No openEHR spec governs this — our own design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EraseReport {
    /// One entry per database, in preparation order.
    pub databases: Vec<DatabaseErasure>,
    /// The instance id the clinical schema stores, if it stores one.
    pub instance_id: Option<uuid::Uuid>,
}

impl EraseReport {
    /// Whether no database holds a schema of this build.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.databases.iter().all(|db| db.schemas.is_empty())
    }

    /// The name of the database the clinical domain lives in.
    #[must_use]
    pub fn clinical_database(&self) -> Option<&str> {
        self.databases
            .iter()
            .find(|db| db.domains.contains(&Domain::Clinical))
            .map(|db| db.database.as_str())
    }
}

/// Returns the schemas this build creates in a database where the sets of
/// `domains` are applied, in the order an erasure drops them: the sets' own,
/// then `ext`.
///
/// The names are the [`domain_migrator`] literals and `ext`, never input.
fn owned_schemas(domains: &[Domain]) -> Vec<&'static str> {
    let mut schemas: Vec<&'static str> = domains
        .iter()
        .rev()
        .map(|domain| domain_migrator(*domain).0)
        .collect();
    schemas.push("ext");
    schemas
}

/// Returns the schemas an erasure drops in the connected database, where the
/// sets of `domains` are applied: those of [`owned_schemas`] that exist, and
/// any other schema name of this build that exists and holds nothing.
///
/// An earlier release created every domain's schema in every database it
/// prepared, so a relocated database may still carry the others, empty; an
/// erasure removes them too, and never one that holds an object.
async fn erasable_schemas(
    conn: &mut PgConnection,
    domains: &[Domain],
) -> Result<Vec<&'static str>, DbError> {
    let mut schemas = present_schemas(&mut *conn, &owned_schemas(domains)).await?;
    let others: Vec<&'static str> = Domain::ALL
        .into_iter()
        .filter(|domain| !domains.contains(domain))
        .map(Domain::schema)
        .collect();
    let names: Vec<String> = others.iter().map(|s| (*s).to_owned()).collect();
    let empty: Vec<String> = sqlx::query_scalar(
        "SELECT n.nspname::text FROM pg_namespace n WHERE n.nspname = ANY($1) \
         AND NOT EXISTS (SELECT 1 FROM pg_class c WHERE c.relnamespace = n.oid) \
         AND NOT EXISTS (SELECT 1 FROM pg_proc p WHERE p.pronamespace = n.oid) \
         AND NOT EXISTS (SELECT 1 FROM pg_type t WHERE t.typnamespace = n.oid)",
    )
    .bind(&names)
    .fetch_all(&mut *conn)
    .await?;
    schemas.extend(
        others
            .into_iter()
            .filter(|schema| empty.iter().any(|name| name == schema)),
    );
    Ok(schemas)
}

/// The schemas among `candidates` that exist in the connected database, in the
/// order given.
async fn present_schemas(
    conn: &mut PgConnection,
    candidates: &[&'static str],
) -> Result<Vec<&'static str>, DbError> {
    let names: Vec<String> = candidates.iter().map(|s| (*s).to_owned()).collect();
    let present: Vec<String> =
        sqlx::query_scalar("SELECT nspname::text FROM pg_namespace WHERE nspname = ANY($1)")
            .bind(&names)
            .fetch_all(&mut *conn)
            .await?;
    Ok(candidates
        .iter()
        .copied()
        .filter(|schema| present.iter().any(|name| name == schema))
        .collect())
}

/// The instance id `clinical.usage_report_instance` stores, when the relation
/// exists and holds its row.
async fn stored_instance_id(conn: &mut PgConnection) -> Result<Option<uuid::Uuid>, DbError> {
    let present: bool =
        sqlx::query_scalar("SELECT to_regclass('clinical.usage_report_instance') IS NOT NULL")
            .fetch_one(&mut *conn)
            .await?;
    if !present {
        return Ok(None);
    }
    let id = sqlx::query_scalar("SELECT instance_id FROM clinical.usage_report_instance")
        .fetch_optional(&mut *conn)
        .await?;
    Ok(id)
}

/// Reports what [`erase_schema`] would drop, issuing no DDL and writing
/// nothing.
///
/// It connects as [`apply_schema`] does, one credential per database, and
/// lists the schemas of every set `apply` runs there, companion sets included.
/// No openEHR spec governs this — our own design.
///
/// # Errors
///
/// [`DbError::DomainMigratorElsewhere`] when a domain's migration credential
/// and its runtime DSN reach different databases, and [`DbError::Sqlx`] when a
/// connection or a catalog read fails.
pub async fn erase_inventory(
    settings: &DbConfig,
    storage: &StorageConfig,
) -> Result<EraseReport, DbError> {
    let mut report = EraseReport {
        databases: Vec::new(),
        instance_id: None,
    };
    for group in preparation_plan(settings, storage).await? {
        let mut conn = migration_connection(&group.dsn).await?;
        let outcome = async {
            let database: String = sqlx::query_scalar("SELECT current_database()::text")
                .fetch_one(&mut conn)
                .await?;
            let schemas = erasable_schemas(&mut conn, &group.sets()).await?;
            let instance_id = if group.domains.contains(&Domain::Clinical) {
                stored_instance_id(&mut conn).await?
            } else {
                None
            };
            Ok::<_, DbError>((database, schemas, instance_id))
        }
        .await;
        close_quietly(conn).await;
        let (database, schemas, instance_id) = outcome?;
        report.instance_id = report.instance_id.or(instance_id);
        report.databases.push(DatabaseErasure {
            database,
            domains: group.domains,
            schemas,
        });
    }
    Ok(report)
}

/// Drops every schema this build creates, in every database the domains reach,
/// and with them every row they hold and the stored instance id.
///
/// Per database, one transaction runs `DROP SCHEMA … CASCADE` for the `ext`
/// schema and each resident domain's schema (`clinical`, `party`, `linkage`,
/// `audit`), so a database is either erased whole or left as it was
/// (<https://www.postgresql.org/docs/18/sql-dropschema.html>). The cascade takes
/// the `btree_gist` extension installed in `ext`, the cold-tier partitions and
/// the default privileges granted in those schemas. Cluster-wide objects stay:
/// the `ferroehr_*` roles, and the database itself. Another domain's schema of
/// this build that a database holds empty, as an earlier release left in a
/// relocated database, is dropped too. A schema already absent is skipped, so a
/// second run drops nothing and reports an empty list.
///
/// The returned report lists, per database, the schemas this call dropped;
/// `instance_id` is the id that was stored before the drop. No openEHR spec
/// governs this — our own design, for the removal of all data and settings
/// that `docs/law/eu/cra/text.html` Annex I Part I(2)(m) asks for.
///
/// # Errors
///
/// [`DbError::DomainMigratorElsewhere`] when a domain's migration credential
/// and its runtime DSN reach different databases, and [`DbError::Sqlx`] when a
/// connection fails,
/// the credential does not own a schema, or a lock is not granted within 30
/// seconds (a server still connected). A database already erased stays
/// erased; the failing one is rolled back.
pub async fn erase_schema(
    settings: &DbConfig,
    storage: &StorageConfig,
) -> Result<EraseReport, DbError> {
    let mut report = EraseReport {
        databases: Vec::new(),
        instance_id: None,
    };
    for group in preparation_plan(settings, storage).await? {
        let mut conn = migration_connection(&group.dsn).await?;
        let outcome = drop_owned_schemas(&mut conn, &group.sets()).await;
        close_quietly(conn).await;
        let (database, schemas, instance_id) = outcome?;
        report.instance_id = report.instance_id.or(instance_id);
        report.databases.push(DatabaseErasure {
            database,
            domains: group.domains,
            schemas,
        });
    }
    Ok(report)
}

/// Drops one database's schemas in one transaction, given the sets applied
/// there, and returns its name, the schemas dropped and the instance id it
/// stored.
async fn drop_owned_schemas(
    conn: &mut PgConnection,
    domains: &[Domain],
) -> Result<(String, Vec<&'static str>, Option<uuid::Uuid>), DbError> {
    let mut tx = conn.begin().await?;
    // A server still connected holds locks on what it reads; the drop fails
    // after 30 s instead of waiting behind it
    // (<https://www.postgresql.org/docs/18/runtime-config-client.html>).
    sqlx::query("SET LOCAL lock_timeout = '30s'")
        .execute(&mut *tx)
        .await?;
    let database: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&mut *tx)
        .await?;
    let instance_id = if domains.contains(&Domain::Clinical) {
        stored_instance_id(&mut tx).await?
    } else {
        None
    };
    let schemas = erasable_schemas(&mut tx, domains).await?;
    for schema in &schemas {
        // The name is one of the `owned_schemas` literals, never input;
        // DROP SCHEMA takes no bind placeholder.
        let statement = format!("DROP SCHEMA IF EXISTS {schema} CASCADE");
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok((database, schemas, instance_id))
}

// ── Deployment posture ───────────────────────────────────────────────────────

/// The cluster a pool reaches, as `pg_control_system().system_identifier`.
///
/// Read from the control file rather than compared as DSN text: two DSNs can
/// reach one cluster through different host names, a proxy or a pooler, and a
/// string comparison would pass a deployment that is not separated (#3226).
///
/// # Errors
/// [`DbError::Sqlx`] when the read fails; a credential the deployment barred
/// from the function is reported, never read as a distinct cluster.
pub async fn cluster_identity(pool: &PgPool) -> Result<String, DbError> {
    let id: String = sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
        .fetch_one(&mut *acquire(pool).await?)
        .await?;
    Ok(id)
}

/// Returns the domains whose database schema preparation reaches on a
/// credential that also serves requests.
///
/// The migration DSN names ONE database, so a domain living in another one is
/// prepared on that database's `[storage.migrate.<domain>]` credential when the
/// deployment names one, and on the domain's own runtime DSN otherwise. The
/// answer comes from the preparation plan itself (the same private plan
/// [`prepare`] runs), which resolves database identity rather than DSN text, so
/// a second credential on the migrator's own database is correctly not
/// counted.
///
/// Empty under [`MigrationMode::Verify`], which issues no DDL at all, and empty
/// when every database is prepared on a migration credential.
///
/// No openEHR spec governs deployment posture — our own design/extension.
///
/// # Errors
/// [`DbError::Sqlx`] when a migration connection cannot be opened or its
/// database identity cannot be read, and [`DbError::DomainMigratorElsewhere`]
/// when a domain's migration credential and its runtime DSN reach different
/// databases.
pub async fn domains_prepared_on_a_runtime_credential(
    settings: &DbConfig,
    storage: &StorageConfig,
) -> Result<Vec<Domain>, DbError> {
    if settings.migrate != MigrationMode::Apply {
        return Ok(Vec::new());
    }
    let mut on_runtime: Vec<Domain> = preparation_plan(settings, storage)
        .await?
        .into_iter()
        .filter(|group| group.credential == PreparationCredential::Runtime)
        .flat_map(|group| group.domains)
        .collect();
    on_runtime.sort_unstable();
    Ok(on_runtime)
}

/// Stamp whether the database guards hold every stored subject reference to
/// an opaque UUID (#3241).
///
/// `required` once the deployment declares its pseudonym namespaces, `open`
/// otherwise. Written by the clinical runtime role on every boot through the
/// `SECURITY DEFINER` writer, and read by the `ehr_subject_pseudonym_guard`
/// trigger — the runtime role holds no privilege on the posture table itself,
/// so it cannot relax a guard by writing its own posture row.
///
/// # Errors
/// [`DbError::Sqlx`] when the write fails.
pub async fn stamp_subject_posture(pool: &PgPool, required: bool) -> Result<(), DbError> {
    let value = if required { "required" } else { "open" };
    sqlx::query("SELECT ext.stamp_posture('subject_pseudonyms', $1)")
        .bind(value)
        .execute(&mut *acquire(pool).await?)
        .await?;
    Ok(())
}

/// How many EHRs hold a subject reference that is not a UUID.
///
/// The rows a newly declared pseudonym namespace finds already stored, which
/// the guard cannot refuse after the fact and boot reports instead of
/// skipping (#3241).
///
/// # Errors
/// [`DbError::Sqlx`] when the count fails.
pub async fn non_pseudonym_subjects(pool: &PgPool) -> Result<i64, DbError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ehr WHERE subject_id IS NOT NULL \
         AND subject_id !~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'",
    )
    .fetch_one(&mut *acquire(pool).await?)
    .await?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `DbError` variant wrapping a cause keeps it as the source and out
    /// of its own message, so a chain printer shows it once (#3684).
    #[test]
    fn wrapped_causes_are_sources_and_not_repeated() {
        use std::error::Error as _;
        let error = DbError::from(sqlx::Error::Protocol("driver detail".to_owned()));
        let source = error.source().expect("the driver error is the source");
        assert!(source.downcast_ref::<sqlx::Error>().is_some());
        assert!(!error.to_string().contains("driver detail"), "{error}");

        let error = DbError::from(sqlx::migrate::MigrateError::VersionMissing(7));
        let source = error.source().expect("the migrator error is the source");
        assert!(
            source
                .downcast_ref::<sqlx::migrate::MigrateError>()
                .is_some()
        );
        assert!(!error.to_string().contains('7'), "{error}");

        let mismatch = SchemaMismatch::NeverMigrated {
            schema: "clinical".to_owned(),
        };
        let mismatch_text = mismatch.to_string();
        let error = DbError::SchemaNotReady(mismatch);
        let source = error.source().expect("the divergence is the source");
        assert!(source.downcast_ref::<SchemaMismatch>().is_some());
        assert!(!error.to_string().contains(&mismatch_text), "{error}");
    }

    /// A database holding linkage without party applies the party set as a
    /// companion; one holding both, or neither, applies only its own.
    #[test]
    fn the_linkage_set_brings_the_party_set_as_a_companion() {
        let group = |domains: Vec<Domain>| PreparationGroup {
            dsn: String::new(),
            credential: PreparationCredential::DomainMigrator,
            domains,
        };
        assert_eq!(
            group(vec![Domain::Linkage]).sets(),
            [Domain::Party, Domain::Linkage]
        );
        assert_eq!(
            group(vec![Domain::Linkage, Domain::Audit]).sets(),
            [Domain::Party, Domain::Linkage, Domain::Audit]
        );
        assert_eq!(
            group(vec![Domain::Party, Domain::Linkage]).sets(),
            [Domain::Party, Domain::Linkage]
        );
        assert_eq!(group(vec![Domain::Audit]).sets(), [Domain::Audit]);
        assert_eq!(group(Domain::ALL.to_vec()).sets(), Domain::ALL);
    }

    /// With no domain relocated and no domain migrator named, the plan is the
    /// one database `[db]` prepares, and it opens no connection to find that
    /// out (the DSN here reaches nothing).
    #[tokio::test]
    async fn a_co_located_layout_plans_one_database_without_connecting() {
        let settings = DbConfig::new("postgres://nobody@unreachable.invalid:1/none");
        let plan = preparation_plan(&settings, &StorageConfig::default())
            .await
            .expect("no connection is needed");
        assert_eq!(plan.len(), 1);
        let only = plan.first().expect("one group");
        assert_eq!(only.domains, Domain::ALL);
        assert_eq!(only.credential, PreparationCredential::Migrator);
    }

    #[test]
    fn defaults_applied() {
        let s = DbConfig::new("postgres://localhost/ferroehr");
        assert_eq!(s.url.expose(), "postgres://localhost/ferroehr");
        assert_eq!(s.max_connections, 20);
        assert_eq!(s.min_connections, 2);
        assert_eq!(s.acquire_timeout_secs, 30);
        assert!(!s.is_dev_default());
    }

    #[test]
    fn default_url_is_the_dev_dsn() {
        assert!(DbConfig::default().is_dev_default());
    }
}
