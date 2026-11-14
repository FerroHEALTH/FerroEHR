// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The measurement behind the access-index record (#5588): how many settings
//! classes a population of EHRs with settings makes, what a replica holds of
//! them, how long a commit spends classifying its settings, and how long a
//! query spends building its access condition.
//!
//! `cargo bench -p ferroehr --bench access_classes -- 10000 100000 1000000`
//! prints one JSON object per population on stdout, the `after` rows of
//! `docs/benchmarks/access-index/record.json`. The population alternates an
//! opt-out from a date particular to each EHR with a mental-health
//! restriction from such a date, each kept by a gate-keeper particular to the
//! EHR. No openEHR spec governs this measurement: our own design.

#![expect(
    clippy::expect_used,
    reason = "bench fixture setup: a fixture that does not read must abort the bench loudly \
              (there is no caller to return an error to)"
)]
#![expect(
    clippy::print_stdout,
    reason = "the bench is a dev binary whose output is the record's rows"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the fixture settings are raw canonical JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

use std::collections::BTreeMap;
use std::time::Instant;

use ferroehr::access_eval::class::{Class, Classes, classify};
use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess};
use ferroehr::access_eval::procedure::{Caller, Emergency};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::ehr::access_settings::EhrAccessSettings;
use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use openehr_rm::draft::access_control::terminology::{AccessDefault, CallerKind, OperationClass};
use serde_json::{Value, json};

/// The professional the condition is built for.
const NURSE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f83";

/// A term of the access control groups.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code, "defining_code": { "_type": "CODE_PHRASE",
        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY }, "code_string": code } })
}

/// The settings of the `i`th EHR of the population.
fn settings(i: u64) -> String {
    let lower = format!(
        "2026-{:02}-{:02}T00:00:00Z",
        1 + i.div_euclid(28) % 12,
        1 + i % 28
    );
    let kind = if i.is_multiple_of(2) {
        "opt_out"
    } else {
        "restriction"
    };
    let mut directive = json!({ "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "Directive" }, "archetype_node_id": "at0001",
        "kind": term(kind), "effect": term("deny"), "emergency_override": true,
        "time_validity": { "_type": "DV_INTERVAL", "lower_unbounded": false,
            "upper_unbounded": true, "lower_included": true, "upper_included": false,
            "lower": { "_type": "DV_DATE_TIME", "value": lower } } });
    if let (false, Some(directive)) = (i.is_multiple_of(2), directive.as_object_mut()) {
        directive.insert(
            "data".to_owned(),
            json!([{ "_type": "ACCESS_DATA_SCOPE", "categories": [{
            "_type": "DV_CODED_TEXT", "value": "mental-health", "defining_code": {
                "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID",
                    "value": "urn:example:categories" }, "code_string": "mental-health" } }] }]),
        );
    }
    let keeper = uuid::Uuid::from_u64_pair(0x0190_b3a2_5a3e_7c1d, i).to_string();
    json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term("open"),
        "directives": [directive],
        "gate_keepers": [{ "_type": "PARTY_IDENTIFIED", "external_ref": { "_type": "PARTY_REF",
            "namespace": "local", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": keeper } } }] })
    .to_string()
}

/// The evaluator of a query by a professional with no facts.
fn request() -> RequestAccess {
    let caller = Caller {
        kind: CallerKind::Professional,
        party: Some(NURSE.to_owned()),
        own_ehr: None,
        acts_for: Vec::new(),
        roles: Vec::new(),
        facts: ferroehr::party_binding::grants::caller::CallerFacts::default(),
        administrator: false,
        settings: SettingsCaller::new(false, false, Some(NURSE.to_owned())),
    };
    RequestAccess::new(
        caller,
        Asking {
            operation: OperationClass::Query,
            purpose: None,
            emergency: Emergency::default(),
        },
        InForce {
            policy: None,
            categories: ferroehr::access_eval::scope::CategoryIndex::default(),
            server_default: AccessDefault::Open,
            now: jiff::Timestamp::now(),
            grants: None,
        },
    )
}

fn main() {
    let check: Value = serde_json::from_str(&settings(1)).expect("the fixture is JSON");
    ferroehr::service::ehr::access_settings::check_written_settings(&check)
        .expect("the fixture holds every invariant");
    let populations: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|arg| arg.parse().ok())
        .collect();
    let access = request();
    for n in populations {
        let mut classes = BTreeMap::new();
        let mut fact_bytes = 0usize;
        let started = Instant::now();
        for i in 0..n {
            let read: AccessDirectiveSettings =
                serde_json::from_str(&settings(i)).expect("the fixture decodes");
            let classified = classify(&read).expect("the fixture classifies");
            fact_bytes += Value::Array(classified.facts).to_string().len();
            classes
                .entry(classified.digest)
                .or_insert_with(|| classified.class.to_string());
        }
        let classify_ns = started
            .elapsed()
            .as_nanos()
            .div_euclid(u128::from(n.max(1)));
        let class_bytes: usize = classes.values().map(String::len).sum();
        let copy = Classes {
            classes: classes
                .values()
                .zip(1_i64..)
                .map(|(text, id)| (id, Class::read(text).expect("the class reads")))
                .collect(),
        };
        let sample: Vec<EhrAccessSettings> = (0..n.min(10_000))
            .map(|i| EhrAccessSettings::from_projected(&settings(i)).expect("the fixture reads"))
            .collect();
        let started = Instant::now();
        let mut admitted = 0usize;
        for read in &sample {
            for cared in [true, false] {
                admitted += usize::from(access.judge(None, Some(read), cared).admits_any());
            }
        }
        let decision_ns = started
            .elapsed()
            .as_nanos()
            .div_euclid(2 * u128::try_from(sample.len().max(1)).unwrap_or(u128::MAX));
        let started = Instant::now();
        let condition = access.condition(&copy, Vec::new());
        let condition_ns = started.elapsed().as_nanos();
        let row = json!({
            "ehrs": n,
            "classes": copy.classes.len(),
            "class_json_bytes": class_bytes,
            "classify_ns_per_commit": classify_ns,
            "fact_bytes_per_ehr": u64::try_from(fact_bytes).unwrap_or(u64::MAX).div_euclid(n.max(1)),
            "decision_ns": decision_ns,
            "condition_ns": condition_ns,
            "restrictive": condition.groups().is_some() && admitted > 0,
        });
        println!("{row}");
    }
}
