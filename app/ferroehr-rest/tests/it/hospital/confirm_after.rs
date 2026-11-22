// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An overlapped read confirms its policy copies after it ran (#6251): its
//! own statement reads the policy revision beside the rows it judges, and a
//! run decided on a copy behind that revision is dropped whole and run once
//! more behind the up-front confirm. On a replica that hears no notification,
//! a policy change made through another replica therefore decides the very
//! next read, the access log holds exactly one entry for it, with the outcome
//! the client got, and the run again is counted once. No openEHR spec governs
//! the copies: our own design.

use http::StatusCode;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ScopeMetrics, SumDataPoint};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use serde_json::{Value, json};

use crate::common::{BASE, EXT};
use crate::hospital::abac::answered;
use crate::hospital::build::world;
use crate::hospital::ledger::{Expect, Ledger};
use crate::hospital::replicas::deaf;
use crate::hospital::world::{
    CONFIDENTIALITY, ENCOUNTER, ROOT, World, call, call_matching, coded_in, text,
};

/// A meter provider whose access-path instruments land in `exporter`.
fn instruments() -> (SdkMeterProvider, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    ferroehr::telemetry::access_path::init(&provider.meter("ferroehr"));
    (provider, exporter)
}

/// The reads run again (`access_policy_reruns_total`), every operation summed.
fn reruns(provider: &SdkMeterProvider, exporter: &InMemoryMetricExporter) -> u64 {
    provider.force_flush().expect("flush the instruments");
    let finished = exporter.get_finished_metrics().expect("finished metrics");
    let Some(last) = finished.last() else {
        return 0;
    };
    last.scope_metrics()
        .flat_map(ScopeMetrics::metrics)
        .filter(|metric| metric.name() == ferroehr::telemetry::access_path::ACCESS_POLICY_RERUNS)
        .map(|metric| match metric.data() {
            AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                sum.data_points().map(SumDataPoint::value).sum::<u64>()
            }
            _ => 0,
        })
        .sum()
}

/// The controller's policy changed through the world's replica, the encounter
/// classified very restricted, which no rule clears for Dr. Jansen; the new
/// policy version.
async fn classify_the_encounter(world: &World) -> String {
    let uri = format!("{EXT}/admin/access_policy");
    let current = call(&world.app, ROOT, "GET", &uri, None, None).await;
    assert_eq!(
        current.status,
        StatusCode::OK,
        "the policy: {}",
        current.body
    );
    let mut policy: Value = current.json();
    let classification = json!({ "_type": "ACCESS_CLASSIFICATION", "name": text("Encounters"),
        "archetype_node_id": "at0002",
        "data": { "_type": "ACCESS_DATA_SCOPE",
            "archetypes": [{ "_type": "ARCHETYPE_ID", "value": ENCOUNTER }] },
        "sensitivities": [coded_in(CONFIDENTIALITY, "V")] });
    policy["classifications"]
        .as_array_mut()
        .expect("the policy classifies")
        .push(classification);
    let changed = call_matching(&world.app, ROOT, ("PUT", &uri), &world.policy, &policy).await;
    assert_eq!(
        changed.status,
        StatusCode::OK,
        "the change: {}",
        changed.body
    );
    changed.etag()
}

/// The read of the encounter by Dr. Jansen on `app`.
fn encounter(world: &World) -> String {
    format!(
        "{BASE}/ehr/{}/composition/{}",
        world.ehr, world.encounter.version
    )
}

/// A policy change the second replica never heard of refuses its next read:
/// the run decided on the old copy is dropped, the read runs once more on the
/// confirmed copy, and the refusal is the one answer and the one access-log
/// entry of the request.
#[tokio::test]
async fn a_read_decided_on_a_policy_copy_behind_the_store_runs_once_more() {
    let (provider, exporter) = instruments();
    let mut world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let uri = encounter(&world);
    let read_on_other = || call(&other, "jansen", "GET", &uri, Some("TREAT"), None);
    answered(&read_on_other().await, StatusCode::OK, None);
    let before = reruns(&provider, &exporter);
    answered(&read_on_other().await, StatusCode::OK, None);
    assert_eq!(
        reruns(&provider, &exporter),
        before,
        "a copy in force runs once"
    );

    world.policy = classify_the_encounter(&world).await;
    let refused = read_on_other().await;
    answered(&refused, StatusCode::FORBIDDEN, Some("refused-sensitivity"));
    assert!(
        !refused.body.contains("Cardiology encounter"),
        "the refusal carries nothing of the composition: {}",
        refused.body
    );
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the read ran once more, and only once"
    );
    answered(
        &read_on_other().await,
        StatusCode::FORBIDDEN,
        Some("refused-sensitivity"),
    );
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the confirmed copy decides the next read at once"
    );
    // The ledger's marker read goes to the world's own replica, whose copy its
    // own policy write left unconfirmed, so it is checked last.
    let mut ledger = Ledger::default();
    ledger.note(
        &world,
        "after the policy change",
        &refused,
        Expect::refused("refused-sensitivity", 6),
    );
    ledger.verify(&world).await;
}

/// A read whose handler answers `400` before any statement reads the policy
/// revision still confirms its copies with the one indexed read, and runs
/// once more when they were behind: the answer stays the `400`.
#[tokio::test]
async fn a_read_refused_before_its_statement_still_confirms_the_policy() {
    let (provider, exporter) = instruments();
    let mut world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let malformed = format!("{BASE}/ehr/{}/composition/not-a-uid", world.ehr);
    let bad = call(&other, "jansen", "GET", &malformed, Some("TREAT"), None).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
    let before = reruns(&provider, &exporter);
    let bad = call(&other, "jansen", "GET", &malformed, Some("TREAT"), None).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
    assert_eq!(
        reruns(&provider, &exporter),
        before,
        "a copy in force runs once"
    );

    world.policy = classify_the_encounter(&world).await;
    let bad = call(&other, "jansen", "GET", &malformed, Some("TREAT"), None).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the read of the revision found the copy behind"
    );
    let uri = encounter(&world);
    let refused = call(&other, "jansen", "GET", &uri, Some("TREAT"), None).await;
    answered(&refused, StatusCode::FORBIDDEN, Some("refused-sensitivity"));
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the copy the 400 confirmed decides the next read at once"
    );
}

/// The replica that writes a policy change confirms its own copies at the
/// revision its transaction produced, so its next read is decided on them at
/// once and runs once (#6265).
#[tokio::test]
async fn the_replica_that_writes_a_policy_change_reads_without_running_again() {
    let (provider, exporter) = instruments();
    let mut world = Box::pin(world()).await;
    let uri = encounter(&world);
    let read = || call(&world.app, "jansen", "GET", &uri, Some("TREAT"), None);
    answered(&read().await, StatusCode::OK, None);
    world.policy = classify_the_encounter(&world).await;
    let before = reruns(&provider, &exporter);
    let refused = call(&world.app, "jansen", "GET", &uri, Some("TREAT"), None).await;
    answered(&refused, StatusCode::FORBIDDEN, Some("refused-sensitivity"));
    assert_eq!(
        reruns(&provider, &exporter),
        before,
        "the writer's copy holds its own change"
    );
}
