// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Behaviour on the typed query IR ([`super::ir`]): the comparison strategy of
//! a version field and the identified-path visitors.

use super::ir::{Coercion, Expr, Operand, PathTarget, QueryIr, SelectValue, VersionField};

impl VersionField {
    /// The typed comparison strategy the field's column reads under: the
    /// commit instant is temporal, `is_pending` boolean, every other field
    /// text.
    #[must_use]
    pub fn coercion(self) -> Coercion {
        match self {
            VersionField::TimeCommitted => Coercion::Temporal,
            VersionField::CommitAuditIsPending => Coercion::Boolean,
            _ => Coercion::Text,
        }
    }
}

// ── identified-path visitors ─────────────────────────────────────────────────

impl Operand {
    /// Calls `f` on every identified path of this operand, nested function
    /// arguments included.
    pub fn for_each_path(&self, f: &mut impl FnMut(&PathTarget)) {
        match self {
            Operand::Path(t) => f(t),
            Operand::Function { args, .. } => args.iter().for_each(|a| a.for_each_path(f)),
            Operand::Literal(_) | Operand::Param(_) => {}
        }
    }
}

impl Expr {
    /// Calls `f` on every identified path of this predicate tree.
    pub fn for_each_path(&self, f: &mut impl FnMut(&PathTarget)) {
        match self {
            Expr::Compare { lhs, rhs, .. } => {
                lhs.for_each_path(f);
                rhs.for_each_path(f);
            }
            Expr::Exists(t) | Expr::Like { path: t, .. } | Expr::Matches { path: t, .. } => f(t),
            Expr::Const(_) => {}
            Expr::And(a, b) | Expr::Or(a, b) => {
                a.for_each_path(f);
                b.for_each_path(f);
            }
            Expr::Not(a) => a.for_each_path(f),
        }
    }
}

impl SelectValue {
    /// Calls `f` on every identified path of this column: the path itself, an
    /// aggregate's argument, or a function's arguments.
    pub fn for_each_path(&self, f: &mut impl FnMut(&PathTarget)) {
        match self {
            SelectValue::Path(t) | SelectValue::Aggregate { arg: Some(t), .. } => f(t),
            SelectValue::Function { args, .. } => args.iter().for_each(|a| a.for_each_path(f)),
            SelectValue::Aggregate { arg: None, .. } | SelectValue::Literal(_) => {}
        }
    }
}

impl QueryIr {
    /// Calls `f` on every identified path the query reads: the SELECT
    /// columns, the WHERE predicate and the ORDER BY keys.
    pub fn for_each_path(&self, f: &mut impl FnMut(&PathTarget)) {
        for column in &self.select {
            column.value.for_each_path(f);
        }
        if let Some(filter) = &self.filter {
            filter.for_each_path(f);
        }
        for key in &self.order_by {
            f(&key.path);
        }
    }
}
