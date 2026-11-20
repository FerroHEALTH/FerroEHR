// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A statement bound from a shape is the statement the request's own build
//! renders, byte for byte and value for value.

use std::sync::Arc;

use openehr_rm::draft::access_control::terminology::Refusal;
use uuid::Uuid;

use super::Shapes;
use crate::access_eval::condition::care::{Care, ProjectedCare};
use crate::access_eval::condition::pinned::PinnedEhr;
use crate::access_eval::condition::{AccessCondition, CallerKeys, ConditionGroups, VerdictPair};
use crate::access_eval::procedure::{Outcome, Rule, Verdict};
use crate::access_eval::scope::{DataMatch, EhrFact, Polarity, Pred};
use crate::aql::ir::{ParamValue, Params, QueryIr};
use crate::aql::lineage::ArchetypeLineage;
use crate::aql::sql::{EhrAccessReaders, EhrScopeKind, PolicyParties, PreparedQuery, SqlCtx};
use crate::ids::EhrId;
use crate::storage::access_projection::{EhrSubject, Revision};

/// A single-EHR query whose EHR is a parameter.
const PINNED: &str = "SELECT c/uid/value, c/name/value FROM EHR e CONTAINS COMPOSITION c \
                      WHERE e/ehr_id/value = $ehr ORDER BY c/name/value DESC";
/// A population query.
const POPULATION: &str = "SELECT e/ehr_id/value, c/uid/value FROM EHR e CONTAINS COMPOSITION c";

fn plan(aql: &str) -> QueryIr {
    let ast = openehr_query::parser::parse_str(aql).expect("the query parses");
    crate::aql::lower_query(&ast, crate::config::profile::SpecProfile::default())
        .expect("the query lowers")
}

fn ctx(ehr_access: AccessCondition, lineage: &Arc<ArchetypeLineage>) -> SqlCtx {
    SqlCtx {
        system_id: "shape.example.org".to_owned(),
        ehr_ids: Vec::new(),
        ehr_scope_kind: EhrScopeKind::Named,
        ehr_access,
        limit: None,
        offset: None,
        archetype_lineage: Arc::clone(lineage),
        policy_parties: PolicyParties::Served,
        access_settings: EhrAccessReaders::Every,
        own_records: None,
    }
}

fn ehr_param(ehr: Uuid) -> Params {
    Params::default().with("ehr", ParamValue::Str(ehr.to_string()))
}

/// The objects of one template, the data scope the verdicts read.
fn lab_results() -> Pred {
    Pred::Data(
        Arc::new(DataMatch {
            templates: Some(vec!["lab results".to_owned()]),
            ..DataMatch::default()
        }),
        Polarity::Deny,
    )
}

/// The verdict of an EHR pinned to `ehr`, judged on revision `ordinal`.
fn pinned(ehr: Uuid, ordinal: i32, admitted: Pred) -> AccessCondition {
    AccessCondition::of_parts(
        None,
        Some(Arc::new(PinnedEhr {
            ehr,
            admitted,
            settled: true,
            revision: Revision {
                settings: Some(ordinal),
                subject: Some(EhrSubject::new("local", ehr.to_string())),
            },
        })),
    )
}

/// A verdict that refuses `when` and admits the rest.
fn refusing(when: Pred) -> Verdict {
    Verdict {
        rules: vec![Rule {
            when,
            then: Outcome::Refused(Refusal::PurposeMissing),
            by: Vec::new(),
        }],
        ..Verdict::admit()
    }
}

/// A class condition that reads the caller's moment and parties, with
/// `cared` as its care and `confined` as the EHRs a subject caller reaches.
fn classes(
    now: jiff::Timestamp,
    parties: &[&str],
    cared: Care,
    confined: Option<Vec<Uuid>>,
) -> AccessCondition {
    let facts = Pred::Any(vec![
        Pred::Ehr(EhrFact::Party(2)),
        Pred::Ehr(EhrFact::Within {
            from: Some((0, true)),
            until: Some((1, false)),
        }),
    ]);
    let pair = VerdictPair {
        cared: Verdict::admit(),
        uncared: refusing(Pred::All(vec![lab_results(), facts])),
    };
    AccessCondition::from(ConditionGroups {
        groups: vec![(vec![1, 2], Arc::new(pair))],
        closed: Arc::new(VerdictPair {
            cared: refusing(Pred::True),
            uncared: refusing(Pred::True),
        }),
        default: Some(Arc::new(VerdictPair {
            cared: Verdict::admit(),
            uncared: refusing(lab_results()),
        })),
        cared,
        subject_namespace: "local".into(),
        facts: CallerKeys {
            now,
            parties: parties
                .iter()
                .map(|p| (*p).to_owned())
                .collect::<Vec<_>>()
                .into(),
            keeper: None,
        },
        confined: confined.map(Into::into),
    })
}

fn projected(teams: Vec<Uuid>, day: jiff::civil::Date) -> Care {
    Care::Projected(ProjectedCare {
        teams: teams.into(),
        day,
        grace: "P14D".to_owned(),
    })
}

/// Prepares the request through `shapes` and holds it to the request's own
/// build.
fn prepared(shapes: &Shapes, ir: &QueryIr, params: &Params, ctx: &SqlCtx) -> PreparedQuery {
    let bound = shapes
        .prepare(ir, params, ctx)
        .expect("the request prepares");
    let built = crate::aql::sql::build_executed(ir, params, ctx).expect("the request builds");
    assert_eq!(bound.sql, built.sql, "the text of the request's own build");
    assert_eq!(
        bound.values.0.0, built.values.0.0,
        "the values of the request's own build"
    );
    bound
}

#[test]
fn a_pinned_query_binds_every_ehr_from_one_shape() {
    let ir = plan(PINNED);
    let shapes = Shapes::new(&ir);
    assert_eq!(shapes.live_params, vec!["ehr".to_owned()]);
    let lineage = Arc::new(ArchetypeLineage::default());
    for (round, ehr) in (0..6).map(|_| Uuid::now_v7()).enumerate() {
        let ordinal = i32::try_from(round).expect("a small round");
        let ctx = ctx(pinned(ehr, ordinal, lab_results()), &lineage);
        prepared(&shapes, &ir, &ehr_param(ehr), &ctx);
    }
    assert_eq!(
        shapes.bound(),
        4,
        "the third request on binds from the shape"
    );
}

#[test]
fn an_id_in_any_case_binds_its_canonical_form() {
    let ir = plan(PINNED);
    let shapes = Shapes::new(&ir);
    let lineage = Arc::new(ArchetypeLineage::default());
    for ehr in (0..4).map(|_| Uuid::now_v7()) {
        let params = Params::default().with("ehr", ParamValue::Str(ehr.to_string().to_uppercase()));
        prepared(
            &shapes,
            &ir,
            &params,
            &ctx(pinned(ehr, 1, Pred::True), &lineage),
        );
    }
    assert_eq!(shapes.bound(), 2);
}

#[test]
fn another_verdict_is_another_shape() {
    let ir = plan(PINNED);
    let shapes = Shapes::new(&ir);
    let lineage = Arc::new(ArchetypeLineage::default());
    for admitted in [lab_results(), Pred::True, lab_results(), Pred::True] {
        let ehr = Uuid::now_v7();
        prepared(
            &shapes,
            &ir,
            &ehr_param(ehr),
            &ctx(pinned(ehr, 1, admitted), &lineage),
        );
    }
    assert_eq!(
        shapes.bound(),
        0,
        "each verdict is seen twice and derived once"
    );
    let ehr = Uuid::now_v7();
    prepared(
        &shapes,
        &ir,
        &ehr_param(ehr),
        &ctx(pinned(ehr, 1, Pred::True), &lineage),
    );
    assert_eq!(shapes.bound(), 1);
}

#[test]
fn a_value_that_is_no_id_keys_by_value() {
    let ir = plan(PINNED);
    let shapes = Shapes::new(&ir);
    let lineage = Arc::new(ArchetypeLineage::default());
    let params = Params::default().with("ehr", ParamValue::Str("no id".to_owned()));
    let ctx = ctx(AccessCondition::default(), &lineage);
    for _ in 0..3 {
        prepared(&shapes, &ir, &params, &ctx);
    }
    assert_eq!(shapes.bound(), 1);
    let other = Params::default().with("ehr", ParamValue::Str("no id either".to_owned()));
    prepared(&shapes, &ir, &other, &ctx);
    assert_eq!(shapes.bound(), 1, "another value is another key");
}

#[test]
fn a_parameter_read_as_a_name_keys_by_value() {
    let ir = plan(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE e/ehr_id/value = $ehr AND c/name/value = $ehr",
    );
    let shapes = Shapes::new(&ir);
    assert!(
        shapes.live_params.is_empty(),
        "the parameter is no EHR id alone"
    );
    let lineage = Arc::new(ArchetypeLineage::default());
    for ehr in (0..4).map(|_| Uuid::now_v7()) {
        prepared(
            &shapes,
            &ir,
            &ehr_param(ehr),
            &ctx(AccessCondition::default(), &lineage),
        );
    }
    assert_eq!(shapes.bound(), 0, "every value is a key of its own");
}

#[test]
fn a_class_condition_binds_the_caller_values() {
    let ir = plan(POPULATION);
    let shapes = Shapes::new(&ir);
    let lineage = Arc::new(ArchetypeLineage::default());
    let days = [
        "2026-11-14",
        "2026-11-15",
        "2026-11-16",
        "2026-11-17",
        "2026-11-18",
    ];
    for (round, day) in days.iter().enumerate() {
        let day: jiff::civil::Date = day.parse().expect("a day");
        let now = jiff::Timestamp::now();
        let teams = (0..=round).map(|_| Uuid::now_v7()).collect();
        let parties = ["party-a", "party-b", "party-c"];
        let parties = parties.get(..=round.min(2)).expect("a prefix");
        let mut ctx = ctx(classes(now, parties, projected(teams, day), None), &lineage);
        ctx.limit = Some(20 + i64::try_from(round).expect("a small round"));
        ctx.offset = Some(10 * i64::try_from(round + 1).expect("a small round"));
        prepared(&shapes, &ir, &Params::default(), &ctx);
    }
    assert_eq!(shapes.bound(), 3);
}

#[test]
fn a_census_confinement_and_scope_bind_from_slots() {
    let ir = plan(POPULATION);
    let shapes = Shapes::new(&ir);
    let lineage = Arc::new(ArchetypeLineage::default());
    for round in 1..=4 {
        let subjects: Vec<EhrSubject> = (0..round)
            .map(|_| EhrSubject::new("local", Uuid::now_v7().to_string()))
            .collect();
        let confined = (0..round).map(|_| Uuid::now_v7()).collect();
        let condition = classes(
            jiff::Timestamp::now(),
            &["party"],
            Care::Census(subjects.into()),
            Some(confined),
        );
        let mut ctx = ctx(condition, &lineage);
        ctx.ehr_ids = (0..round).map(|_| EhrId(Uuid::now_v7())).collect();
        ctx.ehr_scope_kind = EhrScopeKind::PopulationSubset;
        prepared(&shapes, &ir, &Params::default(), &ctx);
    }
    assert_eq!(shapes.bound(), 2);
}

#[test]
fn a_pinned_care_read_binds_from_a_slot() {
    let ir = plan(POPULATION);
    let shapes = Shapes::new(&ir);
    let lineage = Arc::new(ArchetypeLineage::default());
    for round in 0..4 {
        let ehrs: Vec<Uuid> = (0..round).map(|_| Uuid::now_v7()).collect();
        let condition = classes(
            jiff::Timestamp::now(),
            &["party"],
            Care::Ehrs(ehrs.into()),
            None,
        );
        prepared(&shapes, &ir, &Params::default(), &ctx(condition, &lineage));
    }
    assert_eq!(shapes.bound(), 2);
}

#[test]
fn another_lineage_is_another_key() {
    let ir = plan(PINNED);
    let shapes = Shapes::new(&ir);
    for _ in 0..3 {
        let lineage = Arc::new(ArchetypeLineage::default());
        let ehr = Uuid::now_v7();
        prepared(
            &shapes,
            &ir,
            &ehr_param(ehr),
            &ctx(pinned(ehr, 1, Pred::True), &lineage),
        );
    }
    assert_eq!(shapes.bound(), 0);
}

#[test]
fn keys_seen_once_leave_the_shapes_in_use() {
    let ir = plan(PINNED);
    let shapes = Shapes::new(&ir);
    let lineage = Arc::new(ArchetypeLineage::default());
    let request = |admitted: Pred| {
        let ehr = Uuid::now_v7();
        prepared(
            &shapes,
            &ir,
            &ehr_param(ehr),
            &ctx(pinned(ehr, 1, admitted), &lineage),
        );
    };
    for _ in 0..3 {
        request(lab_results());
    }
    assert_eq!(shapes.bound(), 1);
    for n in 0..2 * super::KEYS_PER_PLAN {
        request(Pred::Data(
            Arc::new(DataMatch {
                templates: Some(vec![format!("template {n}")]),
                ..DataMatch::default()
            }),
            Polarity::Deny,
        ));
    }
    request(lab_results());
    assert_eq!(
        shapes.bound(),
        2,
        "the shape in use outlives the keys seen once"
    );
}
