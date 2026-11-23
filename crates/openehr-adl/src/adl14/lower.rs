// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4-only cADL productions, the 1.4→2 converter front end.
//!
//! NOTE: no openEHR spec governs the 1.4 → 2 conversion: our own design.
//!
//! A 1.4 term constraint is kept verbatim in `C_TERMINOLOGY_CODE.constraint` for
//! `crate::adl14::term_constraint` to rewrite; the ordinal shorthand and the
//! inline dADL domain blocks become a `C_COMPLEX_OBJECT` with an attribute
//! tuple (`ADL2/master04.4-cadl_second_order.adoc` §Tuple Constraints).

use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_tuple::CPrimitiveTuple;

use crate::adl14::domain::{
    DomainLoweringError, adl14_code_phrase_parts, is_adl14_domain_type, lower_adl14_domain,
};
use crate::aom::build::{
    ObjectNode, cattr_empty, cobject_to_primitive, complex_object, point_int, point_real,
};
use crate::aom::interval::{point_value_f64, point_value_i32};
use crate::codes::NodeCoding;
use crate::error::SyntaxErrorCode;
use crate::parse::{PResult, Parser};
use openehr_lang::v1_1::lexer::Token;

// ── the ADL 1.4-only object productions ───────────────────────────────────
impl Parser<'_> {
    /// Whether the cursor is at a bare 1.4 inline dADL domain block: a type name,
    /// `<`, then an attribute name (`ADL1.4/master09-customising_adl.adoc`
    /// §Introduction). Matching the shape lets an unsupported domain type reach the
    /// typed refusal of [`Parser::parse_adl14_domain_object`].
    pub(crate) fn is_adl14_domain_block_start(&self) -> bool {
        // An attribute name or an immediate `>` (the empty block, `ADL1.4/master04-dadl.adoc`
        // §Empty Sections); a generic type's parameter list is never empty
        // (`master05-cadl.adoc` §Symbols `V_TYPE_IDENTIFIER`).
        matches!(self.peek(), Some(Token::AlphaUcId(_)))
            && matches!(self.peek_at(1), Some(Token::SymLt))
            && matches!(self.peek_at(2), Some(Token::AlphaLcId(_) | Token::SymGt))
    }

    /// True if the cursor is at a 1.4 qualified/listed terminology constraint
    /// (`[terminology::…]`): either a single-token `TermCodeRef`, or a
    /// `[` ident `::` opening (a code list the lexer split into loose tokens).
    pub(crate) fn is_adl14_qualified_code_start(&self) -> bool {
        match self.peek() {
            Some(Token::TermCodeRef(_)) => true,
            Some(Token::LBracket) => {
                matches!(
                    self.peek_at(1),
                    Some(Token::AlphaLcId(_) | Token::AlphaUcId(_))
                ) && matches!(self.peek_at(2), Some(Token::SymColon))
            }
            _ => false,
        }
    }

    /// 1.4-only: a bare terminology-code object, a single qualified code or a code
    /// list, kept verbatim in `C_TERMINOLOGY_CODE.constraint` (our own design), with
    /// STCDC and STCAC raised by [`Parser::adl14_term_constraint`].
    pub(crate) fn parse_adl14_term_object(&mut self) -> PResult<CObject> {
        let constraint = if let Some(Token::TermCodeRef(raw)) = self.peek().cloned() {
            self.pos += 1;
            // `[terminology::code]` → `terminology::code`.
            raw.trim_start_matches('[').trim_end_matches(']').to_owned()
        } else {
            // `[` terminology `::` code ( `,` code )* ( `;` assumed )? `]`.
            let start = self.cur_span().start;
            self.expect(
                |t| matches!(t, Token::LBracket),
                SyntaxErrorCode::Stccp,
                "expecting '[' opening a terminology code",
            )?;
            let Some(Token::AlphaLcId(terminology) | Token::AlphaUcId(terminology)) = self.bump()
            else {
                return self.err(SyntaxErrorCode::Stccp, "expecting a terminology id");
            };
            // `::`
            self.expect(
                |t| matches!(t, Token::SymColon),
                SyntaxErrorCode::Stccp,
                "expecting ':' in a qualified code",
            )?;
            self.expect(
                |t| matches!(t, Token::SymColon),
                SyntaxErrorCode::Stccp,
                "expecting '::' in a qualified code",
            )?;
            let (codes, assumed) = self.parse_adl14_code_list()?;
            let list_span = start..self.cur_span().end;
            self.expect(
                |t| matches!(t, Token::RBracket),
                SyntaxErrorCode::Stccp,
                "expecting ']' closing a terminology code",
            )?;
            self.adl14_term_constraint(&terminology, &codes, assumed.as_deref(), list_span)?
        };
        Ok(adl14_terminology_code(constraint))
    }

    /// The code list of a 1.4 qualified term constraint, with its optional
    /// `;assumed` value. An empty list (`[openEHR::]`) names only the terminology.
    ///
    /// # Errors
    /// [`SyntaxErrorCode::Stccp`] for a non-code list member or an assumed
    /// value that is not an at-code.
    fn parse_adl14_code_list(&mut self) -> PResult<(Vec<String>, Option<String>)> {
        let mut codes: Vec<String> = Vec::new();
        let mut assumed: Option<String> = None;
        while !matches!(self.peek(), Some(Token::RBracket)) {
            match self.peek().cloned() {
                Some(
                    Token::AtCode(c) | Token::AcCode(c) | Token::IdCode(c) | Token::Integer(c),
                ) => {
                    self.pos += 1;
                    codes.push(c);
                }
                _ => return self.err(SyntaxErrorCode::Stccp, "expecting an at/ac code"),
            }
            if self.eat(|t| matches!(t, Token::SymComma)) {
                continue;
            }
            if self.eat(|t| matches!(t, Token::SymSemiColon)) {
                match self.peek().cloned() {
                    Some(Token::AtCode(a)) => {
                        self.pos += 1;
                        assumed = Some(a);
                    }
                    _ => {
                        return self
                            .err(SyntaxErrorCode::Stccp, "assumed value must be an at-code");
                    }
                }
            }
            break;
        }
        Ok((codes, assumed))
    }

    /// Build the verbatim `terminology::code[,code]*[;assumed]` string of a 1.4 term
    /// constraint, enforcing STCDC and STCAC for both of its spellings
    /// (`ADL1.4/master09-customising_adl.adoc` §Custom Syntax).
    fn adl14_term_constraint(
        &mut self,
        terminology: &str,
        codes: &[String],
        assumed: Option<&str>,
        span: std::ops::Range<usize>,
    ) -> PResult<String> {
        // STCDC: "duplicate code(s) found in code list"
        // (`ADL2/master04.6-cadl_validity_rules.adoc` §Syntax Validity
        // Rules). A repeated member adds no value to the constrained set
        // and is a defect in the source, not a silently-collapsed set.
        let mut seen: Vec<&str> = Vec::with_capacity(codes.len());
        let duplicates: Vec<&str> = codes
            .iter()
            .filter(|c| {
                let dup = seen.contains(&c.as_str());
                seen.push(c.as_str());
                dup
            })
            .map(String::as_str)
            .collect();
        if !duplicates.is_empty() {
            self.push(
                SyntaxErrorCode::Stcdc,
                format!(
                    "duplicate code(s) found in code list: {}",
                    duplicates.join(", ")
                ),
                span,
            );
            return Err(());
        }
        // STCAC: the assumed value must be of the implied type, here the listed set
        // (`ADL1.4/master05-cadl.adoc` §Assumed Values).
        if let Some(a) = assumed
            && !codes.iter().any(|c| c == a)
        {
            self.push(
                SyntaxErrorCode::Stcac,
                format!("assumed value code {a} not found in code list"),
                span,
            );
            return Err(());
        }
        let mut s = format!("{terminology}::{}", codes.join(","));
        if let Some(a) = assumed {
            s.push(';');
            s.push_str(a);
        }
        Ok(s)
    }

    /// Whether the cursor is at the ADL 1.4 pipe-ordinal shorthand, a number
    /// directly followed by `|` (`cadl14.g4` `ordinal_term`).
    pub(crate) fn is_adl14_ordinal_start(&self) -> bool {
        let signed = usize::from(matches!(
            self.peek(),
            Some(Token::SymPlus | Token::SymMinus)
        ));
        matches!(
            self.peek_at(signed),
            Some(Token::Integer(_) | Token::Real(_))
        ) && matches!(self.peek_at(signed + 1), Some(Token::SymIvlDelim))
    }

    /// 1.4-only: the ordinal shorthand `0|[local::at0005], 1|[local::at0006] ; 0`
    /// (`cadl14.g4` `c_ordinal`).
    ///
    /// It lowers to the replacement `ADL2/master04.4` §Tuple Constraints gives: a
    /// `DV_ORDINAL` with one `[value, symbol]` tuple row per term
    /// (`AOM1.4/masterAppA-domain_extension.adoc` §ORDINAL).
    pub(crate) fn parse_adl14_ordinal(&mut self) -> PResult<CObject> {
        let start = self.cur_span().start;
        let mut tuples: Vec<CPrimitiveTuple> = Vec::new();
        loop {
            let is_real = matches!(
                self.peek_at(usize::from(matches!(
                    self.peek(),
                    Some(Token::SymPlus | Token::SymMinus)
                ))),
                Some(Token::Real(_))
            );
            let value = if is_real {
                CPrimitiveObject::CReal(ObjectNode::inline("Real", NodeCoding::At).c_real(
                    vec![point_real(self.parse_signed_real(SyntaxErrorCode::Scrav)?)],
                    None,
                ))
            } else {
                CPrimitiveObject::CInteger(ObjectNode::inline("Integer", NodeCoding::At).c_integer(
                    vec![point_int(i64::from(
                        self.parse_signed_int(SyntaxErrorCode::Sciav)?,
                    ))],
                    None,
                ))
            };
            self.expect(
                |t| matches!(t, Token::SymIvlDelim),
                SyntaxErrorCode::Sccog,
                "expecting '|' between an ordinal value and its symbol",
            )?;
            let symbol = self.parse_adl14_ordinal_symbol()?;
            tuples.push(CPrimitiveTuple {
                // A `value|symbol` ordinal row always has exactly two members,
                // so the `1..*` bound of `C_PRIMITIVE_TUPLE.members` holds by
                // construction here.
                members: {
                    let mut row = openehr_base::containers::NonEmptyVec::of(value);
                    row.push(symbol);
                    row
                },
            });
            if !self.eat(|t| matches!(t, Token::SymComma)) {
                break;
            }
        }
        if self.eat(|t| matches!(t, Token::SymSemiColon)) {
            self.apply_adl14_ordinal_assumed(&mut tuples, start)?;
        }
        Ok(complex_object(
            "DV_ORDINAL".to_owned(),
            String::new(),
            Vec::new(),
            vec![CAttributeTuple {
                members: Some(vec![cattr_empty("value"), cattr_empty("symbol")]),
                tuples: openehr_base::containers::present(tuples),
            }],
            None,
        ))
    }

    /// The symbol half of an ordinal term: `c_terminology_code`
    /// (`cadl14_primitives.g4`), i.e. the qualified `[local::at0005]` /
    /// `[SNOMED-CT::12345]` form or the bare local `[at0005]` / `[ac0001]` form.
    fn parse_adl14_ordinal_symbol(&mut self) -> PResult<CPrimitiveObject> {
        let obj = if self.is_adl14_qualified_code_start() {
            self.parse_adl14_term_object()?
        } else if matches!(self.peek(), Some(Token::LBracket))
            && matches!(self.peek_at(1), Some(Token::AtCode(_) | Token::AcCode(_)))
        {
            self.parse_c_terminology_code(self.node_coding.primitive_node_id(), None)?
        } else {
            return self.err(
                SyntaxErrorCode::Stccp,
                "expecting a terminology code as the ordinal symbol",
            );
        };
        cobject_to_primitive(&obj).map_or_else(
            || {
                self.err(
                    SyntaxErrorCode::Stccp,
                    "the ordinal symbol is not a terminology-code constraint",
                )
            },
            Ok,
        )
    }

    /// Land the `; assumed_ordinal_value` tail on the one ordinal term it names, as
    /// that row's `assumed_value` (`AOM2/master04.2` §`Assumed_value`); a value
    /// naming no term is refused (`ADL1.4/master05-cadl.adoc` §Assumed Values).
    fn apply_adl14_ordinal_assumed(
        &mut self,
        tuples: &mut [CPrimitiveTuple],
        start: usize,
    ) -> PResult<()> {
        let is_real = matches!(
            self.peek_at(usize::from(matches!(
                self.peek(),
                Some(Token::SymPlus | Token::SymMinus)
            ))),
            Some(Token::Real(_))
        );
        let code = if is_real {
            SyntaxErrorCode::Scrav
        } else {
            SyntaxErrorCode::Sciav
        };
        let assumed = if is_real {
            self.parse_signed_real(code)?
        } else {
            f64::from(self.parse_signed_int(code)?)
        };
        let span = start..self.span_at(self.pos.saturating_sub(1)).end;
        let row = tuples.iter_mut().find(|row| {
            row.members
                .first()
                .and_then(ordinal_point_value)
                .is_some_and(|v| (v - assumed).abs() < f64::EPSILON)
        });
        let Some(row) = row else {
            self.push(
                code,
                format!("assumed ordinal value {assumed} is not one of the listed ordinals"),
                span,
            );
            return Err(());
        };
        match row.members.first_mut() {
            Some(CPrimitiveObject::CInteger(c)) => c.assumed_value = Some(assumed),
            Some(CPrimitiveObject::CReal(c)) => c.assumed_value = Some(assumed),
            // Unreachable: `ordinal_point_value` answered `Some` for this member
            // just above, and it answers `Some` only for those two kinds.
            _ => {}
        }
        Ok(())
    }

    /// 1.4-only: an inline dADL domain constraint `(C_DV_QUANTITY) <…>`,
    /// `C_DV_ORDINAL <…>` or `C_CODE_PHRASE <…>`.
    ///
    /// The first two lower to a `C_COMPLEX_OBJECT` with an attribute tuple
    /// (`AOM2/master04.3` §Tuple Constraints: "The tuple constraint type replaces
    /// all domain-specific constraint types"); `C_CODE_PHRASE` to the custom-syntax
    /// `C_TERMINOLOGY_CODE`. Any other domain type is refused with
    /// [`SyntaxErrorCode::Sdinv`], since each targets a different RM type.
    //
    // NOTE: `C_DV_STATE` stays refused: no vendored openEHR text defines it.
    #[expect(
        clippy::too_many_lines,
        reason = "one linear parse: the type name, the ODIN block's token span, then one dispatch per lowered domain type"
    )]
    pub(crate) fn parse_adl14_domain_object(
        &mut self,
        parenthesised: bool,
    ) -> PResult<Vec<CObject>> {
        let start = self.cur_span().start;
        if parenthesised {
            self.pos += 1; // '('
        }
        let Some(Token::AlphaUcId(rm_type)) = self.bump() else {
            return self.err(
                SyntaxErrorCode::Sccog,
                "expecting a domain constrainer type",
            );
        };
        if parenthesised {
            self.expect(
                |t| matches!(t, Token::RParen),
                SyntaxErrorCode::Sccog,
                "expecting ')' after the domain type",
            )?;
        }
        // Depth counts outside `|…|` only (`ADL1.4/master05-cadl.adoc` §Symbols
        // `V_C_DOMAIN_TYPE`: "there can be '>' inside '||' ranges").
        let open = self.pos;
        if !matches!(self.peek(), Some(Token::SymLt)) {
            return self.err(
                SyntaxErrorCode::Sdinv,
                "expecting '<' opening a domain block",
            );
        }
        let mut depth = 0usize;
        let mut in_interval = false;
        let mut close = None;
        while let Some(tok) = self.peek() {
            match tok {
                Token::SymIvlDelim => in_interval = !in_interval,
                Token::SymLt if !in_interval => depth += 1,
                Token::SymGt if !in_interval => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(self.pos);
                        self.pos += 1;
                        break;
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        let Some(close) = close else {
            return self.err(SyntaxErrorCode::Sdinv, "unterminated domain block '<…>'");
        };
        let span = start..self.span_at(close).end;
        if !is_adl14_domain_type(&rm_type) {
            self.push(
                SyntaxErrorCode::Sdinv,
                format!(
                    "inline dADL domain constrainer {rm_type:?} is not supported; only \
                     'C_DV_QUANTITY', 'C_DV_ORDINAL' and 'C_CODE_PHRASE' are lowered"
                ),
                span,
            );
            return Err(());
        }
        let block = self
            .src
            .get(self.span_at(open).start..self.span_at(close).end)
            .unwrap_or_default();
        let Ok(odin) = openehr_lang::v1_1::odin::parse(block) else {
            self.push(SyntaxErrorCode::Sdinv, "invalid dADL in domain block", span);
            return Err(());
        };
        if rm_type == "C_CODE_PHRASE" {
            let parts = match adl14_code_phrase_parts(&odin) {
                Ok(parts) => parts,
                Err(why) => {
                    self.push(
                        SyntaxErrorCode::Sdinv,
                        format!("inline dADL 'C_CODE_PHRASE' block: {why}"),
                        span,
                    );
                    return Err(());
                }
            };
            let constraint = self.adl14_term_constraint(
                &parts.terminology,
                &parts.codes,
                parts.assumed.as_deref(),
                span,
            )?;
            return Ok(vec![adl14_terminology_code(constraint)]);
        }
        match lower_adl14_domain(&rm_type, &odin) {
            Ok(objs) => Ok(objs),
            Err(DomainLoweringError::Empty) => {
                self.push(
                    SyntaxErrorCode::Sdinv,
                    "empty or unsupported inline dADL domain block",
                    span,
                );
                Err(())
            }
            Err(DomainLoweringError::AssumedValueUnmatched(attrs)) => {
                self.push(
                    SyntaxErrorCode::Sdinv,
                    format!(
                        "the domain block's 'assumed_value' ({attrs}) satisfies none of its \
                         'list' rows"
                    ),
                    span,
                );
                Err(())
            }
        }
    }
}

// ── free helpers ──────────────────────────────────────────────────────────

/// The single ordinal value a lowered `[value, symbol]` tuple row constrains,
/// as `f64` for the assumed-value lookup. `None` for any other constraint kind.
fn ordinal_point_value(member: &CPrimitiveObject) -> Option<f64> {
    match member {
        CPrimitiveObject::CInteger(c) => c
            .constraint
            .iter()
            .flatten()
            .next()
            .and_then(point_value_i32)
            .map(f64::from),
        CPrimitiveObject::CReal(c) => c
            .constraint
            .iter()
            .flatten()
            .next()
            .and_then(point_value_f64),
        _ => None,
    }
}

/// A 1.4 term constraint as a `C_TERMINOLOGY_CODE` carrying the verbatim
/// `terminology::code[,code]*[;assumed]` spelling for `crate::adl14::convert`.
fn adl14_terminology_code(constraint: String) -> CObject {
    CObject::CTerminologyCode(
        ObjectNode::inline("Terminology_code", NodeCoding::At)
            .c_terminology_code(constraint, None, None),
    )
}

#[cfg(test)]
mod tests {
    use crate::error::SyntaxErrorCode;
    use crate::parse::{Dialect, parse_definition_body};

    /// An unmodelled inline domain type, `C_DV_STATE` here, is refused by name.
    #[test]
    fn adl14_unsupported_domain_type_is_refused_by_name() {
        let errs = parse_definition_body(
            "ELEMENT[at0000] matches {\n\
             value matches {\n\
             (C_DV_STATE) <\n\
             value = <\"at0001\">\n\
             >\n\
             }\n\
             }",
            Dialect::Adl14,
        )
        .expect_err("an unmodelled domain constrainer must be refused");
        let sdinv = errs
            .iter()
            .find(|e| e.code == SyntaxErrorCode::Sdinv)
            .expect("SDINV");
        assert!(
            sdinv.message.contains("C_DV_STATE"),
            "the message must name the type, got {:?}",
            sdinv.message
        );
    }
}
