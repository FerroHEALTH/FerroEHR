// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The statement budget of a composition read a care relationship decides
//! (#6251): Dr Jansen of the hospital scenario reads the patient's encounter
//! under the controller's policy that requires a care relationship, with the
//! `LISTEN` sessions of a running server connected. It is the posture the
//! capacity harness measures, and the one the read paths of
//! [`super::paths`] do not reach: the care check confirms the party store's
//! revisions in the request (#5806, #6048). No openEHR spec governs the
//! statements of a request: our own design.

use std::sync::atomic::Ordering;

use http::StatusCode;
use tracing::Instrument as _;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{
    CHAINER_RELEASE, CHAINER_ROUND, CHAINER_TAKE, REQUEST_SPAN, RoundTrips, SYSTEM_ROUND, calls,
    counted, drained, ledger,
};
use crate::common::{self, BASE};
use crate::hospital::world::{PURPOSE, World, credential};

/// The requests sent before the measured ones.
const WARM: u64 = 3;

/// The measured requests.
const REQUESTS: u64 = 10;

/// The round trips one care-gated read sends: the party store's revisions
/// and the version read, which carries the policy revision (#6251).
const ROUND_TRIPS: u64 = 2;

/// The statements nested in one care-gated read's round trips, the chainer's
/// rounds left out, measured over three runs (#6251).
const NESTED: u64 = 3;

/// Dr Jansen's read of the encounter, declaring the treatment purpose; its
/// status and body.
async fn read(world: &World) -> (StatusCode, String) {
    let req = common::req(
        "GET",
        format!(
            "{BASE}/ehr/{}/composition/{}",
            world.ehr, world.encounter.vo
        ),
    )
    .auth(credential("jansen"))
    .accept("application/json")
    .header(PURPOSE, "TREAT")
    .build();
    let (status, _, body) = common::send(&world.app, req).await;
    (status, body)
}

/// A care-gated composition read stays within its round trips and its
/// nested statements.
#[tokio::test]
async fn a_care_gated_read_stays_within_its_statement_budget() {
    let round_trips = RoundTrips::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(round_trips.clone()));
    let world = Box::pin(crate::hospital::build::world()).await;
    super::listening(&world.db, &world.svc).await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&world.pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    for _ in 0..WARM {
        let (status, body) = read(&world).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the care relationship admits: {body}"
        );
    }
    drained(&world.pool).await;
    let evicted = testkit::statements::statement_window(&world.pool)
        .await
        .expect("the statement counts reset");
    let sent_before = round_trips.0.load(Ordering::Relaxed);
    for _ in 0..REQUESTS {
        let (status, body) = read(&world)
            .instrument(tracing::info_span!(REQUEST_SPAN))
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the care relationship admits: {body}"
        );
        drained(&world.pool).await;
    }
    let sent = round_trips.0.load(Ordering::Relaxed) - sent_before;
    let statements = counted(&world.pool).await;
    testkit::statements::assert_statements_kept(&world.pool, evicted)
        .await
        .expect("no statement evicted while the test counted them");
    let nested: u64 = statements
        .iter()
        .filter(|s| !s.toplevel)
        .map(|s| s.calls)
        .sum();
    let chainer = calls(
        &statements,
        "SELECT head_seq, head_digest FROM audit.access_log_chain_take",
    ) * CHAINER_TAKE
        + calls(&statements, "SELECT audit.access_log_chain_move") * CHAINER_ROUND
        + calls(&statements, "SELECT audit.audit_event_chain_move") * SYSTEM_ROUND
        + calls(&statements, "SELECT audit.audit_intake_release") * CHAINER_RELEASE;
    let per_request = (
        sent.div_euclid(REQUESTS),
        nested.saturating_sub(chainer).div_euclid(REQUESTS),
    );
    let report = format!(
        "care-gated read: {} round trips (<= {ROUND_TRIPS}) and {} nested statements \
         (<= {NESTED}) per request, {chainer} nested statements of the chainer\n{}",
        per_request.0,
        per_request.1,
        ledger(&statements),
    );
    eprintln!("{report}");
    assert!(sent > 0, "the round trips are counted");
    assert!(
        per_request.0 <= ROUND_TRIPS && per_request.1 <= NESTED,
        "a care-gated read exceeds its statement budget:\n{report}"
    );
}
