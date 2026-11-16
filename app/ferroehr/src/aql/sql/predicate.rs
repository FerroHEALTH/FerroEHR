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
    Operand, PathTarget, RmSource, StdPredicate, TypedLit,
};
use crate::db::iden::{Ehr, Node};
use openehr_query::ast::{BuiltinFunction, DateTimeFunction, NumericFunction, StringFunction};
use openehr_query::lexer::CompOp;

use super::archetype::archetype_predicate;
use super::coerce::{RawPartner, coerce_rhs, numeric_value, target_types};
use super::compact::version_uid_text;
use super::expr::{aql_like_to_sql, as_text, binoper, cast, col, fragment_source, jsonb_path};
use super::from::is_vo_root_type;
use super::jsonpath::jsonpath;
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
        if coercion == Coercion::Raw
            && let Some((target, value, leaf_left)) = self.raw_sides(lhs, rhs)?
        {
            let partner = RawPartner::of(value, target_types(target));
            let (first, second) = partner.modes();
            let first = self.value_expr(target, first)?;
            let second = second.map(|m| self.value_expr(target, m)).transpose()?;
            return Ok(partner.compare(binoper(op), leaf_left, first, second));
        }
        let l = self.operand_value(lhs, coercion)?;
        let r = self.operand_value(rhs, coercion)?;
        Ok(l.binary(binoper(op), r))
    }

    /// The path, the bound value opposite it and whether the path is on the
    /// left, for a comparison of one path with a literal or a parameter, which
    /// a mixed-type (`Raw`) leaf compares by [`RawPartner`]; `None` for any
    /// other pair of operands.
    ///
    /// # Errors
    /// [`SqlError::UnboundParameter`] for an unbound parameter.
    pub(super) fn raw_sides<'o>(
        &self,
        lhs: &'o Operand,
        rhs: &'o Operand,
    ) -> Result<Option<(&'o PathTarget, sea_query::Value, bool)>, AqlError> {
        let (target, other, leaf_left) = match (lhs, rhs) {
            (Operand::Path(t), other) => (t, other, true),
            (other, Operand::Path(t)) => (t, other, false),
            _ => return Ok(None),
        };
        let value = match other {
            Operand::Literal(lit) => super::expr::literal_value(lit),
            Operand::Param(p) => self.param_value(p)?,
            Operand::Path(_) | Operand::Function { .. } => return Ok(None),
        };
        Ok(Some((target, value, leaf_left)))
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
    /// literals compares by its number ([`ValueMode::RawNumeric`]), and against
    /// any other list by its text ([`ValueMode::RawText`]).
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
        let mode = match (numeric, coercion) {
            (true, _) => ValueMode::RawNumeric,
            (false, Coercion::Raw) => ValueMode::RawText,
            (false, coercion) => ValueMode::Value(coercion),
        };
        let mut members = Vec::with_capacity(values.len());
        for b in values {
            let v = self.bind_value(b)?;
            members.push(match (numeric, coercion) {
                (true, _) => numeric_value(v)?,
                (false, Coercion::Raw) => coerce_rhs(v, Coercion::Text)?,
                (false, coercion) => coerce_rhs(v, coercion)?,
            });
        }
        Ok((mode, members))
    }

    /// The EHR-id comparison of `WHERE`: fires for a comparison between an
    /// `EHR.ehr_id/value` path and a string literal or parameter, and lowers it
    /// through [`ehr_id_compare`], the shape the `EHR` class predicate takes
    /// too. Returns `None` (the generic text lowering) for every other shape.
    fn ehr_id_typed_compare(
        &mut self,
        lhs: &Operand,
        op: CompOp,
        rhs: &Operand,
    ) -> Result<Option<Expr>, AqlError> {
        let (path, other, flipped) = match (lhs, rhs) {
            (Operand::Path(t), o) if Self::is_ehr_id_target(t) => (t, o, false),
            (o, Operand::Path(t)) if Self::is_ehr_id_target(t) => (t, o, true),
            _ => return Ok(None),
        };
        let bind = match other {
            Operand::Literal(lit) => Bind::Literal(lit.clone()),
            Operand::Param(p) => Bind::Param(p.clone()),
            _ => return Ok(None),
        };
        let Some(raw) = self.ehr_id_value(&bind)? else {
            return Ok(None);
        };
        let PathTarget::Ehr { source, .. } = path else {
            return Ok(None);
        };
        let alias = self
            .ehr_alias
            .get(&source.0)
            .cloned()
            .ok_or_else(|| SqlError::Unsupported("EHR path without a bound EHR".to_owned()))?;
        let op = if flipped { flip(op) } else { op };
        Ok(Some(ehr_id_compare(&alias, op, &raw)))
    }

    /// The string an EHR id is compared against: a string literal, or a
    /// parameter's value as text; `None` for any other literal.
    ///
    /// # Errors
    /// [`SqlError::UnboundParameter`] for an unbound or `NULL` parameter.
    pub(super) fn ehr_id_value(&self, value: &Bind) -> Result<Option<String>, AqlError> {
        Ok(match value {
            Bind::Literal(TypedLit::String(s) | TypedLit::Temporal(s)) => Some(s.clone()),
            Bind::Param(p) => Some(self.param_str(p)?),
            Bind::Literal(_) => None,
        })
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
            // A mixed-type leaf beside another path or a function compares by
            // its text, which a whole data value has none of (`coerce`).
            Operand::Path(t) if coercion == Coercion::Raw => self.value_expr(t, ValueMode::RawText),
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

    /// A standard predicate on the node `node` (QUERY master03 §Standard
    /// predicate), reading its path as the same path reads in WHERE: the
    /// `uid/value` of a versioned object's root is its bound version's
    /// identifier (RM common master06 §Version Identification), which the
    /// stored fragment does not carry, and every other path reads the
    /// fragment.
    ///
    /// # Errors
    /// [`SqlError::UnboundParameter`] for an unbound parameter.
    pub(super) fn std_cond(&self, node: &str, sp: &StdPredicate) -> Result<Expr, AqlError> {
        let root = self.root_version_alias(node);
        let lhs = match root.as_deref() {
            Some(voa) if is_version_uid(&sp.path) => version_uid_text(voa),
            _ => {
                let jp = jsonpath(&sp.path);
                as_text(jsonb_path(
                    fragment_source(node, Some(&jp), root.as_deref()),
                    &jp,
                    None,
                ))
            }
        };
        let rhs = cast(Expr::val(self.bind_value(&sp.value)?), "text");
        Ok(lhs.binary(binoper(sp.op), rhs))
    }

    /// The standard predicates of an RM source's class expression on its node
    /// `alias` ([`Self::std_cond`]).
    ///
    /// # Errors
    /// [`SqlError::Unsupported`] for a `uid` predicate on a versioned object
    /// no version is bound beside (an `OR` or `NOT CONTAINS` operand), and
    /// [`SqlError::UnboundParameter`] for an unbound parameter.
    pub(super) fn source_std_conds(
        &self,
        alias: &str,
        r: &RmSource,
    ) -> Result<Vec<Expr>, AqlError> {
        let mut out = Vec::with_capacity(r.standard.len());
        for sp in &r.standard {
            // NOTE: RM common master06 §Version Identification gives a versioned object's root its
            // `uid` from the version, so only a bound version answers the predicate on it.
            if is_version_uid(&sp.path)
                && r.rm_type.names().iter().all(|t| is_vo_root_type(t))
                && self.root_version_alias(alias).is_none()
            {
                return Err(SqlError::Unsupported(
                    "a uid predicate on a versioned object reached through OR or NOT CONTAINS"
                        .to_owned(),
                )
                .into());
            }
            out.push(self.std_cond(alias, sp)?);
        }
        Ok(out)
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

/// Whether a standard predicate's path is a versioned object's `uid/value`.
pub(super) fn is_version_uid(path: &[String]) -> bool {
    matches!(path, [uid, value] if uid == "uid" && value == "value")
}

/// Lowers `ehr.id <op> raw`, the one EHR-id comparison of the `EHR` class
/// predicate and of `WHERE`.
///
/// `EHR.ehr_id` is a `HIER_OBJECT_ID` whose `value` is a `String` (RM 1.2.0
/// `EHR`, BASE `OBJECT_ID`), so a value that is not a UUID is a legal operand
/// that equals no stored id: `=` is constant false and `!=` constant true,
/// never a cast error. A UUID compares as `uuid` on `ehr.id`, which keeps the
/// index usable and is case-insensitive, the identifier-equality semantics
/// (BASE `base_types` master05 §Composite Identifiers and Case). An ordering
/// operator compares the id's text, the string comparison QUERY master03
/// §Comparison operators defines.
pub(super) fn ehr_id_compare(alias: &str, op: CompOp, raw: &str) -> Expr {
    match op {
        CompOp::Eq | CompOp::Ne => match raw.parse::<uuid::Uuid>() {
            Ok(u) => {
                col(alias, Ehr::Id).binary(binoper(op), cast(Expr::val(u.to_string()), "uuid"))
            }
            Err(_) => Expr::val(matches!(op, CompOp::Ne)),
        },
        _ => cast(col(alias, Ehr::Id), "text")
            .binary(binoper(op), cast(Expr::val(raw.to_owned()), "text")),
    }
}

/// The operator with its operands swapped (`a < b` is `b > a`).
fn flip(op: CompOp) -> CompOp {
    match op {
        CompOp::Lt => CompOp::Gt,
        CompOp::Gt => CompOp::Lt,
        CompOp::Le => CompOp::Ge,
        CompOp::Ge => CompOp::Le,
        CompOp::Eq | CompOp::Ne => op,
    }
}
