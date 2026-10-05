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
use openehr_am::v2_4::aom2::constraint_model::primitive::c_boolean::CBoolean;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_date::CDate;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_date_time::CDateTime;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_duration::CDuration;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_real::CReal;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_time::CTime;
use openehr_am::v2_4::aom2::constraint_model::primitive::constraint_status::ConstraintStatus;
use openehr_base::prelude::{Iso8601Date, Iso8601DateTime, Iso8601Duration, Iso8601Time};

use crate::aom::build::local_term_code;
use crate::error::SyntaxErrorCode;
use crate::odin::{decode_character, decode_string, escape_regex_delimiter, regex_inner};
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
        Ok(CObject::CBoolean(CBoolean {
            parent: None,
            soc_parent: None,
            rm_type_name: "Boolean".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(constraint),
        }))
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
                    let decoded = decode_character(&c);
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
                    let decoded = decode_character(&c);
                    Some(self.decoded_literal(decoded, span)?)
                }
                _ => {
                    return self.err(SyntaxErrorCode::Scsav, "assumed value must be a character");
                }
            }
        } else {
            None
        };
        Ok(CObject::CString(CString {
            parent: None,
            soc_parent: None,
            rm_type_name: "Character".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(constraint),
        }))
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
        Ok(CObject::CString(CString {
            parent: None,
            soc_parent: None,
            rm_type_name: "String".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(constraint),
        }))
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
        Ok(CObject::CTerminologyCode(CTerminologyCode {
            parent: None,
            soc_parent: None,
            rm_type_name: "Terminology_code".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint,
            constraint_status,
        }))
    }

    fn parse_c_integer(&mut self, node_id: String) -> PResult<CObject> {
        let constraint = self.parse_value_list::<i32>()?;
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            Some(f64::from(self.parse_signed_int(SyntaxErrorCode::Sciav)?))
        } else {
            None
        };
        Ok(CObject::CInteger(CInteger {
            parent: None,
            soc_parent: None,
            rm_type_name: "Integer".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(constraint),
        }))
    }

    fn parse_c_real(&mut self, node_id: String) -> PResult<CObject> {
        let constraint = self.parse_value_list::<f64>()?;
        let assumed_value = if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            Some(self.parse_signed_real(SyntaxErrorCode::Scrav)?)
        } else {
            None
        };
        Ok(CObject::CReal(CReal {
            parent: None,
            soc_parent: None,
            rm_type_name: "Real".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(constraint),
        }))
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
        Ok(CObject::CDate(CDate {
            parent: None,
            soc_parent: None,
            rm_type_name: "Iso8601_date".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(constraint),
            pattern_constraint,
        }))
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
        Ok(CObject::CTime(CTime {
            parent: None,
            soc_parent: None,
            rm_type_name: "Iso8601_time".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(constraint),
            pattern_constraint,
        }))
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
        Ok(CObject::CDateTime(CDateTime {
            parent: None,
            soc_parent: None,
            rm_type_name: "Iso8601_date_time".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(constraint),
            pattern_constraint,
        }))
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
        Ok(CObject::CDuration(CDuration {
            parent: None,
            soc_parent: None,
            rm_type_name: "Iso8601_duration".to_owned(),
            occurrences: None,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(constraint),
            pattern_constraint,
        }))
    }

    /// Split a `CONTAINED_REGEXP` token (`{ /re/ [;"assumed"] }` or the `^re^`
    /// form) into `(regex-with-/-delims, assumed?)`, compile-checking it.
    pub(crate) fn contained_regexp_parts(
        &mut self,
        raw: &str,
        span: std::ops::Range<usize>,
    ) -> PResult<(String, Option<String>)> {
        let body = raw
            .trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim();
        // Optional `;"assumed"` suffix.
        let (regex_part, quoted_assumed) = match body.split_once(';') {
            Some((r, a)) => (
                r.trim(),
                a.trim().strip_prefix('"').and_then(|s| s.strip_suffix('"')),
            ),
            None => (body, None),
        };
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
        Ok((format!("/{}/", escape_regex_delimiter(inner)), assumed))
    }

    /// A body-less type-headed primitive object (`String[id2]`): recognise the
    /// fixed set of ADL primitive RM type names (`master04.5`) and build an
    /// unconstrained primitive; non-primitive names fall through to a complex
    /// object.
    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per ADL primitive C_* struct literal (master04.5); the length is the size of the primitive set"
    )]
    pub(crate) fn primitive_any(rm_type: &str, node_id: &str) -> Option<CObject> {
        let nid = node_id.to_owned();
        let obj = match rm_type {
            "Boolean" => CObject::CBoolean(CBoolean {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type.to_owned(),
                occurrences: None,
                node_id: nid,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(Vec::new()),
            }),
            "String" => CObject::CString(CString {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type.to_owned(),
                occurrences: None,
                node_id: nid,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(Vec::new()),
            }),
            "Integer" => CObject::CInteger(CInteger {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type.to_owned(),
                occurrences: None,
                node_id: nid,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(Vec::new()),
            }),
            "Real" => CObject::CReal(CReal {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type.to_owned(),
                occurrences: None,
                node_id: nid,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(Vec::new()),
            }),
            "Iso8601_date" => CObject::CDate(CDate {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type.to_owned(),
                occurrences: None,
                node_id: nid,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(Vec::new()),
                pattern_constraint: None,
            }),
            "Iso8601_time" => CObject::CTime(CTime {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type.to_owned(),
                occurrences: None,
                node_id: nid,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(Vec::new()),
                pattern_constraint: None,
            }),
            "Iso8601_date_time" => CObject::CDateTime(CDateTime {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type.to_owned(),
                occurrences: None,
                node_id: nid,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(Vec::new()),
                pattern_constraint: None,
            }),
            "Iso8601_duration" => CObject::CDuration(CDuration {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type.to_owned(),
                occurrences: None,
                node_id: nid,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(Vec::new()),
                pattern_constraint: None,
            }),
            "Terminology_code" => CObject::CTerminologyCode(CTerminologyCode {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type.to_owned(),
                occurrences: None,
                node_id: nid,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: String::new(),
                constraint_status: None,
            }),
            _ => return None,
        };
        Some(obj)
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
mod tests {
    use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
        CComplexObject, CComplexObjectData,
    };
    use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
    use openehr_am::v2_4::aom2::constraint_model::primitive::constraint_status::ConstraintStatus;
    use openehr_base::prelude::{Interval, ProperInterval};

    use crate::parse::{Dialect, parse_definition_body};

    fn parse(body: &str) -> CComplexObject {
        parse_definition_body(body, Dialect::Adl2).unwrap_or_else(|e| panic!("parse failed: {e:?}"))
    }

    fn data(cco: &CComplexObject) -> &CComplexObjectData {
        match cco {
            CComplexObject::CComplexObject(d) => d,
            CComplexObject::CArchetypeRoot(_) => panic!("expected plain complex object"),
        }
    }

    /// The `C_INTEGER` constraint list of the `index`-th attribute's first
    /// child.
    fn integer_constraints(d: &CComplexObjectData, index: usize) -> &[Interval<i32>] {
        match &d.attributes.as_deref().unwrap_or_default()[index]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CInteger(ci) => ci.constraint.as_deref().unwrap_or_default(),
            _ => panic!("expected CInteger"),
        }
    }

    /// The first constraint of the `index`-th attribute, as a proper interval.
    fn integer_proper(
        d: &CComplexObjectData,
        index: usize,
    ) -> &openehr_base::v1_3::foundation_types::interval::proper_interval::ProperIntervalData<i32>
    {
        match &integer_constraints(d, index)[0] {
            Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => pi,
            _ => panic!("expected proper interval"),
        }
    }

    #[test]
    fn integer_forms() {
        let cco = parse(
            "WHOLE[id1] matches {\n\
             a matches {55}\n\
             b matches {10, 20, 30}\n\
             c matches {|0..100|}\n\
             d matches {|>0..<100|}\n\
             e matches {|>=10|}\n\
             f matches {|-10..-5|}\n\
             }",
        );
        let d = data(&cco);
        // a: point 55
        match &integer_constraints(d, 0)[0] {
            Interval::PointInterval(p) => assert_eq!(p.lower, Some(55)),
            Interval::ProperInterval(_) => panic!("expected point"),
        }
        // b: three points
        assert_eq!(integer_constraints(d, 1).len(), 3);
        // c: |0..100|
        let c = integer_proper(d, 2);
        assert_eq!(c.lower, Some(0));
        assert_eq!(c.upper, Some(100));
        assert!(c.lower_included && c.upper_included);
        // d: |>0..<100| exclusive both
        let dd = integer_proper(d, 3);
        assert!(!dd.lower_included && !dd.upper_included);
        // e: |>=10| lower bounded, upper unbounded
        let e = integer_proper(d, 4);
        assert_eq!(e.lower, Some(10));
        assert!(e.lower_included);
        assert!(e.upper_unbounded);
        // f: |-10..-5| negative endpoints
        let f = integer_proper(d, 5);
        assert_eq!(f.lower, Some(-10));
        assert_eq!(f.upper, Some(-5));
    }

    #[test]
    fn string_boolean_and_regex() {
        let cco = parse(
            "WHOLE[id1] matches {\n\
             s1 matches {\"something\"}\n\
             s2 matches {/cardio.*/}\n\
             s3 matches {\"a\", \"b\"}\n\
             b1 matches {True}\n\
             b2 matches {True, False}\n\
             }",
        );
        let d = data(&cco);
        match &d.attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => assert_eq!(cs.constraint, Some(vec!["something".to_owned()])),
            _ => panic!("expected CString"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[1]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => assert_eq!(cs.constraint, Some(vec!["/cardio.*/".to_owned()])),
            _ => panic!("expected CString regex"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[2]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => assert_eq!(cs.constraint.as_ref().map_or(0, Vec::len), 2),
            _ => panic!("expected CString list"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[3]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CBoolean(cb) => assert_eq!(cb.constraint, Some(vec![true])),
            _ => panic!("expected CBoolean"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[4]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CBoolean(cb) => assert_eq!(cb.constraint, Some(vec![true, false])),
            _ => panic!("expected CBoolean list"),
        }
    }

    #[test]
    fn date_time_patterns_and_durations() {
        let cco = parse(
            "WHOLE[id1] matches {\n\
             d1 matches {yyyy-mm-??}\n\
             d2 matches {|2000-01-01..2000-02-01|}\n\
             t1 matches {hh:mm:ss}\n\
             dur1 matches {PWD}\n\
             dur2 matches {PWD/|P38W..P39W4D|}\n\
             dur3 matches {|<=PT1H|}\n\
             }",
        );
        let d = data(&cco);
        match &d.attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CDate(c) => assert_eq!(c.pattern_constraint.as_deref(), Some("yyyy-mm-??")),
            _ => panic!("expected CDate pattern"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[1]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CDate(c) => assert_eq!(c.constraint.as_ref().map_or(0, Vec::len), 1),
            _ => panic!("expected CDate interval"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[2]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CTime(c) => assert_eq!(c.pattern_constraint.as_deref(), Some("hh:mm:ss")),
            _ => panic!("expected CTime pattern"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[3]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CDuration(c) => {
                assert_eq!(c.pattern_constraint.as_deref(), Some("PWD"));
                assert!(c.constraint.as_ref().is_none_or(Vec::is_empty));
            }
            _ => panic!("expected CDuration pattern"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[4]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CDuration(c) => {
                assert_eq!(c.pattern_constraint.as_deref(), Some("PWD"));
                assert_eq!(c.constraint.as_ref().map_or(0, Vec::len), 1);
            }
            _ => panic!("expected CDuration pattern+interval"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[5]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CDuration(c) => assert_eq!(c.constraint.as_ref().map_or(0, Vec::len), 1),
            _ => panic!("expected CDuration interval"),
        }
    }

    #[test]
    fn terminology_codes() {
        let cco = parse(
            "WHOLE[id1] matches {\n\
             a matches {[ac1]}\n\
             b matches {[at0004]}\n\
             c matches {[ac2; at0022]}\n\
             d matches {preferred [at0004]}\n\
             e matches {[ac1@snomed_ct]}\n\
             }",
        );
        let d = data(&cco);
        match &d.attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CTerminologyCode(t) => assert_eq!(t.constraint, "ac1"),
            _ => panic!("expected CTerminologyCode"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[2]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CTerminologyCode(t) => {
                assert_eq!(t.constraint, "ac2");
                assert_eq!(
                    t.assumed_value.as_ref().map(|a| a.code_string.as_str()),
                    Some("at0022")
                );
            }
            _ => panic!("expected CTerminologyCode with assumed"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[3]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CTerminologyCode(t) => {
                assert_eq!(t.constraint_status, Some(ConstraintStatus::Preferred));
            }
            _ => panic!("expected CTerminologyCode with strength"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[4]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CTerminologyCode(t) => assert_eq!(t.constraint, "ac1@snomed_ct"),
            _ => panic!("expected CTerminologyCode with binding"),
        }
    }

    #[test]
    fn regular_primitive_type_object() {
        let cco = parse("WHOLE[id1] matches {\n a matches {\n String [id2]\n }\n}");
        let d = data(&cco);
        match &d.attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => {
                assert_eq!(cs.node_id, "id2");
                assert_eq!(cs.rm_type_name, "String");
                assert!(cs.constraint.as_ref().is_none_or(Vec::is_empty));
            }
            _ => panic!("expected regular primitive CString"),
        }
    }

    #[test]
    fn assumed_values() {
        let cco =
            parse("WHOLE[id1] matches {\n a matches {|0..10|; 5}\n s matches {\"x\"; \"y\"}\n}");
        let d = data(&cco);
        match &d.attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CInteger(ci) => assert_eq!(ci.assumed_value, Some(5.0)),
            _ => panic!("expected CInteger with assumed"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[1]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => assert_eq!(cs.assumed_value.as_deref(), Some("y")),
            _ => panic!("expected CString with assumed"),
        }
    }

    /// `ADL1.4/master05-cadl.adoc` §Regular Expression L696-702: the `^…^` and
    /// the backslash-escaped `/…/` spellings "are equivalent", so the caret form
    /// normalises onto the AOM's `/`-delimited carrier WITH the inner delimiters
    /// escaped — and the result re-parses to itself (parse → print → parse is
    /// lossless; the printer emits `C_STRING.constraint` verbatim).
    #[test]
    fn caret_regex_normalises_losslessly() {
        let cco = parse("WHOLE[id1] matches {\n u matches {^km/h|mi/h^}\n}");
        let printed = match &data(&cco).attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => cs.constraint.as_deref().unwrap_or_default()[0].clone(),
            other => panic!("expected CString regex, got {other:?}"),
        };
        assert_eq!(printed, r"/km\/h|mi\/h/");
        // The chapter's own equivalent slash spelling yields the same carrier…
        let slash = parse(
            r"WHOLE[id1] matches {\n u matches {/km\/h|mi\/h/}\n}"
                .replace("\\n", "\n")
                .as_str(),
        );
        match &data(&slash).attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => {
                assert_eq!(cs.constraint.as_deref().unwrap_or_default()[0], printed);
            }
            other => panic!("expected CString regex, got {other:?}"),
        }
        // …and re-parsing the printed form reproduces it unchanged.
        let again = parse(&format!(
            "WHOLE[id1] matches {{\n u matches {{{printed}}}\n}}"
        ));
        match &data(&again).attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => {
                assert_eq!(cs.constraint.as_deref().unwrap_or_default()[0], printed);
            }
            other => panic!("expected CString regex, got {other:?}"),
        }
    }

    /// `ADL1.4/master05-cadl.adoc` §Constraints on Character L813-825 —
    /// `color_name matches {'r', 'g', 'b'}`. No AOM generation defines a
    /// `C_CHARACTER`, so the carrier is `C_STRING` with the constrained RM type
    /// name kept (see [`Parser::parse_c_character`]).
    #[test]
    fn character_lists_land_on_c_string() {
        let cco = parse(
            "WHOLE[id1] matches {\n\
             a matches {'r'}\n\
             b matches {'r', 'g', 'b'}\n\
             c matches {'r', 'g'; 'r'}\n\
             }",
        );
        let d = data(&cco);
        match &d.attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => {
                assert_eq!(cs.rm_type_name, "Character");
                assert_eq!(cs.constraint, Some(vec!["r".to_owned()]));
            }
            other => panic!("expected CString, got {other:?}"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[1]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => assert_eq!(cs.constraint.as_ref().map_or(0, Vec::len), 3),
            other => panic!("expected CString, got {other:?}"),
        }
        match &d.attributes.as_deref().unwrap_or_default()[2]
            .children
            .as_deref()
            .unwrap_or_default()[0]
        {
            CObject::CString(cs) => assert_eq!(cs.assumed_value.as_deref(), Some("r")),
            other => panic!("expected CString, got {other:?}"),
        }
    }
}
