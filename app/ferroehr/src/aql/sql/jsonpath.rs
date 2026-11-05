// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SQL/JSON path text the emitter writes for a fragment path and its
//! filter expressions (PostgreSQL 18 docs §9.16.2, "The SQL/JSON Path
//! Language").
//!
//! Member names come from lexer-restricted identifiers and every compared
//! value travels in the bound `vars` jsonb (`$p0`, `$p1`, …), never in the
//! path text. No openEHR spec governs the lowering: our own design.

use std::fmt::Write as _;

use openehr_query::lexer::CompOp;
use sea_query::Expr;

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::StdPredicate;

use super::Builder;
use super::expr::cast;

impl Builder<'_> {
    /// The filter conditions of standard predicates relative to the current
    /// item `@` (`@.is_pending == $p0`), each value added to `vars` (QUERY
    /// master03 §Standard predicate).
    ///
    /// # Errors
    /// [`AqlError`] when a parameter is unbound or a value is no scalar.
    #[expect(
        clippy::disallowed_types,
        reason = "the jsonpath vars object is SQL/JSON wire material (the bound \
                  third argument of jsonb_path_query*), not an RM value — no \
                  generated type models it"
    )]
    pub(super) fn standard_conds(
        &self,
        preds: &[StdPredicate],
        vars: &mut serde_json::Map<String, serde_json::Value>,
    ) -> Result<Vec<String>, AqlError> {
        let mut conds = Vec::with_capacity(preds.len());
        for sp in preds {
            let value = jsonpath_scalar(self.bind_value(&sp.value)?)?;
            let v = jsonpath_var(value, vars);
            let mut lhs = String::from("@");
            for part in &sp.path {
                let _ = write!(lhs, ".{part}");
            }
            conds.push(format!("{lhs} {} ${v}", jsonpath_op(sp.op)));
        }
        Ok(conds)
    }
}

/// Adds `value` to `vars` under the next free name (`p0`, `p1`, …) and
/// returns that name.
#[expect(
    clippy::disallowed_types,
    reason = "the jsonpath vars object is SQL/JSON wire material (the bound \
              third argument of jsonb_path_query*), not an RM value — no \
              generated type models it"
)]
pub(super) fn jsonpath_var(
    value: serde_json::Value,
    vars: &mut serde_json::Map<String, serde_json::Value>,
) -> String {
    let name = format!("p{}", vars.len());
    vars.insert(name.clone(), value);
    name
}

/// The bound `vars` jsonb argument of a jsonpath call, `None` when the path
/// compares nothing.
#[expect(
    clippy::disallowed_types,
    reason = "the jsonpath vars object is SQL/JSON wire material (the bound \
              third argument of jsonb_path_query*), not an RM value — no \
              generated type models it"
)]
pub(super) fn jsonpath_vars(vars: serde_json::Map<String, serde_json::Value>) -> Option<Expr> {
    (!vars.is_empty()).then(|| {
        cast(
            Expr::val(serde_json::Value::Object(vars).to_string()),
            "jsonb",
        )
    })
}

/// The jsonpath filter-expression operator for an AQL comparison operator
/// (PostgreSQL docs §9.16.2 — the SQL/JSON path comparison operators).
fn jsonpath_op(op: CompOp) -> &'static str {
    match op {
        CompOp::Eq => "==",
        CompOp::Ne => "!=",
        CompOp::Lt => "<",
        CompOp::Le => "<=",
        CompOp::Gt => ">",
        CompOp::Ge => ">=",
    }
}

/// A bound predicate value as the JSON scalar a jsonpath `vars` object
/// carries. A non-scalar bind cannot appear in a filter comparison and is a
/// typed reject.
#[expect(
    clippy::disallowed_types,
    reason = "the jsonpath vars object is SQL/JSON wire material (the bound \
              third argument of jsonb_path_query*), not an RM value — no \
              generated type models it"
)]
fn jsonpath_scalar(value: sea_query::Value) -> Result<serde_json::Value, AqlError> {
    Ok(match value {
        sea_query::Value::String(Some(s)) => serde_json::Value::String(s),
        sea_query::Value::Bool(Some(b)) => serde_json::Value::Bool(b),
        sea_query::Value::BigInt(Some(i)) => serde_json::Value::from(i),
        sea_query::Value::Int(Some(i)) => serde_json::Value::from(i),
        sea_query::Value::Double(Some(f)) => serde_json::Value::from(f),
        other => {
            return Err(SqlError::Unsupported(format!(
                "a fragment-step predicate compares against a non-scalar value ({other:?})"
            ))
            .into());
        }
    })
}

/// Build a jsonpath from a relative object path (`[a, b]` → `$.a.b`) — a node
/// standard-predicate sub-path (QUERY master03 §Standard predicate).
pub(super) fn jsonpath(parts: &[String]) -> String {
    let mut jp = String::from("$");
    for p in parts {
        let _ = write!(jp, ".{p}");
    }
    jp
}
