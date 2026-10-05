// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `rules` section (`ADL2/master07.11`) and the BEL expression printers
//! that also serve slot include/exclude assertions (`master04.6`). Expressions
//! are rendered with full parenthesization so they re-parse to the identical
//! tree.

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use openehr_am::v2_4::aom2::rules::expr_constraint::ExprConstraint;
use openehr_am::v2_4::beom::core::assertion::Assertion;
use openehr_am::v2_4::beom::core::expr_value::ExprValue;
use openehr_am::v2_4::beom::core::expr_value_ref::ExprValueRef;
use openehr_am::v2_4::beom::core::expression::Expression;
use openehr_am::v2_4::beom::core::statement::Statement;
use openehr_am::v2_4::beom::core::statement_set::StatementSet;

use crate::print::definition::{bool_str, cstring_inline, primitive_inline};
use crate::print::odin::quoted;
use crate::print::{PrintError, Printer};

impl Printer {
    // ── rules (master07.11; BEL) ───────────────────────────────────────────
    pub(super) fn rules(&mut self, set: &StatementSet) -> Result<(), PrintError> {
        for stmt in set.statement.iter().flatten() {
            match stmt {
                Statement::Assertion(a) => self.line(1, &assertion_str(a)?),
                Statement::Assignment(a) => {
                    let source = expr_value_str(&a.source, &a.target.name)?;
                    self.line(1, &format!("${} = {source}", a.target.name));
                }
                Statement::VariableDeclaration(v) => {
                    self.line(1, &format!("${} : {}", v.name, type_def_name(&v.r#type)));
                }
            }
        }
        Ok(())
    }
}

// ── rules expression printing (full parenthesization) ──────────────────────

/// A single assertion statement — a `rules` line or a slot include/exclude
/// assertion — rendered from its expression tree.
///
/// The tree is the authority: `ASSERTION.expression` is the "Root of expression
/// tree" and `string_expression` only its "String form of expression"
/// (`LANG/docs/BEL/master04-expression_object_model.adoc` §Core Package,
/// `ASSERTION`), so the printer never reads the string form back.
pub(super) fn assertion_str(a: &Assertion) -> Result<String, PrintError> {
    let expr = statement_expression_str(&a.expression)?;
    Ok(match &a.tag {
        Some(tag) => format!("{tag}: {expr}"),
        None => expr,
    })
}

/// A statement-level expression: the outermost operator needs no parentheses
/// of its own (each operand is rendered fully parenthesized, so precedence
/// stays explicit and the text re-parses to the identical tree).
fn statement_expression_str(e: &Expression) -> Result<String, PrintError> {
    match e {
        Expression::ExprBinaryOperator(b) => {
            let sym = b.symbol.as_deref().unwrap_or(b.operator.as_str());
            let left = expression_str(&b.left_operand)?;
            Ok(if sym == "matches" {
                format!("{left} matches {}", constraint_rhs(&b.right_operand)?)
            } else {
                format!("{left} {sym} {}", expression_str(&b.right_operand)?)
            })
        }
        other => expression_str(other),
    }
}

/// Render an [`Expression`] with full parenthesization so it re-parses to the
/// identical tree regardless of operator precedence (the BEL parser drops
/// redundant parentheses — `bel::parser::parse_primary` — so extra parens never
/// change the built tree).
fn expression_str(e: &Expression) -> Result<String, PrintError> {
    Ok(match e {
        Expression::ExprLiteral(l) => literal_str(&l.item),
        Expression::ExprVariableRef(v) => format!("${}", v.item.name),
        Expression::ExprValueRef(r) => value_ref_str(r)?,
        Expression::ExprBinaryOperator(b) => {
            let sym = b.symbol.as_deref().unwrap_or(b.operator.as_str());
            let left = expression_str(&b.left_operand)?;
            if sym == "matches" {
                format!("({left} matches {})", constraint_rhs(&b.right_operand)?)
            } else {
                format!("({left} {sym} {})", expression_str(&b.right_operand)?)
            }
        }
        Expression::ExprUnaryOperator(u) => {
            let sym = u.symbol.as_deref().unwrap_or(u.operator.as_str());
            if sym == "exists" {
                // `exists` binds a bare reference leaf (no parentheses; the BEL
                // grammar's `parse_ref_leaf`).
                format!("exists {}", ref_leaf_str(&u.operand)?)
            } else {
                format!("{sym} ({})", expression_str(&u.operand)?)
            }
        }
        // NOTE: the BEL function-call production is an identifier + arguments
        // (`LANG/docs/BEL/masterAppA-syntax.adoc`); a leaf `item` (typed `Any`,
        // beom `EXPR_LEAF`) carrying no string name has no spelling — refused.
        Expression::ExprFunctionCall(f) => {
            let Some(name) = f.item.as_ref().and_then(|v| v.as_str()) else {
                return Err(PrintError::NamelessFunctionCall);
            };
            let args = f
                .arguments
                .iter()
                .flatten()
                .map(expression_str)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            format!("{name}({args})")
        }
        Expression::ExprForAll(fa) => {
            let collection = value_ref_str(&fa.operand)?;
            let cond = expression_str(&fa.condition.expression)?;
            format!("for_all $x : {collection} | {cond}")
        }
        Expression::ExprConstraint(c) => constraint_leaf_str(c),
    })
}

/// The RHS of a `matches` operator: the cADL primitive/regex, wrapped in braces.
fn constraint_rhs(e: &Expression) -> Result<String, PrintError> {
    match e {
        Expression::ExprConstraint(c) => Ok(constraint_leaf_str(c)),
        other => expression_str(other),
    }
}

/// A brace-wrapped constraint leaf: an inline primitive, or the archetype-id
/// regex matcher of a slot assertion.
fn constraint_leaf_str(c: &ExprConstraint) -> String {
    match c {
        ExprConstraint::ExprConstraint(d) => format!("{{{}}}", primitive_inline(&d.item)),
        // A C_STRING regex matcher (`master04.3` §Slots based on Lexical
        // Archetype Identifiers).
        ExprConstraint::ExprArchetypeIdConstraint(a) => format!("{{{}}}", cstring_inline(&a.item)),
    }
}

/// The path text of a value reference (an archetype path or a named leaf).
///
/// # Errors
///
/// Refuses a value-reference leaf whose `item` carries no string path.
fn value_ref_str(r: &ExprValueRef) -> Result<String, PrintError> {
    match r {
        ExprValueRef::ExprArchetypeRef(a) => Ok(a.path.clone()),
        // NOTE: the BEL value-reference production is a path
        // (`LANG/docs/BEL/masterAppA-syntax.adoc`); a leaf `item` (typed `Any`,
        // beom `EXPR_LEAF`) carrying no string path has no spelling — refused.
        ExprValueRef::ExprValueRef(v) => v
            .item
            .as_ref()
            .and_then(|x| x.as_str())
            .map(str::to_owned)
            .ok_or(PrintError::PathlessValueRef),
    }
}

/// The bare (unparenthesized) reference leaf an `exists` operator binds.
fn ref_leaf_str(e: &Expression) -> Result<String, PrintError> {
    match e {
        Expression::ExprValueRef(r) => value_ref_str(r),
        Expression::ExprVariableRef(v) => Ok(format!("${}", v.item.name)),
        other => expression_str(other),
    }
}

/// The RHS of an assignment statement, over the `EXPR_VALUE` union, refusing
/// the one subtype no ADL text can carry.
fn expr_value_str(v: &ExprValue, target: &str) -> Result<String, PrintError> {
    Ok(match v {
        ExprValue::ExprBinaryOperator(b) => {
            expression_str(&Expression::ExprBinaryOperator(Box::new(b.clone())))?
        }
        ExprValue::ExprUnaryOperator(u) => {
            expression_str(&Expression::ExprUnaryOperator(Box::new(u.clone())))?
        }
        ExprValue::ExprForAll(f) => expression_str(&Expression::ExprForAll(Box::new(f.clone())))?,
        ExprValue::ExprFunctionCall(f) => expression_str(&Expression::ExprFunctionCall(f.clone()))?,
        ExprValue::ExprLiteral(l) => literal_str(&l.item),
        ExprValue::ExprValueRef(r) => value_ref_str(r)?,
        ExprValue::ExprVariableRef(v) => format!("${}", v.item.name),
        ExprValue::ExprConstraint(c) => constraint_leaf_str(c),
        // NOTE: no released grammar spells EXTERNAL_QUERY — neither
        // `LANG/docs/BEL/masterAppA-syntax.adoc` nor
        // `AM/docs/ADL2/masterAppB-syntax_spec.adoc` has a production for it.
        ExprValue::ExternalQuery(_) => {
            return Err(PrintError::ExternalQuery {
                target: target.to_owned(),
            });
        }
    })
}

/// A BEL literal in its source spelling.
fn literal_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Bool(b) => bool_str(*b).to_owned(),
        serde_json::Value::String(s) => quoted(s),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// The declared type name of a variable declaration (`Any` when untyped).
fn type_def_name(t: &openehr_lang::v1_1::beom::types::expr_type_def::ExprTypeDef) -> String {
    match t {
        openehr_lang::v1_1::beom::types::expr_type_def::ExprTypeDef::TypeDefObjectRef(r) => {
            r.type_name.clone()
        }
        _ => "Any".to_owned(),
    }
}
