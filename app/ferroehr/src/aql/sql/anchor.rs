// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The anchors of a WHERE clause: the entry an index gives a filter on a
//! value.
//!
//! A path binding is a correlated subquery the planner evaluates per outer
//! row, so a filter on a value read every node of the archetype and tested
//! the value last (#4437, #6022). An anchor is the condition a top-level
//! conjunct implies, stated as a FROM item the planner can start from: the
//! versions some node row of which, under the leaf step's own predicate,
//! carries a value the conjunct keeps. Four leaves have one:
//!
//! - a text leaf of an `ELEMENT` value under `=`, by its search text
//!   (`ext.node_search_text`, `idx_node_hot_value_search`);
//! - the code of an `ELEMENT` value under `=`, the `defining_code` of a coded
//!   text or of an ordinal's symbol, by `ext.node_code`
//!   (`idx_node_hot_value_code`);
//! - the `time` of an event and the `origin` of a history under `=`, `<`,
//!   `<=`, `>` or `>=`, by `node.event_time`, which the node insert fills
//!   with the value the comparison reads (`idx_node_hot_event_time`);
//! - the `magnitude` of an `ELEMENT` value under `<`, `<=`, `>` or `>=`, by
//!   `ext.node_magnitude` (`idx_node_hot_value_magnitude`).
//!
//! The conjunct itself stays in the statement and each anchor reads the
//! value the conjunct reads, so an anchor removes work and no row. A conjunct
//! under `OR` or `NOT` implies nothing on its own and gets none.
//!
//! An equality keeps few rows, so its anchor always applies. A range may keep
//! most of the store, and how its anchor enters depends on the statement
//! ([`RangeEntry`]). The comparisons of one leaf bind one node row, so they
//! share one anchor: a window `>= $from AND < $to` reads the rows inside it.
//!
//! A FROM item rather than an `EXISTS`: PostgreSQL pulls an `EXISTS` up into
//! a semi-join it orders freely only while the FROM list stays within
//! `join_collapse_limit`, eight items, which a projection of a few path
//! steps exceeds, and joins it last beyond that (PostgreSQL 18, "Controlling
//! the Planner with Explicit JOIN Clauses"). No openEHR spec governs access
//! paths: our own design.

use sea_query::{Alias, Expr, ExprTrait, Query, SelectStatement};

use openehr_query::lexer::CompOp;

use crate::aql::error::AqlError;
use crate::aql::ir::{
    self, Coercion, FragmentStep, LeafPath, Operand, ParamValue, PathTarget, TypedLit,
};
use crate::db::iden::{Node, VersionRow};

use super::Builder;
use super::expr::{binoper, call, col, hot};

/// The RM type of the one structure node that carries a `value`, as a SQL
/// LITERAL: the value indexes are partial on it, and a partial index serves a
/// statement only when the statement proves its predicate, which a bound
/// parameter in a generic plan cannot (PostgreSQL 18, "Partial Indexes").
const ELEMENT_LITERAL: &str = "'ELEMENT'";

/// How the anchor of a range comparison enters a statement.
///
/// No openEHR spec governs access paths: our own design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum RangeEntry {
    /// The flat shape: the anchor is one more FROM item, the distinct
    /// versions, which the planner places by its estimate of the range.
    #[default]
    Free,
    /// A population statement that streams toward its LIMIT: one anchor
    /// leads the spine as the first row of each version the range keeps, so
    /// the statement reads the index until the LIMIT and never the whole
    /// range.
    Leading,
    /// A statement that reads the versions of the EHRs it names: those
    /// versions are its entry, and a range over the store is none.
    Off,
}

/// What a range comparison reads of a node row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ranged {
    /// `event_time` of an event row.
    EventTime,
    /// `event_time` of a history row, its origin.
    Origin,
    /// `ext.node_magnitude` of an `ELEMENT` row.
    Magnitude,
}

impl Ranged {
    /// The RM types of the rows that carry the value, as SQL literals.
    const fn literals(self) -> &'static str {
        match self {
            Self::EventTime => "'INTERVAL_EVENT', 'POINT_EVENT'",
            Self::Origin => "'HISTORY'",
            Self::Magnitude => ELEMENT_LITERAL,
        }
    }

    /// Whether a step that binds `types` binds only rows that carry the
    /// value, or, for a magnitude, can bind an `ELEMENT`.
    fn binds(self, types: &ir::TypeSet) -> bool {
        let only = |timed: &[&str]| {
            !types.is_empty() && types.names().iter().all(|t| timed.contains(&t.as_str()))
        };
        match self {
            Self::EventTime => only(&["INTERVAL_EVENT", "POINT_EVENT"]),
            Self::Origin => only(&["HISTORY"]),
            Self::Magnitude => types.contains("ELEMENT"),
        }
    }

    /// The value of the row `node`.
    fn read(self, node: &str) -> Expr {
        match self {
            Self::EventTime | Self::Origin => col(node, Node::EventTime),
            Self::Magnitude => call(
                "node_magnitude",
                vec![
                    col(node, Node::ValueType),
                    col(node, Node::ValueNum),
                    col(node, Node::Data),
                ],
            ),
        }
    }

    /// The coercion of the comparison.
    const fn coercion(self) -> Coercion {
        match self {
            Self::EventTime | Self::Origin => Coercion::Temporal,
            Self::Magnitude => Coercion::Magnitude,
        }
    }
}

/// The range comparisons of one leaf: what they read and each operator with
/// its bound.
struct Range<'a> {
    leaf: &'a LeafPath,
    ranged: Ranged,
    bounds: Vec<(CompOp, Expr)>,
}

impl Builder<'_> {
    /// The anchors of the WHERE clause, in the order the statement takes
    /// them: the equalities, then the ranges [`Builder::range_entry`] admits.
    ///
    /// # Errors
    /// [`AqlError`] from the leaf steps' predicate lowering and from a bound
    /// the comparison itself refuses.
    pub(super) fn value_anchors(&mut self) -> Result<Vec<ValueAnchor>, AqlError> {
        let Some(filter) = self.ir.filter.clone() else {
            return Ok(Vec::new());
        };
        let mut anchors = Vec::new();
        let mut ranges: Vec<Range<'_>> = Vec::new();
        let mut conjuncts = vec![&filter];
        while let Some(expr) = conjuncts.pop() {
            match expr {
                ir::Expr::And(a, b) => {
                    conjuncts.push(b);
                    conjuncts.push(a);
                }
                // NOTE: a `Raw` (mixed-type) equality with a string compares as text,
                // the numeric branch being taken on a numeric literal alone
                // (`Builder::value_test`), which no string operand is.
                ir::Expr::Compare {
                    lhs,
                    op: CompOp::Eq,
                    rhs,
                    coercion: Coercion::Text | Coercion::Raw,
                } => {
                    let ((Operand::Path(PathTarget::Data(leaf)), bound)
                    | (bound, Operand::Path(PathTarget::Data(leaf)))) = (lhs, rhs)
                    else {
                        continue;
                    };
                    let text = match bound {
                        Operand::Literal(TypedLit::String(text)) => text.clone(),
                        Operand::Param(name) => match self.params.get(name) {
                            Some(ParamValue::Str(text)) => text.clone(),
                            _ => continue,
                        },
                        _ => continue,
                    };
                    anchors.extend(self.value_anchor(leaf, &text)?);
                }
                // NOTE: AQL QUERY master03 §WHERE (`identifiedExpr`): the grammar puts the
                // path on the left of a comparison and the terminal on its right.
                ir::Expr::Compare {
                    lhs: Operand::Path(PathTarget::Data(leaf)),
                    op,
                    rhs: bound @ (Operand::Literal(_) | Operand::Param(_)),
                    coercion,
                } if self.range_entry != RangeEntry::Off => {
                    let Some(reads) = ranged(leaf, *op, *coercion) else {
                        continue;
                    };
                    let bound = self.operand_value(bound, reads.coercion())?;
                    let leaf: &LeafPath = leaf;
                    match ranges.iter_mut().find(|range| range.leaf == leaf) {
                        Some(range) => range.bounds.push((*op, bound)),
                        None => ranges.push(Range {
                            leaf,
                            ranged: reads,
                            bounds: vec![(*op, bound)],
                        }),
                    }
                }
                _ => {}
            }
        }
        // A leading anchor is the one entry of a streaming statement, which an
        // equality already gives.
        let leading = self.range_entry == RangeEntry::Leading;
        if leading && !anchors.is_empty() {
            return Ok(anchors);
        }
        for range in ranges {
            let anchor = self.range_anchor(&range, leading)?;
            let found = anchor.is_some();
            anchors.extend(anchor);
            if leading && found {
                break;
            }
        }
        Ok(anchors)
    }

    /// The anchor of one equality on `leaf` with `text`, or `None` when the
    /// leaf is neither a text leaf of an `ELEMENT` value
    /// ([`search_text_leaf`]) nor its code ([`code_leaf`]).
    ///
    /// # Errors
    /// [`AqlError`] from the leaf step's predicate lowering.
    fn value_anchor(
        &mut self,
        leaf: &LeafPath,
        text: &str,
    ) -> Result<Option<ValueAnchor>, AqlError> {
        let (function, column) = if code_leaf(&leaf.fragment) {
            ("node_code", Node::ValueCode)
        } else if search_text_leaf(&leaf.fragment, text) {
            ("node_search_text", Node::ValueText)
        } else {
            return Ok(None);
        };
        let Some((types, predicate)) = leaf_step(self.ir, leaf) else {
            return Ok(None);
        };
        if !types.contains("ELEMENT") {
            return Ok(None);
        }
        self.row_anchor(leaf, predicate, false, |node| {
            (
                col(node, Node::RmType).eq(Expr::cust(ELEMENT_LITERAL)),
                call(
                    function,
                    vec![
                        col(node, Node::ValueType),
                        col(node, column),
                        col(node, Node::Data),
                    ],
                )
                .eq(Expr::val(text)),
            )
        })
    }

    /// The anchor of the range comparisons of one leaf, or `None` when the
    /// leaf's step can bind a row that does not carry the value.
    ///
    /// `first` asks for the first row of each version the range keeps
    /// instead of the distinct versions ([`RangeEntry::Leading`]).
    ///
    /// # Errors
    /// [`AqlError`] from the leaf step's predicate lowering.
    fn range_anchor(
        &mut self,
        range: &Range<'_>,
        first: bool,
    ) -> Result<Option<ValueAnchor>, AqlError> {
        let Some((types, predicate)) = leaf_step(self.ir, range.leaf) else {
            return Ok(None);
        };
        if !range.ranged.binds(types) {
            return Ok(None);
        }
        let ranged = range.ranged;
        self.row_anchor(range.leaf, predicate, first, |node| {
            let tests = range
                .bounds
                .iter()
                .map(|(op, bound)| ranged.read(node).binary(binoper(*op), bound.clone()))
                .reduce(ExprTrait::and)
                .unwrap_or_else(|| Expr::val(true));
            (
                Expr::cust_with_exprs(
                    format!("$1 IN ({})", ranged.literals()),
                    [col(node, Node::RmType)],
                ),
                tests,
            )
        })
    }

    /// The versions some node row of which satisfies, under `predicate`, the
    /// two conditions `of` states of a row, its RM type and the test of its
    /// value, as an anchor of `leaf`'s versioned object; `None` when the
    /// leaf's source has no version row.
    ///
    /// With `first` the item is the first such row of each version, found
    /// by the absence of an earlier one, which a scan toward a LIMIT reads
    /// row by row; without it, the distinct versions.
    ///
    /// # Errors
    /// [`AqlError`] from the predicate lowering.
    fn row_anchor(
        &mut self,
        leaf: &LeafPath,
        predicate: Option<&ir::NodeConstraint>,
        first: bool,
        of: impl Fn(&str) -> (Expr, Expr),
    ) -> Result<Option<ValueAnchor>, AqlError> {
        let Some(version) = self.version_alias_of(leaf.source.0) else {
            return Ok(None);
        };
        let key = self.node_key_of(&version);
        let alias = format!("va{}", self.next_ctr());
        let node = format!("{alias}_n");
        let mut select = self.anchor_rows(&node, predicate, &of)?;
        select
            .column((Alias::new(node.as_str()), Node::VoId))
            .column((Alias::new(node.as_str()), Node::SysVersion));
        if first {
            let earlier = format!("{alias}_e");
            let mut before = self.anchor_rows(&earlier, predicate, &of)?;
            before
                .expr(Expr::val(1))
                .and_where(col(&earlier, Node::VoId).eq(col(&node, Node::VoId)))
                .and_where(col(&earlier, Node::SysVersion).eq(col(&node, Node::SysVersion)))
                .and_where(col(&earlier, Node::Num).lt(col(&node, Node::Num)))
                // The fence keeps the probe correlated: without it the planner
                // makes it an anti-join over every row the range keeps.
                .offset(0);
            select.and_where(Expr::exists(before).not());
        } else {
            select.distinct();
        }
        Ok(Some(ValueAnchor {
            alias,
            version,
            key,
            select,
        }))
    }

    /// The hot node rows `node` that satisfy `of` under `predicate`, with no
    /// column selected yet.
    fn anchor_rows(
        &self,
        node: &str,
        predicate: Option<&ir::NodeConstraint>,
        of: &impl Fn(&str) -> (Expr, Expr),
    ) -> Result<SelectStatement, AqlError> {
        let mut select = Query::select();
        select.from_as(Node::Table, Alias::new(node));
        select.and_where(hot(node, Node::Tier));
        let (of_type, test) = of(node);
        select.and_where(of_type);
        if let Some(predicate) = predicate {
            for cond in self.node_constraint_conds(node, predicate)? {
                select.and_where(cond);
            }
        }
        select.and_where(test);
        Ok(select)
    }
}

/// The RM types and the predicate of the node step `leaf`'s fragment is read
/// from: its last structure step, or its source.
fn leaf_step<'a>(
    ir: &'a ir::QueryIr,
    leaf: &'a LeafPath,
) -> Option<(&'a ir::TypeSet, Option<&'a ir::NodeConstraint>)> {
    match leaf.anchor.last() {
        Some(step) => Some((&step.node_types, step.predicate.as_ref())),
        None => match ir.sources.get(leaf.source.0) {
            Some(ir::Source::Rm(source)) => Some((&source.rm_type, None)),
            _ => None,
        },
    }
}

/// What a comparison of `leaf` under `op` and `coercion` reads of a node row
/// an index holds, or `None` for any other comparison.
fn ranged(leaf: &LeafPath, op: CompOp, coercion: Coercion) -> Option<Ranged> {
    if op == CompOp::Ne || leaf.fragment.iter().any(|step| step.predicate.is_some()) {
        return None;
    }
    let names: Vec<&str> = leaf.fragment.iter().map(|s| s.name.as_str()).collect();
    match (coercion, names.as_slice()) {
        (Coercion::Temporal, ["time", "value"]) => Some(Ranged::EventTime),
        (Coercion::Temporal, ["origin", "value"]) => Some(Ranged::Origin),
        (Coercion::Magnitude, ["value", "magnitude"]) if op != CompOp::Eq => {
            Some(Ranged::Magnitude)
        }
        _ => None,
    }
}

/// One anchor ([`Builder::value_anchors`]): the versions some node row of
/// which carries a value, as a FROM item linked to the version row of the
/// leaf's versioned object.
pub(super) struct ValueAnchor {
    /// The item's alias.
    pub(super) alias: String,
    /// The `version` alias the item links to.
    pub(super) version: String,
    /// The column the item's `sys_version` joins: the node key of the version
    /// row under its scope (`head::node_key`, #4944).
    key: Expr,
    /// The item: `SELECT DISTINCT vo_id, sys_version FROM node …`, or the
    /// first row of each version.
    pub(super) select: SelectStatement,
}

impl ValueAnchor {
    /// The join condition between the item and its version row.
    pub(super) fn link(&self) -> Expr {
        col(&self.alias, Node::VoId)
            .eq(col(&self.version, VersionRow::VoId))
            .and(col(&self.alias, Node::SysVersion).eq(self.key.clone()))
    }
}

/// Whether an equality of the fragment path `steps` compares the code
/// `ext.node_code` yields for every `ELEMENT` row that can satisfy it: the
/// `defining_code` of a coded text, or of the symbol of an ordinal or a
/// scale.
fn code_leaf(steps: &[FragmentStep]) -> bool {
    if steps.iter().any(|step| step.predicate.is_some()) {
        return false;
    }
    let names: Vec<&str> = steps.iter().map(|step| step.name.as_str()).collect();
    matches!(
        names.as_slice(),
        ["value", "defining_code", "code_string"]
            | ["value", "symbol", "defining_code", "code_string"]
    )
}

/// Whether an equality of the fragment path `steps` with `text` compares the
/// text `ext.node_search_text` yields for every `ELEMENT` row that can
/// satisfy it: a `DV_IDENTIFIER` id, a `DV_QUANTITY`'s units, a symbol's
/// value, or the `value` member.
///
/// A `DV_ORDINAL` or `DV_SCALE` `value` is a number while its search text is
/// its symbol, so an equality of `value/value` is anchored only on a text no
/// number renders as.
fn search_text_leaf(steps: &[FragmentStep], text: &str) -> bool {
    if steps.iter().any(|step| step.predicate.is_some()) {
        return false;
    }
    let names: Vec<&str> = steps.iter().map(|step| step.name.as_str()).collect();
    match names.as_slice() {
        ["value", "id" | "units"] | ["value", "symbol", "value"] => true,
        ["value", "value"] => text.parse::<f64>().is_err(),
        _ => false,
    }
}
