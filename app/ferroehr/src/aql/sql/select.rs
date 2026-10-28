// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! SELECT-clause lowering (QUERY master03 §SELECT, §DISTINCT, §Aggregate
//! functions): identified paths, literals, aggregates, and scalar functions;
//! whole-object columns project four locator columns the executor reassembles
//! through the node codec.
//!
//! No openEHR spec governs the projection mechanics — this is our own design.
//! Every structure step a SELECT path descends through is bound once per row
//! ([`Builder::bind_select_paths`] states the row grain). A scalar column is
//! emitted as `to_jsonb(expr)`; a whole structure object emits (`vo_id`,
//! `sys_version`, `num`, `num_cap`) and is reassembled at read time.

use sea_query::{Alias, Expr, ExprTrait as _, Func};

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::{
    AggFunc, Coercion, LeafPath, PathTarget, QueryIr, SelectColumn, SelectValue, StructureStep,
};
use crate::db::iden::Node;

use super::expr::{col, leaf_path_string, literal_value, order_coercion, to_jsonb, type_cond};
use super::from::BoundEdge;
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
        let roots = self.served_roots();
        if ir.distinct || aggregates || roots.is_empty() {
            return Vec::new();
        }
        let mut aliases = Vec::with_capacity(roots.len());
        for (i, (root, _)) in roots.iter().enumerate() {
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
        let roots = self.served_roots();
        if ir.distinct || aggregates || roots.is_empty() {
            return Vec::new();
        }
        let mut aliases = Vec::with_capacity(roots.len());
        for (i, (root, _)) in roots.iter().enumerate() {
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
                let agg = match arg {
                    None => aggregate_expr(*func, None, *distinct),
                    Some(target) => self.aggregate_over(*func, target, *distinct)?,
                };
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

    /// Resolves a whole-object leaf to the alias of the node it addresses:
    /// the source node for an empty anchor, else the node
    /// [`Self::bind_select_paths`] bound for the full anchor.
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] when the anchor was not bound (a lowering
    /// defect: every SELECT path is bound before the columns are emitted).
    fn whole_object_alias(&self, leaf: &LeafPath) -> Result<String, AqlError> {
        let (start, rest) = self.walk_start(leaf)?;
        if !rest.is_empty() {
            return Err(SqlError::Unsupported(
                "a whole-object projection whose structure steps are unbound".to_owned(),
            )
            .into());
        }
        Ok(start)
    }

    /// Binds, once per row, every structure step the SELECT paths descend
    /// through (QUERY master03 §SELECT: "Where an identified path is specified,
    /// the data item(s) having that archetype path are returned").
    ///
    /// The row grain is our own design, since the specification does not say
    /// how rows multiply when a path crosses a multi-valued attribute: each
    /// structure step a SELECT path (a column or a function argument)
    /// descends through is bound as if it were a FROM variable, with [`Builder::left_bind`]. Paths rooted at the same
    /// variable that share a step prefix share its binding, so two leaves
    /// below one `EVENT` read the same `EVENT` in a row and never pair values
    /// of different events; a step matching several nodes yields one row per
    /// node (an archetype path "can correspond to more than one item in
    /// runtime data", BASE `architecture_overview/master11-paths.adoc`
    /// §Archetype path Predicate); a step matching none yields one row whose
    /// cells below it are `NULL`, so the projection never removes a row the
    /// FROM and WHERE clauses matched. WHERE and ORDER BY read through these
    /// bindings wherever their own path shares the bound prefix.
    ///
    /// # Errors
    /// [`AqlError`] from resolving a path's source node or a step predicate.
    pub(super) fn bind_select_paths(&mut self) -> Result<(), AqlError> {
        let mut leaves: Vec<(bool, LeafPath)> = Vec::new();
        let mut collect = |t: &PathTarget| match t {
            PathTarget::Data(leaf) => leaves.push((false, (**leaf).clone())),
            PathTarget::EhrStatus(leaf) => leaves.push((true, (**leaf).clone())),
            PathTarget::Version { .. } | PathTarget::Ehr { .. } => {}
        };
        // Aggregate arguments add no binding: an aggregate reads every data
        // item of its own path per row (`aggregate_over_items`).
        for column in &self.ir.select {
            if !matches!(column.value, SelectValue::Aggregate { .. }) {
                column.value.for_each_path(&mut collect);
            }
        }
        for (ehr_status, leaf) in leaves {
            if ehr_status {
                self.ensure_ehr_status_root(leaf.source.0)?;
            }
            if leaf.anchor.is_empty() {
                continue;
            }
            let mut parent = self.source_node(leaf.source.0)?;
            let mut prefix: Vec<StructureStep> = Vec::with_capacity(leaf.anchor.len());
            for step in &leaf.anchor {
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
                let alias = self.left_bind(
                    &parent,
                    BoundEdge::Attribute(&step.attribute),
                    |b, inner| {
                        let mut conds: Vec<Expr> =
                            type_cond(inner, &step.node_types).into_iter().collect();
                        if let Some(pred) = &step.predicate {
                            conds.extend(b.node_constraint_conds(inner, pred)?);
                        }
                        Ok(conds)
                    },
                )?;
                self.step_bindings.push(super::StepBinding {
                    source: leaf.source.0,
                    steps: prefix.clone(),
                    alias: alias.clone(),
                });
                parent = alias;
            }
        }
        Ok(())
    }

    /// The deepest node bound for `leaf`'s anchor prefix, and the anchor steps
    /// left below it.
    ///
    /// The start is the leaf's source node when nothing of its anchor is
    /// bound.
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] when the leaf's source has no node alias.
    pub(super) fn walk_start<'l>(
        &self,
        leaf: &'l LeafPath,
    ) -> Result<(String, &'l [StructureStep]), AqlError> {
        let mut start = self.source_node(leaf.source.0)?;
        let mut rest = leaf.anchor.as_slice();
        let mut depth = 0;
        for binding in &self.step_bindings {
            if binding.source != leaf.source.0
                || binding.steps.len() <= depth
                || !leaf.anchor.starts_with(&binding.steps)
            {
                continue;
            }
            let Some(below) = leaf.anchor.get(binding.steps.len()..) else {
                continue;
            };
            depth = binding.steps.len();
            start.clone_from(&binding.alias);
            rest = below;
        }
        Ok((start, rest))
    }
}

/// Builds an aggregate over every data item a path selects, across all rows
/// (QUERY master03 §Aggregate functions: the input values are "selected by an
/// expression in a form of an identified path applied to data items filtered
/// by `FROM` and `WHERE` clauses").
///
/// `items` is the per-row walk ([`Builder::leaf_items`]), already carrying the
/// WHERE clause for the steps it shares with it ([`Builder::where_filter_for`]),
/// and `item` the value of one item. Each row aggregates its own items in a correlated subquery and
/// the rows combine: `COUNT` sums the per-row counts (0 over no rows, QUERY
/// master03 §COUNT), `SUM` sums the sums, `MIN`/`MAX` take the extreme of the
/// extremes, `AVG` divides the summed sums by the summed counts, and
/// `COUNT(DISTINCT …)` collects each row's items as a jsonb array and counts
/// the distinct elements of all of them. The walk adds no row to the result,
/// so `COUNT(*)` and the other aggregates of the SELECT see the rows FROM and
/// WHERE define. No openEHR spec governs the lowering — our own design.
fn aggregate_over_items(
    func: AggFunc,
    distinct: bool,
    items: &sea_query::SelectStatement,
    item: &Expr,
) -> Expr {
    let per_row = |agg: Expr| -> Expr {
        let mut sub = items.clone();
        sub.expr(agg);
        Expr::from(sub)
    };
    let count = || per_row(Expr::from(Func::count(item.clone())));
    let sum = || per_row(Expr::from(Func::sum(item.clone())));
    match func {
        AggFunc::Count if distinct => {
            // NOTE: QUERY master03 §COUNT does not define value equality for DISTINCT;
            // ours is jsonb equality (numbers by value, so 120 and 120.0 are one).
            let mut sub = items.clone();
            sub.and_where(item.clone().is_not_null());
            sub.expr(Expr::cust_with_exprs(
                "COALESCE(jsonb_agg($1), '[]'::jsonb)",
                [item.clone()],
            ));
            Expr::cust_with_exprs(
                "(SELECT count(DISTINCT e.value) \
                 FROM jsonb_array_elements(COALESCE(jsonb_agg($1), '[]'::jsonb)) AS a, \
                 jsonb_array_elements(a.value) AS e)",
                [Expr::from(sub)],
            )
        }
        AggFunc::Count => Expr::cust_with_exprs("COALESCE(sum($1), 0)::int8", [count()]),
        AggFunc::Sum => Expr::from(Func::sum(sum())),
        AggFunc::Min => Expr::from(Func::min(per_row(Expr::from(Func::min(item.clone()))))),
        AggFunc::Max => Expr::from(Func::max(per_row(Expr::from(Func::max(item.clone()))))),
        // NOTE: QUERY master03 §AVG lets the input type "determine the return type";
        // the mean stays numeric, since an integer mean would truncate a clinical value.
        AggFunc::Avg => Expr::cust_with_exprs("sum($1) / NULLIF(sum($2), 0)", [sum(), count()]),
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

/// Reads a JSON `null` fragment value as SQL `NULL`: QUERY master03
/// §Aggregate functions, "Unless specified otherwise, these functions ignore
/// `NULL` input values", and a stored `null` is no input value.
fn json_null_is_null(value: Expr) -> Expr {
    Expr::cust_with_exprs("NULLIF($1, 'null'::jsonb)", [value])
}

impl Builder<'_> {
    /// The aggregate `func` over the values of `target` (QUERY master03
    /// §Aggregate functions).
    ///
    /// # Errors
    /// [`AqlError`] from lowering the argument.
    fn aggregate_over(
        &mut self,
        func: AggFunc,
        target: &PathTarget,
        distinct: bool,
    ) -> Result<Expr, AqlError> {
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
        let count = matches!(func, AggFunc::Count);
        // A plain count only asks whether each object is there;
        // a DISTINCT count compares the objects themselves.
        self.presence_only = count && !distinct;
        let walk = self.leaf_items(target, mode);
        self.presence_only = false;
        if let Some(mut walk) = walk? {
            if let Some(filter) = self.where_filter_for(&walk)? {
                walk.sub.and_where(filter);
            }
            let item = if count {
                json_null_is_null(walk.item.clone())
            } else {
                walk.item.clone()
            };
            Ok(aggregate_over_items(func, distinct, &walk.sub, &item))
        } else {
            self.presence_only = count && !distinct;
            let value = self.value_expr(target, mode);
            self.presence_only = false;
            let value = value?;
            let value = if count && self.reads_fragment(target, mode) {
                json_null_is_null(value)
            } else {
                value
            };
            Ok(aggregate_expr(func, Some(value), distinct))
        }
    }

    /// Whether `target` is read as a jsonb fragment value (a data path that is
    /// neither uid synthesis nor a promoted column).
    fn reads_fragment(&mut self, target: &PathTarget, mode: ValueMode) -> bool {
        match target {
            PathTarget::Data(leaf) | PathTarget::EhrStatus(leaf) => {
                !self.has_column_read(leaf, mode)
            }
            PathTarget::Version { .. } | PathTarget::Ehr { .. } => false,
        }
    }
}
