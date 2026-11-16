// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The registered template addresses a replica holds in memory (#5775): a
//! registration is seen at once, and a release on any replica reaches the
//! copy through its notification.

use std::time::Duration;

use ferroehr::storage::template_repo::{register_template_ref, release_template_ref};

/// Waits until `holds`, at most ten seconds, naming `what` on a timeout.
async fn wait_until(holds: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !holds() {
        assert!(std::time::Instant::now() < deadline, "{what}: timed out");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn a_registration_holds_at_once_and_a_release_reaches_the_copy() {
    let (db, svc) = crate::fixtures::service().await;
    let pool = db.pool();
    let registered = svc.registered_templates();
    crate::policy_fixtures::listen(&db, &[std::sync::Arc::clone(registered.listened())]);
    wait_until(
        || registered.listened().is_listening(),
        "the listener connects",
    )
    .await;

    assert!(
        !registered
            .holds("Probe.Registered.v1")
            .await
            .expect("asked"),
        "an unknown address is not registered"
    );
    let mut tx = pool.begin().await.expect("a transaction");
    register_template_ref(&mut tx, "Probe.Registered.v1")
        .await
        .expect("registered");
    tx.commit().await.expect("committed");
    assert!(
        registered
            .holds("probe.registered.V1")
            .await
            .expect("asked"),
        "a registration holds at once, compared case-insensitively"
    );

    let seen = registered.listened().notifications();
    let mut tx = pool.begin().await.expect("a transaction");
    release_template_ref(&mut tx, "Probe.Registered.v1")
        .await
        .expect("released");
    tx.commit().await.expect("committed");
    wait_until(
        || registered.listened().notifications() > seen,
        "the release notifies",
    )
    .await;
    assert!(
        !registered
            .holds("Probe.Registered.v1")
            .await
            .expect("asked"),
        "a released address is no longer held"
    );
}
