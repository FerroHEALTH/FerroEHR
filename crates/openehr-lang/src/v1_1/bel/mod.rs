// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! openEHR **Basic Expression Language (BEL)** — a hand-written parser that
//! produces the generated `beom` object model.
//!
//! Spec oracle: `docs/specs/openehr/LANG/docs/BEL/` — the statement/assertion/
//! assignment grammar (`master03-language.adoc`, syntax appendix
//! `masterAppA-syntax.adoc`) whose normative productions are the vendored ADL
//! grammar `crates/openehr-lang/vendor/grammar/v1_1/base_expressions.g4` (BEL imports
//! `cadl2_primitives`, which is why the `matches { … }` constraint leaf is an
//! extension point — see below). The `OPERATOR_KIND` vocabulary is
//! `master04-expression_object_model.adoc`.
//!
//! # The composition seam
//!
//! The parser is generic over a [`BelBuilder`]: the grammar (tokens, operator
//! precedence, statement shapes) lives here **once**, while the tree it
//! constructs is supplied by the builder. Two builders exist:
//!
//! * [`BeomBuilder`] (this crate) builds the pure-BEL `beom` model —
//!   [`parse_statements`] uses it. It rejects the `matches` constraint leaf
//!   (`EXPR_CONSTRAINT` is an AOM extension, defined in `openehr-am`, not beom).
//! * `openehr-adl` supplies its own builder that produces the AM-level
//!   expression model (`openehr_am::v2_4::aom2` — the extender enums composing
//!   the beom variants + the AOM leaves `EXPR_ARCHETYPE_REF`/`EXPR_CONSTRAINT`),
//!   parsing archetype paths and `matches { c_primitive_object }` via its cADL
//!   primitive parser. No expression grammar is duplicated: `openehr-adl` reuses
//!   [`parse_statements_with`] with its builder.
//!
//! `EXPR_ARCHETYPE_REF`/`EXPR_CONSTRAINT` are AOM classes (`AOM2` master05) that
//! `openehr-lang` cannot name (dependency arrows point `adl → lang`); the seam
//! is exactly the boundary at which they enter, so beom stays free of them.

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

mod parser;

use crate::v1_1::beom::core::assertion::Assertion;
use crate::v1_1::beom::core::assignment::Assignment;
use crate::v1_1::beom::core::expr_literal::ExprLiteral;
use crate::v1_1::beom::core::expr_value::ExprValue;
use crate::v1_1::beom::core::expr_value_ref::ExprValueRef;
use crate::v1_1::beom::core::expr_variable_ref::ExprVariableRef;
use crate::v1_1::beom::core::expression::Expression;
use crate::v1_1::beom::core::operator_kind::OperatorKind;
use crate::v1_1::beom::core::statement::Statement;
use crate::v1_1::beom::core::variable_declaration::VariableDeclaration;
use crate::v1_1::beom::types::expr_type_def::ExprTypeDef;
use crate::v1_1::beom::types::type_def_object_ref::TypeDefObjectRef;
use openehr_base::containers::present;

/// A BEL parse/lex error, located by byte offset in the source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BelError {
    /// The lexer met a character it cannot tokenize.
    #[error("BEL lex error at byte {at}: unrecognised input {text:?}")]
    Lex {
        /// Byte offset of the offending input.
        at: usize,
        /// The offending slice.
        text: String,
    },
    /// The parser met an unexpected token (or end of input).
    #[error("BEL parse error at byte {at}: {message}")]
    Parse {
        /// Byte offset of the offending token (or the source length at EOF).
        at: usize,
        /// A human-readable description of what was expected.
        message: String,
    },
    /// A builder rejected a production it does not support (e.g. the pure-beom
    /// [`BeomBuilder`] cannot represent the AOM `matches` constraint leaf).
    #[error("BEL builder rejected a production at byte {at}: {message}")]
    Unsupported {
        /// Byte offset of the offending production.
        at: usize,
        /// Why the builder rejected it.
        message: String,
    },
    /// Expressions nest deeper than [`crate::nesting::MAX_NESTING_DEPTH`] —
    /// an implementation bound, not a spec rule; the parser refuses rather
    /// than recursing without limit.
    #[error("BEL parse error at byte {at}: expression nesting exceeds the limit of {limit} levels")]
    NestingTooDeep {
        /// Byte offset of the token that crossed the bound.
        at: usize,
        /// The bound that was crossed.
        limit: usize,
    },
}

/// A manifest literal recognised by the BEL lexer, handed to
/// [`BelBuilder::literal`]. Verbatim spec forms are preserved as strings for the
/// temporal types (no partial-precision loss).
#[derive(Debug, Clone, PartialEq)]
pub enum BelLiteral {
    /// `True` / `False`.
    Boolean(bool),
    /// An integer literal.
    Integer(i64),
    /// A real literal.
    Real(f64),
    /// A decoded string literal (surrounding quotes removed, escapes decoded).
    String(String),
    /// A character literal.
    Character(char),
    /// An ISO-8601 date (verbatim).
    Date(String),
    /// An ISO-8601 time (verbatim).
    Time(String),
    /// An ISO-8601 date-time (verbatim).
    DateTime(String),
    /// An ISO-8601 duration (verbatim).
    Duration(String),
    /// A terminology-code reference (verbatim incl. brackets) —
    /// `LANG/docs/BEL/master03-language.adoc` §Literals lists
    /// `Terminology_code` among the BEL primitive literal types
    /// (`[snomed_ct::389086002]`), and the BEL grammar reaches
    /// `TERM_CODE_REF` through its `odin_values` import.
    TermCode(String),
    /// An interval literal, carried as its VERBATIM source text
    /// (`|105..135|`) — the constant-declaration RHS is the full
    /// `odin_values` `primitive_object`, which includes the per-type
    /// interval values (`base_expressions.g4` `constant_declaration`;
    /// `master03-language.adoc` §Constants' own
    /// `Systolic_normal_range: Interval<Integer> = |105..135|`). NOTE: the
    /// beom has no interval-literal class, so the verbatim text is the
    /// serialisation an `EXPR_LITERAL.item` can carry — the BEOM-normative
    /// bound (`master02-overview.adoc`).
    Interval(String),
}

impl BelLiteral {
    /// The canonical-JSON value this literal serialises to inside an
    /// `EXPR_LITERAL.item` (booleans/numbers natively, temporals as strings).
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            BelLiteral::Boolean(b) => serde_json::Value::Bool(*b),
            BelLiteral::Integer(i) => serde_json::Value::Number((*i).into()),
            BelLiteral::Real(r) => serde_json::Number::from_f64(*r)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
            BelLiteral::String(s)
            | BelLiteral::Date(s)
            | BelLiteral::Time(s)
            | BelLiteral::DateTime(s)
            | BelLiteral::Duration(s)
            | BelLiteral::TermCode(s)
            | BelLiteral::Interval(s) => serde_json::Value::String(s.clone()),
            BelLiteral::Character(c) => serde_json::Value::String(c.to_string()),
        }
    }
}

/// The seam between the BEL grammar and the tree it builds.
///
/// The parser drives the grammar and calls these constructors; an implementor
/// decides the concrete node types (`beom` here, the AM-level model in
/// `openehr-adl`). Leaf productions that the AOM extends —
/// [`path_ref`](BelBuilder::path_ref) and [`constraint`](BelBuilder::constraint)
/// — are the extension points.
pub trait BelBuilder {
    /// The expression node type this builder produces.
    type Expr;
    /// The statement node type this builder produces.
    type Stmt;

    /// Build a manifest-literal leaf.
    fn literal(&mut self, lit: BelLiteral) -> Self::Expr;

    /// Build a leaf referring to a declared variable (`$name`).
    fn variable_ref(&mut self, name: &str) -> Self::Expr;

    /// Build a leaf referring to a data value by archetype/data path.
    ///
    /// # Errors
    /// Returns [`BelError`] if the builder cannot represent the path.
    fn path_ref(&mut self, path: &str) -> Result<Self::Expr, BelError>;

    /// Build a function-call leaf (`name ( args… )`), e.g. `defined(x)`.
    fn function_call(&mut self, name: &str, args: Vec<Self::Expr>) -> Self::Expr;

    /// Build a binary-operator node.
    fn binary(
        &mut self,
        op: OperatorKind,
        symbol: &str,
        left: Self::Expr,
        right: Self::Expr,
    ) -> Self::Expr;

    /// Build a unary-operator node.
    fn unary(&mut self, op: OperatorKind, symbol: &str, operand: Self::Expr) -> Self::Expr;

    /// Build the right-hand `matches { … }` constraint leaf, given the verbatim
    /// source text of the constraint (the delimited `{ c_primitive_object }` or
    /// the `/regex/` form). This is where the AOM `EXPR_CONSTRAINT` /
    /// `EXPR_ARCHETYPE_ID_CONSTRAINT` leaves enter in `openehr-adl`.
    ///
    /// # Errors
    /// Returns [`BelError`] if the builder does not support constraint leaves
    /// (the pure-beom builder) or the constraint text is invalid.
    fn constraint(&mut self, raw: &str, at: usize) -> Result<Self::Expr, BelError>;

    /// Build a universal-quantification (`for_all`) node over `collection`.
    ///
    /// # Errors
    /// Returns [`BelError`] if the builder cannot represent the quantifier.
    fn for_all(
        &mut self,
        variable: &str,
        collection: Self::Expr,
        condition: Self::Expr,
    ) -> Result<Self::Expr, BelError>;

    /// Build an assertion statement (an optional `tag:` then a boolean expr).
    fn assertion(&mut self, tag: Option<String>, expr: Self::Expr) -> Self::Stmt;

    /// Build an assignment statement (`$target := source`).
    fn assignment(&mut self, target: &str, source: Self::Expr) -> Self::Stmt;

    /// Build a variable declaration (`$name : Type [ := init ]`).
    fn variable_declaration(
        &mut self,
        name: &str,
        type_id: &str,
        init: Option<Self::Expr>,
    ) -> Self::Stmt;

    /// Build a constant declaration (`Name : Type [ = primitive_object ]`) —
    /// `base_expressions.g4` `constant_declaration`;
    /// `LANG/docs/BEL/master03-language.adoc` §Constants.
    fn constant_declaration(
        &mut self,
        name: &str,
        type_id: &str,
        value: Option<Self::Expr>,
    ) -> Self::Stmt;
}

/// Parse BEL `src` into a list of `beom` [`Statement`]s (the pure-BEL model).
///
/// This is the standalone entry point; `openehr-adl` composes the AM-level model
/// via [`parse_statements_with`] and its own builder.
///
/// # Errors
/// Returns [`BelError`] on a lex or parse failure. Note that the pure-beom
/// builder rejects the AOM `matches` constraint leaf ([`BelError::Unsupported`]);
/// callers needing constraints use the AOM builder.
pub fn parse_statements(src: &str) -> Result<Vec<Statement>, BelError> {
    parse_statements_with(src, &mut BeomBuilder::default())
}

/// Parse BEL `src` driving the supplied [`BelBuilder`], returning its statement
/// nodes in source order.
///
/// # Errors
/// Returns [`BelError`] on a lex/parse failure or a builder rejection.
pub fn parse_statements_with<B: BelBuilder>(
    src: &str,
    builder: &mut B,
) -> Result<Vec<B::Stmt>, BelError> {
    let tokens = crate::v1_1::lexer::lex_bel(src).map_err(|failure| BelError::Lex {
        at: failure.span.start,
        text: failure.text,
    })?;
    parser::Parser::new(src, &tokens, builder).parse_statement_block()
}

/// The default [`BelBuilder`] producing the pure-BEL `beom` object model.
///
/// It tracks declared-variable types so a later `$var` reference recovers the
/// declared [`ExprTypeDef`]; an undeclared reference defaults to an
/// object-reference type def carrying the bare name.
#[derive(Debug, Default)]
pub struct BeomBuilder {
    vars: std::collections::BTreeMap<String, ExprTypeDef>,
}

impl BeomBuilder {
    /// A fresh builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The [`ExprTypeDef`] recorded for `name` (or an object-reference default).
    fn type_of(&self, name: &str) -> ExprTypeDef {
        self.vars
            .get(name)
            .cloned()
            .unwrap_or_else(|| object_ref_type(name))
    }
}

/// An [`ExprTypeDef`] naming `type_id` as a general object-reference type.
///
/// NOTE: the beom primitive type-defs carry a differently-typed `type_anchor`
/// per primitive (i32/bool/…); a parser does no value anchoring, so every
/// declared type is recorded as `TYPE_DEF_OBJECT_REF` (its `type_anchor` is a
/// JSON `null`) with the declared `type_name` preserved. No openEHR spec governs
/// this parse-time choice — our own design/extension.
fn object_ref_type(type_id: &str) -> ExprTypeDef {
    ExprTypeDef::TypeDefObjectRef(TypeDefObjectRef {
        type_name: type_id.to_owned(),
        type_anchor: serde_json::Value::Null,
    })
}

impl BelBuilder for BeomBuilder {
    type Expr = Expression;
    type Stmt = Statement;

    fn literal(&mut self, lit: BelLiteral) -> Expression {
        Expression::ExprLiteral(ExprLiteral {
            item: lit.to_json(),
        })
    }

    fn variable_ref(&mut self, name: &str) -> Expression {
        Expression::ExprVariableRef(ExprVariableRef {
            item: VariableDeclaration {
                name: name.to_owned(),
                r#type: self.type_of(name),
            },
        })
    }

    fn path_ref(&mut self, path: &str) -> Result<Expression, BelError> {
        Ok(Expression::ExprValueRef(ExprValueRef {
            item: Some(serde_json::Value::String(path.to_owned())),
        }))
    }

    fn function_call(&mut self, name: &str, args: Vec<Expression>) -> Expression {
        // beom EXPR_FUNCTION_CALL: `item` carries the function reference (its
        // name here), `arguments` the operands.
        Expression::ExprFunctionCall(
            crate::v1_1::beom::core::expr_function_call::ExprFunctionCall {
                item: Some(serde_json::Value::String(name.to_owned())),
                arguments: present(args),
            },
        )
    }

    fn binary(
        &mut self,
        op: OperatorKind,
        symbol: &str,
        left: Expression,
        right: Expression,
    ) -> Expression {
        Expression::ExprBinaryOperator(Box::new(
            crate::v1_1::beom::core::expr_binary_operator::ExprBinaryOperator {
                precedence_overridden: None,
                operator: op,
                symbol: Some(symbol.to_owned()),
                left_operand: Box::new(left),
                right_operand: Box::new(right),
            },
        ))
    }

    fn unary(&mut self, op: OperatorKind, symbol: &str, operand: Expression) -> Expression {
        Expression::ExprUnaryOperator(Box::new(
            crate::v1_1::beom::core::expr_unary_operator::ExprUnaryOperator {
                precedence_overridden: None,
                operator: op,
                symbol: Some(symbol.to_owned()),
                operand: Box::new(operand),
            },
        ))
    }

    fn constraint(&mut self, _raw: &str, at: usize) -> Result<Expression, BelError> {
        Err(BelError::Unsupported {
            at,
            message: "the `matches { … }` constraint leaf is an AOM extension \
                      (EXPR_CONSTRAINT); parse rules with the AOM builder in openehr-adl"
                .to_owned(),
        })
    }

    fn for_all(
        &mut self,
        _variable: &str,
        collection: Expression,
        condition: Expression,
    ) -> Result<Expression, BelError> {
        // beom EXPR_FOR_ALL: `operand` is the collection reference (an
        // EXPR_VALUE_REF), `condition` the per-member assertion.
        // NOTE: EXPR_FOR_ALL declares no bound-variable attribute
        // (`LANG/docs/BEL/master04-expression_object_model.adoc` §Core
        // Package), so the binding name has nowhere to land in the model.
        let operand = match collection {
            Expression::ExprValueRef(r) => r,
            _ => ExprValueRef { item: None },
        };
        Ok(Expression::ExprForAll(Box::new(
            crate::v1_1::beom::core::expr_for_all::ExprForAll {
                precedence_overridden: None,
                operator: OperatorKind::ForAll,
                symbol: Some("for_all".to_owned()),
                condition: Box::new(Assertion {
                    tag: None,
                    string_expression: None,
                    expression: Box::new(condition),
                }),
                operand,
            },
        )))
    }

    fn assertion(&mut self, tag: Option<String>, expr: Expression) -> Statement {
        Statement::Assertion(Assertion {
            tag,
            string_expression: None,
            expression: Box::new(expr),
        })
    }

    fn assignment(&mut self, target: &str, source: Expression) -> Statement {
        Statement::Assignment(Assignment {
            target: VariableDeclaration {
                name: target.to_owned(),
                r#type: self.type_of(target),
            },
            source: to_expr_value(source),
        })
    }

    fn variable_declaration(
        &mut self,
        name: &str,
        type_id: &str,
        init: Option<Expression>,
    ) -> Statement {
        let ty = object_ref_type(type_id);
        self.vars.insert(name.to_owned(), ty.clone());
        // beom VARIABLE_DECLARATION has no initialiser slot (an `:= init` is a
        // separate ASSIGNMENT in the model); the parsed init is discarded here.
        drop(init);
        Statement::VariableDeclaration(VariableDeclaration {
            name: name.to_owned(),
            r#type: ty,
        })
    }

    fn constant_declaration(
        &mut self,
        name: &str,
        type_id: &str,
        value: Option<Expression>,
    ) -> Statement {
        // NOTE: the beom declares no constant class
        // (`LANG/docs/BEL/master04-expression_object_model.adoc` §Core
        // Package), so its one declaration shape carries the construct.
        let ty = object_ref_type(type_id);
        self.vars.insert(name.to_owned(), ty.clone());
        drop(value);
        Statement::VariableDeclaration(VariableDeclaration {
            name: name.to_owned(),
            r#type: ty,
        })
    }
}

/// Lift an [`Expression`] into the `EXPR_VALUE` union used by assignment sources.
fn to_expr_value(e: Expression) -> ExprValue {
    match e {
        Expression::ExprBinaryOperator(b) => ExprValue::ExprBinaryOperator(*b),
        Expression::ExprForAll(f) => ExprValue::ExprForAll(*f),
        Expression::ExprFunctionCall(f) => ExprValue::ExprFunctionCall(f),
        Expression::ExprLiteral(l) => ExprValue::ExprLiteral(l),
        Expression::ExprUnaryOperator(u) => ExprValue::ExprUnaryOperator(*u),
        Expression::ExprValueRef(r) => ExprValue::ExprValueRef(r),
        Expression::ExprVariableRef(v) => ExprValue::ExprVariableRef(v),
    }
}
