// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The BEL recursive-descent parser, generic over a [`BelBuilder`].
//!
//! Grammar: `crates/openehr-lang/vendor/grammar/v1_1/base_expressions.g4` (the
//! normative BEL syntax; `docs/specs/openehr/LANG/docs/BEL/masterAppA-syntax`).
//! Operator precedence follows that grammar: `implies` < `or` < `xor` < `and` <
//! `not` < comparison/`matches` < `+ -` < `* / %` < `^` < unary < primary.
//!
//! NOTE: that order is BEL's (`base_expressions.g4` lists `boolean_expr`
//! alternatives in ascending precedence, so `xor` binds tighter than `or`),
//! and it CONFLICTS with the EL table (`LANG/docs/EL/master05-expressions.adoc`
//! §Primitive Operators: `OR` above `XOR`) — this parser implements BEL, the
//! STABLE spec whose grammar openEHR vendors (EL is DEVELOPMENT with no
//! grammar); a future EL parser must take its precedence from the EL tables,
//! never from this ordering.

use crate::nesting::Nesting;
use crate::v1_1::bel::{BelBuilder, BelError, BelLiteral};
use crate::v1_1::beom::core::operator_kind::OperatorKind;
use crate::v1_1::lexer::{Spanned, Token};

/// The parser cursor over a lexed token slice, driving a `&mut B` builder.
pub(crate) struct Parser<'a, 'b, B: BelBuilder> {
    src: &'a str,
    toks: &'a [Spanned],
    pos: usize,
    builder: &'b mut B,
    nesting: Nesting,
}

impl<'a, 'b, B: BelBuilder> Parser<'a, 'b, B> {
    pub(crate) fn new(src: &'a str, toks: &'a [Spanned], builder: &'b mut B) -> Self {
        Self {
            src,
            toks,
            pos: 0,
            builder,
            nesting: Nesting::ROOT,
        }
    }

    /// Run `production` one nesting level deeper, refusing past the bound.
    ///
    /// Every self-recursive production (a parenthesised or quantified
    /// sub-expression, a `not` or sign chain) enters through here, so the
    /// parser's native recursion is bounded by [`Nesting`].
    fn nested<T>(
        &mut self,
        production: impl FnOnce(&mut Self) -> Result<T, BelError>,
    ) -> Result<T, BelError> {
        let outer = self.nesting;
        self.nesting = outer.descend().map_err(|e| BelError::NestingTooDeep {
            at: self.at(),
            limit: e.limit,
        })?;
        let result = production(self);
        self.nesting = outer;
        result
    }

    // ── cursor helpers ────────────────────────────────────────────────────
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos).map(|s| &s.token)
    }

    fn peek_at(&self, ahead: usize) -> Option<&Token> {
        self.toks.get(self.pos + ahead).map(|s| &s.token)
    }

    fn at(&self) -> usize {
        self.toks
            .get(self.pos)
            .map_or(self.src.len(), |s| s.span.start)
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).map(|s| s.token.clone());
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, want: &Token) -> bool {
        if self.peek() == Some(want) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn err<T>(&self, message: impl Into<String>) -> Result<T, BelError> {
        Err(BelError::Parse {
            at: self.at(),
            message: message.into(),
        })
    }

    // ── statements ────────────────────────────────────────────────────────
    /// `statement_block : statement+` — every statement until end of input.
    pub(crate) fn parse_statement_block(&mut self) -> Result<Vec<B::Stmt>, BelError> {
        let mut out = Vec::new();
        while self.peek().is_some() {
            out.push(self.parse_statement()?);
        }
        Ok(out)
    }

    /// `statement : declaration | assignment | assertion`.
    fn parse_statement(&mut self) -> Result<B::Stmt, BelError> {
        match self.peek() {
            Some(Token::VariableId(_)) => match self.peek_at(1) {
                Some(Token::SymAssignment) => self.parse_assignment(),
                Some(Token::SymColon) => self.parse_variable_declaration(),
                _ => self.parse_assertion(),
            },
            // `Name : Type [= primitive_object]` — a constant declaration
            // (`base_expressions.g4` `constant_declaration`), tried first by
            // backtracking; a UC-tagged assertion resumes when the shape
            // does not complete as a constant.
            Some(Token::AlphaUcId(_)) if self.peek_at(1) == Some(&Token::SymColon) => {
                if let Some(result) = self.try_parse_constant_declaration() {
                    return result;
                }
                self.parse_assertion()
            }
            // `tag :` — a tagged assertion.
            Some(Token::AlphaLcId(_)) if self.peek_at(1) == Some(&Token::SymColon) => {
                self.parse_assertion()
            }
            _ => self.parse_assertion(),
        }
    }

    /// `binding | local_assignment : local_variable ':=' ( bound_path | expression )`.
    fn parse_assignment(&mut self) -> Result<B::Stmt, BelError> {
        let name = self.expect_variable_name()?;
        if !self.eat(&Token::SymAssignment) {
            return self.err("expected ':=' in assignment");
        }
        let source = self.parse_expr()?;
        Ok(self.builder.assignment(&name, source))
    }

    /// `variable_declaration : local_variable ':' type_id ( ':=' expression )?`.
    fn parse_variable_declaration(&mut self) -> Result<B::Stmt, BelError> {
        let name = self.expect_variable_name()?;
        if !self.eat(&Token::SymColon) {
            return self.err("expected ':' in variable declaration");
        }
        let type_id = self.parse_type_id()?;
        let init = if self.eat(&Token::SymAssignment) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(self.builder.variable_declaration(&name, &type_id, init))
    }

    /// `type_id : ALPHA_UC_ID ( '<' type_id ( ',' type_id )* '>' )?`
    /// (`base_expressions.g4`), reconstructed flat (`List<Real>`,
    /// `Interval<Integer>`, `Hash<String,Integer>`) — the declaration types
    /// of `LANG/docs/BEL/master03-language.adoc` §Typing.
    fn parse_type_id(&mut self) -> Result<String, BelError> {
        let Some(Token::AlphaUcId(root)) = self.peek().cloned() else {
            return self.err("expected a type name after ':' in a declaration");
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

    /// `constant_declaration : constant_name ':' type_id ( '=' primitive_object )?`
    /// (`base_expressions.g4`; `LANG/docs/BEL/master03-language.adoc`
    /// §Constants). Dispatched by backtracking from the shared
    /// `NAME ':' …` shape: a tagged assertion resumes if what follows the
    /// `':'` is not `type_id ( '=' … | <end> )` — the grammar leaves the two
    /// productions ambiguous and the §Constants examples settle the reading.
    fn try_parse_constant_declaration(&mut self) -> Option<Result<B::Stmt, BelError>> {
        let start = self.pos;
        let Some(Token::AlphaUcId(name)) = self.peek().cloned() else {
            return None;
        };
        self.pos += 1;
        if !self.eat(&Token::SymColon) {
            self.pos = start;
            return None;
        }
        let Ok(type_id) = self.parse_type_id() else {
            self.pos = start;
            return None;
        };
        match self.peek() {
            // `Name : Type = primitive_object` — the valued constant. The RHS
            // is the odin_values `primitive_object`: an interval value is
            // carried verbatim ([`BelLiteral::Interval`]), everything else
            // parses as the expression the scalar literals already are. (The
            // `primitive_list_value` form is unreachable here — a comma ends
            // the statement read — an honest boundary recorded on the audit.)
            Some(Token::SymEq) => {
                self.pos += 1;
                let value = if self.peek() == Some(&Token::SymIvlDelim) {
                    match self.parse_interval_literal() {
                        Ok(v) => v,
                        Err(e) => return Some(Err(e)),
                    }
                } else {
                    match self.parse_expr() {
                        Ok(v) => v,
                        Err(e) => return Some(Err(e)),
                    }
                };
                Some(Ok(self.builder.constant_declaration(
                    &name,
                    &type_id,
                    Some(value),
                )))
            }
            // `Name : Type` at end of input — a bare constant declaration.
            None => Some(Ok(self.builder.constant_declaration(&name, &type_id, None))),
            // anything else: not a constant — backtrack to the assertion read.
            _ => {
                self.pos = start;
                None
            }
        }
    }

    /// An interval `primitive_object` value (`| … |`), captured VERBATIM from
    /// the source between (and including) its two `|` delimiters — see the
    /// [`BelLiteral::Interval`] adjudication.
    #[expect(
        clippy::expect_used,
        reason = "`self.src` IS the string the token spans were produced from (parse_statements_with lexes `src` and hands the same `src` to Parser::new), and the range runs from the opening delimiter's span start to the closing delimiter's span end, so it is always an in-bounds, char-boundary slice"
    )]
    fn parse_interval_literal(&mut self) -> Result<B::Expr, BelError> {
        let open = self.pos;
        let Some(open_span) = self.toks.get(open).map(|s| s.span.clone()) else {
            return self.err("expected '|' opening an interval value");
        };
        self.pos += 1; // the opening '|'
        while let Some(entry) = self.toks.get(self.pos) {
            let close_end = entry.span.end;
            let closes = entry.token == Token::SymIvlDelim;
            self.pos += 1;
            if closes {
                let text = self
                    .src
                    .get(open_span.start..close_end)
                    .expect("a token span range should slice the source it was lexed from")
                    .to_owned();
                return Ok(self.builder.literal(BelLiteral::Interval(text)));
            }
        }
        self.err("expected '|' closing an interval value")
    }

    /// `assertion : ( ( ALPHA_LC_ID | ALPHA_UC_ID ) ':' )? boolean_expr`.
    fn parse_assertion(&mut self) -> Result<B::Stmt, BelError> {
        let tag = match (self.peek(), self.peek_at(1)) {
            (Some(Token::AlphaLcId(t) | Token::AlphaUcId(t)), Some(Token::SymColon)) => {
                let t = t.clone();
                self.pos += 2; // tag + ':'
                Some(t)
            }
            _ => None,
        };
        let expr = self.parse_expr()?;
        Ok(self.builder.assertion(tag, expr))
    }

    fn expect_variable_name(&mut self) -> Result<String, BelError> {
        match self.bump() {
            Some(Token::VariableId(v)) => Ok(v.trim_start_matches('$').to_owned()),
            _ => self.err("expected a $variable"),
        }
    }

    /// A quantifier BINDING variable: the grammar writes `VARIABLE_ID`
    /// (`$v`), but `LANG/docs/BEL/master03-language.adoc` §Container
    /// Operators states the textual syntax with a bare identifier
    /// (`for_all v : container_var | …`) — the docs text wins the conflict,
    /// so both spellings are accepted here (binding position only; the two
    /// forms denote the same bound name).
    fn expect_binding_name(&mut self) -> Result<String, BelError> {
        match self.bump() {
            Some(Token::VariableId(v)) => Ok(v.trim_start_matches('$').to_owned()),
            Some(Token::AlphaLcId(v)) => Ok(v),
            _ => self.err("expected a binding variable ($name or bare name)"),
        }
    }

    // ── expressions (precedence climbing) ─────────────────────────────────
    /// `expression`/`boolean_expr` entry: lowest-precedence `implies`.
    fn parse_expr(&mut self) -> Result<B::Expr, BelError> {
        self.nested(|p| {
            let mut left = p.parse_or()?;
            while p.eat(&Token::SymImplies) {
                let right = p.parse_or()?;
                left = p.builder.binary(kind("implies"), "implies", left, right);
            }
            Ok(left)
        })
    }

    fn parse_or(&mut self) -> Result<B::Expr, BelError> {
        let mut left = self.parse_xor()?;
        while self.eat(&Token::SymOr) {
            let right = self.parse_xor()?;
            left = self.builder.binary(kind("or"), "or", left, right);
        }
        Ok(left)
    }

    fn parse_xor(&mut self) -> Result<B::Expr, BelError> {
        let mut left = self.parse_and()?;
        while self.eat(&Token::SymXor) {
            let right = self.parse_and()?;
            left = self.builder.binary(kind("xor"), "xor", left, right);
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<B::Expr, BelError> {
        let mut left = self.parse_not()?;
        while self.eat(&Token::SymAnd) {
            let right = self.parse_not()?;
            left = self.builder.binary(kind("and"), "and", left, right);
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<B::Expr, BelError> {
        if self.eat(&Token::SymNot) {
            let operand = self.nested(Self::parse_not)?;
            return Ok(self.builder.unary(kind("not"), "not", operand));
        }
        self.parse_comparison()
    }

    /// A comparison / `matches` leaf over arithmetic operands.
    fn parse_comparison(&mut self) -> Result<B::Expr, BelError> {
        let left = self.parse_additive()?;
        if self.peek() == Some(&Token::SymMatches) {
            self.pos += 1;
            let (raw, at) = self.constraint_rhs()?;
            let rhs = self.builder.constraint(&raw, at)?;
            return Ok(self.builder.binary(kind("matches"), "matches", left, rhs));
        }
        if let Some((op, sym)) = self.peek().and_then(relational) {
            self.pos += 1;
            let right = self.parse_additive()?;
            return Ok(self.builder.binary(kind(op), sym, left, right));
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<B::Expr, BelError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let (op, sym) = match self.peek() {
                Some(Token::SymPlus) => ("plus", "+"),
                Some(Token::SymMinus) => ("minus", "-"),
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_multiplicative()?;
            left = self.builder.binary(kind(op), sym, left, right);
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<B::Expr, BelError> {
        let mut left = self.parse_exponent()?;
        loop {
            let (op, sym) = match self.peek() {
                Some(Token::SymStar) => ("multiply", "*"),
                Some(Token::SymSlash) => ("divide", "/"),
                Some(Token::SymPercent) => ("modulo", "%"),
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_exponent()?;
            left = self.builder.binary(kind(op), sym, left, right);
        }
        Ok(left)
    }

    /// `^` — right-associative (`base_expressions.g4` `<assoc=right>`).
    fn parse_exponent(&mut self) -> Result<B::Expr, BelError> {
        let left = self.parse_unary()?;
        if self.eat(&Token::SymCarat) {
            let right = self.parse_exponent()?;
            return Ok(self.builder.binary(kind("exponent"), "^", left, right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<B::Expr, BelError> {
        if self.eat(&Token::SymMinus) {
            let operand = self.nested(Self::parse_unary)?;
            return Ok(self.builder.unary(kind("minus"), "-", operand));
        }
        if self.eat(&Token::SymPlus) {
            return self.nested(Self::parse_unary);
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<B::Expr, BelError> {
        match self.peek().cloned() {
            Some(Token::LParen) => {
                self.pos += 1;
                let inner = self.parse_expr()?;
                if !self.eat(&Token::RParen) {
                    return self.err("expected ')'");
                }
                Ok(inner)
            }
            Some(Token::SymForAll) => self.parse_for_all(),
            Some(Token::SymThereExists) => self.parse_there_exists(),
            Some(Token::SymExists) => {
                self.pos += 1;
                let operand = self.parse_ref_leaf()?;
                Ok(self.builder.unary(kind("exists"), "exists", operand))
            }
            Some(Token::SymTrue) => {
                self.pos += 1;
                Ok(self.builder.literal(BelLiteral::Boolean(true)))
            }
            Some(Token::SymFalse) => {
                self.pos += 1;
                Ok(self.builder.literal(BelLiteral::Boolean(false)))
            }
            Some(Token::Integer(s)) => {
                self.pos += 1;
                let v = s
                    .parse::<i64>()
                    .map_err(|e| self.parse_err(format!("invalid integer {s:?}: {e}")))?;
                Ok(self.builder.literal(BelLiteral::Integer(v)))
            }
            Some(Token::Real(s)) => {
                self.pos += 1;
                let v = s
                    .parse::<f64>()
                    .map_err(|e| self.parse_err(format!("invalid real {s:?}: {e}")))?;
                Ok(self.builder.literal(BelLiteral::Real(v)))
            }
            Some(Token::String(s)) => {
                self.pos += 1;
                Ok(self.builder.literal(BelLiteral::String(decode_string(&s))))
            }
            Some(Token::Character(s)) => {
                self.pos += 1;
                let c = decode_char(&s);
                Ok(self.builder.literal(BelLiteral::Character(c)))
            }
            Some(Token::Iso8601Date(s)) => {
                self.pos += 1;
                Ok(self.builder.literal(BelLiteral::Date(s)))
            }
            Some(Token::Iso8601Time(s)) => {
                self.pos += 1;
                Ok(self.builder.literal(BelLiteral::Time(s)))
            }
            Some(Token::Iso8601DateTime(s)) => {
                self.pos += 1;
                Ok(self.builder.literal(BelLiteral::DateTime(s)))
            }
            Some(Token::Iso8601Duration(s)) => {
                self.pos += 1;
                Ok(self.builder.literal(BelLiteral::Duration(s)))
            }
            // `[terminology::code]` — a Terminology_code literal
            // (`LANG/docs/BEL/master03-language.adoc` §Literals; the grammar
            // reaches TERM_CODE_REF via its odin_values import). NOTE: the
            // grammar's arithmetic_leaf omits the token — the BEOM-normative
            // reading (`TYPE_DEF_TERMINOLOGY_CODE` exists precisely for these
            // values) plus §Literals ground the leaf position, for equality
            // use.
            Some(Token::TermCodeRef(code)) => {
                self.pos += 1;
                Ok(self.builder.literal(BelLiteral::TermCode(code)))
            }
            Some(Token::AlphaLcId(name)) if self.peek_at(1) == Some(&Token::LParen) => {
                self.parse_function_call(&name)
            }
            _ => self.parse_ref_leaf(),
        }
    }

    /// A reference leaf: `$variable`, `$var/path`, an archetype/data path, or a
    /// bare identifier used as a value reference.
    fn parse_ref_leaf(&mut self) -> Result<B::Expr, BelError> {
        match self.bump() {
            Some(Token::VariableId(v)) => Ok(self.builder.variable_ref(v.trim_start_matches('$'))),
            Some(Token::VariableWithPath(v)) => self.builder.path_ref(&v),
            Some(Token::AdlPath(p)) => self.builder.path_ref(&p),
            Some(Token::AlphaLcId(id) | Token::AlphaUcId(id)) => self.builder.path_ref(&id),
            _ => self.err("expected a value reference (path, $variable, or name)"),
        }
    }

    fn parse_function_call(&mut self, name: &str) -> Result<B::Expr, BelError> {
        self.pos += 1; // name
        self.pos += 1; // '('
        let mut args = Vec::new();
        if self.peek() != Some(&Token::RParen) {
            loop {
                args.push(self.parse_expr()?);
                if !self.eat(&Token::SymComma) {
                    break;
                }
            }
        }
        if !self.eat(&Token::RParen) {
            return self.err("expected ')' closing function arguments");
        }
        Ok(self.builder.function_call(name, args))
    }

    /// `for_all_expr : SYM_FOR_ALL VARIABLE_ID ( ':' | 'in' ) value_ref '|'? boolean_expr`.
    fn parse_for_all(&mut self) -> Result<B::Expr, BelError> {
        self.pos += 1; // for_all
        let var = self.expect_binding_name()?;
        if !self.eat(&Token::SymColon) && !self.eat(&Token::SymIn) {
            return self.err("expected ':' or 'in' after the for_all variable");
        }
        let collection = self.parse_ref_leaf()?;
        self.eat(&Token::SymIvlDelim); // optional '|'
        let condition = self.parse_expr()?;
        self.builder.for_all(&var, collection, condition)
    }

    /// `there_exists_expr` — existential quantification (mapped to the same
    /// quantifier node as `for_all` with an `exists` operator via the builder).
    ///
    /// The `∃` symbol is ALSO the symbolic rendering of the path-existence
    /// operator in ADL 1.4 assertions (ADL1.4 `master06-assertions.adoc`
    /// §Keywords equates `exists` ↔ `∃`), whose operand is a path, not a
    /// `$variable` binding — so a non-variable operand dispatches to the
    /// same unary the `exists` keyword builds.
    fn parse_there_exists(&mut self) -> Result<B::Expr, BelError> {
        self.pos += 1; // there_exists
        let is_binding = matches!(self.peek(), Some(Token::VariableId(_)))
            || (matches!(self.peek(), Some(Token::AlphaLcId(_)))
                && matches!(self.peek_at(1), Some(Token::SymColon | Token::SymIn)));
        if !is_binding {
            let operand = self.parse_ref_leaf()?;
            return Ok(self.builder.unary(kind("exists"), "exists", operand));
        }
        let var = self.expect_binding_name()?;
        if !self.eat(&Token::SymColon) && !self.eat(&Token::SymIn) {
            return self.err("expected ':' or 'in' after the there_exists variable");
        }
        let collection = self.parse_ref_leaf()?;
        self.eat(&Token::SymIvlDelim);
        let condition = self.parse_expr()?;
        self.builder.for_all(&var, collection, condition)
    }

    /// Capture the verbatim source of a `matches` right-hand side: either a
    /// single `CONTAINED_REGEXP` token, or a `{ … }` block captured by
    /// brace-depth over the token stream. Returns `(raw_text, byte_offset)`.
    #[expect(
        clippy::expect_used,
        reason = "`self.src` IS the string the token spans were produced from (parse_statements_with lexes `src` and hands the same `src` to Parser::new), and the range runs from an opening token's span start to a later token's span end, so it is always an in-bounds, char-boundary slice"
    )]
    fn constraint_rhs(&mut self) -> Result<(String, usize), BelError> {
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
        while let Some(s) = self.toks.get(self.pos) {
            let end = s.span.end;
            if s.token == Token::LCurly {
                depth += 1;
            } else if s.token == Token::RCurly {
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
    fn parse_err(&self, message: impl Into<String>) -> BelError {
        BelError::Parse {
            at: self.at(),
            message: message.into(),
        }
    }
}

/// An [`OperatorKind`] from its `OPERATOR_KIND` constant name
/// (`master04-expression_object_model.adoc`); an unknown token is tolerated as
/// `OperatorKind::Other`.
fn kind(name: &str) -> OperatorKind {
    OperatorKind::from_wire(name)
}

/// Map a relational/equality token to its `(operator_kind, symbol)`.
fn relational(t: &Token) -> Option<(&'static str, &'static str)> {
    match t {
        Token::SymEq => Some(("eq", "=")),
        Token::SymNe => Some(("ne", "!=")),
        Token::SymLt => Some(("lt", "<")),
        Token::SymLe => Some(("le", "<=")),
        Token::SymGt => Some(("gt", ">")),
        Token::SymGe => Some(("ge", ">=")),
        _ => None,
    }
}

/// Decode a double-quoted BEL string literal (strip delimiters, decode the
/// `master03` escapes).
///
/// The lexer (`validate_string`) has already run
/// [`crate::v1_1::escape::validate`] over the same text, so the decode cannot fail
/// here.
#[expect(
    clippy::expect_used,
    reason = "`Token::String` only exists when the lexer's validate_string ran crate::v1_1::escape::validate over the same body and it succeeded, so this decode of that body cannot fail"
)]
fn decode_string(raw: &str) -> String {
    crate::v1_1::escape::decode_string_literal(raw)
        .expect("a lexer-validated string literal should decode")
}

/// Decode a single-quoted character literal to a `char`.
///
/// The lexer (`validate_char`) admits only the six quoted forms in a character
/// literal, so the decode cannot fail here, and its token regex admits exactly
/// one body character, so the decoded literal is never empty.
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
