// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! openEHR **Expression Language (EL)** — a hand-written parser over the
//! vendored normative grammar.
//!
//! Spec oracle: `docs/specs/openehr/LANG/docs/EL/` — the terminal entities
//! (`master04-terminal_entities.adoc`), the operator tables and precedence
//! (`master05-expressions.adoc`), and the syntax appendix
//! `masterAppA-syntax.adoc`, which is an `include::` of exactly the two
//! vendored grammars `crates/openehr-lang/vendor/grammar/v1_1/{ElLexer.g4,
//! ElParser.g4}`. Where EL expressions are used in BMM models the shape is
//! fixed: "Expressions as used in BMM models to express class invariants and
//! routine pre- and post-conditions are always in the form of an
//! `BMM_ASSERTION`" (`LANG/docs/bmm3/master10-expressions.adoc` §Usage in BMM
//! Models).
//!
//! # The composition seam
//!
//! The parser is generic over an [`ElBuilder`], the same seam the BEL parser
//! uses ([`crate::v1_1::bel::BelBuilder`]): the grammar lives here once and the tree
//! is supplied by the builder. EL is NOT a BEL extension — `ElParser.g4`
//! imports `Cadl2Parser`, not `base_expressions.g4`, renames every production,
//! adds scoped feature references, tuples and decision tables, and takes its
//! operator precedence from a table that CONTRADICTS the BEL one (see
//! [`parse_boolean_expression_with`]) — so the two parsers share the lexical
//! layer and nothing else.
//!
//! # Boundaries of this reader
//!
//! Decision tables (`ElParser.g4` `dlDecisionTable` and its `dlBinaryChoice`/
//! `dlCaseTable`/`dlConditionTable` alternatives) are refused with
//! [`ElError::Unsupported`]; their `BLOCK_DELIM` and `?` lexical forms have no
//! union production. The `matches { … }` right-hand side is handed to the
//! builder verbatim, because `cInlineOrderedObject`/`cObjectMatcher` come from
//! the unvendored `Cadl2Parser`.

mod parser;

use crate::v1_1::lexer::Token;

/// An EL lex/parse error, located by byte offset in the source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ElError {
    /// The lexer met a character it cannot tokenize under the EL reading.
    #[error("EL lex error at byte {at}: unrecognised input {text:?}")]
    Lex {
        /// Byte offset of the offending input.
        at: usize,
        /// The offending slice.
        text: String,
    },
    /// The parser met an unexpected token (or end of input).
    #[error("EL parse error at byte {at}: {message}")]
    Parse {
        /// Byte offset of the offending token (or the source length at EOF).
        at: usize,
        /// A human-readable description of what was expected.
        message: String,
    },
    /// A production this reader does not realize, or one the builder cannot
    /// represent.
    #[error("EL production unsupported at byte {at}: {message}")]
    Unsupported {
        /// Byte offset of the offending production.
        at: usize,
        /// Why it was refused.
        message: String,
    },
    /// A name the builder could not resolve against the model it materialises
    /// into.
    #[error("EL name `{name}` at byte {at} does not resolve: {message}")]
    Unresolved {
        /// Byte offset of the reference.
        at: usize,
        /// The unresolved name, as written.
        name: String,
        /// What resolution was attempted.
        message: String,
    },
}

/// A manifest literal recognised by the EL lexer, handed to
/// [`ElBuilder::literal`].
///
/// The temporal forms keep their verbatim spec text (no partial-precision
/// loss); `ElParser.g4` `elArithmeticValue` and the `primitiveObject` of the
/// imported cADL parser are their productions.
#[derive(Debug, Clone, PartialEq)]
pub enum ElLiteral {
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
    /// A terminology-code reference (verbatim, incl. brackets).
    TermCode(String),
}

impl ElLiteral {
    /// The name of the openEHR Foundation Types class this literal instantiates.
    ///
    /// `LANG/docs/EL/master04-terminal_entities.adoc` §Literal Values: a
    /// literal's type is "a type known in the model", and the primitive
    /// spellings are the Foundation Types class names.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            ElLiteral::Boolean(_) => "Boolean",
            ElLiteral::Integer(_) => "Integer",
            ElLiteral::Real(_) => "Real",
            ElLiteral::String(_) => "String",
            ElLiteral::Character(_) => "Character",
            ElLiteral::Date(_) => "Date",
            ElLiteral::Time(_) => "Time",
            ElLiteral::DateTime(_) => "Date_Time",
            ElLiteral::Duration(_) => "Duration",
            ElLiteral::TermCode(_) => "Terminology_code",
        }
    }

    /// The serial form of this literal, i.e. `BMM_LITERAL_VALUE.value_literal`
    /// ("A serial representation of the value",
    /// `org.openehr.lang.bmm3.bmm_literal_value.adoc` §Attributes).
    #[must_use]
    pub fn value_literal(&self) -> String {
        match self {
            ElLiteral::Boolean(value) => value.to_string(),
            ElLiteral::Integer(value) => value.to_string(),
            ElLiteral::Real(value) => value.to_string(),
            ElLiteral::Character(value) => value.to_string(),
            ElLiteral::String(text)
            | ElLiteral::Date(text)
            | ElLiteral::Time(text)
            | ElLiteral::DateTime(text)
            | ElLiteral::Duration(text)
            | ElLiteral::TermCode(text) => text.clone(),
        }
    }
}

/// An EL operator, as the `(symbol, function name)` pair the meta-model needs.
///
/// Every EL operator IS a function on its principal operand — "where the
/// expression `100 - 5` is encountered in EL, what is really invoked is
/// `{Integer}._subtract_()`" — and the Arithmetic / Relational / Logical
/// operator tables of `LANG/docs/EL/master05-expressions.adoc` §Primitive
/// Operators give each one's function name. That name is what
/// `EL_BINARY_OPERATOR.call` / `EL_UNARY_OPERATOR.call` carry ("Function call
/// equivalent to this operator expression",
/// `org.openehr.lang.bmm3.el_binary_operator.adoc` §Attributes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElOperator {
    /// The symbol as written in the source.
    pub symbol: &'static str,
    /// The name of the function the operator invokes.
    pub function: &'static str,
}

impl ElOperator {
    /// The operator a boolean/relational/arithmetic token denotes, or `None`
    /// when the token is not an operator.
    fn of(token: &Token) -> Option<Self> {
        let (symbol, function) = match token {
            Token::SymAnd => ("and", "conjunction"),
            Token::SymOr => ("or", "disjunction"),
            Token::SymXor => ("xor", "exclusive_disjunction"),
            Token::SymImplies => ("implies", "implication"),
            // `LANG/docs/EL/master05-expressions.adoc` §Primitive Operators
            // lists no function for `⇔`; the Boolean class's own equality is
            // what material equivalence evaluates to.
            Token::SymIff => ("<=>", "equal"),
            Token::SymNot => ("not", "not"),
            Token::SymEq => ("=", "equal"),
            Token::SymNe => ("!=", "not_equal"),
            Token::SymLt => ("<", "less_than"),
            Token::SymLe => ("<=", "less_than_or_equal"),
            Token::SymGt => (">", "greater_than"),
            Token::SymGe => (">=", "greater_than_or_equal"),
            Token::SymPlus => ("+", "add"),
            Token::SymMinus => ("-", "subtract"),
            Token::SymStar => ("*", "multiply"),
            Token::SymSlash => ("/", "divide"),
            Token::SymPercent => ("%", "modulus"),
            Token::SymCarat => ("^", "exponent"),
            Token::SymMatches => ("matches", "matches"),
            _ => return None,
        };
        Some(Self { symbol, function })
    }
}

/// The seam between the EL grammar and the tree it builds.
///
/// The parser drives `ElParser.g4`'s productions and calls these constructors;
/// an implementor decides the concrete node types. Every method is fallible
/// because the target meta-model's reference leaves carry mandatory
/// definitions (`EL_PROPERTY_REF.definition`, `EL_STATIC_REF.definition`,
/// `EL_LITERAL.value.type`), which only the builder can resolve.
pub trait ElBuilder {
    /// The expression node type this builder produces.
    type Expr;

    /// Builds a manifest-literal leaf (`ElParser.g4` `elArithmeticValue`,
    /// `booleanValue`, `primitiveObject`).
    ///
    /// # Errors
    /// Returns [`ElError`] when the literal's type cannot be resolved.
    fn literal(&mut self, literal: ElLiteral, at: usize) -> Result<Self::Expr, ElError>;

    /// Builds the `Self` reference (`ElParser.g4` `elValueGenerator`).
    ///
    /// # Errors
    /// Returns [`ElError`] when the enclosing type is unknown.
    fn self_ref(&mut self, at: usize) -> Result<Self::Expr, ElError>;

    /// Builds the `Result` reference (`ElParser.g4` `elInstantiableRef`).
    ///
    /// # Errors
    /// Returns [`ElError`] when there is no enclosing routine result.
    fn result_ref(&mut self, at: usize) -> Result<Self::Expr, ElError>;

    /// Builds a bound-variable reference `$name` (`elBoundVariableId`).
    ///
    /// # Errors
    /// Returns [`ElError`] when the builder cannot represent it.
    fn bound_variable(&mut self, name: &str, at: usize) -> Result<Self::Expr, ElError>;

    /// Builds a type reference `{TypeName}` used as a scoper (`elScoper`).
    ///
    /// # Errors
    /// Returns [`ElError`] when the type name is not in the model.
    fn type_ref(&mut self, type_id: &str, at: usize) -> Result<Self::Expr, ElError>;

    /// Builds a feature reference: a lower-case `elFunctionCall` (with `args`
    /// when the call has an argument list) or an upper-case `elConstantId`,
    /// optionally scoped by `scoper` (`elScopedFeatureRef`).
    ///
    /// # Errors
    /// Returns [`ElError`] when the name does not resolve.
    fn feature_ref(
        &mut self,
        scoper: Option<Self::Expr>,
        name: &str,
        args: Option<Vec<Self::Expr>>,
        at: usize,
    ) -> Result<Self::Expr, ElError>;

    /// Builds a binary-operator node.
    ///
    /// # Errors
    /// Returns [`ElError`] when the equivalent call cannot be constructed.
    fn binary(
        &mut self,
        operator: ElOperator,
        left: Self::Expr,
        right: Self::Expr,
        at: usize,
    ) -> Result<Self::Expr, ElError>;

    /// Builds a unary-operator node.
    ///
    /// # Errors
    /// Returns [`ElError`] when the equivalent call cannot be constructed.
    fn unary(
        &mut self,
        operator: ElOperator,
        operand: Self::Expr,
        at: usize,
    ) -> Result<Self::Expr, ElError>;

    /// Builds the `exists` non-null assertion over a value generator
    /// (`ElParser.g4` `elBooleanLeaf`; `ElLexer.g4` calls `SYM_EXISTS` the
    /// "Non-null assertion operator").
    ///
    /// # Errors
    /// Returns [`ElError`] when the operand is not a value generator.
    fn attached(&mut self, operand: Self::Expr, at: usize) -> Result<Self::Expr, ElError>;

    /// Builds a quantified expression (`elForAllExpr`/`elThereExistsExpr`);
    /// `universal` distinguishes `for_all` from `there_exists`.
    ///
    /// # Errors
    /// Returns [`ElError`] when the builder cannot represent the quantifier.
    fn quantified(
        &mut self,
        universal: bool,
        variable: &str,
        collection: Self::Expr,
        condition: Self::Expr,
        at: usize,
    ) -> Result<Self::Expr, ElError>;

    /// Builds a tuple (`ElParser.g4` `elTuple`).
    ///
    /// # Errors
    /// Returns [`ElError`] when the builder cannot represent a tuple.
    fn tuple(&mut self, items: Vec<Self::Expr>, at: usize) -> Result<Self::Expr, ElError>;

    /// Builds the right-hand side of a `matches` constraint, given its
    /// verbatim source text including the `{ }` delimiters.
    ///
    /// # Errors
    /// Returns [`ElError`] when the builder does not support constraint
    /// leaves.
    fn constraint(&mut self, raw: &str, at: usize) -> Result<Self::Expr, ElError>;
}

/// Parses `src` as one EL `elBooleanExpr`, driving `builder`, and requires the
/// whole input to be consumed.
///
/// # Operator precedence
///
/// Taken from the EL operator tables, NOT from the vendored parser grammar.
/// `LANG/docs/EL/master05-expressions.adoc` §Primitive Operators lists the
/// Logical Operators in "descending precendence order" as `NOT` > `AND` >
/// `OR` > `XOR` > `IMPLIES`, and §Precedence and Parentheses makes that table
/// normative ("The precedence of operators follows the order shown in the
/// operator tables above"). `ElParser.g4` `elBooleanExpr` lists its
/// alternatives in the opposite order for `or`/`xor` (ANTLR reads them
/// ascending, which binds `xor` tighter). The docs text is the oracle, so
/// `a xor b or c` reads here as `a xor (b or c)`.
///
/// # Errors
/// Returns [`ElError`] on a lex failure, a parse failure, an unsupported
/// production, or a builder rejection.
pub fn parse_boolean_expression_with<B: ElBuilder>(
    src: &str,
    builder: &mut B,
) -> Result<B::Expr, ElError> {
    let tokens = crate::v1_1::lexer::lex_el(src).map_err(|failure| ElError::Lex {
        at: failure.span.start,
        text: failure.text,
    })?;
    parser::Parser::new(src, &tokens, builder).parse_whole_boolean_expression()
}
