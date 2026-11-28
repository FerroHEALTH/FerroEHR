// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Replicas that boot together against one empty database each prepare it:
//! every preparation succeeds and the database carries each migration once.
//!
//! NOTE: no openEHR spec governs migration mechanics: our own design (#6744).

use ferroehr::db::{DbConfig, MigrationMode, apply_schema};

/// Four preparations started at once against one empty database, as four
/// replicas of a fresh deployment run them.
#[tokio::test]
async fn replicas_preparing_one_empty_database_together_all_succeed() {
    let db = testkit::empty_db().await.expect("an empty database");
    let applying = DbConfig {
        migrations: MigrationMode::Apply,
        ..DbConfig::new(db.url())
    };

    let roles = testkit::roles::lock().await.expect("the roles lock");
    // One task polls the four, so their connections interleave on the server
    // as four processes' would.
    let outcomes = tokio::join!(
        apply_schema(&applying),
        apply_schema(&applying),
        apply_schema(&applying),
        apply_schema(&applying),
    );
    roles.release().await;

    for outcome in [outcomes.0, outcomes.1, outcomes.2, outcomes.3] {
        outcome.expect("every replica's preparation succeeds");
    }
    ferroehr::db::verify_schema(&DbConfig {
        migrations: MigrationMode::Verify,
        ..applying
    })
    .await
    .expect("the database carries exactly this build's migrations");
}
