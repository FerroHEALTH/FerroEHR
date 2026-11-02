// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outbox retention prune deletes the published rows past the window and
//! nothing else (#3330, #4766).
//!
//! `event_outbox` is drained by the AMQP publisher alone, which stamps
//! `published_at`; no cursor reader holds rows back. Proven here against a
//! real database: a published row past the window goes, while a published row
//! inside the window and a row the broker has not confirmed both stay.

#![expect(
    clippy::expect_used,
    reason = "integration tests fail loudly on harness errors"
)]

use ferroehr::extensions::outbox::prune;
use ferroehr::service::FerroEhrService;
use sqlx::PgPool;

async fn seqs(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT seq FROM clinical.event_outbox ORDER BY seq")
        .fetch_all(pool)
        .await
        .expect("read outbox seqs")
}

/// Five EHR creations, each one unpublished outbox row.
async fn seed_rows(pool: &PgPool) -> Vec<i64> {
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool));
    for _ in 0..5 {
        crate::fixtures::create_ehr(&svc).await;
    }
    let seqs = seqs(pool).await;
    assert_eq!(seqs.len(), 5, "one outbox row per EHR creation");
    seqs
}

/// Stamps the rows `seqs` published `age_days` ago.
async fn publish(pool: &PgPool, seqs: &[i64], age_days: i64) {
    sqlx::query(
        "UPDATE clinical.event_outbox \
         SET published_at = now() - make_interval(days => $2::int) \
         WHERE seq = ANY($1)",
    )
    .bind(seqs)
    .bind(age_days)
    .execute(pool)
    .await
    .expect("stamp the published rows");
}

#[tokio::test]
async fn every_published_row_past_the_window_is_pruned() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let seeded = seed_rows(&pool).await;
    publish(&pool, &seeded, 30).await;

    assert_eq!(prune(&pool, 7).await.expect("prune"), 5);
    assert_eq!(seqs(&pool).await, Vec::<i64>::new());
}

#[tokio::test]
async fn a_row_inside_the_window_or_unpublished_stays() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let seeded = seed_rows(&pool).await;
    // Two published past the window, one published inside it, two the broker
    // has not confirmed.
    publish(&pool, &seeded[..2], 30).await;
    publish(&pool, &seeded[2..3], 1).await;

    assert_eq!(prune(&pool, 7).await.expect("prune"), 2);
    assert_eq!(
        seqs(&pool).await,
        seeded[2..],
        "a row inside the window and an unpublished row survive the prune"
    );
}
