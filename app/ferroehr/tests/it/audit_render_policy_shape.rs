// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Audit migration 0035 (#5660): a store that holds a render policy of the
//! shape before it, which states the removed `[audit.store]` retention
//! horizon, is refused with the digests it found and the remedy; an empty
//! store passes. No openEHR spec governs audit storage: our own design.

use sqlx::PgPool;

/// The migration under test.
const MIGRATION: &str = include_str!("../../migrations/audit/0035_render_policy_log_retention.sql");

/// Runs the migration once more over `pool`, as the migrator runs it.
async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path = audit, ext, public")
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(MIGRATION).execute(&mut *tx).await?;
    tx.commit().await
}

/// A migrated store, which holds no render policy of the earlier shape,
/// passes the migration again.
#[tokio::test]
async fn an_empty_store_passes() {
    let db = testkit::db().await.expect("testkit database");
    migrate(&db.pool()).await.expect("an empty store passes");
}

/// A render policy that states the store horizon is refused by its digest,
/// with the remedy in the hint.
#[tokio::test]
async fn a_store_with_an_earlier_render_policy_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let earlier = serde_json::json!({
        "source_id": "cdr-1",
        "horizon": { "unit": "years", "value": 5 },
    });
    let hex: String = sqlx::query_scalar("SELECT encode(audit.register_render_policy($1), 'hex')")
        .bind(earlier)
        .fetch_one(&pool)
        .await
        .expect("register the earlier policy");
    let refused = migrate(&pool)
        .await
        .expect_err("the earlier policy is refused");
    let sqlx::Error::Database(error) = &refused else {
        panic!("a database refusal: {refused}");
    };
    assert!(error.message().contains(&hex), "{}", error.message());
    assert_eq!(
        error.code().as_deref(),
        Some("55000"),
        "object_not_in_prerequisite_state"
    );
    let hint = error
        .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
        .and_then(sqlx::postgres::PgDatabaseError::hint)
        .unwrap_or_default();
    assert!(hint.contains("empty audit store"), "{hint}");
}
