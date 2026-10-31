// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The COMPOSITION commit against an ADL 2 operational template, over a real
//! PostgreSQL 18 (shared testkit harness), driven by the Veredictum catalogue's
//! ADL 2 content cases (`corpus/veredictum/adl2-content/`).
//!
//! Every case's template is uploaded through the ADL 2 definition service and
//! its composition committed on the canonical path: a composition the template
//! admits is created, one that breaks a template constraint is refused with
//! the per-path violations ITS-REST `responses/422.yaml` gives a `422` ("the
//! underlying template ... is not validating the supplied resource") and is
//! not persisted. The FLAT and STRUCTURED bodies reach the same commit after
//! conversion, so one constraint is driven through each of them too.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only reaches \
              `#[test]`-annotated functions, so it misses this module's helpers and async \
              bodies; panicking assertions are the intended shape here (the Rust Book ch11)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::error::ServiceError;
use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
use openehr_rm::prelude::PartyProxy;

/// One commit step of a catalogue workflow.
struct Case {
    workflow: String,
    template: String,
    status: u16,
    body: Value,
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/veredictum/adl2-content")
}

fn yaml(path: &Path) -> Value {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_norway::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn text<'a>(v: &'a Value, pointer: &str) -> &'a str {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no string at {pointer}"))
}

/// The ADL 2 source of every setup template, keyed by its upload workflow.
fn templates() -> BTreeMap<String, String> {
    let setup = yaml(&corpus().join("_setup/content_templates_adl2.arazzo.yaml"));
    setup
        .pointer("/workflows")
        .and_then(Value::as_array)
        .expect("setup workflows")
        .iter()
        .map(|wf| {
            (
                text(wf, "/workflowId").to_owned(),
                text(wf, "/steps/0/requestBody/payload").to_owned(),
            )
        })
        .collect()
}

/// Every commit step of every `content/adl2_*` document.
fn cases() -> Vec<Case> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(corpus().join("content"))
        .expect("content dir")
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in &paths {
        let doc = yaml(path);
        for wf in doc
            .pointer("/workflows")
            .and_then(Value::as_array)
            .expect("workflows")
        {
            let Some(commit) = wf
                .pointer("/steps")
                .and_then(Value::as_array)
                .and_then(|s| s.iter().find(|s| s.get("stepId") == Some(&json!("commit"))))
            else {
                continue;
            };
            let condition = text(commit, "/successCriteria/0/condition");
            out.push(Case {
                workflow: text(wf, "/workflowId").to_owned(),
                template: text(wf, "/dependsOn/0")
                    .rsplit('.')
                    .next()
                    .expect("dependsOn")
                    .to_owned(),
                status: condition
                    .strip_prefix("$statusCode == ")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| panic!("status condition {condition}")),
                body: commit
                    .pointer("/requestBody/payload")
                    .cloned()
                    .expect("commit body"),
            });
        }
    }
    out
}

/// The SM `UPDATE_VERSION` envelope for a creation commit of `data`.
fn creation(data: &Value) -> UpdateVersion<openehr_rm::prelude::Composition> {
    UpdateVersion {
        preceding_version_uid: None,
        lifecycle_state: lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the catalogue body decodes as a COMPOSITION"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded("249"),
            description: None,
            committer: openehr_its::json::from_canonical_value::<PartyProxy>(
                &json!({ "_type": "PARTY_IDENTIFIED", "name": "conformance tester" }),
            )
            .expect("committer"),
            additional_properties: BTreeMap::new(),
        }),
        signature: None,
        additional_properties: BTreeMap::new(),
    }
}

async fn new_ehr(svc: &FerroEhrService) -> EhrId {
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr").to_string();
    EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"))
}

/// Upload each template `cases` names, once.
async fn upload(svc: &FerroEhrService, cases: &[&Case]) {
    let sources = templates();
    let names: BTreeSet<&str> = cases.iter().map(|c| c.template.as_str()).collect();
    for name in names {
        let source = sources
            .get(name)
            .unwrap_or_else(|| panic!("no setup template {name}"));
        svc.upload_artefact(source.clone())
            .await
            .unwrap_or_else(|e| panic!("upload {name}: {e:?}"));
    }
}

/// Every catalogue case the commit itself answers (a `201` or a `422`; the
/// `400` rows are refused at the REST edge before the commit) gets that answer
/// on the canonical path.
#[tokio::test]
async fn every_adl2_content_case_gets_its_answer_on_the_canonical_commit() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let all = cases();
    let committed: Vec<&Case> = all.iter().filter(|c| c.status != 400).collect();
    assert_eq!(
        committed.iter().filter(|c| c.status == 422).count(),
        30,
        "the catalogue's 30 refusal rows"
    );
    upload(&svc, &committed).await;
    let ehr = new_ehr(&svc).await;
    for case in committed {
        let outcome = svc.create_composition(ehr, creation(&case.body)).await;
        match (case.status, outcome) {
            (201, Ok(_)) => {}
            (422, Err(ServiceError::ValidationFailed(violations))) => assert!(
                !violations.is_empty(),
                "{}: refused without a violation",
                case.workflow
            ),
            (status, outcome) => panic!(
                "{}: the catalogue expects {status}, the commit answered {outcome:?}",
                case.workflow
            ),
        }
    }
}

/// The `DV_COUNT` magnitude `|10..20|` case pair, its template uploaded, and
/// the Web Template the REST edge converts FLAT and STRUCTURED bodies with.
async fn count_range(
    svc: &FerroEhrService,
) -> (
    Value,
    std::sync::Arc<openehr_sdt::flat::webtemplate::model::WebTemplate>,
) {
    let all = cases();
    let accepted = all
        .iter()
        .find(|c| c.workflow == "adl2-dv_count-validate_range-accepted")
        .expect("the DV_COUNT range control case");
    upload(svc, &[accepted]).await;
    let wt = svc
        .web_template(text(&accepted.body, "/archetype_details/template_id/value"))
        .await
        .expect("the ADL 2 Web Template resolves");
    (accepted.body.clone(), wt)
}

/// The FLAT form of the accepted `DV_COUNT` composition with its magnitude set
/// to `magnitude`, flattened from the interface id the Web Template names its
/// OBSERVATION by.
fn count_flat(
    body: &Value,
    wt: &openehr_sdt::flat::webtemplate::model::WebTemplate,
    magnitude: i64,
) -> Map<String, Value> {
    let mut body = body.clone();
    *body
        .pointer_mut("/content/0/archetype_node_id")
        .expect("the inlined OBSERVATION") = json!("openEHR-EHR-OBSERVATION.cnf_minimal.v1");
    let mut flat = openehr_sdt::flat::convert::composition_to_flat(&body, wt)
        .expect("the accepted composition flattens");
    let key = flat
        .keys()
        .find(|k| k.contains("/any_event:0/value"))
        .cloned()
        .expect("the DV_COUNT FLAT key");
    flat.insert(key, Value::from(magnitude));
    flat
}

/// A FLAT body breaking the `C_INTEGER` interval is refused at the commit, and
/// the one inside it is created (AOM2 `master04.5` §"Conformance semantics:
/// `C_ORDERED`").
#[tokio::test]
async fn a_flat_body_breaking_an_adl2_constraint_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (body, wt) = count_range(&svc).await;
    let ehr = new_ehr(&svc).await;
    for (magnitude, admitted) in [(15, true), (25, false)] {
        let canonical = openehr_sdt::flat::convert::submitted_composition_from_flat(
            &count_flat(&body, &wt, magnitude),
            &wt,
            "2026-01-01T10:00:00Z",
        )
        .expect("the FLAT body converts");
        let outcome = svc.create_composition(ehr, creation(&canonical)).await;
        assert_eq!(
            outcome.is_ok(),
            admitted,
            "FLAT magnitude {magnitude}: {outcome:?}"
        );
    }
}

/// The STRUCTURED twin of the FLAT case.
#[tokio::test]
async fn a_structured_body_breaking_an_adl2_constraint_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (body, wt) = count_range(&svc).await;
    let ehr = new_ehr(&svc).await;
    for (magnitude, admitted) in [(15, true), (25, false)] {
        let structured =
            openehr_sdt::flat::convert::flat_to_structured(&count_flat(&body, &wt, magnitude))
                .expect("FLAT to STRUCTURED");
        let canonical = openehr_sdt::flat::convert::submitted_composition_from_structured(
            &structured,
            &wt,
            "2026-01-01T10:00:00Z",
        )
        .expect("the STRUCTURED body converts");
        let outcome = svc.create_composition(ehr, creation(&canonical)).await;
        assert_eq!(
            outcome.is_ok(),
            admitted,
            "STRUCTURED magnitude {magnitude}: {outcome:?}"
        );
    }
}
