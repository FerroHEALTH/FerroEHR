// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The scalar WHERE tests (QUERY master03 §WHERE, §Operators), the
//! whitelisted scalar functions (§Functions), and the node/source predicate
//! conditions (§Standard predicate, §Node predicate).
//!
//! No openEHR spec governs the SQL shapes — this is our own design. Every
//! operator maps to a typed `sea-query` binary op; string functions extract as
//! text and numeric functions through the magnitude coercion, following each
//! function's declared signature. The WHERE clause's boolean structure and
//! its step bindings live in the `condition` module.

use sea_query::{Expr, ExprTrait};

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::{
    ArchetypeConstraint, Bind, Coercion, EhrField, LikePattern, NameConstraint, NodeConstraint,
    Operand, PathTarget, StdPredicate, TypedLit,
};
use crate::db::iden::{Ehr, Node};
use openehr_query::ast::{BuiltinFunction, DateTimeFunction, NumericFunction, StringFunction};
use openehr_query::lexer::CompOp;

use super::expr::{
    aql_like_to_sql, archetype_predicate, as_text, binoper, cast, col, fragment_source, jsonb_path,
};
use super::value::{coerce_rhs, jsonpath};
use super::{Builder, ValueMode};

impl Builder<'_> {
    // ── WHERE ──────────────────────────────────────────────────────────────────

    /// Adds the WHERE clause ([`Self::where_condition`]).
    ///
    /// # Errors
    /// [`AqlError`] from the path, parameter and function lowerings.
    pub(super) fn build_where(&mut self) -> Result<(), AqlError> {
        if let Some(cond) = self.where_condition()? {
            self.q.and_where(cond);
        }
        Ok(())
    }

    /// Lowers one comparison, `EXISTS`, `LIKE` or `matches` to its scalar
    /// shape: the path reads one value per row (the bound node, or the first
    /// match below it) and the test applies to that value.
    ///
    /// # Errors
    /// [`AqlError`] from the operand and parameter lowerings.
    pub(super) fn scalar_test(&mut self, expr: &crate::aql::ir::Expr) -> Result<Expr, AqlError> {
        match expr {
            crate::aql::ir::Expr::Compare {
                lhs,
                op,
                rhs,
                coercion,
            } => self.compare_expr(lhs, *op, rhs, *coercion),
            crate::aql::ir::Expr::Exists(target) => Ok(self
                .value_expr(target, ValueMode::Projection)?
                .is_not_null()),
            crate::aql::ir::Expr::Like { path, pattern } => {
                let pat = self.like_pattern(pattern)?;
                Ok(self
                    .value_expr(path, ValueMode::Value(Coercion::Text))?
                    .like(pat))
            }
            crate::aql::ir::Expr::Matches {
                path,
                values,
                coercion,
            } => {
                let (mode, members) = self.matches_members(values, *coercion)?;
                Ok(self.value_expr(path, mode)?.is_in(members))
            }
            crate::aql::ir::Expr::And(..)
            | crate::aql::ir::Expr::Or(..)
            | crate::aql::ir::Expr::Not(_)
            | crate::aql::ir::Expr::Const(_) => Err(SqlError::Unsupported(
                "a compound expression as a scalar test".to_owned(),
            )
            .into()),
        }
    }

    /// Lowers a comparison: the typed EHR-id fast path, else the generic
    /// operand comparison.
    ///
    /// # Errors
    /// [`AqlError`] from the operand lowerings.
    pub(super) fn compare_expr(
        &mut self,
        lhs: &Operand,
        op: CompOp,
        rhs: &Operand,
        coercion: Coercion,
    ) -> Result<Expr, AqlError> {
        // `e/ehr_id/value = <uuid>` compares as uuid on `ehr.id`, keeping the
        // btree usable where a two-sided text cast would not. uuid equality is
        // case-insensitive, which is also the identifier-equality semantics
        // (BASE base_types master05 §Composite Identifiers and Case).
        if let Some(expr) = self.ehr_id_typed_compare(lhs, op, rhs)? {
            return Ok(expr);
        }
        // A mixed-type (`Raw`) leaf compared to a numeric literal is compared
        // numerically (with a NULL-guard); otherwise the Raw set falls through
        // to text, exactly as every other Text leaf (QUERY master03
        // §Comparison operators).
        let (l, r) = if coercion == Coercion::Raw && raw_numeric_wanted(&[lhs, rhs]) {
            (
                self.operand_value_raw_numeric(lhs)?,
                self.operand_value_raw_numeric(rhs)?,
            )
        } else {
            (
                self.operand_value(lhs, coercion)?,
                self.operand_value(rhs, coercion)?,
            )
        };
        Ok(l.binary(binoper(op), r))
    }

    /// The SQL `LIKE` pattern of an AQL pattern (QUERY master03
    /// §Operators/LIKE).
    ///
    /// # Errors
    /// [`SqlError::UnboundParameter`] for an unbound pattern parameter.
    pub(super) fn like_pattern(&self, pattern: &LikePattern) -> Result<String, AqlError> {
        Ok(match pattern {
            LikePattern::Literal(s) => aql_like_to_sql(s),
            LikePattern::Param(p) => aql_like_to_sql(&self.param_str(p)?),
        })
    }

    /// The extraction mode and the coerced members of a `matches` value list
    /// (QUERY master03 §matches): a mixed-type leaf matched against numeric
    /// literals compares numerically.
    ///
    /// # Errors
    /// [`AqlError`] from binding or coercing a member.
    pub(super) fn matches_members(
        &self,
        values: &[Bind],
        coercion: Coercion,
    ) -> Result<(ValueMode, Vec<Expr>), AqlError> {
        let numeric = coercion == Coercion::Raw
            && values
                .iter()
                .any(|b| matches!(b, Bind::Literal(TypedLit::Integer(_) | TypedLit::Real(_))));
        let mode = if numeric {
            ValueMode::RawNumeric
        } else {
            ValueMode::Value(coercion)
        };
        let mut members = Vec::with_capacity(values.len());
        for b in values {
            let v = self.bind_value(b)?;
            members.push(if numeric {
                cast(Expr::val(v), "numeric")
            } else {
                coerce_rhs(v, coercion)?
            });
        }
        Ok((mode, members))
    }

    /// The typed EHR-id comparison fast path: fires only for
    /// `Eq`/`Ne` between an `EHR.ehr_id/value` path and a string literal or
    /// parameter. Returns `None` (fall through to the generic text lowering)
    /// for every other shape — ordering comparisons stay textual (uuid byte
    /// order differs from text order, and the spec defines string comparison).
    fn ehr_id_typed_compare(
        &mut self,
        lhs: &Operand,
        op: CompOp,
        rhs: &Operand,
    ) -> Result<Option<Expr>, AqlError> {
        if !matches!(op, CompOp::Eq | CompOp::Ne) {
            return Ok(None);
        }
        let (path, other, flipped) = match (lhs, rhs) {
            (Operand::Path(t), o) if Self::is_ehr_id_target(t) => (t, o, false),
            (o, Operand::Path(t)) if Self::is_ehr_id_target(t) => (t, o, true),
            _ => return Ok(None),
        };
        let _ = flipped; // equality/inequality are symmetric
        let raw = match other {
            Operand::Literal(TypedLit::String(sv)) => sv.clone(),
            Operand::Param(p) => self.param_str(p)?,
            _ => return Ok(None),
        };
        let PathTarget::Ehr { source, .. } = path else {
            return Ok(None);
        };
        let alias = self
            .ehr_alias
            .get(&source.0)
            .cloned()
            .ok_or_else(|| SqlError::Unsupported("EHR path without a bound EHR".to_owned()))?;
        Ok(Some(match raw.parse::<uuid::Uuid>() {
            Ok(u) => {
                let rhs_uuid = cast(Expr::val(u.to_string()), "uuid");
                col(&alias, Ehr::Id).binary(binoper(op), rhs_uuid)
            }
            // Not a uuid → it can equal no EHR id: `=` is constant false,
            // `!=` constant true.
            Err(_) => Expr::val(matches!(op, CompOp::Ne)),
        }))
    }

    /// Whether a path target addresses the EHR id (`e/ehr_id/value` or the
    /// bare `e/ehr_id`).
    fn is_ehr_id_target(t: &PathTarget) -> bool {
        matches!(
            t,
            PathTarget::Ehr {
                field: EhrField::EhrId | EhrField::Whole,
                ..
            }
        )
    }

    /// Render a whitelisted scalar function call (QUERY master03 §Functions) to
    /// `PostgreSQL`. Arity was validated at lowering; argument typing follows each
    /// function's declared signature (string args extract as text, numeric args
    /// through the magnitude coercion).
    pub(super) fn scalar_fn_expr(
        &mut self,
        func: BuiltinFunction,
        args: &[Operand],
    ) -> Result<Expr, AqlError> {
        // Arity was validated at lowering, but the argument is still fetched
        // rather than indexed: a lowering defect must surface as a typed reject,
        // never a panic on a request path.
        let arg = |i: usize| -> Result<&Operand, AqlError> {
            args.get(i).ok_or_else(|| {
                AqlError::from(SqlError::Unsupported(format!(
                    "{} called with {} argument(s); argument {i} is missing",
                    func.as_str(),
                    args.len()
                )))
            })
        };
        let text = |this: &mut Self, i: usize| this.operand_value(arg(i)?, Coercion::Text);
        let num = |this: &mut Self, i: usize| this.operand_value(arg(i)?, Coercion::Magnitude);
        Ok(match func {
            BuiltinFunction::String(StringFunction::Length) => {
                Expr::cust_with_exprs("length($1)", [text(self, 0)?])
            }
            // SUBSTRING(expression, position[, length]) — 1-based positions,
            // omitted length extracts to end-of-string (PG substr matches).
            BuiltinFunction::String(StringFunction::Substring) => match args.len() {
                2 => {
                    Expr::cust_with_exprs("substr($1, ($2)::int4)", [text(self, 0)?, num(self, 1)?])
                }
                _ => Expr::cust_with_exprs(
                    "substr($1, ($2)::int4, ($3)::int4)",
                    [text(self, 0)?, num(self, 1)?, num(self, 2)?],
                ),
            },
            // POSITION(substring, expression): 1-based index of the first
            // occurrence, 0 when absent — exactly PG strpos(expression, sub).
            BuiltinFunction::String(StringFunction::Position) => {
                Expr::cust_with_exprs("strpos($2, $1)", [text(self, 0)?, text(self, 1)?])
            }
            // The string function CONTAINS(expression, substring) → Boolean.
            BuiltinFunction::String(StringFunction::Contains) => {
                Expr::cust_with_exprs("(strpos($1, $2) > 0)", [text(self, 0)?, text(self, 1)?])
            }
            BuiltinFunction::String(
                string @ (StringFunction::Concat | StringFunction::ConcatWs),
            ) => {
                let name = if string == StringFunction::Concat {
                    "concat"
                } else {
                    "concat_ws"
                };
                let rendered = (0..args.len())
                    .map(|i| text(self, i))
                    .collect::<Result<Vec<_>, _>>()?;
                let placeholders = (1..=rendered.len())
                    .map(|n| format!("${n}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                Expr::cust_with_exprs(format!("{name}({placeholders})"), rendered)
            }
            BuiltinFunction::Numeric(NumericFunction::Abs) => {
                Expr::cust_with_exprs("abs($1)", [num(self, 0)?])
            }
            BuiltinFunction::Numeric(NumericFunction::Mod) => Expr::cust_with_exprs(
                "mod(($1)::numeric, ($2)::numeric)",
                [num(self, 0)?, num(self, 1)?],
            ),
            // CEIL/FLOOR return Integer (QUERY master03 §Numeric functions).
            BuiltinFunction::Numeric(NumericFunction::Ceil) => {
                Expr::cust_with_exprs("(ceil($1))::int8", [num(self, 0)?])
            }
            BuiltinFunction::Numeric(NumericFunction::Floor) => {
                Expr::cust_with_exprs("(floor($1))::int8", [num(self, 0)?])
            }
            // ROUND(expression[, decimal]) — decimal defaults to 0.
            // NOTE: QUERY master03 §ROUND fixes no mode; the `::numeric` cast
            // pins half-away-from-zero (PostgreSQL docs §Mathematical
            // Functions — numeric ties round away from zero), test-pinned.
            BuiltinFunction::Numeric(NumericFunction::Round) => match args.len() {
                1 => Expr::cust_with_exprs("round(($1)::numeric, 0)", [num(self, 0)?]),
                _ => Expr::cust_with_exprs(
                    "round(($1)::numeric, ($2)::int4)",
                    [num(self, 0)?, num(self, 1)?],
                ),
            },
            // Date/time functions: the exact string formats of QUERY master03
            // §Date and time functions.
            BuiltinFunction::DateTime(DateTimeFunction::CurrentDate) => {
                Expr::cust("to_char(now(), 'YYYY-MM-DD')")
            }
            BuiltinFunction::DateTime(DateTimeFunction::CurrentTime) => {
                Expr::cust("to_char(now(), 'HH24:MI:SS')")
            }
            BuiltinFunction::DateTime(
                DateTimeFunction::CurrentDateTime | DateTimeFunction::Now,
            ) => Expr::cust("to_char(now(), 'YYYY-MM-DD\"T\"HH24:MI:SS.MSTZH:TZM')"),
            BuiltinFunction::DateTime(DateTimeFunction::CurrentTimezone) => {
                Expr::cust("to_char(now(), 'TZH:TZM')")
            }
        })
    }

    pub(super) fn operand_value(
        &mut self,
        op: &Operand,
        coercion: Coercion,
    ) -> Result<Expr, AqlError> {
        match op {
            Operand::Path(t) => self.value_expr(t, ValueMode::Value(coercion)),
            Operand::Literal(lit) => coerce_rhs(super::expr::literal_value(lit), coercion),
            Operand::Param(p) => coerce_rhs(self.param_value(p)?, coercion),
            Operand::Function { func, args } => {
                let expr = self.scalar_fn_expr(*func, args)?;
                // A function operand joins the comparison in the requested
                // coercion space like any literal: the date/time functions
                // render ISO-8601 text (QUERY master03 §Date and time
                // functions), which a temporal comparison must cast.
                Ok(match coercion {
                    Coercion::Temporal => cast(expr, "timestamptz"),
                    _ => expr,
                })
            }
        }
    }

    /// Lower a comparison operand for the numeric branch of a mixed-type (`Raw`)
    /// comparison: the path side extracts through the NULL-guarded numeric
    /// mode, literals/params cast to `numeric` (QUERY master03 §Comparison operators).
    pub(super) fn operand_value_raw_numeric(&mut self, op: &Operand) -> Result<Expr, AqlError> {
        match op {
            Operand::Path(t) => self.value_expr(t, ValueMode::RawNumeric),
            Operand::Literal(lit) => {
                Ok(cast(Expr::val(super::expr::literal_value(lit)), "numeric"))
            }
            Operand::Param(p) => Ok(cast(Expr::val(self.param_value(p)?), "numeric")),
            Operand::Function { func, args } => self.scalar_fn_expr(*func, args),
        }
    }

    // ── source / node predicates ───────────────────────────────────────────────

    pub(super) fn archetype_cond(
        &self,
        node: &str,
        a: &ArchetypeConstraint,
    ) -> Result<Expr, AqlError> {
        let value = match a {
            ArchetypeConstraint::NodeCode(c) | ArchetypeConstraint::Archetype(c) => c.clone(),
            ArchetypeConstraint::Param(p) => self.param_str(p)?,
        };
        Ok(archetype_predicate(
            node,
            &value,
            &self.ctx.archetype_lineage,
        ))
    }

    pub(super) fn name_cond(&self, node: &str, n: &NameConstraint) -> Result<Expr, AqlError> {
        match n {
            NameConstraint::Value(s) => Ok(col(node, Node::Name).eq(Expr::val(s.clone()))),
            NameConstraint::Param(p) => Ok(col(node, Node::Name).eq(Expr::val(self.param_str(p)?))),
            // The canonical expansion of the coded-name shortcut (QUERY
            // master03-syntax §Node predicate): code_string AND
            // terminology_id/value compared separately; the informational
            // `|value|` tail was already dropped at analysis. Both sides are
            // promoted columns of `node`, written by the decomposer at commit,
            // so a coded-name predicate is two column comparisons rather than
            // two JSON probes.
            NameConstraint::TermCode { terminology, code } => Ok(col(node, Node::NameCode)
                .eq(Expr::val(code.clone()))
                .and(col(node, Node::NameTerminology).eq(Expr::val(terminology.clone())))),
        }
    }

    pub(super) fn std_cond(&self, node: &str, sp: &StdPredicate) -> Result<Expr, AqlError> {
        let jp = jsonpath(&sp.path);
        let lhs = as_text(jsonb_path(fragment_source(node, Some(&jp)), &jp, None));
        let rhs = cast(Expr::val(self.bind_value(&sp.value)?), "text");
        Ok(lhs.binary(binoper(sp.op), rhs))
    }

    pub(super) fn node_constraint_conds(
        &self,
        node: &str,
        c: &NodeConstraint,
    ) -> Result<Vec<Expr>, AqlError> {
        let mut conds = Vec::new();
        if let Some(a) = &c.archetype {
            conds.push(self.archetype_cond(node, a)?);
        }
        if let Some(n) = &c.name {
            conds.push(self.name_cond(node, n)?);
        }
        for sp in &c.standard {
            conds.push(self.std_cond(node, sp)?);
        }
        Ok(conds)
    }
}

/// Whether a `Raw` (mixed-type) comparison should be evaluated numerically: true
/// when a comparison partner is a numeric literal — "numeric for numbers, text
/// otherwise" (QUERY master03 §Comparison operators). Parameters are untyped at
/// plan time and keep the text path.
pub(super) fn raw_numeric_wanted(operands: &[&Operand]) -> bool {
    operands.iter().any(|o| {
        matches!(
            o,
            Operand::Literal(TypedLit::Integer(_) | TypedLit::Real(_))
        )
    })
}
