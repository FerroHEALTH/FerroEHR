// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The inline `C_PRIMITIVE` productions (`cadl2_primitives.g4`).
//!
//! The primitive-kind dispatch, one production per ADL primitive type, the
//! terminology-code constraint (with the `master04.5` constraint strengths and
//! the OPT `@terminology` operational binding), the contained-regexp shortcut,
//! and the body-less "any primitive" objects of
//! `ADL2/master04.5-cadl_primitive_types.adoc`. One `impl` block over the
//! `Parser` state of [`crate::parse`]; the value lists, intervals and
//! endpoints these productions read live in [`crate::parse::values`], and the
//! date/time constraint-pattern validators in [`crate::parse::patterns`].

use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;
use openehr_am::v2_4::aom2::constraint_model::primitive::constraint_status::ConstraintStatus;
use openehr_base::prelude::{Iso8601Date, Iso8601DateTime, Iso8601Duration, Iso8601Time};

use crate::aom::build::{ObjectNode, local_term_code};
use crate::error::SyntaxErrorCode;
use crate::odin::{decode_string, escape_regex_delimiter, regex_inner};
use crate::parse::values::PrimKind;
use crate::parse::{Dialect, PResult, Parser};
use openehr_lang::v1_1::lexer::Token;

// ── inline primitives (`cadl2_primitives.g4`) ─────────────────────────────
impl Parser<'_> {
    /// True if a type-object `matches { … }` body is a single inline primitive
    /// (a `c_regular_primitive_object`) rather than a complex object. Disambiguates
    /// the leading `[`: an object body's `[attr, …]` is a `c_attribute_tuple`
    /// (complex), whereas `[ac…]`/`[at…]` is a `c_terminology_code` (primitive).
    pub(crate) fn body_is_inline_primitive(&self) -> bool {
        match self.peek() {
            Some(Token::LBracket) => {
                matches!(self.peek_at(1), Some(Token::AcCode(_) | Token::AtCode(_)))
            }
            Some(Token::AlphaLcId(s)) => {
                is_strength_keyword(s) && matches!(self.peek_at(1), Some(Token::LBracket))
            }
            _ => self.is_inline_primitive_start(),
        }
    }

    /// True if the current token begins a `c_inline_primitive_object`.
    pub(crate) fn is_inline_primitive_start(&self) -> bool {
        match self.peek() {
            Some(
                Token::SymTrue
                | Token::SymFalse
                | Token::String(_)
                | Token::Character(_)
                | Token::Integer(_)
                | Token::Real(_)
                | Token::Iso8601Date(_)
                | Token::Iso8601Time(_)
                | Token::Iso8601DateTime(_)
                | Token::Iso8601Duration(_)
                | Token::DateConstraintPattern(_)
                | Token::TimeConstraintPattern(_)
                | Token::DateTimeConstraintPattern(_)
                | Token::DurationConstraintPattern(_)
                | Token::SymIvlDelim
                | Token::LBracket
                | Token::SymPlus
                | Token::SymMinus,
            ) => true,
            Some(Token::AlphaLcId(s)) => is_strength_keyword(s),
            _ => false,
        }
    }

    /// `c_inline_primitive_object` (`cadl2_primitives.g4`): dispatch on the
    /// leading token; `node_id` is `Primitive_node_id` for inline forms.
    pub(crate) fn parse_c_inline_primitive(&mut self, node_id: String) -> PResult<CObject> {
        match self.peek().cloned() {
            Some(Token::SymTrue | Token::SymFalse) => self.parse_c_boolean(node_id),
            Some(Token::String(_)) => self.parse_c_string(node_id),
            Some(Token::Character(_)) => self.parse_c_character(node_id),
            Some(Token::LBracket) => self.parse_c_terminology_code(node_id, None),
            Some(Token::AlphaLcId(s)) if is_strength_keyword(&s) => {
                // ADL2-only: constraint strengths (`required`/`extensible`/
                // `preferred`/`example`) are `C_TERMINOLOGY_CODE.constraint_status`,
                // introduced by `AOM2/master04.2` §Constraint Strengths ("Uniquely
                // in the AOM, a Terminology code constraint may not be required").
                // The 1.4 cADL keyword set (master05 §Keywords L48-53) has no
                // strength vocabulary and AOM 1.4 has no such attribute.
                if self.dialect == Dialect::Adl14 {
                    return self.adl2_only(
                        SyntaxErrorCode::Stccp,
                        format!("the term-constraint strength {s:?}").as_str(),
                    );
                }
                self.pos += 1;
                self.parse_c_terminology_code(node_id, Some(strength_status(&s)))
            }
            Some(Token::DateConstraintPattern(_) | Token::Iso8601Date(_)) => {
                self.parse_c_date(node_id)
            }
            Some(Token::TimeConstraintPattern(_) | Token::Iso8601Time(_)) => {
                self.parse_c_time(node_id)
            }
            Some(Token::DateTimeConstraintPattern(_) | Token::Iso8601DateTime(_)) => {
                self.parse_c_date_time(node_id)
            }
            Some(Token::DurationConstraintPattern(_) | Token::Iso8601Duration(_)) => {
                self.parse_c_duration(node_id)
            }
            Some(Token::Real(_)) => self.parse_c_real(node_id),
            Some(Token::Integer(_)) => self.parse_c_integer(node_id),
            Some(Token::SymIvlDelim) => match self.classify_bar_kind()? {
                PrimKind::Integer => self.parse_c_integer(node_id),
                PrimKind::Real => self.parse_c_real(node_id),
                PrimKind::Date => self.parse_c_date(node_id),
                PrimKind::Time => self.parse_c_time(node_id),
                PrimKind::DateTime => self.parse_c_date_time(node_id),
                PrimKind::Duration => self.parse_c_duration(node_id),
            },
            Some(Token::SymPlus | Token::SymMinus) => {
                if matches!(self.peek_at(1), Some(Token::Real(_))) {
                    self.parse_c_real(node_id)
                } else {
                    self.parse_c_integer(node_id)
                }
            }
            _ => self.err(
                SyntaxErrorCode::Sccog,
                "expecting a primitive constraint value",
            ),
        }
    }

    fn parse_c_boolean(&mut self, node_id: String) -> PResult<CObject> {
        let mut constraint = Vec::new();
        loop {
            match self.peek() {
                Some(Token::SymTrue) => {
                    constraint.push(true);
                    self.pos += 1;
                }
                Some(Token::SymFalse) => {
                    constraint.push(false);
                    self.pos += 1;
                }
                _ => return self.err(SyntaxErrorCode::Scbav, "expecting 'True' or 'False'"),
            }
            if !self.eat(|t| matches!(t, Token::SymComma)) || self.eat_list_continue() {
                break;
            }
        }
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            Some(match self.peek() {
                Some(Token::SymTrue) => {
                    self.pos += 1;
                    true
                }
                Some(Token::SymFalse) => {
                    self.pos += 1;
                    false
                }
                _ => {
                    return self.err(
                        SyntaxErrorCode::Scbav,
                        "assumed value must be 'True' or 'False'",
                    );
                }
            })
        } else {
            None
        };
        let node = ObjectNode {
            rm_type_name: "Boolean".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CBoolean(node.c_boolean(constraint, assumed_value)))
    }

    /// `c_character` — a `Character` constraint written as a list of
    /// single-quoted characters (`ADL1.4/master05-cadl.adoc` §Constraints on
    /// Character L813-825, identically `ADL2/master04.5` §Constraints on
    /// Character L160-172: "`color_name matches {'r', 'g', 'b'}`"). The
    /// single-character regex form (L827-841) needs no separate production —
    /// it is a `CONTAINED_REGEXP` and lands on the same carrier.
    ///
    /// NOTE: neither vendored AOM generation defines a `C_CHARACTER` class
    /// (the AM 1.4.0 and 2.4.0 BMM constraint models both lack a character
    /// constrainer), so the carrier is `C_STRING` — whose value space
    /// contains the single-character strings exactly — with the constrained
    /// RM type name (`Character`) kept on the object. No openEHR spec governs
    /// this mapping — our own design/extension.
    fn parse_c_character(&mut self, node_id: String) -> PResult<CObject> {
        let mut constraint = Vec::new();
        loop {
            match self.peek().cloned() {
                Some(Token::Character(c)) => {
                    let span = self.cur_span();
                    self.pos += 1;
                    let decoded = openehr_lang::v1_1::escape::decode_character_literal(&c);
                    constraint.push(self.decoded_literal(decoded, span)?);
                }
                _ => return self.err(SyntaxErrorCode::Scsav, "expecting a character value"),
            }
            if !self.eat(|t| matches!(t, Token::SymComma)) || self.eat_list_continue() {
                break;
            }
        }
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            match self.peek().cloned() {
                Some(Token::Character(c)) => {
                    let span = self.cur_span();
                    self.pos += 1;
                    let decoded = openehr_lang::v1_1::escape::decode_character_literal(&c);
                    Some(self.decoded_literal(decoded, span)?)
                }
                _ => {
                    return self.err(SyntaxErrorCode::Scsav, "assumed value must be a character");
                }
            }
        } else {
            None
        };
        let node = ObjectNode {
            rm_type_name: "Character".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CString(node.c_string(constraint, assumed_value)))
    }

    fn parse_c_string(&mut self, node_id: String) -> PResult<CObject> {
        let mut constraint = Vec::new();
        loop {
            match self.peek().cloned() {
                Some(Token::String(s)) => {
                    let span = self.cur_span();
                    self.pos += 1;
                    let decoded = decode_string(&s);
                    constraint.push(self.decoded_literal(decoded, span)?);
                }
                _ => return self.err(SyntaxErrorCode::Scsav, "expecting a string value"),
            }
            if !self.eat(|t| matches!(t, Token::SymComma)) || self.eat_list_continue() {
                break;
            }
        }
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            match self.peek().cloned() {
                Some(Token::String(s)) => {
                    let span = self.cur_span();
                    self.pos += 1;
                    let decoded = decode_string(&s);
                    Some(self.decoded_literal(decoded, span)?)
                }
                _ => return self.err(SyntaxErrorCode::Scsav, "assumed value must be a string"),
            }
        } else {
            None
        };
        let node = ObjectNode {
            rm_type_name: "String".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CString(node.c_string(constraint, assumed_value)))
    }

    /// `c_terminology_code : '[' ( AC_CODE ( ';' AT_CODE )? | AT_CODE ) ']'`
    /// (`cadl2_primitives.g4`), extended with the constraint-strength prefix
    /// (`master04.5` §Constraint strengths) and the OPT operational-binding
    /// `[codeN@terminology]` form (`master04.5` §Operational Binding
    /// Constraints — beyond the pinned grammar, but spec-normative).
    pub(crate) fn parse_c_terminology_code(
        &mut self,
        node_id: String,
        constraint_status: Option<ConstraintStatus>,
    ) -> PResult<CObject> {
        self.expect(
            |t| matches!(t, Token::LBracket),
            SyntaxErrorCode::Stccp,
            "expecting '[' opening a terminology code",
        )?;
        let mut constraint = match self.peek().cloned() {
            Some(Token::AcCode(c) | Token::AtCode(c)) => {
                self.pos += 1;
                c
            }
            _ => return self.err(SyntaxErrorCode::Stccp, "expecting an ac-code or at-code"),
        };
        // Optional OPT operational binding `@terminology`.
        //
        // ADL2-only: the `[acN@terminology]` operational binding belongs to the
        // ADL2 terminology-integration surface (`ADL2/master08-terminology_integration.adoc`
        // §Terminology Bindings / OPT2 operational form). The 1.4 cADL keyword set
        // (master05 §Keywords L48-53) has no `@` operator, and 1.4 expresses
        // external terminology through the qualified `[terminology::code, …]` form
        // of `ADL1.4/master09-customising_adl.adoc` §Custom Syntax.
        if matches!(self.peek(), Some(Token::SymAt)) && self.dialect == Dialect::Adl14 {
            return self.adl2_only(
                SyntaxErrorCode::Stccp,
                "an '@terminology' operational binding on a term-code constraint",
            );
        }
        if self.eat(|t| matches!(t, Token::SymAt)) {
            match self.peek().cloned() {
                Some(Token::AlphaLcId(t) | Token::AlphaUcId(t)) => {
                    self.pos += 1;
                    constraint = format!("{constraint}@{t}");
                }
                _ => {
                    return self.err(
                        SyntaxErrorCode::Stccp,
                        "expecting a terminology id after '@'",
                    );
                }
            }
        }
        // Optional assumed at-code (only valid after an ac-code).
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            match self.peek().cloned() {
                Some(Token::AtCode(a)) => {
                    self.pos += 1;
                    Some(local_term_code(&a))
                }
                _ => {
                    return self.err(
                        SyntaxErrorCode::Stccp,
                        "assumed terminology value must be an at-code",
                    );
                }
            }
        } else {
            None
        };
        self.expect(
            |t| matches!(t, Token::RBracket),
            SyntaxErrorCode::Stccp,
            "expecting ']' closing a terminology code",
        )?;
        let node = ObjectNode {
            rm_type_name: "Terminology_code".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CTerminologyCode(node.c_terminology_code(
            constraint,
            constraint_status,
            assumed_value,
        )))
    }

    fn parse_c_integer(&mut self, node_id: String) -> PResult<CObject> {
        let constraint = self.parse_value_list::<i32>()?;
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            Some(f64::from(self.parse_signed_int(SyntaxErrorCode::Sciav)?))
        } else {
            None
        };
        let node = ObjectNode {
            rm_type_name: "Integer".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CInteger(node.c_integer(constraint, assumed_value)))
    }

    fn parse_c_real(&mut self, node_id: String) -> PResult<CObject> {
        let constraint = self.parse_value_list::<f64>()?;
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            Some(self.parse_signed_real(SyntaxErrorCode::Scrav)?)
        } else {
            None
        };
        let node = ObjectNode {
            rm_type_name: "Real".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CReal(node.c_real(constraint, assumed_value)))
    }

    fn parse_c_date(&mut self, node_id: String) -> PResult<CObject> {
        let mut pattern_constraint = None;
        let mut constraint = Vec::new();
        if let Some(Token::DateConstraintPattern(p)) = self.peek().cloned() {
            self.pos += 1;
            self.validate_date_pattern(&p, SyntaxErrorCode::Scdpt)?;
            pattern_constraint = Some(p);
        } else {
            constraint = self.parse_value_list::<Iso8601Date>()?;
        }
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            match self.peek().cloned() {
                Some(Token::Iso8601Date(v)) => {
                    self.pos += 1;
                    Some(Iso8601Date { value: v })
                }
                _ => {
                    return self.err(
                        SyntaxErrorCode::Scdav,
                        "assumed value must be an ISO8601 date",
                    );
                }
            }
        } else {
            None
        };
        let node = ObjectNode {
            rm_type_name: "Iso8601_date".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CDate(node.c_date(
            constraint,
            pattern_constraint,
            assumed_value,
        )))
    }

    fn parse_c_time(&mut self, node_id: String) -> PResult<CObject> {
        let mut pattern_constraint = None;
        let mut constraint = Vec::new();
        if let Some(Token::TimeConstraintPattern(p)) = self.peek().cloned() {
            self.pos += 1;
            self.validate_time_pattern(&p, SyntaxErrorCode::Sctpt)?;
            pattern_constraint = Some(p);
        } else {
            constraint = self.parse_value_list::<Iso8601Time>()?;
        }
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            match self.peek().cloned() {
                Some(Token::Iso8601Time(v)) => {
                    self.pos += 1;
                    Some(Iso8601Time { value: v })
                }
                _ => {
                    return self.err(
                        SyntaxErrorCode::Sctav,
                        "assumed value must be an ISO8601 time",
                    );
                }
            }
        } else {
            None
        };
        let node = ObjectNode {
            rm_type_name: "Iso8601_time".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CTime(node.c_time(
            constraint,
            pattern_constraint,
            assumed_value,
        )))
    }

    fn parse_c_date_time(&mut self, node_id: String) -> PResult<CObject> {
        let mut pattern_constraint = None;
        let mut constraint = Vec::new();
        if let Some(Token::DateTimeConstraintPattern(p)) = self.peek().cloned() {
            self.pos += 1;
            self.validate_date_time_pattern(&p, SyntaxErrorCode::Scdtpt)?;
            pattern_constraint = Some(p);
        } else {
            constraint = self.parse_value_list::<Iso8601DateTime>()?;
        }
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            match self.peek().cloned() {
                Some(Token::Iso8601DateTime(v)) => {
                    self.pos += 1;
                    Some(Iso8601DateTime { value: v })
                }
                _ => {
                    return self.err(
                        SyntaxErrorCode::Scdtav,
                        "assumed value must be an ISO8601 date/time",
                    );
                }
            }
        } else {
            None
        };
        let node = ObjectNode {
            rm_type_name: "Iso8601_date_time".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CDateTime(node.c_date_time(
            constraint,
            pattern_constraint,
            assumed_value,
        )))
    }

    /// `c_duration : ( DURATION_CONSTRAINT_PATTERN ( '/' ( duration_interval |
    /// duration ))? | value/list/interval ) assumed?` (`cadl2_primitives.g4`).
    fn parse_c_duration(&mut self, node_id: String) -> PResult<CObject> {
        let mut pattern_constraint = None;
        let mut constraint = Vec::new();
        if let Some(Token::DurationConstraintPattern(p)) = self.peek().cloned() {
            self.pos += 1;
            self.validate_duration_pattern(&p, SyntaxErrorCode::Scdupt)?;
            pattern_constraint = Some(p);
            if self.eat(|t| matches!(t, Token::SymSlash)) {
                // `pattern/interval` or `pattern/value` mixed form.
                constraint.push(self.parse_value_item::<Iso8601Duration>()?);
            }
        } else {
            constraint = self.parse_value_list::<Iso8601Duration>()?;
        }
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            match self.peek().cloned() {
                Some(Token::Iso8601Duration(v)) => {
                    self.pos += 1;
                    Some(Iso8601Duration { value: v })
                }
                _ => {
                    return self.err(
                        SyntaxErrorCode::Scduav,
                        "assumed value must be an ISO8601 duration",
                    );
                }
            }
        } else {
            None
        };
        let node = ObjectNode {
            rm_type_name: "Iso8601_duration".to_owned(),
            node_id,
            occurrences: None,
        };
        Ok(CObject::CDuration(node.c_duration(
            constraint,
            pattern_constraint,
            assumed_value,
        )))
    }

    /// The `C_STRING` of a `CONTAINED_REGEXP` token (`{ /re/ [;"assumed"] }` or
    /// the `^re^` form): its regex with `/` delimiters, compile-checked, and its
    /// assumed value.
    pub(crate) fn contained_regexp(
        &mut self,
        raw: &str,
        span: std::ops::Range<usize>,
    ) -> PResult<CString> {
        let (regex_part, quoted_assumed) = split_contained_regexp(raw);
        // The regex body itself is NEVER escape-decoded (`master03` §Special
        // Character Sequences, final paragraph); only the `;"assumed"` suffix
        // is.
        let assumed = match quoted_assumed {
            Some(text) => {
                let decoded = openehr_lang::v1_1::escape::decode(text);
                Some(self.decoded_literal(decoded, span.clone())?)
            }
            None => None,
        };
        let inner = regex_inner(regex_part);
        if regex::Regex::new(inner).is_err() {
            self.push(
                SyntaxErrorCode::Scsre,
                format!("{regex_part:?} is not a valid regular expression"),
                span,
            );
            return Err(());
        }
        let regex = format!("/{}/", escape_regex_delimiter(inner));
        Ok(ObjectNode::inline("String", self.node_coding).c_string(vec![regex], assumed))
    }

    /// A body-less type-headed primitive object (`String[id2]`): recognise the
    /// fixed set of ADL primitive RM type names (`master04.5`) and build an
    /// unconstrained primitive; non-primitive names fall through to a complex
    /// object.
    pub(crate) fn primitive_any(rm_type: &str, node_id: &str) -> Option<CObject> {
        let node = ObjectNode {
            rm_type_name: rm_type.to_owned(),
            node_id: node_id.to_owned(),
            occurrences: None,
        };
        Some(match rm_type {
            "Boolean" => CObject::CBoolean(node.c_boolean(Vec::new(), None)),
            "String" => CObject::CString(node.c_string(Vec::new(), None)),
            "Integer" => CObject::CInteger(node.c_integer(Vec::new(), None)),
            "Real" => CObject::CReal(node.c_real(Vec::new(), None)),
            "Iso8601_date" => CObject::CDate(node.c_date(Vec::new(), None, None)),
            "Iso8601_time" => CObject::CTime(node.c_time(Vec::new(), None, None)),
            "Iso8601_date_time" => CObject::CDateTime(node.c_date_time(Vec::new(), None, None)),
            "Iso8601_duration" => CObject::CDuration(node.c_duration(Vec::new(), None, None)),
            "Terminology_code" => {
                CObject::CTerminologyCode(node.c_terminology_code(String::new(), None, None))
            }
            _ => return None,
        })
    }
}

/// Splits a `CONTAINED_REGEXP` token (`{ /re/ [;"assumed"] }` or the `^re^`
/// form) into its regex, delimiters kept, and the text of its optional quoted
/// assumed value.
pub(crate) fn split_contained_regexp(raw: &str) -> (&str, Option<&str>) {
    let body = raw
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();
    match body.split_once(';') {
        Some((regex, assumed)) => (
            regex.trim(),
            assumed
                .trim()
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"')),
        ),
        None => (body, None),
    }
}

// ── constraint-strength keywords (`master04.5`) ───────────────────────────

/// The strength-keyword set (`master04.5` §Constraint strengths).
fn is_strength_keyword(s: &str) -> bool {
    matches!(s, "required" | "extensible" | "preferred" | "example")
}

/// Map a constraint-strength keyword to its ordinal `CONSTRAINT_STATUS`
/// (`master09.05`: required(0) < extensible(1) < preferred(2) < example(3)).
fn strength_status(s: &str) -> ConstraintStatus {
    // `required` (and any unrecognised keyword) maps to the default status.
    match s {
        "extensible" => ConstraintStatus::Extensible,
        "preferred" => ConstraintStatus::Preferred,
        "example" => ConstraintStatus::Example,
        _ => ConstraintStatus::Required,
    }
}

#[cfg(test)]
mod tests;
