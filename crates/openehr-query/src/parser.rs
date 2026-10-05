// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL parser — a `chumsky` parser transcribed from `AqlParser.g4`, turning the
//! [`crate::lexer`] token stream into an [`crate::ast::SelectQuery`].
//!
//! Precedence: within both `containsExpr` and `whereExpr`, `AND` binds tighter
//! than `OR` (the grammar's left recursion is realized here as `or` over `and`
//! over atoms), with parenthesized grouping.
//!
//! Overflowing integer literals and `TOP`/`LIMIT`/`OFFSET` counts are reported
//! as parse errors, never silently coerced.

use crate::ast::{
    AggregateCall, ArchetypePredicate, ClassExprOperand, ColumnExpr, CompareOperand,
    ContainsConstraint, ContainsExpr, FunctionCall, IdentifiedExpr, IdentifiedPath, LikeOperand,
    Limit, MatchesOperand, NodeNameConstraint, NodePredicate, ObjectPath, OrderByExpr, PathPart,
    PathPredicate, PathPredicateOperand, Primitive, SelectClause, SelectExpr, SelectQuery,
    SortOrder, Span, StandardPredicate, StatFunc, Terminal, TerminologyFunction, Top, TopDirection,
    ValueListItem, VersionPredicate, WhereExpr,
};
use crate::lexer::{CompOp, SpannedTokens, Token};
use crate::visit::VisitMut;
use chumsky::prelude::*;

// The chumsky extra-parameter alias. `chumsky::extra::Err` stays fully
// qualified deliberately: shortening it to `Err<..>` would make this alias
// refer to itself.
#[expect(
    unused_qualifications,
    reason = "the local alias shadows the name being qualified — dropping the path makes the definition self-referential"
)]
type Err<'a> = chumsky::extra::Err<Simple<'a, Token>>;

/// One position at which the token stream left the grammar.
///
/// The position is reported twice: [`SyntaxFault::tokens`] indexes the parser's
/// input and [`SyntaxFault::bytes`] locates it in the source text.
#[derive(Debug, Clone, PartialEq)]
pub struct SyntaxFault {
    /// The half-open range of token indices the parser was looking at.
    pub tokens: core::ops::Range<usize>,
    /// The half-open byte range of the source those tokens cover.
    ///
    /// `Some` whenever the parse ran over a spanned stream ([`parse_spanned`],
    /// [`parse_str`]); `None` for [`parse`], whose bare `&[Token]` input
    /// carries no source positions.
    pub bytes: Option<core::ops::Range<usize>>,
    /// The token found there, or `None` when the input ended early.
    pub found: Option<Token>,
}

impl std::fmt::Display for SyntaxFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.found {
            Some(token) => write!(f, "found '{token:?}' at {:?}", self.tokens),
            None => write!(f, "found end of input at {:?}", self.tokens),
        }
    }
}

/// Why an AQL source failed to become a [`SelectQuery`].
///
/// The two variants are the two passes, so a caller branches on the pass that
/// refused rather than on the message. Semantic rejections belong to the
/// engine that consumes the AST, not here.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParseError {
    /// The source did not tokenize.
    // NOTE: `#[error("{0}")]` and not `transparent`, which forwards `source()`
    // to the inner error and so erases the lex error from the cause chain
    // (<https://docs.rs/thiserror/latest/thiserror/derive.Error.html>).
    #[error("{0}")]
    Lex(#[from] crate::lexer::LexError),
    /// The tokens did not match the grammar.
    #[error("{}", faults.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Syntax {
        /// Every position the parser reported, in report order.
        faults: Vec<SyntaxFault>,
    },
}

/// Parses a token slice into a [`SelectQuery`].
///
/// A bare token slice carries no source positions, so every reported
/// [`SyntaxFault`] has `bytes: None` and every [`Span`] in the tree is
/// unknown; use [`parse_spanned`] to keep them.
///
/// # Errors
/// [`ParseError::Syntax`], carrying every token position the parser reported.
pub fn parse(tokens: &[Token]) -> Result<SelectQuery, ParseError> {
    run(tokens, None)
}

/// Parses a spanned token stream into a [`SelectQuery`], keeping source
/// positions: on every [`Span`] in the tree, and on any failure.
///
/// # Errors
/// [`ParseError::Syntax`], each fault carrying both the token indices and the
/// byte range of the source they cover.
pub fn parse_spanned(tokens: &SpannedTokens) -> Result<SelectQuery, ParseError> {
    run(tokens.tokens(), Some(tokens))
}

fn run(tokens: &[Token], spanned: Option<&SpannedTokens>) -> Result<SelectQuery, ParseError> {
    let mut parsed = query().parse(tokens).into_result().map_err(|errs| {
        let faults = errs
            .iter()
            .map(|e: &Simple<'_, Token>| {
                let at = e.span().into_range();
                SyntaxFault {
                    bytes: spanned.map(|stream| stream.byte_span(&at)),
                    tokens: at,
                    found: e.found().cloned(),
                }
            })
            .collect();
        ParseError::Syntax { faults }
    })?;
    ResolveSpans(spanned).visit_select_query_mut(&mut parsed);
    Ok(parsed)
}

/// Turns the token-index spans the grammar records into byte spans of the
/// source, or into unknown spans when the parse ran over a bare token slice.
struct ResolveSpans<'s>(Option<&'s SpannedTokens>);

impl VisitMut for ResolveSpans<'_> {
    fn visit_span_mut(&mut self, node: &mut Span) {
        *node = match (self.0, node.bytes()) {
            (Some(stream), Some(tokens)) => Span::new(stream.byte_span(&tokens)),
            _ => Span::default(),
        };
    }
}

/// The span of a parsed node while it is still a token-index range;
/// [`ResolveSpans`] maps it to bytes once the whole query has parsed.
fn token_span(range: core::ops::Range<usize>) -> Span {
    Span::new(range)
}

/// Lexes then parses `src` in one step.
///
/// # Errors
/// [`ParseError::Lex`] if the source does not tokenize, otherwise
/// [`ParseError::Syntax`], whose faults locate the failure in `src` itself.
pub fn parse_str(src: &str) -> Result<SelectQuery, ParseError> {
    parse_spanned(&crate::lexer::lex_spanned(src)?)
}

// ── leaf parsers ─────────────────────────────────────────────────────────────

fn ident<'a>() -> impl Parser<'a, &'a [Token], String, Err<'a>> + Clone {
    select! { Token::Identifier(s) => s }
}

/// Strips the quotes from a lexed string literal and unescapes it per
/// `AqlLexer.g4` `ESCAPE_SEQ` / `OCTAL_ESC` / `UTF8CHAR`, so the AST carries
/// the decoded value rather than the raw source text.
pub(crate) fn unquote(s: &str) -> String {
    // A value too short to be quoted, or a range that is not a UTF-8 boundary,
    // passes through unchanged rather than panicking.
    let inner = s
        .len()
        .checked_sub(1)
        .and_then(|end| s.get(1..end))
        .unwrap_or(s);
    unescape(inner)
}

/// Decodes AQL string escapes: quotes, `\\`, the control escapes (`abfnrtv`),
/// `UTF8CHAR: '\\u' HEX{4}`, and `OCTAL_ESC` (a `\` followed by 1–3 octal
/// digits).
///
/// An unknown escape passes through verbatim, backslash retained, so the
/// function is total. `\?` and `\*` are not string escapes even though the
/// grammar's `ESCAPE_SEQ` lists `?`: QUERY master03 §Operators/LIKE requires
/// both to match the literal character in a pattern, which only works if the
/// string layer preserves them for the pattern layer.
fn unescape(inner: &str) -> String {
    if !inner.contains('\\') {
        return inner.to_string();
    }
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(&next) = chars.peek() else {
            out.push('\\');
            break;
        };
        match next {
            '\'' | '"' | '\\' => {
                out.push(next);
                chars.next();
            }
            'a' => push_escaped(&mut out, &mut chars, '\u{07}'),
            'b' => push_escaped(&mut out, &mut chars, '\u{08}'),
            'f' => push_escaped(&mut out, &mut chars, '\u{0C}'),
            'n' => push_escaped(&mut out, &mut chars, '\n'),
            'r' => push_escaped(&mut out, &mut chars, '\r'),
            't' => push_escaped(&mut out, &mut chars, '\t'),
            'v' => push_escaped(&mut out, &mut chars, '\u{0B}'),
            'u' => push_utf8_escape(&mut out, &mut chars),
            '0'..='7' => push_octal_escape(&mut out, &mut chars, next),
            _ => out.push('\\'),
        }
    }
    out
}

/// Consume the escape letter (already peeked) and push its decoded char.
fn push_escaped(out: &mut String, chars: &mut std::iter::Peekable<std::str::Chars<'_>>, ch: char) {
    chars.next();
    out.push(ch);
}

/// Decode a `UTF8CHAR` escape (`\uHHHH`), consuming its four hex digits.
///
/// An escape that is not four hex digits naming a scalar value is passed
/// through verbatim (`\u` retained), keeping [`unescape`] total.
fn push_utf8_escape(out: &mut String, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    chars.next(); // consume 'u'
    // Look ahead (without consuming) at the next four chars.
    let hex: String = chars.clone().take(4).collect();
    if hex.len() == 4
        && let Ok(cp) = u32::from_str_radix(&hex, 16)
        && let Some(ch) = char::from_u32(cp)
    {
        for _ in 0..4 {
            chars.next();
        }
        out.push(ch);
        return;
    }
    out.push('\\');
    out.push('u');
}

/// Decode an `OCTAL_ESC` escape (`\` plus one to three octal digits), whose
/// `first` digit has already been peeked.
///
/// A sequence naming no scalar value is passed through verbatim, keeping
/// [`unescape`] total.
fn push_octal_escape(
    out: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    first: char,
) {
    chars.next(); // consume the first octal digit
    let mut digits = String::new();
    digits.push(first);
    while digits.len() < 3
        && let Some(&d) = chars.peek()
        && ('0'..='7').contains(&d)
    {
        digits.push(d);
        chars.next();
    }
    if let Ok(cp) = u32::from_str_radix(&digits, 8)
        && let Some(ch) = char::from_u32(cp)
    {
        out.push(ch);
        return;
    }
    out.push('\\');
    out.push_str(&digits);
}

/// A lexed numeric literal, tagged by the grammar token it came from.
#[derive(Clone, Copy)]
enum NumKind {
    /// `INTEGER`
    Int,
    /// `REAL` / `SCI_REAL`
    Real,
    /// `SCI_INTEGER`
    SciInt,
}

/// Converts a lexed numeric to a [`Primitive`], returning `None` on overflow so
/// the parser reports a hard error instead of coercing to `0`/`inf`.
///
/// `SCI_INTEGER` stays an integer when the magnitude is integral and fits
/// `i64`, else degrades to `Real`.
fn parse_number(kind: NumKind, s: &str) -> Option<Primitive> {
    match kind {
        NumKind::Int => s.parse::<i64>().ok().map(Primitive::Integer),
        NumKind::Real => {
            let r = s.parse::<f64>().ok()?;
            r.is_finite().then_some(Primitive::Real(r))
        }
        NumKind::SciInt => {
            let r = s.parse::<f64>().ok()?;
            if !r.is_finite() {
                return None;
            }
            #[expect(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                reason = "the guard on this very line proves r is integral and inside the i64 range; the i64::MIN/MAX casts are the bound check itself"
            )]
            if r.fract() == 0.0 && r >= i64::MIN as f64 && r <= i64::MAX as f64 {
                Some(Primitive::Integer(r as i64))
            } else {
                Some(Primitive::Real(r))
            }
        }
    }
}

fn primitive<'a>() -> impl Parser<'a, &'a [Token], Primitive, Err<'a>> + Clone {
    // Overflow is a hard parse error, not a silent `0`/`inf`.
    let unsigned = select! {
        Token::Integer(s) => (NumKind::Int, s),
        Token::Real(s) => (NumKind::Real, s),
        Token::SciInteger(s) => (NumKind::SciInt, s),
        Token::SciReal(s) => (NumKind::Real, s),
    }
    .try_map(|(kind, s), span| parse_number(kind, &s).ok_or_else(|| Simple::new(None, span)));
    // numericPrimitive : … | SYM_MINUS numericPrimitive — the minus recurses,
    // so `- - 5` is accepted. Zero leading minuses folds to the bare
    // unsigned literal.
    let signed = just(Token::Minus)
        .repeated()
        .foldr(unsigned, |_minus, p| match p {
            Primitive::Integer(n) => Primitive::Integer(n.wrapping_neg()),
            Primitive::Real(r) => Primitive::Real(-r),
            other => other,
        });
    let other = select! {
        Token::String(s) => Primitive::String(unquote(&s)),
        Token::True => Primitive::Boolean(true),
        Token::False => Primitive::Boolean(false),
        Token::Null => Primitive::Null,
    };
    signed.or(other)
}

fn parameter<'a>() -> impl Parser<'a, &'a [Token], String, Err<'a>> + Clone {
    select! { Token::Parameter(s) => s }
}

// ── paths & predicates (mutually recursive: predicate ← objectPath ← predicate) ──

/// Returns the `(identified_path, path_predicate, standard_predicate)` parsers.
///
/// They are built together because a `pathPredicate` contains an `objectPath`
/// whose `pathPart`s may carry another `pathPredicate`. The bare
/// `standardPredicate` is handed back for `AqlParser.g4 versionPredicate`.
#[expect(
    clippy::type_complexity,
    reason = "the tuple of three mutually-recursive chumsky parsers is the return type; naming it would need an unnameable opaque-type alias per element"
)]
fn path_parsers<'a>() -> (
    impl Parser<'a, &'a [Token], IdentifiedPath, Err<'a>> + Clone,
    impl Parser<'a, &'a [Token], PathPredicate, Err<'a>> + Clone,
    impl Parser<'a, &'a [Token], StandardPredicate, Err<'a>> + Clone,
) {
    // The only genuine recursion is objectPath → pathPart → pathPredicate →
    // objectPath, through `recursive`'s WEAK self-handle: an owned handle
    // embedded in its own definition is an Rc cycle chumsky never breaks. The
    // predicate family handed back is a second instance over the finished
    // `object`, held outside its definition, so it drops with the query parser.
    let object = recursive(|object| {
        let (predicate, _standard) = predicate_parsers(object);
        // pathPart : IDENTIFIER pathPredicate?
        let path_part = ident()
            .then(predicate.or_not())
            .map(|(name, predicate)| PathPart { name, predicate });
        // objectPath : pathPart ('/' pathPart)*
        path_part
            .separated_by(just(Token::Slash))
            .at_least(1)
            .collect::<Vec<_>>()
            .map(|parts| ObjectPath { parts })
    });
    let (predicate, standard) = predicate_parsers(object.clone());

    // identifiedPath : IDENTIFIER pathPredicate? ('/' objectPath)?
    let identified = ident()
        .then(predicate.clone().or_not())
        .then(just(Token::Slash).ignore_then(object).or_not())
        .map_with(|((root, predicate), path), e| IdentifiedPath {
            root,
            predicate,
            path,
            span: token_span(e.span().into_range()),
        });

    (identified, predicate, standard)
}

/// What followed the shared `objectPath` prefix of the two `objectPath`-leading
/// `nodePredicate` alternatives, so the prefix is parsed once.
enum NodePathTail {
    /// `MATCHES CONTAINED_REGEX` — the raw `{/regex/}` token text.
    Regex(String),
    /// `COMPARISON_OPERATOR pathPredicateOperand`.
    Compare(CompOp, PathPredicateOperand),
}

/// The `pathPredicate` and `standardPredicate` parsers over a given
/// `objectPath` parser — the non-recursive half of the path grammar
/// ([`path_parsers`] wires the recursion).
fn predicate_parsers<'a>(
    object: impl Parser<'a, &'a [Token], ObjectPath, Err<'a>> + Clone + 'a,
) -> (
    impl Parser<'a, &'a [Token], PathPredicate, Err<'a>> + Clone,
    impl Parser<'a, &'a [Token], StandardPredicate, Err<'a>> + Clone,
) {
    // pathPredicateOperand : primitive | objectPath | PARAMETER | ID_CODE | AT_CODE
    let code = select! { Token::IdCode(s) => s, Token::AtCode(s) => s };
    let predicate_operand = primitive()
        .map(PathPredicateOperand::Primitive)
        .or(parameter().map(PathPredicateOperand::Parameter))
        .or(code.map(PathPredicateOperand::Code))
        .or(object.clone().map(PathPredicateOperand::Path));

    // standardPredicate : objectPath COMPARISON_OPERATOR pathPredicateOperand
    let comparison = select! { Token::Comparison(op) => op };
    let standard = object
        .clone()
        .then(comparison)
        .then(predicate_operand.clone())
        .map(|((path, op), operand)| StandardPredicate { path, op, operand });

    // archetypePredicate : ARCHETYPE_HRID | PARAMETER
    let archetype = select! { Token::ArchetypeHrid(s) => ArchetypePredicate::Hrid(s) }
        .or(parameter().map(ArchetypePredicate::Parameter));

    // nodePredicate : node/archetype code (+optional name) | parameter |
    //   objectPath MATCHES CONTAINED_REGEX | standardPredicate |
    //   nodePredicate (AND|OR) nodePredicate. AND binds tighter than OR.
    let name_constraint = select! {
        Token::String(s) => NodeNameConstraint::String(unquote(&s)),
        Token::TermCode(s) => NodeNameConstraint::TermCode(s),
        Token::IdCode(s) => NodeNameConstraint::Code(s),
        Token::AtCode(s) => NodeNameConstraint::Code(s),
    }
    .or(parameter().map(NodeNameConstraint::Parameter));

    let node_code = code
        .then(
            just(Token::Comma)
                .ignore_then(name_constraint.clone())
                .or_not(),
        )
        .map(|(code, name)| NodePredicate::Code { code, name });
    let node_archetype = select! { Token::ArchetypeHrid(s) => s }
        .then(just(Token::Comma).ignore_then(name_constraint).or_not())
        .map(|(hrid, name)| NodePredicate::Archetype { hrid, name });
    // The two `objectPath`-leading `nodePredicate` alternatives differ only
    // after the shared prefix, so the prefix is parsed once and the tail
    // dispatches: retrying each alternative from the top would double the work
    // per bracket nesting level.
    let node_path_tail = just(Token::Matches)
        .ignore_then(select! { Token::ContainedRegex(s) => s })
        .map(NodePathTail::Regex)
        .or(comparison
            .then(predicate_operand)
            .map(|(op, operand)| NodePathTail::Compare(op, operand)));
    let node_path = object
        .clone()
        .then(node_path_tail)
        .map(|(path, tail)| match tail {
            NodePathTail::Regex(regex) => NodePredicate::MatchesRegex { path, regex },
            NodePathTail::Compare(op, operand) => {
                NodePredicate::Standard(Box::new(StandardPredicate { path, op, operand }))
            }
        });
    let node_atom = node_code
        .or(node_archetype)
        .or(parameter().map(NodePredicate::Parameter))
        .or(node_path);
    let node_and = node_atom.clone().foldl(
        just(Token::And).ignore_then(node_atom.clone()).repeated(),
        |l, r| NodePredicate::And(Box::new(l), Box::new(r)),
    );
    let node = node_and
        .clone()
        .foldl(just(Token::Or).ignore_then(node_and).repeated(), |l, r| {
            NodePredicate::Or(Box::new(l), Box::new(r))
        });

    // pathPredicate : '[' (standardPredicate | archetypePredicate |
    // nodePredicate) ']'
    //
    // A lone comparison is both a standardPredicate and a nodePredicate, and
    // the grammar lists `standardPredicate` first, so it is lifted back out of
    // the node tree. A lone `ARCHETYPE_HRID`/`PARAMETER` is an
    // archetypePredicate, yet can open a node boolean (`[$p and at0001]`), so
    // `]` belongs to each alternative and a failed one backtracks.
    let close = just(Token::RightBracket);
    let predicate = just(Token::LeftBracket).ignore_then(
        archetype
            .clone()
            .then_ignore(close.clone())
            .map(PathPredicate::Archetype)
            .or(node
                .map(|n| match n {
                    NodePredicate::Standard(s) => PathPredicate::Standard(s),
                    other => PathPredicate::Node(Box::new(other)),
                })
                .then_ignore(close)),
    );

    (predicate, standard)
}

// ── functions & terminals ───────────────────────────────────────────────────

fn terminology_fn<'a>() -> impl Parser<'a, &'a [Token], TerminologyFunction, Err<'a>> + Clone {
    let string = select! { Token::String(s) => unquote(&s) };
    just(Token::Terminology)
        .ignore_then(
            string
                .then_ignore(just(Token::Comma))
                .then(string)
                .then_ignore(just(Token::Comma))
                .then(string)
                .delimited_by(just(Token::LeftParen), just(Token::RightParen)),
        )
        .map(|((operation, arg2), arg3)| TerminologyFunction {
            operation,
            arg2,
            arg3,
        })
}

/// `functionCall`, given the `terminal` parser (its argument grammar).
fn function_parser<'a>(
    terminal: impl Parser<'a, &'a [Token], Terminal, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a [Token], FunctionCall, Err<'a>> + Clone {
    // functionCall : terminologyFunction | name '(' (terminal (',' terminal)*)? ')'
    // The STRING function `CONTAINS(expr, substring)` shares its name with the
    // containment keyword (QUERY master03 §Functions/String functions) — in
    // function position (followed by `(`) the keyword token is the function
    // name, kept as spelled.
    let named = ident()
        .or(select! { Token::Contains(spelled) => spelled })
        .then(
            terminal
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LeftParen), just(Token::RightParen)),
        )
        .map(|(name, args)| FunctionCall::named(name, args));
    terminology_fn().map(FunctionCall::Terminology).or(named)
}

/// `aggregateFunctionCall`, given the `identified_path` parser.
///
/// Aggregates take a path or `*`, never a `terminal`, which is what lets this
/// builder live outside the `terminal` recursion.
fn aggregate_parser<'a>(
    identified: impl Parser<'a, &'a [Token], IdentifiedPath, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a [Token], AggregateCall, Err<'a>> + Clone {
    // aggregateFunctionCall
    let count = just(Token::Count).ignore_then(
        just(Token::Distinct)
            .or_not()
            .then(identified.clone())
            .map(|(d, p)| AggregateCall::Count {
                distinct: d.is_some(),
                path: Some(p),
            })
            .or(just(Token::Asterisk).map(|_| AggregateCall::Count {
                distinct: false,
                path: None,
            }))
            .delimited_by(just(Token::LeftParen), just(Token::RightParen)),
    );
    let stat_name = select! {
        Token::Min => StatFunc::Min,
        Token::Max => StatFunc::Max,
        Token::Sum => StatFunc::Sum,
        Token::Avg => StatFunc::Avg,
    };
    let stat = stat_name
        .then(
            identified
                .clone()
                .delimited_by(just(Token::LeftParen), just(Token::RightParen)),
        )
        .map(|(func, path)| AggregateCall::Stat { func, path });
    count.or(stat)
}

// ── top-level query ──────────────────────────────────────────────────────────

#[expect(
    clippy::too_many_lines,
    reason = "one combinator builder for the whole AqlParser.g4 grammar; the sub-parsers are mutually referential, so splitting it would only move the wiring"
)]
fn query<'a>() -> impl Parser<'a, &'a [Token], SelectQuery, Err<'a>> {
    // The whole path grammar (identified paths, the path predicate, and the
    // bare standard predicate) is built once here and shared by both the
    // SELECT/terminal side (`identified`) and the FROM side (`predicate` on a
    // class operand, `standard` inside a VERSION predicate).
    let (identified, predicate, standard) = path_parsers();

    // terminal : primitive | PARAMETER | identifiedPath | functionCall
    // functionCall needs terminal, so the self-reference goes through
    // `recursive`'s WEAK handle; an owned handle embedded in its own `define`
    // is an Rc cycle chumsky never breaks.
    let terminal = recursive(|terminal| {
        let function = function_parser(terminal);
        primitive()
            .map(Terminal::Primitive)
            .or(parameter().map(Terminal::Parameter))
            .or(function.map(Terminal::Function))
            .or(identified.clone().map(Terminal::Path))
    });
    // Its own instance: an owned handle to `terminal`, cycle-free because it
    // lives outside terminal's definition.
    let function = function_parser(terminal.clone());
    let aggregate = aggregate_parser(identified.clone());

    // ── SELECT ──
    // columnExpr : identifiedPath | primitive | aggregateFunctionCall | functionCall
    let column = aggregate
        .clone()
        .map(ColumnExpr::Aggregate)
        .or(function.clone().map(ColumnExpr::Function))
        .or(primitive().map(ColumnExpr::Primitive))
        .or(identified.clone().map(ColumnExpr::Path));
    let select_expr = column
        .then(just(Token::As).ignore_then(ident()).or_not())
        .map(|(column, alias)| SelectExpr { column, alias });
    // top (deprecated). Overflow is a parse error, not a silent `0`.
    // `Simple` carries no cause payload, so the span IS the whole diagnostic.
    let top_count = select! { Token::Integer(s) => s }
        .try_map(|s: String, span| s.parse::<i64>().ok().ok_or_else(|| Simple::new(None, span)));
    let top = just(Token::Top)
        .ignore_then(top_count)
        .then(
            select! {
                Token::Forward => TopDirection::Forward,
                Token::Backward => TopDirection::Backward,
            }
            .or_not(),
        )
        .map(|(count, direction)| Top { count, direction });
    let select_clause = just(Token::Select)
        .ignore_then(just(Token::Distinct).or_not())
        .then(top.or_not())
        .then(
            select_expr
                .separated_by(just(Token::Comma))
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .map(|((distinct, top), columns)| SelectClause {
            distinct: distinct.is_some(),
            top,
            columns,
        });

    // ── FROM / containsExpr ──
    // classExprOperand : IDENTIFIER variable? pathPredicate? | VERSION variable? [versionPredicate]?
    // versionPredicate : LATEST_VERSION | ALL_VERSIONS | standardPredicate
    // The third (standardPredicate) alternative is wired here — so
    // `VERSION v[commit_audit/time_committed > '2020-01-01']` parses.
    let version_predicate = select! {
        Token::LatestVersion => VersionPredicate::Latest,
        Token::AllVersions => VersionPredicate::All,
    }
    .or(standard.map(|s| VersionPredicate::Standard(Box::new(s))));
    let class_operand = just(Token::Version)
        .ignore_then(ident().or_not())
        .then(
            version_predicate
                .delimited_by(just(Token::LeftBracket), just(Token::RightBracket))
                .or_not(),
        )
        .map(|(variable, predicate)| ClassExprOperand::Version {
            variable,
            predicate,
        })
        .or(ident().then(ident().or_not()).then(predicate.or_not()).map(
            |((rm_type, variable), predicate)| ClassExprOperand::Class {
                rm_type,
                variable,
                predicate,
            },
        ));

    let contains = recursive(|contains| {
        let atom = class_operand
            .then(
                just(Token::Not)
                    .or_not()
                    .then_ignore(select! { Token::Contains(_) => () })
                    .then(contains.clone())
                    .map(|(neg, expr)| {
                        Box::new(ContainsConstraint {
                            negated: neg.is_some(),
                            expr,
                        })
                    })
                    .or_not(),
            )
            .map(|(operand, contains)| ContainsExpr::Contained { operand, contains })
            .or(contains
                .clone()
                .delimited_by(just(Token::LeftParen), just(Token::RightParen)));
        let and = atom.clone().foldl(
            just(Token::And).ignore_then(atom.clone()).repeated(),
            |l, r| ContainsExpr::And(Box::new(l), Box::new(r)),
        );
        and.clone()
            .foldl(just(Token::Or).ignore_then(and).repeated(), |l, r| {
                ContainsExpr::Or(Box::new(l), Box::new(r))
            })
    });

    // ── WHERE / whereExpr ──
    let like_operand = select! { Token::String(s) => LikeOperand::String(unquote(&s)) }
        .or(parameter().map(LikeOperand::Parameter));
    let value_item = primitive()
        .map(ValueListItem::Primitive)
        .or(parameter().map(ValueListItem::Parameter))
        .or(terminology_fn().map(ValueListItem::Terminology));
    let matches_operand = terminology_fn()
        .map(MatchesOperand::Terminology)
        .or(select! { Token::Uri(u) => MatchesOperand::Uri(u) }
            .delimited_by(just(Token::LeftCurly), just(Token::RightCurly)))
        .or(value_item
            .separated_by(just(Token::Comma))
            .at_least(1)
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LeftCurly), just(Token::RightCurly))
            .map(MatchesOperand::ValueList));

    let comparison = select! { Token::Comparison(op) => op };
    let identified_expr = just(Token::Exists)
        .ignore_then(identified.clone())
        .map(IdentifiedExpr::Exists)
        .or(function
            .clone()
            .then(comparison)
            .then(terminal.clone())
            .map(|((f, op), rhs)| IdentifiedExpr::Compare {
                lhs: CompareOperand::Function(f),
                op,
                rhs,
            }))
        .or(identified
            .clone()
            .then(just(Token::Like).ignore_then(like_operand))
            .map(|(path, operand)| IdentifiedExpr::Like { path, operand }))
        .or(identified
            .clone()
            .then(just(Token::Matches).ignore_then(matches_operand))
            .map(|(path, operand)| IdentifiedExpr::Matches { path, operand }))
        .or(identified
            .clone()
            .then(comparison)
            .then(terminal.clone())
            .map(|((path, op), rhs)| IdentifiedExpr::Compare {
                lhs: CompareOperand::Path(path),
                op,
                rhs,
            }));

    let where_expr = recursive(|where_expr| {
        let atom = identified_expr
            .map_with(|expr, e| WhereExpr::Identified(expr, token_span(e.span().into_range())))
            .or(where_expr
                .clone()
                .delimited_by(just(Token::LeftParen), just(Token::RightParen)));
        // Precedence: NOT (unary, tightest) > AND > OR. `NOT a AND b` parses as
        // `(NOT a) AND b`; group with parens for `NOT (a AND b)`.
        let unary = just(Token::Not)
            .repeated()
            .foldr(atom, |_not, e| WhereExpr::Not(Box::new(e)));
        let and = unary
            .clone()
            .foldl(just(Token::And).ignore_then(unary).repeated(), |l, r| {
                WhereExpr::And(Box::new(l), Box::new(r))
            });
        and.clone()
            .foldl(just(Token::Or).ignore_then(and).repeated(), |l, r| {
                WhereExpr::Or(Box::new(l), Box::new(r))
            })
    });

    // ── ORDER BY / LIMIT ──
    let order_by_expr = identified
        .clone()
        .then(
            select! {
                Token::Descending => SortOrder::Descending,
                Token::Desc => SortOrder::Descending,
                Token::Ascending => SortOrder::Ascending,
                Token::Asc => SortOrder::Ascending,
            }
            .or_not(),
        )
        .map(|(path, order)| OrderByExpr { path, order });
    // Overflow is a parse error, not a silent `0`; `Simple` carries no cause
    // payload, so the span is the whole diagnostic.
    let int = select! { Token::Integer(s) => s }
        .try_map(|s: String, span| s.parse::<i64>().ok().ok_or_else(|| Simple::new(None, span)));
    let limit = just(Token::Limit)
        .ignore_then(int)
        .then(just(Token::Offset).ignore_then(int).or_not())
        .map(|(limit, offset)| Limit { limit, offset });

    // selectQuery : selectClause fromClause whereClause? orderByClause?
    //               limitClause? '--'? EOF
    //
    // NOTE: no TIMEWINDOW clause — AQL 1.1 removed it (SPECQUERY-20); a query
    // using it is invalid and fails to parse, which is the conformant outcome.
    select_clause
        .then(just(Token::From).ignore_then(contains))
        .then(just(Token::Where).ignore_then(where_expr).or_not())
        .then(
            just(Token::Order)
                .ignore_then(just(Token::By))
                .ignore_then(
                    order_by_expr
                        .separated_by(just(Token::Comma))
                        .at_least(1)
                        .collect::<Vec<_>>(),
                )
                .or_not(),
        )
        .then(limit.or_not())
        .then_ignore(just(Token::DoubleDash).or_not())
        .map(
            |((((select, from), where_), order_by), limit)| SelectQuery {
                select,
                from,
                where_,
                order_by: order_by.unwrap_or_default(),
                limit,
            },
        )
        .then_ignore(end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_select_from() {
        let q = parse_str("SELECT c FROM COMPOSITION c").expect("parse");
        assert_eq!(q.select.columns.len(), 1);
        assert!(matches!(
            &q.from,
            ContainsExpr::Contained {
                operand: ClassExprOperand::Class { rm_type, variable, .. },
                ..
            } if rm_type == "COMPOSITION" && variable.as_deref() == Some("c")
        ));
    }

    /// QUERY master03 §Operators/LIKE: `\?` and `\*` in a pattern match the
    /// literal character, so string decoding preserves both escapes for the
    /// pattern layer.
    #[test]
    fn like_pattern_escapes_survive_string_decoding() {
        for (src, expected) in [
            (r"'2026-01-01T0\?:00:00Z'", r"2026-01-01T0\?:00:00Z"),
            (r"'2026-01-01T\*'", r"2026-01-01T\*"),
        ] {
            let q = parse_str(&format!(
                "SELECT c FROM COMPOSITION c WHERE c/name/value LIKE {src}"
            ))
            .expect("parse");
            let Some(WhereExpr::Identified(
                IdentifiedExpr::Like {
                    operand: LikeOperand::String(pattern),
                    ..
                },
                _,
            )) = q.where_
            else {
                panic!("not a LIKE: {:?}", q.where_);
            };
            assert_eq!(pattern, expected);
        }
    }

    #[test]
    fn select_path_where_compare() {
        let q = parse_str("SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_id/value = $id")
            .expect("parse");
        assert!(matches!(q.select.columns[0].column, ColumnExpr::Path(_)));
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(IdentifiedExpr::Compare { .. }, _))
        ));
    }

    #[test]
    fn contains_with_archetype_predicate() {
        let q = parse_str(
            "SELECT o FROM EHR e CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v1]",
        )
        .expect("parse");
        match &q.from {
            ContainsExpr::Contained {
                contains: Some(c), ..
            } => {
                assert!(!c.negated);
                assert!(matches!(c.expr, ContainsExpr::Contained { .. }));
            }
            other => panic!("expected contains, got {other:?}"),
        }
    }

    #[test]
    fn distinct_alias_orderby_limit() {
        let q = parse_str(
            "SELECT DISTINCT c/name/value AS n FROM COMPOSITION c ORDER BY c/name/value DESC LIMIT 10 OFFSET 5",
        )
        .expect("parse");
        assert!(q.select.distinct);
        assert_eq!(q.select.columns[0].alias.as_deref(), Some("n"));
        assert_eq!(q.order_by.len(), 1);
        assert_eq!(q.order_by[0].order, Some(SortOrder::Descending));
        assert_eq!(
            q.limit,
            Some(Limit {
                limit: 10,
                offset: Some(5)
            })
        );
    }

    #[test]
    fn timewindow_is_rejected() {
        // AQL 1.1 removed the `TIMEWINDOW` clause from the grammar (QUERY
        // `master00-amendment_record.adoc`, SPECQUERY-20) — a query using it is
        // invalid AQL and must fail to parse. The CNF query corpus (A/109,
        // B/103, C/103) predates the removal; the conformance runner encodes
        // that as a documented corpus-override, never the parser.
        for q in [
            "SELECT e/ehr_id/value FROM EHR e TIMEWINDOW PT12H/2019-10-24",
            "SELECT c FROM COMPOSITION c TIMEWINDOW PT12H/2019-10-24",
            "SELECT c FROM COMPOSITION c TIMEWINDOW PT12H",
        ] {
            assert!(parse_str(q).is_err(), "TIMEWINDOW must not parse: {q}");
        }
    }

    #[test]
    fn aggregate_count() {
        let q = parse_str("SELECT COUNT(*) FROM COMPOSITION c").expect("parse");
        assert!(matches!(
            q.select.columns[0].column,
            ColumnExpr::Aggregate(AggregateCall::Count { path: None, .. })
        ));
    }

    #[test]
    fn where_boolean_precedence() {
        // a AND b OR c  ⇒  Or(And(a,b), c)
        let q = parse_str("SELECT c FROM COMPOSITION c WHERE c/x = 1 AND c/y = 2 OR c/z = 3")
            .expect("parse");
        assert!(matches!(q.where_, Some(WhereExpr::Or(_, _))));
    }

    #[test]
    fn not_binds_tighter_than_and() {
        // NOT a AND b  ⇒  And(Not(a), b)  — not Not(And(a,b))
        let q = parse_str("SELECT c FROM COMPOSITION c WHERE NOT EXISTS c/x AND EXISTS c/y")
            .expect("parse");
        match q.where_ {
            Some(WhereExpr::And(l, _)) => assert!(matches!(*l, WhereExpr::Not(_))),
            other => panic!("expected And(Not(..), ..), got {other:?}"),
        }
    }

    #[test]
    fn node_predicate_boolean_tree() {
        let q = parse_str("SELECT o FROM OBSERVATION o[at0001 AND value/magnitude > 5]")
            .expect("parse");
        match &q.from {
            ContainsExpr::Contained {
                operand:
                    ClassExprOperand::Class {
                        predicate: Some(PathPredicate::Node(n)),
                        ..
                    },
                ..
            } => assert!(matches!(**n, NodePredicate::And(_, _))),
            other => panic!("expected node AND predicate, got {other:?}"),
        }
    }

    #[test]
    fn contained_regex_predicate() {
        let q = parse_str("SELECT o FROM OBSERVATION o[name/value MATCHES {/blood.*/}]")
            .expect("parse");
        assert!(matches!(
            &q.from,
            ContainsExpr::Contained {
                operand: ClassExprOperand::Class {
                    predicate: Some(PathPredicate::Node(_)),
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn trailing_tokens_are_rejected() {
        // EOF is enforced: junk after a complete query must fail.
        assert!(parse_str("SELECT c FROM COMPOSITION c EXTRA").is_err());
    }

    // ── VERSION standard-predicate form ───────────────────────────────────
    #[test]
    fn version_latest_and_all_predicates() {
        for (src, want) in [
            (
                "SELECT c FROM VERSION v[LATEST_VERSION]",
                VersionPredicate::Latest,
            ),
            (
                "SELECT c FROM VERSION v[ALL_VERSIONS]",
                VersionPredicate::All,
            ),
        ] {
            let q = parse_str(src).expect("parse");
            match q.from {
                ContainsExpr::Contained {
                    operand:
                        ClassExprOperand::Version {
                            predicate: Some(p), ..
                        },
                    ..
                } => assert_eq!(p, want),
                other => panic!("expected version operand, got {other:?}"),
            }
        }
    }

    #[test]
    fn version_standard_predicate_parses() {
        // the third versionPredicate alternative (standardPredicate).
        let q = parse_str("SELECT c FROM VERSION v[commit_audit/time_committed > '2020-01-01']")
            .expect("parse");
        match q.from {
            ContainsExpr::Contained {
                operand:
                    ClassExprOperand::Version {
                        variable,
                        predicate: Some(VersionPredicate::Standard(s)),
                    },
                ..
            } => {
                assert_eq!(variable.as_deref(), Some("v"));
                assert_eq!(s.op, CompOp::Gt);
            }
            other => panic!("expected VERSION standard predicate, got {other:?}"),
        }
    }

    #[test]
    fn version_standard_predicate_with_parameter() {
        let q = parse_str("SELECT c FROM VERSION v[uid/value = $vid]").expect("parse");
        assert!(matches!(
            q.from,
            ContainsExpr::Contained {
                operand: ClassExprOperand::Version {
                    predicate: Some(VersionPredicate::Standard(_)),
                    ..
                },
                ..
            }
        ));
    }

    // ── numeric overflow is a parse error ─────────────────────────────────
    #[test]
    fn integer_overflow_is_a_parse_error() {
        // Was silently coerced to `Integer(0)`; must now fail.
        assert!(
            parse_str("SELECT c FROM COMPOSITION c WHERE c/x = 99999999999999999999999").is_err()
        );
    }

    #[test]
    fn in_range_integer_still_parses() {
        let q = parse_str("SELECT c FROM COMPOSITION c WHERE c/x = 9223372036854775807")
            .expect("parse i64::MAX");
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(
                IdentifiedExpr::Compare {
                    rhs: Terminal::Primitive(Primitive::Integer(9_223_372_036_854_775_807)),
                    ..
                },
                _
            ))
        ));
    }

    #[test]
    fn limit_and_top_overflow_are_parse_errors() {
        assert!(parse_str("SELECT c FROM COMPOSITION c LIMIT 99999999999999999999999").is_err());
        assert!(parse_str("SELECT TOP 99999999999999999999999 c FROM COMPOSITION c").is_err());
    }

    #[test]
    fn real_overflow_is_a_parse_error() {
        // Was silently `inf`; must now fail.
        assert!(parse_str("SELECT c FROM COMPOSITION c WHERE c/x = 1.0e999").is_err());
    }

    // ── recursive unary minus ─────────────────────────────────────────────
    #[test]
    fn double_unary_minus_parses() {
        let q = parse_str("SELECT c FROM COMPOSITION c WHERE c/x = - - 5").expect("parse");
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(
                IdentifiedExpr::Compare {
                    rhs: Terminal::Primitive(Primitive::Integer(5)),
                    ..
                },
                _
            ))
        ));
    }

    #[test]
    fn single_negative_numeric_parses() {
        let q = parse_str("SELECT c FROM COMPOSITION c WHERE c/x = -5").expect("parse");
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(
                IdentifiedExpr::Compare {
                    rhs: Terminal::Primitive(Primitive::Integer(-5)),
                    ..
                },
                _
            ))
        ));
    }

    // ── string unescaping ─────────────────────────────────────────────────
    #[test]
    fn string_escapes_are_decoded() {
        let q = parse_str(r"SELECT c FROM COMPOSITION c WHERE c/x = 'a\nb\t\\c'").expect("parse");
        match q.where_ {
            Some(WhereExpr::Identified(
                IdentifiedExpr::Compare {
                    rhs: Terminal::Primitive(Primitive::String(s)),
                    ..
                },
                _,
            )) => assert_eq!(s, "a\nb\t\\c"),
            other => panic!("expected decoded string, got {other:?}"),
        }
    }

    #[test]
    fn string_unicode_and_quote_escapes_are_decoded() {
        assert_eq!(unquote(r"'A\''"), "A'");
        assert_eq!(unquote(r#""a\"b""#), "a\"b");
        // UTF8CHAR `\uHHHH`: `A` -> 'A', `é` -> 'é'.
        assert_eq!(unquote(r"'\u0041\u00e9'"), "Aé");
        // OCTAL_ESC: octal 101 = 'A'.
        assert_eq!(unquote(r"'\101'"), "A");
        // Unknown escape passes through verbatim (total on any input).
        assert_eq!(unquote(r"'x\z'"), r"x\z");
        // A stray `\u` with too few hex digits is left verbatim.
        assert_eq!(unquote(r"'\u12'"), r"\u12");
    }

    // ── three-way predicate classification ────────────────────────────────
    #[test]
    fn bare_standard_predicate_classifies_as_standard() {
        // A lone comparison in a predicate is PathPredicate::Standard, not
        // PathPredicate::Node(NodePredicate::Standard).
        let q = parse_str("SELECT e FROM EHR e[ehr_id/value = '123']").expect("parse");
        match q.from {
            ContainsExpr::Contained {
                operand:
                    ClassExprOperand::Class {
                        predicate: Some(PathPredicate::Standard(s)),
                        ..
                    },
                ..
            } => assert_eq!(s.op, CompOp::Eq),
            other => panic!("expected PathPredicate::Standard, got {other:?}"),
        }
    }

    #[test]
    fn boolean_predicate_stays_node() {
        // A comparison inside an AND tree stays a nodePredicate.
        let q = parse_str("SELECT o FROM OBSERVATION o[at0001 AND value/magnitude > 5]")
            .expect("parse");
        assert!(matches!(
            q.from,
            ContainsExpr::Contained {
                operand: ClassExprOperand::Class {
                    predicate: Some(PathPredicate::Node(_)),
                    ..
                },
                ..
            }
        ));
    }

    // ── SCI_INTEGER retains integer-ness ──────────────────────────────────
    #[test]
    fn scientific_integer_stays_integer() {
        let q = parse_str("SELECT c FROM COMPOSITION c WHERE c/x = 1e3").expect("parse");
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(
                IdentifiedExpr::Compare {
                    rhs: Terminal::Primitive(Primitive::Integer(1000)),
                    ..
                },
                _
            ))
        ));
    }

    #[test]
    fn scientific_real_is_real() {
        let q = parse_str("SELECT c FROM COMPOSITION c WHERE c/x = 1.5e2").expect("parse");
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(
                IdentifiedExpr::Compare {
                    rhs: Terminal::Primitive(Primitive::Real(_)),
                    ..
                },
                _
            ))
        ));
    }

    #[test]
    fn like_operand_parses() {
        let q = parse_str("SELECT c FROM COMPOSITION c WHERE c/name/value LIKE 'blood%'")
            .expect("parse");
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(IdentifiedExpr::Like { .. }, _))
        ));
        let q =
            parse_str("SELECT c FROM COMPOSITION c WHERE c/name/value LIKE $pat").expect("parse");
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(
                IdentifiedExpr::Like {
                    operand: LikeOperand::Parameter(_),
                    ..
                },
                _
            ))
        ));
    }

    #[test]
    fn matches_uri_and_terminology_operands() {
        let q = parse_str(
            "SELECT o FROM OBSERVATION o WHERE o/value MATCHES {http://openehr.org/vs/x}",
        )
        .expect("parse");
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(
                IdentifiedExpr::Matches {
                    operand: MatchesOperand::Uri(_),
                    ..
                },
                _
            ))
        ));
        let q = parse_str(
            "SELECT o FROM OBSERVATION o WHERE o/value/defining_code \
             MATCHES terminology('expand', 'snomed', 'x')",
        )
        .expect("parse");
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(
                IdentifiedExpr::Matches {
                    operand: MatchesOperand::Terminology(_),
                    ..
                },
                _
            ))
        ));
    }

    #[test]
    fn aggregate_stats_parse() {
        for (src, want) in [
            (
                "SELECT MIN(o/value/magnitude) FROM OBSERVATION o",
                StatFunc::Min,
            ),
            (
                "SELECT MAX(o/value/magnitude) FROM OBSERVATION o",
                StatFunc::Max,
            ),
            (
                "SELECT SUM(o/value/magnitude) FROM OBSERVATION o",
                StatFunc::Sum,
            ),
            (
                "SELECT AVG(o/value/magnitude) FROM OBSERVATION o",
                StatFunc::Avg,
            ),
        ] {
            let q = parse_str(src).expect("parse");
            match &q.select.columns[0].column {
                ColumnExpr::Aggregate(AggregateCall::Stat { func, .. }) => assert_eq!(*func, want),
                other => panic!("expected stat aggregate, got {other:?}"),
            }
        }
    }

    #[test]
    fn top_forward_backward_parses() {
        let q = parse_str("SELECT TOP 5 BACKWARD c FROM COMPOSITION c").expect("parse");
        let top = q.select.top.expect("top");
        assert_eq!(top.count, 5);
        assert_eq!(top.direction, Some(TopDirection::Backward));
    }

    #[test]
    fn parameter_predicates_parse() {
        // archetypePredicate PARAMETER, and node predicate `[at0002, $name]`.
        assert!(parse_str("SELECT o FROM OBSERVATION o[$archetypeId]").is_ok());
        let q = parse_str("SELECT o FROM OBSERVATION o[at0002, $name]").expect("parse");
        assert!(matches!(
            q.from,
            ContainsExpr::Contained {
                operand: ClassExprOperand::Class {
                    predicate: Some(PathPredicate::Node(_)),
                    ..
                },
                ..
            }
        ));
    }

    /// The class predicate of a single-operand `FROM`.
    fn class_predicate(src: &str) -> Option<PathPredicate> {
        match parse_str(src).expect("parse").from {
            ContainsExpr::Contained {
                operand: ClassExprOperand::Class { predicate, .. },
                ..
            } => predicate,
            _ => None,
        }
    }

    #[test]
    fn a_node_boolean_may_open_with_a_parameter_or_an_archetype_id() {
        // AqlParser.g4 nodePredicate: PARAMETER | ARCHETYPE_HRID … | nodePredicate (AND|OR) nodePredicate.
        for src in [
            "SELECT k FROM CLUSTER k[$node and name/value='x']",
            "SELECT k FROM CLUSTER k[openEHR-EHR-CLUSTER.device.v1 or at0001]",
        ] {
            assert!(
                matches!(class_predicate(src), Some(PathPredicate::Node(_))),
                "a node boolean: {src}"
            );
        }
        // A lone parameter or HRID stays the archetypePredicate the grammar lists first.
        for src in [
            "SELECT k FROM CLUSTER k[$node]",
            "SELECT k FROM CLUSTER k[openEHR-EHR-CLUSTER.device.v1]",
        ] {
            assert!(
                matches!(class_predicate(src), Some(PathPredicate::Archetype(_))),
                "an archetype predicate: {src}"
            );
        }
    }

    #[test]
    fn hyphenated_term_code_in_predicate_parses() {
        // end-to-end: the node-name term-code slot accepts a hyphenated id.
        let q =
            parse_str("SELECT o FROM OBSERVATION o[at0001, SNOMED-CT::1234|x|]").expect("parse");
        assert!(matches!(
            q.from,
            ContainsExpr::Contained {
                operand: ClassExprOperand::Class {
                    predicate: Some(PathPredicate::Node(_)),
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn line_comment_in_query_parses() {
        // line-comment handling, end-to-end.
        let q = parse_str("SELECT c -- pick the composition\nFROM COMPOSITION c").expect("parse");
        assert_eq!(q.select.columns.len(), 1);
    }

    #[test]
    fn nested_contains_and_matches_valueset() {
        let q = parse_str(
            "SELECT a/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
             WHERE o/value/defining_code MATCHES {'at0001', 'at0002'}",
        )
        .expect("parse");
        // EHR CONTAINS (COMPOSITION CONTAINS OBSERVATION)
        assert!(matches!(
            &q.from,
            ContainsExpr::Contained {
                contains: Some(_),
                ..
            }
        ));
        assert!(matches!(
            q.where_,
            Some(WhereExpr::Identified(IdentifiedExpr::Matches { .. }, _))
        ));
    }
}
