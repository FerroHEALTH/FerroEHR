// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL paths keep an index into `node_hot` under the lean node indexes
//! (#5969): `idx_node_hot_ehr_type` without the ELEMENT rows and
//! `idx_node_hot_arch_subsume` over only the rows with a parsed archetype id.
//!
//! Every shape below is explained over the storage bench's seed, single-EHR
//! and over the population, with sequential scans discouraged, and must read
//! `node_hot` through an index alone. A typed source's generic plan, which a
//! named statement shape runs from its sixth execution (#5981), must still
//! prove the partial predicate. No openEHR spec governs indexes: our own
//! design.

use std::fmt::Write as _;

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use serde_json::Value;
use sqlx::{PgPool, Row as _};

use crate::fixtures::{ckm_example, uv};
use crate::storage_plans::{
    analyze_as_autovacuum, aql_sql, explain_aql, explain_tx, on, render, scans,
};

/// The AQL shapes whose `node_hot` access is held, by path class.
const SHAPES: [(&str, &str); 12] = [
    (
        "class-only entry",
        "SELECT o/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o",
    ),
    (
        "archetyped entry",
        "SELECT o/uid/value FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2]",
    ),
    (
        "ELEMENT by node id under an archetyped entry",
        "SELECT el/value FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2] \
         CONTAINS ELEMENT el[at0004]",
    ),
    (
        "class-only ELEMENT",
        "SELECT el/value FROM EHR e CONTAINS COMPOSITION c CONTAINS ELEMENT el",
    ),
    (
        "path into an archetyped entry",
        "SELECT o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude \
         FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2]",
    ),
    (
        "class-only CLUSTER",
        "SELECT cl/name/value FROM EHR e CONTAINS COMPOSITION c CONTAINS CLUSTER cl",
    ),
    (
        "class-only entry directly under the EHR",
        "SELECT o/uid/value FROM EHR e CONTAINS OBSERVATION o",
    ),
    (
        "archetyped entry directly under the EHR",
        "SELECT o/uid/value FROM EHR e \
         CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2]",
    ),
    (
        "either of two entry classes",
        "SELECT o/uid/value, ev/uid/value FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS (OBSERVATION o OR EVALUATION ev)",
    ),
    (
        "a root the EHR does not contain",
        "SELECT e/ehr_id/value FROM EHR e \
         NOT CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.other.v1]",
    ),
    (
        "either of two roots",
        "SELECT c/uid/value FROM EHR e CONTAINS (COMPOSITION c OR EHR_STATUS s)",
    ),
    (
        "count of an archetyped entry",
        "SELECT count(o) FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2]",
    ),
];

/// The storage bench's seed: its two templates alternating over 20 EHRs,
/// analyzed as the maintenance task leaves it.
async fn seeded(svc: &FerroEhrService, pool: &PgPool) -> Vec<EhrId> {
    let vital_signs = ckm_example(svc, "vital-signs").await;
    let gp_data_set = ckm_example(svc, "gp-data-set").await;
    let mut ehrs = Vec::new();
    for _ in 0..20 {
        ehrs.push(crate::fixtures::create_ehr(svc).await);
    }
    for index in 0..200_usize {
        let body = if index.div_euclid(ehrs.len()).is_multiple_of(2) {
            &vital_signs
        } else {
            &gp_data_set
        };
        svc.create_composition(ehrs[index % ehrs.len()], uv(body, "249", None))
            .await
            .expect("seed a composition");
    }
    analyze_as_autovacuum(pool).await;
    ferroehr::db::parent_statistics::refresh_pools(
        &ferroehr::db::domain::DomainPools::from_shared(pool),
    )
    .await
    .expect("refresh the partitioned parents' statistics");
    ehrs
}

/// Every AQL shape reads `node_hot` through an index, single-EHR and over the
/// population, and the rows it answers are non-empty.
#[tokio::test]
async fn every_aql_path_class_reads_the_node_rows_by_an_index() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehrs = Box::pin(seeded(&svc, &pool)).await;

    let mut report = String::new();
    let mut sequential = false;
    for (name, aql) in SHAPES {
        for (scope, ehr_ids) in [("single-EHR", vec![ehrs[0]]), ("population", Vec::new())] {
            let plan = explain_aql(&pool, aql_sql(aql, ehr_ids)).await;
            let scans = scans(&plan);
            let nodes = on(&scans, "node_hot");
            let returned = plan
                .get(0)
                .and_then(|first| first.get("Plan"))
                .and_then(|root| root.get("Actual Rows"))
                .and_then(Value::as_f64)
                .unwrap_or_default();
            let seq = nodes.iter().any(|scan| scan.index.is_none());
            sequential |= seq || nodes.is_empty() || returned <= 0.0;
            writeln!(
                report,
                "{name}, {scope}: {returned} rows{}\n{}",
                if seq { " (SEQUENTIAL)" } else { "" },
                render(&scans)
            )
            .expect("a String takes the report");
        }
    }
    eprintln!("{report}");
    assert!(
        !sequential,
        "every shape reads node_hot by an index and answers rows:\n{report}"
    );
}

/// The generic plan of a typed source proves the partial predicate of
/// `idx_node_hot_ehr_type`, single-EHR and over the population: the RM type
/// is a literal of the statement, never a bound parameter.
#[tokio::test]
async fn a_typed_sources_generic_plan_enters_through_the_partial_ehr_index() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehrs = Box::pin(seeded(&svc, &pool)).await;

    // The population shape is free to enter elsewhere, by index.
    for (aql, ehr_ids, literal, through_ehr_type) in [
        (
            "SELECT c/uid/value FROM EHR e CONTAINS (COMPOSITION c OR EHR_STATUS s)",
            vec![ehrs[0]],
            "'EHR_STATUS'",
            true,
        ),
        (
            "SELECT cl/name/value FROM EHR e CONTAINS COMPOSITION c CONTAINS CLUSTER cl",
            Vec::new(),
            "'CLUSTER'",
            false,
        ),
    ] {
        let prepared = aql_sql(aql, ehr_ids);
        assert!(
            prepared
                .sql
                .contains(&format!("\"rm_type\" IN ({literal})")),
            "the RM type is a literal of the statement: {}",
            prepared.sql
        );
        let mut tx = explain_tx(&pool).await;
        let plan: Value = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "EXPLAIN (GENERIC_PLAN, FORMAT JSON) {}",
            prepared.sql
        )))
        .fetch_one(&mut *tx)
        .await
        .expect("explain the generic plan")
        .try_get(0)
        .expect("EXPLAIN FORMAT JSON returns one json column");
        tx.rollback().await.expect("roll the explain back");

        let scans = scans(&plan);
        let nodes = on(&scans, "node_hot");
        assert!(
            (!through_ehr_type
                || nodes
                    .iter()
                    .any(|scan| scan.index.as_deref() == Some("idx_node_hot_ehr_type")))
                && !nodes.is_empty()
                && nodes.iter().all(|scan| scan.index.is_some()),
            "the generic plan of {aql} reads node_hot by an index alone{}:\n{}",
            if through_ehr_type {
                ", entering through idx_node_hot_ehr_type"
            } else {
                ""
            },
            render(&scans)
        );
    }
}
