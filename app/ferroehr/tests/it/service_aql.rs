// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end AQL engine tests against a real PostgreSQL 18 (shared testkit harness):
//! seed EHRs + COMPOSITIONs through the service, then execute AQL through the
//! `QueryService` seam and assert on the assembled ITS-REST 1.1.0 `RESULT_SET`.
//!
//! Fixtures: the vendored openEHR SDK `minimal_observation` composition
//! (`openehr-its/tests/vendor/openehr_sdk/composition/canonical_json`), with its
//! `template_id` stripped (so only the template-independent RM-invariant +
//! terminology validation runs) and the leaf ELEMENT value swapped to a
//! `DV_QUANTITY` with a controlled magnitude — giving deterministic values to
//! order/compare/aggregate over.
//!
//! Acceptance set (design §Testing; QUERY 1.1 + the CNF QUERY chapter
//! `docs/specs/openehr/CNF/docs/platform_test_schedule/master11-func_tc_querying.adoc`):
//! CONTAINS chains, WHERE magnitude comparison + ORDER BY magnitude, DISTINCT,
//! aggregates, LIMIT/OFFSET + REST fetch/offset, `$parameters`, `ehr_id` scoping,
//! NOT CONTAINS, VERSION uid/time selection, whole-COMPOSITION reassembly
//! equality, and `LATEST_VERSION` vs `ALL_VERSIONS` over a twice-updated object.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]
#![expect(
    clippy::float_cmp,
    reason = "the asserted magnitudes are integral DV_QUANTITY values, exactly \
              representable in f64 and returned verbatim by the query, so exact \
              equality is the assertion the test intends"
)]
#![expect(
    clippy::too_many_lines,
    reason = "an end-to-end suite drives one long lifecycle per test on purpose: \
              splitting a case would hide the order its assertions depend on"
)]

// `float_cmp`: the magnitudes are exact whole numbers seeded by the test and
// round-tripped losslessly, so exact comparison is intended.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use openehr_rm::prelude::PartyProxy;
use uuid::Uuid;

use ferroehr::service::FerroEhrService;
use ferroehr::service::query::config::QueryConfig;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::status::{CallStatusType, SmError};

use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};

use crate::adl2_fixture::adl2_source;

const OBS_ARCHETYPE: &str = "openEHR-EHR-OBSERVATION.minimal.v1";
/// The magnitude leaf path used throughout (bp.v1-style descent to the ELEMENT).
const MAG_PATH: &str = "data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/magnitude";

/// The SM `UPDATE_VERSION` commit envelope for a bare-RM write, mirroring the
/// adapter's `mk_update_version` (the RM object is the `data`, `If-Match` is the
/// `preceding_version_uid`, and the audit carries the change type + committer).
fn uv<T: serde::de::DeserializeOwned>(
    data: &Value,
    change_code: &str,
    preceding: Option<&str>,
) -> UpdateVersion<T> {
    UpdateVersion {
        preceding_version_uid: preceding.map(|p| p.parse().expect("OBJECT_VERSION_ID")),
        lifecycle_state: lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the fixture commit body decodes as its RM type"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded(change_code),
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

/// The base composition with its template stripped and a `DV_QUANTITY` leaf.
fn base_composition() -> Value {
    let path = format!(
        "{}/../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/minimal_observation.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
        .expect("parse fixture")
}

/// A composition variant with the given display name and leaf magnitude.
fn composition(name: &str, magnitude: f64) -> Value {
    let mut c = base_composition();
    // Strip the template id so only RM-invariant + terminology validation runs
    // (no archetype-conformance pass), and drop any incoming uid.
    if let Some(details) = c
        .get_mut("archetype_details")
        .and_then(Value::as_object_mut)
    {
        details.remove("template_id");
    }
    if let Some(obj) = c.as_object_mut() {
        obj.remove("uid");
    }
    c["name"] = json!({ "_type": "DV_TEXT", "value": name });
    // The content OBSERVATION is an archetype root, so it must carry
    // archetype_details (RM invariant `Is_archetype_root`); the vendored fixture
    // omits it.
    c["content"][0]["archetype_details"] = json!({
        "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": OBS_ARCHETYPE },
        "rm_version": "1.1.0",
    });
    c["content"][0]["data"]["events"][0]["data"]["items"][0]["value"] =
        json!({ "_type": "DV_QUANTITY", "magnitude": magnitude, "units": "mm[Hg]" });
    c
}

async fn create_ehr(svc: &FerroEhrService) -> String {
    // NOTE: the SM `create_ehr` returns the new EHR's `UUID`; `ehr_id.value`
    // IS that uuid, so its string form is the id every assertion here uses.
    svc.create_ehr(None).await.expect("create_ehr").to_string()
}

/// Create a composition in `ehr_id`, returning its `OBJECT_VERSION_ID`.
async fn create_comp(svc: &FerroEhrService, ehr_id: &str, name: &str, magnitude: f64) -> String {
    // NOTE: the SM `create_composition` returns the new `version_uid`
    // directly — the same value the composition's `uid` carries.
    svc.create_composition(
        ehr_id.parse().expect("ehr_id uuid"),
        uv(&composition(name, magnitude), "249", None),
    )
    .await
    .unwrap_or_else(|e| panic!("create_composition ({name}, {magnitude}): {e:?}"))
    .version_uid()
}

/// Commit a COMPOSITION whose content OBSERVATION carries `archetype` as its
/// `archetype_node_id` (and matching `archetype_details.archetype_id`), returning
/// the new `OBJECT_VERSION_ID`. Used to seed parent/specialised archetypes for
/// the subsumption test.
async fn create_comp_arch(
    svc: &FerroEhrService,
    ehr_id: &str,
    name: &str,
    magnitude: f64,
    archetype: &str,
) -> String {
    let mut c = composition(name, magnitude);
    c["content"][0]["archetype_node_id"] = json!(archetype);
    c["content"][0]["archetype_details"]["archetype_id"]["value"] = json!(archetype);
    svc.create_composition(ehr_id.parse().expect("ehr_id uuid"), uv(&c, "249", None))
        .await
        .unwrap_or_else(|e| panic!("create_composition ({name}): {e:?}"))
        .version_uid()
}

/// Count the OBSERVATIONs in `ehr_id` matched by an archetype predicate.
async fn count_obs(svc: &FerroEhrService, ehr_id: &str, archetype: &str) -> i64 {
    let aql = format!("SELECT COUNT(*) FROM EHR e CONTAINS OBSERVATION o[{archetype}]");
    let r = run_aql(svc, &aql, ehr_scope(ehr_id)).await;
    rows(&r)[0][0].as_i64().expect("count is an integer")
}

async fn run_aql(svc: &FerroEhrService, aql: &str, request: AqlQueryRequest) -> Value {
    svc.execute_ad_hoc_query(aql.to_owned(), request)
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"))
        .result_set
}

/// Flip an EHR's `EHR_STATUS.is_queryable` to `false` through the service's
/// canonical status-update path (`ehr_status_update`), supplying the current
/// version `uid` as the `If-Match` precondition.
async fn set_not_queryable(svc: &FerroEhrService, ehr_id: &str) {
    // Read the current EHR_STATUS — its body carries the `uid` we need for the
    // optimistic-concurrency precondition, plus the mandatory RM fields we keep.
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse().expect("ehr_id uuid"));
    let mut body = svc
        .get_ehr_status_at_time(ehr_uuid, None)
        .await
        .expect("get_ehr_status_at_time");
    let if_match = body["uid"]["value"]
        .as_str()
        .expect("EHR_STATUS uid")
        .to_owned();
    let obj = body.as_object_mut().expect("EHR_STATUS object");
    obj.remove("uid");
    obj.insert("is_queryable".to_owned(), json!(false));
    svc.replace_ehr_status(ehr_uuid, uv(&body, "251", Some(&if_match)))
        .await
        .expect("replace_ehr_status is_queryable=false");
}

fn rows(result: &Value) -> &Vec<Value> {
    result["rows"].as_array().expect("rows array")
}

fn ehr_scope(ehr_id: &str) -> AqlQueryRequest {
    // `i_query_service.adoc`: `ehr_ids: List<UUID>`; the single-EHR REST scope is
    // the one-element case.
    AqlQueryRequest {
        ehr_ids: vec![ehr_id.to_owned()],
        ..AqlQueryRequest::default()
    }
}

/// Scope a query to a set of EHRs (`ehr_ids: List<UUID>`).
fn ehr_scope_multi(ehr_ids: &[&str]) -> AqlQueryRequest {
    AqlQueryRequest {
        ehr_ids: ehr_ids.iter().map(|s| (*s).to_owned()).collect(),
        ..AqlQueryRequest::default()
    }
}

#[tokio::test]
async fn aql_acceptance_set() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;
    // Magnitudes 80, 100, 120; two share the name "BP", one is "HR".
    create_comp(&svc, &ehr_id, "BP", 80.0).await;
    create_comp(&svc, &ehr_id, "BP", 120.0).await;
    create_comp(&svc, &ehr_id, "HR", 100.0).await;
    // A second EHR with one composition, to prove ehr_id scoping.
    let other = create_ehr(&svc).await;
    create_comp(&svc, &other, "BP", 999.0).await;

    // ── COUNT + ehr_id scope ────────────────────────────────────────────────
    let r = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&r)[0][0], json!(3), "3 compositions in the scoped EHR");
    assert_eq!(
        r["columns"][0]["name"],
        json!("#0"),
        "unaliased column gets a generated name"
    );

    // Without ehr_id: the population count spans both EHRs.
    let r = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c",
        AqlQueryRequest::default(),
    )
    .await;
    assert_eq!(rows(&r)[0][0], json!(4), "4 compositions across all EHRs");

    // ── CONTAINS chain (EHR→COMPOSITION→OBSERVATION) + ORDER BY magnitude ────
    let aql = format!(
        "SELECT o/{MAG_PATH} AS m \
         FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         ORDER BY o/{MAG_PATH}"
    );
    let r = run_aql(&svc, &aql, ehr_scope(&ehr_id)).await;
    let mags: Vec<f64> = rows(&r)
        .iter()
        .map(|row| row[0].as_f64().expect("magnitude"))
        .collect();
    assert_eq!(
        mags,
        vec![80.0, 100.0, 120.0],
        "ordered ascending by magnitude"
    );
    assert_eq!(
        r["columns"][0]["name"],
        json!("m"),
        "AS alias becomes the column name"
    );

    // ── WHERE magnitude comparison ($parameter) ───────────────────────────────
    let aql = format!(
        "SELECT o/{MAG_PATH} AS m \
         FROM EHR e CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         WHERE o/{MAG_PATH} > $min ORDER BY o/{MAG_PATH}"
    );
    let mut req = ehr_scope(&ehr_id);
    req.parameters = BTreeMap::from([("min".to_owned(), json!(90))]);
    let r = run_aql(&svc, &aql, req).await;
    let mags: Vec<f64> = rows(&r)
        .iter()
        .map(|row| row[0].as_f64().unwrap())
        .collect();
    assert_eq!(mags, vec![100.0, 120.0], "$min = 90 keeps 100 and 120");

    // ── aggregates (MAX/MIN/AVG over magnitude) ───────────────────────────────
    let aql = format!(
        "SELECT MAX(o/{MAG_PATH}), MIN(o/{MAG_PATH}) \
         FROM EHR e CONTAINS OBSERVATION o[{OBS_ARCHETYPE}]"
    );
    let r = run_aql(&svc, &aql, ehr_scope(&ehr_id)).await;
    assert_eq!(rows(&r)[0][0].as_f64().unwrap(), 120.0, "MAX magnitude");
    assert_eq!(rows(&r)[0][1].as_f64().unwrap(), 80.0, "MIN magnitude");

    // ── DISTINCT over the composition name ─────────────────────────────────────
    let r = run_aql(
        &svc,
        "SELECT DISTINCT c/name/value FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    let mut names: Vec<String> = rows(&r)
        .iter()
        .map(|row| row[0].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["BP", "HR"],
        "two distinct names (BP appears twice)"
    );

    // ── LIMIT/OFFSET (AQL) ─────────────────────────────────────────────────────
    let aql = format!(
        "SELECT o/{MAG_PATH} AS m FROM EHR e CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         ORDER BY o/{MAG_PATH} LIMIT 1 OFFSET 1"
    );
    let r = run_aql(&svc, &aql, ehr_scope(&ehr_id)).await;
    assert_eq!(rows(&r).len(), 1, "LIMIT 1");
    assert_eq!(
        rows(&r)[0][0].as_f64().unwrap(),
        100.0,
        "OFFSET 1 → the middle value"
    );

    // ── REST fetch/offset paging (no AQL LIMIT) ────────────────────────────────
    let aql = format!(
        "SELECT o/{MAG_PATH} AS m FROM EHR e CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         ORDER BY o/{MAG_PATH}"
    );
    let mut req = ehr_scope(&ehr_id);
    req.fetch = Some(2);
    req.offset = Some(1);
    let r = run_aql(&svc, &aql, req).await;
    let mags: Vec<f64> = rows(&r)
        .iter()
        .map(|row| row[0].as_f64().unwrap())
        .collect();
    assert_eq!(mags, vec![100.0, 120.0], "fetch=2 offset=1 → rows 2 and 3");

    // fetch composes OVER an AQL LIMIT window (ITS-REST query Request.md
    // §Common Headers and Query Parameters: `fetch` "cannot be combined with
    // AQL-top" — TOP is the ONLY prohibited pairing; `offset`/`fetch` page
    // the result set the AQL clauses define).
    let aql = format!(
        "SELECT o/{MAG_PATH} AS m FROM EHR e CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         ORDER BY o/{MAG_PATH} LIMIT 5"
    );
    let mut req = ehr_scope(&ehr_id);
    req.fetch = Some(2);
    let r = run_aql(&svc, &aql, req).await;
    assert_eq!(
        rows(&r).len(),
        2,
        "fetch=2 pages inside the AQL LIMIT 5 window"
    );

    // fetch + the deprecated TOP modifier stays rejected (Request.md names
    // exactly this pairing), as the SM `precondition_violation` → wire `400`.
    let aql =
        format!("SELECT TOP 2 o/{MAG_PATH} FROM EHR e CONTAINS OBSERVATION o[{OBS_ARCHETYPE}]");
    let mut req = ehr_scope(&ehr_id);
    req.fetch = Some(2);
    let err = svc
        .execute_ad_hoc_query(aql, req)
        .await
        .expect_err("fetch + AQL TOP must be rejected");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ),
        "fetch+TOP is precondition_violation, got {err:?}"
    );

    // ── NOT CONTAINS (none contain a different archetype) ──────────────────────
    let r = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c \
         NOT CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.other.v1]",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&r)[0][0], json!(3), "all 3 lack the other archetype");

    // ── VERSION metadata selection (uid + time_committed) ──────────────────────
    let r = run_aql(
        &svc,
        "SELECT v/uid/value, v/commit_audit/time_committed \
         FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&r).len(), 3, "one latest version per composition");
    for row in rows(&r) {
        assert!(
            row[0].as_str().is_some_and(|u| u.contains("::")),
            "uid is an OBJECT_VERSION_ID: {:?}",
            row[0]
        );
        assert!(
            row[1].as_str().is_some(),
            "time_committed present: {:?}",
            row[1]
        );
    }

    // ── whole-COMPOSITION select reassembles equal to composition_get ──────────
    let single = create_ehr(&svc).await;
    let ovid = create_comp(&svc, &single, "solo", 55.0).await;
    let vo_id = ovid.split("::").next().unwrap();
    let r = run_aql(
        &svc,
        "SELECT c FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&single),
    )
    .await;
    assert_eq!(rows(&r).len(), 1, "one composition in the single-comp EHR");
    let queried = &rows(&r)[0][0];
    let fetched = svc
        .get_composition_latest(
            single.parse().expect("ehr uuid"),
            vo_id.parse().expect("vo uuid"),
        )
        .await
        .expect("get_composition_latest");
    // The stored canonical carries the version uid since commit-time stamping
    // (#439), so the AQL whole-object cell and the read surface serve ONE
    // shape — compared verbatim.
    let expected = fetched.clone();
    assert_eq!(
        queried, &expected,
        "whole-object reassembly equals composition_get"
    );
}

/// The whole-object result-assembly N+1 fix.
///
/// A dashboard-sized multi-row whole-COMPOSITION projection must reassemble
/// **every** row's composition byte-identically to a direct
/// `get_composition_latest`. The executor now collects one subtree anchor per
/// whole-object cell across the whole page and loads them in a SINGLE statement
/// (`storage::node_repo::read_subtrees_canonical`) instead of one follow-up
/// SELECT per candidate row. No countable per-statement seam exists in the
/// harness (no `pg_stat_statements`), so equivalence over a realistic page — plus
/// the mixed scalar+whole-object columns and the duplicate-anchor projection
/// below — is the oracle; correctness is byte-identical, the shape change is the
/// batched loader documented on that function.
#[tokio::test]
async fn whole_object_projection_batches_over_a_multi_row_page() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = create_ehr(&svc).await;

    // A page of distinct compositions in one EHR — each its own versioned
    // object, so the page projects N distinct subtree anchors in one batch.
    let mut expected: BTreeMap<String, Value> = BTreeMap::new();
    for i in 0..8 {
        let name = format!("comp-{i}");
        let ovid = create_comp(&svc, &ehr_id, &name, f64::from(i) + 10.0).await;
        let vo_id = ovid.split("::").next().unwrap();
        let body = svc
            .get_composition_latest(
                ehr_id.parse().expect("ehr uuid"),
                vo_id.parse().expect("vo uuid"),
            )
            .await
            .expect("get_composition_latest");
        // One shape since commit-time uid stamping (#439): the AQL cell and
        // the read surface both carry the version uid — compared verbatim.
        expected.insert(name, body);
    }

    // Mixed scalar (`c/name/value`) + whole-object (`c`) columns across the page:
    // exercises the by-position fill of the batched whole-object cells.
    let r = run_aql(
        &svc,
        "SELECT c/name/value, c FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&r).len(), expected.len(), "all compositions returned");
    for row in rows(&r) {
        let name = row[0].as_str().expect("name cell");
        let whole = &row[1];
        assert_eq!(
            whole,
            expected
                .get(name)
                .unwrap_or_else(|| panic!("unexpected row {name}")),
            "batched whole-object reassembly equals get_composition_latest for {name}"
        );
    }

    // Two whole-object columns projecting the SAME object per row: the loader
    // de-duplicates the anchor and fills both cells from the one reassembly.
    let r = run_aql(
        &svc,
        "SELECT c, c FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&r).len(), expected.len(), "all compositions returned");
    for row in rows(&r) {
        assert_eq!(row[0], row[1], "duplicate whole-object columns are equal");
        let name = row[0]["name"]["value"].as_str().expect("name in body");
        assert_eq!(
            &row[0],
            expected
                .get(name)
                .unwrap_or_else(|| panic!("unexpected row {name}")),
            "de-duplicated whole-object reassembly equals get_composition_latest for {name}"
        );
    }
}

/// Archetype-specialisation subsumption: a query naming a **parent**
/// archetype matches data created with any **specialisation child**, bounded to
/// the same qualified RM entity and major version.
///
/// Spec: BASE `architecture_overview` master10 §Design-time Relationships — "the
/// data created with any specialised archetype will always be matched by queries
/// based on the parent archetype - in other words, a query for 'laboratory'
/// Observations will correctly retrieve 'glucose' Observations as well"; AM
/// master07 §Querying / §Supporting Archetype-based Querying (the matching set,
/// and the hard interface-reference major boundary — a differing major denotes a
/// different logical archetype).
#[tokio::test]
async fn archetype_specialisation_subsumption() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;
    // One composition with the PARENT archetype, one with a SPECIALISATION child
    // (the master10 worked example: laboratory → laboratory-glucose).
    create_comp_arch(
        &svc,
        &ehr_id,
        "parent",
        80.0,
        "openEHR-EHR-OBSERVATION.laboratory.v1",
    )
    .await;
    create_comp_arch(
        &svc,
        &ehr_id,
        "child",
        90.0,
        "openEHR-EHR-OBSERVATION.laboratory-glucose.v1",
    )
    .await;

    // (a) the parent archetype matches BOTH the parent and the specialised child.
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.laboratory.v1").await,
        2,
        "a query for the parent 'laboratory' retrieves both laboratory and \
         laboratory-glucose data (master10 §Design-time Relationships)"
    );

    // (b) the specialised predicate matches only the specialised one.
    assert_eq!(
        count_obs(
            &svc,
            &ehr_id,
            "openEHR-EHR-OBSERVATION.laboratory-glucose.v1"
        )
        .await,
        1,
        "the specialisation-child predicate matches only the child composition"
    );

    // (c) a sibling concept does NOT match — the `-` segment boundary is
    // significant, so neither `laboratory2` (a different concept) nor `labora`
    // (a bare prefix, not a `-`-delimited parent) subsumes the seeded data.
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.laboratory2.v1").await,
        0,
        "'laboratory2' is a distinct concept, not a specialisation of 'laboratory'"
    );
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.labora.v1").await,
        0,
        "'labora' is a bare prefix, not a '-'-delimited specialisation parent"
    );

    // (d) a different major does NOT match — the interface-reference major
    // boundary is hard (AM master07 §Referencing/§Querying).
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.laboratory.v2").await,
        0,
        "major v2 does not match v1 data (interface-reference major boundary)"
    );
}

/// Template-derived lineage matching: a query naming an **AOM2-era** parent
/// archetype retrieves data committed under a specialisation child whose
/// concept shares nothing with the parent's — the lineage comes solely from the
/// stored ADL2 family.
///
/// Spec: AM `Identification` master07 §Supporting Archetype-based Querying —
/// "for specialised archetypes, the specialisation lineage can only be obtained
/// from the operational form of the archetype, found in the template used to
/// create the data"; AM `Identification` master03 §Legacy ADL 1.4 Semantics —
/// for AOM2-era identifiers the `-` separator "no longer has any semantic
/// significance", so a `.vMAJOR.MINOR.PATCH` query must NOT subsume by concept
/// prefix, while the major-only ADL 1.4 form keeps that rule (BASE
/// `architecture_overview` master10 §Design-time Relationships).
#[tokio::test]
async fn archetype_lineage_from_the_stored_adl2_family() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // The stored family: `hdl_result` specialises `lipid_panel`. The child's
    // concept is NOT a `lipid_panel-…` extension, so only the declared
    // `specialize` edge can connect them.
    svc.upload_artefact(adl2_source(
        "archetype",
        "openEHR-EHR-OBSERVATION.lipid_panel.v1.0.0",
        None,
    ))
    .await
    .expect("upload the ADL2 parent archetype");
    svc.upload_artefact(adl2_source(
        "archetype",
        "openEHR-EHR-OBSERVATION.hdl_result.v1.0.0",
        Some("openEHR-EHR-OBSERVATION.lipid_panel.v1"),
    ))
    .await
    .expect("upload the specialised ADL2 child archetype");

    let ehr_id = create_ehr(&svc).await;
    // Data under the CHILD archetype. The `archetype_details.archetype_id` in
    // committed data is an RM `ARCHETYPE_ID`, whose lexical form is major-only
    // (BASE `base_types` master05 §Syntaxes: `version_id = '0' | non-zero-digit,
    // [number]`), so the interface form of the child id is what data carries.
    create_comp_arch(
        &svc,
        &ehr_id,
        "child",
        90.0,
        "openEHR-EHR-OBSERVATION.hdl_result.v1",
    )
    .await;
    // Data under a `-`-extended concept with NO stored artefact — the ADL 1.4
    // heuristic's territory, and the control for the AOM2-era case.
    create_comp_arch(
        &svc,
        &ehr_id,
        "legacy-style child",
        95.0,
        "openEHR-EHR-OBSERVATION.lipid_panel-extra.v1",
    )
    .await;

    // (a) THE EXIT CRITERION — the parent query retrieves the specialisation
    // child's data, which no string rule could have matched.
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.lipid_panel.v1.0.0").await,
        1,
        "an AOM2-era parent query retrieves data committed under its stored \
         specialisation child (AM master07 §Supporting Archetype-based Querying)"
    );

    // (b) the AOM2-era form does NOT subsume by concept prefix — the
    // `lipid_panel-extra` data is matched only by the ADL 1.4 form.
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.lipid_panel.v1").await,
        2,
        "the ADL 1.4-form query keeps its `-` prefix rule AND gains the stored \
         lineage: both the specialisation child and the `-extra` data"
    );

    // (c) an unrelated archetype with no stored family matches nothing.
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.unrelated.v1.0.0").await,
        0,
        "an identifier outside the stored family resolves to itself alone"
    );

    // (d) the child predicate still matches only the child.
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.hdl_result.v1").await,
        1,
        "the specialisation child's own predicate matches only its data"
    );

    // (e) the interface-reference major boundary stays hard for the lineage
    // path too (AM master07 §Referencing).
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.lipid_panel.v2.0.0").await,
        0,
        "a differing major denotes a different logical archetype"
    );

    // (f) deleting the child artefact retracts the lineage edge (the memo is
    // invalidated on the write), leaving the parent query at exact matching.
    svc.delete_artefact("openEHR-EHR-OBSERVATION.hdl_result.v1.0.0".to_owned())
        .await
        .expect("delete the specialised child artefact");
    assert_eq!(
        count_obs(&svc, &ehr_id, "openEHR-EHR-OBSERVATION.lipid_panel.v1.0.0").await,
        0,
        "with the stored family gone the parent query no longer subsumes the child"
    );
}

#[tokio::test]
async fn all_versions_includes_a_branch_row() {
    use ferroehr::ids::{EhrId, VoId};
    use ferroehr::storage::version_repo::import::{VerbatimVersionRow, insert_version_verbatim};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;
    let ovid = create_comp(&svc, &ehr_id, "v", 10.0).await;
    let vo_id = ovid.split("::").next().unwrap().to_owned();
    let vo: Uuid = vo_id.parse().expect("vo_id uuid");
    let ehr_uuid: Uuid = ehr_id.parse().expect("ehr_id uuid");

    // A branch `1.1.1` off trunk node 1, minted by the same system: the live
    // write paths never branch (RM common master06 §Copying §Subsequent Local
    // Modifications), so the row arrives the way an EHR_EXTRACT import brings
    // it, and its node rows are the trunk version's re-keyed to the branch.
    let (contribution_id, commit_audit_id, system): (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT contribution_id, commit_audit_id, creating_system_id FROM version \
             WHERE vo_id = $1 AND sys_version = 1",
    )
    .bind(vo)
    .fetch_one(&pool)
    .await
    .expect("the trunk row's provenance");
    let mut conn = pool.acquire().await.expect("connection");
    insert_version_verbatim(
        &mut conn,
        &VerbatimVersionRow {
            vo_id: VoId(vo),
            kind: "COMPOSITION",
            ehr_id: Some(EhrId(ehr_uuid)),
            sys_version: 2,
            trunk_version: 1,
            branch_number: 1,
            branch_version: 1,
            preceding_version_uid: None,
            other_input_version_uids: None,
            committed_at: Some("2026-01-01T00:00:00Z"),
            lifecycle_state: "532",
            contribution_id,
            commit_audit_id,
            template_id: None,
            signature: None,
            signature_client_supplied: false,
            creating_system_id: &system,
            wrapped_original: None,
            body: None,
        },
    )
    .await
    .expect("a branch row off trunk 1");
    drop(conn);
    sqlx::query(
        "INSERT INTO node SELECT (json_populate_record(n, '{\"sys_version\": 2}'::json)).* \
         FROM node n WHERE n.vo_id = $1 AND n.sys_version = 1",
    )
    .bind(vo)
    .execute(&pool)
    .await
    .expect("the branch version's node rows");

    // LATEST_VERSION is the trunk head and does not see the branch.
    let r = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&r)[0][0],
        json!(1),
        "LATEST_VERSION → the trunk head alone"
    );

    // ALL_VERSIONS applies no lineage filter: the branch row is a version too.
    let r = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&r)[0][0],
        json!(2),
        "ALL_VERSIONS → trunk 1 and branch 1.1.1"
    );
    let r = run_aql(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    let uids: Vec<String> = rows(&r)
        .iter()
        .map(|row| row[0].as_str().expect("a uid string").to_owned())
        .collect();
    assert!(
        uids.iter()
            .any(|uid| uid == &format!("{vo_id}::{system}::1.1.1")),
        "the branch version's OBJECT_VERSION_ID is served: {uids:?}"
    );
}

#[tokio::test]
async fn latest_versus_all_versions() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;
    let ovid = create_comp(&svc, &ehr_id, "v", 10.0).await;
    let vo_id = ovid.split("::").next().unwrap().to_owned();

    // Update the composition twice → sys_version 2 and 3. Each update supplies
    // the current version_uid as `If-Match` (optimistic concurrency).
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse().expect("ehr_id uuid"));
    let vo_uuid: ferroehr::ids::VoId = vo_id.parse().expect("vo_id uuid");
    let mut current = ovid.clone();
    for magnitude in [20.0, 30.0] {
        // NOTE: the SM `update_composition` returns the new
        // `version_uid` directly (the old `.meta.uid`).
        current = svc
            .update_composition(
                ehr_uuid,
                vo_uuid,
                uv(&composition("v", magnitude), "251", Some(&current)),
            )
            .await
            .unwrap_or_else(|e| panic!("update_composition {magnitude}: {e:?}"))
            .version_uid();
    }

    // LATEST_VERSION (the default) sees one version.
    let r = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&r)[0][0],
        json!(1),
        "LATEST_VERSION → 1 current version"
    );

    // ALL_VERSIONS sees all three — a capability EHRbase never had.
    let r = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&r)[0][0], json!(3), "ALL_VERSIONS → all 3 versions");

    // The latest magnitude is the last update.
    let aql = format!(
        "SELECT c/content[{OBS_ARCHETYPE}]/{MAG_PATH} AS m FROM EHR e CONTAINS COMPOSITION c"
    );
    let r = run_aql(&svc, &aql, ehr_scope(&ehr_id)).await;
    assert_eq!(
        rows(&r)[0][0].as_f64().unwrap(),
        30.0,
        "latest magnitude is 30"
    );

    // uid synthesis: the synthesized `c/uid/value` is version-correct under both scopes.
    // LATEST → the current (v3) OBJECT_VERSION_ID.
    let r = run_aql(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&r)[0][0],
        json!(current),
        "LATEST c/uid/value is the current version id"
    );

    // ALL_VERSIONS → one distinct id per version (trees 1/2/3), all on this vo.
    let r = run_aql(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    let mut uids: Vec<String> = rows(&r)
        .iter()
        .map(|row| row[0].as_str().expect("uid").to_owned())
        .collect();
    uids.sort();
    assert_eq!(uids.len(), 3, "one uid per version: {uids:?}");
    assert!(
        uids.iter().all(|u| u.starts_with(&format!("{vo_id}::"))),
        "every uid is on this versioned object: {uids:?}"
    );
    let mut trees: Vec<&str> = uids
        .iter()
        .map(|u| u.rsplit("::").next().unwrap())
        .collect();
    trees.sort_unstable();
    assert_eq!(trees, vec!["1", "2", "3"], "version trees 1/2/3: {uids:?}");
    assert!(
        uids.contains(&ovid) && uids.contains(&current),
        "the v1 and v3 endpoints are present: {uids:?}"
    );
}

/// The query-population gate, mandated by the SM `I_QUERY_SERVICE` interface for
/// both `execute_stored_query` and `execute_ad_hoc_query`. The `ehr_ids`
/// parameter doc reads (verbatim,
/// `docs/specs/openehr/SM/docs/UML/classes/i_query_service.adoc`):
///
/// > Specific set of EHRs on which to execute the query. If none supplied, a
/// > full population query will be performed on all EHRs whose status has the
/// > `is_queryable` flag set to `True`.
///
/// So an ad-hoc population query (no `ehr_id` scope) must include EHRs whose
/// current `EHR_STATUS` is queryable and exclude those whose current `EHR_STATUS`
/// has `is_queryable = False`.
#[tokio::test]
async fn population_query_excludes_not_queryable_ehrs() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // Two EHRs; both start queryable (the default EHR_STATUS). Flip one off
    // through the canonical EHR_STATUS update path.
    let queryable = create_ehr(&svc).await;
    let hidden = create_ehr(&svc).await;
    set_not_queryable(&svc, &hidden).await;

    // A full population query: NO ehr_id scope supplied (`ehr_ids` = none).
    let r = run_aql(
        &svc,
        "SELECT e/ehr_id/value FROM EHR e",
        AqlQueryRequest::default(),
    )
    .await;
    let ids: Vec<String> = rows(&r)
        .iter()
        .map(|row| row[0].as_str().expect("ehr_id cell").to_owned())
        .collect();

    assert!(
        ids.contains(&queryable),
        "the queryable EHR is in the population result set: {ids:?}"
    );
    assert!(
        !ids.contains(&hidden),
        "the non-queryable EHR is excluded from the population result set \
         (is_queryable = True gate): {ids:?}"
    );
}

/// A directly-scoped query (`ehr_ids` supplied) is NOT population-gated: a
/// non-queryable EHR's data is still returned when the caller names it in the
/// scope set. Per `i_query_service.adoc` the `is_queryable` gate governs only
/// the full-population case (no `ehr_ids`); an explicit scope targets specific
/// EHRs regardless of the flag. Pins that flipping `is_queryable` off changes
/// only the full-population result, never a scoped read.
#[tokio::test]
async fn scoped_query_bypasses_the_population_gate() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let hidden = create_ehr(&svc).await;
    create_comp(&svc, &hidden, "BP", 80.0).await;
    set_not_queryable(&svc, &hidden).await;

    // Full population (no ehr_id scope) excludes the non-queryable EHR …
    let full = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c",
        AqlQueryRequest::default(),
    )
    .await;
    assert_eq!(
        rows(&full)[0][0],
        json!(0),
        "the non-queryable EHR is excluded from the full-population count"
    );

    // … but a query scoped directly to it still returns its composition.
    let scoped = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&hidden),
    )
    .await;
    assert_eq!(
        rows(&scoped)[0][0],
        json!(1),
        "the scoped query bypasses the is_queryable gate"
    );
}

/// Multi-EHR scoping (`ehr_ids: List<UUID>`,
/// `docs/specs/openehr/SM/docs/UML/classes/i_query_service.adoc`): a query
/// scoped to a *set* of EHRs must span exactly that set — no more, no fewer.
/// Three EHRs each hold one composition; scoping to two of them counts two.
#[tokio::test]
async fn multi_ehr_ids_scopes_to_the_set() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let a = create_ehr(&svc).await;
    let b = create_ehr(&svc).await;
    let c = create_ehr(&svc).await;
    create_comp(&svc, &a, "BP", 80.0).await;
    create_comp(&svc, &b, "BP", 90.0).await;
    create_comp(&svc, &c, "BP", 100.0).await;

    // Scope to A + B (of A/B/C): the set-scoped count spans only those two.
    let r = run_aql(
        &svc,
        "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope_multi(&[&a, &b]),
    )
    .await;
    assert_eq!(
        rows(&r)[0][0],
        json!(2),
        "the two scoped EHRs, not the third"
    );
}

/// A well-formed but non-existent `ehr_id` in the scope set raises
/// `ehr_id_does_not_exist` (`i_query_service.adoc` declares the error); a
/// malformed id is a `precondition_violation` (`400`).
#[tokio::test]
async fn absent_ehr_id_raises_ehr_id_does_not_exist() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let real = create_ehr(&svc).await;
    let ghost = Uuid::now_v7().to_string();

    // A well-formed, absent id in the set → ehr_id_does_not_exist.
    let err = svc
        .execute_ad_hoc_query(
            "SELECT COUNT(*) FROM EHR e".to_owned(),
            ehr_scope_multi(&[&real, &ghost]),
        )
        .await
        .expect_err("an absent ehr_id must be rejected");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::EhrIdDoesNotExist,
                ..
            }
        ),
        "absent ehr_id is ehr_id_does_not_exist, got {err:?}"
    );

    // A malformed id → precondition_violation.
    let err = svc
        .execute_ad_hoc_query(
            "SELECT COUNT(*) FROM EHR e".to_owned(),
            ehr_scope_multi(&["not-a-uuid"]),
        )
        .await
        .expect_err("a malformed ehr_id must be rejected");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ),
        "malformed ehr_id is precondition_violation, got {err:?}"
    );
}

/// `RESULT_SET.meta._executed_aql` carries the parameter-SUBSTITUTED query text,
/// while `q` keeps the query as submitted (ITS-REST `schemas/query/ResultSet`;
/// QUERY §Parameters). A `$magnitude` bound to a number renders as the literal;
/// a `$name` bound to a string renders as a quoted AQL string literal.
#[tokio::test]
async fn executed_aql_substitutes_bound_parameters() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;
    create_comp(&svc, &ehr_id, "BP", 120.0).await;

    let aql = format!(
        "SELECT o/{MAG_PATH} AS m FROM EHR e CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         WHERE o/{MAG_PATH} > $min"
    );
    let mut req = ehr_scope(&ehr_id);
    req.parameters = BTreeMap::from([("min".to_owned(), json!(50))]);
    let r = run_aql(&svc, &aql, req).await;

    let q = r["q"].as_str().expect("q");
    let executed = r["meta"]["_executed_aql"].as_str().expect("_executed_aql");
    assert!(q.contains("$min"), "q keeps the original $parameter: {q}");
    assert!(
        !executed.contains("$min"),
        "_executed_aql has the parameter substituted: {executed}"
    );
    assert!(
        executed.contains("> 50"),
        "the bound value is rendered into the executed text: {executed}"
    );
}

/// The assembled query response carries the ITS-REST 1.1.0 `RESULT_SET` shape.
///
/// Asserts only what the schema requires:
/// * `rows` is present and an array — the sole `required` field of
///   `docs/specs/openehr/ITS-REST/specifications/schemas/query/ResultSet.yaml`.
/// * every `columns[]` entry carries a `name` — the sole `required` field of
///   `schemas/query/ResultSetColumn.yaml`.
/// * each row is an array whose length equals the number of columns —
///   `schemas/query/ResultSetRow.yaml`: "A set of cells representing a
///   `RESULT_SET` row, one cell for each column."
#[tokio::test]
async fn result_set_carries_the_its_rest_shape() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;
    create_comp(&svc, &ehr_id, "BP", 120.0).await;

    // A two-column projection over the seeded composition (deterministic via the
    // ehr_id scope), so the row/column relationship is meaningful.
    let r = run_aql(
        &svc,
        "SELECT e/ehr_id/value, c/name/value FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;

    // ResultSet.yaml `required: [rows]`.
    let rows = r["rows"].as_array().expect("`rows` is a (required) array");
    assert!(
        !rows.is_empty(),
        "the seeded composition yields at least one row"
    );

    // ResultSetColumn.yaml `required: [name]`.
    let columns = r["columns"].as_array().expect("`columns` is an array");
    for col in columns {
        assert!(
            col["name"].as_str().is_some(),
            "every column carries a `name`: {col:?}"
        );
    }

    // ResultSetRow.yaml: one cell per column.
    for row in rows {
        let cells = row.as_array().expect("each row is an array of cells");
        assert_eq!(
            cells.len(),
            columns.len(),
            "row cell count equals column count (one cell per column)"
        );
    }
}

/// `e/ehr_status` on an EHR-typed variable (B6 cluster 2; ECC-QRY-006/010, the
/// A/106 `get_ehrs` golden). EHR is not a `node` in the store and `EHR_STATUS`
/// is a *separate* versioned object (RM 1.2.0 `EHR.ehr_status`), so the engine
/// resolves the path by joining the EHR's current `EHR_STATUS` VO and
/// reassembling it — rather than rejecting the query. Also exercises the exact
/// A/106 SELECT list (`ehr_id`, `time_created`, `system_id`, `ehr_status`) and
/// leaf extraction under `ehr_status`.
///
/// Golden columns:
/// `docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets/`
/// `query/expected_results/{empty_db,loaded_db}/A/106_get_ehrs.json`.
#[tokio::test]
async fn ehr_status_on_ehr_variable() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;

    // (1) Whole EHR_STATUS reassembled through the engine-level join.
    let r = run_aql(&svc, "SELECT e/ehr_status FROM EHR e", ehr_scope(&ehr_id)).await;
    assert_eq!(rows(&r).len(), 1, "one EHR in the scoped result set");
    let status = &rows(&r)[0][0];
    assert_eq!(
        status["_type"], "EHR_STATUS",
        "the cell is a reassembled EHR_STATUS object: {status:?}"
    );
    assert!(
        status.get("subject").is_some(),
        "the reassembled EHR_STATUS carries its mandatory `subject`: {status:?}"
    );
    assert_eq!(
        status["is_queryable"],
        json!(true),
        "default EHR_STATUS is queryable: {status:?}"
    );

    // (2) The exact A/106 SELECT list resolves. The golden's column metadata
    // (name `#i` + path) is the data-independent oracle.
    let r = run_aql(
        &svc,
        "SELECT e/ehr_id, e/time_created, e/system_id, e/ehr_status FROM EHR e",
        ehr_scope(&ehr_id),
    )
    .await;
    let cols = r["columns"].as_array().expect("columns array");
    let names: Vec<&str> = cols.iter().map(|c| c["name"].as_str().unwrap()).collect();
    let paths: Vec<&str> = cols.iter().map(|c| c["path"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["#0", "#1", "#2", "#3"], "A/106 column names");
    assert_eq!(
        paths,
        vec!["/ehr_id", "/time_created", "/system_id", "/ehr_status"],
        "A/106 column paths"
    );
    assert_eq!(rows(&r).len(), 1, "one row for the scoped EHR");
    assert_eq!(
        rows(&r)[0][3]["_type"],
        "EHR_STATUS",
        "the fourth column is the EHR_STATUS object"
    );

    // (3) Leaf extraction under ehr_status: an inline scalar (`is_queryable`)
    // and an inline object attribute (`subject`, a PARTY_PROXY kept inline in
    // the root fragment).
    let r = run_aql(
        &svc,
        "SELECT e/ehr_status/is_queryable, e/ehr_status/subject FROM EHR e",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&r).len(), 1, "one row");
    assert_eq!(
        rows(&r)[0][0],
        json!(true),
        "e/ehr_status/is_queryable extracts the boolean leaf"
    );
    assert!(
        rows(&r)[0][1]["_type"].as_str().is_some(),
        "e/ehr_status/subject extracts the PARTY_PROXY object: {:?}",
        rows(&r)[0][1]
    );
}

/// The empty-DB shape of the A/106 golden: the query must resolve (200, not a
/// 400 reject) and return the four columns with **no** rows when no EHR exists.
/// Matches `expected_results/empty_db/A/106_get_ehrs.json` (columns + `rows: []`).
#[tokio::test]
async fn ehr_status_query_empty_db() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let r = run_aql(
        &svc,
        "SELECT e/ehr_id, e/time_created, e/system_id, e/ehr_status FROM EHR e",
        AqlQueryRequest::default(),
    )
    .await;
    let cols = r["columns"].as_array().expect("columns array");
    let paths: Vec<&str> = cols.iter().map(|c| c["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        vec!["/ehr_id", "/time_created", "/system_id", "/ehr_status"],
        "A/106 columns present on the empty DB"
    );
    assert!(rows(&r).is_empty(), "no EHRs → empty result set");
}

/// The single-row function set executes end-to-end on PostgreSQL (QUERY
/// master03 §Functions: string, numeric, date/time) — each represented in the
/// IR and executed through to SQL, not merely parsed.
#[tokio::test]
async fn scalar_functions_execute() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;
    create_comp(&svc, &ehr_id, "BP", 81.5).await;

    // String functions over c/name/value = "BP".
    let r = run_aql(
        &svc,
        "SELECT length(c/name/value), position('P', c/name/value), \
                substring(c/name/value, 1, 1), concat(c/name/value, '!'), \
                concat_ws('-', c/name/value, 'x'), contains(c/name/value, 'B') \
         FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    let row = &rows(&r)[0];
    assert_eq!(row[0], json!(2), "LENGTH('BP') = 2");
    assert_eq!(row[1], json!(2), "POSITION is 1-based; 'P' is the 2nd char");
    assert_eq!(row[2], json!("B"), "SUBSTRING 1-based, length 1");
    assert_eq!(row[3], json!("BP!"), "CONCAT");
    assert_eq!(row[4], json!("BP-x"), "CONCAT_WS with separator");
    assert_eq!(row[5], json!(true), "string CONTAINS");

    // Numeric functions over the magnitude 81.5.
    let aql = format!(
        "SELECT abs(o/{MAG_PATH}), ceil(o/{MAG_PATH}), floor(o/{MAG_PATH}), \
                round(o/{MAG_PATH}), round(o/{MAG_PATH}, 1), mod(o/{MAG_PATH}, 2) \
         FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS_ARCHETYPE}]"
    );
    let r = run_aql(&svc, &aql, ehr_scope(&ehr_id)).await;
    let row = &rows(&r)[0];
    assert_eq!(row[0], json!(81.5), "ABS");
    assert_eq!(row[1], json!(82), "CEIL returns Integer");
    assert_eq!(row[2], json!(81), "FLOOR returns Integer");
    assert_eq!(
        row[3],
        json!(82),
        "ROUND defaults to 0 decimals (81.5 → 82)"
    );
    assert_eq!(row[4], json!(81.5), "ROUND to 1 decimal");
    assert_eq!(row[5], json!(1.5), "MOD(81.5, 2)");

    // ROUND's rounding mode is half-AWAY-FROM-ZERO, in both signs and at a
    // decimal place — QUERY master03 §ROUND fixes no mode, so the adjudicated
    // mode is pinned here: a lowering change that drops the ::numeric cast
    // (double precision rounds ties to EVEN: 2.5 → 2) flips these
    // assertions, never a clinical number silently.
    for (magnitude, decimals, expected, label) in [
        (2.5, None, json!(3), "ROUND(2.5) = 3, away from zero"),
        (-2.5, None, json!(-3), "ROUND(-2.5) = -3, away from zero"),
        (
            0.125,
            Some(2),
            json!(0.13),
            "ROUND(0.125, 2) = 0.13 at a decimal place",
        ),
    ] {
        let ehr = create_ehr(&svc).await;
        create_comp(&svc, &ehr, "ties", magnitude).await;
        let call = match decimals {
            None => format!("round(o/{MAG_PATH})"),
            Some(d) => format!("round(o/{MAG_PATH}, {d})"),
        };
        let aql = format!(
            "SELECT {call} FROM EHR e CONTAINS COMPOSITION c \
             CONTAINS OBSERVATION o[{OBS_ARCHETYPE}]"
        );
        let r = run_aql(&svc, &aql, ehr_scope(&ehr)).await;
        assert_eq!(rows(&r)[0][0], expected, "{label}: {r}");
    }

    // Date/time functions: shape checks (values are 'now').
    let r = run_aql(
        &svc,
        "SELECT current_date(), current_time(), now(), current_timezone() \
         FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    let row = &rows(&r)[0];
    let date = row[0].as_str().expect("CURRENT_DATE is a string");
    assert_eq!(date.len(), 10, "YYYY-MM-DD: {date}");
    let time = row[1].as_str().expect("CURRENT_TIME is a string");
    assert_eq!(time.len(), 8, "hh:mm:ss: {time}");
    let dt = row[2].as_str().expect("NOW is a string");
    assert!(
        dt.contains('T') && (dt.contains('+') || dt.contains('-')),
        "YYYY-MM-DDThh:mm:ss.sss±hh:mm: {dt}"
    );
    let tz = row[3].as_str().expect("CURRENT_TIMEZONE is a string");
    assert!(
        tz.contains(':') && (tz.starts_with('+') || tz.starts_with('-')),
        "±hh:mm: {tz}"
    );
}

// ── Promoted context_start ORDER BY + uid synthesis ─────────────────────────

/// An event COMPOSITION with an explicit `context.start_time`.
fn composition_at(name: &str, magnitude: f64, start_time: &str) -> Value {
    let mut c = composition(name, magnitude);
    c["context"]["start_time"] = json!({ "_type": "DV_DATE_TIME", "value": start_time });
    c
}

/// A composition whose context carries `start_time` AND `end_time` at the
/// given (possibly reduced) precision — one value on both the promoted path
/// (`start_time` → `node.context_start`) and the plain jsonb path
/// (`end_time`).
fn composition_spanning(name: &str, temporal: &str) -> Value {
    let mut c = composition_at(name, 1.0, temporal);
    c["context"]["end_time"] = json!({ "_type": "DV_DATE_TIME", "value": temporal });
    c
}

/// Partial-precision date/times (RM `data_types` master07 §Partial Date/Times)
/// compare and order instead of erroring: the temporal coercion routes both
/// sides through the total `ext.openehr_timestamp` floor (partial dates
/// assume the first month/day), so a stored `2019` or `1985-06` — and a
/// reduced-precision comparison literal — answers rows, never the SQLSTATE
/// 22007-driven 400 it answered before. Exercised on BOTH lowerings: the
/// promoted `context_start` column (`start_time`) and the jsonb extraction
/// (`end_time`).
#[tokio::test]
async fn partial_precision_temporals_compare_and_order() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    for (name, temporal) in [
        ("year-only", "2019"),
        ("year-month", "1985-06"),
        ("full", "2020-06-15T10:00:00Z"),
    ] {
        svc.create_composition(
            ehr_id.parse().expect("ehr_id uuid"),
            uv(&composition_spanning(name, temporal), "249", None),
        )
        .await
        .unwrap_or_else(|e| panic!("create ({name}): {e:?}"));
    }

    for path in ["c/context/start_time/value", "c/context/end_time/value"] {
        // `<` against a partial literal: 2019 and 1985-06 floor before 2020.
        let lt = run_aql(
            &svc,
            &format!("SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c WHERE {path} < '2020'"),
            ehr_scope(&ehr_id),
        )
        .await;
        assert_eq!(
            rows(&lt).len(),
            2,
            "{path} < '2020' floors both sides: {lt}"
        );

        // `>` against a partial literal: everything after 1985-01-01.
        let gt = run_aql(
            &svc,
            &format!("SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c WHERE {path} > '1985'"),
            ehr_scope(&ehr_id),
        )
        .await;
        assert_eq!(rows(&gt).len(), 3, "{path} > '1985': {gt}");

        // `=` at equal reduced precision (floor equality).
        let eq = run_aql(
            &svc,
            &format!(
                "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c WHERE {path} = '1985-06'"
            ),
            ehr_scope(&ehr_id),
        )
        .await;
        assert_eq!(rows(&eq).len(), 1, "{path} = '1985-06': {eq}");
        assert_eq!(rows(&eq)[0][0], json!("year-month"));

        // Against NOW(): every stored value precedes the query instant.
        let now = run_aql(
            &svc,
            &format!("SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c WHERE {path} < NOW()"),
            ehr_scope(&ehr_id),
        )
        .await;
        assert_eq!(rows(&now).len(), 3, "{path} < NOW(): {now}");

        // ORDER BY floors partials onto the same axis.
        let ordered = run_aql(
            &svc,
            &format!("SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c ORDER BY {path} ASC"),
            ehr_scope(&ehr_id),
        )
        .await;
        let names: Vec<&str> = rows(&ordered)
            .iter()
            .map(|r| r[0].as_str().expect("name"))
            .collect();
        assert_eq!(
            names,
            ["year-month", "year-only", "full"],
            "{path} orders by the floored instant: {ordered}"
        );
    }

    // A time-only value floors onto 0001-01-01 at the function seam (a
    // DV_TIME value cannot legally sit on these DV_DATE_TIME attributes, so
    // the seam is pinned directly).
    let anchored: Option<String> =
        sqlx::query_scalar("SELECT ext.openehr_timestamp('12:00')::text")
            .fetch_one(&db.pool())
            .await
            .expect("time-only floors, never errors");
    assert!(
        anchored.is_some_and(|t| t.starts_with("0001-01-01 12:00:00")),
        "time-only anchors on 0001-01-01"
    );
    let garbage: Option<String> =
        sqlx::query_scalar("SELECT ext.openehr_timestamp('not-a-date')::text")
            .fetch_one(&db.pool())
            .await
            .expect("garbage is NULL, never an error");
    assert_eq!(garbage, None, "malformed stored text is a comparison miss");
}

/// A persistent COMPOSITION with no `context` (RM ehr master03
/// §COMPOSITION.context [0..1]) — its promoted `context_start` is NULL.
fn composition_persistent(name: &str, magnitude: f64) -> Value {
    let mut c = composition(name, magnitude);
    if let Some(obj) = c.as_object_mut() {
        obj.remove("context");
    }
    c["category"] = json!({
        "_type": "DV_CODED_TEXT",
        "value": "persistent",
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": "431"
        }
    });
    c
}

async fn create_comp_body(svc: &FerroEhrService, ehr_id: &str, body: Value, name: &str) -> String {
    svc.create_composition(ehr_id.parse().expect("ehr_id uuid"), uv(&body, "249", None))
        .await
        .unwrap_or_else(|e| panic!("create_composition ({name}): {e:?}"))
        .version_uid()
}

/// uid synthesis, end to end against real PG 18: the patient-dashboard shape orders
/// by the promoted `node.context_start` column (verified byte-equal to the
/// pre-promotion correlated-subquery ordering in both directions, including the
/// NULL-context row), and `c/uid/value` returns the exact server-assigned
/// `OBJECT_VERSION_ID` (uid synthesis) — never null.
#[tokio::test]
async fn dashboard_context_start_ordering_and_uid() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = create_ehr(&svc).await;

    // Distinct start times, seeded out of chronological order so ORDER BY (not
    // insertion order) drives the result.
    let uid_mid = create_comp_body(
        &svc,
        &ehr_id,
        composition_at("mid", 1.0, "2021-06-15T12:00:00Z"),
        "mid",
    )
    .await;
    let _uid_old = create_comp_body(
        &svc,
        &ehr_id,
        composition_at("old", 2.0, "2020-01-01T00:00:00Z"),
        "old",
    )
    .await;
    let uid_new = create_comp_body(
        &svc,
        &ehr_id,
        composition_at("new", 3.0, "2022-12-31T23:59:59Z"),
        "new",
    )
    .await;
    // A persistent composition → NULL context_start.
    let _uid_persist = create_comp_body(
        &svc,
        &ehr_id,
        composition_persistent("persist", 4.0),
        "persist",
    )
    .await;

    // ── DESC: NULLS FIRST (PG default for DESC), then newest→oldest. This is the
    // identical ordering the pre-promotion `(… )::timestamptz DESC` subquery
    // produced (a NULL sub-select and a NULL column sort identically). ──
    let r = run_aql(
        &svc,
        "SELECT c/uid/value, c/name/value FROM EHR e CONTAINS COMPOSITION c \
         ORDER BY c/context/start_time/value DESC",
        ehr_scope(&ehr_id),
    )
    .await;
    for row in rows(&r) {
        assert!(
            row[0].as_str().is_some(),
            "uid synthesis: c/uid/value must be a non-null OBJECT_VERSION_ID: {row:?}"
        );
    }
    let names_desc: Vec<&str> = rows(&r)
        .iter()
        .map(|row| row[1].as_str().expect("name"))
        .collect();
    assert_eq!(
        names_desc,
        vec!["persist", "new", "mid", "old"],
        "DESC: NULL context first, then newest→oldest"
    );

    // ── ASC: oldest→newest, NULLS LAST (PG default for ASC). ──
    let r = run_aql(
        &svc,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         ORDER BY c/context/start_time/value ASC",
        ehr_scope(&ehr_id),
    )
    .await;
    let names_asc: Vec<&str> = rows(&r)
        .iter()
        .map(|row| row[0].as_str().expect("name"))
        .collect();
    assert_eq!(
        names_asc,
        vec!["old", "mid", "new", "persist"],
        "ASC: oldest→newest, NULL context last"
    );

    // ── uid synthesis: the synthesized uid equals the OBJECT_VERSION_ID create returned. ──
    let r = run_aql(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/name/value = 'new'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&r)[0][0],
        json!(uid_new),
        "c/uid/value == the created OBJECT_VERSION_ID"
    );

    // ── uid synthesis: `c/uid` returns the OBJECT_VERSION_ID object for a specific row. ──
    let r = run_aql(
        &svc,
        "SELECT c/uid FROM EHR e CONTAINS COMPOSITION c WHERE c/name/value = 'mid'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&r)[0][0],
        json!({ "_type": "OBJECT_VERSION_ID", "value": uid_mid }),
        "c/uid is the OBJECT_VERSION_ID object"
    );
}

/// The AQL plan cache is transparent: a repeated query text reuses the
/// lowered plan (a cache hit) yet returns byte-identical results, and the
/// per-request parameter values + paging window still bind correctly on top of
/// the shared plan. No openEHR spec governs the cache — our own performance
/// design — so this asserts behaviour equivalence, not a spec clause.
#[tokio::test]
async fn plan_cache_reuses_plan_and_binds_per_request() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let ehr_id = create_ehr(&svc).await;
    create_comp(&svc, &ehr_id, "BP", 80.0).await;
    create_comp(&svc, &ehr_id, "BP", 100.0).await;
    create_comp(&svc, &ehr_id, "BP", 120.0).await;

    // The RESULT_SET content that must be identical across runs (the `meta.id`
    // and `meta._created` are volatile by design and excluded).
    let content = |r: &Value| json!({ "columns": r["columns"], "rows": r["rows"] });

    // ── Same query twice → cache miss then hit, identical results. ──────────
    let count_q = "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c";
    let before = svc.plan_cache().stats();
    let r1 = run_aql(&svc, count_q, ehr_scope(&ehr_id)).await;
    let after_first = svc.plan_cache().stats();
    assert_eq!(after_first.misses, before.misses + 1, "first run is a miss");
    assert_eq!(after_first.hits, before.hits, "first run is not a hit");

    let r2 = run_aql(&svc, count_q, ehr_scope(&ehr_id)).await;
    let after_second = svc.plan_cache().stats();
    assert_eq!(
        after_second.hits,
        after_first.hits + 1,
        "the repeat query is served from the plan cache (no re-parse)"
    );
    assert_eq!(after_second.misses, after_first.misses, "no new miss");
    assert_eq!(
        content(&r1),
        content(&r2),
        "cached plan yields identical results"
    );
    assert_eq!(rows(&r1)[0][0], json!(3), "3 compositions in scope");

    // ── $parameter values bind per request on top of the cached plan. ───────
    let min_q = format!(
        "SELECT o/{MAG_PATH} AS m FROM EHR e CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         WHERE o/{MAG_PATH} > $min ORDER BY o/{MAG_PATH}"
    );
    let mut low_scope = ehr_scope(&ehr_id);
    low_scope.parameters = BTreeMap::from([("min".to_owned(), json!(90))]);
    let res_low = run_aql(&svc, &min_q, low_scope).await;
    let stats_after_low = svc.plan_cache().stats();

    let mut high_scope = ehr_scope(&ehr_id);
    high_scope.parameters = BTreeMap::from([("min".to_owned(), json!(110))]);
    let res_high = run_aql(&svc, &min_q, high_scope).await;
    let stats_after_high = svc.plan_cache().stats();
    assert_eq!(
        stats_after_high.hits,
        stats_after_low.hits + 1,
        "the second parameterised run reuses the cached plan"
    );
    let mags = |r: &Value| {
        rows(r)
            .iter()
            .map(|row| row[0].as_f64().expect("magnitude"))
            .collect::<Vec<_>>()
    };
    assert_eq!(mags(&res_low), vec![100.0, 120.0], "$min=90 keeps 100,120");
    assert_eq!(
        mags(&res_high),
        vec![120.0],
        "$min=110 keeps only 120 — the bound value varies on the shared plan"
    );

    // ── REST `fetch` paging binds per request on top of the cached plan. ────
    let page_q = format!(
        "SELECT o/{MAG_PATH} FROM EHR e CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] ORDER BY o/{MAG_PATH}"
    );
    let mut req_fetch_one = ehr_scope(&ehr_id);
    req_fetch_one.fetch = Some(1);
    let page_one = run_aql(&svc, &page_q, req_fetch_one).await;
    let stats_one = svc.plan_cache().stats();

    let mut req_fetch_two = ehr_scope(&ehr_id);
    req_fetch_two.fetch = Some(2);
    let page_two = run_aql(&svc, &page_q, req_fetch_two).await;
    let stats_two = svc.plan_cache().stats();
    assert_eq!(
        stats_two.hits,
        stats_one.hits + 1,
        "the second paged run reuses the cached plan"
    );
    assert_eq!(rows(&page_one).len(), 1, "fetch=1 returns one row");
    assert_eq!(
        rows(&page_two).len(),
        2,
        "fetch=2 returns two rows from the same plan"
    );
}

/// The `ehr_id` execution scope binds the EHR SOURCE itself, not only
/// versioned-object roots: a bare `SELECT … FROM EHR e` scoped to one EHR
/// returns exactly that EHR's row, never the population (ITS-REST query
/// `Request.md` §Common Headers and Query Parameters: `ehr_id` — "used to
/// execute the query within a single EHR context").
#[tokio::test]
async fn ehr_scope_binds_the_bare_ehr_source() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let a = create_ehr(&svc).await;
    let _b = create_ehr(&svc).await; // a second EHR the scope must exclude

    let r = run_aql(
        &svc,
        "SELECT e/ehr_id/value AS id FROM EHR e",
        ehr_scope(&a),
    )
    .await;
    let got = rows(&r);
    assert_eq!(got.len(), 1, "scoped bare-EHR query returns one row");
    assert_eq!(
        got[0][0].as_str(),
        Some(a.as_str()),
        "and it is the scoped EHR"
    );
}

/// The two first-execution 500s from the #967 wire batch, reproduced at the
/// service seam and pinned green (QUERY master03 §DISTINCT/§LIMIT and
/// §Date/time functions): SELECT DISTINCT with ORDER BY + LIMIT, and a
/// temporal comparison against `NOW()`.
#[tokio::test]
async fn distinct_order_by_limit_and_now_comparison() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let ehr_id = create_ehr(&svc).await;
    create_comp(&svc, &ehr_id, "BP", 80.0).await;
    create_comp(&svc, &ehr_id, "BP", 120.0).await;
    create_comp(&svc, &ehr_id, "HR", 100.0).await;

    // DISTINCT + ORDER BY + LIMIT: duplicates filter BEFORE the limit
    // (master03 §LIMIT: "applies to remaining rows, after duplicates were
    // filtered out"), and PG's DISTINCT/ORDER BY compatibility rule must be
    // honoured by the generated SQL.
    let aql = "SELECT DISTINCT c/name/value AS name \
               FROM EHR e CONTAINS COMPOSITION c \
               ORDER BY c/name/value ASC LIMIT 3";
    let r = run_aql(&svc, aql, ehr_scope(&ehr_id)).await;
    let names: Vec<&str> = rows(&r)
        .iter()
        .map(|row| row[0].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["BP", "HR"], "2 distinct names, ordered");

    // DISTINCT ordered by an UNSELECTED expression is a typed reject (no
    // spec semantics; the DBMS forbids it) — never a 500.
    let err = svc
        .execute_ad_hoc_query(
            format!(
                "SELECT DISTINCT c/name/value FROM EHR e CONTAINS COMPOSITION c \
                 CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] ORDER BY o/{MAG_PATH}"
            ),
            ehr_scope(&ehr_id),
        )
        .await
        .expect_err("DISTINCT + unselected sort key must be a typed reject");
    assert!(
        err.to_string().contains("must sort by a selected column"),
        "typed reject, not a DB error: {err}"
    );

    // Temporal comparison against NOW(): every committed context start_time
    // precedes the query instant (master03 §CURRENT_DATE_TIME or NOW).
    let aql = "SELECT c/uid/value AS uid \
               FROM EHR e CONTAINS COMPOSITION c \
               WHERE c/context/start_time/value < NOW()";
    let r = run_aql(&svc, aql, ehr_scope(&ehr_id)).await;
    assert_eq!(rows(&r).len(), 3, "all compositions precede NOW()");
}

/// #1448 — LIKE and `matches` on an anchored MULTI-VALUED path lower
/// existentially (any-match), exactly as the comparison operators do: with two
/// sibling ELEMENTs on the same `items[at0004]` path where only the SECOND
/// satisfies the predicate, the row must still return. The scalar LIMIT-1
/// extraction this replaces picked an order-undefined single node and could
/// silently miss it. (QUERY master03 is silent on predicates over
/// multi-valued paths — any-match is the adjudicated own-design semantics
/// recorded on `data_leaf_exists`.)
#[tokio::test]
async fn like_and_matches_are_any_match_on_multi_valued_paths() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;

    let mut c = composition("multi-item", 1.0);
    // A second ELEMENT on the SAME items[at0004] path: first carries a
    // non-matching DV_TEXT, the second the value the predicates target.
    let items = c["content"][0]["data"]["events"][0]["data"]["items"]
        .as_array_mut()
        .expect("items");
    let mut first = items[0].clone();
    first["value"] = json!({ "_type": "DV_TEXT", "value": "alpha" });
    let mut second = items[0].clone();
    second["value"] = json!({ "_type": "DV_TEXT", "value": "zeta match" });
    *items = vec![first, second];
    svc.create_composition(ehr_id.parse().expect("ehr_id uuid"), uv(&c, "249", None))
        .await
        .expect("multi-item composition");

    let like = run_aql(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
         WHERE o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/value \
         LIKE 'zeta*'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&like).len(),
        1,
        "LIKE matches when ANY node on the multi-valued path satisfies it: {like}"
    );

    let matches = run_aql(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
         WHERE o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/value \
         MATCHES {'zeta match'}",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&matches).len(),
        1,
        "matches is any-match on the multi-valued path: {matches}"
    );

    // The non-matching pattern still returns nothing (no vacuous truth).
    let none = run_aql(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
         WHERE o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/value \
         LIKE 'omega*'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&none).len(), 0, "no node satisfies: {none}");
}

/// #2940 — QUERY master03 §Operators/LIKE: a backslash-escaped wildcard
/// (`\?`, `\*`) matches the LITERAL character. Against values containing no
/// `?` or `*`, the escaped patterns select nothing while their unescaped
/// twins select the row — both wildcards, symmetrically (4.0.11 honored `\*`
/// and let `\?` keep matching as the single-character wildcard).
#[tokio::test]
async fn like_escaped_wildcards_match_the_literal_character() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;

    let mut c = composition("escape-target", 1.0);
    c["content"][0]["data"]["events"][0]["data"]["items"][0]["value"] =
        json!({ "_type": "DV_TEXT", "value": "2026-01-01T03:00:00Z" });
    svc.create_composition(ehr_id.parse().expect("ehr_id uuid"), uv(&c, "249", None))
        .await
        .expect("escape-target composition");

    let path = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/value";
    for (pattern, expected, claim) in [
        (
            r"2026-01-01T0?:00:00Z",
            1,
            "unescaped ? is the one-char wildcard",
        ),
        (r"2026-01-01T0\?:00:00Z", 0, "escaped \\? is the literal ?"),
        (r"2026-01-01T0*", 1, "unescaped * is the any-run wildcard"),
        (r"2026-01-01T0\*", 0, "escaped \\* is the literal *"),
    ] {
        let result = run_aql(
            &svc,
            &format!(
                "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
                 WHERE {path} LIKE '{pattern}'"
            ),
            ehr_scope(&ehr_id),
        )
        .await;
        assert_eq!(rows(&result).len(), expected, "{claim}: {result}");
    }
}

/// A composition carrying MULTI-VALUED FRAGMENT attributes — two `links` on
/// the root, two `context/participations`, two composer `identifiers` — where
/// only the LAST element of each list satisfies the predicates below.
fn multi_fragment_composition() -> Value {
    let mut c = composition("multi-fragment", 7.0);
    c["links"] = json!([
        {
            "_type": "LINK",
            "meaning": { "_type": "DV_TEXT", "value": "problem" },
            "type": { "_type": "DV_TEXT", "value": "clinical" },
            "target": { "_type": "DV_EHR_URI", "value": "ehr://x/first" }
        },
        {
            "_type": "LINK",
            "meaning": { "_type": "DV_TEXT", "value": "issue" },
            "type": { "_type": "DV_TEXT", "value": "clinical" },
            "target": { "_type": "DV_EHR_URI", "value": "ehr://x/last" }
        }
    ]);
    c["context"]["participations"] = json!([
        {
            "_type": "PARTICIPATION",
            "function": { "_type": "DV_TEXT", "value": "observer" },
            "performer": { "_type": "PARTY_IDENTIFIED", "name": "Dr First" }
        },
        {
            "_type": "PARTICIPATION",
            "function": { "_type": "DV_TEXT", "value": "assistant" },
            "performer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Second" }
        }
    ]);
    c["composer"] = json!({
        "_type": "PARTY_IDENTIFIED",
        "name": "composer",
        "identifiers": [
            { "_type": "DV_IDENTIFIER", "id": "MRN-1" },
            { "_type": "DV_IDENTIFIER", "id": "MRN-2" }
        ]
    });
    c
}

/// Predicates over MULTI-VALUED FRAGMENT paths are any-match: with two
/// `links`, two `context/participations` and two composer `identifiers` where
/// only the LAST element satisfies each predicate, the row must return for
/// `=`, `LIKE`, `MATCHES` and `EXISTS` alike — the first-match scalar
/// extraction this replaces saw only element one. (QUERY master03 §WHERE is
/// silent on multi-valued predicates — the any-match adjudication on
/// `data_leaf_exists` covers the fragment half too.)
#[tokio::test]
async fn multi_valued_fragment_predicates_are_any_match() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    svc.create_composition(
        ehr_id.parse().expect("ehr_id uuid"),
        uv(&multi_fragment_composition(), "249", None),
    )
    .await
    .expect("multi-fragment composition");
    // A second composition with none of the list-valued attributes, so every
    // predicate below also proves it does NOT match vacuously.
    create_comp(&svc, &ehr_id, "plain", 1.0).await;

    // `=` on the root fragment list (links), last element only.
    let eq = run_aql(
        &svc,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/links/target/value = 'ehr://x/last'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&eq).len(), 1, "= matches the LAST link: {eq}");
    assert_eq!(rows(&eq)[0][0], json!("multi-fragment"));

    // `=` on an anchored fragment list (context → participations), last only.
    let anchored = run_aql(
        &svc,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/context/participations/performer/name = 'Dr Second'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&anchored).len(),
        1,
        "= matches the LAST participation: {anchored}"
    );

    // `LIKE` on the second identifier of the composer.
    let like = run_aql(
        &svc,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/composer/identifiers/id LIKE 'MRN-2*'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&like).len(),
        1,
        "LIKE matches the LAST identifier: {like}"
    );

    // `MATCHES` against a set only the last link satisfies.
    let matches = run_aql(
        &svc,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/links/target/value MATCHES {'ehr://x/last', 'ehr://other'}",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&matches).len(), 1, "MATCHES is any-match: {matches}");

    // `EXISTS` distinguishes the composition carrying links from the plain
    // one (the vendored base fixture has no links).
    let exists = run_aql(
        &svc,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE EXISTS c/links/target/value",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&exists).len(), 1, "EXISTS is per-match: {exists}");
    assert_eq!(rows(&exists)[0][0], json!("multi-fragment"));

    // A predicate no element satisfies still returns nothing (no vacuous truth).
    let none = run_aql(
        &svc,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/links/target/value = 'ehr://x/absent'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&none).len(), 0, "no element satisfies: {none}");
}

/// Projection over a multi-valued fragment path serves EVERY match as one
/// jsonb array cell, and NULL where the path matches nothing — QUERY master03
/// §Identified Paths is silent on the shape; the array cell is the decided
/// semantics (the first-match scalar served one target where the data held
/// two).
#[tokio::test]
async fn multi_valued_fragment_projection_serves_every_match() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    svc.create_composition(
        ehr_id.parse().expect("ehr_id uuid"),
        uv(&multi_fragment_composition(), "249", None),
    )
    .await
    .expect("multi-fragment composition");

    let r = run_aql(
        &svc,
        "SELECT c/links/target/value, c/context/participations/performer/name \
         FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&r)[0][0],
        json!(["ehr://x/first", "ehr://x/last"]),
        "every link target projects, in document order: {r}"
    );
    assert_eq!(
        rows(&r)[0][1],
        json!(["Dr First", "Dr Second"]),
        "every participation performer projects: {r}"
    );

    // The plain composition has no links: the cell is null, never [].
    let ehr2 = create_ehr(&svc).await;
    create_comp(&svc, &ehr2, "plain", 1.0).await;
    let empty = run_aql(
        &svc,
        "SELECT c/links/target/value FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr2),
    )
    .await;
    assert_eq!(
        rows(&empty)[0][0],
        Value::Null,
        "no match is a NULL cell: {empty}"
    );
}

/// A predicate ON a fragment step lowers as a jsonpath filter and selects the
/// matching element — the SECOND link here, so a first-element shortcut fails
/// this (QUERY master03 §Standard predicate at a non-structure step; the
/// filter values bind as jsonpath vars, never spliced).
#[tokio::test]
async fn fragment_step_predicates_filter_the_matching_elements() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    svc.create_composition(
        ehr_id.parse().expect("ehr_id uuid"),
        uv(&multi_fragment_composition(), "249", None),
    )
    .await
    .expect("multi-fragment composition");

    // Projection: only the filtered (second) link's target projects.
    let filtered = run_aql(
        &svc,
        "SELECT c/links[meaning/value = 'issue']/target/value \
         FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&filtered)[0][0],
        json!(["ehr://x/last"]),
        "the filter selects the SECOND link: {filtered}"
    );

    // Predicate: the filtered comparison matches through the second element.
    let hit = run_aql(
        &svc,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/links[meaning/value = 'issue']/target/value = 'ehr://x/last'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&hit).len(), 1, "filtered any-match: {hit}");

    // A filter no element satisfies yields nothing (no vacuous truth).
    let miss = run_aql(
        &svc,
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/links[meaning/value = 'absent']/target/value = 'ehr://x/last'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&miss).len(), 0, "no element passes the filter: {miss}");
}

/// A root predicate on a WHOLE-OBJECT projection serves the object where the
/// source matches and a NULL cell where it does not — row cardinality
/// untouched (QUERY master03 §Identified Paths; the NULL-cell shape mirrors
/// the guarded leaf reads).
#[tokio::test]
async fn whole_object_root_predicate_serves_null_for_non_matches() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    create_comp(&svc, &ehr_id, "minimal-root", 1.0).await;

    let r = run_aql(
        &svc,
        "SELECT c[openEHR-EHR-COMPOSITION.report.v1], c/name/value \
         FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&r).len(), 1, "the row survives: {r}");
    assert_eq!(
        rows(&r)[0][0],
        Value::Null,
        "a non-matching root serves a NULL cell: {r}"
    );

    let matching = run_aql(
        &svc,
        "SELECT c[openEHR-EHR-COMPOSITION.minimal.v1] \
         FROM EHR e CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&matching)[0][0]["_type"],
        json!("COMPOSITION"),
        "the matching root serves the whole object: {matching}"
    );
}

/// An uncoercible client binding is the CALLER's 400, never a 500 (#2593):
/// measured live, `"not-a-date"` against a temporal predicate reached the
/// driver as sqlstate 22007 and answered the opaque internal error.
#[tokio::test]
async fn an_uncoercible_temporal_binding_is_the_callers_400() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    create_comp(&svc, &ehr_id, "BP", 120.0).await;

    let aql = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
               WHERE c/context/start_time/value >= $from"
        .to_owned();
    let mut req = ehr_scope(&ehr_id);
    req.parameters = BTreeMap::from([("from".to_owned(), json!("not-a-date"))]);
    let err = svc
        .execute_ad_hoc_query(aql, req)
        .await
        .expect_err("an uncoercible binding is refused");
    assert_eq!(
        err.status,
        CallStatusType::PreconditionViolation,
        "the caller's defect classifies 400, got {err:?}"
    );
    assert!(
        err.message.contains("cannot be coerced"),
        "the message names the defect class without internals: {}",
        err.message
    );
    assert!(
        !err.message.contains("not-a-date") && !err.message.contains("timestamp"),
        "no driver internals echo to the wire: {}",
        err.message
    );
}

// ── folder containment (#2880) ───────────────────────────────────────────────

/// An `items` entry referencing a this-system versioned composition.
fn folder_ref(comp_root: &str) -> Value {
    json!({
        "_type": "OBJECT_REF",
        "namespace": "local",
        "type": "VERSIONED_COMPOSITION",
        "id": { "_type": "HIER_OBJECT_ID", "value": comp_root }
    })
}

/// The uid root of an `OBJECT_VERSION_ID` string.
fn uid_root(ovid: &str) -> String {
    ovid.split("::").next().expect("uid root").to_owned()
}

/// `FOLDER CONTAINS COMPOSITION` resolves the `FOLDER.items` `OBJECT_REF`s,
/// transitively over the folder subtree, and only those (RM common master05
/// — folders reference versioned objects; QUERY master03 §Containment —
/// CONTAINS is the parent/child relationship, not co-residence in one EHR).
#[tokio::test]
async fn folder_containment_resolves_references() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<Uuid>().expect("ehr uuid"));

    // cB is multiply-classified (course-1 AND course-2 — RM common master05
    // names multiple classification as the design intent); cD is committed
    // but referenced by NO folder.
    let ca = uid_root(&create_comp(&svc, &ehr_id, "cA", 1.0).await);
    let cb = uid_root(&create_comp(&svc, &ehr_id, "cB", 2.0).await);
    let cc = uid_root(&create_comp(&svc, &ehr_id, "cC", 3.0).await);
    let cd = uid_root(&create_comp(&svc, &ehr_id, "cD", 4.0).await);

    let sub = |name: &str, items: Vec<Value>| {
        json!({
            "_type": "FOLDER",
            "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
            "name": { "_type": "DV_TEXT", "value": name },
            "items": items
        })
    };
    let directory = json!({
        "_type": "FOLDER",
        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
        "name": { "_type": "DV_TEXT", "value": "episodes" },
        "items": [folder_ref(&ca)],
        "folders": [
            sub("course-1", vec![folder_ref(&cb)]),
            sub("course-2", vec![folder_ref(&cb), folder_ref(&cc)]),
        ]
    });
    svc.create_directory(ehr_uuid, uv(&directory, "249", None))
        .await
        .expect("create_directory");

    // The full pair set, transitive: episodes reaches every subtree ref.
    let r = run_aql(
        &svc,
        "SELECT f/name/value, c/uid/value \
         FROM EHR e CONTAINS FOLDER f CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    let pairs: std::collections::BTreeSet<(String, String)> = rows(&r)
        .iter()
        .map(|row| {
            (
                row[0].as_str().expect("folder name").to_owned(),
                uid_root(row[1].as_str().expect("composition uid")),
            )
        })
        .collect();
    let expected: std::collections::BTreeSet<(String, String)> = [
        ("episodes", &ca),
        ("episodes", &cb),
        ("episodes", &cc),
        ("course-1", &cb),
        ("course-2", &cb),
        ("course-2", &cc),
    ]
    .into_iter()
    .map(|(f, c)| (f.to_owned(), c.clone()))
    .collect();
    assert_eq!(
        pairs, expected,
        "exactly the reference pairs, transitively — never the EHR cartesian"
    );
    assert_eq!(
        rows(&r).len(),
        6,
        "one row per (folder, referenced object) pair"
    );
    assert!(
        !pairs.iter().any(|(_, c)| *c == cd),
        "the unreferenced composition appears under no folder"
    );

    // Scoped to one sub-folder.
    let r = run_aql(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS FOLDER f CONTAINS COMPOSITION c \
         WHERE f/name/value = 'course-2'",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&r).len(), 2, "course-2 references exactly cB and cC");

    // FOLDER CONTAINS FOLDER: strict ancestor/descendant pairs only.
    let r = run_aql(
        &svc,
        "SELECT f1/name/value, f2/name/value FROM EHR e CONTAINS FOLDER f1 CONTAINS FOLDER f2",
        ehr_scope(&ehr_id),
    )
    .await;
    let folder_pairs: std::collections::BTreeSet<(String, String)> = rows(&r)
        .iter()
        .map(|row| {
            (
                row[0].as_str().expect("parent").to_owned(),
                row[1].as_str().expect("child").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        folder_pairs,
        [("episodes", "course-1"), ("episodes", "course-2")]
            .into_iter()
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .collect(),
        "no self-pairs, no inverted pairs"
    );

    // EHR CONTAINS FOLDER matches every folder node — the same answer with
    // and without ORDER BY (the pre-fix streaming shape returned only the
    // root when unordered).
    let unordered = run_aql(
        &svc,
        "SELECT f/name/value FROM EHR e CONTAINS FOLDER f",
        ehr_scope(&ehr_id),
    )
    .await;
    let ordered = run_aql(
        &svc,
        "SELECT f/name/value FROM EHR e CONTAINS FOLDER f ORDER BY f/name/value",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(rows(&unordered).len(), 3, "root + two sub-folders");
    assert_eq!(
        rows(&ordered).len(),
        rows(&unordered).len(),
        "ORDER BY never changes the row count"
    );

    // NOT CONTAINS negates the same reference edge: every folder here has a
    // reference in its subtree, so the anti-join is empty.
    let r = run_aql(
        &svc,
        "SELECT f/name/value FROM EHR e CONTAINS FOLDER f NOT CONTAINS COMPOSITION c",
        ehr_scope(&ehr_id),
    )
    .await;
    assert_eq!(
        rows(&r).len(),
        0,
        "no folder is without a referenced composition in its subtree"
    );
}

/// `FOLDER CONTAINS FOLDER` is the union edge (#2887): the by-value
/// `folders` nesting AND the folder rows of an items-referenced
/// `VERSIONED_FOLDER` (RM common master05 — `items` references name
/// versioned objects "logically in this folder", with no target-type
/// restriction). One reference hop; the referenced tree's own refs are
/// reached by explicit chaining, as the second query pins.
#[tokio::test]
async fn folder_contains_folder_reaches_items_referenced_versioned_folders() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<Uuid>().expect("ehr uuid"));

    // cX is referenced from the SECOND versioned folder, so it is reachable
    // from the directory only across the folder-reference hop.
    let cx = uid_root(&create_comp(&svc, &ehr_id, "cX", 1.0).await);

    // The directory goes in FIRST (a creation-FOLDER contribution takes the
    // empty directory slot, so the additional hierarchy must come second),
    // and its reference is added by update AFTER the target exists (a
    // dangling local items ref is refused at commit).
    let folder = |name: &str, items: Vec<Value>, folders: Vec<Value>| {
        let mut f = json!({
            "_type": "FOLDER",
            "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
            "name": { "_type": "DV_TEXT", "value": name }
        });
        if !items.is_empty() {
            f["items"] = Value::Array(items);
        }
        if !folders.is_empty() {
            f["folders"] = Value::Array(folders);
        }
        f
    };
    let directory_v1 = folder("episodes", vec![], vec![folder("course-1", vec![], vec![])]);
    let meta = svc
        .create_directory(ehr_uuid, uv(&directory_v1, "249", None))
        .await
        .expect("create_directory");

    // The second VERSIONED_FOLDER (an additional EHR.folders hierarchy — RM
    // ehr master04 §Folders), committed through a CONTRIBUTION: a root with
    // one by-value sub-folder and an items ref to cX.
    let coded = |code: &str, value: &str| {
        json!({
            "_type": "DV_CODED_TEXT",
            "value": value,
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": code
            }
        })
    };
    let shared_plan = folder(
        "shared-plan",
        vec![folder_ref(&cx)],
        vec![folder("plan-week-1", vec![], vec![])],
    );
    let contribution = json!({
        "versions": [{
            "commit_audit": {
                "change_type": coded("249", "creation"),
                "committer": { "_type": "PARTY_IDENTIFIED", "name": "folder author" }
            },
            "lifecycle_state": coded("532", "complete"),
            "data": shared_plan
        }],
        "audit": {
            "change_type": coded("249", "creation"),
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "folder author" }
        }
    });
    let created = svc
        .create_ehr_contribution(ehr_uuid, contribution)
        .await
        .expect("commit the second versioned folder");
    let vf_ovid = created.body["versions"][0]["id"]["value"]
        .as_str()
        .expect("folder version id")
        .to_owned();
    let vf_root = uid_root(&vf_ovid);

    // course-1's items now reference the second VERSIONED_FOLDER.
    let vf_ref = json!({
        "_type": "OBJECT_REF",
        "namespace": "local",
        "type": "VERSIONED_FOLDER",
        "id": { "_type": "HIER_OBJECT_ID", "value": vf_root }
    });
    let directory_v2 = folder(
        "episodes",
        vec![],
        vec![folder("course-1", vec![vf_ref], vec![])],
    );
    svc.update_directory(ehr_uuid, uv(&directory_v2, "251", Some(&meta.uid)))
        .await
        .expect("update_directory with the folder reference");

    // The full (parent, child) folder pair set: by-value nesting inside each
    // tree, plus the reference hop from any folder whose subtree items name
    // the second VERSIONED_FOLDER — which contributes the referenced tree's
    // root AND its by-value descendants.
    let r = run_aql(
        &svc,
        "SELECT f1/name/value, f2/name/value \
         FROM EHR e CONTAINS FOLDER f1 CONTAINS FOLDER f2",
        ehr_scope(&ehr_id),
    )
    .await;
    let pairs: std::collections::BTreeSet<(String, String)> = rows(&r)
        .iter()
        .map(|row| {
            (
                row[0].as_str().expect("parent name").to_owned(),
                row[1].as_str().expect("child name").to_owned(),
            )
        })
        .collect();
    let expected: std::collections::BTreeSet<(String, String)> = [
        ("episodes", "course-1"),
        ("episodes", "shared-plan"),
        ("episodes", "plan-week-1"),
        ("course-1", "shared-plan"),
        ("course-1", "plan-week-1"),
        ("shared-plan", "plan-week-1"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect();
    assert_eq!(
        pairs, expected,
        "the union edge, one reference hop, no dupes"
    );

    // The chained hop composes: the referenced folder's own items refs are
    // that folder's OWN containment edge.
    let r = run_aql(
        &svc,
        "SELECT f2/name/value, c/uid/value \
         FROM EHR e CONTAINS FOLDER f1 CONTAINS FOLDER f2 CONTAINS COMPOSITION c \
         WHERE f1/name/value = 'course-1'",
        ehr_scope(&ehr_id),
    )
    .await;
    let chained: Vec<(String, String)> = rows(&r)
        .iter()
        .map(|row| {
            (
                row[0].as_str().expect("folder name").to_owned(),
                uid_root(row[1].as_str().expect("composition uid")),
            )
        })
        .collect();
    assert_eq!(
        chained,
        vec![("shared-plan".to_owned(), cx.clone())],
        "cX is reached from course-1 only across the explicit second hop"
    );

    // The negated twin: NOT CONTAINS excludes over the SAME union edge, so
    // the only folder with no folder child — by value or by reference — is
    // the leaf.
    let r = run_aql(
        &svc,
        "SELECT f1/name/value \
         FROM EHR e CONTAINS FOLDER f1 NOT CONTAINS FOLDER f2",
        ehr_scope(&ehr_id),
    )
    .await;
    let childless: Vec<String> = rows(&r)
        .iter()
        .map(|row| row[0].as_str().expect("folder name").to_owned())
        .collect();
    assert_eq!(
        childless,
        vec!["plan-week-1".to_owned()],
        "every non-leaf folder has a union-edge child, so only the leaf survives the anti-join"
    );
}

/// `query.max_result_rows` is the largest page one execution serves: a query
/// nothing bounds takes it as its page, an explicit `fetch` or AQL `LIMIT` at
/// or below it is served as written, and a page above it is refused as a
/// precondition (`400`) rather than silently shortened (#3092). ITS-REST leaves
/// the `fetch` default and maximum to the implementation (query `Request.md`
/// §Common Headers and Query Parameters).
#[tokio::test]
async fn an_explicit_page_above_the_result_ceiling_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_query_config(&QueryConfig {
            max_result_rows: 2,
            ..QueryConfig::default()
        });
    let ehr_id = create_ehr(&svc).await;
    for magnitude in [80.0, 90.0, 100.0, 110.0] {
        create_comp(&svc, &ehr_id, "BP", magnitude).await;
    }
    let aql = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";
    let scoped = |fetch: Option<i64>| AqlQueryRequest {
        fetch,
        ..ehr_scope(&ehr_id)
    };
    let refused = |aql: &str, request: AqlQueryRequest| {
        let aql = aql.to_owned();
        let svc = &svc;
        async move {
            let err = svc
                .execute_ad_hoc_query(aql, request)
                .await
                .expect_err("a page above the ceiling is refused");
            assert!(
                matches!(
                    &err,
                    SmError {
                        status: CallStatusType::PreconditionViolation,
                        message,
                        ..
                    } if message.contains("exceeds the largest page") && message.contains("2 rows")
                ),
                "got {err:?}"
            );
        }
    };

    // Nothing bounds the query: the ceiling is the page.
    assert_eq!(rows(&run_aql(&svc, aql, scoped(None)).await).len(), 2);
    // `fetch` at the ceiling is served as written; above it is refused.
    assert_eq!(rows(&run_aql(&svc, aql, scoped(Some(2))).await).len(), 2);
    refused(aql, scoped(Some(3))).await;
    // AQL `LIMIT` alone: the same rule.
    assert_eq!(
        rows(&run_aql(&svc, &format!("{aql} LIMIT 2"), scoped(None)).await).len(),
        2
    );
    refused(&format!("{aql} LIMIT 3"), scoped(None)).await;
    // `LIMIT` + `fetch`: the effective page (the smaller) is what is checked.
    assert_eq!(
        rows(&run_aql(&svc, &format!("{aql} LIMIT 4"), scoped(Some(2))).await).len(),
        2
    );
    assert_eq!(
        rows(&run_aql(&svc, &format!("{aql} LIMIT 2"), scoped(Some(4))).await).len(),
        2
    );
    refused(&format!("{aql} LIMIT 4"), scoped(Some(3))).await;
    // `0` means unbounded: the same page is then served whole.
    let unbounded =
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
            .with_query_config(&QueryConfig {
                max_result_rows: 0,
                ..QueryConfig::default()
            });
    assert_eq!(
        rows(&run_aql(&unbounded, aql, scoped(Some(1_000))).await).len(),
        4
    );
}
