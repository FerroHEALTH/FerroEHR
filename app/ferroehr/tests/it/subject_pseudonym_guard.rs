// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The database's own line of defence for the subject pseudonym (#3241), and
//! the cluster identity the deployment profile reads (#3226).
//!
//! The service refuses a non-UUID subject once namespaces are declared; the
//! trigger `ehr_subject_pseudonym_guard` refuses it at the row, whichever code
//! path writes, keyed on the posture the server stamps at boot. These tests
//! drive the stamp and the row directly, so the guard is proven without the
//! service's rule in the way.

use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::db;

/// Insert an EHR row with the given subject reference straight into `ehr`,
/// the way a repair session or a bypassing code path would.
async fn insert_ehr(pool: &PgPool, subject_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ehr (id, system_id, subject_id, subject_namespace, time_created) \
         VALUES ($1, 'guard-test', $2, 'urn:test:ns', now())",
    )
    .bind(Uuid::now_v7())
    .bind(subject_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// With the posture stamped `required`, a non-UUID subject is refused at the
/// row with the guard's own constraint name; a UUID passes; with `open`, the
/// same non-UUID subject is stored, as the released spec allows.
#[tokio::test]
async fn the_row_guard_follows_the_stamped_posture() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    db::stamp_subject_posture(&pool, true)
        .await
        .expect("stamp required");
    let refused = insert_ehr(&pool, "111222333") // privacy-allow: synthetic
        .await
        .expect_err("a non-UUID subject is refused once namespaces are declared");
    let database = refused
        .as_database_error()
        .expect("a database refusal, not a driver failure");
    assert_eq!(database.code().as_deref(), Some("23514"), "{database}");
    assert_eq!(
        database.constraint(),
        Some("ehr_subject_pseudonym_guard"),
        "{database}"
    );
    insert_ehr(&pool, &Uuid::now_v7().to_string())
        .await
        .expect("a UUID subject is a pseudonym and passes");

    db::stamp_subject_posture(&pool, false)
        .await
        .expect("stamp open");
    insert_ehr(&pool, "mrn-0042")
        .await
        .expect("with no namespace declared the released spec admits any identifier");
    assert_eq!(
        db::non_pseudonym_subjects(&pool).await.expect("count"),
        1,
        "the boot report counts the one stored non-pseudonym subject"
    );
}

/// The cluster identity is readable on the runtime credential and equal for two
/// pools that reach the same server, which is what makes the shared-cluster
/// check an honest one rather than a host-string comparison.
#[tokio::test]
async fn two_pools_on_one_server_report_one_cluster_identity() {
    let db = testkit::db().await.expect("testkit database");
    let a = db::cluster_identity(&db.pool())
        .await
        .expect("identity on pool a");
    let b = db::cluster_identity(&db::domain_pool_from(&db.pool(), db::domain::Domain::Party))
        .await
        .expect("identity on pool b");
    assert!(!a.is_empty());
    assert_eq!(a, b, "the same server answers with one system_identifier");
}
