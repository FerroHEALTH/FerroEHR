// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An overlapped read confirms its policy copies after it ran (#6251): its
//! own statement reads the policy revision beside the rows it judges, and a
//! run decided on a copy behind that revision is dropped whole and run once
//! more behind the up-front confirm. On a replica that hears no notification,
//! a policy change made through another replica therefore decides the very
//! next read, the access log holds exactly one entry for it, with the outcome
//! the client got, and the run again is counted once. No openEHR spec governs
//! the copies: our own design. A commit confirms after it was decided the
//! same way, before it writes (#6256).

use http::StatusCode;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ScopeMetrics, SumDataPoint};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use serde_json::{Value, json};

use crate::common::{BASE, EXT};
use crate::hospital::abac::answered;
use crate::hospital::build::{composition, world};
use crate::hospital::ledger::{Expect, Ledger};
use crate::hospital::replicas::deaf;
use crate::hospital::world::{
    CONFIDENTIALITY, ENCOUNTER, ROOT, SCIM, World, call, call_matching, coded_in, patch_op, scim,
    text,
};

/// A meter provider whose access-path instruments land in `exporter`.
fn instruments() -> (SdkMeterProvider, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    ferroehr::telemetry::access_path::init(&provider.meter("ferroehr"));
    ferroehr::telemetry::metrics::init(&provider.meter("ferroehr"));
    (provider, exporter)
}

/// The points of the counter `name` whose attributes hold `attribute`, or
/// every point when it is `None`, summed.
fn counted(
    provider: &SdkMeterProvider,
    exporter: &InMemoryMetricExporter,
    name: &str,
    attribute: Option<(&str, &str)>,
) -> u64 {
    provider.force_flush().expect("flush the instruments");
    let finished = exporter.get_finished_metrics().expect("finished metrics");
    let Some(last) = finished.last() else {
        return 0;
    };
    last.scope_metrics()
        .flat_map(ScopeMetrics::metrics)
        .filter(|metric| metric.name() == name)
        .map(|metric| match metric.data() {
            AggregatedMetrics::U64(MetricData::Sum(sum)) => sum
                .data_points()
                .filter(|point| {
                    attribute.is_none_or(|(key, value)| {
                        point
                            .attributes()
                            .any(|kv| kv.key.as_str() == key && kv.value.as_str() == value)
                    })
                })
                .map(SumDataPoint::value)
                .sum::<u64>(),
            _ => 0,
        })
        .sum()
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

/// The compositions of the world's EHR, as the controller's query reads them
/// on the world's own replica.
async fn compositions(world: &World) -> usize {
    let body = json!({
        "q": format!(
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = '{}'",
            world.ehr
        )
    });
    let answer = call(
        &world.app,
        ROOT,
        "POST",
        &format!("{BASE}/query/aql"),
        None,
        Some(&body),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "the query: {}", answer.body);
    answer.json()["rows"].as_array().expect("rows").len()
}

/// A commit decided on a policy copy behind the store is refused before it
/// writes, by the revision its own gate read returns, and runs once more
/// behind the up-front confirm (#6256): the encounter the change classified
/// very restricted is refused, nothing of the first run is stored, and the
/// run again is counted once.
#[tokio::test]
async fn a_commit_decided_on_a_policy_copy_behind_the_store_runs_once_more() {
    let (provider, exporter) = instruments();
    let mut world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let uri = format!("{BASE}/ehr/{}/composition", world.ehr);
    let body = composition(ENCOUNTER, "Follow-up encounter", "Dr. Anna Jansen");
    let commit = || call(&other, "jansen", "POST", &uri, Some("TREAT"), Some(&body));
    let landed = commit().await;
    assert_eq!(landed.status, StatusCode::CREATED, "{}", landed.body);
    let before = reruns(&provider, &exporter);
    let landed = commit().await;
    assert_eq!(landed.status, StatusCode::CREATED, "{}", landed.body);
    assert_eq!(
        reruns(&provider, &exporter),
        before,
        "a copy in force commits once"
    );
    let stored = compositions(&world).await;

    world.policy = classify_the_encounter(&world).await;
    let refused = commit().await;
    answered(&refused, StatusCode::FORBIDDEN, Some("refused-sensitivity"));
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the commit ran once more, and only once"
    );
    assert_eq!(
        compositions(&world).await,
        stored,
        "neither run stored the refused encounter"
    );
    answered(
        &commit().await,
        StatusCode::FORBIDDEN,
        Some("refused-sensitivity"),
    );
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the confirmed copy decides the next commit at once"
    );
}

/// The records of the request `request_id` in the system log and the access
/// log together, as `(store, outcome)`, once one has landed, or none after
/// ten seconds.
async fn recorded(world: &World, request_id: &str) -> Vec<(String, i16)> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let rows: Vec<(String, i16)> = sqlx::query_as(
            "SELECT 'system', outcome FROM audit.audit_event WHERE request_id = $1 \
             UNION ALL SELECT 'access', COALESCE(event_outcome, -1)::int2 FROM audit.access_log_record \
             WHERE request_id = $1",
        )
        .bind(request_id)
        .fetch_all(&world.pool)
        .await
        .expect("read the logs");
        if !rows.is_empty() || std::time::Instant::now() >= deadline {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// A read decided on a party copy the store overtook runs once more and
/// leaves one record (#6320): SCIM's deactivation of the nurse, committed
/// through the world's replica, refuses her very next read on a replica that
/// hears nothing. The run that took her held binding lookup read the
/// composition, its clinical statement read the party store's revisions, and
/// the run was dropped: the refusal of the run again is the one answer and
/// the one record of the request, and nothing of the composition leaves.
#[tokio::test]
async fn a_read_decided_on_a_party_copy_behind_the_store_runs_once_more() {
    let (provider, exporter) = instruments();
    let world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let path = format!("{BASE}/ehr/{}/composition/{}", world.ehr, world.vitals.vo);
    let read = || call(&other, "devries", "GET", &path, Some("TREAT"), None);
    for _ in 0..2 {
        answered(&read().await, StatusCode::OK, None);
    }
    let before = reruns(&provider, &exporter);
    answered(&read().await, StatusCode::OK, None);
    assert_eq!(
        reruns(&provider, &exporter),
        before,
        "a copy in force runs once"
    );

    let user = format!("/Users/{}", world.people["devries"].person);
    let body = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let (status, scimmed) = scim(&world.app, SCIM, "PATCH", &user, Some(&body)).await;
    assert_eq!(status, StatusCode::OK, "the deactivation: {scimmed}");
    let refused = read().await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert!(
        !refused.body.contains("archetype_node_id"),
        "the refusal carries nothing of the composition: {}",
        refused.body
    );
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the read ran once more, and only once"
    );
    let records = recorded(&world, &refused.request_id()).await;
    assert_eq!(records.len(), 1, "one record of the request: {records:?}");
    assert_eq!(
        records[0].0, "access",
        "the record is the access-log entry: {records:?}"
    );
    assert_ne!(
        records[0].1, 0,
        "the entry records the run again's refusal: {records:?}"
    );
}

/// Authentication run again leaves what one clean run leaves (#6320): SCIM's
/// deactivation of the nurse, committed through the world's replica, refuses
/// her next read on a replica that hears nothing. The run that took her held
/// binding lookup is dropped with what it counted, so the refusal counts one
/// authentication failure and no lookup hit, and the lookup the run again
/// read decides her next request with no run again.
#[tokio::test]
async fn authentication_run_again_counts_and_holds_as_one_clean_run() {
    let (provider, exporter) = instruments();
    let world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let path = format!("{BASE}/ehr/{}/composition/{}", world.ehr, world.vitals.vo);
    let read = || call(&other, "devries", "GET", &path, Some("TREAT"), None);
    for _ in 0..2 {
        answered(&read().await, StatusCode::OK, None);
    }
    let failures = |provider, exporter| {
        counted(
            provider,
            exporter,
            ferroehr::telemetry::metrics::AUTH_FAILURES,
            None,
        )
    };
    let hits = |provider, exporter| {
        counted(
            provider,
            exporter,
            ferroehr::telemetry::access_path::ACCESS_COPY_LOOKUPS,
            Some(("result", "hit")),
        )
    };
    let (reruns_before, failed_before, hits_before) = (
        reruns(&provider, &exporter),
        failures(&provider, &exporter),
        hits(&provider, &exporter),
    );

    let user = format!("/Users/{}", world.people["devries"].person);
    let body = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let (status, scimmed) = scim(&world.app, SCIM, "PATCH", &user, Some(&body)).await;
    assert_eq!(status, StatusCode::OK, "the deactivation: {scimmed}");
    let refused = read().await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert_eq!(
        reruns(&provider, &exporter),
        reruns_before + 1,
        "the read ran once more"
    );
    assert_eq!(
        failures(&provider, &exporter),
        failed_before + 1,
        "one authentication failure, the run again's"
    );
    assert!(
        hits(&provider, &exporter) <= hits_before + 1,
        "only the run again's lookup is counted, never the dropped run's"
    );

    let again = read().await;
    assert_eq!(again.status, StatusCode::FORBIDDEN, "{}", again.body);
    assert_eq!(
        reruns(&provider, &exporter),
        reruns_before + 1,
        "the lookup the run again read decides the next request at once"
    );
    assert_eq!(failures(&provider, &exporter), failed_before + 2);
}

/// The compositions of the world's EHR as Dr. Jansen's query reads them on
/// `app`.
async fn compositions_read_by_jansen(world: &World, app: &axum::Router) -> usize {
    let body = json!({
        "q": format!(
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = '{}'",
            world.ehr
        )
    });
    let answer = call(
        app,
        "jansen",
        "POST",
        &format!("{BASE}/query/aql"),
        Some("TREAT"),
        Some(&body),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "the query: {}", answer.body);
    answer.json()["rows"].as_array().expect("rows").len()
}

/// A query decided on a policy copy behind the store is confirmed by the
/// revisions its own statement read, and runs once more behind the up-front
/// confirm (#6320): the encounter the change classified very restricted is
/// withheld from the very next query, and the run again is counted once.
#[tokio::test]
async fn a_query_decided_on_a_policy_copy_behind_the_store_runs_once_more() {
    let (provider, exporter) = instruments();
    let mut world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let served = compositions_read_by_jansen(&world, &other).await;
    assert!(served > 0, "the query serves the EHR's compositions");
    let before = reruns(&provider, &exporter);
    assert_eq!(compositions_read_by_jansen(&world, &other).await, served);
    assert_eq!(
        reruns(&provider, &exporter),
        before,
        "a copy in force runs once"
    );

    world.policy = classify_the_encounter(&world).await;
    let withheld = compositions_read_by_jansen(&world, &other).await;
    assert!(
        withheld < served,
        "the change withholds the encounter at once: {withheld} of {served}"
    );
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the query ran once more, and only once"
    );
    assert_eq!(compositions_read_by_jansen(&world, &other).await, withheld);
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the confirmed copy decides the next query at once"
    );
}

/// An `EHR_STATUS` read confirms its policy copies by the revisions its own
/// version read selects (#6320): a copy in force answers in one run, and a
/// policy change it never heard of runs the read once more.
#[tokio::test]
async fn an_ehr_status_read_confirms_the_policy_on_its_own_statement() {
    let (provider, exporter) = instruments();
    let mut world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let uri = format!("{BASE}/ehr/{}/ehr_status", world.ehr);
    let read = || call(&other, "jansen", "GET", &uri, Some("TREAT"), None);
    let first = read().await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let before = reruns(&provider, &exporter);
    assert_eq!(read().await.status, StatusCode::OK);
    assert_eq!(
        reruns(&provider, &exporter),
        before,
        "a copy in force runs once"
    );

    world.policy = classify_the_encounter(&world).await;
    let again = read().await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the read ran once more, and only once"
    );
    assert_eq!(read().await.status, StatusCode::OK);
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the confirmed copy decides the next read at once"
    );
}

/// A query by `POST` decided on a party copy the store overtook runs once
/// more with the same body and leaves one record (#6320): SCIM's
/// deactivation of the nurse, committed through the world's replica, refuses
/// her very next query on a replica that hears nothing. The run that took her
/// held binding lookup was dropped, and the run again read the query from the
/// bytes the first run read: a refusal, never a `400` for a missing body.
#[tokio::test]
async fn a_query_decided_on_a_party_copy_behind_the_store_runs_once_more() {
    let (provider, exporter) = instruments();
    let world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let body = json!({
        "q": format!(
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = '{}'",
            world.ehr
        )
    });
    let uri = format!("{BASE}/query/aql");
    let query = || call(&other, "devries", "POST", &uri, Some("TREAT"), Some(&body));
    for _ in 0..2 {
        answered(&query().await, StatusCode::OK, None);
    }
    let before = reruns(&provider, &exporter);
    answered(&query().await, StatusCode::OK, None);
    assert_eq!(
        reruns(&provider, &exporter),
        before,
        "a copy in force runs once"
    );

    let user = format!("/Users/{}", world.people["devries"].person);
    let deactivate = patch_op(json!([{ "op": "replace", "path": "active", "value": false }]));
    let (status, scimmed) = scim(&world.app, SCIM, "PATCH", &user, Some(&deactivate)).await;
    assert_eq!(status, StatusCode::OK, "the deactivation: {scimmed}");
    let refused = query().await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the query ran once more, and only once"
    );
    let records = recorded(&world, &refused.request_id()).await;
    assert_eq!(records.len(), 1, "one record of the request: {records:?}");
    assert_ne!(
        records[0].1, 0,
        "the entry records the run again's refusal: {records:?}"
    );
}

/// A party read confirms its policy copies by the revisions its own party
/// statement reads through the clinical door (#6320): a copy in force
/// answers in one run, and a policy change it never heard of runs the read
/// once more, with the answer it gave before.
#[tokio::test]
async fn a_party_read_confirms_the_policy_on_its_own_statement() {
    let (provider, exporter) = instruments();
    let mut world = Box::pin(world()).await;
    let (other, _drain, _elsewhere) = deaf(&world).await;
    let uri = format!("{BASE}/demographic/person/{}", world.eva);
    let read = || call(&other, "jansen", "GET", &uri, Some("TREAT"), None);
    let first = read().await;
    let before = reruns(&provider, &exporter);
    assert_eq!(read().await.status, first.status, "{}", first.body);
    assert_eq!(
        reruns(&provider, &exporter),
        before,
        "a copy in force runs once"
    );

    world.policy = classify_the_encounter(&world).await;
    let again = read().await;
    assert_eq!(again.status, first.status, "{}", again.body);
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the read ran once more, and only once"
    );
    assert_eq!(read().await.status, first.status);
    assert_eq!(
        reruns(&provider, &exporter),
        before + 1,
        "the confirmed copy decides the next read at once"
    );
}
