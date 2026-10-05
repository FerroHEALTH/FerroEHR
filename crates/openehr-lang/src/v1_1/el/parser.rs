// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The EL recursive-descent parser, generic over an [`ElBuilder`].
//!
//! Grammar: `crates/openehr-lang/vendor/grammar/v1_1/ElParser.g4` (the normative EL
//! syntax; `docs/specs/openehr/LANG/docs/EL/masterAppA-syntax.adoc` is an
//! include of it). Precedence comes from the EL operator tables — see
//! [`crate::v1_1::el::parse_boolean_expression_with`].

use crate::v1_1::el::{ElBuilder, ElError, ElLiteral, ElOperator};
use crate::v1_1::lexer::{Spanned, Token};

/// The parser cursor over a lexed token slice, driving a `&mut B` builder.
pub(crate) struct Parser<'a, 'b, B: ElBuilder> {
    src: &'a str,
    toks: &'a [Spanned],
    pos: usize,
    builder: &'b mut B,
}

impl<'a, 'b, B: ElBuilder> Parser<'a, 'b, B> {
    pub(crate) fn new(src: &'a str, toks: &'a [Spanned], builder: &'b mut B) -> Self {
        Self {
            src,
            toks,
            pos: 0,
            builder,
        }
    }

    // ── cursor helpers ────────────────────────────────────────────────────
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos).map(|s| &s.token)
    }

    fn at(&self) -> usize {
        self.toks
            .get(self.pos)
            .map_or(self.src.len(), |s| s.span.start)
    }

    fn eat(&mut self, want: &Token) -> bool {
        if self.peek() == Some(want) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn err<T>(&self, message: impl Into<String>) -> Result<T, ElError> {
        Err(ElError::Parse {
            at: self.at(),
            message: message.into(),
        })
    }

    fn unsupported<T>(&self, message: impl Into<String>) -> Result<T, ElError> {
        Err(ElError::Unsupported {
            at: self.at(),
            message: message.into(),
        })
    }

    /// `elBooleanExpr` over the whole input.
    pub(crate) fn parse_whole_boolean_expression(&mut self) -> Result<B::Expr, ElError> {
        if self.peek().is_none() {
            return self.err("expected a boolean expression, found empty input");
        }
        let expr = self.parse_boolean()?;
        if self.peek().is_some() {
            return self.err("unexpected trailing input after the boolean expression");
        }
        Ok(expr)
    }

    // ── boolean ladder (EL table order: not > and > or > xor > implies) ───
    /// The lowest boolean level: `implies`, then the `⇔` equivalence the
    /// grammar puts beside it (`ElParser.g4` `elBooleanExpr`).
    fn parse_boolean(&mut self) -> Result<B::Expr, ElError> {
        let mut left = self.parse_xor()?;
        while let Some(operator) = self
            .peek()
            .filter(|t| matches!(t, Token::SymImplies | Token::SymIff))
            .and_then(ElOperator::of)
        {
            let at = self.at();
            self.pos += 1;
            let right = self.parse_xor()?;
            left = self.builder.binary(operator, left, right, at)?;
        }
        Ok(left)
    }

    fn parse_xor(&mut self) -> Result<B::Expr, ElError> {
        let mut left = self.parse_or()?;
        while self.peek() == Some(&Token::SymXor) {
            let at = self.at();
            self.pos += 1;
            let right = self.parse_or()?;
            left = self
                .builder
                .binary(operator_of(&Token::SymXor)?, left, right, at)?;
        }
        Ok(left)
    }

    fn parse_or(&mut self) -> Result<B::Expr, ElError> {
        let mut left = self.parse_and()?;
        while self.peek() == Some(&Token::SymOr) {
            let at = self.at();
            self.pos += 1;
            let right = self.parse_and()?;
            left = self
                .builder
                .binary(operator_of(&Token::SymOr)?, left, right, at)?;
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<B::Expr, ElError> {
        let mut left = self.parse_not()?;
        while self.peek() == Some(&Token::SymAnd) {
            let at = self.at();
            self.pos += 1;
            let right = self.parse_not()?;
            left = self
                .builder
                .binary(operator_of(&Token::SymAnd)?, left, right, at)?;
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<B::Expr, ElError> {
        if self.peek() == Some(&Token::SymNot) {
            let at = self.at();
            self.pos += 1;
            let operand = self.parse_not()?;
            return self
                .builder
                .unary(operator_of(&Token::SymNot)?, operand, at);
        }
        self.parse_comparison()
    }

    /// `elArithmeticComparisonExpr` / `elObjectComparisonExpr` /
    /// `elArithmeticConstraintExpr` / `elGeneralConstraintExpr` — one
    /// comparison or `matches` over arithmetic-level operands.
    fn parse_comparison(&mut self) -> Result<B::Expr, ElError> {
        let left = self.parse_additive()?;
        if self.peek() == Some(&Token::SymMatches) {
            let at = self.at();
            self.pos += 1;
            let (raw, raw_at) = self.constraint_rhs()?;
            let right = self.builder.constraint(&raw, raw_at)?;
            return self
                .builder
                .binary(operator_of(&Token::SymMatches)?, left, right, at);
        }
        let comparison = self.peek().filter(|t| {
            matches!(
                t,
                Token::SymEq
                    | Token::SymNe
                    | Token::SymLt
                    | Token::SymLe
                    | Token::SymGt
                    | Token::SymGe
            )
        });
        let Some(operator) = comparison.and_then(ElOperator::of) else {
            return Ok(left);
        };
        let at = self.at();
        self.pos += 1;
        let right = self.parse_additive()?;
        self.builder.binary(operator, left, right, at)
    }

    // ── arithmetic ladder (`ElParser.g4` `elArithmeticExpr`) ──────────────
    fn parse_additive(&mut self) -> Result<B::Expr, ElError> {
        let mut left = self.parse_multiplicative()?;
        while let Some(operator) = self
            .peek()
            .filter(|t| matches!(t, Token::SymPlus | Token::SymMinus))
            .and_then(ElOperator::of)
        {
            let at = self.at();
            self.pos += 1;
            let right = self.parse_multiplicative()?;
            left = self.builder.binary(operator, left, right, at)?;
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<B::Expr, ElError> {
        let mut left = self.parse_exponent()?;
        while let Some(operator) = self
            .peek()
            .filter(|t| matches!(t, Token::SymStar | Token::SymSlash | Token::SymPercent))
            .and_then(ElOperator::of)
        {
            let at = self.at();
            self.pos += 1;
            let right = self.parse_exponent()?;
            left = self.builder.binary(operator, left, right, at)?;
        }
        Ok(left)
    }

    /// `^` — right-associative (`ElParser.g4` `<assoc=right>`).
    fn parse_exponent(&mut self) -> Result<B::Expr, ElError> {
        let left = self.parse_unary()?;
        if self.peek() == Some(&Token::SymCarat) {
            let at = self.at();
            self.pos += 1;
            let right = self.parse_exponent()?;
            return self
                .builder
                .binary(operator_of(&Token::SymCarat)?, left, right, at);
        }
        Ok(left)
    }

    /// A prefix sign. `ElParser.g4` has no unary-minus alternative; the
    /// `SYM_MINUS` of a negative numeric literal is folded into the literal
    /// here so `-1` reads as one value (`ElLexer.g4` INTEGER is unsigned).
    fn parse_unary(&mut self) -> Result<B::Expr, ElError> {
        if self.peek() == Some(&Token::SymMinus) {
            let at = self.at();
            self.pos += 1;
            let operand = self.parse_unary()?;
            return self
                .builder
                .unary(operator_of(&Token::SymMinus)?, operand, at);
        }
        if self.eat(&Token::SymPlus) {
            return self.parse_unary();
        }
        self.parse_leaf()
    }

    /// `elBooleanLeaf` / `elArithmeticLeaf` / `elSimpleTerminal`.
    fn parse_leaf(&mut self) -> Result<B::Expr, ElError> {
        let at = self.at();
        match self.peek().cloned() {
            Some(Token::LParen) => {
                self.pos += 1;
                let inner = self.parse_boolean()?;
                if !self.eat(&Token::RParen) {
                    return self.err("expected ')'");
                }
                Ok(inner)
            }
            Some(Token::LBracket) => self.parse_tuple(),
            Some(Token::SymForAll) => self.parse_quantified(true),
            Some(Token::SymThereExists) => self.parse_quantified(false),
            Some(Token::SymExists) => {
                self.pos += 1;
                let operand = self.parse_value_generator()?;
                self.builder.attached(operand, at)
            }
            Some(Token::SymCase | Token::SymChoice) => self.unsupported(
                "decision tables (`ElParser.g4` dlCaseTable/dlConditionTable) are not realized",
            ),
            Some(Token::SymTrue) => {
                self.pos += 1;
                self.builder.literal(ElLiteral::Boolean(true), at)
            }
            Some(Token::SymFalse) => {
                self.pos += 1;
                self.builder.literal(ElLiteral::Boolean(false), at)
            }
            Some(Token::Integer(text)) => {
                self.pos += 1;
                let value = text
                    .parse::<i64>()
                    .map_err(|e| self.parse_err(format!("invalid integer {text:?}: {e}")))?;
                self.builder.literal(ElLiteral::Integer(value), at)
            }
            Some(Token::Real(text)) => {
                self.pos += 1;
                let value = text
                    .parse::<f64>()
                    .map_err(|e| self.parse_err(format!("invalid real {text:?}: {e}")))?;
                self.builder.literal(ElLiteral::Real(value), at)
            }
            Some(Token::String(text)) => {
                self.pos += 1;
                self.builder
                    .literal(ElLiteral::String(decode_string(&text)), at)
            }
            Some(Token::Character(text)) => {
                self.pos += 1;
                self.builder
                    .literal(ElLiteral::Character(decode_char(&text)), at)
            }
            Some(Token::Iso8601Date(text)) => {
                self.pos += 1;
                self.builder.literal(ElLiteral::Date(text), at)
            }
            Some(Token::Iso8601Time(text)) => {
                self.pos += 1;
                self.builder.literal(ElLiteral::Time(text), at)
            }
            Some(Token::Iso8601DateTime(text)) => {
                self.pos += 1;
                self.builder.literal(ElLiteral::DateTime(text), at)
            }
            Some(Token::Iso8601Duration(text)) => {
                self.pos += 1;
                self.builder.literal(ElLiteral::Duration(text), at)
            }
            Some(Token::TermCodeRef(text) | Token::LocalTermCodeRef(text)) => {
                self.pos += 1;
                self.builder.literal(ElLiteral::TermCode(text), at)
            }
            _ => self.parse_value_generator(),
        }
    }

    /// `elTuple : '[' elExpression ( ',' elExpression )+ ']'`.
    fn parse_tuple(&mut self) -> Result<B::Expr, ElError> {
        let at = self.at();
        self.pos += 1;
        let mut items = vec![self.parse_boolean()?];
        while self.eat(&Token::SymComma) {
            items.push(self.parse_boolean()?);
        }
        if !self.eat(&Token::RBracket) {
            return self.err("expected ']' closing a tuple");
        }
        if items.len() < 2 {
            return self.err("a tuple needs at least two items");
        }
        self.builder.tuple(items, at)
    }

    /// `elForAllExpr` / `elThereExistsExpr`: `SYM elLocalVariableId ':'
    /// elValueGenerator '¦' elBooleanExpr`.
    fn parse_quantified(&mut self, universal: bool) -> Result<B::Expr, ElError> {
        let at = self.at();
        self.pos += 1;
        let Some(Token::AlphaLcId(variable)) = self.peek().cloned() else {
            return self.err("expected a binding variable name after the quantifier");
        };
        self.pos += 1;
        if !self.eat(&Token::SymColon) {
            return self.err("expected ':' after the quantifier variable");
        }
        let collection = self.parse_value_generator()?;
        if !self.eat(&Token::SymBrokenBar) {
            return self.err("expected '\u{00A6}' before the quantified condition");
        }
        let condition = self.parse_boolean()?;
        self.builder
            .quantified(universal, &variable, collection, condition, at)
    }

    /// `elValueGenerator : SYM_SELF | elBareRef | elScopedFeatureRef`, with
    /// `elScoper` folded in: each `.`-separated element scopes the next.
    fn parse_value_generator(&mut self) -> Result<B::Expr, ElError> {
        let at = self.at();
        let mut current = match self.peek().cloned() {
            Some(Token::SymSelf) => {
                self.pos += 1;
                self.builder.self_ref(at)?
            }
            Some(Token::SymResult) => {
                self.pos += 1;
                self.builder.result_ref(at)?
            }
            Some(Token::VariableId(name)) => {
                self.pos += 1;
                self.builder
                    .bound_variable(name.trim_start_matches('$'), at)?
            }
            // `elScoper : '{' typeId '}' '.' …`
            Some(Token::LCurly) => {
                self.pos += 1;
                let type_id = self.parse_type_id()?;
                if !self.eat(&Token::RCurly) {
                    return self.err("expected '}' closing a type scoper");
                }
                self.builder.type_ref(&type_id, at)?
            }
            Some(Token::AlphaLcId(_) | Token::AlphaUcId(_)) => self.parse_bare_ref(None)?,
            _ => return self.err("expected a value reference (Self, $variable, name, or {Type})"),
        };
        while self.peek() == Some(&Token::SymDot) {
            self.pos += 1;
            current = self.parse_bare_ref(Some(current))?;
        }
        Ok(current)
    }

    /// `elBareRef : elInstantiableRef | elFunctionCall | elConstantId`, in
    /// the scope of `scoper` when there is one.
    fn parse_bare_ref(&mut self, scoper: Option<B::Expr>) -> Result<B::Expr, ElError> {
        let at = self.at();
        let name = match self.peek().cloned() {
            Some(Token::AlphaLcId(name) | Token::AlphaUcId(name)) => name,
            Some(Token::SymResult) if scoper.is_none() => {
                self.pos += 1;
                return self.builder.result_ref(at);
            }
            _ => return self.err("expected a feature name"),
        };
        self.pos += 1;
        // `elFunctionCall : LC_ID ( '(' elExprList ')' )?` — an argument list
        // is optional, and `elExprList` is non-empty, so `f()` has no
        // production. It is accepted here as the zero-argument call the
        // openEHR schemas write (`signature().result /= Void`).
        let args = if self.peek() == Some(&Token::LParen) {
            self.pos += 1;
            let mut list = Vec::new();
            if self.peek() != Some(&Token::RParen) {
                loop {
                    list.push(self.parse_boolean()?);
                    if !self.eat(&Token::SymComma) {
                        break;
                    }
                }
            }
            if !self.eat(&Token::RParen) {
                return self.err("expected ')' closing an argument list");
            }
            Some(list)
        } else {
            None
        };
        self.builder.feature_ref(scoper, &name, args, at)
    }

    /// `typeId : UC_ID ( '<' typeId ( ',' typeId )* '>' )?`, reconstructed
    /// flat (`List<Real>`, `Hash<String,Integer>`).
    fn parse_type_id(&mut self) -> Result<String, ElError> {
        let Some(Token::AlphaUcId(root)) = self.peek().cloned() else {
            return self.err("expected a type name");
        };
        self.pos += 1;
        if !self.eat(&Token::SymLt) {
            return Ok(root);
        }
        let mut parameters = vec![self.parse_type_id()?];
        while self.eat(&Token::SymComma) {
            parameters.push(self.parse_type_id()?);
        }
        if !self.eat(&Token::SymGt) {
            return self.err("expected '>' closing the generic type parameters");
        }
        Ok(format!("{root}<{}>", parameters.join(",")))
    }

    /// Captures the verbatim `{ … }` source of a `matches` right-hand side by
    /// brace depth, or the single `CONTAINED_REGEXP` token.
    #[expect(
        clippy::expect_used,
        reason = "`self.src` IS the string the token spans were produced from (parse_boolean_expression_with lexes `src` and hands the same `src` to Parser::new), and the range runs from an opening token's span start to a later token's span end, so it is always an in-bounds, char-boundary slice"
    )]
    fn constraint_rhs(&mut self) -> Result<(String, usize), ElError> {
        if let Some(Token::ContainedRegexp(raw)) = self.peek().cloned() {
            let at = self.at();
            self.pos += 1;
            return Ok((raw, at));
        }
        let Some(open) = self.toks.get(self.pos) else {
            return self.err("expected '{' or a regex after 'matches'");
        };
        if open.token != Token::LCurly {
            return self.err("expected '{' or a regex after 'matches'");
        }
        let start = open.span.start;
        let mut depth = 0i32;
        while let Some(entry) = self.toks.get(self.pos) {
            let end = entry.span.end;
            if entry.token == Token::LCurly {
                depth += 1;
            } else if entry.token == Token::RCurly {
                depth -= 1;
            }
            self.pos += 1;
            if depth == 0 {
                let raw = self
                    .src
                    .get(start..end)
                    .expect("a token span range should slice the source it was lexed from")
                    .to_owned();
                return Ok((raw, start));
            }
        }
        self.err("unterminated '{ … }' constraint after 'matches'")
    }

    /// A positional parse diagnostic at the current token.
    ///
    /// NOTE: a `FromStr` failure folded into `message` by a caller stays
    /// flattened rather than carried as a source (RFC 0201) — the located
    /// message IS the answer a grammar diagnostic exists to give.
    fn parse_err(&self, message: impl Into<String>) -> ElError {
        ElError::Parse {
            at: self.at(),
            message: message.into(),
        }
    }
}

/// The [`ElOperator`] of a token the caller has already matched.
fn operator_of(token: &Token) -> Result<ElOperator, ElError> {
    ElOperator::of(token).ok_or_else(|| ElError::Parse {
        at: 0,
        message: format!("{token:?} is not an EL operator"),
    })
}

/// Decodes a double-quoted EL string literal (strip delimiters, decode the
/// `master03` escapes).
///
/// The lexer has already run [`crate::v1_1::escape::validate`] over the same text,
/// so the decode cannot fail here.
#[expect(
    clippy::expect_used,
    reason = "`Token::String` only exists when the lexer's validate_string ran crate::v1_1::escape::validate over the same body and it succeeded, so this decode of that body cannot fail"
)]
fn decode_string(raw: &str) -> String {
    crate::v1_1::escape::decode_string_literal(raw)
        .expect("a lexer-validated string literal should decode")
}

/// Decodes a single-quoted character literal to a `char`.
///
/// The lexer admits only the six quoted forms in a character literal, so the
/// decode cannot fail here, and its token regex admits exactly one body
/// character, so the decoded literal is never empty.
#[expect(
    clippy::expect_used,
    reason = "`Token::Character` only exists when the lexer's validate_char admitted the body, which restricts an escape to the six quoted forms none of which can fail to decode; the same token regex admits one body character or one two-character escape, each decoding to exactly one char, so the literal is never empty"
)]
fn decode_char(raw: &str) -> char {
    crate::v1_1::escape::decode_character_literal(raw)
        .expect("a lexer-validated character literal should decode")
        .chars()
        .next()
        .expect("a lexer-validated character literal should decode to one character")
}
