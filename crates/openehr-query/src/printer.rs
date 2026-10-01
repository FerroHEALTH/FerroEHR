// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Canonical AQL text rendering of the [`crate::ast`].
//!
//! The inverse of the parser, for programmatic query construction. Keywords
//! render in canonical uppercase; paths and predicates reuse the
//! source-preserving `Display` impls in [`crate::ast`]. The corpus-verified
//! invariant: for every AST the parser produces, `parse(to_aql(ast)) == ast`
//! (grammar authority: `docs/specs/openehr/QUERY/docs/AQL/`).

use std::fmt::Write;

use crate::ast::{
    AggregateCall, ClassExprOperand, ColumnExpr, CompareOperand, ContainsConstraint, ContainsExpr,
    FunctionCall, IdentifiedExpr, IdentifiedPath, LikeOperand, MatchesOperand, SelectQuery,
    StatFunc, Terminal, TerminologyFunction, TopDirection, ValueListItem, VersionPredicate,
    WhereExpr, comp_op_text,
};

/// Render a whole query as canonical AQL text.
#[must_use]
pub fn to_aql(query: &SelectQuery) -> String {
    render(query, None)
}

/// Renders `query`, with `from_prefix` written between `FROM` and the
/// containment expression when given.
pub(crate) fn render(query: &SelectQuery, from_prefix: Option<&str>) -> String {
    let mut out = String::new();
    select_clause(&mut out, query);
    out.push_str(" FROM ");
    if let Some(prefix) = from_prefix {
        out.push_str(prefix);
    }
    contains_expr(&mut out, &query.from, ContainsCtx::Top);
    if let Some(where_) = &query.where_ {
        out.push_str(" WHERE ");
        where_expr(&mut out, where_, WhereCtx::Top);
    }
    order_by_clause(&mut out, query);
    if let Some(limit) = &query.limit {
        let _ = write!(out, " LIMIT {}", limit.limit);
        if let Some(offset) = limit.offset {
            let _ = write!(out, " OFFSET {offset}");
        }
    }
    out
}

/// The `ORDER BY` clause, with each term's explicit direction where the query
/// states one.
fn order_by_clause(out: &mut String, query: &SelectQuery) {
    if query.order_by.is_empty() {
        return;
    }
    out.push_str(" ORDER BY ");
    for (i, ob) in query.order_by.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        identified_path(out, &ob.path);
        if let Some(order) = ob.order {
            out.push_str(match order {
                crate::ast::SortOrder::Ascending => " ASC",
                crate::ast::SortOrder::Descending => " DESC",
            });
        }
    }
}

/// Escapes a raw string for embedding in an AQL single-quoted literal, per
/// `AqlLexer.g4 ESCAPE_SEQ`.
///
/// The parser decodes escapes into the AST, so every printer site emitting
/// stored string content passes it back through here: emitting the decoded
/// bytes verbatim re-decodes them on reparse and drifts the AST.
#[must_use]
pub fn escape_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            other => out.push(other),
        }
    }
    out
}

fn select_clause(out: &mut String, query: &SelectQuery) {
    out.push_str("SELECT ");
    if query.select.distinct {
        out.push_str("DISTINCT ");
    }
    if let Some(top) = &query.select.top {
        let _ = write!(out, "TOP {}", top.count);
        match top.direction {
            Some(TopDirection::Forward) => out.push_str(" FORWARD"),
            Some(TopDirection::Backward) => out.push_str(" BACKWARD"),
            None => {}
        }
        out.push(' ');
    }
    for (i, col) in query.select.columns.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        column_expr(out, &col.column);
        if let Some(alias) = &col.alias {
            let _ = write!(out, " AS {alias}");
        }
    }
}

fn column_expr(out: &mut String, col: &ColumnExpr) {
    match col {
        ColumnExpr::Path(p) => identified_path(out, p),
        ColumnExpr::Primitive(p) => primitive(out, p),
        ColumnExpr::Aggregate(a) => aggregate(out, a),
        ColumnExpr::Function(f) => function_call(out, f),
    }
}

/// Render a literal so it reparses as the same variant: a whole-valued
/// `Real` must keep its decimal point (`36.0`, not `36`, which would
/// reparse as an `Integer`); everything else uses the source-preserving
/// `Display`.
fn primitive(out: &mut String, p: &crate::ast::Primitive) {
    match p {
        crate::ast::Primitive::Real(r) if r.fract() == 0.0 && r.is_finite() => {
            let _ = write!(out, "{r:?}");
        }
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

fn aggregate(out: &mut String, agg: &AggregateCall) {
    match agg {
        AggregateCall::Count { distinct, path } => {
            out.push_str("COUNT(");
            if *distinct {
                out.push_str("DISTINCT ");
            }
            match path {
                Some(p) => identified_path(out, p),
                None => out.push('*'),
            }
            out.push(')');
        }
        AggregateCall::Stat { func, path } => {
            out.push_str(match func {
                StatFunc::Min => "MIN(",
                StatFunc::Max => "MAX(",
                StatFunc::Sum => "SUM(",
                StatFunc::Avg => "AVG(",
            });
            identified_path(out, path);
            out.push(')');
        }
    }
}

fn function_call(out: &mut String, call: &FunctionCall) {
    match call {
        FunctionCall::Named { name, args } => {
            let _ = write!(out, "{name}(");
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                terminal(out, arg);
            }
            out.push(')');
        }
        FunctionCall::Terminology(t) => terminology(out, t),
    }
}

fn terminology(out: &mut String, t: &TerminologyFunction) {
    let _ = write!(
        out,
        "TERMINOLOGY('{}', '{}', '{}')",
        escape_string(&t.operation),
        escape_string(&t.arg2),
        escape_string(&t.arg3)
    );
}

fn identified_path(out: &mut String, path: &IdentifiedPath) {
    out.push_str(&path.root);
    if let Some(pred) = &path.predicate {
        let _ = write!(out, "[{pred}]");
    }
    if let Some(object_path) = &path.path {
        let _ = write!(out, "/{object_path}");
    }
}

/// Where a contains expression sits, for parenthesisation.
///
/// The SIDE matters, not just the enclosing operator: `AqlParser.g4`
/// `containsExpr` states `AND`/`OR` as binary alternatives of one recursive
/// rule, which ANTLR4 resolves left-associatively, so a same-precedence child
/// re-parses unchanged on the LEFT and re-associates on the RIGHT. A right
/// operand therefore needs parentheses where a left operand does not.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ContainsCtx {
    /// Directly after `FROM` — no parens needed at any operator.
    Top,
    /// The left operand of an `OR`.
    OrLeft,
    /// The right operand of an `OR`.
    OrRight,
    /// The left operand of an `AND`.
    AndLeft,
    /// The right operand of an `AND`.
    AndRight,
    /// Directly after `CONTAINS` — any boolean child needs parens.
    Nested,
}

/// One class operand and its optional `[NOT] CONTAINS` constraint.
///
/// `containsExpr: classExprOperand (NOT? CONTAINS containsExpr)?` makes the
/// CONTAINS operand a whole `containsExpr`, so it absorbs any `AND`/`OR` that
/// follows it: an unparenthesised `A CONTAINS B` used as a boolean operand
/// would re-parse with the operator moved INSIDE its scope, which changes what
/// the query means — hence the parens.
fn contained_expr(
    out: &mut String,
    operand: &ClassExprOperand,
    contains: Option<&ContainsConstraint>,
    ctx: ContainsCtx,
) {
    let parens = contains.is_some() && !matches!(ctx, ContainsCtx::Top | ContainsCtx::Nested);
    if parens {
        out.push('(');
    }
    class_operand(out, operand);
    if let Some(constraint) = contains {
        if constraint.negated {
            out.push_str(" NOT CONTAINS ");
        } else {
            out.push_str(" CONTAINS ");
        }
        contains_expr(out, &constraint.expr, ContainsCtx::Nested);
    }
    if parens {
        out.push(')');
    }
}

fn contains_expr(out: &mut String, expr: &ContainsExpr, ctx: ContainsCtx) {
    match expr {
        ContainsExpr::Contained { operand, contains } => {
            contained_expr(out, operand, contains.as_deref(), ctx);
        }
        ContainsExpr::And(a, b) => {
            // `AND` binds tighter than `OR`, so only a right-hand `AND` (which
            // would re-associate leftwards) and a `CONTAINS` operand need parens.
            let parens = matches!(ctx, ContainsCtx::AndRight | ContainsCtx::Nested);
            if parens {
                out.push('(');
            }
            contains_expr(out, a, ContainsCtx::AndLeft);
            out.push_str(" AND ");
            contains_expr(out, b, ContainsCtx::AndRight);
            if parens {
                out.push(')');
            }
        }
        ContainsExpr::Or(a, b) => {
            let parens = matches!(
                ctx,
                ContainsCtx::OrRight
                    | ContainsCtx::AndLeft
                    | ContainsCtx::AndRight
                    | ContainsCtx::Nested
            );
            if parens {
                out.push('(');
            }
            contains_expr(out, a, ContainsCtx::OrLeft);
            out.push_str(" OR ");
            contains_expr(out, b, ContainsCtx::OrRight);
            if parens {
                out.push(')');
            }
        }
    }
}

fn class_operand(out: &mut String, operand: &ClassExprOperand) {
    match operand {
        ClassExprOperand::Class {
            rm_type,
            variable,
            predicate,
        } => {
            out.push_str(rm_type);
            if let Some(v) = variable {
                let _ = write!(out, " {v}");
            }
            if let Some(p) = predicate {
                let _ = write!(out, "[{p}]");
            }
        }
        ClassExprOperand::Version {
            variable,
            predicate,
        } => {
            out.push_str("VERSION");
            if let Some(v) = variable {
                let _ = write!(out, " {v}");
            }
            match predicate {
                Some(VersionPredicate::Latest) => out.push_str("[LATEST_VERSION]"),
                Some(VersionPredicate::All) => out.push_str("[ALL_VERSIONS]"),
                Some(VersionPredicate::Standard(s)) => {
                    let _ = write!(out, "[{s}]");
                }
                None => {}
            }
        }
    }
}

/// Precedence and associativity context inside a WHERE tree.
///
/// The SIDE matters, not just the enclosing operator: `AqlParser.g4`
/// `whereExpr` states `AND`/`OR` as binary alternatives of one recursive rule,
/// which ANTLR4 resolves left-associatively, so a same-precedence child
/// re-parses unchanged on the LEFT and re-associates on the RIGHT.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WhereCtx {
    /// Top level — nothing needs parens.
    Top,
    /// The left operand of an `OR`.
    OrLeft,
    /// The right operand of an `OR`.
    OrRight,
    /// The left operand of an `AND`.
    AndLeft,
    /// The right operand of an `AND`.
    AndRight,
    /// The operand of a `NOT` — any boolean child needs parens.
    Not,
}

fn where_expr(out: &mut String, expr: &WhereExpr, ctx: WhereCtx) {
    match expr {
        WhereExpr::Identified(leaf, _) => identified_expr(out, leaf),
        WhereExpr::Not(inner) => {
            out.push_str("NOT ");
            where_expr(out, inner, WhereCtx::Not);
        }
        WhereExpr::And(a, b) => {
            // `AND` binds tighter than `OR`, so only a right-hand `AND` (which
            // would re-associate leftwards) and a `NOT` operand need parens.
            let parens = matches!(ctx, WhereCtx::AndRight | WhereCtx::Not);
            if parens {
                out.push('(');
            }
            where_expr(out, a, WhereCtx::AndLeft);
            out.push_str(" AND ");
            where_expr(out, b, WhereCtx::AndRight);
            if parens {
                out.push(')');
            }
        }
        WhereExpr::Or(a, b) => {
            let parens = matches!(
                ctx,
                WhereCtx::OrRight | WhereCtx::AndLeft | WhereCtx::AndRight | WhereCtx::Not
            );
            if parens {
                out.push('(');
            }
            where_expr(out, a, WhereCtx::OrLeft);
            out.push_str(" OR ");
            where_expr(out, b, WhereCtx::OrRight);
            if parens {
                out.push(')');
            }
        }
    }
}

fn identified_expr(out: &mut String, expr: &IdentifiedExpr) {
    match expr {
        IdentifiedExpr::Exists(path) => {
            out.push_str("EXISTS ");
            identified_path(out, path);
        }
        IdentifiedExpr::Compare { lhs, op, rhs } => {
            match lhs {
                CompareOperand::Path(p) => identified_path(out, p),
                CompareOperand::Function(f) => function_call(out, f),
            }
            out.push_str(comp_op_text(*op));
            terminal(out, rhs);
        }
        IdentifiedExpr::Like { path, operand } => {
            identified_path(out, path);
            out.push_str(" LIKE ");
            match operand {
                LikeOperand::String(s) => {
                    let _ = write!(out, "'{}'", escape_string(s));
                }
                LikeOperand::Parameter(p) => out.push_str(p),
            }
        }
        IdentifiedExpr::Matches { path, operand } => {
            identified_path(out, path);
            out.push_str(" MATCHES ");
            matches_operand(out, operand);
        }
        // Never parser-produced; rendered as the equivalent boolean literal
        // comparison so output stays parseable.
        IdentifiedExpr::Resolved(value) => {
            let _ = write!(out, "{value}={value}");
        }
    }
}

fn matches_operand(out: &mut String, operand: &MatchesOperand) {
    match operand {
        MatchesOperand::ValueList(items) => {
            out.push('{');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                match item {
                    ValueListItem::Primitive(p) => primitive(out, p),
                    ValueListItem::Parameter(p) => out.push_str(p),
                    ValueListItem::Terminology(t) => terminology(out, t),
                }
            }
            out.push('}');
        }
        MatchesOperand::Terminology(t) => terminology(out, t),
        MatchesOperand::Uri(uri) => {
            let _ = write!(out, "{{{uri}}}");
        }
    }
}

fn terminal(out: &mut String, term: &Terminal) {
    match term {
        Terminal::Primitive(p) => primitive(out, p),
        Terminal::Parameter(p) => out.push_str(p),
        Terminal::Path(p) => identified_path(out, p),
        Terminal::Function(f) => function_call(out, f),
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse_str;
    use crate::printer::{escape_string, to_aql};

    /// parse → print → parse must reproduce the same AST.
    fn round_trips(src: &str) {
        let first = parse_str(src).expect("original parses");
        let printed = to_aql(&first);
        let second = parse_str(&printed)
            .unwrap_or_else(|e| panic!("printed AQL failed to parse: {printed}\n  {e}"));
        assert_eq!(first, second, "AST drift through print for: {printed}");
    }

    #[test]
    fn simple_and_shaped_queries_round_trip() {
        round_trips("SELECT e/ehr_id/value FROM EHR e");
        round_trips(
            "SELECT c/uid/value AS uid, o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude \
             FROM EHR e[ehr_id/value=$ehrUid] CONTAINS COMPOSITION c \
             CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2] \
             WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude >= 140 \
             AND (c/context/start_time/value < '2026-01-01' OR EXISTS c/context/end_time) \
             ORDER BY c/context/start_time/value DESC LIMIT 10 OFFSET 20",
        );
        round_trips(
            "SELECT COUNT(*) FROM EHR e CONTAINS (COMPOSITION a OR COMPOSITION b[openEHR-EHR-COMPOSITION.report.v1])",
        );
        round_trips(
            "SELECT c FROM VERSION v[LATEST_VERSION] CONTAINS COMPOSITION c \
             WHERE c/name/value MATCHES {'Vitals', 'Encounter'} AND NOT (c/language/code_string = 'de' OR c/language/code_string = 'fr')",
        );
        round_trips(
            "SELECT DISTINCT e/ehr_id/value FROM EHR e CONTAINS COMPOSITION c WHERE c/name/value LIKE 'Vit*'",
        );
    }

    #[test]
    fn escape_string_makes_lexer_valid_literals() {
        assert_eq!(escape_string("plain"), "plain");
        assert_eq!(escape_string("O'Neil"), "O\\'Neil");
        assert_eq!(escape_string("a\\b"), "a\\\\b");
    }
}
