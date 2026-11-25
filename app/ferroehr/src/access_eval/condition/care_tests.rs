// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A care decision on a caller facts copy that never catches up with the
//! store is refused, never taken (#5806). Our own design.

use std::sync::Arc;

use openehr_rm::draft::access_control::terminology::{AccessDefault, CallerKind, OperationClass};
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics, SumDataPoint,
};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

use super::CareUnsettled;
use crate::access_eval::condition::{Asking, InForce, RequestAccess};
use crate::access_eval::procedure::Caller;
use crate::access_eval::scope::CategoryIndex;
use crate::access_eval::stale::STALE_WAIT;
use crate::error::ServiceError;
use crate::party_binding::grants::caller::facts::CallerFacts;
use crate::party_binding::grants::live::LiveGrants;
use crate::party_binding::grants::live::revisions::{FactsRead, Revisions};
use crate::service::FerroEhrService;
use crate::service::ehr::access_rule::SettingsCaller;
use crate::telemetry::access_path::ACCESS_CARE_STALE;

/// A professional's request judged under `grants`.
fn request(grants: Arc<LiveGrants>) -> RequestAccess {
    RequestAccess::new(
        Caller {
            kind: CallerKind::Professional,
            party: None,
            own_ehrs: Vec::new(),
            patient_grants_only: false,
            acts_for: Vec::new(),
            facts: CallerFacts::default(),
            administrator: false,
            settings: SettingsCaller::new(false, false, None),
            lacks_consent_reference: false,
        },
        Asking {
            operation: OperationClass::Read,
            purpose: None,
            reason: None,
            opt_out_override: false,
            episodes: None,
        },
        InForce {
            policy: None,
            categories: CategoryIndex::default(),
            server_default: AccessDefault::Open,
            now: jiff::Timestamp::now(),
            grants: Some(grants),
        },
    )
}

/// The sum of `access_care_stale_total` with `outcome` in `exporter`.
fn counted(exporter: &InMemoryMetricExporter, outcome: &str) -> u64 {
    let finished = exporter.get_finished_metrics().expect("finished metrics");
    let mut sum = 0;
    for metric in finished
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
        .filter(|m| m.name() == ACCESS_CARE_STALE)
    {
        if let AggregatedMetrics::U64(MetricData::Sum(data)) = metric.data() {
            sum += data
                .data_points()
                .filter(|point| {
                    point
                        .attributes()
                        .any(|kv| kv.key.as_str() == "outcome" && kv.value.as_str() == outcome)
                })
                .map(SumDataPoint::value)
                .sum::<u64>();
        }
    }
    sum
}

/// A copy that stays older than the store for the whole wait refuses the
/// request with the `503` of a refreshing access state, which the REST layer
/// answers with `Retry-After`, and counts the refusal.
#[tokio::test]
async fn a_copy_that_never_catches_up_refuses_the_care_decision() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    crate::telemetry::access_path::init(&provider.meter("ferroehr"));
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&crate::db::domain::DomainPools::from_shared(&db.pool()));
    let grants = Arc::clone(svc.grants());
    grants.refresh().await.expect("the copy reads");
    assert!(
        request(Arc::clone(&grants))
            .care_in_force(std::future::ready(()))
            .await
            .expect("the copy holds the store's revisions"),
        "a copy in force decides at once"
    );

    sqlx::query(
        "INSERT INTO party.binding_caller_fact_party (party_id, stamped_xid) \
         VALUES (uuidv7(), pg_current_xact_id())",
    )
    .execute(&db.pool())
    .await
    .expect("another replica commits a change");
    grants.hold_back();
    let started = std::time::Instant::now();
    let refused = request(grants)
        .care_in_force(std::future::ready(()))
        .await
        .expect_err("a copy that never catches up is refused");
    assert!(
        started.elapsed() >= STALE_WAIT,
        "the request read the copy again for the whole wait first"
    );
    assert!(matches!(refused, CareUnsettled::Refreshing), "{refused:?}");
    let refused = ServiceError::from(refused);
    assert!(
        matches!(refused, ServiceError::AccessRefreshing(_)),
        "the 503 of a refreshing access state: {refused:?}"
    );
    provider.force_flush().expect("flush");
    assert_eq!(counted(&exporter, "refused"), 1, "the refusal is counted");
    assert_eq!(counted(&exporter, "judged_again"), 0);
}

/// A copy that a later read of the store's revisions finds current decides,
/// though the read that brought it there began before the store's revisions
/// the request first read: those can name a facts read the copy never
/// installs once a later one already covers every stamp (#6527).
#[tokio::test]
async fn a_copy_a_later_read_finds_current_decides() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    crate::telemetry::access_path::init(&provider.meter("ferroehr"));
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&crate::db::domain::DomainPools::from_shared(&db.pool()));
    let grants = Arc::clone(svc.grants());
    grants.refresh().await.expect("the copy reads");
    let first = Revisions {
        grants: grants.revision().expect("the copy holds a revision"),
        facts: FactsRead::Behind {
            after: Some(u64::MAX),
        },
        bindings: 0,
    };
    assert!(
        !grants.holds(&first),
        "no read the copy installs is past it"
    );
    super::catch_up(&grants, first)
        .await
        .expect("a later read finds the copy current");
    provider.force_flush().expect("flush");
    assert_eq!(counted(&exporter, "refused"), 0, "nothing is refused");
}
