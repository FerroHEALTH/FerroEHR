// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Artefact-level views and the parent/supplier repository — the crate-wide
//! access layer over an assembled `openehr_am::v2_4::aom2` [`Archetype`].
//!
//! `ArchetypeView` is the borrowed, artefact-kind-agnostic view of the common
//! fields every `AUTHORED_ARCHETYPE` / `TEMPLATE` / `OPERATIONAL_TEMPLATE` /
//! `TEMPLATE_OVERLAY` carries (`docs/specs/openehr/AM/docs/AOM2/master03-archetype_package.adoc`
//! §Top-level Meta-data + `ADL2/master07.03` §Artefact Categories), so a
//! consumer never matches on the artefact-kind enum
//! itself. [`ArchetypeRepository`] is the minimal in-memory parent/supplier seam
//! that specialisation-aware work resolves against, and [`resolve_flat_parent`]
//! classifies a child's declared parent for it.
//!
//! This module sits BELOW validation, flattening, and OPT generation: all three
//! read through it, and it depends on none of them.

use std::collections::HashMap;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::rm_overlay::rm_overlay::RmOverlay;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::prelude::{ResourceAnnotations, TerminologyCode};

use crate::hrid::{hrid_lookup_key, raw_id_lookup_key};
use crate::source::ArtefactKind;

/// A minimal in-memory archetype repository — the parent/supplier seam the
/// specialisation-aware checks resolve against.
///
/// Keyed on the `publisher-package-class.concept` portion of the HRID (version
/// family and namespace are ignored for lookup), so a child's
/// `parent_archetype_id` (`…redefine_occurrences.v1`) resolves to the parsed
/// parent (`…redefine_occurrences.v1.0.0`).
#[derive(Debug, Default, Clone)]
pub struct ArchetypeRepository {
    by_id: HashMap<String, Archetype>,
}

impl ArchetypeRepository {
    /// A new, empty repository.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a parsed archetype under its HRID key.
    pub fn insert(&mut self, archetype: Archetype) {
        let key = hrid_lookup_key(view(&archetype).archetype_id);
        self.by_id.insert(key, archetype);
    }

    /// Resolve a raw archetype-id reference (as it appears in a
    /// `parent_archetype_id` / external ref) to a registered archetype.
    #[must_use]
    pub fn get(&self, raw_id: &str) -> Option<&Archetype> {
        self.by_id.get(&raw_id_lookup_key(raw_id))
    }
}

/// The outcome of resolving a specialised archetype's flat parent — the input
/// the phase-2 specialisation checks validate against.
///
/// A level-0 (non-specialised) parent is its own flat form, so it is returned
/// [`Available`](FlatParent::Available) directly (`ADL2/master09.02`
/// §Differential and Flat Forms: "For a top-level archetype, the flat-form is
/// the same as its differential form"). A parent that is itself specialised
/// needs its deep flat form (produced by [`crate::flatten::flat_form`]), which is
/// owned rather than borrowable, so it is reported
/// [`NeedsFlattener`](FlatParent::NeedsFlattener) and flattened by the caller.
#[derive(Debug, Clone, Copy)]
pub enum FlatParent<'a> {
    /// The archetype is not specialised — the phase-2 specialisation checks do
    /// not apply.
    NotSpecialised,
    /// The flat parent is available (a level-0 parent used as-is).
    Available(&'a Archetype),
    /// The declared parent is registered but is itself specialised, so its deep
    /// flat form is computed (owned) by the caller via
    /// [`crate::flatten::flat_form`] rather than borrowed here.
    NeedsFlattener,
    /// The declared parent could not be resolved in the repository.
    NotFound,
}

/// Resolve `child`'s flat parent from `repo` for the phase-2 specialisation
/// checks.
///
/// Returns [`FlatParent::NotSpecialised`] for a non-specialised archetype,
/// [`FlatParent::NotFound`] when the declared parent is absent from `repo`,
/// [`FlatParent::NeedsFlattener`] when the parent is itself specialised (its
/// deep flat form is not yet computable), and
/// [`FlatParent::Available`] for a level-0 parent (its own flat form).
#[must_use]
pub fn resolve_flat_parent<'a>(child: &Archetype, repo: &'a ArchetypeRepository) -> FlatParent<'a> {
    let Some(parent_id) = view(child).parent_archetype_id else {
        return FlatParent::NotSpecialised;
    };
    let Some(parent) = repo.get(parent_id) else {
        return FlatParent::NotFound;
    };
    if view(parent).is_specialised() {
        // A borrowed level-0 parent is its own flat form; a specialised parent's
        // deep flat form is owned (computed by [`crate::flatten::flat_form`]) and
        // cannot be handed back by borrow — the caller flattens it there.
        return FlatParent::NeedsFlattener;
    }
    FlatParent::Available(parent)
}

/// A borrowed, artefact-kind-agnostic view of an [`Archetype`]'s common fields
/// — the single access point the checks read through.
pub(crate) struct ArchetypeView<'a> {
    pub(crate) kind: ArtefactKind,
    pub(crate) archetype_id: &'a ArchetypeHrid,
    pub(crate) parent_archetype_id: Option<&'a str>,
    pub(crate) definition: &'a CComplexObject,
    pub(crate) terminology: &'a ArchetypeTerminology,
    pub(crate) rm_overlay: Option<&'a RmOverlay>,
    pub(crate) original_language: Option<&'a TerminologyCode>,
    pub(crate) description: Option<&'a ResourceDescription>,
    pub(crate) translations:
        Option<&'a std::collections::BTreeMap<String, openehr_base::prelude::TranslationDetails>>,
    pub(crate) annotations: Option<&'a ResourceAnnotations>,
    pub(crate) adl_version: Option<&'a str>,
    pub(crate) rm_release: &'a str,
    pub(crate) is_differential: bool,
}

impl ArchetypeView<'_> {
    /// True if this archetype specialises a parent.
    pub(crate) fn is_specialised(&self) -> bool {
        self.parent_archetype_id.is_some()
    }

    /// The archetype's specialisation level = the specialisation depth of its
    /// root node id (`master07` §Specialisation Depth; VARCN).
    pub(crate) fn specialisation_level(&self) -> usize {
        crate::codes::specialisation_depth(crate::aom::access::complex_node_id(self.definition))
            .unwrap_or(0)
    }
}

/// Build an [`ArchetypeView`] over any [`Archetype`] variant.
pub(crate) fn view(archetype: &Archetype) -> ArchetypeView<'_> {
    match archetype {
        Archetype::AuthoredArchetype(a) => match a.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => ArchetypeView {
                kind: ArtefactKind::Archetype,
                archetype_id: &d.archetype_id,
                parent_archetype_id: d.parent_archetype_id.as_deref(),
                definition: &d.definition,
                terminology: &d.terminology,
                rm_overlay: d.rm_overlay.as_ref(),
                original_language: Some(&d.original_language),
                description: d.description.as_deref(),
                translations: d.translations.as_ref(),
                annotations: d.annotations.as_ref(),
                adl_version: d.adl_version.as_deref(),
                rm_release: &d.rm_release,
                is_differential: d.is_differential,
            },
            AuthoredArchetype::Template(t) => ArchetypeView {
                kind: ArtefactKind::Template,
                archetype_id: &t.archetype_id,
                parent_archetype_id: t.parent_archetype_id.as_deref(),
                definition: &t.definition,
                terminology: &t.terminology,
                rm_overlay: t.rm_overlay.as_ref(),
                original_language: Some(&t.original_language),
                description: t.description.as_ref(),
                translations: t.translations.as_ref(),
                annotations: t.annotations.as_ref(),
                adl_version: t.adl_version.as_deref(),
                rm_release: &t.rm_release,
                is_differential: t.is_differential,
            },
            AuthoredArchetype::OperationalTemplate(o) => ArchetypeView {
                kind: ArtefactKind::OperationalTemplate,
                archetype_id: &o.archetype_id,
                parent_archetype_id: o.parent_archetype_id.as_deref(),
                definition: &o.definition,
                terminology: &o.terminology,
                rm_overlay: o.rm_overlay.as_ref(),
                original_language: Some(&o.original_language),
                description: o.description.as_ref(),
                translations: o.translations.as_ref(),
                annotations: o.annotations.as_ref(),
                adl_version: o.adl_version.as_deref(),
                rm_release: &o.rm_release,
                is_differential: o.is_differential,
            },
        },
        Archetype::TemplateOverlay(t) => ArchetypeView {
            kind: ArtefactKind::TemplateOverlay,
            archetype_id: &t.archetype_id,
            parent_archetype_id: t.parent_archetype_id.as_deref(),
            definition: &t.definition,
            terminology: &t.terminology,
            rm_overlay: t.rm_overlay.as_ref(),
            original_language: None,
            description: None,
            translations: None,
            annotations: None,
            adl_version: None,
            rm_release: "",
            is_differential: t.is_differential,
        },
    }
}
