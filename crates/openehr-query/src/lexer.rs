// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL lexer — a `logos` tokenizer transcribed from the authoritative
//! `AqlLexer.g4` (vendored at `vendor/grammar/`).
//!
//! Two deliberate departures from a literal transcription:
//!
//! - The grammar's function-id groups (`STRING_FUNCTION_ID`,
//!   `NUMERIC_FUNCTION_ID`, `DATE_TIME_FUNCTION_ID` — `length`, `abs`, `now`,
//!   …) are not reserved: they lex as [`Token::Identifier`] and the parser
//!   classifies a `name(args)` call (`ast::BuiltinFunction`), which
//!   `AqlParser.g4 functionCall` also admits for a bare `IDENTIFIER`. The accepted set is a superset of the
//!   grammar, never a subset. Aggregates and `terminology(...)` keep dedicated
//!   tokens because their argument grammar differs.
//! - Quoted temporal literals (`DATE`/`TIME`/`DATETIME`) lex as
//!   [`Token::String`]. QUERY §Dates and Times resolves the typing of a quoted
//!   value from the identified-path context, not from the literal, so an
//!   untyped string is the faithful carrier at this layer.
//!
//! AQL keywords are case-insensitive, so each keyword uses `ignore(case)`.

use logos::Logos;

/// A comparison operator (`COMPARISON_OPERATOR` in the grammar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompOp {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `<`
    Lt,
    /// `<=`
    Le,
}

/// An AQL token; the text-carrying variants hold an owned `String`.
#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n\f]+")] // WS -> skip (AqlLexer.g4 `WS`)
#[logos(skip "\u{feff}")] // UNICODE_BOM -> skip (AqlLexer.g4 `UNICODE_BOM`)
pub enum Token {
    // ── structural keywords (case-insensitive) ──────────────────────────────
    /// The `select` keyword token (`SELECT` in the grammar; case-insensitive).
    #[token("select", ignore(case))]
    Select,
    /// The `as` keyword token, binding a SELECT column alias.
    #[token("as", ignore(case))]
    As,
    /// The `from` keyword token, opening the FROM clause.
    #[token("from", ignore(case))]
    From,
    /// The `where` keyword token, opening the WHERE clause.
    #[token("where", ignore(case))]
    Where,
    /// The `order` keyword token of `ORDER BY`.
    #[token("order", ignore(case))]
    Order,
    /// The `by` keyword token of `ORDER BY`.
    #[token("by", ignore(case))]
    By,
    /// The `desc` keyword token — descending sort direction.
    #[token("desc", ignore(case))]
    Desc,
    /// The `descending` keyword token — the long spelling of `desc`.
    #[token("descending", ignore(case))]
    Descending,
    /// The `asc` keyword token — ascending sort direction.
    #[token("asc", ignore(case))]
    Asc,
    /// The `ascending` keyword token — the long spelling of `asc`.
    #[token("ascending", ignore(case))]
    Ascending,
    /// The `limit` keyword token, opening the LIMIT clause.
    #[token("limit", ignore(case))]
    Limit,
    /// The `offset` keyword token of the LIMIT clause.
    #[token("offset", ignore(case))]
    Offset,
    /// The `distinct` keyword token of a SELECT clause.
    #[token("distinct", ignore(case))]
    Distinct,
    /// The `version` keyword token — the VERSION class in a FROM clause.
    #[token("version", ignore(case))]
    Version,
    /// The `latest_version` keyword token — the latest-version predicate.
    #[token("latest_version", ignore(case))]
    LatestVersion,
    /// The `all_versions` keyword token — the all-versions predicate.
    #[token("all_versions", ignore(case))]
    AllVersions,
    /// The `top` keyword token of a SELECT clause.
    #[token("top", ignore(case))]
    Top,
    /// The `forward` keyword token — a `TOP` direction.
    #[token("forward", ignore(case))]
    Forward,
    /// The `backward` keyword token — a `TOP` direction.
    #[token("backward", ignore(case))]
    Backward,
    // NOTE: `TIMEWINDOW` is deliberately NOT a token — AQL 1.1 removed the clause
    // from the grammar (QUERY `master00-amendment_record.adoc`, SPECQUERY-20), so a
    // query using it is invalid AQL 1.1 and must fail to parse.
    /// The `contains` keyword token, joining a containment chain, as spelled:
    /// in function position it names the string function `CONTAINS`, whose
    /// spelling the AST keeps.
    #[token("contains", |lex| lex.slice().to_owned(), ignore(case))]
    Contains(String),
    /// The `and` keyword token — boolean conjunction.
    #[token("and", ignore(case))]
    And,
    /// The `or` keyword token — boolean disjunction.
    #[token("or", ignore(case))]
    Or,
    /// The `not` keyword token — boolean negation.
    #[token("not", ignore(case))]
    Not,
    /// The `exists` keyword token — the existence operator.
    #[token("exists", ignore(case))]
    Exists,
    /// The `like` keyword token — the pattern-match operator.
    #[token("like", ignore(case))]
    Like,
    /// The `matches` keyword token — the value-set match operator.
    #[token("matches", ignore(case))]
    Matches,

    // aggregate + terminology (distinct argument grammar → dedicated tokens)
    /// The `count` aggregate-function keyword token.
    #[token("count", ignore(case))]
    Count,
    /// The `min` aggregate-function keyword token.
    #[token("min", ignore(case))]
    Min,
    /// The `max` aggregate-function keyword token.
    #[token("max", ignore(case))]
    Max,
    /// The `sum` aggregate-function keyword token.
    #[token("sum", ignore(case))]
    Sum,
    /// The `avg` aggregate-function keyword token.
    #[token("avg", ignore(case))]
    Avg,
    /// The `terminology` function keyword token.
    #[token("terminology", ignore(case))]
    Terminology,

    // literal keywords
    /// The `true` boolean-literal keyword token.
    #[token("true", ignore(case))]
    True,
    /// The `false` boolean-literal keyword token.
    #[token("false", ignore(case))]
    False,
    /// The `null` literal keyword token.
    #[token("null", ignore(case))]
    Null,

    // ── operators & symbols ─────────────────────────────────────────────────
    /// A comparison operator: `=`, `!=`, `>=`, `<=`, `>` or `<`.
    #[token("=", |_| CompOp::Eq)]
    #[token("!=", |_| CompOp::Ne)]
    #[token(">=", |_| CompOp::Ge)]
    #[token("<=", |_| CompOp::Le)]
    #[token(">", |_| CompOp::Gt)]
    #[token("<", |_| CompOp::Lt)]
    Comparison(CompOp),

    /// The `(` symbol token.
    #[token("(")]
    LeftParen,
    /// The `)` symbol token.
    #[token(")")]
    RightParen,
    /// The `[` symbol token, opening a node predicate.
    #[token("[")]
    LeftBracket,
    /// The `]` symbol token, closing a node predicate.
    #[token("]")]
    RightBracket,
    /// The `{` symbol token.
    #[token("{")]
    LeftCurly,
    /// The `}` symbol token.
    #[token("}")]
    RightCurly,
    /// The `,` symbol token.
    #[token(",")]
    Comma,
    /// The `/` symbol token — a path separator.
    #[token("/")]
    Slash,
    /// The `*` symbol token — the wildcard path/column.
    #[token("*")]
    Asterisk,
    /// The `;` symbol token.
    #[token(";")]
    Semicolon,
    /// The `-` symbol token.
    #[token("-")]
    Minus,
    /// `--` — the grammar's `SYM_DOUBLE_DASH` optional statement terminator.
    ///
    /// `AqlLexer.g4 COMMENT` claims `--` followed by a space or an
    /// end-of-line/EOF, which the callback skips; only a `--` glued to a
    /// non-space character is emitted as this token.
    #[token("--", line_comment)]
    DoubleDash,

    // ── literals & names ────────────────────────────────────────────────────
    /// A `$name` query parameter, slice included (the grammar's `PARAMETER`).
    #[regex(r"\$[a-zA-Z][a-zA-Z0-9_]*", |lex| lex.slice().to_owned())]
    Parameter(String),

    // The grammar's `CODE_STR` permits leading-zero runs (`at0001`), so a plain
    // `[0-9]+(.[0-9]+)*` after the `id`/`at` prefix is the faithful shape.
    /// An `idNN[.NN]*` archetype node code, slice included.
    #[regex(r"id[0-9]+(\.[0-9]+)*", |lex| lex.slice().to_owned())]
    IdCode(String),
    /// An `atNN[.NN]*` archetype node code, slice included.
    #[regex(r"at[0-9]+(\.[0-9]+)*", |lex| lex.slice().to_owned())]
    AtCode(String),

    // Detected by the `-x-x.…vN` shape. The version tail admits the grammar's
    // `VERSION_ID` `-rc`/`-alpha` suffix and the namespace prefix admits `-`
    // per `NAMESPACE`/`LABEL` (`NAME_CHAR` includes `-`).
    #[regex(
        r"([a-zA-Z][a-zA-Z0-9_.\-]*::)?[a-zA-Z][a-zA-Z0-9_]*-[a-zA-Z][a-zA-Z0-9_]*-[a-zA-Z][a-zA-Z0-9_]*\.[a-zA-Z][a-zA-Z0-9_-]*\.v[0-9]+(\.[0-9]+)*((-rc|-alpha)(\.[0-9]+)?)?",
        |lex| lex.slice().to_owned()
    )]
    /// An archetype HRID (optionally namespaced), slice included — e.g.
    /// `openEHR-EHR-OBSERVATION.blood_pressure.v1`.
    ArchetypeHrid(String),

    // `AqlLexer.g4 TERM_CODE`: every segment is `TERM_CODE_CHAR+`, where
    // `TERM_CODE_CHAR = NAME_CHAR | '.'` and `NAME_CHAR = WORD_CHAR | '-'`, so
    // hyphens are legal on both sides of the required `::`.
    #[regex(
        r"[a-zA-Z0-9._\-]+(\([a-zA-Z0-9._\-]+\))?::[a-zA-Z0-9._\-]+(\|[^|\[\]]+\|)?",
        |lex| lex.slice().to_owned()
    )]
    /// A term code, slice included — e.g. `local::at0001` or
    /// `SNOMED-CT::1234|cyanosis|` (the grammar's `TERM_CODE`).
    TermCode(String),

    // A URI, e.g. `http://example.org/x`. Recognised by a `scheme://` lead.
    #[regex(r"[a-zA-Z][a-zA-Z0-9+.\-]*://[^ \t\r\n{}]*", |lex| lex.slice().to_owned())]
    /// A URI, slice included — recognised by its `scheme://` lead.
    Uri(String),

    // One token, so the inner `/` and `{}` are not mistaken for other symbols.
    #[regex(
        r"\{[ \t\r\n]*/(\\.|[^/\r\n])*/[ \t\r\n]*(;[ \t\r\n]*'([^'\\]|\\.)*')?[ \t\r\n]*\}",
        |lex| lex.slice().to_owned()
    )]
    /// A contained regex, slice included — `{/pattern/}` or
    /// `{/pattern/; 'name'}` (the grammar's `CONTAINED_REGEX`).
    ContainedRegex(String),

    // Scientific and plain numerics (order: sci before plain via length).
    /// An integer in scientific notation, lexeme included — e.g. `1e10`.
    #[regex(r"[0-9]+[eE][+\-]?[0-9]+", |lex| lex.slice().to_owned())]
    SciInteger(String),
    /// A real in scientific notation, lexeme included — e.g. `1.5e-3`.
    #[regex(r"[0-9]*\.[0-9]+[eE][+\-]?[0-9]+", |lex| lex.slice().to_owned())]
    SciReal(String),
    /// A plain real literal, lexeme included — e.g. `3.14`.
    #[regex(r"[0-9]*\.[0-9]+", |lex| lex.slice().to_owned())]
    Real(String),
    /// A plain integer literal, lexeme included — e.g. `42`.
    #[regex(r"[0-9]+", |lex| lex.slice().to_owned())]
    Integer(String),

    /// A single- or double-quoted string literal, quotes and escapes preserved
    /// in the slice; also carries quoted temporals (see the module docs).
    #[regex(r"'([^'\\]|\\.)*'", |lex| lex.slice().to_owned())]
    #[regex(r#""([^"\\]|\\.)*""#, |lex| lex.slice().to_owned())]
    String(String),

    // Plain identifier (lowest-priority word token).
    #[regex(r"[a-zA-Z][a-zA-Z0-9_]*", |lex| lex.slice().to_owned())]
    /// A plain identifier, slice included — the lowest-priority word token.
    Identifier(String),
}

/// Callback for the `--` token implementing `AqlLexer.g4 COMMENT`.
///
/// A `--` followed by a space and text, or immediately by end-of-line/EOF, is a
/// line comment: the rest of the line is consumed and the token skipped.
/// Anything else is emitted as the `SYM_DOUBLE_DASH` terminator.
fn line_comment(lex: &mut logos::Lexer<Token>) -> logos::Filter<()> {
    let rem = lex.remainder();
    let is_comment = rem.is_empty() || rem.starts_with([' ', '\t', '\r', '\n']);
    if is_comment {
        let end = rem.find(['\r', '\n']).unwrap_or(rem.len());
        lex.bump(end);
        logos::Filter::Skip
    } else {
        logos::Filter::Emit(())
    }
}

/// A lexed token stream carrying each token's byte span in the source.
///
/// [`SpannedTokens::tokens`] and [`SpannedTokens::spans`] are index-aligned:
/// `spans()[i]` is the byte range of `src` that `tokens()[i]` was lexed from.
/// They stay parallel because a parser over this stream consumes a contiguous
/// `&[Token]`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedTokens {
    tokens: Vec<Token>,
    spans: Vec<std::ops::Range<usize>>,
}

impl SpannedTokens {
    /// Returns the tokens, in source order.
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// Returns each token's byte range in the source.
    ///
    /// Index-aligned with [`SpannedTokens::tokens`].
    #[must_use]
    pub fn spans(&self) -> &[std::ops::Range<usize>] {
        &self.spans
    }

    /// The stream with the tokens at indices `removed` taken out; every kept
    /// token keeps its byte span in the original source.
    #[cfg(feature = "federation")]
    pub(crate) fn without(&self, removed: std::ops::Range<usize>) -> Self {
        let (tokens, spans) = self
            .tokens
            .iter()
            .zip(&self.spans)
            .enumerate()
            .filter(|(i, _)| !removed.contains(i))
            .map(|(_, (token, span))| (token.clone(), span.clone()))
            .unzip();
        Self { tokens, spans }
    }

    /// Consumes the stream and returns its tokens, discarding the spans.
    #[must_use]
    pub fn into_tokens(self) -> Vec<Token> {
        self.tokens
    }

    /// Maps a half-open range of token indices onto the byte range of the
    /// source those tokens cover.
    ///
    /// An empty range, or one that starts past the last token, yields the
    /// empty range just after the last token — the position an end-of-input
    /// report names. An empty stream yields `0..0`.
    #[must_use]
    pub fn byte_span(&self, tokens: &std::ops::Range<usize>) -> std::ops::Range<usize> {
        let after_last = self.spans.last().map_or(0, |span| span.end);
        let start = self
            .spans
            .get(tokens.start)
            .map_or(after_last, |span| span.start);
        let end = if tokens.end > tokens.start {
            tokens
                .end
                .checked_sub(1)
                .and_then(|last| self.spans.get(last))
                .map_or(start, |span| span.end)
        } else {
            start
        };
        start..end
    }
}

/// Lexes `src` into a token vector, or reports the byte span of the first
/// token that fails to lex.
///
/// # Errors
/// Returns [`LexError`] on the first unrecognized token.
pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    lex_spanned(src).map(SpannedTokens::into_tokens)
}

/// Lexes `src` into tokens paired with the byte span each was lexed from.
///
/// The spanned counterpart of [`lex`], which discards the positions.
///
/// # Errors
/// Returns [`LexError`] on the first unrecognized token.
pub fn lex_spanned(src: &str) -> Result<SpannedTokens, LexError> {
    let mut tokens = Vec::new();
    let mut spans = Vec::new();
    let mut lexer = Token::lexer(src);
    while let Some(res) = lexer.next() {
        match res {
            Ok(tok) => {
                tokens.push(tok);
                spans.push(lexer.span());
            }
            Err(()) => {
                return Err(LexError {
                    span: lexer.span(),
                    slice: lexer.slice().to_owned(),
                });
            }
        }
    }
    Ok(SpannedTokens { tokens, spans })
}

/// A lexing failure at a byte span.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unexpected token {slice:?} at bytes {span:?}")]
pub struct LexError {
    /// Byte range of the offending input.
    pub span: std::ops::Range<usize>,
    /// The offending slice.
    pub slice: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Token> {
        lex(src).unwrap_or_else(|e| panic!("lex failed: {e}"))
    }

    #[test]
    fn keywords_are_case_insensitive() {
        assert_eq!(toks("SELECT"), vec![Token::Select]);
        assert_eq!(toks("select"), vec![Token::Select]);
        assert_eq!(toks("Contains"), vec![Token::Contains("Contains".into())]);
    }

    #[test]
    fn simple_select_from() {
        // SELECT c FROM COMPOSITION c
        let t = toks("SELECT c FROM COMPOSITION c");
        assert_eq!(
            t,
            vec![
                Token::Select,
                Token::Identifier("c".into()),
                Token::From,
                Token::Identifier("COMPOSITION".into()),
                Token::Identifier("c".into()),
            ]
        );
    }

    #[test]
    fn path_with_comparison_and_string() {
        // e/ehr_id/value = 'x'
        let t = toks("e/ehr_id/value = 'x'");
        assert_eq!(
            t,
            vec![
                Token::Identifier("e".into()),
                Token::Slash,
                Token::Identifier("ehr_id".into()),
                Token::Slash,
                Token::Identifier("value".into()),
                Token::Comparison(CompOp::Eq),
                Token::String("'x'".into()),
            ]
        );
    }

    #[test]
    fn node_codes_and_archetype_hrid() {
        assert_eq!(toks("at0001"), vec![Token::AtCode("at0001".into())]);
        assert_eq!(toks("id1.2.3"), vec![Token::IdCode("id1.2.3".into())]);
        assert_eq!(
            toks("openEHR-EHR-OBSERVATION.blood_pressure.v1"),
            vec![Token::ArchetypeHrid(
                "openEHR-EHR-OBSERVATION.blood_pressure.v1".into()
            )]
        );
    }

    #[test]
    fn id_prefix_alone_is_identifier() {
        // `id` / `at` without a code are plain identifiers.
        assert_eq!(toks("id"), vec![Token::Identifier("id".into())]);
        assert_eq!(
            toks("attribute"),
            vec![Token::Identifier("attribute".into())]
        );
    }

    #[test]
    fn numbers_and_params() {
        assert_eq!(toks("42"), vec![Token::Integer("42".into())]);
        assert_eq!(toks("3.14"), vec![Token::Real("3.14".into())]);
        assert_eq!(toks("1e10"), vec![Token::SciInteger("1e10".into())]);
        assert_eq!(toks("$ehrId"), vec![Token::Parameter("$ehrId".into())]);
    }

    #[test]
    fn comparison_operators() {
        assert_eq!(toks(">="), vec![Token::Comparison(CompOp::Ge)]);
        assert_eq!(toks("!="), vec![Token::Comparison(CompOp::Ne)]);
        assert_eq!(toks("<"), vec![Token::Comparison(CompOp::Lt)]);
    }

    #[test]
    fn contains_query_shape() {
        // FROM EHR e CONTAINS COMPOSITION c
        let t = toks("FROM EHR e CONTAINS COMPOSITION c");
        assert_eq!(
            t,
            vec![
                Token::From,
                Token::Identifier("EHR".into()),
                Token::Identifier("e".into()),
                Token::Contains("CONTAINS".into()),
                Token::Identifier("COMPOSITION".into()),
                Token::Identifier("c".into()),
            ]
        );
    }

    #[test]
    fn hyphenated_term_codes_lex_as_one_token() {
        // Hyphenated terminology ids lex as one TERM_CODE, not `id Minus code`.
        assert_eq!(
            toks("SNOMED-CT::1234"),
            vec![Token::TermCode("SNOMED-CT::1234".into())]
        );
        assert_eq!(
            toks("ISO_639-1::en"),
            vec![Token::TermCode("ISO_639-1::en".into())]
        );
        assert_eq!(
            toks("SNOMED-CT::1234|cyanosis|"),
            vec![Token::TermCode("SNOMED-CT::1234|cyanosis|".into())]
        );
        // The parenthesised terminology-version form also admits hyphens.
        assert_eq!(
            toks("snomed-ct(3.1)::3415004"),
            vec![Token::TermCode("snomed-ct(3.1)::3415004".into())]
        );
    }

    #[test]
    fn subtraction_is_not_a_term_code_regression() {
        // Without `::` there is no TERM_CODE, so `a-b` stays separate tokens.
        assert_eq!(
            toks("a-b"),
            vec![
                Token::Identifier("a".into()),
                Token::Minus,
                Token::Identifier("b".into()),
            ]
        );
        assert_eq!(
            toks("a - 1"),
            vec![
                Token::Identifier("a".into()),
                Token::Minus,
                Token::Integer("1".into()),
            ]
        );
    }

    #[test]
    fn archetype_hrid_version_suffixes_and_namespace_hyphen() {
        // A `-rc`/`-alpha` suffix and a hyphenated namespace both lex as one
        // ARCHETYPE_HRID.
        assert_eq!(
            toks("openEHR-EHR-OBSERVATION.blood_pressure.v1.0.0-rc.2"),
            vec![Token::ArchetypeHrid(
                "openEHR-EHR-OBSERVATION.blood_pressure.v1.0.0-rc.2".into()
            )]
        );
        assert_eq!(
            toks("openEHR-EHR-OBSERVATION.blood_pressure.v2-alpha"),
            vec![Token::ArchetypeHrid(
                "openEHR-EHR-OBSERVATION.blood_pressure.v2-alpha".into()
            )]
        );
        assert_eq!(
            toks("org-x::openEHR-EHR-OBSERVATION.blood_pressure.v1"),
            vec![Token::ArchetypeHrid(
                "org-x::openEHR-EHR-OBSERVATION.blood_pressure.v1".into()
            )]
        );
    }

    #[test]
    fn line_comments_are_skipped() {
        // `-- text` to end of line is a comment (skipped).
        assert_eq!(
            toks("SELECT c -- trailing comment\nFROM COMPOSITION c"),
            vec![
                Token::Select,
                Token::Identifier("c".into()),
                Token::From,
                Token::Identifier("COMPOSITION".into()),
                Token::Identifier("c".into()),
            ]
        );
        // A bare `--` at end-of-input is a comment too (skipped).
        assert_eq!(toks("c --"), vec![Token::Identifier("c".into())]);
        assert_eq!(toks("c --\n"), vec![Token::Identifier("c".into())]);
        // `--` glued to a non-space token is the `SYM_DOUBLE_DASH` terminator.
        assert_eq!(
            toks("--foo"),
            vec![Token::DoubleDash, Token::Identifier("foo".into())]
        );
    }

    #[test]
    fn spans_skip_the_whitespace_between_tokens() {
        let src = "SELECT   c";
        let stream = lex_spanned(src).unwrap_or_else(|e| panic!("lex failed: {e}"));
        assert_eq!(stream.spans(), &[0..6, 9..10]);
        assert_eq!(stream.byte_span(&(0..2)), 0..10);
        assert_eq!(stream.byte_span(&(1..2)), 9..10);
    }

    #[test]
    fn a_token_range_past_the_end_maps_to_the_end_of_the_source() {
        let stream = lex_spanned("SELECT").unwrap_or_else(|e| panic!("lex failed: {e}"));
        // chumsky reports end-of-input as the empty range at the stream length.
        assert_eq!(stream.byte_span(&(1..1)), 6..6);
        assert_eq!(stream.byte_span(&(0..0)), 0..0);

        let empty = lex_spanned("").unwrap_or_else(|e| panic!("lex failed: {e}"));
        assert_eq!(empty.byte_span(&(0..0)), 0..0);
    }

    #[test]
    fn utf_bom_is_skipped() {
        // a leading BOM is skipped, not a lex error.
        assert_eq!(toks("\u{feff}SELECT"), vec![Token::Select]);
    }

    #[test]
    fn term_code_and_predicate_brackets() {
        // [openEHR-EHR-OBSERVATION.x.v1] and local::at0001
        assert_eq!(
            toks("local::at0001"),
            vec![Token::TermCode("local::at0001".into())]
        );
        let t = toks("[at0001]");
        assert_eq!(
            t,
            vec![
                Token::LeftBracket,
                Token::AtCode("at0001".into()),
                Token::RightBracket
            ]
        );
    }
}
