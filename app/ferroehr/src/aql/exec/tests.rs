// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The served EHRs of a result page (#6522): each EHR once per row that
//! names it, in the order of first appearance, linear in the EHRs an
//! aggregate row names.

use super::*;

/// The id `n` of the test.
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[tokio::test]
async fn a_row_counts_each_ehr_once_in_the_order_of_first_appearance() {
    let db = testkit::db().await.expect("testkit database");
    let rows =
        sqlx::query("SELECT * FROM (VALUES ($1::uuid, $2::uuid), ($2, $2), ($3, NULL)) AS r(a, b)")
            .bind(id(1))
            .bind(id(2))
            .bind(id(3))
            .fetch_all(&db.pool())
            .await
            .expect("the rows");
    let cols = ["a".to_owned(), "b".to_owned()];
    assert_eq!(
        served_ehrs(&rows, &cols, false).expect("the served EHRs"),
        vec![(id(1), 1), (id(2), 2), (id(3), 1)],
        "an EHR named twice in a row counts once, a NULL names none"
    );
}

#[tokio::test]
async fn an_aggregate_row_of_many_ehrs_counts_each_once() {
    let db = testkit::db().await.expect("testkit database");
    let many: Vec<Uuid> = (1..=50_000).map(id).collect();
    let rows = sqlx::query("SELECT $1::uuid[] || $1::uuid[] AS a")
        .bind(&many)
        .fetch_all(&db.pool())
        .await
        .expect("the row");
    let started = std::time::Instant::now();
    let served = served_ehrs(&rows, &["a".to_owned()], true).expect("the served EHRs");
    let took = started.elapsed();
    assert_eq!(
        served,
        many.iter().map(|ehr| (*ehr, 1)).collect::<Vec<_>>(),
        "each EHR of the aggregate counts once, in order"
    );
    assert!(
        took < std::time::Duration::from_secs(1),
        "100,000 EHRs in one row are counted in linear time: {took:?}"
    );
}
