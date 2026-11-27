// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![allow(
    clippy::panic,
    clippy::expect_used,
    reason = "a browser journey asserts by panicking, and the shared harness panics when a configured stack cannot be driven"
)]
#![allow(
    clippy::print_stdout,
    reason = "the skip-with-reason and progress lines ARE this suite's report"
)]
#![allow(
    unreachable_pub,
    dead_code,
    reason = "the shared `common` harness is compiled into every journey binary; each one drives a different subset of it"
)]
#![allow(
    clippy::disallowed_types,
    reason = "the journey seeds its records as raw JSON by the testing rule (.claude/rules/testing.md)"
)]
//! End-to-end journeys over Query › Cohort: the administrator runs AQL over
//! the EHRs a bound predicate selects, reads the rows of a cohort as large as
//! the CDR's small-cell threshold, and finds the rows of a smaller one
//! withheld; and runs AQL over the demo seed's patients named as subjects.
//!
//! The journeys' posture binds the `city` predicate to the `at0012` ELEMENT
//! of `openEHR-DEMOGRAPHIC-PERSON.person.v1` (`docker/viewer/e2e-env.yml`)
//! under the default threshold of 5. Each run seeds persons of a city no
//! earlier run named, each the subject of an EHR of its own.

use crate::common;

use common::{Harness, env, fill, login_basic_as, main_text, submit, wait_main_contains};

/// The AQL each run sends: the id of every EHR in the cohort.
const AQL: &str = "SELECT e/ehr_id/value AS ehr FROM EHR e";

/// The CDR's default small-cell threshold
/// (`[privacy.cohort] small_cell_threshold`).
const THRESHOLD: usize = 5;

/// A PERSON living in `city`.
fn person(city: &str) -> serde_json::Value {
    let archetype = "openEHR-DEMOGRAPHIC-PERSON.person.v1";
    serde_json::json!({
        "_type": "PERSON", "name": { "_type": "DV_TEXT", "value": "Cohort journey person" },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "identities": [{ "_type": "PARTY_IDENTITY",
            "name": { "_type": "DV_TEXT", "value": "legal name" }, "archetype_node_id": "at0001",
            "details": { "_type": "ITEM_TREE", "name": { "_type": "DV_TEXT", "value": "Details" },
                "archetype_node_id": "at0002", "items": [{ "_type": "ELEMENT",
                    "name": { "_type": "DV_TEXT", "value": "Name" }, "archetype_node_id": "at0003",
                    "value": { "_type": "DV_TEXT", "value": "Cohort journey person" } }] } }],
        "details": { "_type": "ITEM_TREE", "name": { "_type": "DV_TEXT", "value": "details" },
            "archetype_node_id": "at0010", "items": [{ "_type": "ELEMENT",
                "name": { "_type": "DV_TEXT", "value": "City" }, "archetype_node_id": "at0012",
                "value": { "_type": "DV_TEXT", "value": city } }] }
    })
}

/// An `EHR_STATUS` whose subject is the PERSON `party`.
fn status(party: &str) -> serde_json::Value {
    let archetype = "openEHR-EHR-EHR_STATUS.generic.v1";
    serde_json::json!({
        "_type": "EHR_STATUS", "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.2.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "subject": { "_type": "PARTY_SELF", "external_ref": { "_type": "PARTY_REF",
            "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": party } } },
        "is_queryable": true, "is_modifiable": true
    })
}

/// Seeds `size` persons of `city`, each the subject of a new EHR, and answers
/// the EHRs' ids.
async fn cohort_of(cdr: &str, city: &str, size: usize) -> Vec<String> {
    let mut ehrs = Vec::new();
    for _ in 0..size {
        let party = common::admin_create(cdr, "/demographic/person", &person(city)).await;
        ehrs.push(common::admin_create(cdr, "/ehr", &status(&party)).await);
    }
    ehrs
}

/// Runs the cohort of `city` from the Cohort tab.
async fn run(h: &Harness, city: &str) {
    fill(h, "#cohort-city", city).await;
    fill(h, "#aql", AQL).await;
    submit(h, "header.fui-page-header button[form='query-form']").await;
    wait_main_contains(h, "EHRs in the cohort").await;
}

#[tokio::test]
async fn admin_runs_a_cohort_and_a_small_cell_is_withheld() {
    let Some(h) = Harness::start("cohort").await else {
        return;
    };
    let Some(cdr) = common::cdr_url("cohort") else {
        h.finish().await;
        return;
    };
    let served = common::fresh("e2e-city");
    let ehrs = cohort_of(&cdr, &served, THRESHOLD).await;
    let small = common::fresh("e2e-village");
    cohort_of(&cdr, &small, 1).await;
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    common::follow_nav(&h, "/query").await;
    common::open_tab(&h, "cohort").await;
    wait_main_contains(&h, "Fill the predicates or name the subjects").await;
    h.shot(1, "cohort-form").await;
    run(&h, &served).await;
    let text = main_text(&h).await;
    for ehr in &ehrs {
        assert!(
            text.contains(ehr.as_str()),
            "the cohort's rows name {ehr}: {text}"
        );
    }
    assert!(
        !text.contains("The rows are withheld"),
        "a cohort at the threshold is served: {text}"
    );
    h.shot(2, "cohort-rows").await;

    run(&h, &small).await;
    let text = main_text(&h).await;
    assert!(
        text.contains("The rows are withheld"),
        "a cohort under the threshold is withheld: {text}"
    );
    h.shot(3, "cohort-withheld").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

/// The identifier domain the demo seed names its patients in, beside no
/// patient identity source (`scripts/sandbox/seed/manifest.json`).
const SEED_SUBJECTS: &str = "ferroehr-sandbox";

#[tokio::test]
async fn admin_runs_a_cohort_of_the_seeds_named_patients() {
    let Some(h) = Harness::start("cohort-subjects").await else {
        return;
    };
    let Some(ehr) = env("UI_E2E_SEEDED_EHR_ID") else {
        println!("SKIP cohort-subjects: UI_E2E_SEEDED_EHR_ID is unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    };
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    common::follow_nav(&h, "/query").await;
    common::open_tab(&h, "cohort").await;
    wait_main_contains(&h, "Subject identifiers").await;
    fill(&h, "#cohort-subject-namespace", SEED_SUBJECTS).await;
    let patients: Vec<String> = (1..=6).map(|n| format!("sandbox-patient-{n:02}")).collect();
    fill(&h, "#cohort-subject-ids", &patients.join("\n")).await;
    fill(&h, "#aql", AQL).await;
    h.shot(1, "cohort-subjects").await;
    submit(&h, "header.fui-page-header button[form='query-form']").await;
    wait_main_contains(&h, "EHRs in the cohort").await;
    let text = main_text(&h).await;
    assert!(
        text.contains(ehr.as_str()),
        "the first named patient's EHR is in the rows: {text}"
    );
    assert!(
        !text.contains("The rows are withheld"),
        "six named patients reach the threshold: {text}"
    );
    h.shot(2, "cohort-subjects-rows").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
