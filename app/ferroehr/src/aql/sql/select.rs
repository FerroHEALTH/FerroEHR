// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! SELECT-clause lowering (QUERY master03 §SELECT, §DISTINCT, §Aggregate
//! functions): identified paths, literals, aggregates, and scalar functions;
//! whole-object columns project four locator columns the executor reassembles
//! through the node codec.
//!
//! No openEHR spec governs the projection mechanics — this is our own design. A scalar column is emitted as `to_jsonb(expr)`;
//! a whole structure object emits (`vo_id`, `sys_version`, `num`, `num_cap`) and
//! is reassembled at read time.

use sea_query::{Alias, Expr, ExprTrait as _, Func};

use crate::aql::error::AqlError;
use crate::aql::ir::{AggFunc, Coercion, LeafPath, PathTarget, QueryIr, SelectColumn, SelectValue};
use crate::db::iden::Node;

use super::expr::{col, leaf_path_string, literal_value, order_coercion, to_jsonb, type_cond};
use super::{Builder, CellKind, ColumnSpec, ValueMode};

impl Builder<'_> {
    pub(super) fn build_select(&mut self) -> Result<Vec<ColumnSpec>, AqlError> {
        let mut specs = Vec::with_capacity(self.ir.select.len());
        for (i, col) in self.ir.select.clone().iter().enumerate() {
            specs.push(self.emit_select_column(i, col)?);
        }
        Ok(specs)
    }

    /// Append the hidden access-logging columns: one `ehr_id` per bound VO
    /// root, after the projection.
    ///
    /// Returns their SQL aliases, or empty when this plan cannot carry them —
    /// `SELECT DISTINCT`, where an extra column changes which rows are
    /// distinct, and an aggregate projection, where an ungrouped column is not
    /// valid SQL. The contract those two exclusions leave is on
    /// [`super::ACCESS_EHR_PREFIX`].
    pub(super) fn build_access_ehr_columns(&mut self, ir: &QueryIr) -> Vec<String> {
        let aggregates = ir
            .select
            .iter()
            .any(|column| matches!(column.value, SelectValue::Aggregate { .. }));
        if ir.distinct || aggregates || self.group_roots.is_empty() {
            return Vec::new();
        }
        let roots = self.group_roots.clone();
        let mut aliases = Vec::with_capacity(roots.len());
        for (i, root) in roots.iter().enumerate() {
            let alias = format!("{}{i}", super::ACCESS_EHR_PREFIX);
            self.q
                .expr_as(col(root, Node::EhrId), Alias::new(alias.as_str()));
            aliases.push(alias);
        }
        aliases
    }

    /// Append the hidden version-locator columns: one `(vo_id, sys_version)`
    /// pair per bound VO root, under the same two exclusions as the access-EHR
    /// columns, so the origins of the served data aggregate over exactly the
    /// version rows that were served (#3212).
    pub(super) fn build_access_version_columns(&mut self, ir: &QueryIr) -> Vec<(String, String)> {
        let aggregates = ir
            .select
            .iter()
            .any(|column| matches!(column.value, SelectValue::Aggregate { .. }));
        if ir.distinct || aggregates || self.group_roots.is_empty() {
            return Vec::new();
        }
        let roots = self.group_roots.clone();
        let mut aliases = Vec::with_capacity(roots.len());
        for (i, root) in roots.iter().enumerate() {
            let vo = format!("{}{i}", super::ACCESS_VO_PREFIX);
            let sv = format!("{}{i}", super::ACCESS_SV_PREFIX);
            self.q
                .expr_as(col(root, Node::VoId), Alias::new(vo.as_str()));
            self.q
                .expr_as(col(root, Node::SysVersion), Alias::new(sv.as_str()));
            aliases.push((vo, sv));
        }
        aliases
    }

    fn emit_select_column(&mut self, i: usize, col: &SelectColumn) -> Result<ColumnSpec, AqlError> {
        let name = col.alias.clone().unwrap_or_else(|| format!("#{i}"));
        match &col.value {
            SelectValue::Path(PathTarget::Data(leaf)) if leaf.is_whole_object() => {
                let mut spec = self.emit_whole_object(i, name, leaf)?;
                // Prefer the query's own path text (the CNF goldens compare it
                // verbatim; `"/"` for a bare variable).
                if col.path.is_some() {
                    spec.path.clone_from(&col.path);
                }
                Ok(spec)
            }
            // `e/ehr_status` (whole) / `e/ehr_status/<structure child>` (whole):
            // join the EHR_STATUS root, then reassemble the addressed subtree.
            SelectValue::Path(PathTarget::EhrStatus(leaf)) if leaf.is_whole_object() => {
                self.ensure_ehr_status_root(leaf.source.0)?;
                let mut spec = self.emit_whole_object(i, name, leaf)?;
                if col.path.is_some() {
                    spec.path.clone_from(&col.path);
                }
                Ok(spec)
            }
            SelectValue::Path(target) => {
                let expr = self.value_expr(target, ValueMode::Projection)?;
                let sql_col = format!("col{i}");
                self.q.expr_as(to_jsonb(expr), Alias::new(sql_col.as_str()));
                Ok(ColumnSpec {
                    name,
                    path: col.path.clone().or_else(|| target_path_string(target)),
                    kind: CellKind::Scalar,
                    sql_cols: vec![sql_col],
                })
            }
            SelectValue::Literal(lit) => {
                let sql_col = format!("col{i}");
                self.q.expr_as(
                    to_jsonb(Expr::val(literal_value(lit))),
                    Alias::new(sql_col.as_str()),
                );
                Ok(ColumnSpec {
                    name,
                    path: None,
                    kind: CellKind::Scalar,
                    sql_cols: vec![sql_col],
                })
            }
            SelectValue::Aggregate {
                func,
                arg,
                distinct,
            } => {
                let inner = match arg {
                    None => None,
                    Some(target) => {
                        let mode = match func {
                            AggFunc::Count => ValueMode::Projection,
                            // MIN/MAX preserve the argument leaf's own
                            // comparison type — "Input values type should be
                            // either String, Date, Time, Integer or Real, and it
                            // will also determine the return type" (QUERY master03
                            // §MIN, §MAX) — never a forced numeric magnitude.
                            AggFunc::Min | AggFunc::Max => ValueMode::Value(order_coercion(target)),
                            // SUM/AVG accept Integer/Real only (§SUM, §AVG); the
                            // analyzer rejects a non-numeric leaf, so the
                            // magnitude coercion is always type-safe here.
                            AggFunc::Sum | AggFunc::Avg => ValueMode::Value(Coercion::Magnitude),
                        };
                        Some(self.value_expr(target, mode)?)
                    }
                };
                let agg = aggregate_expr(*func, inner, *distinct);
                let sql_col = format!("col{i}");
                self.q.expr_as(to_jsonb(agg), Alias::new(sql_col.as_str()));
                Ok(ColumnSpec {
                    name,
                    path: None,
                    kind: CellKind::Scalar,
                    sql_cols: vec![sql_col],
                })
            }
            SelectValue::Function { func, args } => {
                let expr = self.scalar_fn_expr(*func, args)?;
                let sql_col = format!("col{i}");
                self.q.expr_as(to_jsonb(expr), Alias::new(sql_col.as_str()));
                Ok(ColumnSpec {
                    name,
                    path: None,
                    kind: CellKind::Scalar,
                    sql_cols: vec![sql_col],
                })
            }
        }
    }

    fn emit_whole_object(
        &mut self,
        i: usize,
        name: String,
        leaf: &LeafPath,
    ) -> Result<ColumnSpec, AqlError> {
        let anchor = self.whole_object_alias(leaf)?;
        // A root predicate guards the projection: a row whose source node does
        // not satisfy it serves a NULL cell, never a dropped row — expressed by
        // CASE-guarding the vo_id locator alone (the executor short-circuits a
        // NULL vo_id to a null cell before reading the other three).
        let root_cond = match &leaf.root_predicate {
            Some(pred) => {
                let src = self.source_node(leaf.source.0)?;
                self.node_constraint_conds(&src, pred)?
                    .into_iter()
                    .reduce(sea_query::ExprTrait::and)
            }
            None => None,
        };
        let cols = ["vo", "sv", "num", "cap"];
        let node_cols = ["vo_id", "sys_version", "num", "num_cap"];
        let mut sql_cols = Vec::with_capacity(4);
        for (suffix, ncol) in cols.iter().zip(node_cols) {
            let sql_col = format!("col{i}_{suffix}");
            let locator = col(&anchor, ncol);
            let expr: Expr = match (&root_cond, ncol) {
                (Some(cond), "vo_id") => sea_query::CaseStatement::new()
                    .case(cond.clone(), locator)
                    .into(),
                _ => locator,
            };
            self.q.expr_as(expr, Alias::new(sql_col.as_str()));
            sql_cols.push(sql_col);
        }
        Ok(ColumnSpec {
            name,
            path: Some(leaf_path_string(leaf)),
            kind: CellKind::WholeObject,
            sql_cols,
        })
    }

    /// Resolve a whole-object leaf to a node alias present in the FROM. Empty
    /// anchor → the source node; otherwise the anchor chain is joined in and its
    /// final node alias returned.
    fn whole_object_alias(&mut self, leaf: &LeafPath) -> Result<String, AqlError> {
        let src = self.source_node(leaf.source.0)?;
        if leaf.anchor.is_empty() {
            return Ok(src);
        }
        let mut prev = src;
        for step in &leaf.anchor {
            let alias = format!("w{}", self.next_ctr());
            self.q.from_as(Node::Table, Alias::new(alias.as_str()));
            self.q.and_where(super::expr::hot(&alias, Node::Tier));
            self.q
                .and_where(col(&alias, Node::VoId).eq(col(&prev, Node::VoId)));
            self.q
                .and_where(col(&alias, Node::SysVersion).eq(col(&prev, Node::SysVersion)));
            self.q.and_where(
                col(&alias, Node::Num).between(col(&prev, Node::Num), col(&prev, Node::NumCap)),
            );
            if let Some(cond) = type_cond(&alias, &step.node_types) {
                self.q.and_where(cond);
            }
            if let Some(pred) = &step.predicate {
                for cond in self.node_constraint_conds(&alias, pred)? {
                    self.q.and_where(cond);
                }
            }
            prev = alias;
        }
        Ok(prev)
    }
}

/// Build the built-in aggregate expression (QUERY master03 §Aggregate functions).
/// The typed `Func` aggregates render `COUNT/MIN/MAX/SUM/AVG` and
/// `COUNT(*)`/`COUNT(DISTINCT …)` through the typed API.
fn aggregate_expr(func: AggFunc, inner: Option<Expr>, distinct: bool) -> Expr {
    match (func, inner) {
        // `COUNT(1)` is identical to `COUNT(*)` in Postgres (the sea-query
        // documented idiom for a count-all).
        (AggFunc::Count, None) => Expr::from(Func::count(Expr::val(1))),
        (AggFunc::Count, Some(e)) if distinct => Expr::from(Func::count_distinct(e)),
        (AggFunc::Count, Some(e)) => Expr::from(Func::count(e)),
        (AggFunc::Min, Some(e)) => Expr::from(Func::min(e)),
        (AggFunc::Max, Some(e)) => Expr::from(Func::max(e)),
        (AggFunc::Sum, Some(e)) => Expr::from(Func::sum(e)),
        (AggFunc::Avg, Some(e)) => Expr::from(Func::avg(e)),
        // MIN/MAX/SUM/AVG always carry an argument (grammar-enforced).
        (_, None) => Expr::val(Option::<i64>::None),
    }
}

/// A best-effort AQL path string for a `RESULT_SET` column's `path`
/// (ITS-REST 1.1.0 `RESULT_SET_COLUMN.path`).
fn target_path_string(target: &PathTarget) -> Option<String> {
    match target {
        PathTarget::Data(leaf) => Some(leaf_path_string(leaf)),
        // EHR / VERSION / EHR_STATUS columns carry their `path` from the query
        // text (`SelectColumn.path`), which the caller prefers; this fallback is
        // unused for them (a leaf under EHR_STATUS is rooted below `ehr_status`,
        // so its bare leaf path would omit that prefix).
        PathTarget::Version { .. } | PathTarget::Ehr { .. } | PathTarget::EhrStatus(_) => None,
    }
}
