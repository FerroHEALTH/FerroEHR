// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL `LIKE` on a COMPOSITION's `archetype_details/template_id/value`, end
//! to end over committed compositions of full-SemVer template ids (#5082).
//!
//! QUERY `master03-syntax.adoc` §Operators/LIKE admits `LIKE` on any `String`
//! operand, with `*` the any-run and `?` the one-character wildcard; no
//! clause narrows it for the template id. The fixtures are the vendored
//! `minimal_event` OPT and its composition, with the template id rewritten
//! (and nothing else changed) to `discharge_report.v1.0.0`,
//! `discharge_report.v1.2.0`, `discharge_report.v2.0.0` and the decoy
//! `dischargeXreport.v1.0.0`, which only a `_` read as the SQL one-character
//! wildcard would match. The plan shape (the `version.template_id` column and
//! its index, no node walk) is pinned by
//! `aql_planner::a_composition_template_like_reads_the_version_column` and
//! `storage_plans::a_template_like_reads_the_template_index`.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;

use crate::fixtures::uv;

/// The template id the vendored `minimal_event` fixtures carry.
const FIXTURE_TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

/// The projection every case runs, filtered by the `WHERE` it is given.
const SELECT: &str = "SELECT c/uid/value, c/archetype_details/template_id/value \
                      FROM EHR e CONTAINS COMPOSITION c WHERE ";

/// Reads a vendored corpus file with the template id rewritten to `template_id`.
fn fixture_as(relative: &str, template_id: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus")
        .join(relative);
    std::fs::read_to_string(path)
        .expect("the vendored fixture is readable")
        .replace(FIXTURE_TEMPLATE_ID, template_id)
}

/// Uploads the `minimal_event` OPT as `template_id`.
async fn upload(svc: &FerroEhrService, template_id: &str) {
    svc.template_adl14_upload(&fixture_as("templates/minimal_event.opt", template_id))
        .await
        .unwrap_or_else(|e| panic!("upload {template_id}: {e:?}"));
}

/// Commits the `minimal_event` composition of `template_id` into `ehr`,
/// returning its `OBJECT_VERSION_ID`.
async fn commit(svc: &FerroEhrService, ehr: &str, template_id: &str) -> String {
    let body: Value = serde_json::from_str(&fixture_as(
        "fixtures/composition/minimal_event.v1.json",
        template_id,
    ))
    .expect("the composition fixture is JSON");
    svc.create_composition(ehr.parse().expect("ehr_id uuid"), uv(&body, "249", None))
        .await
        .unwrap_or_else(|e| panic!("commit {template_id}: {e:?}"))
        .version_uid()
}

/// Runs `aql` with `parameters` over the whole population, returning each
/// row as its (uid, template id) pair.
async fn select(
    svc: &FerroEhrService,
    aql: &str,
    parameters: &[(&str, &str)],
) -> BTreeSet<(String, String)> {
    let request = AqlQueryRequest {
        parameters: parameters
            .iter()
            .map(|(name, value)| ((*name).to_owned(), json!(value)))
            .collect(),
        ..AqlQueryRequest::default()
    };
    let outcome = svc
        .execute_aql(aql, None, &request)
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"));
    outcome.result_set["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(|row| {
            (
                row[0].as_str().expect("uid").to_owned(),
                row[1].as_str().expect("template id").to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn like_on_the_template_id_selects_by_semver_pattern() {
    let (_db, svc) = crate::fixtures::service().await;
    for template_id in [
        "discharge_report.v1.0.0",
        "discharge_report.v1.2.0",
        "discharge_report.v2.0.0",
        "dischargeXreport.v1.0.0",
    ] {
        upload(&svc, template_id).await;
    }
    let first = crate::fixtures::create_ehr(&svc).await.to_string();
    let second = crate::fixtures::create_ehr(&svc).await.to_string();
    let row = |uid: &str, template_id: &str| (uid.to_owned(), template_id.to_owned());
    let first_v100 = row(
        &commit(&svc, &first, "discharge_report.v1.0.0").await,
        "discharge_report.v1.0.0",
    );
    let first_v120 = row(
        &commit(&svc, &first, "discharge_report.v1.2.0").await,
        "discharge_report.v1.2.0",
    );
    let first_v200 = row(
        &commit(&svc, &first, "discharge_report.v2.0.0").await,
        "discharge_report.v2.0.0",
    );
    commit(&svc, &first, "dischargeXreport.v1.0.0").await;
    let second_v100 = row(
        &commit(&svc, &second, "discharge_report.v1.0.0").await,
        "discharge_report.v1.0.0",
    );
    let template = "c/archetype_details/template_id/value";
    let v1 = BTreeSet::from([first_v100.clone(), first_v120.clone(), second_v100.clone()]);

    for (case, where_clause, parameters, expected) in [
        (
            "the `*` wildcard keeps every v1 and no decoy",
            format!("{template} LIKE 'discharge_report.v1.*'"),
            vec![],
            v1.clone(),
        ),
        (
            "an EHR-scoped LIKE keeps that EHR's v1 only",
            format!("e/ehr_id/value = $ehr AND {template} LIKE 'discharge_report.v1.*'"),
            vec![("ehr", first.as_str())],
            BTreeSet::from([first_v100.clone(), first_v120.clone()]),
        ),
        (
            "a parameter-bound pattern reads as the literal",
            format!("{template} LIKE $pattern"),
            vec![("pattern", "discharge_report.v1.*")],
            v1.clone(),
        ),
        (
            "the `?` wildcard takes one character",
            format!("{template} LIKE 'discharge_report.v?.0.0'"),
            vec![],
            BTreeSet::from([first_v100.clone(), first_v200.clone(), second_v100.clone()]),
        ),
        (
            "a `?` in the minor position keeps every v1",
            format!("{template} LIKE 'discharge_report.v1.?.0'"),
            vec![],
            v1.clone(),
        ),
        (
            "a pattern no template id matches selects nothing",
            format!("{template} LIKE 'discharge_report.v3.*'"),
            vec![],
            BTreeSet::new(),
        ),
        (
            "the `=` control",
            format!("{template} = 'discharge_report.v1.2.0'"),
            vec![],
            BTreeSet::from([first_v120.clone()]),
        ),
        (
            "the MATCHES control",
            format!("{template} MATCHES {{'discharge_report.v1.0.0', 'discharge_report.v1.2.0'}}"),
            vec![],
            v1.clone(),
        ),
    ] {
        let got = select(&svc, &format!("{SELECT}{where_clause}"), &parameters).await;
        assert_eq!(got, expected, "{case}: WHERE {where_clause}");
    }
}
