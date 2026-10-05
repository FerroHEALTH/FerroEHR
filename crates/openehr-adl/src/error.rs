// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Typed ADL2 syntax errors.
//!
//! [`SyntaxErrorCode`] mirrors 1:1 the openEHR *Syntax Validity Rules* code
//! catalogue — the `S*` codes defined in
//! `docs/specs/openehr/AM/docs/ADL2/master04.6-cadl_validity_rules.adoc`
//! (§Syntax Validity Rules). The full catalogue is present as the error
//! vocabulary for the whole ADL2 front end; the outer/ODIN parser in this
//! crate raises only the subset reachable at the artefact + identification +
//! ODIN-section level. Codes raised only by the cADL definition / rules parser
//! are present-but-unused here by design — the enum is the catalogue, not only
//! the slice this outer parser reaches.

use openehr_lang::v1_1::position::line_col;

/// An ADL2 syntax-error code.
///
/// Each variant's doc comment is the normative gloss verbatim from
/// `ADL2/master04.6-cadl_validity_rules.adoc` §Syntax Validity Rules
/// (message templates there use `$1`/`$2` placeholders; the concrete text is
/// carried in [`SyntaxError::message`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxErrorCode {
    /// `SUNK` — Syntax error (unknown cause).
    Sunk,
    /// `SARID` — Syntax error in artefact identification clause; expecting
    /// archetype id (format = `model_issuer-package-class.concept.version`).
    Sarid,
    /// `SASID` — Syntax error in 'specialise' clause; expecting parent
    /// archetype id (`model_issuer-ref_model-model_class.concept.version`).
    Sasid,
    /// `SACO` — Syntax error in 'concept' clause; expecting `TERM_CODE`
    /// reference.
    Saco,
    /// `SALA` — Syntax error in language section.
    Sala,
    /// `SALAN` — Syntax error: no language section found.
    Salan,
    /// `SADS` — Syntax error in description section.
    Sads,
    /// `SADF` — Syntax error in definition section.
    Sadf,
    /// `SAIV` — Syntax error in invariant section.
    Saiv,
    /// `SAON` — Syntax error in terminology section.
    Saon,
    /// `SAAN` — Syntax error in annotations section.
    Saan,
    /// `SDSF` — Syntax error: differential syntax not allowed in top-level
    /// archetype.
    Sdsf,
    /// `SDINV` — Syntax error: invalid ODIN section; error: `$1`.
    Sdinv,
    /// `SCCOG` — Syntax error: expecting a new node definition, primitive
    /// node definition, 'use' path, or 'archetype' reference.
    Sccog,
    /// `SUAID` — Syntax error: expecting `[archetype_id]` in `use_archetype`
    /// statement.
    Suaid,
    /// `SUAIDI` — Syntax error: invalid archetype id `$1`.
    Suaidi,
    /// `SOCCF` — Syntax error: expecting an 'occurrences expression', e.g.
    /// `occurrences matches {n..m}`.
    Soccf,
    /// `SUNPA` — Syntax error: expecting absolute path in `use_node`
    /// statement.
    Sunpa,
    /// `SCOAT` — Syntax error: expecting attribute definition(s).
    Scoat,
    /// `SUAS` — Syntax error: error after `use_archetype` keyword; expecting
    /// Object node definition.
    Suas,
    /// `SCAS` — Syntax error: expecting a 'any' node, 'leaf' node, or new
    /// node definition.
    Scas,
    /// `SINVS` — Syntax error: illegal invariant expression at identifier
    /// `$1`.
    Sinvs,
    /// `SEXPT` — Syntax error: expecting absolute path after exists keyword.
    Sexpt,
    /// `SEXLSG` — Syntax error: existence single value must be 0 or 1.
    Sexlsg,
    /// `SEXLU1` — Syntax error: existence upper limit must be 0 or 1 when
    /// lower limit is 0.
    Sexlu1,
    /// `SEXLU2` — Syntax error: existence upper limit must be 1 when lower
    /// limit is 1.
    Sexlu2,
    /// `SEXLMG` — Syntax error: existence must be one of `0..0`, `0..1`, or
    /// `1..1`.
    Sexlmg,
    /// `SCIAV` — Syntax error: invalid assumed value; must be an integer.
    Sciav,
    /// `SCRAV` — Syntax error: invalid assumed value; must be a real number.
    Scrav,
    /// `SCDAV` — Syntax error: invalid assumed value; must be an ISO8601
    /// date.
    Scdav,
    /// `SCTAV` — Syntax error: invalid assumed value; must be an ISO8601
    /// time.
    Sctav,
    /// `SCDTAV` — Syntax error: invalid assumed value; must be an ISO8601
    /// date/time.
    Scdtav,
    /// `SCDUAV` — Syntax error: invalid assumed value; must be an ISO8601
    /// duration.
    Scduav,
    /// `SCSAV` — Syntax error: invalid assumed value; must be a string.
    Scsav,
    /// `SCBAV` — Syntax error: invalid assumed value; must be a 'True' or
    /// 'False'.
    Scbav,
    /// `SCOAV` — Syntax error: invalid assumed value; must be an ordinal
    /// integer value.
    Scoav,
    /// `SCDPT` — Syntax error: invalid date constraint pattern `$1`; allowed
    /// patterns: `$2`.
    Scdpt,
    /// `SCTPT` — Syntax error: invalid time constraint pattern `$1`; allowed
    /// patterns: `$2`.
    Sctpt,
    /// `SCDTPT` — Syntax error: invalid date/time constraint pattern `$1`;
    /// allowed patterns: `$2`.
    Scdtpt,
    /// `SCDUPT` — Syntax error: invalid duration constraint pattern `$1`;
    /// legal pattern `P[Y|y][M|m][W|w][D|d][T[H|h][M|m][S|s]]` or
    /// `P[W|w] [/duration_interval]`.
    Scdupt,
    /// `SCSRE` — Syntax error: regular expression compile error `$1` is not a
    /// valid regular expression.
    Scsre,
    /// `STCCP` — Syntax error: invalid term code constraint pattern `$1`:
    /// `$2`.
    Stccp,
    /// `STCDC` — Syntax error: duplicate code(s) found in code list.
    Stcdc,
    /// `STCAC` — Syntax error: assumed value code `$1` not found in code
    /// list.
    Stcac,
    /// `STCNT` — Syntax error: terminology not specified.
    Stcnt,
}

impl SyntaxErrorCode {
    /// The bare mnemonic (e.g. `"SADF"`), as used in the spec catalogue and
    /// the ADL Workbench conformance corpus file names.
    #[must_use]
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::Sunk => "SUNK",
            Self::Sarid => "SARID",
            Self::Sasid => "SASID",
            Self::Saco => "SACO",
            Self::Sala => "SALA",
            Self::Salan => "SALAN",
            Self::Sads => "SADS",
            Self::Sadf => "SADF",
            Self::Saiv => "SAIV",
            Self::Saon => "SAON",
            Self::Saan => "SAAN",
            Self::Sdsf => "SDSF",
            Self::Sdinv => "SDINV",
            Self::Sccog => "SCCOG",
            Self::Suaid => "SUAID",
            Self::Suaidi => "SUAIDI",
            Self::Soccf => "SOCCF",
            Self::Sunpa => "SUNPA",
            Self::Scoat => "SCOAT",
            Self::Suas => "SUAS",
            Self::Scas => "SCAS",
            Self::Sinvs => "SINVS",
            Self::Sexpt => "SEXPT",
            Self::Sexlsg => "SEXLSG",
            Self::Sexlu1 => "SEXLU1",
            Self::Sexlu2 => "SEXLU2",
            Self::Sexlmg => "SEXLMG",
            Self::Sciav => "SCIAV",
            Self::Scrav => "SCRAV",
            Self::Scdav => "SCDAV",
            Self::Sctav => "SCTAV",
            Self::Scdtav => "SCDTAV",
            Self::Scduav => "SCDUAV",
            Self::Scsav => "SCSAV",
            Self::Scbav => "SCBAV",
            Self::Scoav => "SCOAV",
            Self::Scdpt => "SCDPT",
            Self::Sctpt => "SCTPT",
            Self::Scdtpt => "SCDTPT",
            Self::Scdupt => "SCDUPT",
            Self::Scsre => "SCSRE",
            Self::Stccp => "STCCP",
            Self::Stcdc => "STCDC",
            Self::Stcac => "STCAC",
            Self::Stcnt => "STCNT",
        }
    }
}

impl std::fmt::Display for SyntaxErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.mnemonic())
    }
}

/// A syntax error located in the source text.
///
/// Carries the catalogue [`code`](SyntaxError::code), a human-readable
/// [`message`](SyntaxError::message), the byte [`span`](SyntaxError::span),
/// and the 1-based [`line`](SyntaxError::line) / [`column`](SyntaxError::column)
/// of the offending input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code} at line {line}, column {column}: {message}")]
pub struct SyntaxError {
    /// The `S*` catalogue code.
    pub code: SyntaxErrorCode,
    /// A concrete, human-readable message (the resolved form of the
    /// catalogue's `$1`/`$2` template where applicable).
    pub message: String,
    /// 1-based line number of the offending input.
    pub line: usize,
    /// 1-based column number of the offending input.
    pub column: usize,
    /// Byte range of the offending input in the original source.
    pub span: std::ops::Range<usize>,
}

impl SyntaxError {
    /// Build a [`SyntaxError`] for `code`/`message` at byte `span`, resolving
    /// the 1-based line/column against `src`.
    #[must_use]
    pub fn at(
        code: SyntaxErrorCode,
        message: impl Into<String>,
        span: std::ops::Range<usize>,
        src: &str,
    ) -> Self {
        let (line, column) = line_col(src, span.start);
        Self {
            code,
            message: message.into(),
            line,
            column,
            span,
        }
    }
}

/// Turn a shared-lexer failure into the catalogue's lexical error.
///
/// The `S*` code space is a verbatim 1:1 mirror of the openEHR catalogue
/// (`ADL2/master04.6-cadl_validity_rules.adoc` §Syntax Validity Rules) and
/// carries no code for a lexical defect, so every lexical failure reports
/// under `SUNK` ("Syntax error (unknown cause)") and names the offending input
/// in the message.
pub(crate) fn lexical(failure: &openehr_lang::v1_1::lexer::LexError, src: &str) -> SyntaxError {
    SyntaxError::at(
        SyntaxErrorCode::Sunk,
        format!("unrecognised token {:?}", failure.text),
        failure.span.clone(),
        src,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonics_are_unique_and_count_45() {
        let all = [
            SyntaxErrorCode::Sunk,
            SyntaxErrorCode::Sarid,
            SyntaxErrorCode::Sasid,
            SyntaxErrorCode::Saco,
            SyntaxErrorCode::Sala,
            SyntaxErrorCode::Salan,
            SyntaxErrorCode::Sads,
            SyntaxErrorCode::Sadf,
            SyntaxErrorCode::Saiv,
            SyntaxErrorCode::Saon,
            SyntaxErrorCode::Saan,
            SyntaxErrorCode::Sdsf,
            SyntaxErrorCode::Sdinv,
            SyntaxErrorCode::Sccog,
            SyntaxErrorCode::Suaid,
            SyntaxErrorCode::Suaidi,
            SyntaxErrorCode::Soccf,
            SyntaxErrorCode::Sunpa,
            SyntaxErrorCode::Scoat,
            SyntaxErrorCode::Suas,
            SyntaxErrorCode::Scas,
            SyntaxErrorCode::Sinvs,
            SyntaxErrorCode::Sexpt,
            SyntaxErrorCode::Sexlsg,
            SyntaxErrorCode::Sexlu1,
            SyntaxErrorCode::Sexlu2,
            SyntaxErrorCode::Sexlmg,
            SyntaxErrorCode::Sciav,
            SyntaxErrorCode::Scrav,
            SyntaxErrorCode::Scdav,
            SyntaxErrorCode::Sctav,
            SyntaxErrorCode::Scdtav,
            SyntaxErrorCode::Scduav,
            SyntaxErrorCode::Scsav,
            SyntaxErrorCode::Scbav,
            SyntaxErrorCode::Scoav,
            SyntaxErrorCode::Scdpt,
            SyntaxErrorCode::Sctpt,
            SyntaxErrorCode::Scdtpt,
            SyntaxErrorCode::Scdupt,
            SyntaxErrorCode::Scsre,
            SyntaxErrorCode::Stccp,
            SyntaxErrorCode::Stcdc,
            SyntaxErrorCode::Stcac,
            SyntaxErrorCode::Stcnt,
        ];
        assert_eq!(all.len(), 45);
        let mut seen = std::collections::HashSet::new();
        for c in all {
            assert!(seen.insert(c.mnemonic()), "duplicate mnemonic {c}");
        }
        assert_eq!(seen.len(), 45);
    }

    /// A defect's reported position comes from the shared
    /// [`openehr_lang::v1_1::position::line_col`] (whose own tests pin the
    /// arithmetic); what this crate owns is that `SyntaxError::at` resolves the
    /// SPAN START against the source it was handed.
    #[test]
    fn a_syntax_error_reports_the_span_start_as_a_line_and_column() {
        let src = "ab\ncdé/f";
        let slash = src.find('/').expect("the fixture contains a slash");
        let err = SyntaxError::at(SyntaxErrorCode::Sunk, "x", slash..slash + 1, src);
        assert_eq!((err.line, err.column), (2, 4));
    }
}
