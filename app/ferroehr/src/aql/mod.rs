// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL execution engine — our own typed IR over the
//! greenfield node store.
//!
//! This module is the planning front half of the engine: it turns a parsed
//! [`openehr_query::ast::SelectQuery`] into a typed [`ir::QueryIr`] through path
//! analysis (`analyze`) and AST-to-IR lowering (`lower`). The back half lives in
//! the SQL package, and the IR carries no SQL.
//!
//! Spec authority: the vendored QUERY 1.1 text at
//! `docs/specs/openehr/QUERY/docs/AQL/`; the RM typing oracle is the generated
//! `openehr_rm::v1_2::model`.
//!
//! Entry point: [`plan`].

mod analyze;
pub mod error;
pub mod exec;
pub mod ir;
pub mod lineage;
mod lower;
pub mod sql;
pub mod terminology;

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
/// This is the single entry point of the planning package. It never touches the
/// database and never produces SQL.
///
/// # Errors
///
/// * [`AqlError::Feature`] — a syntactically valid construct outside the
///   accepted feature envelope (each variant cites its QUERY spec section).
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
/// This is the request-independent half of [`plan`]: the IR is a pure,
/// deterministic function of the query AST (no parameter *value*, paging
/// window, EHR scope, or system id is baked in — those bind at SQL-build time,
/// [`sql::build`]). That purity is what lets the query service cache the lowered
/// IR keyed on the query text ([`crate::service::query`] plan cache); the
/// per-request [`check_params`] then runs against the caller's [`Params`].
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
    let mut out = BTreeSet::new();
    for source in &ir.sources {
        collect_source(source, &mut out);
    }
    if let Some(filter) = &ir.filter {
        collect_expr(filter, &mut out);
    }
    for col in &ir.select {
        collect_select(&col.value, &mut out);
    }
    for key in &ir.order_by {
        collect_path_target(&key.path, &mut out);
    }
    out.into_iter().collect()
}

fn collect_source(source: &Source, out: &mut BTreeSet<String>) {
    match source {
        Source::Ehr(s) => {
            for p in &s.predicates {
                collect_bind(&p.value, out);
            }
        }
        Source::Rm(s) => {
            if let Some(a) = &s.archetype {
                collect_archetype(a, out);
            }
            if let Some(n) = &s.name {
                collect_name(n, out);
            }
            for sp in &s.standard {
                collect_bind(&sp.value, out);
            }
            collect_scope(&s.scope, out);
        }
        Source::Version(s) => collect_scope(&s.scope, out),
    }
}

fn collect_scope(scope: &VersionScope, out: &mut BTreeSet<String>) {
    if let VersionScope::Predicate(p) = scope {
        collect_bind(&p.value, out);
    }
}

fn collect_expr(expr: &Expr, out: &mut BTreeSet<String>) {
    match expr {
        Expr::Compare { lhs, rhs, .. } => {
            collect_operand(lhs, out);
            collect_operand(rhs, out);
        }
        Expr::Exists(t) => collect_path_target(t, out),
        Expr::Const(_) => {}
        Expr::Like { path, pattern } => {
            collect_path_target(path, out);
            if let ir::LikePattern::Param(p) = pattern {
                out.insert(p.clone());
            }
        }
        Expr::Matches { path, values, .. } => {
            collect_path_target(path, out);
            for v in values {
                collect_bind(v, out);
            }
        }
        Expr::And(a, b) | Expr::Or(a, b) => {
            collect_expr(a, out);
            collect_expr(b, out);
        }
        Expr::Not(a) => collect_expr(a, out),
    }
}

fn collect_operand(op: &Operand, out: &mut BTreeSet<String>) {
    match op {
        Operand::Path(t) => collect_path_target(t, out),
        Operand::Param(p) => {
            out.insert(p.clone());
        }
        Operand::Function { args, .. } => {
            for a in args {
                collect_operand(a, out);
            }
        }
        Operand::Literal(_) => {}
    }
}

fn collect_select(value: &SelectValue, out: &mut BTreeSet<String>) {
    match value {
        SelectValue::Path(t) => collect_path_target(t, out),
        SelectValue::Aggregate { arg, .. } => {
            if let Some(t) = arg {
                collect_path_target(t, out);
            }
        }
        SelectValue::Function { args, .. } => {
            for a in args {
                collect_operand(a, out);
            }
        }
        SelectValue::Literal(_) => {}
    }
}

fn collect_path_target(target: &PathTarget, out: &mut BTreeSet<String>) {
    let leaf = match target {
        PathTarget::Data(leaf) | PathTarget::EhrStatus(leaf) => leaf,
        PathTarget::Version { .. } | PathTarget::Ehr { .. } => return,
    };
    if let Some(c) = &leaf.root_predicate {
        collect_node_constraint(c, out);
    }
    for step in &leaf.anchor {
        if let Some(c) = &step.predicate {
            collect_node_constraint(c, out);
        }
    }
    for step in &leaf.fragment {
        if let Some(c) = &step.predicate {
            collect_node_constraint(c, out);
        }
    }
}

fn collect_node_constraint(c: &NodeConstraint, out: &mut BTreeSet<String>) {
    if let Some(a) = &c.archetype {
        collect_archetype(a, out);
    }
    if let Some(n) = &c.name {
        collect_name(n, out);
    }
    for sp in &c.standard {
        collect_bind(&sp.value, out);
    }
}

fn collect_archetype(a: &ArchetypeConstraint, out: &mut BTreeSet<String>) {
    if let ArchetypeConstraint::Param(p) = a {
        out.insert(p.clone());
    }
}

fn collect_name(n: &NameConstraint, out: &mut BTreeSet<String>) {
    if let NameConstraint::Param(p) = n {
        out.insert(p.clone());
    }
}

fn collect_bind(b: &Bind, out: &mut BTreeSet<String>) {
    if let Bind::Param(p) = b {
        out.insert(p.clone());
    }
}
