// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AST visitors: they reach every node of every corpus query, an override
//! that changes nothing changes nothing, and a rewrite through `VisitMut`
//! replaces one condition and removes another.

#![allow(
    clippy::panic,
    clippy::panic_in_result_fn,
    reason = "integration-test assertions inside Result-returning tests, the shape testing.md prefers"
)]

use openehr_query::ast::{
    ArchetypePredicate, CompareOperand, IdentifiedExpr, IdentifiedPath, LikeOperand,
    NodeNameConstraint, NodePredicate, PathPredicateOperand, Terminal, ValueListItem, WhereExpr,
};
use openehr_query::lexer::{Token, lex};
use openehr_query::parser::{ParseError, parse_str};
use openehr_query::printer::to_aql;
use openehr_query::visit::{Visit, VisitMut, walk_where_expr_mut};

use crate::corpus::standard_queries;

/// Counts every parameter occurrence, at every position the grammar admits one.
#[derive(Default)]
struct Parameters(usize);

impl<'ast> Visit<'ast> for Parameters {
    fn visit_terminal(&mut self, node: &'ast Terminal) {
        if matches!(node, Terminal::Parameter(_)) {
            self.0 += 1;
        }
        openehr_query::visit::walk_terminal(self, node);
    }
    fn visit_path_predicate_operand(&mut self, node: &'ast PathPredicateOperand) {
        if matches!(node, PathPredicateOperand::Parameter(_)) {
            self.0 += 1;
        }
        openehr_query::visit::walk_path_predicate_operand(self, node);
    }
    fn visit_value_list_item(&mut self, node: &'ast ValueListItem) {
        if matches!(node, ValueListItem::Parameter(_)) {
            self.0 += 1;
        }
        openehr_query::visit::walk_value_list_item(self, node);
    }
    fn visit_like_operand(&mut self, node: &'ast LikeOperand) {
        if matches!(node, LikeOperand::Parameter(_)) {
            self.0 += 1;
        }
    }
    fn visit_node_name_constraint(&mut self, node: &'ast NodeNameConstraint) {
        if matches!(node, NodeNameConstraint::Parameter(_)) {
            self.0 += 1;
        }
    }
    fn visit_archetype_predicate(&mut self, node: &'ast ArchetypePredicate) {
        if matches!(node, ArchetypePredicate::Parameter(_)) {
            self.0 += 1;
        }
    }
    fn visit_node_predicate(&mut self, node: &'ast NodePredicate) {
        if matches!(node, NodePredicate::Parameter(_)) {
            self.0 += 1;
        }
        openehr_query::visit::walk_node_predicate(self, node);
    }
}

/// Counts every identified path.
#[derive(Default)]
struct Paths(usize);

impl<'ast> Visit<'ast> for Paths {
    fn visit_identified_path(&mut self, node: &'ast IdentifiedPath) {
        self.0 += 1;
        openehr_query::visit::walk_identified_path(self, node);
    }
}

/// A rewrite that overrides nothing.
struct Untouched;

impl VisitMut for Untouched {}

#[test]
fn the_visitor_reaches_every_parameter_the_source_carries() -> Result<(), ParseError> {
    let mut checked = 0;
    for src in standard_queries() {
        let query = parse_str(&src)?;
        let written = lex(&src)?
            .iter()
            .filter(|t| matches!(t, Token::Parameter(_)))
            .count();
        let mut reached = Parameters::default();
        reached.visit_select_query(&query);
        assert_eq!(reached.0, written, "parameters reached in: {src}");
        checked += written;
    }
    assert!(checked > 0, "the corpus carries parameters to reach");
    Ok(())
}

#[test]
fn every_path_of_a_query_is_reached_once() -> Result<(), ParseError> {
    let query = parse_str(
        "SELECT c/uid/value, COUNT(o/data), MAX(o/x/magnitude) \
         FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
         WHERE EXISTS c/context AND o/a LIKE 'x*' AND o/b MATCHES {1, 2} \
         AND length(o/c/value) > o/d/magnitude ORDER BY c/name/value",
    )?;
    let mut paths = Paths::default();
    paths.visit_select_query(&query);
    // c/uid, o/data, o/x, c/context, o/a, o/b, o/c (function argument),
    // o/d (terminal), c/name (ORDER BY).
    assert_eq!(paths.0, 9);
    Ok(())
}

#[test]
fn a_rewrite_that_overrides_nothing_leaves_every_corpus_query_unchanged() -> Result<(), ParseError>
{
    for src in standard_queries() {
        let original = parse_str(&src)?;
        let mut rewritten = original.clone();
        Untouched.visit_select_query_mut(&mut rewritten);
        assert_eq!(rewritten, original, "untouched rewrite of: {src}");
        assert_eq!(to_aql(&rewritten), to_aql(&original));
    }
    Ok(())
}

/// Replaces the subject predicate with an `ehr_id` scope, and drops the
/// namespace predicate beside it.
struct Rescope {
    scope: WhereExpr,
}

fn compares_path(expr: &WhereExpr, text: &str) -> bool {
    matches!(
        expr,
        WhereExpr::Identified(IdentifiedExpr::Compare { lhs: CompareOperand::Path(p), .. }, _)
            if p.column_path_text() == text
    )
}

impl VisitMut for Rescope {
    fn visit_where_expr_mut(&mut self, node: &mut WhereExpr) {
        if let WhereExpr::And(left, right) = node
            && compares_path(right, "/ehr_status/subject/external_ref/namespace")
        {
            let kept = std::mem::replace(
                left.as_mut(),
                WhereExpr::identified(IdentifiedExpr::Resolved(true)),
            );
            *node = kept;
        }
        if compares_path(node, "/ehr_status/subject/external_ref/id/value") {
            *node = self.scope.clone();
        }
        walk_where_expr_mut(self, node);
    }
}

#[test]
fn a_rewrite_replaces_one_condition_and_removes_another() -> Result<(), ParseError> {
    let scope = parse_str(
        "SELECT e FROM EHR e WHERE e/ehr_id/value = '7d44b88c-4199-4bad-97dc-d78268e01398'",
    )?
    .where_
    .ok_or_else(|| ParseError::Syntax { faults: Vec::new() })?;
    let mut query = parse_str(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE e/ehr_status/subject/external_ref/id/value = '12345' \
         AND e/ehr_status/subject/external_ref/namespace = 'nhs' \
         AND c/name/value = 'Vitals'",
    )?;
    Rescope { scope }.visit_select_query_mut(&mut query);
    let expected = parse_str(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
         WHERE e/ehr_id/value = '7d44b88c-4199-4bad-97dc-d78268e01398' AND c/name/value = 'Vitals'",
    )?;
    assert_eq!(query, expected);
    assert!(!to_aql(&query).contains("12345"));
    Ok(())
}
