// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Archetype scope: the node ids and codes under a `C_ARCHETYPE_ROOT` belong to
//! the archetype it includes, never to the archetype that includes it.
//!
//! Rule texts: `AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Validity Rules `C_OBJECT` (VCOSU: "every object node must be unique within
//! the archetype") and `OPT2/master03-opt_raw.adoc` (the flat terminology of
//! each constituent archetype is gathered under `component_terminologies`,
//! keyed by its archetype id). The node-id walks restart at each archetype
//! root; on an operational template the code checks below a root judge its
//! codes against that constituent's terminology.

use std::collections::{BTreeMap, BTreeSet};

use openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

/// The archetype root `obj` opens, when it is one.
pub(super) fn archetype_root(obj: &CObject) -> Option<&CArchetypeRoot> {
    match obj {
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(root)) => Some(root),
        _ => None,
    }
}

/// Every code `term` defines, in any language (a code defined in one language
/// counts as defined for the definedness checks).
pub(super) fn defined_codes(term: &ArchetypeTerminology) -> BTreeSet<&str> {
    term.term_definitions
        .values()
        .flat_map(|m| m.keys().map(String::as_str))
        .collect()
}

/// The terminology of a constituent an operational template carries none for,
/// so the codes under its root are judged against an empty terminology.
static NO_TERMINOLOGY: ArchetypeTerminology = ArchetypeTerminology {
    is_differential: false,
    original_language: String::new(),
    concept_code: String::new(),
    term_definitions: BTreeMap::new(),
    term_bindings: None,
    value_sets: None,
    terminology_extracts: None,
};

/// The constituent terminologies of an operational template by archetype id;
/// `None` on any other artefact (`ArchetypeView::components`).
pub(super) type Components<'a> = Option<&'a BTreeMap<String, ArchetypeTerminology>>;

/// The terminology that governs the subtree of `root`: on an operational
/// template, the constituent's entry in `components` (empty when the template
/// carries none for it); `None` on any other artefact, whose code checks keep
/// one scope.
pub(super) fn constituent_terminology<'a>(
    components: Components<'a>,
    root: &CArchetypeRoot,
) -> Option<&'a ArchetypeTerminology> {
    Some(
        components?
            .get(&root.archetype_ref)
            .unwrap_or(&NO_TERMINOLOGY),
    )
}

/// Enters the subtree of `obj`: below a nested archetype root of an
/// operational template `defined` becomes the constituent's codes, and the
/// outer codes are returned for [`leave`].
pub(super) fn enter<'a>(
    obj: &CObject,
    components: Components<'a>,
    defined: &mut BTreeSet<&'a str>,
) -> Option<BTreeSet<&'a str>> {
    let term = constituent_terminology(components, archetype_root(obj)?)?;
    Some(std::mem::replace(defined, defined_codes(term)))
}

/// Leaves a subtree [`enter`] opened, restoring the outer codes.
pub(super) fn leave<'a>(outer: Option<BTreeSet<&'a str>>, defined: &mut BTreeSet<&'a str>) {
    if let Some(outer) = outer {
        *defined = outer;
    }
}
