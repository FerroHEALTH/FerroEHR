// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Traversal of the [`crate::ast`]: a read-only [`Visit`] and a mutating
//! [`VisitMut`], each exhaustive over every node type.
//!
//! Every trait method defaults to the matching `walk_*` function, which visits
//! the node's children in source order. A visitor overrides the methods for the
//! nodes it cares about and calls the `walk_*` function from the override when
//! it still wants the children. The walk functions destructure every struct and
//! match every enum without a wildcard, so a new AST field or variant fails to
//! compile here until the traversal covers it.
//!
//! # Examples
//!
//! Collect every path compared in the WHERE clause:
//!
//! ```
//! use openehr_query::ast::{CompareOperand, IdentifiedExpr};
//! use openehr_query::parser::parse_str;
//! use openehr_query::visit::{Visit, walk_identified_expr};
//!
//! #[derive(Default)]
//! struct Compared(Vec<String>);
//!
//! impl<'ast> Visit<'ast> for Compared {
//!     fn visit_identified_expr(&mut self, node: &'ast IdentifiedExpr) {
//!         if let IdentifiedExpr::Compare { lhs: CompareOperand::Path(p), .. } = node {
//!             self.0.push(p.column_path_text());
//!         }
//!         walk_identified_expr(self, node);
//!     }
//! }
//!
//! let query = parse_str("SELECT e/ehr_id/value FROM EHR e WHERE e/time_created/value > '2026'")?;
//! let mut compared = Compared::default();
//! compared.visit_select_query(&query);
//! assert_eq!(compared.0, ["/time_created/value"]);
//! # Ok::<(), openehr_query::parser::ParseError>(())
//! ```

use crate::ast::{
    AggregateCall, ArchetypePredicate, ClassExprOperand, ColumnExpr, CompareOperand,
    ContainsConstraint, ContainsExpr, FunctionCall, IdentifiedExpr, IdentifiedPath, LikeOperand,
    Limit, MatchesOperand, NodeNameConstraint, NodePredicate, ObjectPath, OrderByExpr, PathPart,
    PathPredicate, PathPredicateOperand, Primitive, SelectClause, SelectExpr, SelectQuery, Span,
    StandardPredicate, Terminal, TerminologyFunction, Top, ValueListItem, VersionPredicate,
    WhereExpr,
};

/// A read-only traversal of the AST, borrowing each node for `'ast`.
///
/// Every method defaults to its `walk_*` function; see the module docs.
pub trait Visit<'ast> {
    /// Visits a whole query.
    fn visit_select_query(&mut self, node: &'ast SelectQuery) {
        walk_select_query(self, node);
    }
    /// Visits the `SELECT` clause.
    fn visit_select_clause(&mut self, node: &'ast SelectClause) {
        walk_select_clause(self, node);
    }
    /// Visits the deprecated `TOP` clause (a leaf).
    fn visit_top(&mut self, _node: &'ast Top) {}
    /// Visits one selected column.
    fn visit_select_expr(&mut self, node: &'ast SelectExpr) {
        walk_select_expr(self, node);
    }
    /// Visits the expression of a selected column.
    fn visit_column_expr(&mut self, node: &'ast ColumnExpr) {
        walk_column_expr(self, node);
    }
    /// Visits a `FROM` containment expression.
    fn visit_contains_expr(&mut self, node: &'ast ContainsExpr) {
        walk_contains_expr(self, node);
    }
    /// Visits a `[NOT] CONTAINS` constraint.
    fn visit_contains_constraint(&mut self, node: &'ast ContainsConstraint) {
        walk_contains_constraint(self, node);
    }
    /// Visits a class or `VERSION` operand of the `FROM` clause.
    fn visit_class_expr_operand(&mut self, node: &'ast ClassExprOperand) {
        walk_class_expr_operand(self, node);
    }
    /// Visits a `WHERE` boolean tree.
    fn visit_where_expr(&mut self, node: &'ast WhereExpr) {
        walk_where_expr(self, node);
    }
    /// Visits one `WHERE` condition.
    fn visit_identified_expr(&mut self, node: &'ast IdentifiedExpr) {
        walk_identified_expr(self, node);
    }
    /// Visits the left operand of a comparison.
    fn visit_compare_operand(&mut self, node: &'ast CompareOperand) {
        walk_compare_operand(self, node);
    }
    /// Visits a terminal (the right operand of a comparison, or a function
    /// argument).
    fn visit_terminal(&mut self, node: &'ast Terminal) {
        walk_terminal(self, node);
    }
    /// Visits one `ORDER BY` term.
    fn visit_order_by_expr(&mut self, node: &'ast OrderByExpr) {
        walk_order_by_expr(self, node);
    }
    /// Visits the `LIMIT` clause (a leaf).
    fn visit_limit(&mut self, _node: &'ast Limit) {}
    /// Visits an identified path.
    fn visit_identified_path(&mut self, node: &'ast IdentifiedPath) {
        walk_identified_path(self, node);
    }
    /// Visits an object path.
    fn visit_object_path(&mut self, node: &'ast ObjectPath) {
        walk_object_path(self, node);
    }
    /// Visits one part of an object path.
    fn visit_path_part(&mut self, node: &'ast PathPart) {
        walk_path_part(self, node);
    }
    /// Visits a `[…]` path predicate.
    fn visit_path_predicate(&mut self, node: &'ast PathPredicate) {
        walk_path_predicate(self, node);
    }
    /// Visits a standard predicate.
    fn visit_standard_predicate(&mut self, node: &'ast StandardPredicate) {
        walk_standard_predicate(self, node);
    }
    /// Visits an archetype predicate (a leaf).
    fn visit_archetype_predicate(&mut self, _node: &'ast ArchetypePredicate) {}
    /// Visits a node predicate.
    fn visit_node_predicate(&mut self, node: &'ast NodePredicate) {
        walk_node_predicate(self, node);
    }
    /// Visits the name constraint after a node code (a leaf).
    fn visit_node_name_constraint(&mut self, _node: &'ast NodeNameConstraint) {}
    /// Visits a `VERSION` predicate.
    fn visit_version_predicate(&mut self, node: &'ast VersionPredicate) {
        walk_version_predicate(self, node);
    }
    /// Visits the right operand of a predicate comparison.
    fn visit_path_predicate_operand(&mut self, node: &'ast PathPredicateOperand) {
        walk_path_predicate_operand(self, node);
    }
    /// Visits the operand of `LIKE` (a leaf).
    fn visit_like_operand(&mut self, _node: &'ast LikeOperand) {}
    /// Visits the operand of `MATCHES`.
    fn visit_matches_operand(&mut self, node: &'ast MatchesOperand) {
        walk_matches_operand(self, node);
    }
    /// Visits one item of a `MATCHES` value list.
    fn visit_value_list_item(&mut self, node: &'ast ValueListItem) {
        walk_value_list_item(self, node);
    }
    /// Visits a function call.
    fn visit_function_call(&mut self, node: &'ast FunctionCall) {
        walk_function_call(self, node);
    }
    /// Visits a `TERMINOLOGY(…)` call (a leaf).
    fn visit_terminology_function(&mut self, _node: &'ast TerminologyFunction) {}
    /// Visits an aggregate call.
    fn visit_aggregate_call(&mut self, node: &'ast AggregateCall) {
        walk_aggregate_call(self, node);
    }
    /// Visits a literal (a leaf).
    fn visit_primitive(&mut self, _node: &'ast Primitive) {}
    /// Visits the source position of the node that holds it (a leaf).
    fn visit_span(&mut self, _node: &'ast Span) {}
}

/// Visits the clauses of a query in source order.
pub fn walk_select_query<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast SelectQuery) {
    let SelectQuery {
        select,
        from,
        where_,
        order_by,
        limit,
    } = node;
    v.visit_select_clause(select);
    v.visit_contains_expr(from);
    if let Some(where_) = where_ {
        v.visit_where_expr(where_);
    }
    for term in order_by {
        v.visit_order_by_expr(term);
    }
    if let Some(limit) = limit {
        v.visit_limit(limit);
    }
}

/// Visits the `TOP` clause and every column.
pub fn walk_select_clause<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast SelectClause) {
    let SelectClause {
        distinct: _,
        top,
        columns,
    } = node;
    if let Some(top) = top {
        v.visit_top(top);
    }
    for column in columns {
        v.visit_select_expr(column);
    }
}

/// Visits the column expression.
pub fn walk_select_expr<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast SelectExpr) {
    let SelectExpr { column, alias: _ } = node;
    v.visit_column_expr(column);
}

/// Visits whichever expression the column is.
pub fn walk_column_expr<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast ColumnExpr) {
    match node {
        ColumnExpr::Path(path) => v.visit_identified_path(path),
        ColumnExpr::Primitive(primitive) => v.visit_primitive(primitive),
        ColumnExpr::Aggregate(aggregate) => v.visit_aggregate_call(aggregate),
        ColumnExpr::Function(function) => v.visit_function_call(function),
    }
}

/// Visits the operand and its constraint, or both sides of a boolean.
pub fn walk_contains_expr<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast ContainsExpr) {
    match node {
        ContainsExpr::Contained { operand, contains } => {
            v.visit_class_expr_operand(operand);
            if let Some(constraint) = contains {
                v.visit_contains_constraint(constraint);
            }
        }
        ContainsExpr::And(left, right) | ContainsExpr::Or(left, right) => {
            v.visit_contains_expr(left);
            v.visit_contains_expr(right);
        }
    }
}

/// Visits the contained expression.
pub fn walk_contains_constraint<'ast, V: Visit<'ast> + ?Sized>(
    v: &mut V,
    node: &'ast ContainsConstraint,
) {
    let ContainsConstraint { negated: _, expr } = node;
    v.visit_contains_expr(expr);
}

/// Visits the operand's predicate.
pub fn walk_class_expr_operand<'ast, V: Visit<'ast> + ?Sized>(
    v: &mut V,
    node: &'ast ClassExprOperand,
) {
    match node {
        ClassExprOperand::Class {
            rm_type: _,
            variable: _,
            predicate,
        } => {
            if let Some(predicate) = predicate {
                v.visit_path_predicate(predicate);
            }
        }
        ClassExprOperand::Version {
            variable: _,
            predicate,
        } => {
            if let Some(predicate) = predicate {
                v.visit_version_predicate(predicate);
            }
        }
    }
}

/// Visits a condition and its span, or the operands of a boolean.
pub fn walk_where_expr<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast WhereExpr) {
    match node {
        WhereExpr::Identified(expr, span) => {
            v.visit_identified_expr(expr);
            v.visit_span(span);
        }
        WhereExpr::Not(inner) => v.visit_where_expr(inner),
        WhereExpr::And(left, right) | WhereExpr::Or(left, right) => {
            v.visit_where_expr(left);
            v.visit_where_expr(right);
        }
    }
}

/// Visits the operands of a condition.
pub fn walk_identified_expr<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast IdentifiedExpr) {
    match node {
        IdentifiedExpr::Exists(path) => v.visit_identified_path(path),
        IdentifiedExpr::Compare { lhs, op: _, rhs } => {
            v.visit_compare_operand(lhs);
            v.visit_terminal(rhs);
        }
        IdentifiedExpr::Like { path, operand } => {
            v.visit_identified_path(path);
            v.visit_like_operand(operand);
        }
        IdentifiedExpr::Matches { path, operand } => {
            v.visit_identified_path(path);
            v.visit_matches_operand(operand);
        }
        IdentifiedExpr::Resolved(_) => {}
    }
}

/// Visits the path or the function call.
pub fn walk_compare_operand<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast CompareOperand) {
    match node {
        CompareOperand::Path(path) => v.visit_identified_path(path),
        CompareOperand::Function(function) => v.visit_function_call(function),
    }
}

/// Visits whichever node the terminal is; a parameter is a leaf.
pub fn walk_terminal<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast Terminal) {
    match node {
        Terminal::Primitive(primitive) => v.visit_primitive(primitive),
        Terminal::Parameter(_) => {}
        Terminal::Path(path) => v.visit_identified_path(path),
        Terminal::Function(function) => v.visit_function_call(function),
    }
}

/// Visits the ordering path.
pub fn walk_order_by_expr<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast OrderByExpr) {
    let OrderByExpr { path, order: _ } = node;
    v.visit_identified_path(path);
}

/// Visits the root predicate, the object path and the span.
pub fn walk_identified_path<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast IdentifiedPath) {
    let IdentifiedPath {
        root: _,
        predicate,
        path,
        span,
    } = node;
    if let Some(predicate) = predicate {
        v.visit_path_predicate(predicate);
    }
    if let Some(path) = path {
        v.visit_object_path(path);
    }
    v.visit_span(span);
}

/// Visits every part.
pub fn walk_object_path<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast ObjectPath) {
    let ObjectPath { parts } = node;
    for part in parts {
        v.visit_path_part(part);
    }
}

/// Visits the part's predicate.
pub fn walk_path_part<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast PathPart) {
    let PathPart { name: _, predicate } = node;
    if let Some(predicate) = predicate {
        v.visit_path_predicate(predicate);
    }
}

/// Visits whichever predicate form this is.
pub fn walk_path_predicate<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast PathPredicate) {
    match node {
        PathPredicate::Standard(standard) => v.visit_standard_predicate(standard),
        PathPredicate::Archetype(archetype) => v.visit_archetype_predicate(archetype),
        PathPredicate::Node(node) => v.visit_node_predicate(node),
    }
}

/// Visits the constrained path and the operand.
pub fn walk_standard_predicate<'ast, V: Visit<'ast> + ?Sized>(
    v: &mut V,
    node: &'ast StandardPredicate,
) {
    let StandardPredicate {
        path,
        op: _,
        operand,
    } = node;
    v.visit_object_path(path);
    v.visit_path_predicate_operand(operand);
}

/// Visits the name constraint, the comparison, the path or both operands.
pub fn walk_node_predicate<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast NodePredicate) {
    match node {
        NodePredicate::Code { code: _, name } | NodePredicate::Archetype { hrid: _, name } => {
            if let Some(name) = name {
                v.visit_node_name_constraint(name);
            }
        }
        NodePredicate::Parameter(_) => {}
        NodePredicate::Standard(standard) => v.visit_standard_predicate(standard),
        NodePredicate::MatchesRegex { path, regex: _ } => v.visit_object_path(path),
        NodePredicate::And(left, right) | NodePredicate::Or(left, right) => {
            v.visit_node_predicate(left);
            v.visit_node_predicate(right);
        }
    }
}

/// Visits the standard predicate, when the version predicate is one.
pub fn walk_version_predicate<'ast, V: Visit<'ast> + ?Sized>(
    v: &mut V,
    node: &'ast VersionPredicate,
) {
    match node {
        VersionPredicate::Latest | VersionPredicate::All => {}
        VersionPredicate::Standard(standard) => v.visit_standard_predicate(standard),
    }
}

/// Visits the literal or the path; a parameter or a code is a leaf.
pub fn walk_path_predicate_operand<'ast, V: Visit<'ast> + ?Sized>(
    v: &mut V,
    node: &'ast PathPredicateOperand,
) {
    match node {
        PathPredicateOperand::Primitive(primitive) => v.visit_primitive(primitive),
        PathPredicateOperand::Path(path) => v.visit_object_path(path),
        PathPredicateOperand::Parameter(_) | PathPredicateOperand::Code(_) => {}
    }
}

/// Visits every list item, or the terminology call; a URI is a leaf.
pub fn walk_matches_operand<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast MatchesOperand) {
    match node {
        MatchesOperand::ValueList(items) => {
            for item in items {
                v.visit_value_list_item(item);
            }
        }
        MatchesOperand::Terminology(terminology) => v.visit_terminology_function(terminology),
        MatchesOperand::Uri(_) => {}
    }
}

/// Visits the literal or the terminology call; a parameter is a leaf.
pub fn walk_value_list_item<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast ValueListItem) {
    match node {
        ValueListItem::Primitive(primitive) => v.visit_primitive(primitive),
        ValueListItem::Parameter(_) => {}
        ValueListItem::Terminology(terminology) => v.visit_terminology_function(terminology),
    }
}

/// Visits every argument, or the terminology call.
pub fn walk_function_call<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast FunctionCall) {
    match node {
        FunctionCall::Builtin { args, .. } | FunctionCall::Other { name: _, args } => {
            for arg in args {
                v.visit_terminal(arg);
            }
        }
        FunctionCall::Terminology(terminology) => v.visit_terminology_function(terminology),
    }
}

/// Visits the aggregated path, when there is one.
pub fn walk_aggregate_call<'ast, V: Visit<'ast> + ?Sized>(v: &mut V, node: &'ast AggregateCall) {
    match node {
        AggregateCall::Count { distinct: _, path } => {
            if let Some(path) = path {
                v.visit_identified_path(path);
            }
        }
        AggregateCall::Stat { func: _, path } => v.visit_identified_path(path),
    }
}

/// A mutating traversal of the AST.
///
/// The mirror of [`Visit`] over `&mut` nodes: an override may rewrite or
/// replace the node it is handed (`*node = …`), and decides whether the
/// children of the result are walked. Removing a `WHERE` condition is a
/// rewrite of its parent: replace the `And` holding it with the other operand.
pub trait VisitMut {
    /// Visits a whole query.
    fn visit_select_query_mut(&mut self, node: &mut SelectQuery) {
        walk_select_query_mut(self, node);
    }
    /// Visits the `SELECT` clause.
    fn visit_select_clause_mut(&mut self, node: &mut SelectClause) {
        walk_select_clause_mut(self, node);
    }
    /// Visits the deprecated `TOP` clause (a leaf).
    fn visit_top_mut(&mut self, _node: &mut Top) {}
    /// Visits one selected column.
    fn visit_select_expr_mut(&mut self, node: &mut SelectExpr) {
        walk_select_expr_mut(self, node);
    }
    /// Visits the expression of a selected column.
    fn visit_column_expr_mut(&mut self, node: &mut ColumnExpr) {
        walk_column_expr_mut(self, node);
    }
    /// Visits a `FROM` containment expression.
    fn visit_contains_expr_mut(&mut self, node: &mut ContainsExpr) {
        walk_contains_expr_mut(self, node);
    }
    /// Visits a `[NOT] CONTAINS` constraint.
    fn visit_contains_constraint_mut(&mut self, node: &mut ContainsConstraint) {
        walk_contains_constraint_mut(self, node);
    }
    /// Visits a class or `VERSION` operand of the `FROM` clause.
    fn visit_class_expr_operand_mut(&mut self, node: &mut ClassExprOperand) {
        walk_class_expr_operand_mut(self, node);
    }
    /// Visits a `WHERE` boolean tree.
    fn visit_where_expr_mut(&mut self, node: &mut WhereExpr) {
        walk_where_expr_mut(self, node);
    }
    /// Visits one `WHERE` condition.
    fn visit_identified_expr_mut(&mut self, node: &mut IdentifiedExpr) {
        walk_identified_expr_mut(self, node);
    }
    /// Visits the left operand of a comparison.
    fn visit_compare_operand_mut(&mut self, node: &mut CompareOperand) {
        walk_compare_operand_mut(self, node);
    }
    /// Visits a terminal (the right operand of a comparison, or a function
    /// argument).
    fn visit_terminal_mut(&mut self, node: &mut Terminal) {
        walk_terminal_mut(self, node);
    }
    /// Visits one `ORDER BY` term.
    fn visit_order_by_expr_mut(&mut self, node: &mut OrderByExpr) {
        walk_order_by_expr_mut(self, node);
    }
    /// Visits the `LIMIT` clause (a leaf).
    fn visit_limit_mut(&mut self, _node: &mut Limit) {}
    /// Visits an identified path.
    fn visit_identified_path_mut(&mut self, node: &mut IdentifiedPath) {
        walk_identified_path_mut(self, node);
    }
    /// Visits an object path.
    fn visit_object_path_mut(&mut self, node: &mut ObjectPath) {
        walk_object_path_mut(self, node);
    }
    /// Visits one part of an object path.
    fn visit_path_part_mut(&mut self, node: &mut PathPart) {
        walk_path_part_mut(self, node);
    }
    /// Visits a `[…]` path predicate.
    fn visit_path_predicate_mut(&mut self, node: &mut PathPredicate) {
        walk_path_predicate_mut(self, node);
    }
    /// Visits a standard predicate.
    fn visit_standard_predicate_mut(&mut self, node: &mut StandardPredicate) {
        walk_standard_predicate_mut(self, node);
    }
    /// Visits an archetype predicate (a leaf).
    fn visit_archetype_predicate_mut(&mut self, _node: &mut ArchetypePredicate) {}
    /// Visits a node predicate.
    fn visit_node_predicate_mut(&mut self, node: &mut NodePredicate) {
        walk_node_predicate_mut(self, node);
    }
    /// Visits the name constraint after a node code (a leaf).
    fn visit_node_name_constraint_mut(&mut self, _node: &mut NodeNameConstraint) {}
    /// Visits a `VERSION` predicate.
    fn visit_version_predicate_mut(&mut self, node: &mut VersionPredicate) {
        walk_version_predicate_mut(self, node);
    }
    /// Visits the right operand of a predicate comparison.
    fn visit_path_predicate_operand_mut(&mut self, node: &mut PathPredicateOperand) {
        walk_path_predicate_operand_mut(self, node);
    }
    /// Visits the operand of `LIKE` (a leaf).
    fn visit_like_operand_mut(&mut self, _node: &mut LikeOperand) {}
    /// Visits the operand of `MATCHES`.
    fn visit_matches_operand_mut(&mut self, node: &mut MatchesOperand) {
        walk_matches_operand_mut(self, node);
    }
    /// Visits one item of a `MATCHES` value list.
    fn visit_value_list_item_mut(&mut self, node: &mut ValueListItem) {
        walk_value_list_item_mut(self, node);
    }
    /// Visits a function call.
    fn visit_function_call_mut(&mut self, node: &mut FunctionCall) {
        walk_function_call_mut(self, node);
    }
    /// Visits a `TERMINOLOGY(…)` call (a leaf).
    fn visit_terminology_function_mut(&mut self, _node: &mut TerminologyFunction) {}
    /// Visits an aggregate call.
    fn visit_aggregate_call_mut(&mut self, node: &mut AggregateCall) {
        walk_aggregate_call_mut(self, node);
    }
    /// Visits a literal (a leaf).
    fn visit_primitive_mut(&mut self, _node: &mut Primitive) {}
    /// Visits the source position of the node that holds it (a leaf).
    fn visit_span_mut(&mut self, _node: &mut Span) {}
}

/// Visits the clauses of a query in source order.
pub fn walk_select_query_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut SelectQuery) {
    let SelectQuery {
        select,
        from,
        where_,
        order_by,
        limit,
    } = node;
    v.visit_select_clause_mut(select);
    v.visit_contains_expr_mut(from);
    if let Some(where_) = where_ {
        v.visit_where_expr_mut(where_);
    }
    for term in order_by {
        v.visit_order_by_expr_mut(term);
    }
    if let Some(limit) = limit {
        v.visit_limit_mut(limit);
    }
}

/// Visits the `TOP` clause and every column.
pub fn walk_select_clause_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut SelectClause) {
    let SelectClause {
        distinct: _,
        top,
        columns,
    } = node;
    if let Some(top) = top {
        v.visit_top_mut(top);
    }
    for column in columns {
        v.visit_select_expr_mut(column);
    }
}

/// Visits the column expression.
pub fn walk_select_expr_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut SelectExpr) {
    let SelectExpr { column, alias: _ } = node;
    v.visit_column_expr_mut(column);
}

/// Visits whichever expression the column is.
pub fn walk_column_expr_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut ColumnExpr) {
    match node {
        ColumnExpr::Path(path) => v.visit_identified_path_mut(path),
        ColumnExpr::Primitive(primitive) => v.visit_primitive_mut(primitive),
        ColumnExpr::Aggregate(aggregate) => v.visit_aggregate_call_mut(aggregate),
        ColumnExpr::Function(function) => v.visit_function_call_mut(function),
    }
}

/// Visits the operand and its constraint, or both sides of a boolean.
pub fn walk_contains_expr_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut ContainsExpr) {
    match node {
        ContainsExpr::Contained { operand, contains } => {
            v.visit_class_expr_operand_mut(operand);
            if let Some(constraint) = contains {
                v.visit_contains_constraint_mut(constraint);
            }
        }
        ContainsExpr::And(left, right) | ContainsExpr::Or(left, right) => {
            v.visit_contains_expr_mut(left);
            v.visit_contains_expr_mut(right);
        }
    }
}

/// Visits the contained expression.
pub fn walk_contains_constraint_mut<V: VisitMut + ?Sized>(
    v: &mut V,
    node: &mut ContainsConstraint,
) {
    let ContainsConstraint { negated: _, expr } = node;
    v.visit_contains_expr_mut(expr);
}

/// Visits the operand's predicate.
pub fn walk_class_expr_operand_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut ClassExprOperand) {
    match node {
        ClassExprOperand::Class {
            rm_type: _,
            variable: _,
            predicate,
        } => {
            if let Some(predicate) = predicate {
                v.visit_path_predicate_mut(predicate);
            }
        }
        ClassExprOperand::Version {
            variable: _,
            predicate,
        } => {
            if let Some(predicate) = predicate {
                v.visit_version_predicate_mut(predicate);
            }
        }
    }
}

/// Visits a condition and its span, or the operands of a boolean.
pub fn walk_where_expr_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut WhereExpr) {
    match node {
        WhereExpr::Identified(expr, span) => {
            v.visit_identified_expr_mut(expr);
            v.visit_span_mut(span);
        }
        WhereExpr::Not(inner) => v.visit_where_expr_mut(inner),
        WhereExpr::And(left, right) | WhereExpr::Or(left, right) => {
            v.visit_where_expr_mut(left);
            v.visit_where_expr_mut(right);
        }
    }
}

/// Visits the operands of a condition.
pub fn walk_identified_expr_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut IdentifiedExpr) {
    match node {
        IdentifiedExpr::Exists(path) => v.visit_identified_path_mut(path),
        IdentifiedExpr::Compare { lhs, op: _, rhs } => {
            v.visit_compare_operand_mut(lhs);
            v.visit_terminal_mut(rhs);
        }
        IdentifiedExpr::Like { path, operand } => {
            v.visit_identified_path_mut(path);
            v.visit_like_operand_mut(operand);
        }
        IdentifiedExpr::Matches { path, operand } => {
            v.visit_identified_path_mut(path);
            v.visit_matches_operand_mut(operand);
        }
        IdentifiedExpr::Resolved(_) => {}
    }
}

/// Visits the path or the function call.
pub fn walk_compare_operand_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut CompareOperand) {
    match node {
        CompareOperand::Path(path) => v.visit_identified_path_mut(path),
        CompareOperand::Function(function) => v.visit_function_call_mut(function),
    }
}

/// Visits whichever node the terminal is; a parameter is a leaf.
pub fn walk_terminal_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut Terminal) {
    match node {
        Terminal::Primitive(primitive) => v.visit_primitive_mut(primitive),
        Terminal::Parameter(_) => {}
        Terminal::Path(path) => v.visit_identified_path_mut(path),
        Terminal::Function(function) => v.visit_function_call_mut(function),
    }
}

/// Visits the ordering path.
pub fn walk_order_by_expr_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut OrderByExpr) {
    let OrderByExpr { path, order: _ } = node;
    v.visit_identified_path_mut(path);
}

/// Visits the root predicate, the object path and the span.
pub fn walk_identified_path_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut IdentifiedPath) {
    let IdentifiedPath {
        root: _,
        predicate,
        path,
        span,
    } = node;
    if let Some(predicate) = predicate {
        v.visit_path_predicate_mut(predicate);
    }
    if let Some(path) = path {
        v.visit_object_path_mut(path);
    }
    v.visit_span_mut(span);
}

/// Visits every part.
pub fn walk_object_path_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut ObjectPath) {
    let ObjectPath { parts } = node;
    for part in parts {
        v.visit_path_part_mut(part);
    }
}

/// Visits the part's predicate.
pub fn walk_path_part_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut PathPart) {
    let PathPart { name: _, predicate } = node;
    if let Some(predicate) = predicate {
        v.visit_path_predicate_mut(predicate);
    }
}

/// Visits whichever predicate form this is.
pub fn walk_path_predicate_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut PathPredicate) {
    match node {
        PathPredicate::Standard(standard) => v.visit_standard_predicate_mut(standard),
        PathPredicate::Archetype(archetype) => v.visit_archetype_predicate_mut(archetype),
        PathPredicate::Node(node) => v.visit_node_predicate_mut(node),
    }
}

/// Visits the constrained path and the operand.
pub fn walk_standard_predicate_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut StandardPredicate) {
    let StandardPredicate {
        path,
        op: _,
        operand,
    } = node;
    v.visit_object_path_mut(path);
    v.visit_path_predicate_operand_mut(operand);
}

/// Visits the name constraint, the comparison, the path or both operands.
pub fn walk_node_predicate_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut NodePredicate) {
    match node {
        NodePredicate::Code { code: _, name } | NodePredicate::Archetype { hrid: _, name } => {
            if let Some(name) = name {
                v.visit_node_name_constraint_mut(name);
            }
        }
        NodePredicate::Parameter(_) => {}
        NodePredicate::Standard(standard) => v.visit_standard_predicate_mut(standard),
        NodePredicate::MatchesRegex { path, regex: _ } => v.visit_object_path_mut(path),
        NodePredicate::And(left, right) | NodePredicate::Or(left, right) => {
            v.visit_node_predicate_mut(left);
            v.visit_node_predicate_mut(right);
        }
    }
}

/// Visits the standard predicate, when the version predicate is one.
pub fn walk_version_predicate_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut VersionPredicate) {
    match node {
        VersionPredicate::Latest | VersionPredicate::All => {}
        VersionPredicate::Standard(standard) => v.visit_standard_predicate_mut(standard),
    }
}

/// Visits the literal or the path; a parameter or a code is a leaf.
pub fn walk_path_predicate_operand_mut<V: VisitMut + ?Sized>(
    v: &mut V,
    node: &mut PathPredicateOperand,
) {
    match node {
        PathPredicateOperand::Primitive(primitive) => v.visit_primitive_mut(primitive),
        PathPredicateOperand::Path(path) => v.visit_object_path_mut(path),
        PathPredicateOperand::Parameter(_) | PathPredicateOperand::Code(_) => {}
    }
}

/// Visits every list item, or the terminology call; a URI is a leaf.
pub fn walk_matches_operand_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut MatchesOperand) {
    match node {
        MatchesOperand::ValueList(items) => {
            for item in items {
                v.visit_value_list_item_mut(item);
            }
        }
        MatchesOperand::Terminology(terminology) => v.visit_terminology_function_mut(terminology),
        MatchesOperand::Uri(_) => {}
    }
}

/// Visits the literal or the terminology call; a parameter is a leaf.
pub fn walk_value_list_item_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut ValueListItem) {
    match node {
        ValueListItem::Primitive(primitive) => v.visit_primitive_mut(primitive),
        ValueListItem::Parameter(_) => {}
        ValueListItem::Terminology(terminology) => v.visit_terminology_function_mut(terminology),
    }
}

/// Visits every argument, or the terminology call.
pub fn walk_function_call_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut FunctionCall) {
    match node {
        FunctionCall::Builtin { args, .. } | FunctionCall::Other { name: _, args } => {
            for arg in args {
                v.visit_terminal_mut(arg);
            }
        }
        FunctionCall::Terminology(terminology) => v.visit_terminology_function_mut(terminology),
    }
}

/// Visits the aggregated path, when there is one.
pub fn walk_aggregate_call_mut<V: VisitMut + ?Sized>(v: &mut V, node: &mut AggregateCall) {
    match node {
        AggregateCall::Count { distinct: _, path } => {
            if let Some(path) = path {
                v.visit_identified_path_mut(path);
            }
        }
        AggregateCall::Stat { func: _, path } => v.visit_identified_path_mut(path),
    }
}
