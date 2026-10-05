// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The ODIN reclassification table of the LANG 1.0.0 generation.
//!
//! [`reclassify`] answers exactly one question: *given a token the shared DFA
//! produced for the source slice `slice`, what would the Release-1.0.0 ODIN
//! lexer (`vendor/grammar/v1_0/{odin.g4, odin_values.g4, base_patterns.g4,
//! base_lexer.g4}`) have produced for that same slice?* — `Some(token)` when
//! ODIN admits the slice as one token (possibly re-tagged), `None` when it
//! does not. `None` sends the caller into [`super::narrow`], which retries
//! shorter prefixes; a slice no prefix of which ODIN admits is an ODIN
//! lexical error.
//!
//! The shared DFA is the workspace token superset (the union the v1_1
//! generation reads under four languages), so a `None` here is never
//! "unsupported": it is always the statement that the Release-1.0.0 ODIN
//! production set does not reach this text. ODIN is the ONE machine-readable
//! syntax LANG 1.0.0 publishes — its EL is DEVELOPMENT prose with no grammar
//! and BEL first appears in 1.1.0 — so this generation carries exactly one
//! reading.

use super::token::Token;

/// What the Release-1.0.0 ODIN lexer would produce for `slice`, or `None`
/// when it admits no single token spanning exactly `slice`.
///
/// `src`/`start` locate the slice in the original source; only the string
/// reading needs them (the multi-line leader budget is a column measurement).
#[expect(
    clippy::match_same_arms,
    reason = "the arms are grouped by lexical family so each carries the grammar production and citation its reading comes from; collapsing equal bodies would erase which production a refusal belongs to"
)]
pub(super) fn reclassify(token: &Token, slice: &str, src: &str, start: usize) -> Option<Token> {
    match token {
        // The BOM is tolerated (and dropped).
        Token::Bom => Some(Token::Bom),

        // ── keywords of the OTHER readings of the shared token superset ──
        // `LANG/docs/odin/master03-basics.adoc` §Keywords: "ODIN has no
        // keywords of its own" — every cADL/BEL/EL keyword the union DFA
        // recognises is an ordinary ODIN identifier, so each is demoted to
        // the identifier its spelling gives.
        Token::SymMatches
        | Token::SymAnd
        | Token::SymOr
        | Token::SymXor
        | Token::SymNot
        | Token::SymImplies
        | Token::SymForAll
        | Token::SymExists
        | Token::SymThereExists
        | Token::SymIn
        | Token::SymThen
        | Token::SymNotMatches
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
        | Token::SymClosed => demote_word(slice),

        // Symbol-only operators of the other readings: no ODIN production and
        // no identifier spelling to demote to.
        Token::SymIff | Token::SymBrokenBar => None,

        // Re-tag-only variants (the EL word keywords): the shared DFA never
        // produces them, and ODIN has no production for them either.
        Token::SymSelf
        | Token::SymResult
        | Token::SymCase
        | Token::SymChoice
        | Token::SymAssert => None,

        // `infinity` is an interval endpoint (`AM/docs/ADL1.4/master04-dadl`
        // §Intervals of Ordered Primitive Types), so it stays a token; where
        // it stands as an attribute NAME the key-position re-tag in
        // [`super::lex_odin`] demotes it.
        Token::SymInfinity => Some(token.clone()),

        // Booleans are case-insensitive (`LANG/docs/odin/master07-leaf_data`
        // §Boolean Data: "Boolean values can be indicated by the following
        // values (case-insensitive)").
        Token::SymTrue | Token::SymFalse => Some(token.clone()),

        // ── the ADL-only token classes with zero ODIN production ──
        // A code with no dot reads back as a plain identifier; a dotted one
        // has no single-token reading at all and is split by the caller.
        Token::RootIdCode(_) | Token::IdCode(_) | Token::AtCode(_) | Token::AcCode(_) => {
            demote_word(slice)
        }
        Token::ArchetypeId(_) | Token::VersionId(_) | Token::Guid(_) => None,
        // `PYMD`/`PWD` are also well-formed uppercase identifiers.
        Token::DateTimeConstraintPattern(_)
        | Token::DateConstraintPattern(_)
        | Token::TimeConstraintPattern(_)
        | Token::DurationConstraintPattern(_) => demote_word(slice),
        Token::VariableWithPath(_) | Token::VariableId(_) => None,

        // ── ISO 8601 values ──
        // The ODIN reading normalises the `AM/docs/ADL1.4/master08-adl`
        // §Revision History space form to the ISO `T` designator, so every
        // consumer sees valid ISO 8601; the `T` forms pass through untouched.
        Token::Iso8601DateTime(text) => Some(Token::Iso8601DateTime(text.replacen(' ', "T", 1))),
        Token::Iso8601Date(_) | Token::Iso8601Time(_) | Token::Iso8601Duration(_) => {
            Some(token.clone())
        }

        // ── composed primitives ──
        // `CONTAINED_REGEXP` is a cADL constraint form; ODIN has no `{`.
        Token::ContainedRegexp(_) | Token::LCurly | Token::RCurly => None,
        // A qualified term code is an ODIN leaf value
        // (`master07-leaf_data` §Terms and Term Codes).
        Token::TermCodeRef(_) => Some(token.clone()),
        Token::EmbeddedUri(_) => Some(token.clone()),
        Token::LocalTermCodeRef(_) => Some(token.clone()),
        // `<# … #>` is the ODIN plug-in-syntax block
        // (`LANG/docs/odin/master09-plug_in_syntaxes`).
        Token::PlugInBlock(_) => Some(token.clone()),
        Token::AdlPath(text) => odin_path(text).then(|| token.clone()),

        // ── atomic primitives ──
        Token::Real(_) | Token::Character(_) | Token::Integer(_) => Some(token.clone()),
        Token::String(text) => Some(Token::String(strip_line_leaders(
            text,
            leader_budget(src, start),
        ))),

        // ── identifiers ──
        Token::AlphaUcId(_) | Token::AlphaLcId(_) => Some(token.clone()),
        // NOTE: the Release-1.0.0 `base_lexer.g4` declares no
        // `ALPHA_UNDERSCORE_ID` — a `_`-initial word is not a 1.0.0 token
        // (the variant exists in the shared superset for the 1.1.0 readings).
        Token::AlphaUnderscoreId(_) => None,

        // ── symbols the ODIN grammar spells ──
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

        // `odin.g4` has neither an assignment nor a bare colon, and no
        // arithmetic/binding operators.
        Token::SymAssignment | Token::SymColon | Token::SymPercent | Token::SymCarat => None,
        // `@` opens the optional document prefix `schema_identifier ::= '@'
        // schema '=' URI` (`LANG/docs/odin/master04-odin_artefacts` intro),
        // so the ODIN reading admits it — a misplaced `@` is the parser's
        // refusal, not the lexer's. (The vendored `odin.g4` start rule lacks
        // the production; the docs text wins.)
        Token::SymAt => Some(token.clone()),
        // `<>` is the ADL 1.4 assertion spelling of `SYM_NE`; under ODIN the
        // two characters are the separate `SYM_LT` `SYM_GT` an empty ODIN
        // block is written with.
        Token::SymNe => None,
        Token::SymListContinue => Some(token.clone()),
        // NOTE: `master07-leaf_data` §Intervals (generation-identical text)
        // defines `|N +/-M|` / `|N±M|` — the docs text wins over the
        // Release-1.0.0 `odin_values.g4`'s missing ± production.
        Token::SymPlusOrMinus => Some(token.clone()),
        // `odin_values.g4` spells the wildcard endpoint as the ASCII `*` only.
        Token::SymStar => (slice == "*").then(|| token.clone()),
    }
}

/// Read `slice` back as the identifier the Release-1.0.0 ODIN lexer would
/// have produced, or `None` when it is not a 1.0.0 identifier.
///
/// The 1.0.0 identifier classes are `ALPHA_UC_ID` and `ALPHA_LC_ID` alone —
/// `base_patterns.g4` `identifier : ALPHA_UC_ID | ALPHA_LC_ID`, and the
/// era's `base_lexer.g4` declares no `_`-initial class.
fn demote_word(slice: &str) -> Option<Token> {
    let mut chars = slice.chars();
    let head = chars.next()?;
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    match head {
        'a'..='z' => Some(Token::AlphaLcId(slice.to_owned())),
        'A'..='Z' => Some(Token::AlphaUcId(slice.to_owned())),
        _ => None,
    }
}

/// Whether the Release-1.0.0 `ADL_PATH` production admits `text`: absolute
/// `(/seg)+` or relative `seg(/seg)+`, every segment head an `ALPHA_LC_ID`
/// (`base_lexer.g4` `PATH_SEGMENT : ALPHA_LC_ID ('[' PATH_ATTRIBUTE ']')?`).
///
/// The v1_1 reading widens the segment head to either case on the ground of
/// its `odin_object_key` production; that production does not exist in this
/// generation (`attribute_id : ALPHA_LC_ID`), so the widening's ground is
/// absent and the lexer rule stands as written.
fn odin_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0usize;
    let mut leading_slashes = 0usize;
    while bytes.get(at) == Some(&b'/') {
        leading_slashes += 1;
        at += 1;
    }
    if leading_slashes > 1 {
        return false;
    }
    let mut segments = 0usize;
    loop {
        match bytes.get(at) {
            Some(head) if head.is_ascii_lowercase() => at += 1,
            _ => return false,
        }
        while matches!(bytes.get(at), Some(c) if c.is_ascii_alphanumeric() || *c == b'_') {
            at += 1;
        }
        if bytes.get(at) == Some(&b'[') {
            at += 1;
            loop {
                match bytes.get(at) {
                    Some(b']') => break,
                    Some(b'\r' | b'\n') | None => return false,
                    Some(_) => at += 1,
                }
            }
            at += 1;
        }
        segments += 1;
        if bytes.get(at) == Some(&b'/') {
            at += 1;
        } else {
            break;
        }
    }
    at == bytes.len()
        && ((leading_slashes == 1 && segments >= 1) || (leading_slashes == 0 && segments >= 2))
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
