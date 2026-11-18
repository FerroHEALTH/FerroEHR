// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The projected cells rendered in Rust: the statement returns stored and
//! typed values, never a `jsonb` it builds, and the executor writes each cell
//! once ([`crate::aql::exec`]).
//!
//! A data leaf returns the stored columns of the node row it reads, once per
//! node alias however many cells read it ([`NodeGroup`]); its fragment is
//! rebuilt and its path evaluated in Rust. A VERSION or EHR field, a function
//! and an aggregate return their typed value, a `timestamptz` beside its
//! session offset; a literal returns nothing. A `SELECT DISTINCT` keeps its
//! cells in SQL, where PostgreSQL compares them, and so does a list of
//! attestations, which PostgreSQL aggregates. No openEHR spec governs the
//! projection mechanics: our own design.

use sea_query::{Alias, Expr};

use crate::aql::error::AqlError;
use crate::aql::ir::{
    AggFunc, Coercion, EhrField, PathTarget, SelectColumn, SelectValue, VersionField,
};
use crate::db::iden::Node;

use super::compact::root_columns;
use super::expr::{cast, col, leaf_path_string, order_coercion};
use super::{Builder, CellKind, ColumnRender, ColumnSpec, FragmentCell, NodeGroup, ValueMode};

/// A rendered cell: its kind, its SQL columns and its fallback
/// `RESULT_SET` path.
type Rendered = (CellKind, Vec<String>, Option<String>);

impl Builder<'_> {
    /// The cell of SELECT column `i` rendered in Rust, or `None` when the
    /// column keeps its `jsonb` in SQL.
    ///
    /// # Errors
    /// [`AqlError`] from lowering the column's value.
    pub(super) fn rendered_column(
        &mut self,
        i: usize,
        column: &SelectColumn,
    ) -> Result<Option<ColumnSpec>, AqlError> {
        if self.ir.distinct {
            return Ok(None);
        }
        let name = column.alias.clone().unwrap_or_else(|| format!("#{i}"));
        let spec = |kind, sql_cols, path| ColumnSpec {
            name: name.clone(),
            path,
            kind,
            sql_cols,
            reference: None,
        };
        Ok(match &column.value {
            SelectValue::Literal(lit) => {
                Some(spec(CellKind::Literal(lit.clone()), Vec::new(), None))
            }
            SelectValue::Function { func, args } => {
                let expr = self.scalar_fn_expr(*func, args)?;
                Some(spec(
                    CellKind::Column(ColumnRender::Typed),
                    self.typed_columns(i, expr, false),
                    None,
                ))
            }
            SelectValue::Aggregate {
                func,
                arg,
                distinct,
            } => {
                let temporal = matches!(func, AggFunc::Min | AggFunc::Max)
                    && arg
                        .as_ref()
                        .is_some_and(|t| order_coercion(t) == Coercion::Temporal);
                let expr = self.aggregate_value(*func, arg.as_ref(), *distinct)?;
                let render = if temporal {
                    ColumnRender::Timestamp
                } else {
                    ColumnRender::Typed
                };
                Some(spec(
                    CellKind::Column(render),
                    self.typed_columns(i, expr, temporal),
                    None,
                ))
            }
            SelectValue::Path(target) => self
                .rendered_path(i, target)?
                .map(|(kind, cols, path)| spec(kind, cols, column.path.clone().or(path))),
        })
    }

    /// The cell of a projected path rendered in Rust, with its SQL columns
    /// and its fallback `RESULT_SET` path, or `None` for an attestation path
    /// and a data leaf with unbound structure steps.
    fn rendered_path(
        &mut self,
        i: usize,
        target: &PathTarget,
    ) -> Result<Option<Rendered>, AqlError> {
        let leaf = match target {
            PathTarget::Data(leaf) => leaf,
            PathTarget::EhrStatus(leaf) => {
                self.ensure_ehr_status_root(leaf.source.0)?;
                leaf
            }
            PathTarget::Version { field, .. } => {
                let temporal = *field == VersionField::TimeCommitted;
                return Ok(Some(self.typed_path(i, target, temporal)?));
            }
            PathTarget::Ehr { field, .. } => {
                let temporal = *field == EhrField::TimeCreated;
                return Ok(Some(self.typed_path(i, target, temporal)?));
            }
            PathTarget::Attestations { .. } => return Ok(None),
        };
        let path = matches!(target, PathTarget::Data(_)).then(|| leaf_path_string(leaf));
        if let Some((uid, whole)) = self.version_uid_text(leaf) {
            let render = if whole {
                ColumnRender::VersionUid
            } else {
                ColumnRender::Typed
            };
            let cols = self.typed_columns(i, uid, false);
            return Ok(Some((CellKind::Column(render), cols, path)));
        }
        let (start, rest) = self.walk_start(leaf)?;
        if !rest.is_empty() {
            return Ok(None);
        }
        let mut sql_cols = Vec::new();
        let guarded = match &leaf.root_predicate {
            Some(pred) => {
                let src = self.source_node(leaf.source.0)?;
                let cond = self
                    .node_constraint_conds(&src, pred)?
                    .into_iter()
                    .reduce(sea_query::ExprTrait::and);
                if let Some(cond) = cond {
                    let alias = format!("col{i}");
                    self.q.expr_as(cond, Alias::new(alias.as_str()));
                    sql_cols.push(alias);
                }
                !sql_cols.is_empty()
            }
            None => false,
        };
        let program = self.fragment_program(leaf)?;
        let list = program.is_some() && leaf.fragment_multi_valued();
        let group = self.node_group(&start);
        let cell = FragmentCell {
            group,
            path: program,
            list,
            guarded,
        };
        Ok(Some((CellKind::Fragment(cell), sql_cols, path)))
    }

    /// The typed columns of a VERSION or EHR field.
    fn typed_path(
        &mut self,
        i: usize,
        target: &PathTarget,
        temporal: bool,
    ) -> Result<Rendered, AqlError> {
        let expr = self.value_expr(target, ValueMode::Projection)?;
        let render = if temporal {
            ColumnRender::Timestamp
        } else {
            ColumnRender::Typed
        };
        Ok((
            CellKind::Column(render),
            self.typed_columns(i, expr, temporal),
            None,
        ))
    }

    /// Selects `expr` as column `col{i}` and, for a `timestamptz`, its
    /// session offset in seconds as `col{i}_tz`.
    fn typed_columns(&mut self, i: usize, expr: Expr, temporal: bool) -> Vec<String> {
        let value = format!("col{i}");
        let mut cols = Vec::with_capacity(2);
        if temporal {
            let offset = format!("col{i}_tz");
            self.q.expr_as(
                cast(
                    Expr::cust_with_exprs("extract(timezone FROM $1)", [expr.clone()]),
                    "int4",
                ),
                Alias::new(offset.as_str()),
            );
            self.q.expr_as(expr, Alias::new(value.as_str()));
            cols.push(value);
            cols.push(offset);
        } else {
            self.q.expr_as(expr, Alias::new(value.as_str()));
            cols.push(value);
        }
        cols
    }

    /// The index of the node group of `alias`, selecting its columns on
    /// first use.
    fn node_group(&mut self, alias: &str) -> usize {
        if let Some(at) = self.node_groups.iter().position(|(held, _)| held == alias) {
            return at;
        }
        let at = self.node_groups.len();
        let root = self.root_version_alias(alias);
        let (version_uid, template_id) = root_columns(alias, root.as_deref());
        let columns = super::expr::node_columns(
            alias,
            cast(col(alias, Node::ValueNum), "text"),
            version_uid,
            template_id,
        );
        for (n, expr) in columns.into_iter().enumerate() {
            self.q
                .expr_as(expr, Alias::new(format!("g{at}_{n}").as_str()));
        }
        self.node_groups.push((
            alias.to_owned(),
            NodeGroup {
                first: format!("g{at}_0"),
            },
        ));
        at
    }
}
