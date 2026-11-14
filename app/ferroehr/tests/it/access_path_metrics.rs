// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access evaluator's decisions as the operator metrics count them: one
//! per EHR a request reached, by outcome and `refused-*` code, and each step
//! an emergency lifted, with no EHR id in any label. No openEHR spec governs
//! metrics: our own design (#5591).

use std::sync::{Arc, OnceLock};

use ferroehr::access_eval::condition::{
    Asking, EhrEntry, InForce, RequestAccess, with_request_access,
};
use ferroehr::access_eval::procedure::{Caller, Emergency, Outcome};
use ferroehr::access_eval::scope::{CategoryIndex, Item};
use ferroehr::ids::EhrId;
use ferroehr::party_binding::grants::caller::CallerFacts;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::telemetry::build_info::BuildInfo;
use ferroehr::telemetry::{access_path, metrics};
use ferrohealth_telemetry::section::TelemetrySection;
use ferrohealth_telemetry::telemetry::Telemetry;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, OperationClass, Term as _,
};

/// The process's provider with the shipped instruments installed on it.
fn recorder() -> &'static Telemetry {
    static TELEMETRY: OnceLock<Telemetry> = OnceLock::new();
    TELEMETRY.get_or_init(|| {
        let telemetry = Telemetry::new(
            &TelemetrySection::default(),
            ferroehr::telemetry::service(&BuildInfo::current()),
        )
        .expect("build the telemetry provider");
        let meter = telemetry.meter(metrics::SCOPE);
        metrics::init(&meter);
        access_path::init(&meter);
        telemetry
    })
}

/// A professional asking to read, under a server default that requires consent, declaring
/// `emergency`.
fn request(emergency: Emergency) -> Arc<RequestAccess> {
    let party = "7f8e9d0c-1b2a-4c3d-8e4f-5a6b7c8d9e0f".to_owned();
    Arc::new(RequestAccess::new(
        Caller {
            kind: CallerKind::Professional,
            party: Some(party.clone()),
            own_ehr: None,
            acts_for: Vec::new(),
            roles: Vec::new(),
            facts: CallerFacts::default(),
            administrator: false,
            settings: SettingsCaller::new(false, false, Some(party)),
        },
        Asking {
            operation: OperationClass::Read,
            purpose: None,
            emergency,
        },
        InForce {
            policy: None,
            categories: CategoryIndex::default(),
            server_default: AccessDefault::ConsentRequired,
            now: jiff::Timestamp::now(),
            grants: None,
        },
    ))
}

/// Judge `ehr_id`, an EHR with no settings, and ask for the EHR itself.
async fn ask(access: Arc<RequestAccess>, ehr_id: EhrId) -> Outcome {
    let entry = EhrEntry {
        settings: Arc::new(None),
        subject: None,
        version: None,
    };
    let judged = Arc::clone(&access);
    with_request_access(access, async move {
        let verdict = judged.judge_ehr(ehr_id, &entry);
        judged.note(ehr_id, &verdict, &Item::Ehr)
    })
    .await
}

/// The rendered value of the sample line starting `series`, or `0`.
fn sample(rendered: &str, series: &str) -> u64 {
    rendered
        .lines()
        .filter(|line| line.starts_with(series))
        .find_map(|line| line.rsplit(' ').next().and_then(|v| v.parse::<u64>().ok()))
        .unwrap_or(0)
}

#[tokio::test]
async fn decisions_are_counted_by_outcome_and_code_and_name_no_ehr() {
    let telemetry = recorder();
    let refused_ehr = EhrId(uuid::Uuid::now_v7());
    let lifted_ehr = EhrId(uuid::Uuid::now_v7());

    let refused = ask(request(Emergency::default()), refused_ehr).await;
    let (Outcome::Absent(code) | Outcome::Refused(code)) = refused else {
        panic!("a default that requires consent refuses a caller with no permit: {refused:?}");
    };
    let glass = Emergency {
        declared: true,
        break_glass: true,
        opt_out_override: false,
    };
    assert_eq!(ask(request(glass), lifted_ehr).await, Outcome::Admit);

    let rendered = telemetry
        .render_prometheus()
        .expect("render the exposition");
    let refusal = format!(
        "access_decisions_total{{code=\"{}\",outcome=\"refused\"",
        code.code()
    );
    assert_eq!(sample(&rendered, &refusal), 1, "{rendered}");
    assert_eq!(
        sample(
            &rendered,
            "access_decisions_total{code=\"none\",outcome=\"admitted\""
        ),
        1,
        "{rendered}"
    );
    assert_eq!(
        sample(
            &rendered,
            "access_emergency_overrides_total{lifted=\"policy\""
        ),
        1,
        "{rendered}"
    );
    for ehr_id in [refused_ehr, lifted_ehr] {
        assert!(
            !rendered.contains(&ehr_id.0.to_string()),
            "a label names the EHR {ehr_id:?}: {rendered}"
        );
    }
}
