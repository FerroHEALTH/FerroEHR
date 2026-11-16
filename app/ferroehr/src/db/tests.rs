// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The database error chain, the domain layout and the pool defaults.

use super::*;
use crate::db::pool::{POOL_CEILING, POOL_FLOOR, POOL_KEPT_FLOOR};

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

/// With no domain relocated and no domain migrator named, the plan is the
/// one database `[database]` prepares, and it opens no connection to find that
/// out (the DSN here reaches nothing).
#[tokio::test]
async fn a_co_located_layout_plans_one_database_without_connecting() {
    let settings = DbConfig::new("postgres://nobody@unreachable.invalid:1/none");
    let plan = preparation_plan(&settings)
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
    assert_eq!(s.max_connections, None);
    assert_eq!(s.min_connections, None);
    assert!((POOL_FLOOR..=POOL_CEILING).contains(&s.pool_size()));
    assert_eq!(s.pool_kept(), 0);
    assert_eq!(s.acquire_timeout, units::Duration::from_secs(30));
    assert!(!s.is_dev_default());
}

#[test]
fn default_url_is_the_dev_dsn() {
    assert!(DbConfig::default().is_dev_default());
}

/// The server bounds an unset pool size by what it admits at once and keeps
/// one connection per core of an unset floor open; a set size and a set
/// floor are kept, and a pool never keeps more than its size.
#[test]
fn the_server_sizes_an_unset_pool_for_its_concurrency_and_keeps_a_core_each_open() {
    let unset = DbConfig::new("postgres://localhost/ferroehr");
    let small = unset.for_serving(1);
    assert_eq!(
        (small.max_connections, small.min_connections),
        (Some(1), Some(1))
    );
    assert_eq!(unset.for_serving(0).max_connections, Some(1));
    let derived = unset.for_serving(256);
    assert_eq!(derived.max_connections, Some(unset.pool_size()));
    let cores = u32::try_from(std::thread::available_parallelism().map_or(1, usize::from))
        .expect("the cores fit u32");
    assert_eq!(
        derived.pool_kept(),
        cores.max(POOL_KEPT_FLOOR).min(unset.pool_size())
    );
    let mut set = unset.clone();
    set.max_connections = Some(50);
    set.min_connections = Some(80);
    assert_eq!(set.for_serving(4).max_connections, Some(50));
    assert_eq!(set.pool_kept(), 50);
    set.min_connections = Some(5);
    assert_eq!(set.for_serving(4).pool_kept(), 5);
}
