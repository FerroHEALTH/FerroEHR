// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the AQL planner (path analysis + IR lowering). No DB.
//!
//! Queries are parsed by `openehr_query` and planned by [`ferroehr::aql::plan`]; each
//! test asserts either the lowered IR shape (accepted constructs) or the exact
//! [`AqlError`] variant (rejected constructs), covering the design doc's feature
//! envelope. Several queries are taken verbatim from the QUERY 1.1 worked
//! examples (`docs/specs/openehr/QUERY/docs/AQL/`).

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use openehr_query::ast::{BuiltinFunction, StringFunction};
use openehr_query::lexer::CompOp;
use openehr_query::parser::parse_str;

use ferroehr::aql::error::{AnalysisError, AqlError, AqlFeatureError};
use ferroehr::aql::ir::{
    AggFunc, ArchetypeConstraint, Coercion, ContainsTree, EhrField, Expr, LeafPath, Link, Operand,
    PathTarget, QueryIr, SelectValue, Source, TypedLit, VersionField, VersionScope,
};
use ferroehr::aql::ir::{ParamValue, Params};
use ferroehr::aql::lineage::ArchetypeLineage;
use ferroehr::aql::plan;
use ferroehr::aql::sql::SqlCtx;

// ── helpers ──────────────────────────────────────────────────────────────────

fn plan_ok(q: &str) -> QueryIr {
    let ast = parse_str(q).unwrap_or_else(|e| panic!("parse failed for {q:?}: {e}"));
    plan(
        &ast,
        &Params::new(),
        ferroehr::config::profile::SpecProfile::default(),
    )
    .unwrap_or_else(|e| panic!("plan failed for {q:?}: {e}"))
}

/// Plan `q` (parameterless) and lower it to SQL, returning the generated SQL
/// text. Literals bind as `$n` placeholders; structural SQL (EXISTS, OR, CASE …)
/// is asserted directly against the text — the same technique as
/// [`ferroehr::aql::sql::archetype_predicate_sql_for_tests`].
fn build_sql(q: &str) -> String {
    let ir = plan_ok(q);
    let ctx = SqlCtx {
        system_id: "sys.example.com".to_owned(),
        ehr_ids: Vec::new(),
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    ferroehr::aql::sql::build(&ir, &Params::new(), &ctx)
        .unwrap_or_else(|e| panic!("SQL build failed for {q:?}: {e}"))
        .sql
}

#[expect(
    clippy::panic_in_result_fn,
    reason = "the parse step is fixture setup, not the behaviour under test: an \
              unparseable query in this suite is a broken fixture and must fail \
              loudly, while the returned Result carries the PLANNING outcome the \
              test asserts on"
)]
fn plan_with(q: &str, params: &Params) -> Result<QueryIr, AqlError> {
    let ast = parse_str(q).unwrap_or_else(|e| panic!("parse failed for {q:?}: {e}"));
    plan(
        &ast,
        params,
        ferroehr::config::profile::SpecProfile::default(),
    )
}

fn plan_err(q: &str) -> AqlError {
    let ast = parse_str(q).unwrap_or_else(|e| panic!("parse failed for {q:?}: {e}"));
    plan(
        &ast,
        &Params::new(),
        ferroehr::config::profile::SpecProfile::default(),
    )
    .expect_err(&format!("expected planning to fail for {q:?}"))
}

fn data_leaf(value: &SelectValue) -> &LeafPath {
    match value {
        SelectValue::Path(PathTarget::Data(l)) => l,
        other => panic!("expected a data-path select value, got {other:?}"),
    }
}

fn anchor_attrs(leaf: &LeafPath) -> Vec<&str> {
    leaf.anchor.iter().map(|s| s.attribute.as_str()).collect()
}

fn fragment_names(leaf: &LeafPath) -> Vec<&str> {
    leaf.fragment.iter().map(|s| s.name.as_str()).collect()
}

// ── path-split table ───────────────────────────────────────────────────────

/// The design doc's canonical bp.v1 split (rooted at the COMPOSITION var,
/// descending through `content`): anchor = the structure hops down to the
/// ELEMENT node, fragment = `value/magnitude`.
#[test]
fn path_split_bp_example() {
    let ir = plan_ok(
        "SELECT c/content[openEHR-EHR-OBSERVATION.bp.v1]/data[at0001]/events[at0006]\
         /data/items[at0004]/value/magnitude \
         FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.report.v1]",
    );
    let leaf = data_leaf(&ir.select[0].value);
    assert_eq!(
        anchor_attrs(leaf),
        ["content", "data", "events", "data", "items"],
        "structure hops down to the ELEMENT node"
    );
    assert_eq!(fragment_names(leaf), ["value", "magnitude"]);
    assert_eq!(leaf.coercion, Coercion::Magnitude, "magnitude is numeric");
    assert!(
        leaf.anchor.iter().any(|s| s.multi_valued),
        "content/events/items are list-valued anchor steps"
    );
    // The leaf resolves to the numeric primitive(s) behind `magnitude`.
    assert!(leaf.types.contains("Real") || leaf.types.contains("Integer"));
}

/// A path-split table: query path → (anchor attrs, fragment names, coercion).
#[test]
fn path_split_table() {
    struct Case {
        select: &'static str,
        anchor: &'static [&'static str],
        fragment: &'static [&'static str],
        coercion: Coercion,
    }
    let from = "FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1]";
    let cases = [
        // whole-object select of the source node: no hops, no fragment.
        Case {
            select: "o",
            anchor: &[],
            fragment: &[],
            coercion: Coercion::Raw,
        },
        // a fragment-only leaf: name/value lives in the OBSERVATION node itself.
        Case {
            select: "o/name/value",
            anchor: &[],
            fragment: &["name", "value"],
            coercion: Coercion::Text,
        },
        // numeric magnitude via structure hops then a numeric-primitive fragment.
        Case {
            select: "o/data[at0001]/events[at0006]/data/items[at0004]/value/magnitude",
            anchor: &["data", "events", "data", "items"],
            fragment: &["value", "magnitude"],
            coercion: Coercion::Magnitude,
        },
        // the whole polymorphic DV value: fragment starts at `value`, mixed
        // candidate types ⇒ guarded Raw.
        Case {
            select: "o/data[at0001]/events[at0006]/data/items[at0004]/value",
            anchor: &["data", "events", "data", "items"],
            fragment: &["value"],
            coercion: Coercion::Raw,
        },
    ];
    for c in cases {
        let ir = plan_ok(&format!("SELECT {} {from}", c.select));
        let leaf = data_leaf(&ir.select[0].value);
        assert_eq!(anchor_attrs(leaf), c.anchor, "anchor for {}", c.select);
        assert_eq!(
            fragment_names(leaf),
            c.fragment,
            "fragment for {}",
            c.select
        );
        assert_eq!(leaf.coercion, c.coercion, "coercion for {}", c.select);
    }
}

#[test]
fn temporal_leaf_coercion() {
    // POINT_EVENT.time is a DV_DATE_TIME → Temporal.
    let ir = plan_ok(
        "SELECT o/data[at0001]/events[at0006]/time/value \
         FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1]",
    );
    let leaf = data_leaf(&ir.select[0].value);
    assert_eq!(anchor_attrs(leaf), ["data", "events"]);
    assert_eq!(fragment_names(leaf), ["time", "value"]);
    assert_eq!(leaf.coercion, Coercion::Temporal);
}

// ── FROM / CONTAINS trees ─────────────────────────────────────────────────────

#[test]
fn simple_ehr_contains_composition() {
    let ir = plan_ok("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c");
    assert_eq!(ir.sources.len(), 2);
    assert!(matches!(ir.sources[0], Source::Ehr(_)));
    assert!(matches!(ir.sources[1], Source::Rm(_)));
    match &ir.contains {
        ContainsTree::Operand {
            source,
            contained: Some(inner),
        } => {
            assert_eq!(source.0, 0);
            assert_eq!(inner.link, Link::Contains);
            assert!(matches!(inner.tree, ContainsTree::Operand { .. }));
        }
        other => panic!("unexpected contains tree: {other:?}"),
    }
}

#[test]
fn contains_or_tree() {
    let ir = plan_ok(
        "SELECT e/ehr_id/value FROM EHR e CONTAINS \
         (OBSERVATION o1[openEHR-EHR-OBSERVATION.lab.v1] \
          OR OBSERVATION o2[openEHR-EHR-OBSERVATION.glucose.v1])",
    );
    let ContainsTree::Operand {
        contained: Some(inner),
        ..
    } = &ir.contains
    else {
        panic!("expected EHR operand with a contained subtree");
    };
    assert!(matches!(inner.tree, ContainsTree::Or(_, _)));
}

#[test]
fn ehr_contains_two_vos_and() {
    let ir = plan_ok(
        "SELECT e/ehr_id/value FROM EHR e CONTAINS \
         (COMPOSITION c1[openEHR-EHR-COMPOSITION.referral.v1] \
          AND COMPOSITION c2[openEHR-EHR-COMPOSITION.report.v1])",
    );
    assert_eq!(ir.sources.len(), 3);
    let ContainsTree::Operand {
        contained: Some(inner),
        ..
    } = &ir.contains
    else {
        panic!("expected EHR operand");
    };
    assert!(matches!(inner.tree, ContainsTree::And(_, _)));
}

#[test]
fn not_contains_is_recorded() {
    let ir = plan_ok(
        "SELECT c/uid/value FROM EHR e CONTAINS \
         COMPOSITION c[openEHR-EHR-COMPOSITION.referral.v1] \
         NOT CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.lab.v1]",
    );
    // e -> c, then c NOT CONTAINS o.
    let ContainsTree::Operand {
        contained: Some(ehr_child),
        ..
    } = &ir.contains
    else {
        panic!("expected EHR operand");
    };
    let ContainsTree::Operand {
        contained: Some(comp_child),
        ..
    } = &ehr_child.tree
    else {
        panic!("expected COMPOSITION operand");
    };
    assert_eq!(comp_child.link, Link::NotContains);
}

// ── version scoping ───────────────────────────────────────────────────────────

fn rm_scope(ir: &QueryIr, idx: usize) -> &VersionScope {
    match &ir.sources[idx] {
        Source::Rm(s) => &s.scope,
        other => panic!("source {idx} is not an RM source: {other:?}"),
    }
}

#[test]
fn version_latest_is_default() {
    let ir = plan_ok("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c");
    assert_eq!(*rm_scope(&ir, 1), VersionScope::Latest);
}

#[test]
fn version_all_versions_scopes_contained_vo() {
    let ir = plan_ok(
        "SELECT c/uid/value FROM EHR e CONTAINS \
         VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c",
    );
    // sources: EHR(0), VERSION(1), COMPOSITION(2); the COMPOSITION inherits ALL.
    assert!(matches!(&ir.sources[1], Source::Version(s) if s.scope == VersionScope::All));
    assert_eq!(*rm_scope(&ir, 2), VersionScope::All);
}

#[test]
fn version_latest_explicit() {
    let ir = plan_ok(
        "SELECT c/uid/value FROM EHR e CONTAINS \
         VERSION v[LATEST_VERSION] CONTAINS COMPOSITION c",
    );
    assert_eq!(*rm_scope(&ir, 2), VersionScope::Latest);
}

#[test]
fn version_at_time_standard_predicate() {
    let ir = plan_ok(
        "SELECT c/uid/value FROM EHR e CONTAINS \
         VERSION v[commit_audit/time_committed >= '2021-01-01T00:00:00'] \
         CONTAINS COMPOSITION c",
    );
    let Source::Version(v) = &ir.sources[1] else {
        panic!("expected a VERSION source");
    };
    match &v.scope {
        VersionScope::Predicate(p) => {
            assert_eq!(p.field, VersionField::TimeCommitted);
            assert_eq!(p.op, CompOp::Ge);
        }
        other => panic!("expected a version predicate scope, got {other:?}"),
    }
    assert!(v.scope.is_at_time(), "time_committed comparison is at-time");
    // and it propagates to the contained COMPOSITION.
    assert!(matches!(rm_scope(&ir, 2), VersionScope::Predicate(_)));
}

#[test]
fn version_field_in_select() {
    let ir = plan_ok(
        "SELECT v/commit_audit/time_committed FROM EHR e CONTAINS \
         VERSION v CONTAINS COMPOSITION c",
    );
    assert!(matches!(
        &ir.select[0].value,
        SelectValue::Path(PathTarget::Version {
            field: VersionField::TimeCommitted,
            ..
        })
    ));
}

// ── SELECT / DISTINCT / aggregates / TOP / LIMIT ──────────────────────────────

/// An aggregate beside a plain projection is refused at planning: AQL 1.1
/// defines no grouping (QUERY master03 §Aggregate functions), so the shape
/// has no defined result and must never reach SQL, where it surfaced as an
/// ungrouped-column database error (#3054).
#[test]
fn an_aggregate_beside_a_plain_projection_is_refused() {
    for q in [
        "SELECT e/ehr_id/value, COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c",
        "SELECT COUNT(*), c/name/value FROM EHR e CONTAINS COMPOSITION c",
        "SELECT MAX(c/context/start_time/value), c FROM EHR e CONTAINS COMPOSITION c",
    ] {
        assert!(
            matches!(
                plan_err(q),
                AqlError::Feature(AqlFeatureError::MixedAggregateProjection)
            ),
            "{q} must be refused as a mixed aggregate projection"
        );
    }
    // Literals beside an aggregate are constant per row set and stay accepted.
    let ir = plan_ok("SELECT 'n', COUNT(*) FROM EHR e CONTAINS COMPOSITION c");
    assert_eq!(ir.select.len(), 2);
}

#[test]
fn distinct_and_alias() {
    let ir = plan_ok("SELECT DISTINCT c/name/value AS cname FROM EHR e CONTAINS COMPOSITION c");
    assert!(ir.distinct);
    assert_eq!(ir.select[0].alias.as_deref(), Some("cname"));
}

#[test]
fn aggregates() {
    let ir = plan_ok("SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c");
    assert!(matches!(
        &ir.select[0].value,
        SelectValue::Aggregate {
            func: AggFunc::Count,
            arg: None,
            distinct: false
        }
    ));

    let ir = plan_ok("SELECT COUNT(DISTINCT c/uid/value) FROM EHR e CONTAINS COMPOSITION c");
    assert!(matches!(
        &ir.select[0].value,
        SelectValue::Aggregate {
            func: AggFunc::Count,
            arg: Some(_),
            distinct: true
        }
    ));

    let ir = plan_ok(
        "SELECT MAX(o/data[at0001]/events[at0006]/data/items[at0004]/value/magnitude) \
         FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1]",
    );
    assert!(matches!(
        &ir.select[0].value,
        SelectValue::Aggregate {
            func: AggFunc::Max,
            ..
        }
    ));
}

#[test]
fn top_maps_to_limit() {
    let ir = plan_ok("SELECT TOP 5 c/uid/value FROM EHR e CONTAINS COMPOSITION c");
    assert_eq!(ir.limit, Some(5));
    assert_eq!(ir.offset, None);
}

#[test]
fn limit_and_offset() {
    let ir = plan_ok("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c LIMIT 10 OFFSET 20");
    assert_eq!(ir.limit, Some(10));
    assert_eq!(ir.offset, Some(20));
}

#[test]
fn top_with_limit_rejected() {
    let err = plan_err("SELECT TOP 5 c/uid/value FROM EHR e CONTAINS COMPOSITION c LIMIT 10");
    assert!(matches!(
        err,
        AqlError::Feature(AqlFeatureError::TopWithLimit)
    ));
}

// ── WHERE ──────────────────────────────────────────────────────────────────

#[test]
fn where_comparison_magnitude() {
    let ir = plan_ok(
        "SELECT o/data[at0001]/events[at0006]/data/items[at0004]/value/magnitude \
         FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1] \
         WHERE o/data[at0001]/events[at0006]/data/items[at0004]/value/magnitude >= 140",
    );
    match ir.filter.as_ref().unwrap() {
        Expr::Compare {
            op,
            coercion,
            lhs,
            rhs,
        } => {
            assert_eq!(*op, CompOp::Ge);
            assert_eq!(*coercion, Coercion::Magnitude);
            assert!(matches!(lhs, Operand::Path(PathTarget::Data(_))));
            assert!(matches!(rhs, Operand::Literal(TypedLit::Integer(140))));
        }
        other => panic!("expected a comparison, got {other:?}"),
    }
}

#[test]
fn where_exists_like_matches_and_boolean_tree() {
    let ir = plan_ok(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE EXISTS c/context/other_context \
           AND c/name/value LIKE 'left *' \
           OR c/archetype_details/template_id/value MATCHES {'t1', 't2'}",
    );
    let filter = ir.filter.as_ref().unwrap();
    // top level is an OR (AND binds tighter in the parser).
    let Expr::Or(lhs, rhs) = filter else {
        panic!("expected top-level OR, got {filter:?}");
    };
    assert!(matches!(lhs.as_ref(), Expr::And(_, _)));
    let Expr::Matches { values, .. } = rhs.as_ref() else {
        panic!("expected MATCHES on the right");
    };
    assert_eq!(values.len(), 2);
}

#[test]
fn temporal_literal_retyped_from_context() {
    // POINT_EVENT.time/value is temporal → the quoted RHS is retyped Temporal.
    let ir = plan_ok(
        "SELECT o/name/value FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1] \
         WHERE o/data[at0001]/events[at0006]/time/value >= '2021-01-01T00:00:00'",
    );
    match ir.filter.as_ref().unwrap() {
        Expr::Compare { rhs, coercion, .. } => {
            assert_eq!(*coercion, Coercion::Temporal);
            assert!(matches!(rhs, Operand::Literal(TypedLit::Temporal(_))));
        }
        other => panic!("expected a comparison, got {other:?}"),
    }
}

// ── parameters ─────────────────────────────────────────────────────────────

#[test]
fn parameters_are_collected_and_validated() {
    let q = "SELECT c/uid/value FROM EHR e[ehr_id/value=$ehr] CONTAINS COMPOSITION c \
             WHERE c/name/value = $cname";
    // both bound → ok, params sorted+unique.
    let params = Params::new()
        .with("ehr", ParamValue::Str("1234".into()))
        .with("cname", ParamValue::Str("BP".into()));
    let ir = plan_with(q, &params).unwrap();
    assert_eq!(ir.params, vec!["cname".to_string(), "ehr".to_string()]);
}

#[test]
fn missing_parameter_is_rejected() {
    let q = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE c/name/value = $cname";
    let err = plan_with(q, &Params::new()).unwrap_err();
    assert!(matches!(
        err,
        AqlError::Analysis(AnalysisError::UnboundParameter(p)) if p == "cname"
    ));
}

#[test]
fn ehr_id_predicate_lowers_to_field() {
    let ir = plan_with(
        "SELECT e/ehr_id/value FROM EHR e[ehr_id/value=$id] CONTAINS COMPOSITION c",
        &Params::new().with("id", ParamValue::Str("x".into())),
    )
    .unwrap();
    let Source::Ehr(ehr) = &ir.sources[0] else {
        panic!("expected EHR source");
    };
    assert_eq!(ehr.predicates.len(), 1);
    assert_eq!(ehr.predicates[0].field, EhrField::EhrId);
}

// ── rejections (exact variants) ───────────────────────────────────────────────

#[test]
fn terminology_function_rejected() {
    let err = plan_err(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/category/defining_code/code_string \
           MATCHES TERMINOLOGY('expand', 'hl7.org/fhir/4.0', 'http://snomed.info/sct')",
    );
    assert!(matches!(
        err,
        AqlError::Feature(AqlFeatureError::MatchesTerminology)
    ));
}

#[test]
fn matches_uri_rejected() {
    let err = plan_err(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/category/defining_code/code_string MATCHES {http://snomed.info/id/442031002}",
    );
    assert!(matches!(
        err,
        AqlError::Feature(AqlFeatureError::MatchesUri)
    ));
}

/// The QUERY spec makes the bracket predicate `[archetype_node_id='…']` the
/// standard-predicate form of the archetype and node shortcut predicates
/// (`QUERY/docs/AQL/master03-syntax.adoc` §"Archetype predicate": "These
/// predicates could also be written as standard predicates"; §"Node
/// predicate"), so its operand set is exactly {archetype identifier, archetype
/// node code} — the two shapes the RM lets `LOCATABLE.archetype_node_id` hold.
/// Both twins are asserted: the two legal forms plan into their respective
/// constraints, a third shape is a typed reject rather than an archetype
/// constraint that can never match.
#[test]
fn archetype_node_id_predicate_admits_only_archetype_ids_and_node_codes() {
    let constraint = |q: &str| {
        let ir = plan_ok(q);
        ir.sources
            .iter()
            .find_map(|s| match s {
                Source::Rm(r) => r.archetype.clone(),
                Source::Ehr(_) | Source::Version(_) => None,
            })
            .unwrap_or_else(|| panic!("no archetype constraint planned for {q:?}"))
    };
    assert!(matches!(
        constraint(
            "SELECT c/uid/value FROM EHR e CONTAINS \
             COMPOSITION c[archetype_node_id='openEHR-EHR-COMPOSITION.encounter.v1']"
        ),
        ArchetypeConstraint::Archetype(a) if a == "openEHR-EHR-COMPOSITION.encounter.v1"
    ));
    assert!(matches!(
        constraint(
            "SELECT o/uid/value FROM EHR e CONTAINS OBSERVATION o[archetype_node_id='at0001']"
        ),
        ArchetypeConstraint::NodeCode(c) if c == "at0001"
    ));

    let err = plan_err(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c[archetype_node_id='openEHR-garbage']",
    );
    assert!(
        matches!(
            &err,
            AqlError::Analysis(AnalysisError::MalformedArchetypeNodeId(v))
                if v == "openEHR-garbage"
        ),
        "got {err:?}"
    );
}

#[test]
fn demographic_source_rejected() {
    let err = plan_err("SELECT p/uid/value FROM EHR e CONTAINS PERSON p");
    assert!(matches!(
        err,
        AqlError::Feature(AqlFeatureError::UnsupportedSourceClass(c)) if c == "PERSON"
    ));
}

#[test]
fn branch_version_id_rejected() {
    let err = plan_err(
        "SELECT c/uid/value FROM EHR e CONTAINS \
         VERSION v[uid/value='abc::sys.example.com::1.1.1'] CONTAINS COMPOSITION c",
    );
    assert!(matches!(
        err,
        AqlError::Feature(AqlFeatureError::BranchVersionAddressing)
    ));
}

#[test]
fn unknown_class_rejected() {
    let err = plan_err("SELECT x/uid/value FROM EHR e CONTAINS NOTATYPE x");
    assert!(matches!(
        err,
        AqlError::Analysis(AnalysisError::UnknownClass(c)) if c == "NOTATYPE"
    ));
}

#[test]
fn unknown_variable_rejected() {
    let err = plan_err("SELECT zzz/uid/value FROM EHR e CONTAINS COMPOSITION c");
    assert!(matches!(
        err,
        AqlError::Analysis(AnalysisError::UnknownVariable(v)) if v == "zzz"
    ));
}

#[test]
fn unresolvable_attribute_rejected() {
    let err = plan_err("SELECT c/not_an_attr FROM EHR e CONTAINS COMPOSITION c");
    assert!(matches!(
        err,
        AqlError::Analysis(AnalysisError::UnresolvableAttribute { attribute, .. })
            if attribute == "not_an_attr"
    ));
}

#[test]
fn unsupported_function_rejected() {
    let err = plan_err("SELECT frobnicate(c/uid/value) FROM EHR e CONTAINS COMPOSITION c");
    assert!(matches!(
        err,
        AqlError::Feature(AqlFeatureError::UnsupportedFunction(f)) if f == "frobnicate"
    ));
}

#[test]
fn whitelisted_function_accepted() {
    let ir = plan_ok("SELECT length(c/name/value) FROM EHR e CONTAINS COMPOSITION c");
    assert!(matches!(
        &ir.select[0].value,
        SelectValue::Function {
            func: BuiltinFunction::String(StringFunction::Length),
            ..
        }
    ));
}

// ── chapter-16 audit additions (QUERY master03) ──────────────────────────────

/// Variable names must be unique within an AQL statement (§Variables/Syntax).
#[test]
fn duplicate_variable_rejected() {
    let e = plan_err("SELECT c FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION c");
    assert!(matches!(
        e,
        AqlError::Analysis(AnalysisError::DuplicateVariable(v)) if v == "c"
    ));
}

/// Variable names are not case-sensitive (§Variables/Syntax) — both for the
/// uniqueness check and for reference resolution.
#[test]
fn variable_names_fold_case() {
    let e = plan_err("SELECT c FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION C");
    assert!(matches!(
        e,
        AqlError::Analysis(AnalysisError::DuplicateVariable(_))
    ));
    // A reference differing only by case resolves.
    plan_ok("SELECT C/name/value FROM EHR e CONTAINS COMPOSITION c");
}

/// `row_count` minimum is 1, `offset` minimum is 0 (§LIMIT/Syntax).
#[test]
fn limit_zero_rejected() {
    let e = plan_err("SELECT c FROM COMPOSITION c LIMIT 0");
    assert!(matches!(
        e,
        AqlError::Analysis(AnalysisError::PagingBounds {
            clause: "LIMIT",
            ..
        })
    ));
}

/// SUM/AVG accept Integer/Real input only (§Functions/SUM, AVG) — a textual
/// leaf is a typed reject.
#[test]
fn sum_over_textual_leaf_rejected() {
    let e = plan_err("SELECT SUM(c/name/value) FROM COMPOSITION c");
    assert!(matches!(
        e,
        AqlError::Analysis(AnalysisError::AggregateInputType { func: "SUM", .. })
    ));
}

/// Scalar-function arity is validated at lowering (§Functions).
#[test]
fn function_arity_enforced() {
    let e = plan_err("SELECT length(c/name/value, c/name/value) FROM COMPOSITION c");
    assert!(matches!(
        e,
        AqlError::Analysis(AnalysisError::FunctionArity { func: "LENGTH", .. })
    ));
    let e = plan_err("SELECT current_date(c/name/value) FROM COMPOSITION c");
    assert!(matches!(
        e,
        AqlError::Analysis(AnalysisError::FunctionArity {
            func: "CURRENT_DATE",
            ..
        })
    ));
}

/// The whole single-row function set plans (§Functions: string, numeric,
/// date/time incl. `CURRENT_TIMEZONE`, and the string function `CONTAINS`).
#[test]
fn full_scalar_function_set_plans() {
    for q in [
        "SELECT substring(c/name/value, 1, 3) FROM COMPOSITION c",
        "SELECT position('x', c/name/value) FROM COMPOSITION c",
        "SELECT contains(c/name/value, 'x') FROM COMPOSITION c",
        "SELECT concat(c/name/value, '!') FROM COMPOSITION c",
        "SELECT concat_ws('-', c/name/value, 'x') FROM COMPOSITION c",
        "SELECT abs(o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude) \
         FROM OBSERVATION o",
        "SELECT round(o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude) \
         FROM OBSERVATION o",
        "SELECT current_date() FROM COMPOSITION c",
        "SELECT current_timezone() FROM COMPOSITION c",
        "SELECT now() FROM COMPOSITION c",
    ] {
        plan_ok(q);
    }
}

// ── SQL lowering: the G-row fixes (QUERY master03) ────────────────────────────

/// Anchor-probe `EXISTS` count. The population gate is no longer an `EXISTS`
/// subquery (a direct `ehr.is_queryable` column filter over a join),
/// so every remaining `EXISTS` is a containment anchor (OR / NOT CONTAINS).
fn anchor_exists(sql: &str) -> usize {
    sql.matches("EXISTS(SELECT")
        .count()
        .saturating_sub(currency_probes(sql))
}

/// The trunk-head currency probes in `sql`: the `EXISTS` over `vo_head` every
/// `LATEST_VERSION` version alias carries (`vo_head.trunk_head_sys_version` IS
/// the trunk head — RM common master06 §The 'Virtual Version Tree').
fn currency_probes(sql: &str) -> usize {
    sql.matches(r#"FROM "vo_head" AS "#).count()
}

/// A predicate whose leaf crosses a MULTI-VALUED FRAGMENT attribute
/// (`LOCATABLE.links` — LINK is no structure root, so the whole list lives in
/// the root node's fragment) lowers existentially through the set-returning
/// `jsonb_path_query`, never the first-match scalar: QUERY master03 §WHERE is
/// silent on multi-valued predicates, and any-match is the adjudicated
/// semantics recorded on `data_leaf_exists`.
#[test]
fn multi_valued_fragment_predicate_lowers_existentially() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/links/target/value = 'ehr://x/1'",
    );
    assert!(
        sql.contains("jsonb_path_query("),
        "set-returning per-match extraction: {sql}"
    );
    assert!(
        !sql.contains("jsonb_path_query_first"),
        "no first-match scalar remains on the links predicate: {sql}"
    );
    assert!(sql.contains("EXISTS"), "existential shape: {sql}");
}

/// The same existential lowering applies when the multi-valued fragment sits
/// UNDER an anchored structure walk (`context` is a structure hop,
/// `participations` its list-valued fragment).
#[test]
fn anchored_multi_valued_fragment_predicate_is_existential() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/context/participations/performer/name = 'Dr Second'",
    );
    assert!(
        sql.contains("jsonb_path_query(") && sql.contains("BETWEEN"),
        "anchored walk + per-match extraction: {sql}"
    );
    assert!(sql.contains("EXISTS"), "existential shape: {sql}");
}

/// Projection over a multi-valued fragment path yields every match as ONE
/// jsonb array cell (`jsonb_path_query_array`, NULL when nothing matches) —
/// QUERY master03 §Identified Paths is silent; the array cell is the decided
/// semantics (no element invisible, row cardinality untouched).
#[test]
fn multi_valued_fragment_projection_is_an_array_cell() {
    let sql = build_sql("SELECT c/links/target/value FROM EHR e CONTAINS COMPOSITION c");
    assert!(
        sql.contains("jsonb_path_query_array("),
        "array-cell projection: {sql}"
    );
    assert!(
        sql.contains("nullif("),
        "an empty match set is a NULL cell, not []: {sql}"
    );
}

/// A single-valued fragment leaf keeps the exact first-match scalar
/// extraction — the existential machinery only fires where multiplicity is
/// real.
#[test]
fn single_valued_fragment_predicate_keeps_the_scalar_extraction() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/name/value = 'report'",
    );
    assert!(
        sql.contains("jsonb_path_query_first"),
        "single-valued stays scalar: {sql}"
    );
    assert!(
        !sql.contains("jsonb_path_query("),
        "no set-returning extraction for a single-valued leaf: {sql}"
    );
}

/// `EXISTS <path>` over a multi-valued fragment lowers through the same
/// per-match shape — exact in both polarities (the test is boolean, so the
/// three-valued comparison caveat does not apply).
#[test]
fn exists_over_multi_valued_fragment_is_per_match() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE EXISTS c/links/target/value",
    );
    assert!(
        sql.contains("jsonb_path_query(") && sql.contains("IS NOT NULL"),
        "existence probes every match: {sql}"
    );
}

/// A predicate on the path ROOT reaches the SQL: the leaf value only exists
/// where the source node satisfies it (QUERY master03 §Identified Paths — a
/// node predicate qualifies the object the path reads). Before the fix the
/// constraint silently vanished and any composition's name matched.
#[test]
fn path_root_predicate_reaches_the_sql() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c[openEHR-EHR-COMPOSITION.report.v1]/name/value = 'x'",
    );
    assert!(
        sql.contains("arch_concept"),
        "the root archetype predicate is lowered, not dropped: {sql}"
    );
    // Projection context: the guarded read is a CASE over the same condition.
    let sql = build_sql(
        "SELECT c[openEHR-EHR-COMPOSITION.report.v1]/name/value \
         FROM EHR e CONTAINS COMPOSITION c",
    );
    assert!(
        sql.contains("CASE WHEN") && sql.contains("arch_concept"),
        "the projected read is guarded by the root predicate: {sql}"
    );
}

/// A predicate on a FRAGMENT step lowers to a jsonpath FILTER whose compared
/// values travel in the bound `vars` jsonb argument (the third
/// `jsonb_path_query*` argument), never spliced into the path text — QUERY
/// master03 §Node predicate / §Standard predicate at a non-structure step.
#[test]
fn fragment_step_predicate_lowers_to_a_jsonpath_filter() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/links[meaning/value = 'issue']/target/value = 'x'",
    );
    assert!(
        sql.contains("AS jsonb)"),
        "the filter variables bind as the jsonb vars argument: {sql}"
    );
    assert!(
        sql.contains("jsonb_path_query("),
        "the predicated multi-valued fragment stays existential: {sql}"
    );
}

/// A root predicate on a whole-object projection guards the `vo_id` locator:
/// a non-matching row serves a NULL cell (the executor short-circuits a NULL
/// `vo_id`), never a dropped row.
#[test]
fn whole_object_root_predicate_guards_the_locator() {
    let sql = build_sql(
        "SELECT c[openEHR-EHR-COMPOSITION.report.v1] \
         FROM EHR e CONTAINS COMPOSITION c",
    );
    assert!(
        sql.contains("CASE WHEN") && sql.contains("arch_concept"),
        "the vo_id locator is CASE-guarded by the predicate: {sql}"
    );
    assert!(
        sql.contains(r#""vo_id" END"#),
        "the guard wraps the vo_id locator: {sql}"
    );
}

/// OR-containment under an EHR lowers to a disjunction of correlated
/// `EXISTS` subqueries (QUERY master03 §Containment — "Logical operators AND and
/// OR"). Previously `sql.rs` returned `SqlError::Unsupported` for any `OR` in the
/// FROM tree; it must now build.
#[test]
fn or_contains_under_ehr_lowers_to_disjunctive_exists() {
    let sql = build_sql(
        "SELECT e/ehr_id/value FROM EHR e CONTAINS \
         (OBSERVATION o1[openEHR-EHR-OBSERVATION.lab.v1] \
          OR OBSERVATION o2[openEHR-EHR-OBSERVATION.glucose.v1])",
    );
    assert_eq!(
        anchor_exists(&sql),
        2,
        "one anchor EXISTS per OR branch: {sql}"
    );
    assert!(sql.contains(" OR "), "the branches are OR-combined: {sql}");
}

/// OR-containment under a COMPOSITION shares the parent VO and
/// interval-anchors each branch inside its node subtree (`num BETWEEN`).
#[test]
fn or_contains_under_vo_interval_anchors_each_branch() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS \
         COMPOSITION c[openEHR-EHR-COMPOSITION.report.v1] CONTAINS \
         (OBSERVATION o[openEHR-EHR-OBSERVATION.lab.v1] \
          OR OBSERVATION o1[openEHR-EHR-OBSERVATION.glucose.v1])",
    );
    assert_eq!(anchor_exists(&sql), 2, "{sql}");
    assert!(sql.contains(" OR "), "{sql}");
    assert!(
        sql.contains("BETWEEN"),
        "each branch is interval-anchored in the parent VO: {sql}"
    );
}

/// a nested AND/OR containment tree lowers to the matching boolean tree of
/// `EXISTS` filters (QUERY master03 §Containment).
#[test]
fn nested_and_or_contains_tree_builds() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS \
         COMPOSITION c[openEHR-EHR-COMPOSITION.report.v1] CONTAINS \
         (OBSERVATION o1[openEHR-EHR-OBSERVATION.lab.v1] \
          OR (OBSERVATION o2[openEHR-EHR-OBSERVATION.glucose.v1] \
              AND OBSERVATION o3[openEHR-EHR-OBSERVATION.bp.v1]))",
    );
    assert_eq!(
        anchor_exists(&sql),
        3,
        "one anchor EXISTS per operand: {sql}"
    );
    assert!(sql.contains(" OR ") && sql.contains(" AND "), "{sql}");
}

/// NOT CONTAINS generalises to a negated `EXISTS` over an arbitrary
/// operand tree — a compound (OR) operand now builds (previously
/// `SqlError::Unsupported`). QUERY master03 §Containment, §NOT.
#[test]
fn not_contains_compound_operand_builds() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS \
         COMPOSITION c[openEHR-EHR-COMPOSITION.referral.v1] NOT CONTAINS \
         (OBSERVATION o1[openEHR-EHR-OBSERVATION.lab.v1] \
          OR OBSERVATION o2[openEHR-EHR-OBSERVATION.glucose.v1])",
    );
    assert!(sql.contains("NOT"), "the exclusion is negated: {sql}");
    assert_eq!(anchor_exists(&sql), 2, "{sql}");
}

/// a mixed-type (`Raw`) leaf compared to a numeric literal extracts
/// numerically with a `jsonb_typeof` guard (non-number occurrences → NULL),
/// never a lexical text compare (QUERY master03 §Comparison operators).
#[test]
fn raw_leaf_numeric_comparison_dispatches_to_numeric() {
    let sql = build_sql(
        "SELECT o/name/value FROM EHR e CONTAINS \
         OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1] \
         WHERE o/data[at0001]/events[at0006]/data/items[at0004]/value >= 140",
    );
    assert!(
        sql.contains("jsonb_typeof") && sql.contains("'number'"),
        "the polymorphic value leaf is compared numerically under a type guard: {sql}"
    );
}

/// the same mixed-type leaf compared to a *string* literal stays on the
/// text path (no numeric guard).
#[test]
fn raw_leaf_text_comparison_stays_text() {
    let sql = build_sql(
        "SELECT o/name/value FROM EHR e CONTAINS \
         OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1] \
         WHERE o/data[at0001]/events[at0006]/data/items[at0004]/value = 'x'",
    );
    assert!(
        !sql.contains("jsonb_typeof"),
        "a text comparison must not use the numeric guard: {sql}"
    );
}

/// MIN/MAX over a String leaf compare textually, not by forced numeric
/// magnitude — "Input values type should be either String, Date, Time, Integer
/// or Real, and it will also determine the return type" (QUERY master03 §MAX).
#[test]
fn min_max_over_text_leaf_is_not_forced_numeric() {
    let sql = build_sql("SELECT MAX(c/name/value) FROM COMPOSITION c");
    assert!(
        !sql.contains("numeric") && !sql.contains("openehr_magnitude"),
        "MAX over a text leaf is not coerced numeric: {sql}"
    );
}

/// MIN/MAX over a numeric leaf still lowers numerically.
#[test]
fn min_max_over_numeric_leaf_is_numeric() {
    let sql = build_sql(
        "SELECT MAX(o/data[at0001]/events[at0006]/data/items[at0004]/value/magnitude) \
         FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1]",
    );
    assert!(
        sql.contains("numeric"),
        "MAX over a numeric leaf casts to numeric: {sql}"
    );
}

// ── Promoted context_start fast path + uid synthesis ────────────────────────

/// The patient-dashboard shape orders by the promoted `node.context_start`
/// column, not the correlated `EVENT_CONTEXT` extraction + `::timestamptz` cast.
#[test]
fn dashboard_order_by_uses_context_start_column() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         ORDER BY c/context/start_time/value DESC",
    );
    assert!(
        sql.contains(r#""context_start" DESC"#),
        "ORDER BY reads the promoted column: {sql}"
    );
    // The ordering no longer casts an extracted EVENT_CONTEXT value to timestamptz.
    assert!(
        !sql.contains("timestamptz"),
        "no per-row ::timestamptz cast for the ordering: {sql}"
    );
}

/// The same context start-time leaf in a temporal WHERE comparison also reads
/// the promoted column (the fast path lives in the shared value lowering, so it
/// covers ORDER BY, WHERE, and aggregation uniformly).
#[test]
fn where_temporal_compare_uses_context_start_column() {
    let sql = build_sql(
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/context/start_time/value > '2021-01-01T00:00:00Z'",
    );
    assert!(
        sql.contains(r#""context_start""#),
        "WHERE comparison reads the promoted column: {sql}"
    );
}

/// A near-miss must fall back to the general lowering: a temporal leaf on an
/// OBSERVATION (not a promoted type) still extracts through the subtree.
#[test]
fn non_promoted_temporal_leaf_falls_back_to_subquery() {
    let sql = build_sql(
        "SELECT o/name/value \
         FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1] \
         ORDER BY o/data[at0001]/events[at0006]/time/value DESC",
    );
    assert!(
        !sql.contains("context_start"),
        "OBSERVATION time is not the promoted composition leaf: {sql}"
    );
    assert!(
        sql.contains("jsonb_path_query_first") && sql.contains("openehr_timestamp("),
        "the general temporal lowering (extraction + the total floor) is used: {sql}"
    );
}

/// A near-miss on the promoted type but a different path (`end_time`, not the
/// registered `start_time`) also falls back — the match is path-exact.
#[test]
fn other_composition_context_path_falls_back() {
    let sql = build_sql(
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c \
         ORDER BY c/context/end_time/value DESC",
    );
    assert!(
        !sql.contains("context_start"),
        "only the registered context/start_time/value promotes: {sql}"
    );
}

/// Projection of the context start-time reads the canonical JSON fragment, not
/// the timestamptz column — the fast path is comparison/ordering only, so the
/// `RESULT_SET` cell keeps the verbatim `DV_DATE_TIME` value.
#[test]
fn projection_of_context_start_time_is_not_promoted() {
    let sql = build_sql("SELECT c/context/start_time/value FROM EHR e CONTAINS COMPOSITION c");
    assert!(
        !sql.contains("context_start"),
        "projection must extract the canonical value, not the timestamptz column: {sql}"
    );
    assert!(
        sql.contains("jsonb_path_query_first"),
        "projection extracts the leaf from the fragment: {sql}"
    );
}

/// uid synthesis: `c/uid/value` on a COMPOSITION variable synthesizes the `OBJECT_VERSION_ID`
/// from the joined `version` (RM common master06 §Version Identification) —
/// it is not stored in the fragment.
#[test]
fn composition_uid_value_is_synthesized_from_the_version_row() {
    let sql = build_sql("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c");
    assert!(
        sql.contains("creating_system_id"),
        "uid/value is composed from version columns: {sql}"
    );
}

/// uid synthesis: `c/uid` yields the `OBJECT_VERSION_ID` object cell.
#[test]
fn composition_uid_object_is_built() {
    let sql = build_sql("SELECT c/uid FROM EHR e CONTAINS COMPOSITION c");
    assert!(
        sql.contains("jsonb_build_object"),
        "uid projects the OBJECT_VERSION_ID object: {sql}"
    );
    assert!(
        sql.contains("creating_system_id"),
        "the object's value is the synthesized version id: {sql}"
    );
}

/// uid synthesis: a contained object's own `uid` is NOT synthesized — only a
/// versioned-object root gets a server-assigned `OBJECT_VERSION_ID`; the
/// OBSERVATION's uid falls through to the stored fragment.
#[test]
fn contained_object_uid_is_not_synthesized() {
    let sql = build_sql(
        "SELECT o/uid/value \
         FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.bp.v1]",
    );
    assert!(
        sql.contains("jsonb_path_query_first"),
        "the observation uid is read from its stored fragment: {sql}"
    );
}

// ── Typed EHR-id predicate + single correlated population gate ──────────────

/// `e/ehr_id/value = '<uuid>'` lowers to a uuid-typed comparison on `ehr.id`
/// (index-served; value-based equality = the case-insensitive identifier
/// semantics of BASE `base_types` master05 §Composite Identifiers and Case),
/// never the index-blind text-cast-both-sides form.
#[test]
fn ehr_id_equality_is_uuid_typed() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE e/ehr_id/value = '11111111-2222-4333-8444-555555555555'",
    );
    assert!(
        sql.contains("CAST($") && sql.contains("AS uuid)"),
        "uuid-typed RHS expected: {sql}"
    );
    assert!(
        !sql.contains(r#"CAST("e0"."id" AS text) ="#),
        "the text-cast equality must be gone: {sql}"
    );
}

/// A non-uuid literal can equal no EHR id: `=` lowers to constant FALSE and
/// `!=` to constant TRUE — no scan, no cast error.
#[test]
fn ehr_id_equality_with_a_non_uuid_literal_is_constant() {
    let sql = build_sql("SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_id/value = 'not-a-uuid'");
    assert!(
        sql.contains("FALSE") || sql.contains('$'),
        "constant lowering: {sql}"
    );
    assert!(
        !sql.contains("AS uuid)"),
        "no uuid cast for a non-uuid literal: {sql}"
    );
}

/// Ordering comparisons on the EHR id stay textual (uuid byte order is not
/// text order; QUERY master03 defines string comparison) — the typed fast
/// path fires for equality only.
#[test]
fn ehr_id_ordering_stays_textual() {
    let sql = build_sql(
        "SELECT e/ehr_id/value FROM EHR e \
         WHERE e/ehr_id/value > '11111111-2222-4333-8444-555555555555'",
    );
    assert!(
        sql.contains(r#"CAST("e0"."id" AS text)"#),
        "ordering keeps the text form: {sql}"
    );
}

/// ORDER BY `e/ehr_id/value` sorts by the raw `ehr.id` uuid column (index-served),
/// not the `CAST(id AS text)` the projection reads — canonical UUID text order
/// equals uuid binary order, so the sequence is identical (BASE
/// `base_types` master05 §Basic Types — Uuid). The projected cell keeps its
/// text form.
#[test]
fn order_by_ehr_id_uses_the_raw_uuid_column() {
    let sql = build_sql("SELECT e/ehr_id/value FROM EHR e ORDER BY e/ehr_id/value DESC");
    assert!(
        sql.contains(r#"ORDER BY "e0"."id" DESC"#),
        "ORDER BY sorts by the raw uuid column: {sql}"
    );
    assert!(
        !sql.contains(r#"ORDER BY CAST("e0"."id" AS text)"#),
        "the ORDER BY no longer casts the id to text: {sql}"
    );
    // The projected cell still renders the id as text (to_jsonb of the cast).
    assert!(
        sql.contains(r#"CAST("e0"."id" AS text)"#),
        "the projection keeps the text cast: {sql}"
    );
}

/// The population gate (SM `I_QUERY_SERVICE`: full-population queries run over
/// `is_queryable = true` EHRs) is emitted ONCE per join-connected component —
/// a VO root linked to its EHR alias is covered by the alias's gate — and as a
/// direct `ehr.is_queryable` column filter, NOT a per-query `EXISTS`
/// probe over every current `EHR_STATUS` root.
#[test]
fn population_gate_is_single_and_column_filtered() {
    let sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE e/ehr_id/value = '11111111-2222-4333-8444-555555555555' \
         ORDER BY c/context/start_time/value DESC LIMIT 20",
    );
    // The COMPOSITION root is join-linked to the EHR alias, so exactly one gate
    // is emitted, on the already-joined `ehr` row's promoted column (the boolean
    // binds as a parameter).
    assert_eq!(
        sql.matches(r#""e0"."is_queryable" = $"#).count(),
        1,
        "exactly one population gate on the EHR alias: {sql}"
    );
    assert!(
        !sql.contains("AS \"qgv") && anchor_exists(&sql) == 0,
        "the per-EHR_STATUS EXISTS gate is gone: {sql}"
    );
}

/// A bare VO source with no EHR variable still gets its own gate (nothing else
/// covers it): the owning `ehr` row is joined on the node's `ehr_id` and its
/// promoted `is_queryable` column is filtered.
#[test]
fn unlinked_vo_root_keeps_its_own_gate() {
    let sql = build_sql("SELECT c/uid/value FROM COMPOSITION c");
    assert_eq!(
        sql.matches(r#""qg0"."is_queryable" = $"#).count(),
        1,
        "the bare root is gated on the joined ehr row: {sql}"
    );
    assert!(
        sql.contains(r#""qg0"."id" = "n0"."ehr_id""#),
        "the gate correlates the joined ehr row to the VO root's ehr_id: {sql}"
    );
    assert!(
        anchor_exists(&sql) == 0,
        "the gate is a join + column filter, not an EXISTS probe: {sql}"
    );
}

/// Every partitioned relation the emitter names carries `tier = 'hot'` as a
/// SQL LITERAL, never a bound parameter.
///
/// `version`, `node` and `vo_attestation` are partitioned by tier; a literal is
/// what lets PostgreSQL prune the cold partition at PLAN time rather than at
/// execution (PostgreSQL 18, "Partition Pruning"). Archived content therefore
/// leaves the queryable store until it is restored. No openEHR spec governs
/// storage tiering — our own design/extension.
#[test]
fn every_partitioned_relation_is_pinned_to_the_hot_tier_by_a_literal() {
    for aql in [
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
        WARD_QUERY,
        "SELECT f1/name/value FROM EHR e CONTAINS FOLDER f1 NOT CONTAINS FOLDER f2",
        "SELECT c/uid/value FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c",
    ] {
        let sql = build_sql(aql);
        // One `tier =` comparison per partitioned relation in the statement,
        // and every one of them written out.
        let comparisons = sql.matches(r#""tier" = "#).count();
        // sea-query parenthesises a custom expression, so the emitted form is
        // `"x"."tier" = ('hot')` — a literal either way.
        let literals = sql.matches(r#""tier" = ('hot')"#).count();
        assert!(comparisons > 0, "no tier predicate at all in: {sql}");
        assert_eq!(
            literals, comparisons,
            "every tier predicate must be the literal 'hot', never a bind: {sql}"
        );
    }
}

// ── the LIMIT-streaming FROM shape ───────────────────────────────────────────

/// Lower `q` with an effective LIMIT (the streaming-shape trigger).
fn build_sql_limited(q: &str) -> String {
    let ir = plan_ok(q);
    let ctx = SqlCtx {
        system_id: "sys.example.com".to_owned(),
        ehr_ids: Vec::new(),
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: Some(50),
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    ferroehr::aql::sql::build(&ir, &Params::new(), &ctx)
        .unwrap_or_else(|e| panic!("SQL build failed for {q:?}: {e}"))
        .sql
}

const WARD_QUERY: &str = "SELECT e/ehr_id/value, c/uid/value \
     FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o [openEHR-EHR-OBSERVATION.blood_pressure.v2] \
     WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude > 130";

/// A LIMIT-bearing, unordered population query lowers to the STREAMING
/// shape: the version spine is the single FROM item, every node source is a
/// `LATERAL` subquery behind the `OFFSET 0` pull-up fence, and the root is
/// pre-filtered by `version.kind` — so the planner walks current versions
/// lazily and stops at the LIMIT instead of materializing an
/// archetype-anchor bitmap over the corpus (QUERY master03 §LIMIT: without
/// ORDER BY, which rows return is explicitly non-deterministic).
#[test]
fn limit_without_order_by_streams() {
    let sql = build_sql_limited(WARD_QUERY);
    assert!(
        sql.contains(r#"FROM "version" AS "v"#),
        "the version spine drives: {sql}"
    );
    // The dead-root elision (the post-streaming ladder's rung 1): this query
    // reads nothing from the COMPOSITION root's node row (`c/uid/value` is
    // synthesized from the spine) and pins no root condition beyond the RM
    // type the spine's `kind` already filters — so NO root lateral is
    // emitted, and the OBSERVATION lateral binds directly on the spine's
    // `(vo_id, sys_version)`.
    assert_eq!(
        sql.matches("JOIN LATERAL").count(),
        2,
        "one lateral for the observation source plus the correlated predicate probe (no dead root lateral): {sql}"
    );
    assert!(
        !sql.contains(r#""num" = "#),
        "no root `num = 0` probe survives the dead-root elision: {sql}"
    );
    // sea-query binds the fence's 0 as a parameter — the pull-up block is
    // syntactic (an OFFSET clause of any value), so the fence holds.
    assert_eq!(
        sql.matches("OFFSET $").count(),
        2,
        "every lateral carries the pull-up fence: {sql}"
    );
    // The multi-valued-path predicate stays a CORRELATED probe: the EXISTS
    // lives inside a lateral (never a bare WHERE sublink the planner could
    // decorrelate into a corpus-wide materialize).
    assert!(
        sql.contains("EXISTS(SELECT") && sql.contains(r#""hit""#),
        "the predicate EXISTS is hosted in the lateral boolean probe: {sql}"
    );
    assert!(
        sql.contains(r#""kind" IN"#),
        "the spine pre-filters by versioned-object kind: {sql}"
    );
    assert!(sql.contains("LIMIT"), "the paging window survives: {sql}");
    // The dead-audit fix: nothing projects an audit field, so no audit join.
    assert!(
        !sql.contains(r#""audit""#),
        "no audit join without an audit-field projection: {sql}"
    );
}

/// The dead-root elision is CONDITIONAL: a root that carries its own node
/// condition (an archetype predicate) or is read by a data path keeps its
/// lateral — only a genuinely unreferenced root is elided.
#[test]
fn referenced_streaming_root_keeps_its_lateral() {
    // An archetype predicate on the root pins the root node row.
    let sql = build_sql_limited(
        "SELECT e/ehr_id/value, c/uid/value \
         FROM EHR e CONTAINS COMPOSITION c [openEHR-EHR-COMPOSITION.encounter.v1] \
         CONTAINS OBSERVATION o [openEHR-EHR-OBSERVATION.blood_pressure.v2] \
         WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude > 130",
    );
    assert_eq!(
        sql.matches("JOIN LATERAL").count(),
        3,
        "an archetype-constrained root keeps its lateral: {sql}"
    );
    // A projected data path from the root pins the root node row too.
    let sql = build_sql_limited(
        "SELECT c/context/start_time/value \
         FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o [openEHR-EHR-OBSERVATION.blood_pressure.v2] \
         WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude > 130",
    );
    assert!(
        sql.matches("JOIN LATERAL").count() >= 3,
        "a root-projecting query keeps the root lateral: {sql}"
    );
}

/// `ORDER BY` needs the full row set in a defined order — the flat shape
/// stays (no laterals).
#[test]
fn order_by_keeps_the_flat_shape() {
    let sql = build_sql_limited(&format!("{WARD_QUERY} ORDER BY c/uid/value"));
    assert!(!sql.contains("JOIN LATERAL"), "flat under ORDER BY: {sql}");
}

/// Without a LIMIT there is nothing to stream toward — flat.
#[test]
fn no_limit_keeps_the_flat_shape() {
    let sql = build_sql(WARD_QUERY);
    assert!(!sql.contains("JOIN LATERAL"), "flat without LIMIT: {sql}");
}

/// An EHR-scoped execution is already bounded by the `ehr_id` indexes — flat.
#[test]
fn ehr_scoped_execution_keeps_the_flat_shape() {
    let ir = plan_ok(WARD_QUERY);
    let ctx = SqlCtx {
        system_id: "sys.example.com".to_owned(),
        ehr_ids: vec![ferroehr::ids::EhrId(uuid::Uuid::nil())],
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: Some(50),
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    let sql = ferroehr::aql::sql::build(&ir, &Params::new(), &ctx)
        .unwrap_or_else(|e| panic!("SQL build failed: {e}"))
        .sql;
    assert!(!sql.contains("JOIN LATERAL"), "flat when EHR-scoped: {sql}");
}

/// Aggregates and DISTINCT consume the full row set — flat.
#[test]
fn aggregates_and_distinct_keep_the_flat_shape() {
    let agg = build_sql_limited("SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c");
    assert!(!agg.contains("JOIN LATERAL"), "flat under COUNT: {agg}");
    let distinct =
        build_sql_limited("SELECT DISTINCT c/uid/value FROM EHR e CONTAINS COMPOSITION c");
    assert!(
        !distinct.contains("JOIN LATERAL"),
        "flat under DISTINCT: {distinct}"
    );
}

/// OR-containment needs the disjunctive EXISTS machinery — falls back flat.
#[test]
fn or_containment_keeps_the_flat_shape() {
    let sql = build_sql_limited(
        "SELECT c/uid/value FROM COMPOSITION c CONTAINS \
         (OBSERVATION o [openEHR-EHR-OBSERVATION.blood_pressure.v2] OR \
          EVALUATION ev [openEHR-EHR-EVALUATION.problem_diagnosis.v1])",
    );
    assert!(
        !sql.contains("JOIN LATERAL"),
        "flat under OR containment: {sql}"
    );
}

/// The streaming shape keeps the population gate: a bare root (no EHR
/// variable) still joins the queryable gate; an EHR-linked root is covered
/// by its EHR alias.
#[test]
fn streaming_shape_keeps_the_population_gate() {
    let linked = build_sql_limited(WARD_QUERY);
    assert_eq!(
        linked.matches(r#""is_queryable" = $"#).count(),
        1,
        "exactly one gate: {linked}"
    );
    assert!(
        linked.contains(r#""e0"."is_queryable" = $"#),
        "the gate sits on the EHR alias: {linked}"
    );
    // A bare VO root projecting only `uid/value` needs no node row at all
    // (dead-root elision): the whole query is the spine + the gate join —
    // still the streaming shape (the spine drives), now with zero laterals.
    let bare = build_sql_limited("SELECT c/uid/value FROM COMPOSITION c");
    assert!(
        bare.contains(r#"FROM "version""#) && !bare.contains("JOIN LATERAL"),
        "a bare uid-only VO root streams as the spine alone: {bare}"
    );
    assert!(
        bare.contains(r#""is_queryable" = $"#),
        "the bare root keeps its queryable gate: {bare}"
    );
}

// ── coded-name node predicates (QUERY master03 §Node predicate) ──────────────

/// The name term-code shortcut decomposes into its canonical expansion:
/// `name/defining_code/code_string` AND `name/defining_code/terminology_id/value`
/// compared separately (the spec's own example pair, master03 §Node predicate).
#[test]
fn name_term_code_predicate_decomposes() {
    let ir = plan_ok(
        "SELECT o/name/value FROM COMPOSITION c CONTAINS \
         OBSERVATION o CONTAINS ELEMENT e[at0002, snomed_ct(3.1)::313267000]",
    );
    let dumped = format!("{ir:?}");
    assert!(
        dumped.contains("terminology: \"snomed_ct(3.1)\"")
            && dumped.contains("code: \"313267000\""),
        "parts decomposed (version suffix stays with the terminology): {dumped}"
    );
    // SQL shape: the two PROMOTED columns ANDed on the constrained node, never
    // a JSON probe — the decomposer writes both at commit (QUERY
    // master03-syntax §Node predicate).
    let sql = build_sql(
        "SELECT o/name/value FROM COMPOSITION c CONTAINS \
         OBSERVATION o CONTAINS ELEMENT e[at0002, snomed_ct(3.1)::313267000]",
    );
    assert!(
        sql.contains(r#"("n2"."name_code" = $"#) && sql.contains(r#""n2"."name_terminology" = $"#),
        "code_string AND terminology_id both compared as promoted columns: {sql}"
    );
    assert!(
        !sql.contains(r#"jsonb_path_query_first("n2"."data""#),
        "the coded-name predicate opens no JSON fragment: {sql}"
    );
}

/// The informational `|value|` tail takes no part in matching — only the
/// terminology and code land in the IR (master03: `icd10AM::F60.1|Schizoid
/// personality disorder|`).
#[test]
fn name_term_code_informational_tail_dropped() {
    let ir = plan_ok(
        "SELECT e FROM COMPOSITION c CONTAINS \
         ELEMENT e[at0003, icd10AM::F60.1|Schizoid personality disorder|]",
    );
    let dumped = format!("{ir:?}");
    assert!(
        dumped.contains("terminology: \"icd10AM\"") && dumped.contains("code: \"F60.1\""),
        "parts decomposed: {dumped}"
    );
    assert!(
        !dumped.contains("Schizoid"),
        "informational tail dropped: {dumped}"
    );
}

/// A bare at-code name operand is a term from the archetype's own terminology:
/// the canonical expansion asserts `terminology_id/value = 'local'`
/// (master03 §Node predicate: `[at0002 and name/defining_code/code_string='at0003'
/// and name/defining_code/terminology_id/value='local']`).
#[test]
fn name_at_code_asserts_local_terminology() {
    let ir = plan_ok("SELECT e FROM COMPOSITION c CONTAINS ELEMENT e[at0002, at0003]");
    let dumped = format!("{ir:?}");
    assert!(
        dumped.contains("terminology: \"local\"") && dumped.contains("code: \"at0003\""),
        "local terminology asserted: {dumped}"
    );
}

/// `TOP n BACKWARD` is a typed reject carrying the spec's rewrite guidance
/// (owner disposition on #966: the deprecated direction variant is refused
/// loudly, never a silent first-n answer); plain `TOP n` and the default
/// FORWARD direction keep working (QUERY §SELECT/TOP — deprecated but defined).
#[test]
fn top_backward_rejected_with_guidance() {
    let e = plan_err("SELECT TOP 10 BACKWARD c/uid/value FROM COMPOSITION c");
    let msg = e.to_string();
    assert!(
        matches!(e, AqlError::Feature(AqlFeatureError::TopBackward(10))),
        "typed reject: {msg}"
    );
    assert!(
        msg.contains("deprecated") && msg.contains("ORDER BY") && msg.contains("LIMIT 10"),
        "the reject carries the rewrite guidance: {msg}"
    );
    // Plain TOP and the explicit default direction stay accepted.
    plan_ok("SELECT TOP 10 c/uid/value FROM COMPOSITION c");
    plan_ok("SELECT TOP 10 FORWARD c/uid/value FROM COMPOSITION c");
}

// ── coded version-field sub-paths (#976) ─────────────────────────────────────

/// The coded version fields are sub-path-sensitive: `defining_code/code_string`
/// reads the stored code column, `value` renders the rubric via a CASE over
/// the openEHR terminology group, `defining_code/terminology_id/value` is the
/// `openehr` constant — and any other suffix (incl. the bare coded object) is
/// a typed reject.
#[test]
fn version_coded_field_subpaths() {
    let code = build_sql(
        "SELECT v/uid/value FROM VERSION v[commit_audit/change_type/defining_code/code_string='249'] \
         CONTAINS COMPOSITION c",
    );
    assert!(
        code.contains(r#""change_type" = CAST"#),
        "the code form compares the stored column directly: {code}"
    );

    let rubric = build_sql(
        "SELECT v/uid/value FROM VERSION v[commit_audit/change_type/value='creation'] \
         CONTAINS COMPOSITION c",
    );
    assert!(
        rubric.contains("CASE") && rubric.contains(r#""change_type""#),
        "rubric renders as a CASE over the group: {rubric}"
    );

    let lifecycle_rubric = build_sql(
        "SELECT v/uid/value FROM VERSION v[lifecycle_state/value='complete'] CONTAINS COMPOSITION c",
    );
    assert!(
        lifecycle_rubric.contains("CASE") && lifecycle_rubric.contains(r#""lifecycle_state""#),
        "lifecycle rubric CASE: {lifecycle_rubric}"
    );

    let term = build_sql(
        "SELECT v/uid/value FROM VERSION \
         v[commit_audit/change_type/defining_code/terminology_id/value='openehr'] \
         CONTAINS COMPOSITION c",
    );
    // The terminology id is a bound constant on both sides — no column joins
    // the comparison at all.
    assert!(
        !term.contains(r#""change_type""#) && term.contains("= CAST"),
        "terminology form is a constant comparison: {term}"
    );

    // The bare coded object has no defined scalar comparison form.
    let e = plan_err(
        "SELECT v/uid/value FROM VERSION v[commit_audit/change_type='249'] CONTAINS COMPOSITION c",
    );
    assert!(
        matches!(
            e,
            AqlError::Feature(AqlFeatureError::UnsupportedVersionPredicate(_))
        ),
        "bare coded object rejects: {e}"
    );
}

/// #1448 — LIKE and `matches` on an anchored data leaf lower through the
/// EXISTENTIAL shape (`EXISTS(SELECT 1 …)`), the same lowering the comparison
/// operators use, never the scalar LIMIT-1 extraction (order-undefined on a
/// multi-valued path).
#[test]
fn like_and_matches_lower_existentially_on_anchored_leaves() {
    let like_sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/context/other_context[at0001]/items[at0002]/value/value LIKE 'x*'",
    );
    assert!(
        like_sql.contains("EXISTS(SELECT") && like_sql.contains("LIKE"),
        "LIKE on an anchored leaf is existential: {like_sql}"
    );
    assert!(
        !like_sql.contains("LIMIT 1"),
        "no scalar LIMIT-1 extraction remains under the LIKE: {like_sql}"
    );

    let matches_sql = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/context/other_context[at0001]/items[at0002]/value/value MATCHES {'a', 'b'}",
    );
    assert!(
        matches_sql.contains("EXISTS(SELECT") && matches_sql.contains("IN ("),
        "matches on an anchored leaf is existential: {matches_sql}"
    );
    // NOT flips the polarity back to the scalar shape (three-valued SQL
    // semantics preserved for absent leaves) — same rule as the comparisons.
    let negated = build_sql(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE NOT c/context/other_context[at0001]/items[at0002]/value/value LIKE 'x*'",
    );
    assert!(
        anchor_exists(&negated) == 0,
        "negative polarity keeps the scalar lowering: {negated}"
    );
}

/// The stable-profile AQL gate: refusal machinery + the no-false-positive
/// property.
///
/// The ENTIRE RM 1.1.0 → 1.2.0 model delta is `EHR.tags` (verified first-hand
/// against the vendored BMMs, 2026-08-05: no 1.2.0-only classes, one
/// 1.2.0-only attribute), and `EHR` paths resolve through the fixed
/// [`EhrField`] set that does not include `tags` — so no CURRENTLY-plannable
/// query diverges between the profiles. The gate is the safety net that
/// makes that stay true by construction when the envelope grows: it is
/// pinned here at the membership level, plus the property that stable
/// planning refuses nothing development accepts today.
#[test]
fn stable_profile_gate_matches_the_released_model() {
    use ferroehr::config::profile::SpecProfile;

    // The one model-level delta, straight from the generated v1_1 model:
    // EHR.tags exists in 1.2.0, not in 1.1.0.
    assert!(openehr_rm::v1_2::model::attribute("EHR", "tags").is_some());
    assert!(openehr_rm::v1_1::model::attribute("EHR", "tags").is_none());

    // No false positives: every query in this suite's envelope plans the
    // same under both profiles.
    for q in [
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
        "SELECT f/details FROM EHR e CONTAINS FOLDER f",
        "SELECT o/data/events/data/items/value/magnitude FROM EHR e \
         CONTAINS COMPOSITION c CONTAINS OBSERVATION o",
    ] {
        let ast = parse_str(q).expect("parses");
        assert!(
            plan(&ast, &Params::new(), SpecProfile::Stable).is_ok(),
            "stable must not refuse RM 1.1.0 surface: {q}"
        );
        assert!(
            plan(&ast, &Params::new(), SpecProfile::Development).is_ok(),
            "development baseline: {q}"
        );
    }
}

// ── containment-edge classification (#2880) ──────────────────────────────────

/// Plan `q` and expect the SQL BUILD (not planning) to refuse it.
fn build_err(q: &str) -> AqlError {
    let ir = plan_ok(q);
    let ctx = SqlCtx {
        system_id: "sys.example.com".to_owned(),
        ehr_ids: Vec::new(),
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    ferroehr::aql::sql::build(&ir, &Params::new(), &ctx)
        .err()
        .unwrap_or_else(|| panic!("expected the SQL build to refuse {q:?}"))
}

/// `FOLDER CONTAINS COMPOSITION` resolves through `FOLDER.items` `OBJECT_REF`s
/// (RM common master05 — folders hold references to versioned objects), over
/// the whole folder subtree: the ref uid roots are compared against the
/// child's `vo_id`.
#[test]
fn folder_contains_composition_resolves_items_refs() {
    let sql = build_sql("SELECT c/uid/value FROM EHR e CONTAINS FOLDER f CONTAINS COMPOSITION c");
    assert!(
        sql.contains("jsonb_array_elements") && sql.contains("'items'"),
        "the edge is the items reference lookup: {sql}"
    );
    assert!(
        sql.contains("split_part"),
        "the ref uid root is compared, not the full OBJECT_VERSION_ID: {sql}"
    );
}

/// `FOLDER CONTAINS FOLDER` is the union edge (#2887): a by-value STRICT
/// descendant (or every folder would trivially contain itself) OR any folder
/// row of an items-referenced `VERSIONED_FOLDER` (RM common master05 — the
/// `items` references name versioned objects "logically in this folder",
/// with no target-type restriction).
#[test]
fn folder_contains_folder_unites_subtree_and_items_reference() {
    let sql = build_sql("SELECT f2/name/value FROM EHR e CONTAINS FOLDER f1 CONTAINS FOLDER f2");
    assert!(
        sql.contains(r#""n2"."num" > "n1"."num""#)
            && sql.contains(r#""n2"."num" <= "n1"."num_cap""#),
        "the by-value branch is a strict interval, no self-pair: {sql}"
    );
    assert!(
        sql.contains(") OR EXISTS") || sql.contains(") OR (EXISTS"),
        "the by-value branch is united with the items-reference branch: {sql}"
    );
    assert!(
        sql.contains("jsonb_array_elements") && sql.contains("'items'"),
        "the reference branch is the items lookup: {sql}"
    );
}

/// A `CONTAINS` pair the RM defines no containment relationship for is a
/// typed refusal, never a silent cartesian (the pre-#2880 behaviour).
#[test]
fn versioned_object_under_versioned_object_is_refused() {
    let err =
        build_err("SELECT c2/uid/value FROM EHR e CONTAINS COMPOSITION c1 CONTAINS COMPOSITION c2");
    assert!(
        err.to_string().contains("no RM containment relationship"),
        "typed containment refusal, got: {err}"
    );
}

/// `FOLDER f1 NOT CONTAINS FOLDER f2` negates the union edge (#2887): the
/// anti-join's branch carries its own version spine and the by-value strict
/// interval OR the items-reference lookup — the same shape the positive edge
/// renders.
#[test]
fn not_contains_folder_negates_the_union_edge() {
    let sql =
        build_sql("SELECT f1/name/value FROM EHR e CONTAINS FOLDER f1 NOT CONTAINS FOLDER f2");
    assert!(
        sql.contains("NOT EXISTS") || sql.contains("NOT (EXISTS"),
        "the exclusion is an anti-join: {sql}"
    );
    assert!(
        sql.contains(") OR EXISTS") || sql.contains(") OR (EXISTS"),
        "the negated edge unites the by-value branch with the reference branch: {sql}"
    );
    assert!(
        sql.contains("jsonb_array_elements") && sql.contains("'items'"),
        "the reference branch is the items lookup: {sql}"
    );
    assert!(
        sql.contains(r#""xv1"."sys_version" = "xv1_head"."trunk_head_sys_version""#),
        "the branch folder binds its own version spine at latest scope: {sql}"
    );
}

/// `FOLDER f NOT CONTAINS COMPOSITION` negates the same reference edge — the
/// anti-join probes `FOLDER.items`, not the (vacuously true) node interval.
#[test]
fn not_contains_under_folder_uses_the_reference_edge() {
    let sql =
        build_sql("SELECT f/name/value FROM EHR e CONTAINS FOLDER f NOT CONTAINS COMPOSITION c");
    assert!(
        sql.contains("NOT EXISTS") || sql.contains("NOT (EXISTS"),
        "the exclusion is an anti-join: {sql}"
    );
    assert!(
        sql.contains("jsonb_array_elements") && sql.contains("'items'"),
        "the negated edge is the items reference lookup: {sql}"
    );
}

/// A FOLDER root never takes the streaming shape: the streaming root binds
/// `num = 0`, but `EHR CONTAINS FOLDER` matches every folder node
/// (containment is transitive — QUERY master03 §Containment), so the flat
/// shape owns it.
#[test]
fn folder_root_never_streams() {
    let sql = build_sql_limited("SELECT f/name/value FROM EHR e CONTAINS FOLDER f");
    assert!(
        !sql.contains("JOIN LATERAL"),
        "FOLDER roots take the flat shape: {sql}"
    );
}
