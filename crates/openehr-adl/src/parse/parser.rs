// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cADL structure productions (`cadl2.g4`).
//!
//! The definition root, the type-headed object forms, attributes with their
//! existence/cardinality clauses, the `_default` value
//! (`ADL2/master06-default_values.adoc`), and the second-order
//! attribute/primitive tuples of `ADL2/master04.4-cadl_second_order.adoc` —
//! one `impl` block over the `Parser` state of [`crate::parse`].
//!
//! The object productions dispatch into [`crate::parse::refs`] (archetype
//! slots, archetype roots, internal-reference proxies),
//! [`crate::parse::primitives`] (the inline `C_PRIMITIVE` family) and — in the
//! `Dialect::Adl14` dialect only — [`crate::adl14::lower`] (the 1.4
//! qualified/listed terminology constraints, the pipe-ordinal shorthand and
//! the inline dADL domain blocks).

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_tuple::CPrimitiveTuple;
use openehr_am::v2_4::aom2::constraint_model::sibling_order::SiblingOrder;
use openehr_base::prelude::{Cardinality, MultiplicityInterval};

use crate::aom::access::common_mut;
use crate::aom::build::{
    cobject_to_primitive, complex_object, cstring_regex, into_archetype_root, mult, tuple_member,
};
use crate::error::SyntaxErrorCode;
use crate::odin::{is_interval, odin_to_json};
use crate::parse::{Dialect, PResult, Parser};
use openehr_lang::v1_1::lexer::Token;

// ── productions ───────────────────────────────────────────────────────────
impl Parser<'_> {
    /// The definition root: a single `c_complex_object` (`cadl2.g4`).
    pub(crate) fn parse_root(&mut self) -> PResult<CComplexObject> {
        if !matches!(self.peek(), Some(Token::AlphaUcId(_))) {
            return self.err(
                SyntaxErrorCode::Sccog,
                "expected a root object type at the start of the definition",
            );
        }
        let obj = self.parse_type_object()?;
        match obj {
            CObject::CComplexObject(cco) => Ok(cco),
            _ => self.err(
                SyntaxErrorCode::Sccog,
                "definition root must be a complex object",
            ),
        }
    }

    /// `rm_type_id : ALPHA_UC_ID ( '<' rm_type_id ( ',' rm_type_id )* '>' )?`
    /// (`base_expressions.g4` `type_id`); returns the reconstructed type name.
    pub(crate) fn parse_rm_type_id(&mut self) -> PResult<String> {
        let Some(Token::AlphaUcId(base)) = self.peek().cloned() else {
            return self.err(
                SyntaxErrorCode::Sccog,
                "expected a reference-model type name",
            );
        };
        self.pos += 1;
        if !matches!(self.peek(), Some(Token::SymLt)) {
            return Ok(base);
        }
        // Generic type, e.g. `Interval<Quantity>`.
        self.pos += 1;
        let mut s = format!("{base}<");
        loop {
            s.push_str(&self.parse_rm_type_id()?);
            if self.eat(|t| matches!(t, Token::SymComma)) {
                s.push(',');
            } else {
                break;
            }
        }
        self.expect(
            |t| matches!(t, Token::SymGt),
            SyntaxErrorCode::Sccog,
            "expected '>' closing a generic type",
        )?;
        s.push('>');
        Ok(s)
    }

    /// A node id: `ROOT_ID_CODE | ID_CODE | AT_CODE` (`master04.2` dual coding
    /// — at-coded and id-coded archetypes both appear in this position).
    pub(crate) fn parse_node_id(&mut self) -> PResult<String> {
        match self.peek().cloned() {
            Some(Token::RootIdCode(c) | Token::IdCode(c) | Token::AtCode(c)) => {
                self.pos += 1;
                Ok(c)
            }
            _ => self.err(
                SyntaxErrorCode::Sccog,
                "expected a node id code (id-code or at-code)",
            ),
        }
    }

    /// `c_occurrences : SYM_OCCURRENCES SYM_MATCHES '{' multiplicity '}'`.
    pub(crate) fn parse_occurrences(&mut self) -> PResult<MultiplicityInterval> {
        self.pos += 1; // SYM_OCCURRENCES
        self.expect(
            |t| matches!(t, Token::SymMatches),
            SyntaxErrorCode::Soccf,
            "expecting 'matches' after 'occurrences'",
        )?;
        self.expect(
            |t| matches!(t, Token::LCurly),
            SyntaxErrorCode::Soccf,
            "expecting '{' in occurrences expression",
        )?;
        let m = self.parse_multiplicity(SyntaxErrorCode::Soccf)?;
        self.expect(
            |t| matches!(t, Token::RCurly),
            SyntaxErrorCode::Soccf,
            "expecting '}' closing occurrences",
        )?;
        Ok(m)
    }

    /// `multiplicity : INTEGER | '*' | INTEGER '..' ( INTEGER | '*' )`.
    fn parse_multiplicity(&mut self, code: SyntaxErrorCode) -> PResult<MultiplicityInterval> {
        if self.eat(|t| matches!(t, Token::SymStar)) {
            // `*` == 0..* .
            return Ok(mult(Some(0), None, false, true));
        }
        let lo = self.parse_uint(code)?;
        if self.eat(|t| matches!(t, Token::SymIvlSep)) {
            if self.eat(|t| matches!(t, Token::SymStar)) {
                return Ok(mult(Some(lo), None, false, true));
            }
            let up = self.parse_uint(code)?;
            return Ok(mult(Some(lo), Some(up), false, false));
        }
        Ok(mult(Some(lo), Some(lo), false, false))
    }

    fn parse_uint(&mut self, code: SyntaxErrorCode) -> PResult<i32> {
        match self.peek().cloned() {
            Some(Token::Integer(s)) => {
                self.pos += 1;
                // The lexeme and span are already in the pushed diagnostic; a
                // `ParseIntError` adds nothing to it.
                let Ok(v) = s.parse::<i32>() else {
                    self.push(
                        code,
                        format!("invalid integer {s:?}"),
                        self.span_at(self.pos - 1),
                    );
                    return Err(());
                };
                Ok(v)
            }
            _ => self.err(code, "expected an integer"),
        }
    }

    /// A type-headed object: `c_complex_object` or `c_regular_primitive_object`
    /// (`cadl2.g4`). Distinguished by whether the `matches { … }` body (or the
    /// bare, body-less form) holds attribute defs or a single inline primitive.
    /// The `'[' ID_CODE [',' archetype_ref] ']'` node bracket, if present.
    ///
    /// The two-part form is the OPT-inlined `C_ARCHETYPE_ROOT`: a flattened
    /// slot-filler or external reference carrying the full archetype id inside
    /// the bracket and an inline body (OPT2 master03 §Artefact Structure +
    /// §Flattening; the same shape as `cadl14.g4` `c_archetype_root`).
    ///
    /// NOTE: `cadl2.g4` requires `'[' ID_CODE ']'`, but a *missing* node id is
    /// a semantic defect (VCOID, `AOM2/master08`), not a syntax error, so an
    /// absent `[…]` yields an empty node id that validation flags.
    ///
    /// # Errors
    /// A malformed bracket — a missing archetype id after `,`, or a missing
    /// closing `]`.
    fn parse_node_bracket(&mut self) -> PResult<(String, Option<String>)> {
        if !self.eat(|t| matches!(t, Token::LBracket)) {
            return Ok((String::new(), None));
        }
        let node_id = self.parse_node_id()?;
        let mut archetype_ref: Option<String> = None;
        if self.eat(|t| matches!(t, Token::SymComma)) {
            match self.peek().cloned() {
                Some(Token::ArchetypeId(a)) => {
                    self.pos += 1;
                    archetype_ref = Some(a);
                }
                _ => {
                    return self.err(
                        SyntaxErrorCode::Suaid,
                        "expecting an archetype id after ',' in a node reference",
                    );
                }
            }
        }
        self.expect(
            |t| matches!(t, Token::RBracket),
            SyntaxErrorCode::Sccog,
            "expecting ']' after the node id",
        )?;
        Ok((node_id, archetype_ref))
    }

    /// The `matches { … }` body of a type-headed object: the deprecated
    /// `matches {*}` any-form (`master04.2`), a single inline primitive
    /// constraint, or an attribute-definition body.
    ///
    /// # Errors
    /// An empty body, an unterminated body, or the nested constraint's own
    /// syntax errors.
    fn parse_matches_body(&mut self, rm_type: &str, node_id: &str) -> PResult<CObject> {
        self.pos += 1; // SYM_MATCHES
        self.expect(
            |t| matches!(t, Token::LCurly),
            SyntaxErrorCode::Scoat,
            "expecting '{' after 'matches'",
        )?;
        if matches!(self.peek(), Some(Token::RCurly)) {
            let span = self.cur_span();
            self.push(
                SyntaxErrorCode::Scoat,
                "expecting attribute definition(s)",
                span,
            );
            return Err(());
        }
        if self.eat(|t| matches!(t, Token::SymStar)) {
            self.expect(
                |t| matches!(t, Token::RCurly),
                SyntaxErrorCode::Scoat,
                "expecting '}' after '*'",
            )?;
            return Ok(complex_object(
                rm_type.to_owned(),
                node_id.to_owned(),
                Vec::new(),
                Vec::new(),
                None,
            ));
        }
        if self.body_is_inline_primitive() {
            let mut prim = self.parse_c_inline_primitive(node_id.to_owned())?;
            self.expect(
                |t| matches!(t, Token::RCurly),
                SyntaxErrorCode::Scas,
                "expecting '}' after the primitive constraint",
            )?;
            // A regular primitive object carries the declared RM type name.
            rm_type.clone_into(common_mut(&mut prim).0);
            return Ok(prim);
        }
        let (attrs, tuples, default) = self.parse_object_body()?;
        self.expect(
            |t| matches!(t, Token::RCurly),
            SyntaxErrorCode::Scoat,
            "expecting '}' closing the object body",
        )?;
        Ok(complex_object(
            rm_type.to_owned(),
            node_id.to_owned(),
            attrs,
            tuples,
            default,
        ))
    }

    fn parse_type_object(&mut self) -> PResult<CObject> {
        let type_span = self.cur_span();
        let rm_type = self.parse_rm_type_id()?;
        let (node_id, archetype_ref) = self.parse_node_bracket()?;
        let occurrences = if matches!(self.peek(), Some(Token::SymOccurrences)) {
            Some(self.parse_occurrences()?)
        } else {
            None
        };

        if self.at_negated_matches() {
            return self.negated_matches_reject(SyntaxErrorCode::Sccog);
        }
        let mut obj = if matches!(self.peek(), Some(Token::SymMatches)) {
            self.parse_matches_body(&rm_type, &node_id)?
        } else if let Some(prim) = Self::primitive_any(&rm_type, &node_id) {
            // Body-less primitive type (e.g. `String[id2]`): an unconstrained
            // primitive object (`master04.5` regular primitive form).
            prim
        } else {
            // Body-less complex object: an "any" complex constraint.
            complex_object(
                rm_type.clone(),
                node_id.clone(),
                Vec::new(),
                Vec::new(),
                None,
            )
        };

        // An `archetype_ref` in the node bracket makes this a `C_ARCHETYPE_ROOT`
        // (OPT-inlined filler / external reference), carrying the body as its
        // flattened structure (OPT2 master03 §Flattening).
        if let Some(archetype_ref) = archetype_ref {
            obj = into_archetype_root(obj, archetype_ref);
        }
        if occurrences.is_some() {
            *common_mut(&mut obj).2 = occurrences;
        }
        // Ensure the reconstructed type name / node id survive on complex objs.
        {
            let (ty, nid, _, _) = common_mut(&mut obj);
            if ty.is_empty() {
                *ty = rm_type;
            }
            if nid.is_empty() {
                *nid = node_id;
            }
        }
        let _ = type_span;
        Ok(obj)
    }

    /// The body of a complex object: `c_attribute_def+ default_value?`.
    /// Returns `(attributes, attribute_tuples, default_value)`.
    fn parse_object_body(
        &mut self,
    ) -> PResult<(
        Vec<CAttribute>,
        Vec<CAttributeTuple>,
        Option<serde_json::Value>,
    )> {
        let mut attrs = Vec::new();
        let mut tuples = Vec::new();
        let mut default = None;
        while !matches!(self.peek(), Some(Token::RCurly) | None) {
            match self.peek() {
                Some(Token::AlphaUnderscoreId(s)) if s == "_default" => {
                    // ADL2-only: the `_default` pseudo-attribute is introduced by
                    // `ADL2/master06-default_values.adoc` §Default Values; the 1.4
                    // cADL keyword set (master05 §Keywords L48-53) has no such
                    // construct and no 1.4 chapter defines default values.
                    if self.dialect == Dialect::Adl14 {
                        return self
                            .adl2_only(SyntaxErrorCode::Scoat, "the '_default' pseudo-attribute");
                    }
                    default = Some(self.parse_default_value()?);
                    break; // default_value is last in the body (`cadl2.g4`).
                }
                Some(Token::LBracket) => {
                    // ADL2-only: second-order attribute tuples
                    // (`ADL2/master04.4-cadl_second_order.adoc` §Second Order
                    // Constraints). ADL 1.4 expresses the same co-constraint with
                    // an inline dADL domain type — `AOM2/master04.3` §Tuple
                    // Constraints: "The tuple constraint type replaces all
                    // domain-specific constraint types defined in ADL/AOM 1.4,
                    // including `C_DV_QUANTITY` and `C_DV_ORDINAL`."
                    if self.dialect == Dialect::Adl14 {
                        return self.adl2_only(
                            SyntaxErrorCode::Scoat,
                            "a second-order attribute tuple '[attr, …] matches {…}'",
                        );
                    }
                    tuples.push(self.parse_c_attribute_tuple()?);
                }
                _ => attrs.push(self.parse_c_attribute()?),
            }
        }
        Ok((attrs, tuples, default))
    }

    /// `c_attribute : (ADL_PATH | rm_attribute_id) c_existence? c_cardinality?
    /// ( SYM_MATCHES ( '{' c_objects '}' | CONTAINED_REGEXP) )?`.
    fn parse_c_attribute(&mut self) -> PResult<CAttribute> {
        let (rm_attribute_name, differential_path) = match self.peek().cloned() {
            Some(Token::AdlPath(p)) => {
                self.pos += 1;
                let (parent, name) = split_diff_path(&p);
                (name, Some(parent))
            }
            Some(Token::AlphaLcId(a)) => {
                self.pos += 1;
                (a, None)
            }
            _ => {
                return self.err(
                    SyntaxErrorCode::Scoat,
                    "expecting an attribute name or differential path",
                );
            }
        };

        let existence = if matches!(self.peek(), Some(Token::SymExistence)) {
            Some(self.parse_existence()?)
        } else {
            None
        };
        let cardinality = if matches!(self.peek(), Some(Token::SymCardinality)) {
            Some(self.parse_cardinality()?)
        } else {
            None
        };
        let is_multiple = cardinality.is_some();

        if self.at_negated_matches() {
            return self.negated_matches_reject(SyntaxErrorCode::Scoat);
        }
        let children = if self.eat(|t| matches!(t, Token::SymMatches)) {
            if let Some(Token::ContainedRegexp(raw)) = self.peek().cloned() {
                // `attr matches {/re/}` — a C_STRING regex shortcut (`cadl2.g4`).
                self.pos += 1;
                let span = self.span_at(self.pos - 1);
                let (regex, assumed) = self.contained_regexp_parts(&raw, span)?;
                vec![CObject::CString(cstring_regex(regex, assumed))]
            } else {
                self.expect(
                    |t| matches!(t, Token::LCurly),
                    SyntaxErrorCode::Scas,
                    "expecting '{' or a contained regexp after 'matches'",
                )?;
                let objs = self.parse_c_objects_body()?;
                self.expect(
                    |t| matches!(t, Token::RCurly),
                    SyntaxErrorCode::Scas,
                    "expecting '}' closing the attribute body",
                )?;
                objs
            }
        } else {
            Vec::new()
        };

        Ok(CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name,
            existence,
            children: openehr_base::containers::present(children),
            differential_path,
            cardinality,
            is_multiple,
        })
    }

    /// `c_existence : SYM_EXISTENCE SYM_MATCHES '{' existence '}'`;
    /// `existence : INTEGER | INTEGER '..' INTEGER`. Validated per the
    /// `SEXL*` rules (`master04.6`): only `{0}`,`{0..0}`,`{0..1}`,`{1}`,`{1..1}`.
    fn parse_existence(&mut self) -> PResult<MultiplicityInterval> {
        self.pos += 1; // SYM_EXISTENCE
        self.expect(
            |t| matches!(t, Token::SymMatches),
            SyntaxErrorCode::Sexlmg,
            "expecting 'matches' after 'existence'",
        )?;
        self.expect(
            |t| matches!(t, Token::LCurly),
            SyntaxErrorCode::Sexlmg,
            "expecting '{' in existence expression",
        )?;
        let body_span = self.cur_span();
        let lo = self.parse_uint(SyntaxErrorCode::Sexlmg)?;
        let hi = if self.eat(|t| matches!(t, Token::SymIvlSep)) {
            self.parse_uint(SyntaxErrorCode::Sexlmg)?
        } else {
            lo
        };
        self.expect(
            |t| matches!(t, Token::RCurly),
            SyntaxErrorCode::Sexlmg,
            "expecting '}' closing existence",
        )?;
        // Validity of the existence interval (`master04.6` §SEXL*).
        if lo == hi {
            if lo != 0 && lo != 1 {
                self.push(
                    SyntaxErrorCode::Sexlsg,
                    "existence single value must be 0 or 1",
                    body_span,
                );
                return Err(());
            }
        } else if lo == 0 {
            if hi != 1 {
                self.push(
                    SyntaxErrorCode::Sexlu1,
                    "existence upper limit must be 0 or 1 when lower is 0",
                    body_span,
                );
                return Err(());
            }
        } else if lo == 1 {
            if hi != 1 {
                self.push(
                    SyntaxErrorCode::Sexlu2,
                    "existence upper limit must be 1 when lower is 1",
                    body_span,
                );
                return Err(());
            }
        } else {
            self.push(
                SyntaxErrorCode::Sexlmg,
                "existence must be one of {0..0}, {0..1}, {1..1}",
                body_span,
            );
            return Err(());
        }
        Ok(mult(Some(lo), Some(hi), false, false))
    }

    /// `c_cardinality : SYM_CARDINALITY SYM_MATCHES '{' cardinality '}'`;
    /// `cardinality : multiplicity ( multiplicity_mod multiplicity_mod? )?`.
    fn parse_cardinality(&mut self) -> PResult<Cardinality> {
        self.pos += 1; // SYM_CARDINALITY
        self.expect(
            |t| matches!(t, Token::SymMatches),
            SyntaxErrorCode::Soccf,
            "expecting 'matches' after 'cardinality'",
        )?;
        self.expect(
            |t| matches!(t, Token::LCurly),
            SyntaxErrorCode::Soccf,
            "expecting '{' in cardinality expression",
        )?;
        let interval = self.parse_multiplicity(SyntaxErrorCode::Soccf)?;
        // NOTE: `ADL2/master04.3` §Cardinality — a `cardinality` clause that
        // omits the ordering/uniqueness modifier defaults to an ordered,
        // non-unique container, applied verbatim here.
        let mut is_ordered = true;
        let mut is_unique = false;
        for _ in 0..2 {
            if self.eat(|t| matches!(t, Token::SymSemiColon)) {
                match self.peek() {
                    Some(Token::SymOrdered) => {
                        is_ordered = true;
                        self.pos += 1;
                    }
                    Some(Token::SymUnordered) => {
                        is_ordered = false;
                        self.pos += 1;
                    }
                    Some(Token::SymUnique) => {
                        is_unique = true;
                        self.pos += 1;
                    }
                    _ => {
                        return self.err(
                            SyntaxErrorCode::Soccf,
                            "expecting 'ordered', 'unordered', or 'unique' cardinality modifier",
                        );
                    }
                }
            } else {
                break;
            }
        }
        self.expect(
            |t| matches!(t, Token::RCurly),
            SyntaxErrorCode::Soccf,
            "expecting '}' closing cardinality",
        )?;
        Ok(Cardinality {
            interval,
            is_ordered,
            is_unique,
        })
    }

    /// `default_value : SYM_DEFAULT SYM_EQ '<' odin_text '>'` — the ODIN body
    /// is parsed by `openehr_lang::v1_1::odin` and converted to canonical JSON.
    fn parse_default_value(&mut self) -> PResult<serde_json::Value> {
        self.pos += 1; // SYM_DEFAULT (`_default`)
        self.expect(
            |t| matches!(t, Token::SymEq),
            SyntaxErrorCode::Sadf,
            "expecting '=' after '_default'",
        )?;
        // The optional RM-type cast of the typed form
        // `_default = (DV_CODED_TEXT) < … >` (`master06-default_values.adoc`
        // §Syntax); it lands as the value's `_type` in the canonical JSON.
        let mut cast: Option<String> = None;
        if self.eat(|t| matches!(t, Token::LParen)) {
            let Some(Token::AlphaUcId(name)) = self.peek() else {
                return self.err(
                    SyntaxErrorCode::Sadf,
                    "expecting an RM type name in the '_default = (TYPE)' cast",
                );
            };
            cast = Some(name.clone());
            self.pos += 1;
            self.expect(
                |t| matches!(t, Token::RParen),
                SyntaxErrorCode::Sadf,
                "expecting ')' closing the '_default' type cast",
            )?;
        }
        // The whole `<…>` block, delimiters included, is what the ODIN reader
        // is handed: `odin_text` is `attr_vals | object_value_block`
        // (`LANG/docs/odin/master05-content.adoc` §General Structure), so a
        // block whose content is a bare leaf or interval (`<|0..5|>`, `<5>`)
        // only parses with its delimiters in place.
        let start_byte = self.cur_span().start;
        self.expect(
            |t| matches!(t, Token::SymLt),
            SyntaxErrorCode::Sadf,
            "expecting '<' opening the default value",
        )?;
        // Capture the balanced `<…>` span by `<`/`>` depth. A `<` or `>` inside
        // an interval is a bound operator, not a block delimiter
        // (`LANG/docs/odin/master07-leaf_data.adoc` §Intervals of Ordered
        // Primitive Types: `|>N..<M|`), so the `|…|` pairs are tracked and
        // their contents skipped.
        let mut depth = 1usize;
        let mut end_byte = start_byte;
        let mut in_interval = false;
        while depth > 0 {
            match self.bump() {
                Some(Token::SymIvlDelim) => in_interval = !in_interval,
                Some(Token::SymLt) if !in_interval => depth += 1,
                Some(Token::SymGt) if !in_interval => {
                    depth -= 1;
                    if depth == 0 {
                        end_byte = self.span_at(self.pos - 1).end;
                    }
                }
                Some(_) => {}
                None => return self.err(SyntaxErrorCode::Sadf, "unterminated default value block"),
            }
        }
        let text = self.src.get(start_byte..end_byte).unwrap_or_default();
        match openehr_lang::v1_1::odin::parse(text) {
            Ok(v) => {
                let mut json = match odin_to_json(&v) {
                    Ok(json) => json,
                    Err(e) => {
                        self.push(SyntaxErrorCode::Sdinv, e.to_string(), start_byte..end_byte);
                        return Err(());
                    }
                };
                // An interval already carries its own canonical `_type`
                // (`Point_interval`/`Proper_interval`); the cast on one names
                // the generic slot type (`Interval<Quantity>`), which is not a
                // canonical-JSON class tag.
                if let Some(t) = cast
                    && !is_interval(&v)
                    && let serde_json::Value::Object(m) = &mut json
                {
                    m.insert("_type".to_owned(), serde_json::Value::String(t));
                }
                Ok(json)
            }
            Err(e) => {
                self.push(
                    SyntaxErrorCode::Sadf,
                    format!("invalid ODIN default value: {e}"),
                    start_byte..end_byte,
                );
                Err(())
            }
        }
    }

    /// `c_attribute_tuple : '[' rm_attribute_id (',' rm_attribute_id)* ']'
    /// SYM_MATCHES '{' c_primitive_tuple (',' c_primitive_tuple)* '}'`.
    fn parse_c_attribute_tuple(&mut self) -> PResult<CAttributeTuple> {
        self.pos += 1; // '['
        let mut members = Vec::new();
        loop {
            let Some(Token::AlphaLcId(a)) = self.peek().cloned() else {
                return self.err(
                    SyntaxErrorCode::Scoat,
                    "expecting an attribute name in a tuple header",
                );
            };
            self.pos += 1;
            members.push(tuple_member(a));
            if !self.eat(|t| matches!(t, Token::SymComma)) {
                break;
            }
        }
        self.expect(
            |t| matches!(t, Token::RBracket),
            SyntaxErrorCode::Scoat,
            "expecting ']' closing the tuple header",
        )?;
        self.expect(
            |t| matches!(t, Token::SymMatches),
            SyntaxErrorCode::Scoat,
            "expecting 'matches' after the tuple header",
        )?;
        self.expect(
            |t| matches!(t, Token::LCurly),
            SyntaxErrorCode::Scoat,
            "expecting '{' opening the tuple values",
        )?;
        let mut tuples = Vec::new();
        loop {
            tuples.push(self.parse_c_primitive_tuple()?);
            if !self.eat(|t| matches!(t, Token::SymComma)) {
                break;
            }
        }
        self.expect(
            |t| matches!(t, Token::RCurly),
            SyntaxErrorCode::Scoat,
            "expecting '}' closing the tuple values",
        )?;
        Ok(CAttributeTuple {
            members: openehr_base::containers::present(members),
            tuples: openehr_base::containers::present(tuples),
        })
    }

    /// `c_primitive_tuple : '[' c_primitive_tuple_item (',' …)* ']'`;
    /// `c_primitive_tuple_item : '{' c_inline_primitive_object '}' |
    /// CONTAINED_REGEXP`.
    fn parse_c_primitive_tuple(&mut self) -> PResult<CPrimitiveTuple> {
        self.expect(
            |t| matches!(t, Token::LBracket),
            SyntaxErrorCode::Scoat,
            "expecting '[' opening a tuple row",
        )?;
        let mut items = Vec::new();
        loop {
            if let Some(Token::ContainedRegexp(raw)) = self.peek().cloned() {
                self.pos += 1;
                let span = self.span_at(self.pos - 1);
                let (regex, assumed) = self.contained_regexp_parts(&raw, span)?;
                items.push(CPrimitiveObject::CString(cstring_regex(regex, assumed)));
            } else {
                self.expect(
                    |t| matches!(t, Token::LCurly),
                    SyntaxErrorCode::Scoat,
                    "expecting '{' opening a tuple item",
                )?;
                let obj = self.parse_c_inline_primitive("Primitive_node_id".to_owned())?;
                self.expect(
                    |t| matches!(t, Token::RCurly),
                    SyntaxErrorCode::Scoat,
                    "expecting '}' closing a tuple item",
                )?;
                let prim = cobject_to_primitive(&obj).ok_or(())?;
                items.push(prim);
            }
            if !self.eat(|t| matches!(t, Token::SymComma)) {
                break;
            }
        }
        self.expect(
            |t| matches!(t, Token::RBracket),
            SyntaxErrorCode::Scoat,
            "expecting ']' closing a tuple row",
        )?;
        // `C_PRIMITIVE_TUPLE.members` is `1..*`; the loop above requires at
        // least one item before the closing bracket, so an empty row is a
        // syntax error rather than an empty tuple.
        let Ok(members) = openehr_base::containers::NonEmptyVec::new(items) else {
            let span = self.cur_span();
            self.push(
                SyntaxErrorCode::Scoat,
                "a tuple row must state at least one value",
                span,
            );
            return Err(());
        };
        Ok(CPrimitiveTuple { members })
    }

    /// `c_objects : c_regular_object_ordered+ | c_inline_primitive_object`,
    /// with the empty case raising `SCAS`.
    fn parse_c_objects_body(&mut self) -> PResult<Vec<CObject>> {
        if matches!(self.peek(), Some(Token::RCurly)) {
            return self.err(
                SyntaxErrorCode::Scas,
                "expecting a 'any', 'leaf', or new node definition",
            );
        }
        if self.eat(|t| matches!(t, Token::SymStar)) {
            return Ok(Vec::new()); // deprecated `matches {*}` == any.
        }
        if let Some(objs) = self.parse_c_objects_whole_body()? {
            return Ok(objs);
        }
        let mut objs = Vec::new();
        while !matches!(self.peek(), Some(Token::RCurly) | None) {
            match self.parse_adl14_sibling()? {
                Some(alternatives) => objs.extend(alternatives),
                None => objs.push(self.parse_c_regular_object_ordered()?),
            }
        }
        Ok(objs)
    }

    /// The forms that constrain the WHOLE `matches {…}` body as one object.
    ///
    /// [`None`] means the body is a sibling list to be walked object by object.
    ///
    /// # Errors
    /// Propagates the sub-parser's [`SyntaxError`](crate::error::SyntaxError).
    fn parse_c_objects_whole_body(&mut self) -> PResult<Option<Vec<CObject>>> {
        // 1.4-only (converter front end; no openEHR spec — see `crate::adl14`):
        // a qualified/listed terminology constraint (`[local::at1]`,
        // `[local:: a, b ; c]`, `[openehr::524]`). A single `[local::code]` is
        // one `TermCodeRef` token; a list lexes as `[` ident `::` codes …. Both
        // must reach `parse_adl14_term_object` rather than the ADL2 inline
        // terminology-code path (which expects a bare `[at1]`/`[ac1]`).
        if self.dialect == Dialect::Adl14 && self.is_adl14_qualified_code_start() {
            return Ok(Some(vec![self.parse_adl14_term_object()?]));
        }
        // The 1.4 ordinal shorthand OPENS with a number, so it would otherwise
        // be swallowed by the inline-primitive path as an integer/real
        // constraint and then trip over the `|`. It keeps precedence here (as
        // it always had) and is dispatched per-sibling by the caller's loop.
        let at_adl14_ordinal = self.dialect == Dialect::Adl14 && self.is_adl14_ordinal_start();
        if self.is_inline_primitive_start() && !at_adl14_ordinal {
            return Ok(Some(vec![
                self.parse_c_inline_primitive("Primitive_node_id".to_owned())?,
            ]));
        }
        Ok(None)
    }

    /// One ADL 1.4-only sibling form, when the cursor stands on one.
    ///
    /// The pipe-ordinal shorthand `0|[local::at0005], 1|[…]` (`cadl14.g4`
    /// `c_ordinal`) and an inline dADL domain block are dispatched PER SIBLING
    /// rather than as whole-body special cases: each is one alternative among
    /// the siblings of its block, so it may stand beside a regular complex
    /// object in either order (`ADL1.4/master05-cadl.adoc` §Mixed Structures:
    /// "at any given node, all three types can co-exist"). A domain block whose
    /// `list` rows constrain DIFFERENT member sets lowers to SEVERAL
    /// alternatives, which is why the return is a list; a sibling-order marker
    /// before the block still routes through the single-object shim.
    ///
    /// # Errors
    /// Propagates the sub-parser's [`SyntaxError`](crate::error::SyntaxError).
    fn parse_adl14_sibling(&mut self) -> PResult<Option<Vec<CObject>>> {
        if self.dialect != Dialect::Adl14 {
            return Ok(None);
        }
        if self.is_adl14_ordinal_start() {
            return Ok(Some(vec![self.parse_adl14_ordinal()?]));
        }
        match self.peek() {
            Some(Token::LParen) => Ok(Some(self.parse_adl14_domain_object(true)?)),
            Some(Token::AlphaUcId(_)) if self.is_adl14_domain_block_start() => {
                Ok(Some(self.parse_adl14_domain_object(false)?))
            }
            _ => Ok(None),
        }
    }

    /// The single-object shim over [`Parser::parse_adl14_domain_object`] for
    /// positions that can hold exactly one object (a sibling-order-marked
    /// entry). A block that partitions into several alternatives cannot carry
    /// ONE order marker for all of them, so it is refused loudly.
    fn parse_adl14_domain_single(&mut self, parenthesised: bool) -> PResult<CObject> {
        let mut objs = self.parse_adl14_domain_object(parenthesised)?;
        if objs.len() == 1 {
            return Ok(objs.remove(0));
        }
        self.err(
            SyntaxErrorCode::Sdinv,
            "a domain block whose 'list' rows constrain different member sets lowers to              several alternatives and cannot carry a single sibling-order marker",
        )
    }

    /// `c_regular_object_ordered : sibling_order? c_regular_object`.
    ///
    /// NOTE: the `before`/`after` sibling-order markers are NOT dialect-gated.
    /// `ADL1.4/master05-cadl.adoc` §Keywords L53 lists `before` and `after` among
    /// "the keywords … recognised in cADL" for the 1.4 formalism itself, so a
    /// sibling order in a 1.4 text is legal 1.4 cADL — even though no 1.4 chapter
    /// elaborates its semantics beyond that list. Refusing it here would
    /// contradict the 1.4 keyword set.
    fn parse_c_regular_object_ordered(&mut self) -> PResult<CObject> {
        let sibling = if matches!(self.peek(), Some(Token::SymAfter | Token::SymBefore)) {
            Some(self.parse_sibling_order()?)
        } else {
            None
        };
        let mut obj = self.parse_c_regular_object()?;
        if sibling.is_some() {
            *common_mut(&mut obj).3 = sibling;
        }
        Ok(obj)
    }

    /// `sibling_order : ( SYM_AFTER | SYM_BEFORE ) '[' ID_CODE ']'`.
    fn parse_sibling_order(&mut self) -> PResult<SiblingOrder> {
        let is_before = matches!(self.peek(), Some(Token::SymBefore));
        self.pos += 1;
        self.expect(
            |t| matches!(t, Token::LBracket),
            SyntaxErrorCode::Sccog,
            "expecting '[' after 'before'/'after'",
        )?;
        let sibling_node_id = self.parse_node_id()?;
        self.expect(
            |t| matches!(t, Token::RBracket),
            SyntaxErrorCode::Sccog,
            "expecting ']' after the sibling node id",
        )?;
        Ok(SiblingOrder {
            is_before,
            sibling_node_id,
        })
    }

    /// `c_regular_object : c_complex_object | c_archetype_root |
    /// c_complex_object_proxy | archetype_slot | c_regular_primitive_object`.
    /// One object inside an attribute body.
    ///
    /// NOTE: the `=~` / `!~` operators appear only in defective prose
    /// (`ADL1.4/master05-cadl.adoc` §Regular Expression L691-693 — both
    /// example regexes unterminated) while the chapter's own §Syntax
    /// `c_string_spec` (L1244-1249) and §Symbols lexer define no such token,
    /// so the operator is a typed refusal naming the defect — guessing a
    /// negated-regex semantics from defective prose would be a silent wrong
    /// answer.
    fn parse_c_regular_object(&mut self) -> PResult<CObject> {
        self.nested(Self::parse_c_regular_object_at_level)
    }

    /// [`Self::parse_c_regular_object`] at the level the caller entered.
    fn parse_c_regular_object_at_level(&mut self) -> PResult<CObject> {
        if self.at_regex_match_operator() {
            return self.err(
                SyntaxErrorCode::Sccog,
                "the '=~' / '!~' regex-match operators are not a cADL production; \
                 write the regex constraint directly, as '{/re/}' or '{^re^}'",
            );
        }
        if self.dialect == Dialect::Adl14 {
            // 1.4-only object forms (converter front end; no openEHR spec —
            // see `crate::adl14`): a bare qualified/listed terminology
            // constraint, or an inline dADL domain block `(TYPE) <…>`.
            match self.peek() {
                Some(Token::TermCodeRef(_) | Token::LBracket) => {
                    return self.parse_adl14_term_object();
                }
                Some(Token::LParen) => return self.parse_adl14_domain_single(true),
                // ADL2-only: `use_archetype` (`C_ARCHETYPE_ROOT`). The 1.4 cADL
                // keyword set (master05 §Keywords L51) has `use_node` and
                // `allow_archetype` only — an external archetype reference is
                // expressed in 1.4 by an `allow_archetype` slot whose assertions
                // name the archetype id (master05 §Archetype Slots).
                Some(Token::SymUseArchetype) => {
                    return self.adl2_only(SyntaxErrorCode::Sccog, "'use_archetype'");
                }
                // Bare `C_DV_QUANTITY <…>` / `(C_CODE_PHRASE) <…>` (no parens): a
                // domain type immediately followed by an ODIN block would
                // otherwise be misread as a generic type by `parse_rm_type_id`.
                Some(Token::AlphaUcId(_)) if self.is_adl14_domain_block_start() => {
                    return self.parse_adl14_domain_single(false);
                }
                _ => {}
            }
        }
        match self.peek() {
            Some(Token::SymUseArchetype) => self.parse_c_archetype_root(),
            Some(Token::SymUseNode) => self.parse_c_complex_object_proxy(),
            Some(Token::SymAllowArchetype) => self.parse_archetype_slot(),
            Some(Token::AlphaUcId(_)) => self.parse_type_object(),
            _ => self.err(
                SyntaxErrorCode::Sccog,
                "expecting a new node definition, primitive node, 'use' path, or archetype reference",
            ),
        }
    }
}

/// Split a differential ADL path into `(parent_path, last_attribute_name)`.
fn split_diff_path(p: &str) -> (String, String) {
    match p.rsplit_once('/') {
        Some((parent, name)) => (parent.to_owned(), name.to_owned()),
        None => (String::new(), p.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
        CComplexObject, CComplexObjectData,
    };
    use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
    use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
    use openehr_am::v2_4::aom2::constraint_model::sibling_order::SiblingOrder;
    use openehr_base::prelude::MultiplicityInterval;

    use crate::aom::access::common_mut;
    use crate::error::SyntaxErrorCode;
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

    #[test]
    fn minimal_object_with_node_id() {
        let cco = parse("OBSERVATION[id1]");
        let d = data(&cco);
        assert_eq!(d.rm_type_name, "OBSERVATION");
        assert_eq!(d.node_id, "id1");
        assert!(d.attributes.as_ref().is_none_or(Vec::is_empty));
    }

    #[test]
    fn at_coded_root() {
        let cco = parse("OBSERVATION[at0000] matches {\n value matches { DV_TEXT[at0001] }\n}");
        let d = data(&cco);
        assert_eq!(d.node_id, "at0000");
        assert_eq!(d.attributes.as_ref().map_or(0, Vec::len), 1);
        assert_eq!(
            d.attributes.as_deref().unwrap_or_default()[0].rm_attribute_name,
            "value"
        );
    }

    #[test]
    fn occurrences_and_cardinality() {
        let cco = parse(
            "OBSERVATION[id1] matches {\n\
             items cardinality matches {1..*; unordered} matches {\n\
             ELEMENT[id2] occurrences matches {0..1} matches { value matches { DV_TEXT[id3] } }\n\
             }\n}",
        );
        let d = data(&cco);
        let items = &d.attributes.as_deref().unwrap_or_default()[0];
        assert_eq!(items.rm_attribute_name, "items");
        let card = items.cardinality.as_ref().expect("cardinality");
        assert_eq!(card.interval.lower, Some(1));
        assert!(card.interval.upper_unbounded);
        assert!(!card.is_ordered);
        assert!(items.is_multiple);
        let elem = &items.children.as_deref().unwrap_or_default()[0];
        let (_, node_id, occ, _) = obj_common(elem);
        assert_eq!(node_id, "id2");
        let occ = occ.as_ref().expect("occurrences");
        assert_eq!(occ.lower, Some(0));
        assert_eq!(occ.upper, Some(1));
    }

    fn obj_common(
        obj: &CObject,
    ) -> (
        String,
        String,
        Option<MultiplicityInterval>,
        Option<SiblingOrder>,
    ) {
        // Read-only mirror of `common_mut` for tests.
        let mut obj = obj.clone();
        let (ty, nid, occ, sib) = common_mut(&mut obj);
        (ty.clone(), nid.clone(), occ.clone(), sib.clone())
    }

    #[test]
    fn attribute_and_ordinal_tuples() {
        let cco = parse(
            "OBSERVATION[id1] matches {\n\
             value matches {\n\
             DV_QUANTITY[id2] matches {\n\
             [magnitude, units] matches {\n\
             [{|>=0.0|}, {\"mmol/l\"}],\n\
             [{0.0}, {\"mg/dl\"}]\n\
             }\n}\n}\n\
             ord matches {\n\
             DV_ORDINAL[id3] matches {\n\
             [value, symbol] matches {\n\
             [{0}, {[at11]}],\n\
             [{1}, {[at12]}]\n\
             }\n}\n}\n}",
        );
        let d = data(&cco);
        // DV_QUANTITY under value
        let q = &d.attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0];
        match q {
            CObject::CComplexObject(CComplexObject::CComplexObject(qd)) => {
                assert_eq!(qd.attribute_tuples.as_ref().map_or(0, Vec::len), 1);
                let t = &qd.attribute_tuples.as_deref().unwrap_or_default()[0];
                assert_eq!(t.members.as_ref().map_or(0, Vec::len), 2);
                assert_eq!(
                    t.members.as_deref().unwrap_or_default()[0].rm_attribute_name,
                    "magnitude"
                );
                assert_eq!(t.tuples.as_ref().map_or(0, Vec::len), 2);
                assert_eq!(t.tuples.as_deref().unwrap_or_default()[0].members.len(), 2);
                assert!(matches!(
                    t.tuples.as_deref().unwrap_or_default()[0].members[0],
                    CPrimitiveObject::CReal(_)
                ));
                assert!(matches!(
                    t.tuples.as_deref().unwrap_or_default()[0].members[1],
                    CPrimitiveObject::CString(_)
                ));
            }
            _ => panic!("expected DV_QUANTITY complex object"),
        }
        // DV_ORDINAL tuple with terminology members
        let o = &d.attributes.as_deref().unwrap_or_default()[1]
            .children
            .as_deref()
            .unwrap_or_default()[0];
        match o {
            CObject::CComplexObject(CComplexObject::CComplexObject(od)) => {
                let t = &od.attribute_tuples.as_deref().unwrap_or_default()[0];
                assert!(matches!(
                    t.tuples.as_deref().unwrap_or_default()[0].members[0],
                    CPrimitiveObject::CInteger(_)
                ));
                assert!(matches!(
                    t.tuples.as_deref().unwrap_or_default()[0].members[1],
                    CPrimitiveObject::CTerminologyCode(_)
                ));
            }
            _ => panic!("expected DV_ORDINAL complex object"),
        }
    }

    /// An ODIN interval is a legal `_default` datum, encoded as the canonical
    /// `Interval<T>` object rather than refused.
    ///
    /// `ADL2/master06-default_values.adoc` §Syntax: default values "are
    /// expressed in any regular object instance syntax, including ODIN syntax",
    /// and `LANG/docs/odin/master07-leaf_data.adoc` §Intervals of Ordered
    /// Primitive Types lists intervals among ODIN's leaf DATA forms — so an
    /// interval in a `_default` is an instance value, not a constraint. The
    /// JSON object it lands in is our own design/extension (no openEHR spec
    /// mandates the intermediate shape), mirroring the codec's `Proper_interval`
    /// encoding.
    #[test]
    fn adl2_default_value_encodes_an_interval() {
        let cco = parse(
            "OBSERVATION[id1] matches {\n\
             data matches {\n\
             HISTORY[id2] matches {\n\
             _default = (DV_QUANTITY) <\n\
             units = <\"mm[Hg]\">\n\
             magnitude = <|0.0..5.0|>\n\
             >\n\
             }\n\
             }\n\
             }",
        );
        let history = &data(&cco).attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default()[0];
        let CObject::CComplexObject(CComplexObject::CComplexObject(h)) = history else {
            panic!("expected the HISTORY complex object")
        };
        let default = h
            .default_value
            .as_ref()
            .expect("the `_default` must be read");
        assert_eq!(default["_type"], serde_json::json!("DV_QUANTITY"));
        assert_eq!(default["units"], serde_json::json!("mm[Hg]"));
        assert_eq!(
            default["magnitude"],
            serde_json::json!({
                "_type": "Proper_interval",
                "lower": 0.0,
                "upper": 5.0,
                "lower_unbounded": false,
                "upper_unbounded": false,
                "lower_included": true,
                "upper_included": true,
            })
        );
    }

    #[test]
    fn differential_path_attribute() {
        let cco = parse(
            "OBSERVATION[id1.1] matches {\n\
             /data[id2]/items[id4.1]/value matches { DV_TEXT[id5] }\n\
             }",
        );
        let d = data(&cco);
        let a = &d.attributes.as_deref().unwrap_or_default()[0];
        assert_eq!(a.rm_attribute_name, "value");
        assert_eq!(
            a.differential_path.as_deref(),
            Some("/data[id2]/items[id4.1]")
        );
    }

    #[test]
    fn existence_and_scas_errors() {
        // Existence {5} is invalid -> SEXLSG.
        let errs = parse_definition_body(
            "WHOLE[id1] matches {\n a existence matches {5} matches { DV_TEXT[id2] }\n}",
            Dialect::Adl2,
        )
        .expect_err("should fail");
        assert!(
            errs.iter().any(|e| e.code == SyntaxErrorCode::Sexlsg),
            "{errs:?}"
        );

        // Empty attribute body -> SCAS.
        let errs =
            parse_definition_body("ENTRY[id1] matches {\n value matches {}\n}", Dialect::Adl2)
                .expect_err("should fail");
        assert!(
            errs.iter().any(|e| e.code == SyntaxErrorCode::Scas),
            "{errs:?}"
        );

        // Empty object body -> SCOAT.
        let errs = parse_definition_body(
            "ENTRY[id1] matches {\n value matches { ELEMENT[id2] matches {} }\n}",
            Dialect::Adl2,
        )
        .expect_err("should fail");
        assert!(
            errs.iter().any(|e| e.code == SyntaxErrorCode::Scoat),
            "{errs:?}"
        );
    }
}
