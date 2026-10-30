// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The WHERE clause (QUERY master03 §WHERE, §Logical operators).
//!
//! QUERY master03 §WHERE is silent on paths that reach several nodes, so the
//! grain is our own design, built to keep AQL's operators boolean (§Logical
//! operators: `AND` holds when both operands do, `NOT X` when `X` is false):
//!
//! * Every structure step a WHERE path still has to descend below the node it
//!   starts from (a FROM variable, or a node the SELECT bound,
//!   `Builder::walk_start`) is bound ONCE for the whole WHERE clause, exactly
//!   as a SELECT step is bound: one node per binding, one `NULL` binding when
//!   nothing matches. Identical steps (predicate included) are one binding;
//!   distinct steps are independent bindings.
//! * A row is kept when SOME assignment of those bindings makes the whole WHERE
//!   clause true under SQL three-valued logic: the clause lowers to
//!   `EXISTS(<the bindings> WHERE <the clause over single values>)`.
//!
//! Every path in the clause but an `EXISTS` operand, inside a function
//! argument or on either side of a comparison too, reads one value off its
//! bound node, so the boolean laws
//! hold over each assignment (De Morgan, distribution, `NOT (x > 140)` true
//! when some node fails `x > 140`, `NULL` when `x` is absent). Reads of uid
//! synthesis, promoted columns, VERSION and EHR fields stay outside the
//! bindings as exact scalars.
//!
//! Elements of a list attribute inside one node's data (`links`,
//! `participations`, `identifiers`) are not bindings: a test of such a path
//! against a value holds when ANY element of the bound node satisfies it, and
//! is unknown when the list is empty or absent. A list tail beside another
//! path or inside a function reads its first element.
//!
//! `EXISTS <path>` is path-level (QUERY master03 §EXISTS: "true means the data
//! associated with the specified path exists"): it reads through the SELECT
//! bindings and walks its remaining steps in its own existential, so it is
//! the same for every assignment of the clause's bindings.
//!
//! An aggregate's per-row item walk (`Builder::where_filter_for`) re-lowers the
//! clause with the steps it shares fixed to the item's own nodes, so an item
//! counts when some assignment of the remaining bindings makes the clause true
//! for it.

use std::collections::VecDeque;

use sea_query::{Alias, Expr, ExprTrait as _, Query, SelectStatement};

use openehr_query::lexer::CompOp;

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::{
    self, Coercion, FragmentStep, LeafPath, Operand, ParamValue, PathTarget, StructureStep,
    TypedLit,
};
use crate::db::iden::{Node, VersionRow};

use super::expr::{binoper, call, col, hot};
use super::from::{attribute_child, left_binding_rows};
use super::predicate::raw_numeric_wanted;
use super::value::ItemWalk;
use super::{Builder, StepBinding, ValueMode};

/// The RM type of the one structure node that carries a `value`, as a SQL
/// LITERAL: `idx_node_hot_value_search` is partial on it, and a partial index
/// serves a statement only when the statement proves its predicate, which a
/// bound parameter in a generic plan cannot (PostgreSQL 18, "Partial
/// Indexes").
const ELEMENT_LITERAL: &str = "'ELEMENT'";

impl Builder<'_> {
    /// The WHERE clause of the statement, or `None` without one.
    ///
    /// # Errors
    /// [`AqlError`] from the path, parameter and function lowerings.
    pub(super) fn where_condition(&mut self) -> Result<Option<Expr>, AqlError> {
        let Some(filter) = self.ir.filter.clone() else {
            return Ok(None);
        };
        if !self.streaming {
            // The flat FROM is a list of comma items the planner orders
            // freely, so an anchor joins it as one more item; the streaming
            // shape placed its anchors ahead of the version spine.
            for anchor in self.value_anchors()? {
                let link = anchor.link();
                self.q
                    .from_subquery(anchor.select, Alias::new(anchor.alias.as_str()));
                self.q.and_where(link);
            }
        }
        self.lower_where(&filter, false).map(Some)
    }

    /// The value anchors of the WHERE clause: one per top-level conjunct that
    /// tests a text leaf of an `ELEMENT` value for equality with a literal or
    /// a parameter.
    ///
    /// A path binding is a correlated subquery the planner evaluates per
    /// outer row, so a lookup by a value read every node of the archetype and
    /// tested the value last (#4437). The anchor is the condition the
    /// conjunct implies, stated as a FROM item the planner can start from:
    /// the versions some `ELEMENT` row of which, under the leaf step's own
    /// predicate, carries the value as its search text
    /// (`ext.node_search_text`), which `idx_node_hot_value_search` serves.
    /// A FROM item rather than an `EXISTS`: PostgreSQL pulls an `EXISTS` up
    /// into a semi-join it orders freely only while the FROM list stays
    /// within `join_collapse_limit`, eight items, which a projection of a
    /// few path steps exceeds, and joins it last beyond that (PostgreSQL 18,
    /// "Controlling the Planner with Explicit JOIN Clauses"). A conjunct
    /// under `OR` or `NOT` implies nothing on its own and gets no anchor. No
    /// openEHR spec governs access paths: our own design.
    ///
    /// # Errors
    /// [`AqlError`] from the leaf step's predicate lowering.
    pub(super) fn value_anchors(&mut self) -> Result<Vec<ValueAnchor>, AqlError> {
        let Some(filter) = self.ir.filter.clone() else {
            return Ok(Vec::new());
        };
        let mut anchors = Vec::new();
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
                    if let Some(anchor) = self.value_anchor(leaf, text)? {
                        anchors.push(anchor);
                    }
                }
                _ => {}
            }
        }
        Ok(anchors)
    }

    /// The `version` alias of the versioned object that contains the source
    /// `sid` through `CONTAINS` edges: the nearest source at or above it that
    /// opened a version row, or `None` for a source no version row carries
    /// (an EHR, or a source under `NOT CONTAINS`).
    fn version_alias_of(&self, sid: usize) -> Option<String> {
        fn find(
            tree: &ir::ContainsTree,
            roots: &std::collections::BTreeMap<usize, String>,
            above: Option<&str>,
            sid: usize,
        ) -> Option<String> {
            match tree {
                ir::ContainsTree::Operand { source, contained } => {
                    let own = roots.get(&source.0).map(String::as_str).or(above);
                    if source.0 == sid {
                        return own.map(ToOwned::to_owned);
                    }
                    match contained.as_deref() {
                        Some(ir::Contained {
                            link: ir::Link::Contains,
                            tree,
                        }) => find(tree, roots, own, sid),
                        _ => None,
                    }
                }
                ir::ContainsTree::And(a, b) | ir::ContainsTree::Or(a, b) => {
                    find(a, roots, above, sid).or_else(|| find(b, roots, above, sid))
                }
            }
        }
        find(&self.ir.contains, &self.vo_alias, None, sid)
    }

    /// The anchor of one equality on `leaf` with `text`, or `None` when the
    /// leaf is no text leaf of an `ELEMENT` value ([`search_text_leaf`]).
    ///
    /// # Errors
    /// [`AqlError`] from the leaf step's predicate lowering.
    fn value_anchor(
        &mut self,
        leaf: &LeafPath,
        text: String,
    ) -> Result<Option<ValueAnchor>, AqlError> {
        if !search_text_leaf(&leaf.fragment, &text) {
            return Ok(None);
        }
        let (types, predicate) = match leaf.anchor.last() {
            Some(step) => (&step.node_types, step.predicate.as_ref()),
            None => match self.ir.sources.get(leaf.source.0) {
                Some(ir::Source::Rm(source)) => (&source.rm_type, None),
                _ => return Ok(None),
            },
        };
        if !types.contains("ELEMENT") {
            return Ok(None);
        }
        let Some(version) = self.version_alias_of(leaf.source.0) else {
            return Ok(None);
        };
        let alias = format!("va{}", self.next_ctr());
        let node = format!("{alias}_n");
        let mut select = Query::select();
        select
            .distinct()
            .column((Alias::new(node.as_str()), Node::VoId))
            .column((Alias::new(node.as_str()), Node::SysVersion))
            .from_as(Node::Table, Alias::new(node.as_str()));
        select.and_where(hot(&node, Node::Tier));
        select.and_where(col(&node, Node::RmType).eq(Expr::cust(ELEMENT_LITERAL)));
        if let Some(predicate) = predicate {
            for cond in self.node_constraint_conds(&node, predicate)? {
                select.and_where(cond);
            }
        }
        select.and_where(
            call(
                "node_search_text",
                vec![
                    col(&node, Node::ValueType),
                    col(&node, Node::ValueText),
                    col(&node, Node::Data),
                ],
            )
            .eq(Expr::val(text)),
        );
        Ok(Some(ValueAnchor {
            alias,
            version,
            select,
        }))
    }

    /// The WHERE clause as a filter on an aggregate's items, or `None` when no
    /// WHERE path reads through the steps the walk binds.
    ///
    /// QUERY master03 §Aggregate functions: the input values are "applied to
    /// data items filtered by `FROM` and `WHERE` clauses", so an item counts
    /// only when some assignment of the WHERE bindings, with the shared steps
    /// fixed to the item's own nodes, makes the clause true; a plain
    /// projection of the path keeps those rows. The elements of a list
    /// attribute inside one node are not bindings, so they are not filtered
    /// one by one: a kept node contributes all of them.
    ///
    /// # Errors
    /// [`AqlError`] from the WHERE lowering.
    pub(super) fn where_filter_for(&mut self, walk: &ItemWalk) -> Result<Option<Expr>, AqlError> {
        let Some(filter) = self.ir.filter.clone() else {
            return Ok(None);
        };
        let Some(first) = walk.bindings.first() else {
            return Ok(None);
        };
        let mut shared = false;
        filter.for_each_path(&mut |t| {
            if let PathTarget::Data(leaf) | PathTarget::EhrStatus(leaf) = t {
                shared |= leaf.source.0 == first.source && leaf.anchor.starts_with(&first.steps);
            }
        });
        if !shared {
            return Ok(None);
        }
        let mark = self.step_bindings.len();
        self.step_bindings.extend(walk.bindings.iter().cloned());
        let filter = self.lower_where(&filter, true);
        self.step_bindings.truncate(mark);
        filter.map(Some)
    }

    /// Lowers `filter` under the RULE of the module documentation.
    ///
    /// `nested` is whether the result sits inside another subquery; a
    /// top-level existential is hosted by [`Builder::correlated_exists`].
    ///
    /// # Errors
    /// [`AqlError`] from the path, parameter and function lowerings.
    fn lower_where(&mut self, filter: &ir::Expr, nested: bool) -> Result<Expr, AqlError> {
        let mut exists = VecDeque::new();
        self.collect_exists(filter, &mut exists)?;
        let mark = self.step_bindings.len();
        let mut bindings = Query::select();
        let lowered = self
            .bind_where_steps(filter, &mut bindings)
            .and_then(|bound| {
                let inside = nested || bound;
                self.where_formula(filter, inside, &mut exists)
                    .map(|f| (bound, f))
            });
        self.step_bindings.truncate(mark);
        let (bound, formula) = lowered?;
        if !bound {
            return Ok(formula);
        }
        bindings.and_where(formula);
        bindings.expr(Expr::val(1));
        Ok(if nested {
            Expr::exists(bindings)
        } else {
            self.correlated_exists(bindings)
        })
    }

    /// Binds, in `bindings`' FROM, every structure step the WHERE paths still
    /// descend, and registers each binding so the paths read through it.
    /// Returns whether any step was bound.
    ///
    /// # Errors
    /// [`AqlError`] from a step predicate or a path's start node.
    fn bind_where_steps(
        &mut self,
        filter: &ir::Expr,
        bindings: &mut SelectStatement,
    ) -> Result<bool, AqlError> {
        let mut leaves: Vec<(bool, LeafPath)> = Vec::new();
        bound_paths(filter, &mut |t| match t {
            PathTarget::Data(leaf) => leaves.push((false, (**leaf).clone())),
            PathTarget::EhrStatus(leaf) => leaves.push((true, (**leaf).clone())),
            PathTarget::Version { .. } | PathTarget::Ehr { .. } => {}
        });
        let mut bound = false;
        for (ehr_status, leaf) in leaves {
            if ehr_status {
                self.ensure_ehr_status_root(leaf.source.0)?;
            }
            if self.has_column_read(&leaf, ValueMode::Value(leaf.coercion)) {
                continue;
            }
            let (start, rest) = self.walk_start(&leaf)?;
            let rest: Vec<StructureStep> = rest.to_vec();
            let bound_steps = leaf.anchor.len().saturating_sub(rest.len());
            let mut prefix: Vec<StructureStep> =
                leaf.anchor.iter().take(bound_steps).cloned().collect();
            let mut parent = start;
            for step in rest {
                prefix.push(step.clone());
                let existing = self
                    .step_bindings
                    .iter()
                    .find(|b| b.source == leaf.source.0 && b.steps == prefix)
                    .map(|b| b.alias.clone());
                if let Some(alias) = existing {
                    parent = alias;
                    continue;
                }
                let alias = format!("w{}", self.next_ctr());
                let inner = format!("{alias}_n");
                let mut on =
                    hot(&inner, Node::Tier).and(attribute_child(&inner, &parent, &step.attribute));
                if let Some(cond) = super::expr::type_cond(&inner, &step.node_types) {
                    on = on.and(cond);
                }
                if let Some(pred) = &step.predicate {
                    for cond in self.node_constraint_conds(&inner, pred)? {
                        on = on.and(cond);
                    }
                }
                bindings.from_function(
                    left_binding_rows(&alias, &inner, on),
                    Alias::new(alias.as_str()),
                );
                self.step_bindings.push(StepBinding {
                    source: leaf.source.0,
                    steps: prefix.clone(),
                    alias: alias.clone(),
                });
                parent = alias;
                bound = true;
            }
        }
        Ok(bound)
    }

    /// Lowers the boolean structure of the WHERE clause over the values its
    /// paths read off their bound nodes, under SQL three-valued logic.
    ///
    /// # Errors
    /// [`AqlError`] from the path, parameter and function lowerings.
    fn where_formula(
        &mut self,
        expr: &ir::Expr,
        nested: bool,
        exists: &mut VecDeque<ExistsOperand>,
    ) -> Result<Expr, AqlError> {
        match expr {
            ir::Expr::And(a, b) => Ok(self
                .where_formula(a, nested, exists)?
                .and(self.where_formula(b, nested, exists)?)),
            ir::Expr::Or(a, b) => Ok(self
                .where_formula(a, nested, exists)?
                .or(self.where_formula(b, nested, exists)?)),
            ir::Expr::Not(a) => Ok(self.where_formula(a, nested, exists)?.not()),
            ir::Expr::Const(b) => Ok(Expr::val(*b)),
            ir::Expr::Exists(_) => match exists.pop_front() {
                Some(ExistsOperand::Value(test)) => Ok(test),
                Some(ExistsOperand::Walk(sub)) if nested => Ok(Expr::exists(*sub)),
                Some(ExistsOperand::Walk(sub)) => Ok(self.correlated_exists(*sub)),
                None => Err(SqlError::Unsupported(
                    "an EXISTS operand lowered out of order".to_owned(),
                )
                .into()),
            },
            // NOTE: QUERY master03 §WHERE is silent; a list tail beside another path
            // or inside a function reads its first element, as a projection does.
            ir::Expr::Compare { .. } | ir::Expr::Like { .. } | ir::Expr::Matches { .. } => {
                match self.list_test(expr, nested)? {
                    Some(test) => Ok(test),
                    None => self.scalar_test(expr),
                }
            }
        }
    }

    /// The test an `EXISTS <path>` operand lowers to (QUERY master03
    /// §EXISTS).
    ///
    /// # Errors
    /// [`AqlError`] from lowering the path.
    fn exists_operand(&mut self, target: &PathTarget) -> Result<ExistsOperand, AqlError> {
        Ok(match self.leaf_items(target, ValueMode::Projection)? {
            Some(walk) => {
                let mut sub = walk.sub;
                sub.and_where(walk.item.is_not_null());
                sub.expr(Expr::val(1));
                ExistsOperand::Walk(Box::new(sub))
            }
            None => ExistsOperand::Value(
                self.value_expr(target, ValueMode::Projection)?
                    .is_not_null(),
            ),
        })
    }

    /// Lowers every `EXISTS` operand of `expr`, in the order
    /// [`Self::where_formula`] meets them, before the WHERE bindings exist.
    ///
    /// QUERY master03 §EXISTS: "true means the data associated with the
    /// specified path exists". Existence is a property of the path, so the
    /// operand reads through the SELECT bindings (one answer per bound node)
    /// and walks its remaining steps in its own existential, never through the
    /// clause's assignment; within an assignment it is a constant, which keeps
    /// the boolean laws.
    ///
    /// # Errors
    /// [`AqlError`] from the path lowerings.
    fn collect_exists(
        &mut self,
        expr: &ir::Expr,
        out: &mut VecDeque<ExistsOperand>,
    ) -> Result<(), AqlError> {
        match expr {
            ir::Expr::And(a, b) | ir::Expr::Or(a, b) => {
                self.collect_exists(a, out)?;
                self.collect_exists(b, out)
            }
            ir::Expr::Not(a) => self.collect_exists(a, out),
            ir::Expr::Exists(target) => {
                // EXISTS only asks whether the data is there, so a whole object
                // is tested by its row rather than by its rebuilt fragment.
                self.presence_only = true;
                let operand = self.exists_operand(target);
                self.presence_only = false;
                out.push_back(operand?);
                Ok(())
            }
            ir::Expr::Compare { .. }
            | ir::Expr::Like { .. }
            | ir::Expr::Matches { .. }
            | ir::Expr::Const(_) => Ok(()),
        }
    }

    /// Lowers a test of one path against bound values when that path ends in a
    /// list attribute inside its node's data: it holds when ANY element of the
    /// bound node satisfies it (QUERY master03 §WHERE is silent on multi-valued
    /// paths; our own design). `None` for every other test.
    ///
    /// # Errors
    /// [`AqlError`] from the bound-value and path lowerings.
    fn list_test(&mut self, expr: &ir::Expr, nested: bool) -> Result<Option<Expr>, AqlError> {
        let target = match expr {
            ir::Expr::Compare { lhs, rhs, .. } => match (lhs, rhs) {
                (Operand::Path(t), b) | (b, Operand::Path(t)) if !matches!(b, Operand::Path(_)) => {
                    t
                }
                _ => return Ok(None),
            },
            ir::Expr::Like { path, .. } | ir::Expr::Matches { path, .. } => path,
            ir::Expr::Exists(_)
            | ir::Expr::And(..)
            | ir::Expr::Or(..)
            | ir::Expr::Not(_)
            | ir::Expr::Const(_) => {
                return Ok(None);
            }
        };
        let leaf = match target {
            PathTarget::Data(leaf) | PathTarget::EhrStatus(leaf) => leaf,
            PathTarget::Version { .. } | PathTarget::Ehr { .. } => return Ok(None),
        };
        // NOTE: QUERY master03 §WHERE is silent; list elements inside one node are
        // not bindings (an aggregate under this test still counts every element).
        if leaf.fragment.is_empty() || !leaf.fragment_multi_valued() {
            return Ok(None);
        }
        let Some((mode, test)) = self.value_test(expr)? else {
            return Ok(None);
        };
        if let PathTarget::EhrStatus(_) = target {
            self.ensure_ehr_status_root(leaf.source.0)?;
        }
        if self.has_column_read(leaf, mode) {
            return Ok(None);
        }
        let (at, rest) = self.walk_start(leaf)?;
        if !rest.is_empty() {
            return Ok(None);
        }
        let (items, value) = self.item_at(leaf, mode, &at)?;
        let Some(mut sub) = items else {
            return Ok(None);
        };
        if let Some(pred) = &leaf.root_predicate {
            let src = self.source_node(leaf.source.0)?;
            for cond in self.node_constraint_conds(&src, pred)? {
                sub.and_where(cond);
            }
        }
        sub.expr(Expr::val(1));
        let mut hit = sub.clone();
        hit.and_where(test(value));
        let (present, hit) = if nested {
            (Expr::exists(sub), Expr::exists(hit))
        } else {
            (self.correlated_exists(sub), self.correlated_exists(hit))
        };
        // An absent or empty list is unknown, as every absent path is.
        Ok(Some(
            sea_query::CaseStatement::new().case(present, hit).into(),
        ))
    }

    /// The extraction mode and the test a one-path test applies to the value
    /// it reads, or `None` for every other expression.
    ///
    /// # Errors
    /// [`AqlError`] from the bound-value lowerings.
    fn value_test(&mut self, expr: &ir::Expr) -> Result<Option<(ValueMode, TestFn)>, AqlError> {
        let (mode, test): (ValueMode, TestFn) = match expr {
            ir::Expr::Compare {
                lhs,
                op,
                rhs,
                coercion,
            } => {
                let path_is_lhs = matches!(lhs, Operand::Path(_));
                let bound = if path_is_lhs { rhs } else { lhs };
                let raw = *coercion == Coercion::Raw && raw_numeric_wanted(&[lhs, rhs]);
                let (mode, bound) = if raw {
                    (
                        ValueMode::RawNumeric,
                        self.operand_value_raw_numeric(bound)?,
                    )
                } else {
                    (
                        ValueMode::Value(*coercion),
                        self.operand_value(bound, *coercion)?,
                    )
                };
                let op = binoper(*op);
                (
                    mode,
                    if path_is_lhs {
                        Box::new(move |value: Expr| value.binary(op, bound))
                    } else {
                        Box::new(move |value: Expr| bound.binary(op, value))
                    },
                )
            }
            ir::Expr::Like { pattern, .. } => {
                let pattern = self.like_pattern(pattern)?;
                (
                    ValueMode::Value(Coercion::Text),
                    Box::new(move |value: Expr| value.like(pattern)),
                )
            }
            ir::Expr::Matches {
                values, coercion, ..
            } => {
                let (mode, members) = self.matches_members(values, *coercion)?;
                (mode, Box::new(move |value: Expr| value.is_in(members)))
            }
            ir::Expr::Exists(_)
            | ir::Expr::And(..)
            | ir::Expr::Or(..)
            | ir::Expr::Not(_)
            | ir::Expr::Const(_) => {
                return Ok(None);
            }
        };
        Ok(Some((mode, test)))
    }
}

/// A test applied to the value one path reads.
type TestFn = Box<dyn FnOnce(Expr) -> Expr>;

/// One `EXISTS` operand, lowered before the WHERE bindings exist.
enum ExistsOperand {
    /// A single-valued read off a source or bound node: `value IS NOT NULL`.
    Value(Expr),
    /// The operand's own walk (FROM/WHERE, selecting 1).
    Walk(Box<SelectStatement>),
}

/// Calls `f` on every path of `expr` that reads through the WHERE bindings:
/// every path except an `EXISTS` operand.
fn bound_paths(expr: &ir::Expr, f: &mut impl FnMut(&PathTarget)) {
    match expr {
        ir::Expr::And(a, b) | ir::Expr::Or(a, b) => {
            bound_paths(a, f);
            bound_paths(b, f);
        }
        ir::Expr::Not(a) => bound_paths(a, f),
        ir::Expr::Exists(_) | ir::Expr::Const(_) => {}
        ir::Expr::Compare { .. } | ir::Expr::Like { .. } | ir::Expr::Matches { .. } => {
            expr.for_each_path(f);
        }
    }
}

/// One value anchor ([`Builder::value_anchors`]): the distinct versions whose
/// `ELEMENT` rows carry a value, as a FROM item linked to the version row of
/// the leaf's versioned object.
pub(super) struct ValueAnchor {
    /// The item's alias.
    pub(super) alias: String,
    /// The `version` alias the item links to.
    pub(super) version: String,
    /// The item: `SELECT DISTINCT vo_id, sys_version FROM node …`.
    pub(super) select: SelectStatement,
}

impl ValueAnchor {
    /// The join condition between the item and its version row.
    pub(super) fn link(&self) -> Expr {
        col(&self.alias, Node::VoId)
            .eq(col(&self.version, VersionRow::VoId))
            .and(col(&self.alias, Node::SysVersion).eq(col(&self.version, VersionRow::SysVersion)))
    }
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
