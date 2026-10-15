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

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::{self, Coercion, LeafPath, Operand, PathTarget, StructureStep};

use super::expr::{binoper, hot};
use super::from::{attribute_child, left_binding_rows};
use super::predicate::raw_numeric_wanted;
use super::value::ItemWalk;
use super::{Builder, StepBinding, ValueMode};

impl Builder<'_> {
    /// The WHERE clause of the statement, or `None` without one.
    ///
    /// # Errors
    /// [`AqlError`] from the path, parameter and function lowerings.
    pub(super) fn where_condition(&mut self) -> Result<Option<Expr>, AqlError> {
        let Some(filter) = self.ir.filter.clone() else {
            return Ok(None);
        };
        self.lower_where(&filter, false).map(Some)
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
                let mut on = hot(&inner, crate::db::iden::Node::Tier).and(attribute_child(
                    &inner,
                    &parent,
                    &step.attribute,
                ));
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
                let operand = match self.leaf_items(target, ValueMode::Projection)? {
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
                };
                out.push_back(operand);
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
