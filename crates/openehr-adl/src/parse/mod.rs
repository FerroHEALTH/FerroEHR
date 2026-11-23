// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cADL definition-section parser: recursive descent over the shared token
//! stream, transcribed from `vendor/grammar/v2_4/{cadl2.g4, cadl2_primitives.g4}`
//! into the generated AOM2 constraint model.

pub mod default_value;
pub mod parser;
pub mod patterns;
pub mod primitives;
pub mod refs;
pub mod values;

use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;

use crate::aom::build::{ObjectNode, cobject_to_primitive};
use crate::codes::NodeCoding;
use crate::error::{SyntaxError, SyntaxErrorCode};
use crate::odin::regex_inner;
use crate::parse::primitives::split_contained_regexp;
use openehr_lang::nesting::Nesting;
use openehr_lang::v1_1::lexer::{Spanned, Token};

/// Internal parse result: `Err(())` signals a bail-out; the concrete
/// [`SyntaxError`] is already recorded in [`Parser::errors`].
pub(crate) type PResult<T> = Result<T, ()>;

/// Which ADL dialect the cADL parser accepts.
///
/// `Adl14` adds the 1.4-only terminology and inline-dADL domain forms and
/// refuses the constructs ADL 2 introduced (`use_archetype`, `closed`,
/// `_default`, tuples, term-constraint strengths, `@terminology` bindings) with
/// a typed [`SyntaxError`], since the cADL 1.4 keyword set is closed
/// (`ADL1.4/master05-cadl.adoc` §Keywords).
///
/// NOTE: no openEHR spec governs 1.4→2 conversion; the additive half feeds
/// `crate::adl14` and is our own design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// The spec-conformant ADL2 grammar.
    Adl2,
    /// ADL2 plus the 1.4-only tolerance forms (converter front end only).
    Adl14,
}

/// Parse a raw cADL `definition`-section body into a [`CComplexObject`] using
/// the grammar of `dialect`; error spans are relative to `body`.
///
/// # Errors
/// Returns every [`SyntaxError`] found (the `S*` catalogue codes of
/// `ADL2/master04.6`). Lexer failures surface as [`SyntaxErrorCode::Sunk`], a
/// malformed inline dADL domain block as [`SyntaxErrorCode::Sdinv`].
pub fn parse_definition_body(
    body: &str,
    dialect: Dialect,
) -> Result<CComplexObject, Vec<SyntaxError>> {
    let toks = match openehr_lang::v1_1::lexer::lex(openehr_lang::v1_1::lexer::Language::Adl, body)
    {
        Ok(t) => t,
        Err(failure) => return Err(vec![crate::error::lexical(&failure, body)]),
    };
    let mut parser = Parser {
        src: body,
        toks: &toks,
        pos: 0,
        errors: Vec::new(),
        dialect,
        nesting: Nesting::ROOT,
        node_coding: root_coding(&toks),
    };
    let root = parser.parse_root();
    match root {
        Ok(cco) if parser.errors.is_empty() => {
            if parser.pos < parser.toks.len() {
                let span = parser.span_at(parser.pos);
                parser.push(
                    SyntaxErrorCode::Sunk,
                    "unexpected trailing input after the definition object",
                    span,
                );
                Err(parser.errors)
            } else {
                Ok(cco)
            }
        }
        _ => {
            if parser.errors.is_empty() {
                parser.errors.push(SyntaxError::at(
                    SyntaxErrorCode::Sunk,
                    "empty definition",
                    0..0,
                    body,
                ));
            }
            Err(parser.errors)
        }
    }
}

/// The node coding of the archetype whose `definition` section body is `body`,
/// read off its root node id; a body that does not lex reads as at-coded,
/// since it fails to parse whatever its coding.
pub(crate) fn definition_coding(body: &str) -> NodeCoding {
    openehr_lang::v1_1::lexer::lex(openehr_lang::v1_1::lexer::Language::Adl, body)
        .map_or(NodeCoding::At, |toks| root_coding(&toks))
}

/// The coding of the archetype whose definition `toks` lexes, read off its
/// first node id, the root's. A body with no node id fails to parse whatever
/// its coding, so it reads as at-coded.
fn root_coding(toks: &[Spanned]) -> NodeCoding {
    toks.iter()
        .find_map(|t| match &t.token {
            Token::RootIdCode(c) | Token::IdCode(c) | Token::AtCode(c) => {
                Some(NodeCoding::of_root(c))
            }
            _ => None,
        })
        .unwrap_or(NodeCoding::At)
}

/// The recursive-descent cADL parser over a token slice.
pub(crate) struct Parser<'a> {
    pub(crate) src: &'a str,
    pub(crate) toks: &'a [Spanned],
    pub(crate) pos: usize,
    pub(crate) errors: Vec<SyntaxError>,
    pub(crate) dialect: Dialect,
    pub(crate) nesting: Nesting,
    /// The coding of the archetype being parsed. Text parsed with no archetype
    /// around it (a rule operand, a slot assertion) is at-coded, the openEHR
    /// primary code system (`org.openehr.am.aom2.adl_code_definitions`
    /// §Description).
    pub(crate) node_coding: NodeCoding,
}

// ── cursor + error helpers ────────────────────────────────────────────────
impl Parser<'_> {
    pub(crate) fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos).map(|s| &s.token)
    }

    pub(crate) fn peek_at(&self, ahead: usize) -> Option<&Token> {
        self.toks.get(self.pos + ahead).map(|s| &s.token)
    }

    pub(crate) fn span_at(&self, idx: usize) -> std::ops::Range<usize> {
        self.toks
            .get(idx)
            .map_or(self.src.len()..self.src.len(), |s| s.span.clone())
    }

    pub(crate) fn cur_span(&self) -> std::ops::Range<usize> {
        self.span_at(self.pos)
    }

    pub(crate) fn bump(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).map(|s| s.token.clone());
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    pub(crate) fn push(
        &mut self,
        code: SyntaxErrorCode,
        msg: impl Into<String>,
        span: std::ops::Range<usize>,
    ) {
        self.errors.push(SyntaxError::at(code, msg, span, self.src));
    }

    /// Record an error at the current position and bail.
    pub(crate) fn err<T>(&mut self, code: SyntaxErrorCode, msg: impl Into<String>) -> PResult<T> {
        let span = self.cur_span();
        self.push(code, msg, span);
        Err(())
    }

    /// Run `production` one object-nesting level deeper, refusing past the
    /// [`Nesting`] bound as `SUNK` (`ADL2/master04.6` §Syntax Validity Rules), since
    /// no `S*` code describes an implementation limit.
    pub(crate) fn nested<T>(
        &mut self,
        production: impl FnOnce(&mut Self) -> PResult<T>,
    ) -> PResult<T> {
        let outer = self.nesting;
        match outer.descend() {
            Ok(inner) => self.nesting = inner,
            Err(exceeded) => {
                return self.err(SyntaxErrorCode::Sunk, format!("constraint {exceeded}"));
            }
        }
        let result = production(self);
        self.nesting = outer;
        result
    }

    /// Take a decoded string/character literal, or report its escape defect at
    /// `span` and bail.
    ///
    /// NOTE: the `S*` codes mirror `ADL2/master04.6` §Syntax Validity Rules, which
    /// has no code for a defect inside a literal, so this reports `SUNK`.
    pub(crate) fn decoded_literal(
        &mut self,
        decoded: Result<String, openehr_lang::v1_1::escape::EscapeError>,
        span: std::ops::Range<usize>,
    ) -> PResult<String> {
        match decoded {
            Ok(text) => Ok(text),
            Err(defect) => {
                self.push(SyntaxErrorCode::Sunk, defect.to_string(), span);
                Err(())
            }
        }
    }

    /// Refuse an ADL 2-only cADL construct in the [`Dialect::Adl14`] dialect, since
    /// the cADL 1.4 keyword set is closed (`ADL1.4/master05-cadl.adoc` §Keywords).
    pub(crate) fn adl2_only<T>(&mut self, code: SyntaxErrorCode, construct: &str) -> PResult<T> {
        self.err(
            code,
            format!("{construct} is an ADL 2 construct and is not valid in ADL 1.4"),
        )
    }

    /// True if the cursor is at a NEGATED matches operator (`~matches`,
    /// `~is_in`, `∉`) — lexically `SymNot SymMatches` or the single `∉`.
    pub(crate) fn at_negated_matches(&self) -> bool {
        matches!(self.peek(), Some(Token::SymNotMatches))
            || (matches!(self.peek(), Some(Token::SymNot))
                && matches!(self.peek_at(1), Some(Token::SymMatches)))
    }

    /// Whether the cursor is at `=~` or `!~`, which arrive as token pairs
    /// (`adl_keywords.g4` `SYM_NOT`).
    pub(crate) fn at_regex_match_operator(&self) -> bool {
        matches!(self.peek(), Some(Token::SymEq | Token::SymNot))
            && matches!(self.peek_at(1), Some(Token::SymNot))
    }

    /// Refuse a negated matches operator, saying where negation is admitted.
    ///
    /// `ADL1.4/master05-cadl.adoc` §Keywords lists `~matches`/`~is_in`, but no
    /// normative production (`cadl14.g4`, `adl_keywords.g4`) admits them; prefix
    /// `not` is admitted only on a boolean expression (`base_expressions.g4`), in
    /// slot assertions and rules.
    pub(crate) fn negated_matches_reject<T>(&mut self, code: SyntaxErrorCode) -> PResult<T> {
        self.err(
            code,
            "a negated matches operator ('~matches' / '~is_in' / '∉') is not a cADL production; \
             negation is available as the prefix 'not' operator of a slot include/exclude assertion",
        )
    }

    /// Consume a token matching `pred` or record `code`/`msg` and bail.
    pub(crate) fn expect(
        &mut self,
        pred: impl Fn(&Token) -> bool,
        code: SyntaxErrorCode,
        msg: &str,
    ) -> PResult<()> {
        if self.peek().is_some_and(&pred) {
            self.pos += 1;
            Ok(())
        } else {
            self.err(code, msg.to_owned())
        }
    }

    pub(crate) fn eat(&mut self, pred: impl Fn(&Token) -> bool) -> bool {
        if self.peek().is_some_and(&pred) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

// ── re-entrant sub-parsers (driven by `crate::rules`) ─────────────────────

/// Parse a `matches { … }` primitive right-hand side into a
/// [`CPrimitiveObject`] for an `EXPR_CONSTRAINT` leaf (`AOM2` master05), with
/// the short-form primitive node id of `coding`.
///
/// # Errors
/// Returns the cADL `S*` catalogue errors if the constraint is malformed or its
/// body is not a single primitive constraint.
pub(crate) fn parse_inline_primitive_text(
    raw: &str,
    coding: NodeCoding,
) -> Result<CPrimitiveObject, Vec<SyntaxError>> {
    let toks = openehr_lang::v1_1::lexer::lex(openehr_lang::v1_1::lexer::Language::Adl, raw)
        .map_err(|failure| vec![crate::error::lexical(&failure, raw)])?;
    let mut parser = Parser {
        src: raw,
        toks: &toks,
        pos: 0,
        errors: Vec::new(),
        dialect: Dialect::Adl2,
        nesting: Nesting::ROOT,
        node_coding: coding,
    };
    let parsed: PResult<CObject> = (|| {
        parser.expect(
            |t| matches!(t, Token::LCurly),
            SyntaxErrorCode::Sccog,
            "expecting '{' opening a constraint",
        )?;
        let obj = parser.parse_c_inline_primitive(parser.node_coding.primitive_node_id())?;
        parser.expect(
            |t| matches!(t, Token::RCurly),
            SyntaxErrorCode::Sccog,
            "expecting '}' closing a constraint",
        )?;
        Ok(obj)
    })();
    match parsed {
        Ok(obj) if parser.errors.is_empty() => cobject_to_primitive(&obj).ok_or_else(|| {
            vec![SyntaxError::at(
                SyntaxErrorCode::Sccog,
                "constraint body is not a primitive constraint",
                0..raw.len(),
                raw,
            )]
        }),
        _ => {
            if parser.errors.is_empty() {
                parser.errors.push(SyntaxError::at(
                    SyntaxErrorCode::Sccog,
                    "invalid primitive constraint",
                    0..raw.len(),
                    raw,
                ));
            }
            Err(parser.errors)
        }
    }
}

/// Parse a contained-regexp right-hand side (`{ /re/ }`, `{ ^re^ }`) into the
/// [`CString`] of an archetype-id slot assertion (`AOM2` master05; `master04.3`
/// §Archetype Slots), compile-checked (`SCSRE`).
///
/// # Errors
/// [`SyntaxErrorCode::Scsre`] if the regex does not compile.
pub(crate) fn parse_contained_regexp_text(
    raw: &str,
    coding: NodeCoding,
) -> Result<CString, Vec<SyntaxError>> {
    let (regex_part, quoted_assumed) = split_contained_regexp(raw);
    // The regex body itself is NEVER escape-decoded (`master03` §Special
    // Character Sequences, final paragraph); only the `;"assumed"` suffix is.
    let assumed = match quoted_assumed {
        Some(text) => Some(openehr_lang::v1_1::escape::decode(text).map_err(|defect| {
            vec![SyntaxError::at(
                SyntaxErrorCode::Sunk,
                defect.to_string(),
                0..raw.len(),
                raw,
            )]
        })?),
        None => None,
    };
    let inner = regex_inner(regex_part);
    if regex::Regex::new(inner).is_err() {
        return Err(vec![SyntaxError::at(
            SyntaxErrorCode::Scsre,
            format!("{inner:?} is not a valid regular expression"),
            0..raw.len(),
            raw,
        )]);
    }
    Ok(ObjectNode::inline("String", coding).c_string(vec![regex_part.to_owned()], assumed))
}

#[cfg(test)]
mod tests {
    use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
    use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

    use crate::error::SyntaxErrorCode;
    use crate::parse::{Dialect, parse_definition_body};

    /// Every ADL2-only cADL construct is refused in the 1.4 dialect with the
    /// typed code for its parse position, and is still accepted in ADL 2 — the
    /// gate must narrow the 1.4 dialect only (`ADL1.4/master05-cadl.adoc`
    /// §Keywords L48-53 is the closed 1.4 keyword set).
    #[test]
    fn adl2_only_constructs_are_refused_in_the_14_dialect_only() {
        let cases: &[(&str, SyntaxErrorCode)] = &[
            // `_default` (ADL2/master06-default_values.adoc).
            (
                "OBSERVATION[at0000] matches {\n\
                 data matches {\n\
                 HISTORY[at0001] matches {\n\
                 _default = (DV_QUANTITY) <units = <\"mm[Hg]\"> magnitude = <5.0>>\n\
                 }\n\
                 }\n\
                 }",
                SyntaxErrorCode::Scoat,
            ),
            // Second-order attribute tuple (ADL2/master04.4).
            (
                "OBSERVATION[at0000] matches {\n\
                 value matches {\n\
                 DV_QUANTITY matches {\n\
                 [magnitude, units] matches {\n\
                 [{|>=0.0|}, {\"mm[Hg]\"}]\n\
                 }\n\
                 }\n\
                 }\n\
                 }",
                SyntaxErrorCode::Scoat,
            ),
            // `use_archetype` (ADL2/master04.3 §Archetype Slots).
            (
                "SECTION[at0000] matches {\n\
                 items cardinality matches {0..*} matches {\n\
                 use_archetype CLUSTER[at0001, openEHR-EHR-CLUSTER.dimensions.v1]\n\
                 }\n\
                 }",
                SyntaxErrorCode::Sccog,
            ),
            // The slot `closed` marker (ADL2/master04.3 §Archetype Slots).
            (
                "SECTION[at0000] matches {\n\
                 items cardinality matches {0..*} matches {\n\
                 allow_archetype CLUSTER[at0001] closed\n\
                 }\n\
                 }",
                SyntaxErrorCode::Sccog,
            ),
            // Constraint strength (AOM2/master04.2 §Constraint Strengths).
            (
                "ELEMENT[at0000] matches {\n\
                 value matches {\n\
                 DV_CODED_TEXT matches {\n\
                 defining_code matches {preferred [ac0001]}\n\
                 }\n\
                 }\n\
                 }",
                SyntaxErrorCode::Stccp,
            ),
            // `@terminology` operational binding (ADL2/master08).
            (
                "ELEMENT[at0000] matches {\n\
                 value matches {\n\
                 DV_CODED_TEXT matches {\n\
                 defining_code matches {[ac0001@snomed]}\n\
                 }\n\
                 }\n\
                 }",
                SyntaxErrorCode::Stccp,
            ),
        ];
        for (src, code) in cases {
            let errs = parse_definition_body(src, Dialect::Adl14)
                .err()
                .unwrap_or_else(|| panic!("the 1.4 dialect must refuse:\n{src}"));
            assert!(
                errs.iter().any(|e| &e.code == code),
                "expected {code} for:\n{src}\ngot {:?}",
                errs.iter().map(|e| e.code).collect::<Vec<_>>()
            );
            assert!(
                parse_definition_body(src, Dialect::Adl2).is_ok(),
                "the ADL2 dialect must still accept:\n{src}"
            );
        }
    }

    /// `before`/`after` are cADL 1.4 keywords (`ADL1.4/master05-cadl.adoc`
    /// §Keywords L53), so a sibling order is legal in a 1.4 text and must not be
    /// gated with the ADL2-only constructs.
    #[test]
    fn adl14_accepts_sibling_order() {
        let cco = parse_definition_body(
            "CLUSTER[at0000] matches {\n\
             items cardinality matches {0..*} matches {\n\
             ELEMENT[at0001] matches {*}\n\
             before [at0001] ELEMENT[at0002] matches {*}\n\
             }\n\
             }",
            Dialect::Adl14,
        )
        .expect("sibling order is legal ADL 1.4 cADL");
        let CComplexObject::CComplexObject(d) = &cco else {
            panic!("expected a plain complex object root");
        };
        let CObject::CComplexObject(CComplexObject::CComplexObject(second)) =
            &d.attributes.as_deref().unwrap_or_default()[0]
                .children
                .as_deref()
                .unwrap_or_default()[1]
        else {
            panic!("expected the re-ordered ELEMENT");
        };
        let order = second.sibling_order.as_ref().expect("a sibling order");
        assert!(order.is_before);
        assert_eq!(order.sibling_node_id, "at0001");
    }

    /// The negated-matches family and the `=~`/`!~` regex-match operators are
    /// named by `ADL1.4/master05-cadl.adoc` (§Keywords L47/L95-98, §Regular
    /// Expression L691-693) but defined by no grammar production, so each is a
    /// typed refusal rather than a silent affirmative reading.
    #[test]
    fn operators_without_a_production_are_refused() {
        for (src, code) in [
            (
                "WHOLE[id1] matches {\n v matches {\n TEXT ~matches {\"a\"}\n }\n}",
                SyntaxErrorCode::Sccog,
            ),
            (
                "WHOLE[id1] matches {\n v ~is_in {\n TEXT matches {*}\n }\n}",
                SyntaxErrorCode::Scoat,
            ),
            (
                "WHOLE[id1] matches {\n v \u{2209} {\n TEXT matches {*}\n }\n}",
                SyntaxErrorCode::Scoat,
            ),
            (
                "WHOLE[id1] matches {\n v matches {=~ /[a-z]+/}\n}",
                SyntaxErrorCode::Sccog,
            ),
            (
                "WHOLE[id1] matches {\n v matches {!~ /[a-z]+/}\n}",
                SyntaxErrorCode::Sccog,
            ),
        ] {
            let errs = parse_definition_body(src, Dialect::Adl2)
                .expect_err("an operator with no cADL production must be refused");
            assert!(
                errs.iter().any(|e| e.code == code),
                "expected {code} for {src:?}, got {:?}",
                errs.iter().map(|e| e.code).collect::<Vec<_>>()
            );
        }
    }
}
