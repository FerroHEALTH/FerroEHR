// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Schema preparation under the separated-credential posture: which
//! credential [`ferroehr::db::prepare`] runs each half on, and what an
//! operator is told when it is the wrong one.
//!
//! These tests run the REAL boot sequence — the pools the binary opens, then
//! `db::prepare` over them — rather than assembling a router over a database
//! something else already migrated. That distinction is the whole point of
//! the module: preparation spans every schema (the DDL of all five migration
//! sets under `apply`, all five `_sqlx_migrations` bookkeeping tables under
//! `verify`), while each runtime credential of the documented posture holds
//! exactly one pseudonymisation domain, so a test that never boots cannot see
//! the failure at all.
//!
//! NOTE: no openEHR spec governs migration mechanics or database roles — our
//! own design/extension (GDPR Art. 4(5) and Art. 32(1)(a); the migrations
//! carry the derivation).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              fixture helpers and async bodies; a failing fixture must panic at \
              the fixture (the Rust Book ch11)"
)]

use crate::fixtures::{throwaway_password, with_role};
use ferroehr::config::deployment::DeploymentProfile;
use ferroehr::config::secret::SecretUrl;
use ferroehr::db::domain::{Domain, DomainDsn, StorageConfig};
use ferroehr::db::{DbConfig, DbError, MigrationMode};

/// `SQLSTATE` 42501 `insufficient_privilege` — what `PostgreSQL` reports for a
/// refused read, whether the missing grant is on the relation or on its schema
/// (`PostgreSQL` docs § Appendix A "`PostgreSQL` Error Codes", class 42).
const SQLSTATE_INSUFFICIENT_PRIVILEGE: &str = "42501";

/// A fresh non-superuser login role that is a member of `roles` (a
/// comma-separated `IN ROLE` list), returned as `(role name, DSN)`.
///
/// How a production deployment runs, and never as superuser, which would
/// bypass every privilege check these tests are about. Roles are
/// cluster-global on the shared testkit server, so the login role is named off
/// the clone's database name and the testkit sweep reaps it.
async fn login_role(db: &testkit::TestDb, suffix: &str, roles: &str) -> (String, String) {
    let login = format!("{}_{suffix}", db.name());
    let password = throwaway_password();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD '{password}' IN ROLE {roles}"
    )))
    .execute(&db.pool())
    .await
    .expect("create the login role");
    let dsn = with_role(db.url(), &login, &password);
    (login, dsn)
}

/// A DSN for a credential that can actually prepare the schema.
///
/// The clone's relations are owned by the harness role that built the
/// template, so a login role that is a MEMBER of it holds what
/// `ferroehr_migrator` holds in a deployment where that role owns them —
/// every schema, and no superuser attribute (which is an attribute, never
/// inherited: `PostgreSQL` 18, CREATE ROLE,
/// <https://www.postgresql.org/docs/18/sql-createrole.html>).
async fn migrator_dsn(db: &testkit::TestDb) -> String {
    let owner: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&db.pool())
        .await
        .expect("read the harness role");
    login_role(db, "prepmig", &owner).await.1
}

/// The real boot sequence on three credentials: one per pseudonymisation
/// domain plus the one that prepares the schema.
///
/// This is what the book recommends and the chart configures, and until
/// `[db] migrate_url` existed it could not get past boot at all: `verify`
/// failed on the FIRST migration set, because a role that is a member of
/// `ferroehr_clinical` and nothing else cannot read `ext._sqlx_migrations`.
#[tokio::test]
async fn the_boot_sequence_prepares_the_schema_on_separated_credentials() {
    let db = testkit::db().await.expect("testkit database");
    let settings = DbConfig {
        url: SecretUrl::new(login_role(&db, "prepclin", "ferroehr_clinical").await.1),
        migrate_url: Some(SecretUrl::new(migrator_dsn(&db).await)),
        migrate: MigrationMode::Verify,
        ..DbConfig::default()
    };
    let storage = StorageConfig {
        party: DomainDsn {
            url: Some(SecretUrl::new(
                login_role(&db, "prepdemo", "ferroehr_party").await.1,
            )),
            url_file: None,
        },
        ..StorageConfig::default()
    };
    assert!(
        storage.is_separated(Domain::Party) && settings.migrator_is_separated(),
        "the fixture must configure three DSNs, else this passes vacuously"
    );
    assert_ne!(
        settings.migrate_dsn(),
        settings.url.expose(),
        "and preparation must not be running as the clinical credential"
    );
    assert_ne!(
        settings.migrate_dsn(),
        storage.dsn(Domain::Party, &settings),
        "nor as the party one"
    );

    // The binary's own sequence: every runtime pool, then preparation.
    let pools = ferroehr::db::connect_domains(&settings, &storage)
        .await
        .expect("the domain pools connect");
    ferroehr::db::prepare(&settings, &storage, &pools, DeploymentProfile::Sandbox)
        .await
        .expect("`verify` prepares the schema on the migration credential");

    // And `apply` reaches the same state: on an already-migrated database it
    // is the bootstrap statements plus five no-op migrator runs, all of which
    // the migration credential may issue and neither runtime one may.
    let applying = DbConfig {
        migrate: MigrationMode::Apply,
        ..settings
    };
    ferroehr::db::prepare(&applying, &storage, &pools, DeploymentProfile::Sandbox)
        .await
        .expect("`apply` prepares the schema on the migration credential");

    // The sequence ends with two usable runtime pools, each on its own domain.
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM clinical.version")
        .fetch_one(&pools.clinical)
        .await
        .expect("the clinical pool reads its own domain");
    let parties: i64 = sqlx::query_scalar("SELECT count(*) FROM party.version")
        .fetch_one(&pools.party)
        .await
        .expect("the party pool reads its own domain");
    assert_eq!(
        (versions, parties),
        (0, 0),
        "a freshly prepared database carries no content"
    );
}

/// A single-DSN deployment is byte-identical to what it was: `migrate_url`
/// unset falls back to `url`, in both migration modes.
#[tokio::test]
async fn a_single_dsn_deployment_prepares_the_schema_as_before() {
    let db = testkit::db().await.expect("testkit database");
    let settings = DbConfig {
        migrate: MigrationMode::Verify,
        ..DbConfig::new(db.url())
    };
    assert!(
        !settings.migrator_is_separated(),
        "the fallback posture leaves `migrate_url` unset"
    );
    assert_eq!(
        settings.migrate_dsn(),
        settings.url.expose(),
        "and preparation then runs on the clinical DSN"
    );

    let pools = ferroehr::db::connect_domains(&settings, &StorageConfig::default())
        .await
        .expect("the pools connect");
    ferroehr::db::prepare(
        &settings,
        &StorageConfig::default(),
        &pools,
        DeploymentProfile::Sandbox,
    )
    .await
    .expect("`verify` prepares the schema on the one DSN");

    let applying = DbConfig {
        migrate: MigrationMode::Apply,
        ..settings
    };
    ferroehr::db::prepare(
        &applying,
        &StorageConfig::default(),
        &pools,
        DeploymentProfile::Sandbox,
    )
    .await
    .expect("`apply` prepares the schema on the one DSN");
}

/// A credential that cannot read a migration set is told which schema and
/// which role, not handed a bare 42501 about a table it has never heard of.
///
/// The refusal here is on the bookkeeping TABLE: a member of `ferroehr_clinical`
/// may enter `ext` (its helper functions are on the clinical search path) and
/// is refused the `SELECT`.
#[tokio::test]
async fn a_refused_bookkeeping_table_names_the_schema_and_the_role() {
    let db = testkit::db().await.expect("testkit database");
    let (role, dsn) = login_role(&db, "prepnoext", "ferroehr_clinical").await;
    let settings = DbConfig {
        migrate_url: Some(SecretUrl::new(dsn.clone())),
        migrate: MigrationMode::Verify,
        ..DbConfig::new(dsn)
    };

    let error = ferroehr::db::verify_schema(&settings, &StorageConfig::default())
        .await
        .expect_err("a clinical credential cannot read every migration set");

    let rendered = error.to_string();
    assert!(
        rendered.contains("migrate_url"),
        "the refusal must carry the remedy: {rendered}"
    );
    let DbError::SchemaUnreadable {
        schema,
        role: named,
        source,
    } = error
    else {
        panic!("a privilege refusal must not surface as a bare driver error: {rendered}")
    };
    assert_eq!(schema, "ext", "the first set it cannot read");
    assert_eq!(named, role, "the credential the operator has to change");
    assert!(
        rendered.contains("ext") && rendered.contains(&role),
        "and both must reach the operator's terminal: {rendered}"
    );
    let sqlx::Error::Database(refusal) = &source else {
        panic!("the cause must be PostgreSQL's own refusal: {source}")
    };
    assert_eq!(
        refusal.code().as_deref(),
        Some(SQLSTATE_INSUFFICIENT_PRIVILEGE),
        "and it must be the privilege refusal, kept as the cause: {source}"
    );
}

/// The same for a refusal on the SCHEMA rather than on the table — a distinct
/// statement in the check, and the shape a separated deployment hits on every
/// domain it does not own.
///
/// The fixture grants the clinical credential the two bookkeeping tables it
/// would otherwise stop at, so the check reaches `party`, which it cannot enter
/// at all.
#[tokio::test]
async fn a_refused_schema_names_the_schema_and_the_role() {
    let db = testkit::db().await.expect("testkit database");
    let (role, dsn) = login_role(&db, "prepnodem", "ferroehr_clinical").await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT ON ext._sqlx_migrations, clinical._sqlx_migrations TO {role}"
    )))
    .execute(&db.pool())
    .await
    .expect("grant the two sets this credential is allowed to see");
    let settings = DbConfig {
        migrate_url: Some(SecretUrl::new(dsn.clone())),
        migrate: MigrationMode::Verify,
        ..DbConfig::new(dsn)
    };

    let error = ferroehr::db::verify_schema(&settings, &StorageConfig::default())
        .await
        .expect_err("a clinical credential cannot enter the party schema");

    let rendered = error.to_string();
    let DbError::SchemaUnreadable {
        schema,
        role: named,
        source,
    } = error
    else {
        panic!("a privilege refusal must not surface as a bare driver error: {rendered}")
    };
    assert_eq!(schema, "party", "the first set it cannot reach");
    assert_eq!(named, role, "the credential the operator has to change");
    let sqlx::Error::Database(refusal) = &source else {
        panic!("the cause must be PostgreSQL's own refusal: {source}")
    };
    assert_eq!(
        refusal.code().as_deref(),
        Some(SQLSTATE_INSUFFICIENT_PRIVILEGE),
        "and it must be the privilege refusal, kept as the cause: {source}"
    );
}
