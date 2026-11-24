// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A seeded template database, shared by the tests of one test binary build.
//!
//! [`seeded_db`] seeds a clone of the migrated template once, under an
//! advisory lock, renames it into a template named for the seed and the test
//! binary, and hands every caller a clone of that. nextest runs each test in a
//! process of its own, so the server, not the process, holds what the tests
//! share. A rebuilt binary seeds again; a seeded template older than the
//! template grace is swept like any template of another fingerprint. No
//! openEHR spec governs test infrastructure: our own design.

use std::hash::{Hash as _, Hasher as _};

use sqlx::{Connection as _, PgConnection};

use crate::{
    DB_PREFIX, TEMPLATE, TestDb, TestkitError, connect_ready, ensure_template, provision, server,
    template_complete, template_stamp,
};

/// Where a new test database comes from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Source<'a> {
    /// A clone of the migrated template.
    Migrated(&'a str),
    /// A clone of a seeded template, made under its seed's lock.
    Seeded(&'a str),
    /// An empty database.
    Empty,
}

/// The first key of every seed's advisory lock; the second comes from the
/// seed's key.
const SEED_LOCK_CLASS: i32 = 0x0EB2_7E58;

/// A fresh database holding what `seed` writes into a migrated clone, the
/// seed run once per `label` and test binary build on the shared server.
///
/// `seed` must leave no session on the database it seeds: it closes every
/// pool it opened on it, the [`TestDb::pool`] included, before it returns.
/// A database a test receives is its own, so a test may write to it.
///
/// # Errors
///
/// Returns [`TestkitError`] when the server cannot be acquired, the template
/// cannot be built or seeded, or the clone or its pool fails.
pub async fn seeded_db(
    label: &str,
    seed: impl AsyncFnOnce(&TestDb),
) -> Result<TestDb, TestkitError> {
    let server = server().await?;
    let migrated = TEMPLATE.get_or_try_init(|| ensure_template(server)).await?;
    let key = seed_key(label);
    let template = format!("{DB_PREFIX}tpl_seed_{key:016x}");
    let stamp = format!("seed-{key:016x}");
    let [.., a, b, c, d] = key.to_be_bytes();
    let lock = i32::from_be_bytes([a, b, c, d]);
    let mut admin = connect_ready(server).await?;
    sqlx::query("SELECT pg_advisory_lock($1, $2)")
        .bind(SEED_LOCK_CLASS)
        .bind(lock)
        .execute(&mut admin)
        .await?;
    let seeded = async {
        if !template_complete(&mut admin, &template, &stamp).await? {
            build(&mut admin, server, migrated, &template, &stamp, seed).await?;
        }
        provision(server, Source::Seeded(&template)).await
    }
    .await;
    drop(
        sqlx::query("SELECT pg_advisory_unlock($1, $2)")
            .bind(SEED_LOCK_CLASS)
            .bind(lock)
            .execute(&mut admin)
            .await,
    );
    drop(admin.close().await);
    seeded
}

/// Seeds a clone of `migrated` with `seed`, renames it to `template` and
/// stamps it with `stamp`, under the seed's lock.
async fn build(
    admin: &mut PgConnection,
    server: &str,
    migrated: &str,
    template: &str,
    stamp: &str,
    seed: impl AsyncFnOnce(&TestDb),
) -> Result<(), TestkitError> {
    let drop_sql = format!("DROP DATABASE IF EXISTS {template} WITH (FORCE)");
    sqlx::raw_sql(sqlx::AssertSqlSafe(drop_sql))
        .execute(&mut *admin)
        .await?;
    let staging = provision(server, Source::Migrated(migrated)).await?;
    seed(&staging).await;
    staging.pool.close().await;
    // The stamp lands while the seeded clone still carries its leased name, so
    // no sweep meets the template unstamped.
    let comment_sql = format!(
        "COMMENT ON DATABASE {} IS '{}'",
        staging.name,
        template_stamp(stamp, crate::now_secs())
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(comment_sql))
        .execute(&mut *admin)
        .await?;
    let rename_sql = format!("ALTER DATABASE {} RENAME TO {template}", staging.name);
    sqlx::raw_sql(sqlx::AssertSqlSafe(rename_sql))
        .execute(&mut *admin)
        .await?;
    drop(staging);
    Ok(())
}

/// Creates database `name` from the seeded `template`, which the caller's
/// seed lock keeps whole, or empty without one.
pub(crate) async fn clone(
    admin: &mut PgConnection,
    name: &str,
    template: Option<&str>,
) -> Result<(), sqlx::Error> {
    let sql = match template {
        Some(template) => format!("CREATE DATABASE {name} TEMPLATE {template}"),
        None => format!("CREATE DATABASE {name}"),
    };
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(admin)
        .await?;
    Ok(())
}

/// The key of the seed `label` for this test binary build: the label, the
/// migration fingerprint, and the size and modification time of the running
/// executable.
fn seed_key(label: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    label.hash(&mut hasher);
    ferroehr::db::migration_fingerprint().hash(&mut hasher);
    if let Ok(metadata) = std::env::current_exe().and_then(std::fs::metadata) {
        metadata.len().hash(&mut hasher);
        metadata
            .modified()
            .ok()
            .and_then(|modified| jiff::Timestamp::try_from(modified).ok())
            .map(jiff::Timestamp::as_nanosecond)
            .hash(&mut hasher);
    }
    hasher.finish()
}
