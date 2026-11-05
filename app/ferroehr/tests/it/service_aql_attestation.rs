// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL over `ORIGINAL_VERSION.attestations` and the `ATTESTATION` attributes
//! of a commit audit (#4989), planned and executed against a real PostgreSQL
//! 18 (shared testkit harness).
//!
//! RM common `master06-change_control_package.adoc` §Attestation: an
//! attestation arrives on the version at committal or is added after it, and
//! signing a pending one adds a second `ATTESTATION` with `is_pending = false`
//! (`master04-generic_package.adoc` §Attestation). `ATTESTATION` is no
//! `LOCATABLE`, so QUERY master03 §Identified Paths reaches it by path from
//! the `VERSION` variable. A comparison below `attestations` holding when any
//! attestation matches, and the `LAST(...)` function reading the most recent
//! one, are FerroEHR's own decisions: AQL 1.1.0 is silent (QUERY master03
//! §Functions admits implementation functions).

use serde_json::{Value, json};

use ferroehr::aql::error::{AnalysisError, AqlError, AqlFeatureError};
use ferroehr::aql::ir::{Params, PathTarget, SelectValue};
use ferroehr::aql::plan;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use openehr_query::parser::parse_str;

use crate::fixtures::{
    attestation_by, change_type, commit_audit, composition, first_version_uid, plan_ok, uv, vo_of,
};

/// The review worklist (#4989): the latest version of each composition whose
/// MOST RECENT attestation is pending. A pending attestation may follow a
/// completed one (RM common master04 §Attestation), so presence of a pending
/// one and absence of a completed one does not decide it.
const WORKLIST: &str = "SELECT c/uid/value \
     FROM EHR e CONTAINS VERSION v[LATEST_VERSION] CONTAINS COMPOSITION c \
     WHERE LAST(v/attestations/is_pending) = true";

/// The portable pair: some pending attestation and no completed one.
const PENDING_NEVER_SIGNED: &str = "SELECT c/uid/value \
     FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c \
     WHERE EXISTS v/attestations[is_pending=true] \
       AND NOT EXISTS v/attestations[is_pending=false]";

// ── planning ─────────────────────────────────────────────────────────────────

fn plan_err(q: &str) -> AqlError {
    let ast = parse_str(q).unwrap_or_else(|e| panic!("parse failed for {q:?}: {e}"));
    plan(
        &ast,
        &Params::default(),
        ferroehr::config::profile::SpecProfile::default(),
    )
    .expect_err(&format!("expected {q:?} to be refused"))
}

fn build_sql(q: &str) -> ferroehr::aql::sql::PreparedQuery {
    let ir = plan_ok(q);
    let ctx = ferroehr::aql::sql::SqlCtx {
        system_id: "sys.example.com".to_owned(),
        ehr_ids: Vec::new(),
        ehr_scope_kind: ferroehr::aql::sql::EhrScopeKind::Named,
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: None,
        offset: None,
        archetype_lineage: std::sync::Arc::new(ferroehr::aql::lineage::ArchetypeLineage::default()),
    };
    ferroehr::aql::sql::build(&ir, &Params::default(), &ctx)
        .unwrap_or_else(|e| panic!("SQL build failed for {q:?}: {e}"))
}

/// `v/attestations[...]/...` resolves against `ATTESTATION`: the predicate on
/// `attestations` is the leaf's root predicate and the residual path a
/// fragment with no structure hop.
#[test]
fn an_attestation_path_resolves_against_attestation() {
    let ir = plan_ok(
        "SELECT v/attestations[is_pending=true]/reason/value \
         FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
    );
    let Some(SelectValue::Path(PathTarget::Attestations { leaf, .. })) =
        ir.select.first().map(|c| &c.value)
    else {
        panic!("expected an attestation path, got {:?}", ir.select);
    };
    assert!(leaf.anchor.is_empty(), "ATTESTATION has no node row");
    let names: Vec<&str> = leaf.fragment.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["reason", "value"]);
    let pred = leaf
        .root_predicate
        .as_ref()
        .expect("the attestations predicate");
    assert_eq!(pred.standard.len(), 1);
    assert_eq!(pred.standard[0].path, ["is_pending"]);
}

/// The two ATTESTATION attributes of a commit audit resolve to version
/// fields, `is_pending` comparing as a Boolean.
#[test]
fn commit_audit_attestation_attributes_resolve() {
    for path in [
        "v/commit_audit/is_pending",
        "v/commit_audit/reason",
        "v/commit_audit/reason/value",
    ] {
        let ir = plan_ok(&format!(
            "SELECT {path} FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c"
        ));
        assert!(
            matches!(
                ir.select.first().map(|c| &c.value),
                Some(SelectValue::Path(PathTarget::Version { .. }))
            ),
            "{path} is a version field"
        );
    }
    plan_ok(
        "SELECT c/uid/value FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c \
         WHERE v/commit_audit/is_pending = true",
    );
}

/// `ATTESTATION` is no `LOCATABLE`: an archetype or name predicate names an
/// attribute it lacks, as does an attribute ATTESTATION does not declare.
#[test]
fn attributes_attestation_lacks_are_typed_refusals() {
    for q in [
        "SELECT v/attestations[at0001] FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        "SELECT v/attestations[name/value='x'] FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        "SELECT v/attestations[zz=1] FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        "SELECT v/attestations/zz FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
    ] {
        let error = plan_err(q);
        assert!(
            matches!(
                error,
                AqlError::Analysis(
                    AnalysisError::UnresolvableAttribute { .. }
                        | AnalysisError::AttributeNotInProfile { .. }
                )
            ),
            "{q}: expected an unresolvable-attribute refusal, got {error:?}"
        );
    }
    let error = plan_err(
        "SELECT v/commit_audit/attested_view FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
    );
    assert!(
        matches!(
            error,
            AqlError::Feature(AqlFeatureError::UnsupportedVersionPredicate(_))
        ),
        "got {error:?}"
    );
}

/// The attestation rows are read under the hot-tier literal, keyed on the
/// version row, with every compared value bound and none in the SQL text.
#[test]
fn the_attestation_read_is_hot_correlated_and_bound() {
    for query in [WORKLIST, PENDING_NEVER_SIGNED] {
        let sql = build_sql(query).sql;
        assert!(sql.contains(r#""vo_attestation""#), "{sql}");
        assert_eq!(
            sql.matches(r#""tier" = "#).count(),
            sql.matches(r#""tier" = ('hot')"#).count(),
            "every tier predicate is the literal 'hot': {sql}"
        );
        assert!(
            sql.contains(r#""x0"."sys_version" = "v2"."sys_version""#),
            "the read is keyed on the version row: {sql}"
        );
    }
    let pair = build_sql(PENDING_NEVER_SIGNED).sql;
    assert!(pair.contains("jsonb_path_exists"), "{pair}");
    assert!(
        !pair.contains("is_pending == true"),
        "values are bound: {pair}"
    );
    // `LAST` reads the most recent attestation: the reverse of the list order.
    let latest = build_sql(WORKLIST).sql;
    assert!(
        latest.contains(
            r#"ORDER BY "x0"."at_committal" ASC, "x0"."time_committed" DESC, "x0"."id" DESC LIMIT"#
        ),
        "{latest}"
    );
}

// ── execution ────────────────────────────────────────────────────────────────

async fn run(svc: &FerroEhrService, aql: &str, ehr: EhrId) -> Vec<Value> {
    let request = AqlQueryRequest {
        ehr_ids: vec![ehr],
        ..AqlQueryRequest::default()
    };
    svc.execute_aql(aql, None, &request)
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"))
        .result_set["rows"]
        .as_array()
        .expect("rows array")
        .clone()
}

/// The first cell of every row, as a sorted list of strings.
fn first_cells(rows: &[Value]) -> Vec<String> {
    let mut cells: Vec<String> = rows
        .iter()
        .map(|r| r[0].as_str().expect("a string cell").to_owned())
        .collect();
    cells.sort();
    cells
}

/// Commit a COMPOSITION through a CONTRIBUTION whose member carries
/// `attestations` at committal (`UPDATE_VERSION.attestations`).
async fn commit_with(svc: &FerroEhrService, ehr: EhrId, name: &str, attestations: Value) -> String {
    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [{
            "_type": "ORIGINAL_VERSION",
            "commit_audit": commit_audit("249", "creation", "transcriptionist"),
            "lifecycle_state": change_type("532", "complete"),
            "data": composition(name),
            "attestations": attestations
        }],
        "audit": commit_audit("249", "creation", "transcriptionist")
    });
    let created = svc
        .create_ehr_contribution(ehr, contribution)
        .await
        .expect("a contribution with attestations commits");
    first_version_uid(&created.body)
}

/// Attest `ovid` after committal: a `666|attestation|` CONTRIBUTION that adds
/// an ATTESTATION to the existing version, completed or pending.
async fn attest(svc: &FerroEhrService, ehr: EhrId, ovid: &str, reason: &str, is_pending: bool) {
    let contribution = json!({
        "versions": [{
            "preceding_version_uid": { "value": ovid },
            "lifecycle_state": change_type("532", "complete"),
            "commit_audit": commit_audit("666", "attestation", "senior clinician"),
            "attestations": [attestation_by("senior clinician", reason, is_pending)]
        }],
        "audit": commit_audit("666", "attestation", "senior clinician")
    });
    svc.create_ehr_contribution(ehr, contribution)
        .await
        .expect("a 666 contribution attests the version");
}

/// Pending at committal, then signed after it: the worklist and the portable
/// pair return the version while it is pending and not once it is signed, and
/// a bare comparison below `attestations` is any-element and stays true.
#[tokio::test]
async fn the_worklist_returns_a_pending_version_until_it_is_signed() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let pending = commit_with(
        &svc,
        ehr,
        "pending result",
        json!([attestation_by("transcriptionist", "authorisation", true)]),
    )
    .await;
    let plain = svc
        .create_composition(ehr, uv(&composition("plain"), "249", None))
        .await
        .expect("a plain composition")
        .version_uid();

    for query in [WORKLIST, PENDING_NEVER_SIGNED] {
        assert_eq!(
            first_cells(&run(&svc, query, ehr).await),
            std::slice::from_ref(&pending),
            "{query}"
        );
    }
    let any_pending = "SELECT c/uid/value FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c \
         WHERE v/attestations/is_pending = true";
    assert_eq!(
        first_cells(&run(&svc, any_pending, ehr).await),
        std::slice::from_ref(&pending)
    );

    attest(&svc, ehr, &pending, "signed", false).await;

    for query in [WORKLIST, PENDING_NEVER_SIGNED] {
        assert!(
            run(&svc, query, ehr).await.is_empty(),
            "signed, so off the worklist: {query}"
        );
    }
    // The pending attestation stays in place beside the completed one.
    assert_eq!(
        first_cells(&run(&svc, any_pending, ehr).await),
        std::slice::from_ref(&pending)
    );
    let any_done = "SELECT c/uid/value FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c \
         WHERE v/attestations/is_pending = false";
    assert_eq!(
        first_cells(&run(&svc, any_done, ehr).await),
        std::slice::from_ref(&pending)
    );
    // A version with no attestation is unknown under the comparison and under
    // its negation alike.
    let negated = "SELECT c/uid/value FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c \
         WHERE NOT (v/attestations/is_pending = true)";
    assert!(
        !first_cells(&run(&svc, negated, ehr).await).contains(&plain),
        "no attestation is unknown, not false"
    );

    // SELECT v/attestations is the list, at committal first.
    let listed = run(
        &svc,
        "SELECT c/uid/value, v/attestations, v/attestations/is_pending, \
                v/attestations/reason/value \
         FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c ORDER BY c/name/value",
        ehr,
    )
    .await;
    assert_eq!(listed.len(), 2);
    let row = |ovid: &str| {
        listed
            .iter()
            .find(|r| r[0] == json!(ovid))
            .expect("the query returns a row for each composition")
    };
    let attested = row(&pending);
    let list = attested[1].as_array().expect("the attestations list");
    assert_eq!(list.len(), 2);
    assert!(list.iter().all(|a| a["_type"] == "ATTESTATION"));
    assert_eq!(attested[2], json!([true, false]));
    assert_eq!(attested[3], json!(["authorisation", "signed"]));
    let unattested = row(&plain);
    assert_eq!(
        unattested[1],
        Value::Null,
        "no attestation is an empty cell"
    );
    assert_eq!(unattested[2], Value::Null);

    // A predicate on `attestations` binds its value, a parameter included.
    let by_reason = run_with(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c \
         WHERE EXISTS v/attestations[reason/value=$reason]",
        ehr,
        json!({ "reason": "signed" }),
    )
    .await;
    assert_eq!(first_cells(&by_reason), std::slice::from_ref(&pending));
    let counted = run(
        &svc,
        "SELECT COUNT(v/attestations) FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        ehr,
    )
    .await;
    assert_eq!(counted[0][0], json!(2));
}

async fn run_with(svc: &FerroEhrService, aql: &str, ehr: EhrId, params: Value) -> Vec<Value> {
    let request = AqlQueryRequest {
        ehr_ids: vec![ehr],
        parameters: serde_json::from_value(params).expect("query parameters"),
        ..AqlQueryRequest::default()
    };
    svc.execute_aql(aql, None, &request)
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"))
        .result_set["rows"]
        .as_array()
        .expect("rows array")
        .clone()
}

/// `LATEST_VERSION` reads the attestations of each object's current version,
/// so an older version still pending appears only under `ALL_VERSIONS`
/// (master06 §Attestation: "When subsequent Versions are added, the existing
/// Attestations can not be assumed to be valid for the new Version").
#[tokio::test]
async fn latest_and_all_versions_read_their_own_versions_attestations() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let v1 = commit_with(
        &svc,
        ehr,
        "result",
        json!([attestation_by("transcriptionist", "authorisation", true)]),
    )
    .await;
    let vo: VoId = vo_of(&v1).parse().expect("vo id");
    svc.update_composition(ehr, vo, uv(&composition("result"), "251", Some(&v1)))
        .await
        .expect("a second version");

    let latest = "SELECT v/uid/value FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c \
         WHERE EXISTS v/attestations[is_pending=true]";
    assert!(
        run(&svc, latest, ehr).await.is_empty(),
        "the second version carries no attestation"
    );
    let all = "SELECT v/uid/value FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c \
         WHERE EXISTS v/attestations[is_pending=true] \
           AND NOT EXISTS v/attestations[is_pending=false]";
    assert_eq!(
        first_cells(&run(&svc, all, ehr).await),
        std::slice::from_ref(&v1)
    );
    let every = "SELECT v/uid/value, v/attestations/is_pending \
         FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c";
    let rows = run(&svc, every, ehr).await;
    assert_eq!(rows.len(), 2);
    for row in rows {
        let expected = if row[0] == json!(v1) {
            json!([true])
        } else {
            Value::Null
        };
        assert_eq!(
            row[1], expected,
            "the first version carries its pending attestation, the second none"
        );
    }
}

/// `v/commit_audit/is_pending` and `v/commit_audit/reason` read the
/// `ATTESTATION` attributes of a commit audit that is one (master06
/// §Attestation, "Marking content for review and signing"), and are empty on
/// a plain `AUDIT_DETAILS`. The commit paths refuse an ATTESTATION commit
/// audit (ITS-REST `UpdateAudit.yaml`), so the row is stored as a dump load
/// or replay stores one.
#[tokio::test]
async fn commit_audit_attestation_attributes_read_the_stored_audit() {
    let (db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let marked = svc
        .create_composition(ehr, uv(&composition("marked"), "249", None))
        .await
        .expect("a composition")
        .version_uid();
    let plain = svc
        .create_composition(ehr, uv(&composition("plain"), "249", None))
        .await
        .expect("a composition")
        .version_uid();
    let vo: uuid::Uuid = vo_of(&marked).parse().expect("vo id");
    sqlx::query(
        "UPDATE commit_audit SET attestation = $1 \
         WHERE id = (SELECT commit_audit_id FROM version WHERE vo_id = $2)",
    )
    .bind(json!({
        "reason": { "_type": "DV_TEXT", "value": "review required" },
        "is_pending": true
    }))
    .bind(vo)
    .execute(&db.pool())
    .await
    .expect("mark the commit audit as an ATTESTATION");

    let rows = run(
        &svc,
        "SELECT c/uid/value, v/commit_audit/is_pending, v/commit_audit/reason/value \
         FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        ehr,
    )
    .await;
    for row in &rows {
        if row[0] == json!(marked) {
            assert_eq!(row[1], json!(true));
            assert_eq!(row[2], json!("review required"));
        } else {
            assert_eq!(row[0], json!(plain));
            assert_eq!(row[1], Value::Null);
            assert_eq!(row[2], Value::Null);
        }
    }
    let pending = run(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c \
         WHERE v/commit_audit/is_pending = true",
        ehr,
    )
    .await;
    assert_eq!(first_cells(&pending), std::slice::from_ref(&marked));
    let scoped = run(
        &svc,
        "SELECT c/uid/value FROM EHR e \
         CONTAINS VERSION v[commit_audit/is_pending=true] CONTAINS COMPOSITION c",
        ehr,
    )
    .await;
    assert_eq!(first_cells(&scoped), [marked]);
}

/// Sam Heard's case on #4989: a version signed and later marked pending again
/// because further action is needed is back on the worklist, since only the
/// latest attestation confers the responsibility. The portable pair misses it.
#[tokio::test]
async fn a_pending_attestation_after_signing_puts_the_version_back_on_the_worklist() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let signed = commit_with(
        &svc,
        ehr,
        "signed result",
        json!([attestation_by("clinician a", "signed", false)]),
    )
    .await;
    assert!(
        run(&svc, WORKLIST, ehr).await.is_empty(),
        "signed at committal"
    );

    attest(&svc, ehr, &signed, "further action required", true).await;

    assert_eq!(
        first_cells(&run(&svc, WORKLIST, ehr).await),
        std::slice::from_ref(&signed)
    );
    assert!(
        run(&svc, PENDING_NEVER_SIGNED, ehr).await.is_empty(),
        "the pair cannot see a pending attestation after a completed one"
    );
    let latest = run(
        &svc,
        "SELECT LAST(v/attestations), LAST(v/attestations/reason/value) \
         FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        ehr,
    )
    .await;
    assert_eq!(latest[0][0]["_type"], "ATTESTATION");
    assert_eq!(latest[0][0]["is_pending"], json!(true));
    assert_eq!(latest[0][1], json!("further action required"));

    attest(&svc, ehr, &signed, "signed", false).await;
    assert!(run(&svc, WORKLIST, ehr).await.is_empty(), "signed again");
}

/// The worklist counts the latest version of each composition only: a
/// pending first version superseded by a second raises no responsibility,
/// and `ALL_VERSIONS` still finds it.
#[tokio::test]
async fn the_worklist_reads_the_latest_version_only() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let v1 = commit_with(
        &svc,
        ehr,
        "result",
        json!([attestation_by("transcriptionist", "authorisation", true)]),
    )
    .await;
    assert_eq!(
        first_cells(&run(&svc, WORKLIST, ehr).await),
        std::slice::from_ref(&v1)
    );
    let vo: VoId = vo_of(&v1).parse().expect("vo id");
    svc.update_composition(ehr, vo, uv(&composition("result"), "251", Some(&v1)))
        .await
        .expect("a second version");

    assert!(
        run(&svc, WORKLIST, ehr).await.is_empty(),
        "the new version carries none"
    );
    let history = "SELECT v/uid/value \
         FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c \
         WHERE LAST(v/attestations/is_pending) = true";
    assert_eq!(
        first_cells(&run(&svc, history, ehr).await),
        std::slice::from_ref(&v1)
    );
}

/// `LAST` takes exactly one path below `attestations`; anything else is a
/// typed refusal rather than an answer.
#[test]
fn last_over_anything_but_one_attestation_path_is_refused() {
    plan_ok(WORKLIST);
    for q in [
        "SELECT LAST(c/name/value) FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        "SELECT LAST(v/commit_audit/is_pending) FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        "SELECT LAST() FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        "SELECT LAST(v/attestations, v/attestations) FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
        "SELECT LAST('x') FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c",
    ] {
        let error = plan_err(q);
        assert!(
            matches!(error, AqlError::Feature(AqlFeatureError::LastArgument(_))),
            "{q}: got {error:?}"
        );
    }
}
