// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The LANG 1.0.0 lexical layer: the shared `logos` token superset read under
//! this generation's ONE language, ODIN.
//!
//! LANG Release-1.0.0 publishes exactly one machine-readable syntax — ODIN
//! (`vendor/grammar/v1_0/{odin.g4, odin_values.g4, base_patterns.g4,
//! base_lexer.g4}`, the release's own syntax-appendix include set). Its
//! Expression Language is DEVELOPMENT prose with no grammar and BEL first
//! appears in 1.1.0, so this generation carries no other reading and no other
//! entry point — [`lex_odin`] is the whole surface.
//!
//! # The two-stage contract
//!
//! 1. **One DFA, the workspace token superset** ([`Token`]). It is the same
//!    union shape the v1_1 generation reads under four languages; it runs
//!    once over the source and produces the longest match at every position.
//! 2. **The ODIN RECLASSIFICATION pass**. For each token it asks what the
//!    Release-1.0.0 ODIN lexer would have produced for the same source
//!    slice, and:
//!    - keeps the token when ODIN admits it;
//!    - **re-tags** it when ODIN reads the same text differently — the
//!      keyword variants stay UNIT variants and ODIN, which reserves
//!      nothing, gets the identifier variant back, read off the source at
//!      the token's span;
//!    - **narrows** it when ODIN's longest match at that position is
//!      shorter, by retrying successively shorter prefixes — the union can
//!      only ever match at least as far as one member;
//!    - **fails** when no prefix is an ODIN token at all, which is exactly
//!      where a stand-alone 1.0.0 ODIN lexer reported a lexical error.
//!
//! # Adjudications this layer encodes
//!
//! - ODIN reserves nothing (`LANG/docs/odin/master03-basics.adoc` §Keywords:
//!   "ODIN has no keywords of its own"), so the reclassification pass demotes
//!   every foreign keyword of the shared superset to an identifier.
//! - The 1.0.0-only lexical deltas — `,`-only fractional seconds on times,
//!   `.`-only on durations, no `ALPHA_UNDERSCORE_ID` — are pinned on the
//!   affected tokens in `token.rs` and in `reclassify`'s arms, each with its
//!   Release-1.0.0 citation.

mod reclassify;
mod token;

use logos::Logos;

/// The source text a lexer span names.
///
/// Total by construction: every span reaching this function was produced by
/// `logos` over the SAME `src`, so it always names a character boundary inside
/// it. Returning an empty string instead would report a lexical defect with no
/// text — a silent wrong diagnostic.
#[expect(
    clippy::expect_used,
    reason = "the span comes from lexing this same `src`, so it always slices"
)]
fn span_text(src: &str, span: core::ops::Range<usize>) -> &str {
    src.get(span)
        .expect("a span produced by lexing this source should slice it")
}

use crate::v1_0::lexer::reclassify::reclassify;
pub use crate::v1_0::lexer::token::Token;

/// A lexed token together with its byte span in the original source.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    /// The token.
    pub token: Token,
    /// Byte range of the token in the original source.
    pub span: std::ops::Range<usize>,
}

/// A lexical failure: the byte span of the offending input and its text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unrecognised token {text:?} at byte offset {}", .span.start)]
pub struct LexError {
    /// Byte range of the offending input.
    pub span: std::ops::Range<usize>,
    /// The offending slice, verbatim.
    pub text: String,
}

/// Lex `src` under the Release-1.0.0 ODIN reading (`LANG/docs/odin/`;
/// `vendor/grammar/v1_0/{odin.g4, odin_values.g4}` over `base_patterns.g4` +
/// `base_lexer.g4`).
///
/// ODIN is a standalone leaf-data notation — it backs BMM `.bmm`/`.idx` files
/// and the ADL description/terminology/annotation sections alike — and
/// reserves no keywords, so this reading covers only the ODIN value +
/// structure subset.
///
/// # Errors
/// Returns a [`LexError`] at the byte span of the first input that is not an
/// ODIN token (an unrecognised character or an illegal string escape).
pub fn lex_odin(src: &str) -> Result<Vec<Spanned>, LexError> {
    let mut out = Vec::new();
    let mut lexer = Token::lexer(src);
    while let Some(result) = lexer.next() {
        let span = lexer.span();
        let text = span_text(src, span.clone());
        let Ok(produced) = result else {
            let end = stuck_at(src, span.start, span.end);
            return Err(LexError {
                text: span_text(src, span.start..end).to_owned(),
                span: span.start..end,
            });
        };
        match reclassify(&produced, text, src, span.start) {
            // The BOM carries no syntax and is dropped.
            Some(Token::Bom) => {}
            Some(read) => out.push(Spanned { token: read, span }),
            None => {
                let resumed = narrow(src, span.start, span.end, &mut out)?;
                lexer = Token::lexer(src);
                lexer.bump(resumed);
            }
        }
    }
    retag_odin_value_words_in_key_position(src, &mut out);
    Ok(out)
}

/// Re-tag `true`/`false`/`infinity` to identifiers where they stand as ODIN
/// attribute names.
///
/// `LANG/docs/odin/master03-basics.adoc` §Keywords: "ODIN has no keywords of
/// its own: all identifiers are assumed to come from an information model" —
/// yet these three words stay TOKENS under the ODIN reading because they are
/// genuine ODIN VALUES (`master07-leaf_data` §Boolean Data; the interval
/// endpoints of `AM/docs/ADL1.4/master04-dadl` §Intervals of Ordered Primitive
/// Types), so the per-token `reclassify` pass cannot demote them. Key
/// position is decidable with one token of lookahead instead: an attribute
/// name is always followed by `=` (the Release-1.0.0 `odin.g4`
/// `attr_val : attribute_id '=' object_block`), and no VALUE position ever
/// is — so a value word immediately before `SYM_EQ` is re-tagged to the
/// identifier its spelling gives, exactly as the reclassification pass
/// demotes every other keyword.
fn retag_odin_value_words_in_key_position(src: &str, spanned: &mut [Spanned]) {
    let mut index = 0;
    while index < spanned.len() {
        let is_value_word = matches!(
            spanned.get(index).map(|s| &s.token),
            Some(Token::SymTrue | Token::SymFalse | Token::SymInfinity)
        );
        let before_eq = matches!(spanned.get(index + 1).map(|s| &s.token), Some(Token::SymEq));
        if is_value_word
            && before_eq
            && let Some(entry) = spanned.get_mut(index)
        {
            let slice = span_text(src, entry.span.clone());
            entry.token = if slice.starts_with(|c: char| c.is_ascii_uppercase()) {
                Token::AlphaUcId(slice.to_owned())
            } else {
                Token::AlphaLcId(slice.to_owned())
            };
        }
        index += 1;
    }
}

/// Emit the longest prefix of `src[start..limit]` that IS a single ODIN
/// token, and return where the caller resumes.
///
/// The union DFA matched `limit`, which the ODIN reading refused. A
/// stand-alone ODIN DFA would have taken the longest prefix some production
/// of ITS OWN admits — and every such production is in the union, so
/// re-running the union over each shorter prefix and asking `reclassify`
/// again finds exactly that prefix.
///
/// # Errors
/// Returns a [`LexError`] spanning the first character when no prefix at all
/// is an ODIN token — the position a stand-alone lexer failed at.
fn narrow(
    src: &str,
    start: usize,
    limit: usize,
    out: &mut Vec<Spanned>,
) -> Result<usize, LexError> {
    let mut end = previous_boundary(src, limit, start);
    while end > start {
        if let Some(produced) = single_token(src, start, end) {
            let text = span_text(src, start..end);
            if let Some(read) = reclassify(&produced, text, src, start) {
                if !matches!(read, Token::Bom) {
                    out.push(Spanned {
                        token: read,
                        span: start..end,
                    });
                }
                return Ok(end);
            }
        }
        end = previous_boundary(src, end, start);
    }
    let end = next_boundary(src, start);
    Err(LexError {
        span: start..end,
        text: span_text(src, start..end).to_owned(),
    })
}

/// Where the ODIN lexer stops consuming when the shared DFA gets stuck at
/// `start` after reaching `union_end`.
///
/// A DFA walks as far as ANY of its patterns can still be matching before it
/// reports failure, so the union's extent overshoots a member whose
/// production set cannot leave the start state on that character at all.
/// Exactly one character in the union is in that position: `?`, which starts
/// a token only through the leading `??` field of `TIME_CONSTRAINT_PATTERN`
/// (`ADL1.4/master05-cadl.adoc` §Symbols L1420), a cADL-only production — the
/// ODIN reading cannot begin any token with it, so it fails on the character
/// itself. Every other union-only class produces a whole TOKEN that the
/// reclassification pass refuses, which [`narrow`] already resolves. The
/// failing OFFSET is the same either way; this keeps the reported extent the
/// same too.
fn stuck_at(src: &str, start: usize, union_end: usize) -> usize {
    if src.get(start..).is_some_and(|rest| rest.starts_with('?')) {
        next_boundary(src, start)
    } else {
        union_end
    }
}

/// The single token the shared DFA produces for exactly `src[start..end]`, or
/// `None` when that text is not one whole token.
fn single_token(src: &str, start: usize, end: usize) -> Option<Token> {
    let text = src.get(start..end)?;
    let mut lexer = Token::lexer(text);
    let produced = lexer.next()?.ok()?;
    (lexer.span().end == text.len()).then_some(produced)
}

/// The greatest character boundary strictly below `at`, floored at `floor`.
fn previous_boundary(src: &str, at: usize, floor: usize) -> usize {
    let mut back = at.saturating_sub(1);
    while back > floor && !src.is_char_boundary(back) {
        back -= 1;
    }
    back
}

/// The character boundary just above `at` (the end of the character there).
fn next_boundary(src: &str, at: usize) -> usize {
    let mut forward = at.saturating_add(1);
    while forward < src.len() && !src.is_char_boundary(forward) {
        forward += 1;
    }
    forward.min(src.len())
}

#[cfg(test)]
mod tests {
    use super::{Token, lex_odin};

    fn odin(src: &str) -> Vec<Token> {
        lex_odin(src)
            .unwrap_or_else(|e| panic!("lex failed: {e}"))
            .into_iter()
            .map(|s| s.token)
            .collect()
    }

    /// `LANG/docs/odin/master03-basics.adoc` §Keywords: "ODIN has no keywords
    /// of its own" — every foreign keyword reads back as an ODIN identifier.
    #[test]
    fn odin_reserves_no_foreign_keyword() {
        for word in [
            "matches",
            "occurrences",
            "existence",
            "cardinality",
            "ordered",
            "include",
            "then",
            "and",
            "or",
            "not",
            "implies",
            "for_all",
            "there_exists",
            "in",
            "use_archetype",
        ] {
            assert_eq!(
                odin(word),
                vec![Token::AlphaLcId(word.to_owned())],
                "{word} must be an ODIN identifier"
            );
        }
        assert_eq!(odin("MATCHES"), vec![Token::AlphaUcId("MATCHES".into())]);
    }

    /// The ADL-only token classes have zero ODIN production, so their text
    /// reads back under ODIN's own rules.
    #[test]
    fn odin_has_no_adl_only_token_classes() {
        assert_eq!(odin("at0000"), vec![Token::AlphaLcId("at0000".into())]);
        assert_eq!(
            odin("id1.1"),
            vec![
                Token::AlphaLcId("id1".into()),
                Token::SymDot,
                Token::Integer("1".into()),
            ]
        );
        assert_eq!(
            odin("2.0.5"),
            vec![
                Token::Real("2.0".into()),
                Token::SymDot,
                Token::Integer("5".into()),
            ]
        );
        assert_eq!(
            odin("yyyy-mm-dd"),
            vec![
                Token::AlphaLcId("yyyy".into()),
                Token::SymMinus,
                Token::AlphaLcId("mm".into()),
                Token::SymMinus,
                Token::AlphaLcId("dd".into()),
            ]
        );
        // `$variable`, `{`, `:`, `%`, `^` and `!=` have no ODIN production.
        for refused in ["$v", "{", ":", "%", "^", "!="] {
            assert!(lex_odin(refused).is_err(), "ODIN must refuse {refused:?}");
        }
        // `<>` is not an ODIN `SYM_NE`: it splits into the `SYM_LT` `SYM_GT`
        // an empty ODIN block is written with.
        assert_eq!(odin("<>"), vec![Token::SymLt, Token::SymGt]);
        // `@` IS an ODIN token: it opens the document prefix
        // `schema_identifier ::= '@' schema '=' URI`
        // (`LANG/docs/odin/master04-odin_artefacts` intro); a misplaced `@`
        // is the parser's refusal, not the lexer's.
        assert_eq!(odin("@"), vec![Token::SymAt]);
    }

    /// NOTE: the Release-1.0.0 `base_lexer.g4` declares no
    /// `ALPHA_UNDERSCORE_ID`, so a `_`-initial word is a 1.0.0 lex error.
    #[test]
    fn underscore_initial_words_are_not_1_0_0_tokens() {
        assert!(lex_odin("_default").is_err());
        assert!(lex_odin("_x = <1>").is_err());
        // An INTERIOR underscore is ordinary `WORD_CHAR` text.
        assert_eq!(
            odin("some_attr"),
            vec![Token::AlphaLcId("some_attr".into())]
        );
    }

    /// ISO 8601 values, including the `??` partial forms of
    /// `master07-leaf_data` §Partial Date/Times (docs text this generation
    /// shares verbatim with 1.1.0).
    #[test]
    fn iso_values_and_partials() {
        assert_eq!(
            odin("2004-06-01"),
            vec![Token::Iso8601Date("2004-06-01".into())]
        );
        assert_eq!(odin("2004-06"), vec![Token::Iso8601Date("2004-06".into())]);
        assert_eq!(
            odin("2004-06-??"),
            vec![Token::Iso8601Date("2004-06-??".into())]
        );
        assert_eq!(
            odin("2004-06-01T10:30:00"),
            vec![Token::Iso8601DateTime("2004-06-01T10:30:00".into())]
        );
        assert_eq!(
            odin("10:30:00"),
            vec![Token::Iso8601Time("10:30:00".into())]
        );
        assert_eq!(odin("P1Y2M"), vec![Token::Iso8601Duration("P1Y2M".into())]);
        assert_eq!(odin("PT30M"), vec![Token::Iso8601Duration("PT30M".into())]);
        assert_eq!(odin("P0W"), vec![Token::Iso8601Duration("P0W".into())]);
    }

    /// NOTE: fractional seconds on times take `,` alone in Release-1.0.0
    /// (`base_lexer.g4` `ISO8601_TIME : … ( SYM_COMMA INTEGER )?`; the
    /// `master07-leaf_data` example `16:35:04,5`); `.` postdates it.
    #[test]
    fn time_fractional_seconds_are_comma_separated() {
        assert_eq!(
            odin("16:35:04,5"),
            vec![Token::Iso8601Time("16:35:04,5".into())]
        );
        // A dot does not extend the time token.
        assert_eq!(
            odin("16:35:04.5"),
            vec![
                Token::Iso8601Time("16:35:04".into()),
                Token::SymDot,
                Token::Integer("5".into()),
            ]
        );
        assert_eq!(
            odin("2004-06-01T16:35:04,5"),
            vec![Token::Iso8601DateTime("2004-06-01T16:35:04,5".into())]
        );
    }

    /// NOTE: fractional seconds in durations take `.` alone in Release-1.0.0
    /// (`base_lexer.g4` `ISO8601_DURATION : … ('.' DIGIT+)? [sS]`); the `,`
    /// alternative postdates it.
    #[test]
    fn duration_fractional_seconds_are_dot_separated() {
        assert_eq!(
            odin("PT2.5S"),
            vec![Token::Iso8601Duration("PT2.5S".into())]
        );
        // The comma form is not a 1.0.0 duration token.
        assert_ne!(
            odin("PT2,5S"),
            vec![Token::Iso8601Duration("PT2,5S".into())]
        );
    }

    /// `master07-leaf_data` §Terms and Term Codes + the embedded-URI leaf.
    #[test]
    fn term_code_ref_and_embedded_uri() {
        assert_eq!(
            odin("[ISO_639-1::en]"),
            vec![Token::TermCodeRef("[ISO_639-1::en]".into())]
        );
        assert_eq!(
            odin("<http://loinc.org/id/9272-6>"),
            vec![Token::EmbeddedUri("<http://loinc.org/id/9272-6>".into())]
        );
        // a `<[…]>` value block is NOT a URI: `<` then term code then `>`.
        assert_eq!(
            odin("<[ISO_639-1::en]>"),
            vec![
                Token::SymLt,
                Token::TermCodeRef("[ISO_639-1::en]".into()),
                Token::SymGt,
            ]
        );
    }

    /// `AM/docs/ADL1.4/master04-dadl` §Symbols `V_LOCAL_TERM_CODE_REF` is an
    /// ODIN leaf value; integer container keys still lex as keys.
    #[test]
    fn local_term_code_ref_and_keys() {
        assert_eq!(
            odin("[at0200]"),
            vec![Token::LocalTermCodeRef("[at0200]".into())]
        );
        assert_eq!(
            odin("[1]"),
            vec![Token::LBracket, Token::Integer("1".into()), Token::RBracket,]
        );
    }

    /// Intervals: delimiters, relational bounds and the `infinity` endpoint
    /// (`AM/docs/ADL1.4/master04-dadl` §Intervals of Ordered Primitive Types).
    #[test]
    fn interval_symbols_and_infinity() {
        assert_eq!(
            odin("|>=0.0..<10.0|"),
            vec![
                Token::SymIvlDelim,
                Token::SymGe,
                Token::Real("0.0".into()),
                Token::SymIvlSep,
                Token::SymLt,
                Token::Real("10.0".into()),
                Token::SymIvlDelim,
            ]
        );
        assert_eq!(
            odin("|0..infinity|"),
            vec![
                Token::SymIvlDelim,
                Token::Integer("0".into()),
                Token::SymIvlSep,
                Token::SymInfinity,
                Token::SymIvlDelim,
            ]
        );
        // `1..5` must lex as INTEGER SYM_IVL_SEP INTEGER, not a REAL.
        assert_eq!(
            odin("1..5"),
            vec![
                Token::Integer("1".into()),
                Token::SymIvlSep,
                Token::Integer("5".into()),
            ]
        );
    }

    /// Booleans are case-insensitive (`master07-leaf_data` §Boolean Data),
    /// and a value word before `=` re-tags to the identifier ODIN's
    /// no-keywords rule gives it.
    #[test]
    fn boolean_words_and_key_position_retag() {
        assert_eq!(odin("True"), vec![Token::SymTrue]);
        assert_eq!(odin("true"), vec![Token::SymTrue]);
        assert_eq!(odin("fAlSe"), vec![Token::SymFalse]);
        assert_eq!(
            odin("true = <1>"),
            vec![
                Token::AlphaLcId("true".into()),
                Token::SymEq,
                Token::SymLt,
                Token::Integer("1".into()),
                Token::SymGt,
            ]
        );
    }

    /// `AM/docs/ADL1.4/master08-adl` §Revision History Section writes
    /// `time_committed = <2004-11-02 09:31:04+1000>`; the ODIN reading
    /// accepts the space form and normalises it to the ISO `T` designator.
    #[test]
    fn space_separated_date_time_is_normalised() {
        assert_eq!(
            odin("2004-11-02 09:31:04+1000"),
            vec![Token::Iso8601DateTime("2004-11-02T09:31:04+1000".into())]
        );
    }

    /// `LANG/docs/odin/master07-leaf_data` §String Data: multi-line string
    /// contents drop the white-space leaders of the continuation lines.
    #[test]
    fn multi_line_string_leaders_are_stripped() {
        let src = "    text = <\"first\n        second\">";
        let stripped = Token::String("\"first\nsecond\"".into());
        assert!(odin(src).contains(&stripped));
    }

    /// `master03` escapes: the legal quoted forms lex, an illegal escape is a
    /// lex error, and a BOM is skipped.
    #[test]
    fn strings_characters_and_escapes() {
        assert_eq!(
            odin(r#""a\"x'c\\d""#),
            vec![Token::String(r#""a\"x'c\\d""#.into())]
        );
        for ok in [
            r"'\n'", r"'\t'", r"'\r'", r"'\\'", r#"'\"'"#, r"'\''", "'x'", "'ü'",
        ] {
            assert!(lex_odin(ok).is_ok(), "legal character must lex: {ok}");
        }
        assert!(lex_odin(r#""bad \d escape""#).is_err());
        assert!(lex_odin(r"'\q'").is_err());
        assert_eq!(
            odin("\u{feff}archetype"),
            vec![Token::AlphaLcId("archetype".into())]
        );
    }

    /// Line comments are skipped
    /// (`LANG/docs/odin/master03-basics.adoc` §Comments).
    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            odin("items -- a trailing comment\nvalue"),
            vec![
                Token::AlphaLcId("items".into()),
                Token::AlphaLcId("value".into())
            ]
        );
    }

    /// The `(syntax) <# … #>` plug-in block
    /// (`LANG/docs/odin/master09-plug_in_syntaxes`).
    #[test]
    fn plug_in_block_lexes() {
        assert_eq!(
            odin("(cadl) <# ELEMENT[at0001] #>"),
            vec![
                Token::LParen,
                Token::AlphaLcId("cadl".into()),
                Token::RParen,
                Token::PlugInBlock("<# ELEMENT[at0001] #>".into()),
            ]
        );
    }

    /// Path segment heads are lower-case in Release-1.0.0 (`base_lexer.g4`
    /// `PATH_SEGMENT : ALPHA_LC_ID …`; the v1_1 either-case widening's
    /// `odin_object_key` ground does not exist in this generation).
    #[test]
    fn path_segment_heads_are_lower_case() {
        assert_eq!(odin("/foo/bar"), vec![Token::AdlPath("/foo/bar".into())]);
        assert_ne!(odin("/Foo/bar"), vec![Token::AdlPath("/Foo/bar".into())]);
    }
}
