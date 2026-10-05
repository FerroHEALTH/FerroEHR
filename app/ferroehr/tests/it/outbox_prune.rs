// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outbox retention prune honours every active cursor reader (#3330).
//!
//! `event_outbox` is read by the AMQP drainer (which stamps `published_at`)
//! and by cursor readers with their own high-water mark. The prune used to
//! delete on age alone, so a row a lagging reader had not reached was gone
//! before it got there. Proven here against a real database: rows at or below
//! the lowest active cursor go once past the window, rows above it stay, and an
//! inactive reader holds nothing.

#![expect(
    clippy::expect_used,
    reason = "integration tests fail loudly on harness errors"
)]

use ferroehr::extensions::outbox::OutboxReader;
use ferroehr::extensions::outbox::{advance, cursor, prune, reconcile};
use ferroehr::service::FerroEhrService;
use sqlx::PgPool;

async fn seqs(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT seq FROM clinical.event_outbox ORDER BY seq")
        .fetch_all(pool)
        .await
        .expect("read outbox seqs")
}

/// Five EHR creations, each one published outbox row aged well past the
/// window.
async fn seed_published_rows(pool: &PgPool) -> Vec<i64> {
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool));
    for _ in 0..5 {
        svc.create_ehr(None).await.expect("create_ehr");
    }
    sqlx::query("UPDATE clinical.event_outbox SET published_at = now() - interval '30 days'")
        .execute(pool)
        .await
        .expect("back-date the published stamps");
    let seqs = seqs(pool).await;
    assert_eq!(seqs.len(), 5, "one outbox row per EHR creation");
    seqs
}

#[tokio::test]
async fn prune_stops_at_the_lowest_active_reader_cursor() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let seeded = seed_published_rows(&pool).await;

    // The reader is active and has processed the first two rows only.
    reconcile(&pool, OutboxReader::FHIR_OUTBOUND, true)
        .await
        .expect("reconcile active");
    advance(&pool, OutboxReader::FHIR_OUTBOUND, seeded[1])
        .await
        .expect("advance cursor");

    let pruned = prune(&pool, 7).await.expect("prune");
    assert_eq!(pruned, 2, "only the rows at or below the cursor are pruned");
    assert_eq!(
        seqs(&pool).await,
        seeded[2..],
        "every row the reader has not reached survives the window"
    );

    // The reader catches up: the rest goes on the next pass.
    advance(&pool, OutboxReader::FHIR_OUTBOUND, seeded[4])
        .await
        .expect("advance to the end");
    assert_eq!(prune(&pool, 7).await.expect("prune"), 3);
    assert!(seqs(&pool).await.is_empty());
}

#[tokio::test]
async fn an_inactive_reader_holds_no_floor() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let seeded = seed_published_rows(&pool).await;

    // A cursor left behind by a reader the configuration switched off.
    advance(&pool, OutboxReader::FHIR_OUTBOUND, seeded[0])
        .await
        .expect("advance cursor");
    reconcile(&pool, OutboxReader::FHIR_OUTBOUND, false)
        .await
        .expect("reconcile inactive");

    assert_eq!(prune(&pool, 7).await.expect("prune"), 5);
    assert!(seqs(&pool).await.is_empty());
    // Reconciling keeps the cursor: switching the reader back on resumes
    // where it left off.
    assert_eq!(
        cursor(&pool, OutboxReader::FHIR_OUTBOUND)
            .await
            .expect("cursor"),
        seeded[0]
    );
}

#[tokio::test]
async fn advance_is_monotonic_and_registers_the_reader_active() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    assert_eq!(
        cursor(&pool, OutboxReader::FHIR_OUTBOUND)
            .await
            .expect("cursor"),
        0,
        "an unregistered reader reads as zero"
    );
    advance(&pool, OutboxReader::FHIR_OUTBOUND, 10)
        .await
        .expect("advance");
    advance(&pool, OutboxReader::FHIR_OUTBOUND, 5)
        .await
        .expect("a lower seq is a no-op");
    assert_eq!(
        cursor(&pool, OutboxReader::FHIR_OUTBOUND)
            .await
            .expect("cursor"),
        10
    );
    let active: bool = sqlx::query_scalar(
        "SELECT active FROM clinical.event_outbox_reader WHERE reader = 'fhir-outbound'",
    )
    .fetch_one(&pool)
    .await
    .expect("read active");
    assert!(active, "a reader that advances is running, so it is active");
}
