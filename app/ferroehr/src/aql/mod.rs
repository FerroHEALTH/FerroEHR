// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL execution engine — our own typed IR over the
//! greenfield node store.
//!
//! Spec authority: the vendored QUERY 1.1 text at
//! `docs/specs/openehr/QUERY/docs/AQL/`; the RM typing oracle is the generated
//! `openehr_rm::v1_2::model`.

pub mod access_log;
mod analyze;
pub mod cell;
pub mod constraints;
pub mod error;
pub mod exec;
pub mod fragment;
pub mod ir;
mod ir_impl;
pub mod join;
pub mod lineage;
mod lower;
mod read;
pub mod redact;
pub mod sql;
pub mod terminology;
mod version_path;

use std::collections::BTreeSet;

use openehr_query::ast::SelectQuery;

use error::{AnalysisError, AqlError};
use ir::{
    ArchetypeConstraint, Bind, Expr, NameConstraint, NodeConstraint, Operand, Params, PathTarget,
    QueryIr, SelectValue, Source, VersionScope,
};

/// Plan an AQL query: analyse and lower it into a typed [`QueryIr`], validating
/// that every referenced `$parameter` is supplied in `params`.
///
/// # Errors
///
/// * [`AqlError::Feature`] — a syntactically valid construct outside the
///   accepted feature envelope (each variant cites its QUERY spec section).
///   The refusals that depend on the FROM tree's shape come later, from
///   [`sql::build`].
/// * [`AqlError::Analysis`] — an unknown class/variable, an unresolvable
///   attribute, a type mismatch, or an unbound parameter.
pub fn plan(
    query: &SelectQuery,
    params: &Params,
    profile: crate::config::profile::SpecProfile,
) -> Result<QueryIr, AqlError> {
    let ir = lower_query(query, profile)?;
    check_params(&ir, params)?;
    Ok(ir)
}

/// Analyse and lower `query` into a typed [`QueryIr`], recording every
/// referenced `$parameter` name in [`QueryIr::params`] — **without** checking
/// that those parameters are supplied.
///
/// The IR is a pure function of the query AST, with no parameter value, paging
/// window, EHR scope or system id baked in, which lets the query service cache
/// it by query text.
///
/// # Errors
///
/// [`AqlError::Feature`] / [`AqlError::Analysis`] as [`plan`], minus the
/// unbound-parameter check (which is [`check_params`]).
pub fn lower_query(
    query: &SelectQuery,
    profile: crate::config::profile::SpecProfile,
) -> Result<QueryIr, AqlError> {
    let mut ir = lower::lower(query, profile)?;
    ir.params = collect_params(&ir);
    Ok(ir)
}

/// Validate that every `$parameter` referenced by `ir` (in [`QueryIr::params`],
/// populated by [`lower_query`]) is bound in `params`.
///
/// # Errors
///
/// [`AqlError::Analysis`] with [`AnalysisError::UnboundParameter`] for the first
/// referenced parameter with no supplied binding.
pub fn check_params(ir: &QueryIr, params: &Params) -> Result<(), AqlError> {
    for name in &ir.params {
        if !params.contains(name) {
            return Err(AnalysisError::UnboundParameter(name.clone()).into());
        }
    }
    Ok(())
}

/// Collect every `$parameter` name referenced anywhere in the IR (sorted, unique).
fn collect_params(ir: &QueryIr) -> Vec<String> {
    let uses = Uses::of(ir);
    uses.ehr_id.union(&uses.other).cloned().collect()
}

/// The `$parameter`s of `ir` that bind only an EHR id (sorted, unique).
///
/// Each is compared with `ehr_id/value` (the `EHR` class predicate, or a
/// `WHERE` comparison with the path) and with nothing else, so its value
/// reaches the statement as that id alone.
///
/// The SQL text of such a query does not depend on the id it names, which
/// lets a statement shape serve every EHR ([`sql::shape`]). No openEHR spec
/// governs this: our own design (#5783).
#[must_use]
pub fn ehr_id_params(ir: &QueryIr) -> Vec<String> {
    let uses = Uses::of(ir);
    uses.ehr_id.difference(&uses.other).cloned().collect()
}

/// Where the `$parameter`s of a plan are read: as an EHR id, or anywhere
/// else.
#[derive(Default)]
struct Uses {
    ehr_id: BTreeSet<String>,
    other: BTreeSet<String>,
}

impl Uses {
    fn of(ir: &QueryIr) -> Self {
        let mut uses = Self::default();
        for source in &ir.sources {
            uses.source(source);
        }
        if let Some(filter) = &ir.filter {
            uses.expr(filter);
        }
        for col in &ir.select {
            uses.select(&col.value);
        }
        for key in &ir.order_by {
            uses.path_target(&key.path);
        }
        uses
    }

    fn source(&mut self, source: &Source) {
        match source {
            Source::Ehr(s) => {
                for p in &s.predicates {
                    match (&p.field, &p.value) {
                        (ir::EhrField::EhrId, Bind::Param(name)) => {
                            self.ehr_id.insert(name.clone());
                        }
                        _ => self.bind(&p.value),
                    }
                }
            }
            Source::Rm(s) => {
                if let Some(a) = &s.archetype {
                    self.archetype(a);
                }
                if let Some(n) = &s.name {
                    self.name(n);
                }
                for sp in &s.standard {
                    self.bind(&sp.value);
                }
                self.scope(&s.scope);
            }
            Source::Version(s) => self.scope(&s.scope),
        }
    }

    fn scope(&mut self, scope: &VersionScope) {
        if let VersionScope::Predicate(p) = scope {
            self.bind(&p.value);
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Compare { lhs, rhs, .. } => match (lhs, rhs) {
                (Operand::Path(path), Operand::Param(name))
                | (Operand::Param(name), Operand::Path(path))
                    if is_ehr_id(path) =>
                {
                    self.ehr_id.insert(name.clone());
                }
                _ => {
                    self.operand(lhs);
                    self.operand(rhs);
                }
            },
            Expr::Exists(t) => self.path_target(t),
            Expr::Const(_) => {}
            Expr::Like { path, pattern } => {
                self.path_target(path);
                if let ir::LikePattern::Param(p) = pattern {
                    self.other.insert(p.clone());
                }
            }
            Expr::Matches { path, values, .. } => {
                self.path_target(path);
                for v in values {
                    self.bind(v);
                }
            }
            Expr::And(a, b) | Expr::Or(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::Not(a) => self.expr(a),
        }
    }

    fn operand(&mut self, op: &Operand) {
        match op {
            Operand::Path(t) => self.path_target(t),
            Operand::Param(p) => {
                self.other.insert(p.clone());
            }
            Operand::Function { args, .. } => {
                for a in args {
                    self.operand(a);
                }
            }
            Operand::Literal(_) => {}
        }
    }

    fn select(&mut self, value: &SelectValue) {
        match value {
            SelectValue::Path(t) => self.path_target(t),
            SelectValue::Aggregate { arg, .. } => {
                if let Some(t) = arg {
                    self.path_target(t);
                }
            }
            SelectValue::Function { args, .. } => {
                for a in args {
                    self.operand(a);
                }
            }
            SelectValue::Literal(_) => {}
        }
    }

    fn path_target(&mut self, target: &PathTarget) {
        let leaf = match target {
            PathTarget::Data(leaf)
            | PathTarget::EhrStatus(leaf)
            | PathTarget::Attestations { leaf, .. } => leaf,
            PathTarget::Version { .. } | PathTarget::Ehr { .. } => return,
        };
        let anchor = leaf
            .anchor
            .iter()
            .filter_map(|step| step.predicate.as_ref());
        let fragment = leaf
            .fragment
            .iter()
            .filter_map(|step| step.predicate.as_ref());
        for c in leaf.root_predicate.iter().chain(anchor).chain(fragment) {
            self.node_constraint(c);
        }
    }

    fn node_constraint(&mut self, c: &NodeConstraint) {
        if let Some(a) = &c.archetype {
            self.archetype(a);
        }
        if let Some(n) = &c.name {
            self.name(n);
        }
        for sp in &c.standard {
            self.bind(&sp.value);
        }
    }

    fn archetype(&mut self, a: &ArchetypeConstraint) {
        if let ArchetypeConstraint::Param(p) = a {
            self.other.insert(p.clone());
        }
    }

    fn name(&mut self, n: &NameConstraint) {
        if let NameConstraint::Param(p) = n {
            self.other.insert(p.clone());
        }
    }

    fn bind(&mut self, b: &Bind) {
        if let Bind::Param(p) = b {
            self.other.insert(p.clone());
        }
    }
}

/// Whether `path` is an EHR's id: `e/ehr_id/value`, or the bare `e`, which a
/// comparison reads as its id.
fn is_ehr_id(path: &PathTarget) -> bool {
    matches!(
        path,
        PathTarget::Ehr {
            field: ir::EhrField::EhrId | ir::EhrField::Whole,
            ..
        }
    )
}
