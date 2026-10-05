// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL2 serializer: render an assembled `openehr_am::v2_4::aom2`
//! [`Archetype`] back to ADL2 source text.
//!
//! The printer is the inverse of [`crate::assemble::parse_artefact`] at the
//! model level: `parse_artefact(print(a))` reconstructs a structurally-equal
//! [`Archetype`]. It emits the differential source form with text keyword
//! operators (`matches`), the canonical section order (identification →
//! specialise → language → description → definition → rules → terminology →
//! annotations → `rm_overlay` / `component_terminologies`), ODIN for the ODIN
//! sections, cADL for the definition (every construct [`crate::parse`] parses),
//! and the `rules` via the stored expression tree.
//!
//! Section order is example-derived (`ADL2/master07.04`; the vendored grammar
//! has no top-level ordering production). NOTE: no openEHR spec governs the
//! exact whitespace layout — our own design/extension, chosen so the output
//! re-lexes 1:1.
//!
//! The implementation is one private module per artefact section — `header`
//! (identification / language / description / annotations / `rm_overlay` /
//! `component_terminologies`), `terminology` (the terminology section body),
//! `definition` (the cADL definition section and every primitive-value
//! rendering), `rules` (the `rules` section and the BEL expression printers),
//! and `odin` (the generic ODIN leaf/keyed-list rendering the other sections
//! share). This module keeps only the printer state, the artefact-kind
//! projection, and the top-level section driver; the [`print`](fn@print)
//! function is the crate's serializer seam for a whole artefact, and
//! [`assertion_text`] the one for a single assertion (the string form
//! `ASSERTION.string_expression` carries).

mod definition;
mod header;
mod odin;
mod rules;
mod terminology;

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::rm_overlay::rm_overlay::RmOverlay;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::beom::core::assertion::Assertion;
use openehr_am::v2_4::beom::core::statement_set::StatementSet;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::prelude::{ResourceAnnotations, TerminologyCode, TranslationDetails, Uuid};

/// A refusal from the ADL2 serializer.
///
/// The printer renders only what a released grammar spells; a modelled node
/// with no surface syntax is refused rather than rendered into invented (or
/// empty) text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PrintError {
    /// An `EXTERNAL_QUERY` reached the printer as an assignment source.
    #[error(
        "EXTERNAL_QUERY (assignment to ${target}) has no ADL surface syntax: neither released \
         grammar defines a production for it"
    )]
    ExternalQuery {
        /// Name of the variable the refused assignment targets.
        target: String,
    },
    /// An `EXPR_FUNCTION_CALL` whose leaf `item` carries no string name.
    #[error(
        "EXPR_FUNCTION_CALL without a string name has no ADL surface syntax: the BEL function \
         call production requires an identifier"
    )]
    NamelessFunctionCall,
    /// An `EXPR_VALUE_REF` whose leaf `item` carries no string path.
    #[error(
        "EXPR_VALUE_REF without a string path has no ADL surface syntax: the BEL value \
         reference production requires a path"
    )]
    PathlessValueRef,
}

/// Serializes an assembled [`Archetype`] to ADL2 source text.
///
/// `parse_artefact(&print(a)?)` reconstructs an [`Archetype`] structurally
/// equal to `a` (the round-trip gate).
///
/// # Errors
///
/// Returns [`PrintError`] when the archetype carries a modelled node the
/// released ADL/BEL grammars give no syntax for.
pub fn print(archetype: &Archetype) -> Result<String, PrintError> {
    let mut p = Printer { out: String::new() };
    p.archetype(archetype)?;
    Ok(p.out)
}

/// Renders one [`Assertion`] to its ADL/BEL string form, from its expression
/// tree.
///
/// This is the string form `ASSERTION.string_expression` carries
/// (`LANG/docs/BEL/master04-expression_object_model.adoc` §Core Package: the
/// tree is the root, the string its serialisation), so
/// [`crate::rules::parse_slot_assertions`] fills that attribute through here
/// and `parse → print → parse` stays a fixed point.
///
/// # Errors
///
/// Returns [`PrintError`] when the assertion carries a modelled node the
/// released ADL/BEL grammars give no syntax for — the same refusal
/// [`print`](fn@print) raises, so both serializer seams carry one contract.
pub fn assertion_text(assertion: &Assertion) -> Result<String, PrintError> {
    rules::assertion_str(assertion)
}

/// The output accumulator every section module writes its lines through.
struct Printer {
    out: String,
}

impl Printer {
    fn line(&mut self, depth: usize, s: &str) {
        for _ in 0..depth {
            self.out.push('\t');
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn blank(&mut self) {
        self.out.push('\n');
    }

    // ── artefact ──────────────────────────────────────────────────────────
    fn archetype(&mut self, a: &Archetype) -> Result<(), PrintError> {
        let parts = Parts::of(a);
        self.identification(&parts);
        if let Some(parent) = parts.parent_archetype_id {
            self.blank();
            self.line(0, "specialize");
            self.line(1, parent);
        }
        if !parts.is_overlay {
            self.language(parts.original_language, parts.translations);
            self.description(parts.description);
        }
        self.blank();
        self.line(0, "definition");
        self.definition(parts.definition)?;
        if !parts.rules.is_empty() {
            self.blank();
            self.line(0, "rules");
            for set in parts.rules {
                self.rules(set)?;
            }
        }
        self.terminology_section(parts.terminology);
        if let Some(ann) = parts.annotations {
            self.annotations(ann);
        }
        if let Some(rm) = parts.rm_overlay {
            self.rm_overlay(rm);
        }
        if let Some(ct) = parts.component_terminologies {
            self.component_terminologies(ct);
        }
        for overlay in parts.overlays {
            self.blank();
            self.line(
                0,
                "----------------------------------------------------------------",
            );
            self.blank();
            self.archetype(&Archetype::TemplateOverlay(Box::new(overlay.clone())))?;
        }
        Ok(())
    }
}

// ── the artefact-kind projection ──────────────────────────────────────────

/// A uniform view of the fields the printer needs, over the four
/// [`Archetype`] variants.
struct Parts<'a> {
    keyword: &'a str,
    /// True for a flattened artefact (prints the `flat` keyword prefix,
    /// `ADL2/master07.04` §Artefact declaration): a specialised archetype whose
    /// `is_differential` flag is cleared by the flattener.
    flat: bool,
    is_overlay: bool,
    archetype_id: &'a ArchetypeHrid,
    parent_archetype_id: Option<&'a str>,
    adl_version: Option<&'a str>,
    rm_release: Option<&'a str>,
    uid: Option<&'a Uuid>,
    build_uid: Option<&'a Uuid>,
    is_generated: bool,
    is_controlled: Option<bool>,
    other_meta_data: Option<&'a BTreeMap<String, String>>,
    original_language: &'a TerminologyCode,
    translations: Option<&'a Translations>,
    description: Option<&'a ResourceDescription>,
    definition: &'a CComplexObject,
    rules: &'a [StatementSet],
    terminology: &'a ArchetypeTerminology,
    annotations: Option<&'a ResourceAnnotations>,
    rm_overlay: Option<&'a RmOverlay>,
    component_terminologies: Option<&'a BTreeMap<String, ArchetypeTerminology>>,
    overlays: &'a [openehr_am::v2_4::aom2::archetype::template_overlay::TemplateOverlay],
}

/// The `translations` map of an authored artefact, keyed by language code.
type Translations = BTreeMap<String, TranslationDetails>;

/// A placeholder language for a `TEMPLATE_OVERLAY` (which has no language of its
/// own — it inherits the owner's); the printer never emits it.
static OVERLAY_LANG: std::sync::LazyLock<TerminologyCode> =
    std::sync::LazyLock::new(|| TerminologyCode {
        terminology_id: "ISO_639-1".to_owned(),
        terminology_version: None,
        code_string: "en".to_owned(),
        uri: None,
    });

const NO_OVERLAYS: &[openehr_am::v2_4::aom2::archetype::template_overlay::TemplateOverlay] = &[];

impl<'a> Parts<'a> {
    fn of(a: &'a Archetype) -> Self {
        match a {
            Archetype::TemplateOverlay(o) => Parts {
                keyword: "template_overlay",
                flat: false,
                is_overlay: true,
                archetype_id: &o.archetype_id,
                parent_archetype_id: o.parent_archetype_id.as_deref(),
                adl_version: None,
                rm_release: None,
                uid: None,
                build_uid: None,
                is_generated: false,
                is_controlled: None,
                other_meta_data: None,
                original_language: &OVERLAY_LANG,
                translations: None,
                description: None,
                definition: &o.definition,
                rules: o.rules.as_deref().unwrap_or_default(),
                terminology: &o.terminology,
                annotations: None,
                rm_overlay: o.rm_overlay.as_ref(),
                component_terminologies: None,
                overlays: NO_OVERLAYS,
            },
            Archetype::AuthoredArchetype(inner) => match inner.as_ref() {
                AuthoredArchetype::AuthoredArchetype(d) => Parts {
                    keyword: "archetype",
                    flat: d.parent_archetype_id.is_some() && !d.is_differential,
                    is_overlay: false,
                    archetype_id: &d.archetype_id,
                    parent_archetype_id: d.parent_archetype_id.as_deref(),
                    adl_version: d.adl_version.as_deref(),
                    rm_release: Some(&d.rm_release),
                    uid: d.uid.as_ref(),
                    build_uid: Some(&d.build_uid),
                    is_generated: d.is_generated,
                    is_controlled: d.is_controlled,
                    other_meta_data: Some(&d.other_meta_data),
                    original_language: &d.original_language,
                    translations: d.translations.as_ref(),
                    description: d.description.as_deref(),
                    definition: &d.definition,
                    rules: d.rules.as_deref().unwrap_or_default(),
                    terminology: &d.terminology,
                    annotations: d.annotations.as_ref(),
                    rm_overlay: d.rm_overlay.as_ref(),
                    component_terminologies: None,
                    overlays: NO_OVERLAYS,
                },
                AuthoredArchetype::Template(t) => Parts {
                    keyword: "template",
                    flat: t.parent_archetype_id.is_some() && !t.is_differential,
                    is_overlay: false,
                    archetype_id: &t.archetype_id,
                    parent_archetype_id: t.parent_archetype_id.as_deref(),
                    adl_version: t.adl_version.as_deref(),
                    rm_release: Some(&t.rm_release),
                    uid: t.uid.as_ref(),
                    build_uid: Some(&t.build_uid),
                    is_generated: t.is_generated,
                    is_controlled: t.is_controlled,
                    other_meta_data: Some(&t.other_meta_data),
                    original_language: &t.original_language,
                    translations: t.translations.as_ref(),
                    description: t.description.as_ref(),
                    definition: &t.definition,
                    rules: t.rules.as_deref().unwrap_or_default(),
                    terminology: &t.terminology,
                    annotations: t.annotations.as_ref(),
                    rm_overlay: t.rm_overlay.as_ref(),
                    component_terminologies: None,
                    overlays: t.overlays.as_deref().unwrap_or_default(),
                },
                AuthoredArchetype::OperationalTemplate(o) => Parts {
                    keyword: "operational_template",
                    flat: false,
                    is_overlay: false,
                    archetype_id: &o.archetype_id,
                    parent_archetype_id: o.parent_archetype_id.as_deref(),
                    adl_version: o.adl_version.as_deref(),
                    rm_release: Some(&o.rm_release),
                    uid: o.uid.as_ref(),
                    build_uid: Some(&o.build_uid),
                    is_generated: o.is_generated,
                    is_controlled: o.is_controlled,
                    other_meta_data: Some(&o.other_meta_data),
                    original_language: &o.original_language,
                    translations: o.translations.as_ref(),
                    description: o.description.as_ref(),
                    definition: &o.definition,
                    rules: o.rules.as_deref().unwrap_or_default(),
                    terminology: &o.terminology,
                    annotations: o.annotations.as_ref(),
                    rm_overlay: o.rm_overlay.as_ref(),
                    component_terminologies: o.component_terminologies.as_ref(),
                    overlays: NO_OVERLAYS,
                },
            },
        }
    }
}
