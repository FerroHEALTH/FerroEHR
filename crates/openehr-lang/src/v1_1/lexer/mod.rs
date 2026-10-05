// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The ONE openEHR lexical layer: a single `logos` token superset plus four
//! thin per-language entry points.
//!
//! ADL2/cADL, ODIN, BEL and EL are four readings of one family of lexical
//! rules — the vendored `.g4` grammars all build on the same base id/symbol
//! layer (`vendor/grammar/v1_1/`), and each language's syntax appendix is an
//! include of those files, which makes them normative-by-reference. This
//! module is that shared layer, and it is the ONLY lexer in the workspace:
//! `openehr-adl` and the `odin`/`bel`/`el` readers here all consume
//! [`Token`]/[`Spanned`] from it.
//!
//! # The two-stage contract
//!
//! 1. **One DFA, the union of the four lexical layers** ([`Token`]). It runs
//!    once over the source and produces the longest match at every position.
//! 2. **A per-language RECLASSIFICATION pass**. For each
//!    token it asks what that language's own lexer would have produced for the
//!    same source slice, and:
//!    - keeps the token when the language admits it;
//!    - **re-tags** it when the language reads the same text differently — the
//!      keyword variants stay UNIT variants and a language that reserves
//!      nothing simply gets the identifier variant back, read off the source
//!      at the token's span;
//!    - **narrows** it when the language's longest match at that position is
//!      shorter, by retrying successively shorter prefixes — the
//!      union can only ever match at least as far as one member;
//!    - **fails** when no prefix is a token of that language at all, which is
//!      exactly where its stand-alone lexer reported a lexical error.
//!
//! The pass is therefore a total function from the union reading to each member
//! reading, not a filter, and the `lexer_equivalence` battery in `tests/it/`
//! pins each entry point token-for-token and span-for-span.
//!
//! # Per-language adjudications
//!
//! - ODIN reserves nothing (`LANG/docs/odin/master03-basics.adoc` §Keywords:
//!   "ODIN has no keywords of its own"), so the ODIN pass demotes every
//!   cADL/BEL keyword to an attribute identifier.
//! - cADL section keywords are not reserved either (`AM/docs/ADL2/master07.04`:
//!   they "can safely appear as identifiers in the definition and terminology
//!   sections"), so they are not tokens at all: `language`, `definition`, … lex
//!   as `ALPHA_LC_ID` and the outer parser recognises a header positionally at
//!   column 0. A section word inside a quoted multi-line value therefore cannot
//!   read as a header.
//! - Keyword matching is ASCII-case-insensitive in the cADL reading
//!   (`adl_keywords.g4` spells every keyword `[Mm][Aa][Tt]…`-style;
//!   `ADL1.4/master05-cadl.adoc` §Symbols L1326-1354 says the same in prose) and
//!   case-SENSITIVE for BEL's operators, which `base_expressions.g4` spells that
//!   way.
//! - The EL reading takes the cADL layer wherever `ElLexer.g4` declares nothing
//!   and `ElLexer.g4`'s own case-sensitive spelling wherever it does.

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

use crate::v1_1::lexer::reclassify::{Language, reclassify};
pub use crate::v1_1::lexer::token::Token;

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

/// Lex `src` under the ADL2 / cADL reading (`AM/docs/ADL2/`; `adl2.g4`,
/// `cadl2.g4`, `adl_keywords.g4` over `base_lexer.g4`).
///
/// A whole ADL2 source file is lexed once as a single stream, so this reading
/// is the union of the outer artefact grammar, cADL, the ODIN sections and the
/// rules sub-syntax — the outer parser only *parses* the identification header
/// and the ODIN sections, capturing the cADL definition and rules bodies as
/// raw spans, so their tokens only need to lex here, not classify perfectly.
///
/// # Errors
/// Returns a [`LexError`] at the byte span of the first input that is not a
/// cADL token (an unrecognised character or an illegal string escape).
pub fn lex_adl(src: &str) -> Result<Vec<Spanned>, LexError> {
    lex_with(Language::Adl, src)
}

/// Lex `src` under the ODIN reading (`LANG/docs/odin/`; `odin.g4`,
/// `odin_values.g4` over `base_lexer.g4`).
///
/// ODIN is a standalone leaf-data notation — it backs BMM `.bmm`/`.idx` files
/// and the ADL description/terminology/annotation sections alike — and
/// reserves no keywords, so this reading covers only the ODIN value +
/// structure subset.
///
/// # Errors
/// Returns a [`LexError`] at the byte span of the first input that is not an
/// ODIN token.
pub fn lex_odin(src: &str) -> Result<Vec<Spanned>, LexError> {
    let mut spanned = lex_with(Language::Odin, src)?;
    retag_odin_value_words_in_key_position(src, &mut spanned);
    Ok(spanned)
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
/// name is always followed by `=` (`odin.g4` `attr_val : odin_object_key '='
/// object_block`), and no VALUE position ever is — so a value word
/// immediately before `SYM_EQ` is re-tagged to the identifier its spelling
/// gives, exactly as the reclassification pass demotes every other keyword.
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

/// Lex `src` under the BEL reading (`LANG/docs/BEL/`; `base_expressions.g4`).
///
/// The Basic Expression Language surface: statements, assertions, assignments,
/// operators (text + symbol forms), literals, variables, paths and the
/// `matches { … }` constraint delimiters.
///
/// # Errors
/// Returns a [`LexError`] at the byte span of the first input that is not a
/// BEL token.
pub fn lex_bel(src: &str) -> Result<Vec<Spanned>, LexError> {
    lex_with(Language::Bel, src)
}

/// Lex `src` under the Expression Language reading (`LANG/docs/EL/`;
/// `vendor/grammar/v1_1/ElLexer.g4`, which the EL syntax appendix
/// `masterAppA-syntax.adoc` includes verbatim).
///
/// `ElLexer.g4` imports `Cadl2Lexer`, `SymbolsLexer` and `GeneralIdsLexer`
/// (none of which upstream publishes in the same repository), so this reading
/// is the cADL one for the inherited layer plus `ElLexer.g4`'s own rules,
/// which ANTLR gives precedence.
///
/// Two `ElLexer.g4` symbols have no union production and are therefore not
/// lexable here: `?` (`SYM_INTERROGATION`) and the guillemets `«`/`»`. `?`
/// reaches only `dlBinaryChoice` and the guillemets reach no `ElParser.g4`
/// production at all.
///
/// # Errors
/// Returns a [`LexError`] at the byte span of the first input that is not an
/// EL token.
pub fn lex_el(src: &str) -> Result<Vec<Spanned>, LexError> {
    lex_with(Language::El, src)
}

/// Run the shared DFA over `src` and reclassify every token into `language`'s
/// own reading.
fn lex_with(language: Language, src: &str) -> Result<Vec<Spanned>, LexError> {
    let mut out = Vec::new();
    let mut lexer = Token::lexer(src);
    while let Some(result) = lexer.next() {
        let span = lexer.span();
        let text = span_text(src, span.clone());
        let Ok(produced) = result else {
            let end = stuck_at(language, src, span.start, span.end);
            return Err(LexError {
                text: span_text(src, span.start..end).to_owned(),
                span: span.start..end,
            });
        };
        match reclassify(language, &produced, text, src, span.start) {
            // The BOM carries no syntax; the readings that tolerate it drop it.
            Some(Token::Bom) => {}
            Some(read) => out.push(Spanned { token: read, span }),
            None => {
                let resumed = narrow(language, src, span.start, span.end, &mut out)?;
                lexer = Token::lexer(src);
                lexer.bump(resumed);
            }
        }
    }
    Ok(out)
}

/// Emit the longest prefix of `src[start..limit]` that IS a single token of
/// `language`, and return where the caller resumes.
///
/// The union DFA matched `limit`, which `language` refused. Its own DFA would
/// have taken the longest prefix some production of ITS OWN admits — and every
/// such production is in the union, so re-running the union over each shorter
/// prefix and asking `reclassify` again finds exactly that prefix.
///
/// # Errors
/// Returns a [`LexError`] spanning the first character when no prefix at all is
/// a token of `language` — the position its stand-alone lexer failed at.
fn narrow(
    language: Language,
    src: &str,
    start: usize,
    limit: usize,
    out: &mut Vec<Spanned>,
) -> Result<usize, LexError> {
    let mut end = previous_boundary(src, limit, start);
    while end > start {
        if let Some(produced) = single_token(src, start, end) {
            let text = span_text(src, start..end);
            if let Some(read) = reclassify(language, &produced, text, src, start) {
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

/// Where `language`'s own lexer stops consuming when the shared DFA gets stuck
/// at `start` after reaching `union_end`.
///
/// A DFA walks as far as ANY of its patterns can still be matching before it
/// reports failure, so the union's extent overshoots a member whose production
/// set cannot leave the start state on that character at all. Exactly one
/// character in the union is in that position: `?`, which starts a token only
/// through the leading `??` field of `TIME_CONSTRAINT_PATTERN`
/// (`ADL1.4/master05-cadl.adoc` §Symbols L1420), a cADL-only production — the
/// ODIN and BEL readings cannot begin any token with it, so they fail on the
/// character itself. Every other union-only class produces a whole TOKEN that
/// the reclassification pass refuses, which [`narrow`] already resolves. The
/// failing OFFSET is the same either way; this keeps the reported extent the
/// same too.
fn stuck_at(language: Language, src: &str, start: usize, union_end: usize) -> usize {
    let cannot_start =
        language != Language::Adl && src.get(start..).is_some_and(|rest| rest.starts_with('?'));
    if cannot_start {
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
    use super::{Token, lex_adl, lex_bel, lex_odin};

    fn adl(src: &str) -> Vec<Token> {
        lex_adl(src)
            .unwrap_or_else(|e| panic!("lex failed: {e}"))
            .into_iter()
            .map(|s| s.token)
            .collect()
    }

    fn odin(src: &str) -> Vec<Token> {
        lex_odin(src)
            .unwrap_or_else(|e| panic!("lex failed: {e}"))
            .into_iter()
            .map(|s| s.token)
            .collect()
    }

    fn bel(src: &str) -> Vec<Token> {
        lex_bel(src)
            .unwrap_or_else(|e| panic!("lex failed: {e}"))
            .into_iter()
            .map(|s| s.token)
            .collect()
    }

    #[test]
    fn codes_and_identifiers() {
        assert_eq!(adl("id1"), vec![Token::RootIdCode("id1".into())]);
        assert_eq!(adl("id1.1"), vec![Token::RootIdCode("id1.1".into())]);
        assert_eq!(adl("id2"), vec![Token::IdCode("id2".into())]);
        assert_eq!(adl("id0.1"), vec![Token::IdCode("id0.1".into())]);
        assert_eq!(adl("at0000"), vec![Token::AtCode("at0000".into())]);
        assert_eq!(adl("at0.1"), vec![Token::AtCode("at0.1".into())]);
        assert_eq!(adl("ac1"), vec![Token::AcCode("ac1".into())]);
        assert_eq!(
            adl("OBSERVATION"),
            vec![Token::AlphaUcId("OBSERVATION".into())]
        );
        assert_eq!(adl("items"), vec![Token::AlphaLcId("items".into())]);
        // `id`/`at` without a code are plain identifiers.
        assert_eq!(adl("identity"), vec![Token::AlphaLcId("identity".into())]);
    }

    #[test]
    fn archetype_and_version_ids() {
        assert_eq!(
            adl("openehr-TEST_PKG-WHOLE.most_minimal.v2.0.0"),
            vec![Token::ArchetypeId(
                "openehr-TEST_PKG-WHOLE.most_minimal.v2.0.0".into()
            )]
        );
        // partial version (ARCHETYPE_REF shape) folds into the same token.
        assert_eq!(
            adl("openehr-TASK_PLANNING-TASK_PLAN.good_include.v0"),
            vec![Token::ArchetypeId(
                "openehr-TASK_PLANNING-TASK_PLAN.good_include.v0".into()
            )]
        );
        assert_eq!(adl("2.0.5"), vec![Token::VersionId("2.0.5".into())]);
        assert_eq!(adl("1.0.2"), vec![Token::VersionId("1.0.2".into())]);
    }

    #[test]
    fn iso_values_and_partials() {
        assert_eq!(
            adl("2004-06-01"),
            vec![Token::Iso8601Date("2004-06-01".into())]
        );
        assert_eq!(adl("2004-06"), vec![Token::Iso8601Date("2004-06".into())]);
        assert_eq!(
            adl("2004-06-??"),
            vec![Token::Iso8601Date("2004-06-??".into())]
        );
        assert_eq!(
            adl("2004-06-01T10:30:00"),
            vec![Token::Iso8601DateTime("2004-06-01T10:30:00".into())]
        );
        assert_eq!(adl("10:30:00"), vec![Token::Iso8601Time("10:30:00".into())]);
        assert_eq!(adl("P1Y2M"), vec![Token::Iso8601Duration("P1Y2M".into())]);
        assert_eq!(adl("PT30M"), vec![Token::Iso8601Duration("PT30M".into())]);
        assert_eq!(adl("P0W"), vec![Token::Iso8601Duration("P0W".into())]);
    }

    #[test]
    fn constraint_patterns() {
        assert_eq!(
            adl("yyyy-mm-dd"),
            vec![Token::DateConstraintPattern("yyyy-mm-dd".into())]
        );
        assert_eq!(
            adl("yyyy-??-XX"),
            vec![Token::DateConstraintPattern("yyyy-??-XX".into())]
        );
        assert_eq!(
            adl("hh:mm:ss"),
            vec![Token::TimeConstraintPattern("hh:mm:ss".into())]
        );
        assert_eq!(
            adl("yyyy-mm-ddThh:mm:ss"),
            vec![Token::DateTimeConstraintPattern(
                "yyyy-mm-ddThh:mm:ss".into()
            )]
        );
        assert_eq!(
            adl("PYMD"),
            vec![Token::DurationConstraintPattern("PYMD".into())]
        );
        // a real type name that starts with `P` but has other letters wins by
        // length as an identifier.
        assert_eq!(
            adl("POINT_EVENT"),
            vec![Token::AlphaUcId("POINT_EVENT".into())]
        );
    }

    #[test]
    fn interval_and_range_symbols() {
        // `1..5` must lex as INTEGER SYM_IVL_SEP INTEGER, not a REAL.
        assert_eq!(
            adl("1..5"),
            vec![
                Token::Integer("1".into()),
                Token::SymIvlSep,
                Token::Integer("5".into()),
            ]
        );
        assert_eq!(adl("1.5"), vec![Token::Real("1.5".into())]);
        assert_eq!(
            adl("|>=0.0..<10.0|"),
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
    }

    #[test]
    fn term_code_ref_and_embedded_uri() {
        assert_eq!(
            adl("[ISO_639-1::en]"),
            vec![Token::TermCodeRef("[ISO_639-1::en]".into())]
        );
        assert_eq!(
            adl("<http://loinc.org/id/9272-6>"),
            vec![Token::EmbeddedUri("<http://loinc.org/id/9272-6>".into())]
        );
        // a `<[…]>` value block is NOT a URI: `<` then term code then `>`.
        assert_eq!(
            adl("<[ISO_639-1::en]>"),
            vec![
                Token::SymLt,
                Token::TermCodeRef("[ISO_639-1::en]".into()),
                Token::SymGt,
            ]
        );
    }

    #[test]
    fn brackets_with_codes_are_not_term_codes() {
        // `[id2]` / `[ac1]` have no `::` so they split into bracket + code.
        assert_eq!(
            adl("[id2]"),
            vec![
                Token::LBracket,
                Token::IdCode("id2".into()),
                Token::RBracket
            ]
        );
        assert_eq!(
            adl("[ac1]"),
            vec![
                Token::LBracket,
                Token::AcCode("ac1".into()),
                Token::RBracket
            ]
        );
    }

    #[test]
    fn unicode_operators_lex() {
        assert_eq!(adl("\u{2208}"), vec![Token::SymMatches]);
        assert_eq!(adl("\u{2227}"), vec![Token::SymAnd]);
        assert_eq!(adl("\u{2203}"), vec![Token::SymThereExists]);
        assert_eq!(adl("matches"), vec![Token::SymMatches]);
        assert_eq!(adl("*"), vec![Token::SymStar]);
        assert_eq!(adl("\u{2217}"), vec![Token::SymStar]);
    }

    #[test]
    fn comments_and_overlay_separator_are_skipped() {
        assert_eq!(
            adl("id1 -- a trailing comment\nitems"),
            vec![
                Token::RootIdCode("id1".into()),
                Token::AlphaLcId("items".into())
            ]
        );
        assert_eq!(
            adl("--------------\ntemplate_overlay"),
            vec![Token::AlphaLcId("template_overlay".into())]
        );
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(
            adl(r#""a\"x'c\\d""#),
            vec![Token::String(r#""a\"x'c\\d""#.into())]
        );
        // multi-line string.
        assert_eq!(
            adl("\"line1\nline2\""),
            vec![Token::String("\"line1\nline2\"".into())]
        );
        // illegal escape (`\d`) is a lex error per master03.
        assert!(lex_adl(r#""bad \d escape""#).is_err());
        // BOM is skipped, not an error.
        assert_eq!(
            adl("\u{feff}archetype"),
            vec![Token::AlphaLcId("archetype".into())]
        );
    }

    #[test]
    fn character_escapes() {
        // The six legal quoted forms and a plain/unicode character lex
        // (`ADL2/master03-file_encoding.adoc` §Special Character Sequences).
        for ok in [
            r"'\n'", r"'\t'", r"'\r'", r"'\\'", r#"'\"'"#, r"'\''", "'x'", "'ü'",
        ] {
            assert!(lex_adl(ok).is_ok(), "legal character must lex: {ok}");
        }
        // "Any other character combination starting with a backslash is
        // illegal" — an unknown escape fails the lex.
        assert!(lex_adl(r"'\q'").is_err());
        assert!(lex_adl(r"'\d'").is_err());
    }

    #[test]
    fn boolean_word_symbols() {
        assert_eq!(adl("True"), vec![Token::SymTrue]);
        assert_eq!(adl("False"), vec![Token::SymFalse]);
        assert_eq!(adl("true"), vec![Token::SymTrue]);
    }

    /// `ADL1.4/master05-cadl.adoc` §Symbols L1326-1354 + `adl_keywords.g4`
    /// spell every keyword in the `[Mm][Aa]…` case-insensitive form.
    #[test]
    fn keywords_are_case_insensitive() {
        assert_eq!(adl("MATCHES"), vec![Token::SymMatches]);
        assert_eq!(adl("Is_In"), vec![Token::SymMatches]);
        assert_eq!(adl("OCCURRENCES"), vec![Token::SymOccurrences]);
        assert_eq!(adl("Existence"), vec![Token::SymExistence]);
        assert_eq!(adl("CaRdInAlItY"), vec![Token::SymCardinality]);
        assert_eq!(adl("ORDERED"), vec![Token::SymOrdered]);
        assert_eq!(adl("UNORDERED"), vec![Token::SymUnordered]);
        assert_eq!(adl("Unique"), vec![Token::SymUnique]);
        assert_eq!(adl("INFINITY"), vec![Token::SymInfinity]);
        assert_eq!(adl("Use_Node"), vec![Token::SymUseNode]);
        assert_eq!(adl("ALLOW_ARCHETYPE"), vec![Token::SymAllowArchetype]);
        assert_eq!(adl("Include"), vec![Token::SymInclude]);
        assert_eq!(adl("EXCLUDE"), vec![Token::SymExclude]);
        assert_eq!(adl("Before"), vec![Token::SymBefore]);
        assert_eq!(adl("AFTER"), vec![Token::SymAfter]);
        assert_eq!(adl("TRUE"), vec![Token::SymTrue]);
        assert_eq!(adl("fAlSe"), vec![Token::SymFalse]);
        assert_eq!(adl("NOT"), vec![Token::SymNot]);
        assert_eq!(adl("Implies"), vec![Token::SymImplies]);
        // The one keyword the grammars spell case-sensitively.
        assert_eq!(adl("there_exists"), vec![Token::SymThereExists]);
        assert_eq!(
            adl("There_Exists"),
            vec![Token::AlphaUcId("There_Exists".into())]
        );
    }

    /// `infinity` is its own keyword token (`master05` §Keywords L50, §Symbols
    /// L1349), used as an interval endpoint (`|0..infinity|`, L771).
    #[test]
    fn infinity_is_a_keyword_not_an_identifier() {
        assert_eq!(
            adl("|0..infinity|"),
            vec![
                Token::SymIvlDelim,
                Token::Integer("0".into()),
                Token::SymIvlSep,
                Token::SymInfinity,
                Token::SymIvlDelim,
            ]
        );
        assert_eq!(
            adl("|-infinity..5.0|"),
            vec![
                Token::SymIvlDelim,
                Token::SymMinus,
                Token::SymInfinity,
                Token::SymIvlSep,
                Token::Real("5.0".into()),
                Token::SymIvlDelim,
            ]
        );
    }

    /// Literal-substituted pattern fields (`master05` §Patterns L894), the
    /// ASCII timezone modifiers (L852 + the tz table L900-906) and the
    /// space-separated date/time pattern (§Symbols L1422 `[ T]`).
    #[test]
    fn pattern_variants() {
        assert_eq!(
            adl("1995-??-XX"),
            vec![Token::DateConstraintPattern("1995-??-XX".into())]
        );
        assert_eq!(
            adl("1995-mm-dd"),
            vec![Token::DateConstraintPattern("1995-mm-dd".into())]
        );
        assert_eq!(
            adl("hh:mm:ss+hh:mm"),
            vec![Token::TimeConstraintPattern("hh:mm:ss+hh:mm".into())]
        );
        assert_eq!(
            adl("hh:mm:ss-hh"),
            vec![Token::TimeConstraintPattern("hh:mm:ss-hh".into())]
        );
        assert_eq!(
            adl("hh:mm:ss+hhmm"),
            vec![Token::TimeConstraintPattern("hh:mm:ss+hhmm".into())]
        );
        assert_eq!(
            adl("hh:mm:ssZ"),
            vec![Token::TimeConstraintPattern("hh:mm:ssZ".into())]
        );
        assert_eq!(
            adl("yyyy-mm-dd hh:mm:XX"),
            vec![Token::DateTimeConstraintPattern(
                "yyyy-mm-dd hh:mm:XX".into()
            )]
        );
        assert_eq!(
            adl("yyyy-mm-ddThh:mm:ss\u{00B1}hh:mm"),
            vec![Token::DateTimeConstraintPattern(
                "yyyy-mm-ddThh:mm:ss\u{00B1}hh:mm".into()
            )]
        );
        // A text that is both a legal VALUE and a legal all-literal pattern
        // stays the value reading (see the note on the ISO8601 value tokens).
        assert_eq!(
            adl("2004-06-01"),
            vec![Token::Iso8601Date("2004-06-01".into())]
        );
    }

    /// The `^…^` regex delimiter is a `CONTAINED_REGEXP` in its own right
    /// (`master05` §Regular Expression L696-702; `V_REGEXP` L1476).
    #[test]
    fn caret_delimited_regexp_lexes() {
        assert_eq!(
            adl("{^km/h|mi/h^}"),
            vec![Token::ContainedRegexp("{^km/h|mi/h^}".into())]
        );
        assert_eq!(
            adl(r"{/km\/h|mi\/h/}"),
            vec![Token::ContainedRegexp(r"{/km\/h|mi\/h/}".into())]
        );
    }

    /// `LANG/docs/odin/master03-basics.adoc` §Keywords: "ODIN has no keywords
    /// of its own" — every cADL/BEL keyword reads back as an ODIN identifier.
    #[test]
    fn odin_reserves_no_cadl_or_bel_keyword() {
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
        // `$variable`, `{`, `:` and `%` have no ODIN production at all.
        for refused in ["$v", "{", ":", "%", "^", "!="] {
            assert!(lex_odin(refused).is_err(), "ODIN must refuse {refused:?}");
        }
        // `@` IS an ODIN token: it opens the document prefix
        // `schema_identifier ::= '@' schema '=' URI`
        // (`LANG/docs/odin/master04-odin_artefacts` intro); a misplaced `@`
        // is the parser's refusal, not the lexer's.
        assert_eq!(odin("@"), vec![Token::SymAt]);
    }

    /// `AM/docs/ADL1.4/master04-dadl` §Symbols `V_LOCAL_TERM_CODE_REF` is an
    /// ODIN leaf value; cADL splits the same text into bracket + code.
    #[test]
    fn local_term_code_ref_is_odin_only() {
        assert_eq!(
            odin("[at0200]"),
            vec![Token::LocalTermCodeRef("[at0200]".into())]
        );
        assert_eq!(
            adl("[at0200]"),
            vec![
                Token::LBracket,
                Token::AtCode("at0200".into()),
                Token::RBracket,
            ]
        );
        // integer / date container keys still lex as keys, not local codes.
        assert_eq!(
            odin("[1]"),
            vec![Token::LBracket, Token::Integer("1".into()), Token::RBracket,]
        );
    }

    /// `AM/docs/ADL1.4/master08-adl` §Revision History Section writes
    /// `time_committed = <2004-11-02 09:31:04+1000>`; the ODIN reading accepts
    /// the space form and normalises it to the ISO `T` designator, while cADL
    /// reads the same text as an all-literal constraint pattern.
    #[test]
    fn space_separated_date_time_is_an_odin_widening() {
        assert_eq!(
            odin("2004-11-02 09:31:04+1000"),
            vec![Token::Iso8601DateTime("2004-11-02T09:31:04+1000".into())]
        );
        assert_eq!(
            adl("2004-11-02 09:31:04+1000"),
            vec![
                Token::DateTimeConstraintPattern("2004-11-02 09:31:04".into()),
                Token::SymPlus,
                Token::Integer("1000".into()),
            ]
        );
    }

    /// `LANG/docs/odin/master07-leaf_data` §String Data: multi-line string
    /// contents drop the white-space leaders of the continuation lines — an
    /// ODIN-only transform; cADL and BEL keep the literal verbatim.
    #[test]
    fn multi_line_string_leaders_are_stripped_for_odin_only() {
        let src = "    text = <\"first\n        second\">";
        let stripped = Token::String("\"first\nsecond\"".into());
        let verbatim = Token::String("\"first\n        second\"".into());
        assert!(odin(src).contains(&stripped));
        assert!(adl(src).contains(&verbatim));
    }

    /// `base_expressions.g4` spells BEL's operators case-sensitively, has no
    /// exponent on `INTEGER`, no `ALPHA_UNDERSCORE_ID`, and adds `in` and the
    /// ADL 1.4 `<>` spelling of `SYM_NE`.
    #[test]
    fn bel_keeps_its_own_narrower_lexical_surface() {
        assert_eq!(bel("matches"), vec![Token::SymMatches]);
        assert_eq!(bel("MATCHES"), vec![Token::AlphaUcId("MATCHES".into())]);
        assert_eq!(bel("in"), vec![Token::SymIn]);
        assert_eq!(adl("in"), vec![Token::AlphaLcId("in".into())]);
        assert_eq!(bel("<>"), vec![Token::SymNe]);
        assert_eq!(adl("<>"), vec![Token::SymLt, Token::SymGt]);
        assert_eq!(
            bel("29e6"),
            vec![Token::Integer("29".into()), Token::AlphaLcId("e6".into())]
        );
        assert_eq!(adl("29e6"), vec![Token::Integer("29e6".into())]);
        assert_eq!(bel("TRUE"), vec![Token::AlphaUcId("TRUE".into())]);
        assert_eq!(adl("TRUE"), vec![Token::SymTrue]);
        assert!(lex_bel("_default").is_err());
        // the movable-path leader and the single-segment predicate form.
        assert_eq!(bel("//foo/bar"), vec![Token::AdlPath("//foo/bar".into())]);
        assert_eq!(
            bel("foo[at0001]"),
            vec![Token::AdlPath("foo[at0001]".into())]
        );
        assert_eq!(
            adl("foo[at0001]"),
            vec![
                Token::AlphaLcId("foo".into()),
                Token::LBracket,
                Token::AtCode("at0001".into()),
                Token::RBracket,
            ]
        );
    }

    /// `base_lexer.g4 ADL_PATH` requires a lower-case segment head; `odin.g4`
    /// takes either case, so the two readings split `/Foo/bar` differently.
    #[test]
    fn path_segment_head_case_differs_between_cadl_and_odin() {
        assert_eq!(odin("/Foo/bar"), vec![Token::AdlPath("/Foo/bar".into())]);
        assert_eq!(
            adl("/Foo/bar"),
            vec![
                Token::SymSlash,
                Token::AlphaUcId("Foo".into()),
                Token::AdlPath("/bar".into()),
            ]
        );
    }

    /// A BOM is tolerated by the cADL and ODIN readings and refused by BEL,
    /// exactly as the three stand-alone lexers did.
    #[test]
    fn byte_order_mark_is_language_specific() {
        assert_eq!(
            adl("\u{feff}archetype"),
            vec![Token::AlphaLcId("archetype".into())]
        );
        assert_eq!(
            odin("\u{feff}archetype"),
            vec![Token::AlphaLcId("archetype".into())]
        );
        let error = lex_bel("\u{feff}archetype").expect_err("BEL refuses a BOM");
        assert_eq!(error.span, 0..3);
    }
}
