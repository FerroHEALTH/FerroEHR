// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Schema preparation for a storage domain in a database of its own: the
//! linkage domain prepared and verified in a second database, on that
//! database's `[storage.migrate.linkage]` credential.
//!
//! The second database comes from `testkit::empty_db()`, because preparing an
//! empty database IS the behaviour under test; the first is an ordinary
//! migrated clone.
//!
//! NOTE: no openEHR spec governs migration mechanics or database roles — our
//! own design/extension.

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

/// A fresh login role that is a member of `domain_role` and nothing else,
/// returned as `(login, password)`.
///
/// Roles are cluster-global on the shared testkit server, so one login reaches
/// both databases; it is named off the first clone's database name so the
/// testkit sweep reaps it.
async fn login(db: &testkit::TestDb, suffix: &str, domain_role: &str) -> (String, String) {
    let login = format!("{}_{suffix}", db.name());
    let password = throwaway_password();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD '{password}' IN ROLE {domain_role}"
    )))
    .execute(&db.pool())
    .await
    .expect("create the login role");
    (login, password)
}

/// The first database's DSN for every domain but linkage, and the second
/// database for linkage: on its runtime login, and on the harness credential
/// as that database's migrator.
fn relocated_linkage(runtime: &str, migrator: &str) -> StorageConfig {
    let mut storage = StorageConfig {
        linkage: DomainDsn {
            url: Some(SecretUrl::new(runtime)),
            url_file: None,
        },
        ..StorageConfig::default()
    };
    storage.migrate.linkage.url = Some(SecretUrl::new(migrator));
    storage
}

/// Whether `relation` exists in the database `pool` reaches.
async fn exists(pool: &sqlx::PgPool, relation: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(relation)
        .fetch_one(pool)
        .await
        .expect("probe the catalog")
}

/// The linkage domain alone in a second database is prepared under `apply`,
/// verified under `verify`, and boots under both.
///
/// The linkage grant file revokes a function only the party set creates, so
/// the second database receives the party set as a companion: its schema and
/// bookkeeping, and no clinical relation at all.
#[tokio::test]
async fn a_linkage_domain_in_its_own_database_is_prepared_and_verified() {
    let db = testkit::db().await.expect("testkit database");
    let second = testkit::empty_db().await.expect("a second, empty database");
    let (linkage_login, password) = login(&db, "rellink", "ferroehr_linkage").await;
    let storage = relocated_linkage(
        &with_role(second.url(), &linkage_login, &password),
        second.url(),
    );
    let applying = DbConfig {
        migrate: MigrationMode::Apply,
        ..DbConfig::new(db.url())
    };

    ferroehr::db::apply_schema(&applying, &storage)
        .await
        .expect("`apply` prepares the linkage domain in its own database");
    let relocated = second.pool();
    assert!(exists(&relocated, "linkage.subject_ehr").await);
    assert!(exists(&relocated, "linkage._sqlx_migrations").await);
    assert!(
        exists(&relocated, "party._sqlx_migrations").await,
        "the companion party set is applied beside linkage"
    );
    assert!(
        !exists(&relocated, "clinical.version").await,
        "and nothing of the clinical domain reaches the linkage database"
    );

    let verifying = DbConfig {
        migrate: MigrationMode::Verify,
        ..applying.clone()
    };
    ferroehr::db::verify_schema(&verifying, &storage)
        .await
        .expect("`verify` accepts the linkage domain in its own database");

    // The binary's own boot sequence, in both modes.
    let pools = ferroehr::db::connect_domains(&verifying, &storage)
        .await
        .expect("the domain pools connect");
    for settings in [&verifying, &applying] {
        ferroehr::db::prepare(settings, &storage, &pools, DeploymentProfile::Sandbox)
            .await
            .expect("the relocated layout boots");
    }
    let mapped: i64 = sqlx::query_scalar("SELECT count(*) FROM subject_ehr")
        .fetch_one(&pools.linkage)
        .await
        .expect("the linkage pool reads its own domain in the second database");
    assert_eq!(mapped, 0);
    pools.close().await;

    let inventory = ferroehr::db::erase_inventory(&applying, &storage)
        .await
        .expect("the erasure inventory reaches both databases");
    let linkage_database = inventory
        .databases
        .iter()
        .find(|database| database.domains == [Domain::Linkage])
        .expect("the second database is its own entry");
    assert_eq!(
        linkage_database.schemas,
        ["linkage", "party", "ext"],
        "an erasure drops the companion set's schema too"
    );
}

/// Without a migration credential for its database, the relocated domain is
/// verified on its runtime role, which holds one domain and cannot read the
/// `ext` bookkeeping; the refusal names the credential to set.
#[tokio::test]
async fn a_relocated_domain_without_its_migrator_names_the_remedy() {
    let db = testkit::db().await.expect("testkit database");
    let second = testkit::empty_db().await.expect("a second, empty database");
    let (linkage_login, password) = login(&db, "relnomig", "ferroehr_linkage").await;
    let prepared = relocated_linkage(
        &with_role(second.url(), &linkage_login, &password),
        second.url(),
    );
    let settings = DbConfig {
        migrate: MigrationMode::Apply,
        ..DbConfig::new(db.url())
    };
    ferroehr::db::apply_schema(&settings, &prepared)
        .await
        .expect("prepare the second database");

    let mut runtime_only = prepared;
    runtime_only.migrate.linkage.url = None;
    let error = ferroehr::db::verify_schema(
        &DbConfig {
            migrate: MigrationMode::Verify,
            ..settings
        },
        &runtime_only,
    )
    .await
    .expect_err("a linkage runtime role cannot read the ext bookkeeping");
    let rendered = error.to_string();
    let DbError::SchemaUnreadable { schema, role, .. } = error else {
        panic!("a privilege refusal must not surface as a bare driver error: {rendered}")
    };
    assert_eq!(schema, "ext");
    assert_eq!(role, linkage_login);
    assert!(
        rendered.contains("[storage.migrate.<domain>] url"),
        "the refusal names the relocated domain's migrator: {rendered}"
    );
}

/// A domain's migration credential must reach the database the domain is
/// served from; one that reaches another database is refused before anything
/// is prepared there.
#[tokio::test]
async fn a_migrator_reaching_another_database_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let second = testkit::empty_db().await.expect("a second, empty database");
    let storage = relocated_linkage(
        &format!("{}?application_name=linkage", db.url()),
        second.url(),
    );

    let error = ferroehr::db::apply_schema(&DbConfig::new(db.url()), &storage)
        .await
        .expect_err("the two linkage credentials reach two databases");
    assert!(
        matches!(
            error,
            DbError::DomainMigratorElsewhere {
                domain: Domain::Linkage
            }
        ),
        "{error}"
    );
    assert!(
        !exists(&second.pool(), "ext._sqlx_migrations").await,
        "and nothing was prepared in the database the migrator reaches"
    );
}
