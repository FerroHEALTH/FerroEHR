// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cADL reference productions (`cadl2.g4`).
//!
//! `allow_archetype` archetype slots and their include/exclude assertion
//! blocks, `use_archetype` (`C_ARCHETYPE_ROOT`) and `use_node`
//! (`C_COMPLEX_OBJECT_PROXY`) — `ADL2/master04.3-cadl_complex_types.adoc`
//! §Archetype Slots + §Internal References. One `impl` block over the
//! `Parser` state of [`crate::parse`].

use openehr_am::v2_4::aom2::constraint_model::archetype_slot::ArchetypeSlot;
use openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object_proxy::CComplexObjectProxy;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::beom::core::assertion::Assertion;
use openehr_base::prelude::MultiplicityInterval;

use crate::error::{SyntaxError, SyntaxErrorCode};
use crate::parse::{Dialect, PResult, Parser};
use openehr_lang::v1_1::lexer::Token;

/// The variable part of an `ARCHETYPE_SLOT`: everything the source may state
/// after the reference-model type and the node id.
#[derive(Default)]
struct SlotBody {
    /// The ADL2-only `closed` marker (`ARCHETYPE_SLOT.is_closed`).
    is_closed: bool,
    /// A restated `occurrences` interval.
    occurrences: Option<MultiplicityInterval>,
    /// The `include` assertions of the `matches` block.
    includes: Vec<Assertion>,
    /// The `exclude` assertions of the `matches` block.
    excludes: Vec<Assertion>,
}

// ── slots, archetype roots, internal references ───────────────────────────
impl Parser<'_> {
    /// `c_archetype_root : SYM_USE_ARCHETYPE rm_type_id '[' ID_CODE ','
    /// archetype_ref ']' c_occurrences?`.
    pub(crate) fn parse_c_archetype_root(&mut self) -> PResult<CObject> {
        self.pos += 1; // SYM_USE_ARCHETYPE
        let rm_type = self.parse_rm_type_id()?;
        self.expect(
            |t| matches!(t, Token::LBracket),
            SyntaxErrorCode::Suas,
            "expecting '[' after 'use_archetype'",
        )?;
        // NOTE: `cadl2.g4` mandates `'[' ID_CODE ',' archetype_ref ']'`, but the
        // legacy ADL 1.5 form `use_archetype TYPE [archetype_id]` (no id-code)
        // also occurs; accept it with an empty node id (resolved on upgrade).
        let node_id = if matches!(
            self.peek(),
            Some(Token::IdCode(_) | Token::AtCode(_) | Token::RootIdCode(_))
        ) && matches!(self.peek_at(1), Some(Token::SymComma))
        {
            let n = self.parse_node_id()?;
            self.pos += 1; // ','
            n
        } else {
            String::new()
        };
        let archetype_ref = match self.peek().cloned() {
            Some(Token::ArchetypeId(a)) => {
                self.pos += 1;
                a
            }
            _ => {
                return self.err(
                    SyntaxErrorCode::Suaid,
                    "expecting [archetype_id] in 'use_archetype' statement",
                );
            }
        };
        self.expect(
            |t| matches!(t, Token::RBracket),
            SyntaxErrorCode::Suaid,
            "expecting ']' after the archetype id",
        )?;
        let occurrences = if matches!(self.peek(), Some(Token::SymOccurrences)) {
            Some(self.parse_occurrences()?)
        } else {
            None
        };
        Ok(CObject::CComplexObject(CComplexObject::CArchetypeRoot(
            Box::new(CArchetypeRoot {
                parent: None,
                soc_parent: None,
                rm_type_name: rm_type,
                occurrences,
                node_id,
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                attributes: openehr_base::containers::present(Vec::new()),
                attribute_tuples: openehr_base::containers::present(Vec::new()),
                archetype_ref,
            }),
        )))
    }

    /// `c_complex_object_proxy : SYM_USE_NODE rm_type_id '[' ID_CODE ']'
    /// c_occurrences? ADL_PATH`.
    pub(crate) fn parse_c_complex_object_proxy(&mut self) -> PResult<CObject> {
        self.pos += 1; // SYM_USE_NODE
        let rm_type = self.parse_rm_type_id()?;
        // ADL 1.4 `use_node TYPE /path` carries no `[id]` bracket (the converter
        // synthesises one). Accept the missing bracket in the 1.4 dialect;
        // `cadl2.g4` mandates it otherwise. No openEHR spec governs 1.4→2 — see
        // `crate::adl14`.
        let node_id =
            if self.dialect == Dialect::Adl14 && !matches!(self.peek(), Some(Token::LBracket)) {
                String::new()
            } else {
                self.expect(
                    |t| matches!(t, Token::LBracket),
                    SyntaxErrorCode::Sccog,
                    "expecting '[' after 'use_node'",
                )?;
                let n = self.parse_node_id()?;
                self.expect(
                    |t| matches!(t, Token::RBracket),
                    SyntaxErrorCode::Sccog,
                    "expecting ']' after the node id",
                )?;
                n
            };
        let occurrences = if matches!(self.peek(), Some(Token::SymOccurrences)) {
            Some(self.parse_occurrences()?)
        } else {
            None
        };
        let target_path = match self.peek().cloned() {
            Some(Token::AdlPath(p)) => {
                self.pos += 1;
                p
            }
            _ => {
                return self.err(
                    SyntaxErrorCode::Sunpa,
                    "expecting an absolute path in 'use_node' statement",
                );
            }
        };
        Ok(CObject::CComplexObjectProxy(CComplexObjectProxy {
            parent: None,
            soc_parent: None,
            rm_type_name: rm_type,
            occurrences,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            target_path,
        }))
    }

    /// `archetype_slot : SYM_ALLOW_ARCHETYPE rm_type_id '[' ID_CODE ']'
    /// (( c_occurrences? ( SYM_MATCHES '{' c_includes? c_excludes? '}' )? ) |
    /// SYM_CLOSED )`.
    ///
    /// In the ADL 1.4 dialect the `[node_id]` is OPTIONAL: ADL1.4
    /// master05-cadl.adoc §Archetype Slots writes the anonymous form
    /// (`allow_archetype OBSERVATION occurrences ∈ {0..1} ∈ {…}`) in its own
    /// normative examples, and §cADL node types shows the identified form
    /// (`allow_archetype ENTRY[at2002]`) — both are legal 1.4 source. The
    /// AOM 1.4 node-id rule (anonymous where no sibling disambiguation is
    /// needed) is enforced by VCOID in the 1.4 validation pass, not here;
    /// `cadl2.g4` mandates the bracket in ADL 2.
    /// The slot's `'[' ID_CODE ']'` node id, which ADL 1.4 may omit.
    ///
    /// The AOM 1.4 node-id rule (anonymous where no sibling disambiguation is
    /// needed) is enforced by VCOID in the 1.4 validation pass, not here;
    /// `cadl2.g4` mandates the bracket in ADL 2.
    ///
    /// # Errors
    /// A missing bracket in ADL 2, or a malformed node id.
    fn parse_slot_node_id(&mut self) -> PResult<String> {
        if self.dialect == Dialect::Adl14 && !matches!(self.peek(), Some(Token::LBracket)) {
            return Ok(String::new());
        }
        self.expect(
            |t| matches!(t, Token::LBracket),
            SyntaxErrorCode::Sccog,
            "expecting '[' after 'allow_archetype'",
        )?;
        let node_id = self.parse_node_id()?;
        self.expect(
            |t| matches!(t, Token::RBracket),
            SyntaxErrorCode::Sccog,
            "expecting ']' after the node id",
        )?;
        Ok(node_id)
    }

    pub(crate) fn parse_archetype_slot(&mut self) -> PResult<CObject> {
        self.pos += 1; // SYM_ALLOW_ARCHETYPE
        let rm_type = self.parse_rm_type_id()?;
        let node_id = self.parse_slot_node_id()?;

        let body = if matches!(self.peek(), Some(Token::SymClosed)) {
            self.parse_closed_slot_marker()?
        } else {
            self.parse_open_slot_body()?
        };

        Ok(CObject::ArchetypeSlot(ArchetypeSlot {
            parent: None,
            soc_parent: None,
            rm_type_name: rm_type,
            occurrences: body.occurrences,
            node_id,
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            includes: openehr_base::containers::present(body.includes),
            excludes: openehr_base::containers::present(body.excludes),
            is_closed: body.is_closed,
        }))
    }

    /// Consumes the ADL2-only `closed` slot marker.
    ///
    /// `ADL2/master04.3` §Archetype Slots (`ARCHETYPE_SLOT.is_closed`,
    /// redefinition rule VDSSC). The 1.4 cADL keyword set (master05 §Keywords
    /// L51-52) has `allow_archetype` with `include`/`exclude` only, so the
    /// marker is refused in that dialect.
    fn parse_closed_slot_marker(&mut self) -> PResult<SlotBody> {
        if self.dialect == Dialect::Adl14 {
            return self.adl2_only(SyntaxErrorCode::Sccog, "the archetype-slot 'closed' marker");
        }
        self.pos += 1;
        Ok(SlotBody {
            is_closed: true,
            ..Default::default()
        })
    }

    /// Parses an open slot's optional occurrences and its
    /// `matches { include … exclude … }` assertion block.
    fn parse_open_slot_body(&mut self) -> PResult<SlotBody> {
        let mut body = SlotBody::default();
        if matches!(self.peek(), Some(Token::SymOccurrences)) {
            body.occurrences = Some(self.parse_occurrences()?);
        }
        if self.at_negated_matches() {
            return self.negated_matches_reject(SyntaxErrorCode::Sccog);
        }
        if self.eat(|t| matches!(t, Token::SymMatches)) {
            self.expect(
                |t| matches!(t, Token::LCurly),
                SyntaxErrorCode::Sccog,
                "expecting '{' after 'matches' in a slot",
            )?;
            if self.eat(|t| matches!(t, Token::SymInclude)) {
                body.includes.extend(self.parse_slot_assertions()?);
            }
            if self.eat(|t| matches!(t, Token::SymExclude)) {
                body.excludes.extend(self.parse_slot_assertions()?);
            }
            self.expect(
                |t| matches!(t, Token::RCurly),
                SyntaxErrorCode::Sccog,
                "expecting '}' closing the slot body",
            )?;
        }
        Ok(body)
    }

    /// Parse the assertion block after a slot `include`/`exclude` keyword
    /// (`master04.3` §Archetype Slots; cADL grammar `c_includes : SYM_INCLUDE
    /// assertion+`).
    ///
    /// The block is captured as a raw span (the token run to the next
    /// `exclude`/`}` at brace-depth 0) and handed to
    /// [`crate::rules::parse_slot_assertions`], which parses it via the BEL
    /// composition into one or more AOM [`Assertion`] trees
    /// (`EXPR_ARCHETYPE_REF matches EXPR_ARCHETYPE_ID_CONSTRAINT`, `master05`),
    /// each carrying its own rendered `string_expression`; the
    /// `archetype_id/value matches {/regex/}` regex is compile-checked (`SCSRE`).
    /// A block may carry more than one assertion (grammar `assertion+`), so every
    /// parsed assertion is returned.
    fn parse_slot_assertions(&mut self) -> PResult<Vec<Assertion>> {
        let start = self.pos;
        let start_byte = self.cur_span().start;
        let mut end_byte = start_byte;
        let mut depth = 0i32;
        while let Some(tok) = self.peek() {
            match tok {
                Token::LCurly => depth += 1,
                Token::RCurly | Token::SymExclude if depth == 0 => break,
                Token::RCurly => depth -= 1,
                _ => {}
            }
            end_byte = self.cur_span().end;
            self.pos += 1;
        }
        if self.pos == start {
            return self.err(
                SyntaxErrorCode::Sccog,
                "expecting an assertion after 'include'/'exclude'",
            );
        }
        let text = self.src.get(start_byte..end_byte).unwrap_or_default();
        // Parse the assertion tree(s) (`EXPR_ARCHETYPE_REF matches
        // EXPR_ARCHETYPE_ID_CONSTRAINT`, `master05` / `master04.3`) via the BEL
        // AOM composition.
        match crate::rules::parse_slot_assertions(text) {
            Ok(assertions) => Ok(assertions),
            Err(errs) => {
                for e in errs {
                    self.errors.push(SyntaxError::at(
                        e.code,
                        e.message,
                        (e.span.start + start_byte)..(e.span.end + start_byte),
                        self.src,
                    ));
                }
                Err(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
        CComplexObject, CComplexObjectData,
    };
    use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

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
    fn adl14_anonymous_archetype_slot() {
        // ADL1.4 master05-cadl.adoc §Archetype Slots writes the slot WITHOUT
        // a node id in its own normative examples ("allow_archetype
        // OBSERVATION occurrences ∈ {0..1} ∈ { include ... }"); §cADL node
        // types shows the identified form (`allow_archetype ENTRY[at2002]`).
        // Both must parse in the 1.4 dialect; ADL 2 keeps the bracket
        // mandatory (cadl2.g4).
        let cco = parse_definition_body(
            "SECTION[at0000] matches {\n\
             items cardinality matches {0..*; unordered} matches {\n\
             allow_archetype OBSERVATION occurrences matches {0..1} matches {\n\
             include\n\
             archetype_id/value matches {/openEHR-EHR-OBSERVATION\\.bp_measurement\\.v1/}\n\
             }\n\
             allow_archetype ENTRY[at2002] matches {\n\
             include\n\
             archetype_id/value matches {/.*/}\n\
             }\n\
             }\n\
             }",
            Dialect::Adl14,
        )
        .expect("the spec's own anonymous slot form must parse as ADL 1.4");
        let CComplexObject::CComplexObject(d) = &cco else {
            panic!("expected a plain complex object root");
        };
        let items = &d.attributes.as_deref().unwrap_or_default()[0];
        let CObject::ArchetypeSlot(anon) = &items.children.as_deref().unwrap_or_default()[0] else {
            panic!("expected the anonymous slot");
        };
        assert_eq!(anon.rm_type_name, "OBSERVATION");
        assert!(anon.node_id.is_empty(), "anonymous slot has no node id");
        assert_eq!(anon.includes.as_ref().map_or(0, Vec::len), 1);
        let CObject::ArchetypeSlot(named) = &items.children.as_deref().unwrap_or_default()[1]
        else {
            panic!("expected the identified slot");
        };
        assert_eq!(named.node_id, "at2002");

        // The bracket stays MANDATORY in ADL 2 (cadl2.g4 archetype_slot).
        assert!(
            parse_definition_body(
                "SECTION[id1] matches {\n\
                 items cardinality matches {0..*} matches {\n\
                 allow_archetype OBSERVATION occurrences matches {0..1}\n\
                 }\n\
                 }",
                Dialect::Adl2,
            )
            .is_err()
        );
    }

    #[test]
    fn slot_use_node_use_archetype_and_sibling() {
        let cco = parse(
            "SECTION[id1] matches {\n\
             items cardinality matches {0..*} matches {\n\
             allow_archetype OBSERVATION[id2] occurrences matches {0..1} matches {\n\
             include\n\
             archetype_id/value matches {/openEHR-EHR-OBSERVATION\\.foo.*\\.v1/}\n\
             exclude\n\
             archetype_id/value matches {/.*/}\n\
             }\n\
             use_archetype CLUSTER[id3, openEHR-EHR-CLUSTER.device.v1]\n\
             after[id3] use_node ELEMENT[id4] /items[id5]/value\n\
             allow_archetype SECTION[id6] closed\n\
             }\n}",
        );
        let d = data(&cco);
        let children = d.attributes.as_deref().unwrap_or_default()[0]
            .children
            .as_deref()
            .unwrap_or_default();
        // slot
        match &children[0] {
            CObject::ArchetypeSlot(s) => {
                assert_eq!(s.node_id, "id2");
                assert_eq!(s.includes.as_ref().map_or(0, Vec::len), 1);
                assert!(
                    s.includes.as_deref().unwrap_or_default()[0]
                        .string_expression
                        .as_ref()
                        .unwrap()
                        .contains("archetype_id")
                );
                assert_eq!(s.excludes.as_ref().map_or(0, Vec::len), 1);
                assert!(!s.is_closed);
            }
            _ => panic!("expected ArchetypeSlot"),
        }
        // use_archetype -> C_ARCHETYPE_ROOT
        match &children[1] {
            CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => {
                assert_eq!(r.node_id, "id3");
                assert_eq!(r.archetype_ref, "openEHR-EHR-CLUSTER.device.v1");
            }
            _ => panic!("expected CArchetypeRoot"),
        }
        // use_node -> proxy, with a sibling order
        match &children[2] {
            CObject::CComplexObjectProxy(p) => {
                assert_eq!(p.target_path, "/items[id5]/value");
                let so = p.sibling_order.as_ref().expect("sibling order");
                assert!(!so.is_before);
                assert_eq!(so.sibling_node_id, "id3");
            }
            _ => panic!("expected CComplexObjectProxy"),
        }
        // closed slot (id-coded, no matches)
        match &children[3] {
            CObject::ArchetypeSlot(s) => assert!(s.is_closed),
            _ => panic!("expected closed ArchetypeSlot"),
        }
    }
}
