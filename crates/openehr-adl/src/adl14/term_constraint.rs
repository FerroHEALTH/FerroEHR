// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 terminology-constraint rewrite, shared by the 1.4→2 converter
//! and by [`crate::opt::create_opt`] over an unconverted 1.4 constituent.
//!
//! A 1.4 source keeps its qualified constraints (`[openehr::433]`,
//! `[local::at0168, at0169]`) in `C_TERMINOLOGY_CODE.constraint`. ADL2 spells
//! only a local at- or ac-code there (`ADL2/master04.5-cadl_primitive_types.adoc`
//! §Formal Terminology Constraint), so a single local code becomes its
//! at-code, a code list becomes a minted ac-code value set, and an external
//! code becomes a minted at-code bound to the code's URI. The output form is
//! spec-governed; the rewrite procedure is our own design (no openEHR spec
//! governs 1.4→2 conversion), pinned by the `upgrade_from_14` fixtures.
//!
//! The two callers differ only in the code space a 1.4 local code lands in,
//! which is the [`CodeSpace`] seam: the converter shifts every code into the
//! ADL2 numbering, while the OPT keeps the 1.4 codes as authored, because its
//! template paths and the data in a CDR name the 1.4 node codes.

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;
use openehr_am::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::aom2::terminology::value_set::ValueSet;

use crate::adl14::code_shift::{first_num, highest_ac_number};
use crate::adl14::convert::ConvertConfig;
use crate::adl14::log::ConversionLog;
use crate::adl14::walk::{cco_data_mut, collect_local_value_codes, collect_node_codes};

/// The code space a 1.4 code lands in when its constraint is rewritten.
pub(super) trait CodeSpace {
    /// The ADL2 at-code of a 1.4 `local` value code.
    fn value_code(&mut self, minter: &mut TermMinter<'_>, code: &str) -> String;
    /// The ADL2 ac-code of a bare 1.4 ac constraint reference.
    fn value_set_code(&mut self, minter: &mut TermMinter<'_>, code: &str) -> String;
}

/// The 1.4 codes kept as authored: the code space of an OPT over an
/// unconverted 1.4 constituent.
struct AsAuthored;

impl CodeSpace for AsAuthored {
    fn value_code(&mut self, _: &mut TermMinter<'_>, code: &str) -> String {
        code.to_owned()
    }

    fn value_set_code(&mut self, _: &mut TermMinter<'_>, code: &str) -> String {
        code.to_owned()
    }
}

/// A terminology entry the rewrite minted.
struct Minted {
    code: String,
    text: String,
    description: String,
    /// The `(terminology, uri)` binding of a minted external at-code.
    binding: Option<(String, String)>,
    /// The members of a minted value set.
    value_set_members: Option<Vec<String>>,
}

/// The minting state of one archetype's rewrite: the next free at- and
/// ac-numbers, the entries minted so far, and the log that makes a repeated
/// code or code list reuse its first mint.
pub(super) struct TermMinter<'a> {
    binding_uri_templates: &'a BTreeMap<String, String>,
    pub(super) log: &'a mut ConversionLog,
    next_at: i64,
    next_ac: i64,
    minted: Vec<Minted>,
}

impl<'a> TermMinter<'a> {
    /// A minter that allocates from `at1` and `ac1` until
    /// [`Self::allocate_above`] moves it.
    pub(super) fn new(
        binding_uri_templates: &'a BTreeMap<String, String>,
        log: &'a mut ConversionLog,
    ) -> Self {
        Self {
            binding_uri_templates,
            log,
            next_at: 1,
            next_ac: 1,
            minted: Vec::new(),
        }
    }

    /// Allocates every later at- and ac-code above the given numbers.
    pub(super) fn allocate_above(&mut self, at: i64, ac: i64) {
        self.next_at = at + 1;
        self.next_ac = ac + 1;
    }

    pub(super) fn alloc_at(&mut self) -> String {
        let c = format!("at{}", self.next_at);
        self.next_at += 1;
        c
    }

    pub(super) fn alloc_ac(&mut self) -> String {
        let c = format!("ac{}", self.next_ac);
        self.next_ac += 1;
        c
    }

    /// Convert one 1.4 `C_TERMINOLOGY_CODE.constraint` encoding
    /// (`terminology::code[,code…][;assumed]`, or an already-ADL2 `at/ac` code)
    /// into an ADL2 constraint, minting codes as needed. `owner_text` is the
    /// enclosing element's rubric (for the `(synthesised)` value-set label).
    /// Returns `(constraint, assumed)`.
    fn convert_constraint(
        &mut self,
        codes: &mut impl CodeSpace,
        raw: &str,
        owner_text: &str,
    ) -> (String, Option<String>) {
        // Split off an assumed value (`…;code`).
        let (body, assumed_raw) = match raw.split_once(';') {
            Some((b, a)) => (b, Some(a)),
            None => (raw, None),
        };
        // Not a qualified 1.4 code: a bare ac-code (a 1.4 `CONSTRAINT_REF`
        // reference or a reference-set ac minted by the OPT front end) moves
        // into the code space with its terminology entry (`ac0001`→`ac2` in
        // the converter; the `ac0.K` specialisation form keeps its number —
        // see `shift_code`). Anything else (`at5`) is already ADL2; pass through.
        let Some((terminology, codes_str)) = body.split_once("::") else {
            if AdlCodeDefinitionsData::is_value_set_code(body) {
                let ac = codes.value_set_code(self, body);
                let assumed = assumed_raw.map(|a| codes.value_code(self, a));
                return (ac, assumed);
            }
            return (body.to_owned(), assumed_raw.map(str::to_owned));
        };
        let source_codes: Vec<&str> = codes_str
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let is_local = terminology == "local";
        let mut at_code = |minter: &mut Self, code: &str| {
            if is_local {
                codes.value_code(minter, code)
            } else {
                minter.external_at_code(terminology, code)
            }
        };

        // Map each source code to its ADL2 at-code (a local at-code into the
        // code space; mint for an external code).
        let mut at_codes: Vec<String> = Vec::new();
        for code in &source_codes {
            at_codes.push(at_code(self, code));
        }
        let assumed = assumed_raw.map(|a| at_code(self, a));

        if at_codes.len() == 1 {
            return (at_codes.into_iter().next().unwrap_or_default(), assumed);
        }
        (self.value_set_ac(&at_codes, owner_text), assumed)
    }

    /// The minted ac-code value set for a code LIST, minted on first sight
    /// and reused for an identical member signature (idempotent via the log).
    fn value_set_ac(&mut self, at_codes: &[String], owner_text: &str) -> String {
        let signature = at_codes.join(",");
        if let Some(existing) = self.log.value_set(&signature) {
            return existing.to_owned();
        }
        let ac = self.alloc_ac();
        self.log.record_value_set(&signature, &ac);
        let (text, description) = value_set_rubric(owner_text);
        self.minted.push(Minted {
            code: ac.clone(),
            text,
            description,
            binding: None,
            value_set_members: Some(at_codes.to_vec()),
        });
        ac
    }

    /// The minted at-code for an external `terminology::code`, minting and
    /// recording a binding on first sight (idempotent via the log).
    fn external_at_code(&mut self, terminology: &str, code: &str) -> String {
        let key = format!("{terminology}::{code}");
        if let Some(existing) = self.log.external_at_code(&key) {
            return existing.to_owned();
        }
        let at = self.alloc_at();
        self.log.record_external_at_code(&key, &at);
        let uri = match self.binding_uri_templates.get(terminology) {
            Some(tmpl) => tmpl.replace("{code}", code),
            // Fabricated fallback URI (flagged) where no template is configured.
            None => format!("urn:adl14:{terminology}:{code}"),
        };
        self.minted.push(Minted {
            code: at.clone(),
            // NOTE: no openEHR spec governs 1.4→2 conversion — our own
            // design/extension; the minted rubric reuses the external code
            // as its text, this rewrite resolving no external terminology.
            text: code.to_owned(),
            description: code.to_owned(),
            binding: Some((terminology.to_owned(), uri)),
            value_set_members: None,
        });
        at
    }

    /// Adds every minted term to one language's `term_definitions`.
    pub(super) fn add_terms(&self, terms: &mut BTreeMap<String, ArchetypeTerm>) {
        // NOTE: no openEHR spec governs 1.4→2 conversion — our own
        // design/extension; one rubric serves every language, the rewrite
        // having no translator for per-language rubrics.
        for m in &self.minted {
            terms.insert(
                m.code.clone(),
                ArchetypeTerm {
                    code: m.code.clone(),
                    text: m.text.clone(),
                    description: m.description.clone(),
                    other_items: None,
                },
            );
        }
    }

    /// Adds the binding of every minted external at-code.
    pub(super) fn add_bindings(&self, bindings: &mut BTreeMap<String, BTreeMap<String, String>>) {
        for m in &self.minted {
            if let Some((terminology, uri)) = &m.binding {
                bindings
                    .entry(terminology.clone())
                    .or_default()
                    .insert(m.code.clone(), uri.clone());
            }
        }
    }

    /// Adds one value set per minted ac-code that carries members.
    pub(super) fn add_value_sets(&self, value_sets: &mut BTreeMap<String, ValueSet>) {
        for m in &self.minted {
            if let Some(members) = &m.value_set_members
                && let Ok(members) = openehr_base::containers::NonEmptyVec::new(members.clone())
            {
                value_sets.insert(
                    m.code.clone(),
                    ValueSet {
                        id: m.code.clone(),
                        members,
                    },
                );
            }
        }
    }
}

/// Rewrites every 1.4 terminology constraint of an unconverted archetype's
/// `definition` in place, keeping its 1.4 codes as authored, and adds the
/// at-codes, value sets and bindings it mints to `terminology`.
///
/// Minted codes are allocated above every at- and ac-number the archetype
/// already uses, so none collides with a 1.4 code. A definition with no 1.4
/// constraint is left as it stands.
pub(crate) fn rewrite_as_authored(
    definition: &mut CComplexObject,
    terminology: &mut ArchetypeTerminology,
) {
    let templates = ConvertConfig::default().binding_uri_templates;
    let mut log = ConversionLog::default();
    let mut minter = TermMinter::new(&templates, &mut log);
    minter.allocate_above(
        highest_at_number(definition, terminology),
        highest_ac_number(definition, terminology),
    );
    rewrite_definition(definition, &mut minter, &mut AsAuthored);
    if minter.minted.is_empty() {
        return;
    }
    for terms in terminology.term_definitions.values_mut() {
        minter.add_terms(terms);
    }
    minter.add_bindings(terminology.term_bindings.get_or_insert_with(BTreeMap::new));
    minter.add_value_sets(terminology.value_sets.get_or_insert_with(BTreeMap::new));
    if terminology
        .term_bindings
        .as_ref()
        .is_some_and(BTreeMap::is_empty)
    {
        terminology.term_bindings = None;
    }
    if terminology
        .value_sets
        .as_ref()
        .is_some_and(BTreeMap::is_empty)
    {
        terminology.value_sets = None;
    }
}

/// The highest at-number an unconverted archetype already uses, across its
/// term definitions, its node codes and its local value codes.
fn highest_at_number(definition: &CComplexObject, terminology: &ArchetypeTerminology) -> i64 {
    let mut highest = 0i64;
    let mut track = |code: &str| {
        if AdlCodeDefinitionsData::is_at_code(code)
            && let Some(n) = first_num(code)
        {
            highest = highest.max(n);
        }
    };
    for terms in terminology.term_definitions.values() {
        for code in terms.keys() {
            track(code);
        }
    }
    collect_node_codes(definition, &mut track);
    collect_local_value_codes(definition, &mut track);
    highest
}

/// Rewrites every terminology constraint of `def` in place.
pub(super) fn rewrite_definition(
    def: &mut CComplexObject,
    minter: &mut TermMinter<'_>,
    codes: &mut impl CodeSpace,
) {
    rewrite_cco(def, minter, codes, "");
}

fn rewrite_cco(
    cco: &mut CComplexObject,
    minter: &mut TermMinter<'_>,
    codes: &mut impl CodeSpace,
    owner_text: &str,
) {
    let Some(d) = cco_data_mut(cco) else { return };
    for attr in d.attributes.iter_mut().flatten() {
        for child in attr.children.iter_mut().flatten() {
            rewrite_obj(child, minter, codes, owner_text);
        }
    }
    for tuple in d.attribute_tuples.iter_mut().flatten() {
        rewrite_tuple(tuple, minter, codes, owner_text);
    }
}

/// Rewrites the terminology codes inside one `C_ATTRIBUTE_TUPLE` in place.
///
/// Both halves of the tuple carry constraints: the tuple MEMBERS' child
/// objects, and the tuple ROWS, whose primitive members hold the actual
/// terminology codes (ordinal symbols and the like).
fn rewrite_tuple(
    tuple: &mut CAttributeTuple,
    minter: &mut TermMinter<'_>,
    codes: &mut impl CodeSpace,
    owner_text: &str,
) {
    for member in tuple.members.iter_mut().flatten() {
        for child in member.children.iter_mut().flatten() {
            rewrite_obj(child, minter, codes, owner_text);
        }
    }
    for row in tuple.tuples.iter_mut().flatten() {
        for m in &mut row.members {
            if let CPrimitiveObject::CTerminologyCode(tc) = m {
                rewrite_terminology_code(tc, minter, codes, owner_text);
            }
        }
    }
}

/// Rewrites one `C_TERMINOLOGY_CODE`'s constraint and assumed value in place.
fn rewrite_terminology_code(
    tc: &mut CTerminologyCode,
    minter: &mut TermMinter<'_>,
    codes: &mut impl CodeSpace,
    owner_text: &str,
) {
    let (constraint, assumed) = minter.convert_constraint(codes, &tc.constraint, owner_text);
    tc.constraint = constraint;
    if let Some(a) = assumed {
        tc.assumed_value = Some(openehr_base::prelude::TerminologyCode {
            terminology_id: "local".to_owned(),
            terminology_version: None,
            code_string: a,
            uri: None,
        });
    }
}

fn rewrite_obj(
    obj: &mut CObject,
    minter: &mut TermMinter<'_>,
    codes: &mut impl CodeSpace,
    owner_text: &str,
) {
    match obj {
        CObject::CTerminologyCode(tc) => rewrite_terminology_code(tc, minter, codes, owner_text),
        CObject::CComplexObject(cco) => rewrite_cco(cco, minter, codes, owner_text),
        _ => {}
    }
}

fn value_set_rubric(owner_text: &str) -> (String, String) {
    // The fixtures label a minted value set with the owning element's
    // rubric + " (synthesised)".
    let base = if owner_text.is_empty() {
        "Value set".to_owned()
    } else {
        owner_text.to_owned()
    };
    (
        format!("{base} (synthesised)"),
        format!("{base} (synthesised)"),
    )
}
