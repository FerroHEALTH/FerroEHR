// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! WHERE-clause lowering (QUERY master03 §WHERE, §Operators, §Logical
//! operators), the whitelisted scalar functions (§Functions), and the node/
//! source predicate conditions (§Standard predicate, §Node predicate).
//!
//! No openEHR spec governs the SQL shapes — this is our own design. Every operator maps to a typed `sea-query`
//! binary op; string functions extract as text and numeric functions through the
//! magnitude coercion, following each function's declared signature.

use sea_query::{Expr, ExprTrait as _};

use crate::aql::error::{AqlError, SqlError};
use crate::aql::ir::{
    ArchetypeConstraint, Bind, Coercion, EhrField, Expr as IrExpr, LeafPath, LikePattern,
    NameConstraint, NodeConstraint, Operand, PathTarget, StdPredicate, TypedLit,
};
use crate::db::iden::{Ehr, Node};
use openehr_query::ast::{BuiltinFunction, DateTimeFunction, NumericFunction, StringFunction};
use openehr_query::lexer::CompOp;

use super::expr::{aql_like_to_sql, archetype_predicate, as_text, binoper, cast, col, jsonb_path};
use super::value::{coerce_rhs, jsonpath};
use super::{Builder, ValueMode};

impl Builder<'_> {
    // ── WHERE ──────────────────────────────────────────────────────────────────

    pub(super) fn build_where(&mut self) -> Result<(), AqlError> {
        let Some(filter) = self.ir.filter.clone() else {
            return Ok(());
        };
        let cond = self.where_expr(&filter, true)?;
        self.q.and_where(cond);
        Ok(())
    }

    /// Lower one WHERE expression. `positive` tracks the polarity (flipped
    /// under `NOT`): the existential anchored-leaf lowering
    /// ([`data_leaf_exists`](Self::data_leaf_exists)) applies only in
    /// positive positions, so `NOT (path > x)` keeps the scalar shape and
    /// its SQL three-valued behaviour for absent leaves is unchanged.
    pub(super) fn where_expr(&mut self, expr: &IrExpr, positive: bool) -> Result<Expr, AqlError> {
        match expr {
            IrExpr::And(a, b) => Ok(self
                .where_expr(a, positive)?
                .and(self.where_expr(b, positive)?)),
            IrExpr::Or(a, b) => Ok(self
                .where_expr(a, positive)?
                .or(self.where_expr(b, positive)?)),
            IrExpr::Not(a) => Ok(self.where_expr(a, !positive)?.not()),
            IrExpr::Compare {
                lhs,
                op,
                rhs,
                coercion,
            } => self.compare_expr(lhs, *op, rhs, *coercion, positive),
            IrExpr::Exists(target) => self.exists_path_expr(target),
            IrExpr::Like { path, pattern } => self.like_expr(path, pattern, positive),
            IrExpr::Const(b) => Ok(Expr::val(*b)),
            IrExpr::Matches {
                path,
                values,
                coercion,
            } => self.matches_expr(path, values, *coercion, positive),
        }
    }

    /// Lowers `EXISTS <path>` (QUERY master03 §EXISTS): true when the path has
    /// a value on ANY node the anchored walk matches / ANY fragment item — the
    /// existential shape is exact for pure existence in BOTH polarities (the
    /// test is boolean, so the three-valued caveat on comparisons does not
    /// apply). A leaf the existential lowering declines (uid synthesis, a
    /// promoted column, a single-valued inline read) keeps the scalar
    /// `IS NOT NULL` shape, which is exact there.
    ///
    /// # Errors
    /// [`AqlError`] from the path lowerings.
    fn exists_path_expr(&mut self, target: &PathTarget) -> Result<Expr, AqlError> {
        if let Some(leaf) = Self::existential_leaf(target) {
            let leaf = leaf.clone();
            self.ensure_leaf_root(target)?;
            if let Some(expr) = self.data_leaf_exists(
                &leaf,
                ValueMode::Projection,
                sea_query::ExprTrait::is_not_null,
            )? {
                return Ok(expr);
            }
        }
        Ok(self
            .value_expr(target, ValueMode::Projection)?
            .is_not_null())
    }

    /// Lowers a comparison, taking the first lowering that applies: the typed
    /// EHR-id fast path, the existential anchored-leaf shape, then the generic
    /// operand comparison.
    ///
    /// # Errors
    /// [`AqlError`] from the operand lowerings.
    fn compare_expr(
        &mut self,
        lhs: &Operand,
        op: CompOp,
        rhs: &Operand,
        coercion: Coercion,
        positive: bool,
    ) -> Result<Expr, AqlError> {
        // `e/ehr_id/value = <uuid>` compares as uuid on `ehr.id`, keeping the
        // btree usable where a two-sided text cast would not. uuid equality is
        // case-insensitive, which is also the identifier-equality semantics
        // (BASE base_types master05 §Composite Identifiers and Case).
        if let Some(expr) = self.ehr_id_typed_compare(lhs, op, rhs)? {
            return Ok(expr);
        }
        // Existential lowering for an anchored data leaf compared to a bound
        // value (any matched node satisfies) — positive polarity only.
        if positive && let Some(expr) = self.exists_compare(lhs, op, rhs, coercion)? {
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

    /// Lowers a LIKE, with the existential anchored-leaf shape in positive
    /// positions.
    ///
    /// The existential lowering is the any-match unification `exists_compare`
    /// applies: the scalar LIMIT-1 extraction's matched-node choice is
    /// order-undefined when the path matches several nodes, so it fires in
    /// positive polarity only, like every existential arm.
    ///
    /// # Errors
    /// [`AqlError`] from the parameter and path lowerings.
    fn like_expr(
        &mut self,
        path: &PathTarget,
        pattern: &LikePattern,
        positive: bool,
    ) -> Result<Expr, AqlError> {
        let pat = match pattern {
            LikePattern::Literal(s) => aql_like_to_sql(s),
            LikePattern::Param(p) => aql_like_to_sql(&self.param_str(p)?),
        };
        if positive && let Some(leaf) = Self::existential_leaf(path) {
            let leaf = leaf.clone();
            self.ensure_leaf_root(path)?;
            let pat = pat.clone();
            if let Some(expr) =
                self.data_leaf_exists(&leaf, ValueMode::Value(Coercion::Text), |extract| {
                    extract.like(pat)
                })?
            {
                return Ok(expr);
            }
        }
        let lhs = self.value_expr(path, ValueMode::Value(Coercion::Text))?;
        Ok(lhs.like(pat))
    }

    /// Lowers a MATCHES (QUERY master03 §matches), with the same existential
    /// anchored-leaf shape LIKE uses.
    ///
    /// A mixed-type leaf matched against numeric literals compares numerically.
    ///
    /// # Errors
    /// [`AqlError`] from the member-value and path lowerings.
    fn matches_expr(
        &mut self,
        path: &PathTarget,
        values: &[Bind],
        coercion: Coercion,
        positive: bool,
    ) -> Result<Expr, AqlError> {
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
        if positive && let Some(leaf) = Self::existential_leaf(path) {
            let leaf = leaf.clone();
            self.ensure_leaf_root(path)?;
            let members = members.clone();
            if let Some(expr) =
                self.data_leaf_exists(&leaf, mode, |extract| extract.is_in(members))?
            {
                return Ok(expr);
            }
        }
        let lhs = self.value_expr(path, mode)?;
        Ok(lhs.is_in(members))
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

    /// The existential lowering of `path OP bound` / `bound OP path` where
    /// the path is an anchored data leaf and the other operand is a
    /// literal/param/scalar-function (never a second path — correlating two
    /// walks stays scalar). Returns `Ok(None)` where the lowering does not
    /// apply; the caller falls back to the scalar comparison.
    /// The anchored data leaf of an existential-lowering candidate path —
    /// the same `PathTarget` arms [`exists_compare`](Self::exists_compare)
    /// admits. `None` falls back to the scalar lowering.
    fn existential_leaf(target: &PathTarget) -> Option<&LeafPath> {
        match target {
            PathTarget::Data(leaf) | PathTarget::EhrStatus(leaf) => Some(leaf),
            _ => None,
        }
    }

    /// The `EHR_STATUS` root join an existential `EhrStatus` leaf needs before
    /// its walk (the `exists_compare` preamble, shared by LIKE/matches).
    fn ensure_leaf_root(&mut self, target: &PathTarget) -> Result<(), AqlError> {
        if let PathTarget::EhrStatus(leaf) = target {
            self.ensure_ehr_status_root(leaf.source.0)?;
        }
        Ok(())
    }

    fn exists_compare(
        &mut self,
        lhs: &Operand,
        op: CompOp,
        rhs: &Operand,
        coercion: Coercion,
    ) -> Result<Option<Expr>, AqlError> {
        // Exactly one side a path; the extract keeps its written orientation.
        let (leaf_target, bound, path_is_lhs) = match (lhs, rhs) {
            (Operand::Path(t), b) if !matches!(b, Operand::Path(_)) => (t, b, true),
            (b, Operand::Path(t)) if !matches!(b, Operand::Path(_)) => (t, b, false),
            _ => return Ok(None),
        };
        let leaf = match leaf_target {
            PathTarget::Data(leaf) => leaf.clone(),
            PathTarget::EhrStatus(leaf) => {
                self.ensure_ehr_status_root(leaf.source.0)?;
                leaf.clone()
            }
            _ => return Ok(None),
        };
        // The same numeric-vs-text branch as the scalar comparison (QUERY
        // master03 §Comparison operators).
        let raw_numeric = coercion == Coercion::Raw && raw_numeric_wanted(&[lhs, rhs]);
        let (mode, bound_expr) = if raw_numeric {
            (
                ValueMode::RawNumeric,
                self.operand_value_raw_numeric(bound)?,
            )
        } else {
            (
                ValueMode::Value(coercion),
                self.operand_value(bound, coercion)?,
            )
        };
        self.data_leaf_exists(&leaf, mode, |extract| {
            if path_is_lhs {
                extract.binary(binoper(op), bound_expr)
            } else {
                bound_expr.binary(binoper(op), extract)
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
    fn operand_value_raw_numeric(&mut self, op: &Operand) -> Result<Expr, AqlError> {
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
        let lhs = as_text(jsonb_path(col(node, Node::Data), &jp, None));
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
fn raw_numeric_wanted(operands: &[&Operand]) -> bool {
    operands.iter().any(|o| {
        matches!(
            o,
            Operand::Literal(TypedLit::Integer(_) | TypedLit::Real(_))
        )
    })
}
