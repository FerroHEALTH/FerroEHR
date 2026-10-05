// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The per-language reclassification tables.
//!
//! [`reclassify`] answers exactly one question: *given a token the shared DFA
//! produced for the source slice `slice`, what would `language`'s own lexer
//! have produced for that same slice?* — `Some(token)` when the language
//! admits the slice as one token (possibly re-tagged), `None` when it does
//! not. `None` sends the caller into [`super::narrow`], which retries shorter
//! prefixes; a slice no prefix of which the language admits is that language's
//! lexical error.
//!
//! Because the shared DFA is the UNION of the four lexical layers, a `None`
//! here is never "unsupported": it is always the statement that the language's
//! own production set does not reach this text, which is precisely what its
//! stand-alone lexer expressed by not having the rule.

use super::token::Token;

/// Which language's lexical layer a token stream is being read under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Language {
    /// The ADL2 outer + cADL reading (`AM/docs/ADL2/`, `adl2.g4`/`cadl2.g4`).
    Adl,
    /// The ODIN reading (`LANG/docs/odin/`, `odin.g4`/`odin_values.g4`).
    Odin,
    /// The BEL reading (`LANG/docs/BEL/`, `base_expressions.g4`).
    Bel,
    /// The Expression Language reading (`LANG/docs/EL/`, `ElLexer.g4`).
    ///
    /// `ElLexer.g4` is an `import Cadl2Lexer, SymbolsLexer, GeneralIdsLexer`
    /// with its own rules layered on top, and ANTLR gives the importing
    /// grammar's rule precedence — so this reading is the cADL one wherever
    /// `ElLexer.g4` declares nothing, and `ElLexer.g4`'s own (case-SENSITIVE)
    /// spelling wherever it does.
    El,
}

/// What `language`'s own lexer would produce for `slice`, or `None` when it
/// admits no single token spanning exactly `slice`.
///
/// `src`/`start` locate the slice in the original source; only the ODIN string
/// reading needs them (the multi-line leader budget is a column measurement).
#[expect(
    clippy::too_many_lines,
    reason = "one exhaustive match over the whole token superset; splitting it would forfeit the compiler's proof that every token has a per-language reading"
)]
#[expect(
    clippy::match_same_arms,
    reason = "the arms are grouped by lexical family so each carries the grammar production and citation its reading comes from; collapsing equal bodies would erase which production a refusal belongs to"
)]
pub(super) fn reclassify(
    language: Language,
    token: &Token,
    slice: &str,
    src: &str,
    start: usize,
) -> Option<Token> {
    match token {
        // ── the BOM: tolerated (and dropped) by ADL/ODIN, refused by BEL/EL ──
        Token::Bom => match language {
            Language::Adl | Language::Odin => Some(Token::Bom),
            Language::Bel | Language::El => None,
        },

        // ── keywords shared by the cADL, BEL and EL layers ──
        // BEL's `base_expressions.g4` and EL's `ElLexer.g4` both spell these
        // case-SENSITIVELY, so a differently-cased spelling stays an
        // identifier there.
        Token::SymMatches => shared_keyword(
            language,
            token,
            slice,
            &["matches", "is_in", "\u{2208}"],
            &["matches", "is_in", "\u{2208}"],
        ),
        Token::SymAnd => shared_keyword(
            language,
            token,
            slice,
            &["and", "\u{2227}"],
            &["and", "AND", "\u{2227}"],
        ),
        Token::SymOr => shared_keyword(
            language,
            token,
            slice,
            &["or", "\u{2228}"],
            &["or", "OR", "\u{2228}"],
        ),
        // `⊻` is not in `ElLexer.g4` `SYM_XOR`; the EL Logical Operators table
        // (`LANG/docs/EL/master05-expressions.adoc` §Primitive Operators)
        // lists it as the symbol form, and the docs text is the oracle.
        Token::SymXor => shared_keyword(
            language,
            token,
            slice,
            &["xor"],
            &["xor", "XOR", "\u{22BB}"],
        ),
        Token::SymNot => shared_keyword(
            language,
            token,
            slice,
            &["not", "~", "\u{223C}", "\u{00AC}", "!"],
            &["not", "NOT", "!", "~", "\u{00AC}"],
        ),
        Token::SymImplies => shared_keyword(
            language,
            token,
            slice,
            &["implies", "\u{00AE}", "->"],
            &["implies", "\u{21D2}", "\u{2192}"],
        ),
        Token::SymForAll => shared_keyword(
            language,
            token,
            slice,
            &["for_all", "\u{2200}"],
            &["for_all", "\u{2200}"],
        ),
        Token::SymExists => {
            shared_keyword(language, token, slice, &["exists"], &["exists", "\u{25A1}"])
        }
        Token::SymThereExists => shared_keyword(
            language,
            token,
            slice,
            &["there_exists", "\u{2203}"],
            &["there_exists", "\u{2203}"],
        ),

        // ── the BEL/EL quantifier binding keyword ──
        Token::SymIn => match language {
            Language::Bel => Some(token.clone()),
            Language::El => (slice == "in").then(|| token.clone()),
            Language::Adl | Language::Odin => demote_word(language, slice),
        },

        // ── EL-only operators (`ElLexer.g4` `SYM_IFF`, `SYM_BROKEN_BAR`) ──
        Token::SymIff | Token::SymBrokenBar => match language {
            Language::El => Some(token.clone()),
            Language::Adl | Language::Odin | Language::Bel => None,
        },

        // ── the EL-only word keywords, re-tagged off the identifier the
        //    shared DFA produced (`ElLexer.g4` `SYM_SELF`/`SYM_RESULT`/
        //    `SYM_CASE`/`SYM_CHOICE`/`SYM_ASSERT`) ──
        Token::SymSelf
        | Token::SymResult
        | Token::SymCase
        | Token::SymChoice
        | Token::SymAssert => None,

        // ── cADL-only keywords: identifiers everywhere else ──
        Token::SymNotMatches
        | Token::SymOccurrences
        | Token::SymExistence
        | Token::SymCardinality
        | Token::SymOrdered
        | Token::SymUnordered
        | Token::SymUnique
        | Token::SymUseNode
        | Token::SymUseArchetype
        | Token::SymAllowArchetype
        | Token::SymInclude
        | Token::SymExclude
        | Token::SymAfter
        | Token::SymBefore
        | Token::SymClosed => match language {
            Language::Adl => Some(token.clone()),
            // EL reserves only what `ElLexer.g4` itself declares. These are
            // cADL constraint keywords, reachable in EL only inside the
            // `matches { … }` block `ElParser.g4` delegates to `Cadl2Parser` —
            // which this reader captures verbatim — so in EL expression
            // position each is an ordinary feature name.
            Language::Odin | Language::Bel | Language::El => demote_word(language, slice),
        },
        // `ElLexer.g4` re-declares `SYM_THEN : 'then' | 'THEN'`, so EL takes
        // exactly those two spellings.
        Token::SymThen => shared_keyword(language, token, slice, &[], &["then", "THEN"]),

        // `infinity` is an interval endpoint in cADL and in ODIN
        // (`AM/docs/ADL1.4/master04-dadl` §Intervals of Ordered Primitive
        // Types); `base_expressions.g4` has no such keyword.
        Token::SymInfinity => match language {
            Language::Adl | Language::Odin => Some(token.clone()),
            Language::Bel | Language::El => demote_word(language, slice),
        },

        // Booleans are case-insensitive in cADL and ODIN; BEL spells only the
        // two capitalisations its grammar lists.
        Token::SymTrue => shared_boolean(language, token, slice, &["true", "True"]),
        Token::SymFalse => shared_boolean(language, token, slice, &["false", "False"]),

        // ── the ADL-only token classes with zero ODIN/BEL production ──
        // A code with no dot reads back as a plain identifier; a dotted one
        // has no single-token reading at all and is split by the caller.
        Token::RootIdCode(_) | Token::IdCode(_) | Token::AtCode(_) | Token::AcCode(_) => {
            match language {
                Language::Adl => Some(token.clone()),
                Language::Odin | Language::Bel | Language::El => demote_word(language, slice),
            }
        }
        Token::ArchetypeId(_) | Token::VersionId(_) | Token::Guid(_) => match language {
            Language::Adl => Some(token.clone()),
            Language::Odin | Language::Bel | Language::El => None,
        },
        Token::DateTimeConstraintPattern(_)
        | Token::DateConstraintPattern(_)
        | Token::TimeConstraintPattern(_)
        | Token::DurationConstraintPattern(_) => match language {
            Language::Adl => Some(token.clone()),
            // `PYMD`/`PWD` are also well-formed uppercase identifiers.
            Language::Odin | Language::Bel | Language::El => demote_word(language, slice),
        },
        // `ElLexer.g4` `BOUND_VARIABLE_ID : '$' LC_ID` has no path suffix.
        Token::VariableWithPath(_) => match language {
            Language::Adl | Language::Bel => Some(token.clone()),
            Language::Odin | Language::El => None,
        },
        Token::VariableId(_) => match language {
            Language::Adl | Language::Bel | Language::El => Some(token.clone()),
            Language::Odin => None,
        },

        // ── ISO 8601 values ──
        Token::Iso8601DateTime(text) => iso_date_time(language, text),
        Token::Iso8601Date(text) => match language {
            Language::Adl | Language::Odin | Language::El => Some(token.clone()),
            // `base_expressions.g4` `ISO8601_DATE` is the complete
            // `yyyy-mm-dd` form only — no `??` fields, no year-month form.
            Language::Bel => (text.len() == 10 && !text.contains('?')).then(|| token.clone()),
        },
        Token::Iso8601Time(text) => match language {
            Language::Adl | Language::Odin | Language::El => Some(token.clone()),
            // BEL's `ISO8601_TIME` is the union form minus the `??` fields.
            Language::Bel => (!text.contains('?')).then(|| token.clone()),
        },
        Token::Iso8601Duration(_) => Some(token.clone()),

        // ── composed primitives ──
        // `CONTAINED_REGEXP` is a cADL/BEL constraint form; ODIN has no `{`.
        Token::ContainedRegexp(_) | Token::LCurly | Token::RCurly => match language {
            Language::Adl | Language::Bel | Language::El => Some(token.clone()),
            Language::Odin => None,
        },
        // A qualified term code is a cADL primitive, an ODIN leaf value, AND
        // a BEL literal: `LANG/docs/BEL/master03-language.adoc` §Literals
        // lists `Terminology_code` among the BEL primitive literal types, and
        // the BEL grammar reaches `TERM_CODE_REF` through its `odin_values`
        // import (`constant_declaration`'s `primitive_object`).
        Token::TermCodeRef(_) => Some(token.clone()),
        // An embedded URI has no `base_expressions.g4` production and the
        // §Literals `Uri` row shows a BARE URI (lexically unproductive in
        // expression text) — the BEL boundary.
        Token::EmbeddedUri(_) => match language {
            Language::Adl | Language::Odin => Some(token.clone()),
            Language::Bel | Language::El => None,
        },
        // `ElLexer.g4` `LOCAL_TERM_CODE_REF : '[' ALPHANUM_US_CHAR+ ']'` is
        // narrower than the ODIN form, which also admits `.` and `-`.
        Token::LocalTermCodeRef(_) => match language {
            Language::Odin => Some(token.clone()),
            Language::El => is_local_term_code(slice).then(|| token.clone()),
            Language::Adl | Language::Bel => None,
        },
        // `<# … #>` is the ODIN plug-in-syntax block
        // (`LANG/docs/odin/master09-plug_in_syntaxes`); neither the ADL nor
        // the BEL grammar has any `#` production, so both refuse it.
        Token::PlugInBlock(_) => match language {
            Language::Odin => Some(token.clone()),
            Language::Adl | Language::Bel | Language::El => None,
        },
        Token::AdlPath(text) => path(language, text).then(|| token.clone()),

        // ── atomic primitives ──
        Token::Real(_) | Token::Character(_) => Some(token.clone()),
        Token::Integer(text) => match language {
            Language::Adl | Language::Odin | Language::El => Some(token.clone()),
            // `base_expressions.g4` `INTEGER : DIGIT+` — no exponent suffix.
            Language::Bel => (!text.contains(['e', 'E'])).then(|| token.clone()),
        },
        Token::String(text) => match language {
            Language::Adl | Language::Bel | Language::El => Some(token.clone()),
            Language::Odin => Some(Token::String(strip_line_leaders(
                text,
                leader_budget(src, start),
            ))),
        },

        // ── identifiers ──
        // Under EL the five word keywords `ElLexer.g4` declares over the
        // imported id layer are re-tagged here, exactly as the ODIN pass
        // demotes a keyword it does not reserve.
        Token::AlphaUcId(_) | Token::AlphaLcId(_) => match language {
            Language::El => Some(el_word(slice, token)),
            Language::Adl | Language::Odin | Language::Bel => Some(token.clone()),
        },
        // `base_expressions.g4` has no `ALPHA_UNDERSCORE_ID`, and neither the
        // EL nor the cADL parser reaches one.
        Token::AlphaUnderscoreId(_) => match language {
            Language::Adl | Language::Odin => Some(token.clone()),
            Language::Bel | Language::El => None,
        },

        // ── symbols every layer shares ──
        Token::LParen
        | Token::RParen
        | Token::LBracket
        | Token::RBracket
        | Token::SymComma
        | Token::SymSemiColon
        | Token::SymEq
        | Token::SymLe
        | Token::SymGe
        | Token::SymGt
        | Token::SymLt
        | Token::SymIvlSep
        | Token::SymDot
        | Token::SymIvlDelim
        | Token::SymPlus
        | Token::SymMinus
        | Token::SymSlash => Some(token.clone()),

        // ── symbols only some layers carry ──
        // `odin.g4` has neither an assignment nor a bare colon, and no
        // arithmetic/binding operators.
        Token::SymAssignment | Token::SymColon | Token::SymPercent | Token::SymCarat => {
            match language {
                Language::Adl | Language::Bel => Some(token.clone()),
                // `ElLexer.g4` `SYM_ASSIGNMENT : ':='` — no `::=` spelling.
                Language::El => (slice != "::=").then(|| token.clone()),
                Language::Odin => None,
            }
        }
        // `@` opens the optional document prefix `schema_identifier ::= '@'
        // schema '=' URI` (`LANG/docs/odin/master04-odin_artefacts` intro),
        // so the ODIN reading admits it — a misplaced `@` is the parser's
        // refusal, not the lexer's. (The vendored `odin.g4` start rule lacks
        // the production; the docs text wins.)
        Token::SymAt => Some(token.clone()),
        // `<>` is the ADL 1.4 assertion spelling of `SYM_NE` and is admitted
        // only by BEL; under cADL and ODIN the two characters are the separate
        // `SYM_LT` `SYM_GT` an empty ODIN block is written with.
        Token::SymNe => match language {
            Language::Adl | Language::El => (slice != "<>").then(|| token.clone()),
            Language::Bel => Some(token.clone()),
            Language::Odin => None,
        },
        // `SYM_LIST_CONTINUE` and `SYM_PLUS_OR_MINUS` are ODIN/cADL value
        // syntax with no BEL production.
        Token::SymListContinue | Token::SymPlusOrMinus => match language {
            Language::Adl | Language::Odin | Language::El => Some(token.clone()),
            Language::Bel => None,
        },
        // `odin_values.g4` spells the wildcard endpoint as the ASCII `*` only.
        Token::SymStar => match language {
            Language::Adl | Language::Bel | Language::El => Some(token.clone()),
            Language::Odin => (slice == "*").then(|| token.clone()),
        },
    }
}

/// A keyword the cADL and BEL layers share: kept as-is under cADL (whose
/// `adl_keywords.g4` spells every keyword case-insensitively), kept under BEL
/// only for the exact spellings `base_expressions.g4` lists, and demoted to an
/// identifier under ODIN.
///
/// `LANG/docs/odin/master03-basics.adoc` §Keywords: "ODIN has no keywords of
/// its own" — so every one of these words is an ordinary ODIN identifier.
fn shared_keyword(
    language: Language,
    token: &Token,
    slice: &str,
    bel_spellings: &[&str],
    el_spellings: &[&str],
) -> Option<Token> {
    match language {
        Language::Adl => Some(token.clone()),
        Language::Bel | Language::El => {
            let spellings = if language == Language::Bel {
                bel_spellings
            } else {
                el_spellings
            };
            if spellings.contains(&slice) {
                Some(token.clone())
            } else {
                demote_word(language, slice)
            }
        }
        Language::Odin => demote_word(language, slice),
    }
}

/// The EL reading of an identifier slice: one of the five word keywords
/// `ElLexer.g4` declares over the imported id layer, or the identifier itself.
fn el_word(slice: &str, token: &Token) -> Token {
    match slice {
        "Self" => Token::SymSelf,
        "Result" => Token::SymResult,
        "case" => Token::SymCase,
        "choice" => Token::SymChoice,
        "assert" => Token::SymAssert,
        _ => token.clone(),
    }
}

/// Whether `slice` is an `ElLexer.g4` `LOCAL_TERM_CODE_REF`, i.e. `'['
/// ALPHANUM_US_CHAR+ ']'` — no `.` and no `-`, unlike the ODIN form.
fn is_local_term_code(slice: &str) -> bool {
    let Some(body) = slice
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// `SYM_TRUE`/`SYM_FALSE`: case-insensitive under cADL (`base_lexer.g4`) and
/// ODIN (`LANG/docs/odin/master07-leaf_data` §Boolean Data, "Boolean values
/// can be indicated by the following values (case-insensitive)"), but spelled
/// out as two literals by `base_expressions.g4`.
fn shared_boolean(
    language: Language,
    token: &Token,
    slice: &str,
    bel_spellings: &[&str],
) -> Option<Token> {
    match language {
        Language::Adl | Language::Odin | Language::El => Some(token.clone()),
        Language::Bel => {
            if bel_spellings.contains(&slice) {
                Some(token.clone())
            } else {
                demote_word(language, slice)
            }
        }
    }
}

/// The `ISO8601_DATE_TIME` reading per language.
fn iso_date_time(language: Language, text: &str) -> Option<Token> {
    match language {
        // cADL has no space-separated form; where the all-literal text is
        // also a legal `DATE_TIME_CONSTRAINT_PATTERN` that is the reading
        // `base_lexer.g4` + `ADL1.4/master05-cadl.adoc` §Symbols L1422 give.
        Language::Adl => {
            if text.contains(' ') {
                is_date_time_constraint_pattern(text)
                    .then(|| Token::DateTimeConstraintPattern(text.to_owned()))
            } else {
                Some(Token::Iso8601DateTime(text.to_owned()))
            }
        }
        // The ODIN reading normalises the `master08-adl` §Revision History
        // space form to the ISO `T` designator, so every consumer sees valid
        // ISO 8601; the `T` forms pass through untouched.
        Language::Odin => Some(Token::Iso8601DateTime(text.replacen(' ', "T", 1))),
        // BEL's `ISO8601_DATE_TIME` is the union form minus the `??` partials
        // and minus the space separator.
        Language::Bel => {
            (!text.contains(['?', ' '])).then(|| Token::Iso8601DateTime(text.to_owned()))
        }
        // EL inherits the cADL date-time layer but has no space-separated
        // form: `ElParser.g4` `elArithmeticValue` reaches only `dateTimeValue`.
        Language::El => (!text.contains(' ')).then(|| Token::Iso8601DateTime(text.to_owned())),
    }
}

/// Whether `text` is a `DATE_TIME_CONSTRAINT_PATTERN`
/// (`ADL1.4/master05-cadl.adoc` §Symbols L1422, with the literal-field,
/// ASCII-timezone and `[T ]`-separator widenings the chapter grants).
///
/// NOTE: This is the LEXICAL shape only — the token's regex, needed here
/// because the space-separated ODIN date-time widening overlaps it and the
/// cADL reading must recover the pattern token at the same span. It is not a
/// second home for pattern VALIDITY: the `master04.5` valid-pattern tables
/// (field degradation `??` → `XX`, designator order, where a timezone
/// modifier is admitted) are a parser-level check that raises `S*` codes, and
/// they stay in the cADL parser.
fn is_date_time_constraint_pattern(text: &str) -> bool {
    let mut rest = text;
    let fields: [&[&str]; 6] = [
        &["yyyy", "YYYY", "yyy", "YYY"],
        &["mm", "MM", "??", "XX", "xx"],
        &["dd", "DD", "??", "XX", "xx"],
        &["hh", "HH", "??", "XX", "xx"],
        &["mm", "MM", "??", "XX", "xx"],
        &["ss", "SS", "??", "XX", "xx"],
    ];
    let separators = ['-', '-', ' ', ':', ':'];
    let digits = [4usize, 2, 2, 2, 2, 2];
    for (index, spellings) in fields.iter().enumerate() {
        let width = digits.get(index).copied().unwrap_or(2);
        rest = match take_field(rest, spellings, width) {
            Some(r) => r,
            None => return false,
        };
        if let Some(&separator) = separators.get(index) {
            // The date/time separator accepts both `T` and a space.
            let accepted: &[char] = if index == 2 {
                &['T', ' ']
            } else {
                &[separator]
            };
            match rest.strip_prefix(accepted) {
                Some(r) => rest = r,
                None => return false,
            }
        }
    }
    timezone_suffix_is_pattern(rest)
}

/// Consume one constraint-pattern field: one of `spellings`, or `width`
/// literal digits (`master05` L894 — "the 'yyyy' etc match strings can be
/// replaced by literal date/time numbers").
fn take_field<'a>(rest: &'a str, spellings: &[&str], width: usize) -> Option<&'a str> {
    for spelling in spellings {
        if let Some(tail) = rest.strip_prefix(spelling) {
            return Some(tail);
        }
    }
    let head = rest.get(..width)?;
    if head.bytes().all(|b| b.is_ascii_digit()) {
        rest.get(width..)
    } else {
        None
    }
}

/// Whether `rest` is a legal constraint-pattern timezone modifier: empty,
/// `Z`, or `±hh`/`±hh:mm`/`±hhmm` with `+`, `-` or `±` (master05 §Patterns
/// L852 + the `<<timezone_constraints>>` table L900-906).
fn timezone_suffix_is_pattern(rest: &str) -> bool {
    if rest.is_empty() || rest == "Z" {
        return true;
    }
    let Some(tail) = rest
        .strip_prefix('+')
        .or_else(|| rest.strip_prefix('-'))
        .or_else(|| rest.strip_prefix('\u{00B1}'))
    else {
        return false;
    };
    let Some(tail) = tail.strip_prefix("hh").or_else(|| tail.strip_prefix("HH")) else {
        return false;
    };
    let tail = tail.strip_prefix(':').unwrap_or(tail);
    tail.is_empty() || tail == "mm" || tail == "MM"
}

/// Read `slice` back as the identifier the language's own lexer would have
/// produced, or `None` when it is not an identifier there.
///
/// `base_expressions.g4` has no `ALPHA_UNDERSCORE_ID`, so a `_`-initial word
/// is not a BEL token at all.
fn demote_word(language: Language, slice: &str) -> Option<Token> {
    let mut chars = slice.chars();
    let head = chars.next()?;
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    match head {
        'a'..='z' => Some(Token::AlphaLcId(slice.to_owned())),
        'A'..='Z' => Some(Token::AlphaUcId(slice.to_owned())),
        '_' if !matches!(language, Language::Bel | Language::El) => {
            Some(Token::AlphaUnderscoreId(slice.to_owned()))
        }
        _ => None,
    }
}

/// The measured shape of an `ADL_PATH` slice.
struct PathShape {
    /// How many `/` characters lead the path (0, 1, or the BEL movable `2`).
    leading_slashes: usize,
    /// How many `segment ('[' predicate ']')?` segments follow.
    segments: usize,
    /// Whether every segment head is lower-case (the cADL/BEL requirement).
    all_heads_lower: bool,
    /// Whether the FIRST segment carries a `[predicate]`.
    first_has_predicate: bool,
}

/// Whether `language`'s own `ADL_PATH` production admits `text`.
fn path(language: Language, text: &str) -> bool {
    let Some(shape) = path_shape(text) else {
        return false;
    };
    match language {
        // `base_lexer.g4 ADL_PATH`: absolute `(/seg)+` or relative
        // `seg(/seg)+`, every `ADL_PATH_SEGMENT` head an `ALPHA_LC_ID`.
        Language::Adl => {
            shape.all_heads_lower
                && ((shape.leading_slashes == 1 && shape.segments >= 1)
                    || (shape.leading_slashes == 0 && shape.segments >= 2))
        }
        // The ODIN reading takes either case on the segment head — a
        // docs-text-grounded widening over `base_lexer.g4`'s lower-case-only
        // `ADL_PATH_SEGMENT`: ODIN object keys may be upper-case or `_`-initial
        // identifiers (`odin.g4` `odin_object_key`), and every node is reachable
        // by a path (`LANG/docs/odin/master02-overview`), so a path must be able
        // to name an upper-case-keyed attribute. (`_`-initial segment HEADS
        // remain un-lexable in every language — a spec-internal gap between the
        // object-key and path-segment grammars.)
        Language::Odin => {
            (shape.leading_slashes == 1 && shape.segments >= 1)
                || (shape.leading_slashes == 0 && shape.segments >= 2)
        }
        // BEL adds the movable-path leader `//…` (ADL1.4 `master07-paths.adoc`
        // §Grammar `movable_path: SYM_MOVABLE_LEADER relative_path`) and the
        // single-segment-with-predicate form (`relative_path: path_segment`).
        Language::Bel => {
            shape.all_heads_lower
                && ((shape.leading_slashes >= 1
                    && shape.leading_slashes <= 2
                    && shape.segments >= 1)
                    || (shape.leading_slashes == 0 && shape.segments >= 2)
                    || (shape.leading_slashes == 0
                        && shape.segments == 1
                        && shape.first_has_predicate))
        }
        // `ElParser.g4` has no path production at all — a dotted feature chain
        // is `elScopedFeatureRef`, lexed as separate ids and `'.'`.
        Language::El => false,
    }
}

/// Measure an `ADL_PATH` slice, or `None` when it is not a well-formed path.
fn path_shape(text: &str) -> Option<PathShape> {
    let bytes = text.as_bytes();
    let mut at = 0usize;
    let mut leading_slashes = 0usize;
    while bytes.get(at) == Some(&b'/') {
        leading_slashes += 1;
        at += 1;
    }
    let mut segments = 0usize;
    let mut all_heads_lower = true;
    let mut first_has_predicate = false;
    loop {
        match bytes.get(at) {
            Some(head) if head.is_ascii_alphabetic() => {
                if !head.is_ascii_lowercase() {
                    all_heads_lower = false;
                }
                at += 1;
            }
            _ => return None,
        }
        while matches!(bytes.get(at), Some(c) if c.is_ascii_alphanumeric() || *c == b'_') {
            at += 1;
        }
        let mut has_predicate = false;
        if bytes.get(at) == Some(&b'[') {
            at += 1;
            loop {
                match bytes.get(at) {
                    Some(b']') => break,
                    Some(b'\r' | b'\n') | None => return None,
                    Some(_) => at += 1,
                }
            }
            at += 1;
            has_predicate = true;
        }
        if segments == 0 {
            first_has_predicate = has_predicate;
        }
        segments += 1;
        if bytes.get(at) == Some(&b'/') {
            at += 1;
        } else {
            break;
        }
    }
    (at == bytes.len()).then_some(PathShape {
        leading_slashes,
        segments,
        all_heads_lower,
        first_has_predicate,
    })
}

/// How many leading white-space characters may be removed from each
/// continuation line of a multi-line string: the column at which the string's
/// first line of content starts.
///
/// `LANG/docs/odin/master07-leaf_data` §String Data (verbatim in
/// `AM/docs/ADL1.4/master04-dadl` §String Data): "The exact contents of the
/// string are computed as being the characters between the double quote
/// characters, with the removal of white space leaders up to the left-most
/// character of the first line of the string." The left-most character of the
/// string's first line is the one immediately after the opening `"`, so its
/// column is the budget.
fn leader_budget(src: &str, start: usize) -> usize {
    let Some(before) = src.get(..start) else {
        return 0;
    };
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    // +1 for the opening quote itself.
    before
        .get(line_start..)
        .map_or(0, |indent| indent.chars().count() + 1)
}

/// Remove up to `budget` leading white-space characters from every line of
/// `raw` after the first (see [`leader_budget`]). Single-line strings are
/// returned untouched.
///
/// NOTE: The stripping runs AFTER the shared lexer has validated the raw
/// literal's `master03` escapes, and cannot change that verdict: the removed
/// run always begins immediately after a newline, so the character preceding
/// it is never a backslash and no escape sequence can be created or destroyed.
fn strip_line_leaders(raw: &str, budget: usize) -> String {
    if !raw.contains('\n') {
        return raw.to_owned();
    }
    let mut out = String::with_capacity(raw.len());
    for (index, line) in raw.split_inclusive('\n').enumerate() {
        if index == 0 {
            out.push_str(line);
            continue;
        }
        let kept = line
            .char_indices()
            .take(budget)
            .take_while(|(_, c)| *c == ' ' || *c == '\t')
            .last()
            .map_or(0, |(i, c)| i + c.len_utf8());
        out.push_str(line.get(kept..).unwrap_or(line));
    }
    out
}
