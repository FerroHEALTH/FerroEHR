// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The OPT 1.4 writer: an operational template from a template definition
//! and the ADL 1.4 archetypes it references.
//!
//! The AM overview describes the tool chain (`AM/docs/Overview/
//! master05-artefacts.adoc` §Artefacts): a template references archetypes,
//! and "a template flattener can generate the final output, an operational
//! template". This module is that flattener for the 1.4 artefacts. Its input
//! is a [`TemplateDefinition`] (the root archetype, the archetypes each slot
//! takes, the constraints the template narrows) and an [`ArchetypeLibrary`]
//! of AOM 1.4 archetypes in the ITS-XML `Archetype.xsd` model, the form
//! [`crate::xml::runtime::from_xml`] reads an AM 1.4 archetype document into.
//! Its output is the [`crate::opt14`] model of ITS-XML `Template.xsd`, the
//! schema that alone defines the OPT 1.4 format, and [`write()`] serializes it.
//!
//! An ADL 1.4 archetype needs no flattening of its own: "up until ADL 1.4,
//! archetypes were expressed in 'flat' form" (`AM/docs/ADL2/
//! master07.02-adl_conventions.adoc` §File-naming Convention), and a 1.4
//! legacy archetype is "always in flat form" (`AM/docs/AOM2/
//! master03-archetype_package.adoc` §Structural Variants). Template
//! flattening follows the raw-OPT steps of `AM/docs/OPT2/
//! master03-opt_raw.adoc` §Flattening, the only flattening
//! semantics the specifications state: internal references are replaced by
//! an inline copy of their target, slot fillers by an inline copy of the
//! archetype, and deleted nodes (`occurrences {0}`, `existence {0}`) are
//! removed.
//!
//! The same inputs give the same bytes: every collection is walked in input
//! order or through a `BTreeMap`, so a product can commit the OPT and a check
//! can regenerate it and fail on drift.
//!
//! # Examples
//!
//! ```
//! use openehr_base::v1_3::base_types::identification::archetype_id::ArchetypeId;
//! use openehr_base::v1_3::base_types::identification::template_id::TemplateId;
//! use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
//! use openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase;
//! use openehr_its::opt14_writer::{ArchetypeLibrary, ArchetypeUse, TemplateDefinition};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let library = ArchetypeLibrary::default();
//! let definition = TemplateDefinition {
//!     template_id: TemplateId { value: "Example".to_owned() },
//!     concept: "Example".to_owned(),
//!     language: CodePhrase {
//!         terminology_id: TerminologyId { value: "ISO_639-1".to_owned() },
//!         code_string: "en".to_owned(),
//!         preferred_term: None,
//!     },
//!     description: None,
//!     uid: None,
//!     root: ArchetypeUse::new(ArchetypeId {
//!         value: "openEHR-EHR-EVALUATION.clinical_synopsis.v1".to_owned(),
//!     }),
//! };
//! // The library holds no archetype, so the writer refuses the definition.
//! assert!(openehr_its::opt14_writer::build(&definition, &library).is_err());
//! # Ok(())
//! # }
//! ```

mod assemble;
mod interval;
mod leaf;
mod path;
mod redefine;
mod slot;

use std::collections::BTreeMap;

use openehr_base::v1_3::base_types::identification::archetype_id::ArchetypeId;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::template_id::TemplateId;
use openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase;

use crate::opt14::types::{
    Archetype, CAttribute, CObject, Intervalofinteger, OperationalTemplate, ResourceDescription,
};
use crate::xml::runtime::XmlError;

/// The archetypes a template definition may name, keyed by archetype id.
///
/// The writer resolves every [`ArchetypeUse`] here by the exact archetype id
/// string; it never guesses a version.
#[derive(Debug, Clone, Default)]
pub struct ArchetypeLibrary {
    by_id: BTreeMap<String, Archetype>,
}

impl ArchetypeLibrary {
    /// Adds an archetype under its own `archetype_id`, returning the one it
    /// replaces.
    pub fn insert(&mut self, archetype: Archetype) -> Option<Archetype> {
        self.by_id
            .insert(archetype.archetype_id.value.clone(), archetype)
    }

    /// Returns the archetype with the id `archetype_id`.
    #[must_use]
    pub fn get(&self, archetype_id: &str) -> Option<&Archetype> {
        self.by_id.get(archetype_id)
    }

    /// Returns the number of archetypes in the library.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Returns whether the library holds no archetype.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

impl FromIterator<Archetype> for ArchetypeLibrary {
    fn from_iter<I: IntoIterator<Item = Archetype>>(iter: I) -> Self {
        let mut library = Self::default();
        for archetype in iter {
            library.insert(archetype);
        }
        library
    }
}

/// A template definition: what an operational template is built from.
///
/// It carries the header the OPT states (`Template.xsd`
/// `OPERATIONAL_TEMPLATE`) and the tree of archetype uses, the root first.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateDefinition {
    /// The OPT `template_id`.
    pub template_id: TemplateId,
    /// The OPT `concept`.
    pub concept: String,
    /// The OPT `language`; every constituent archetype must define its terms
    /// in this language.
    pub language: CodePhrase,
    /// The OPT `description`, written as given.
    pub description: Option<ResourceDescription>,
    /// The OPT `uid`.
    pub uid: Option<HierObjectId>,
    /// The root archetype and what the template does with it.
    pub root: ArchetypeUse,
}

/// One use of an archetype in a template: the archetype, the slots it fills
/// and the constraints it narrows.
///
/// Every path is an ADL 1.4 path (`AM/docs/ADL1.4/master07-paths.adoc`)
/// relative to this archetype's root, such as `/data[at0001]/items[at0004]`.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchetypeUse {
    /// The archetype, by its full id in the [`ArchetypeLibrary`].
    pub archetype_id: ArchetypeId,
    /// The slot fillers, in the order they appear after their slot.
    pub fills: Vec<SlotFill>,
    /// The constraints the template narrows on this archetype.
    pub narrowings: Vec<Narrowing>,
}

impl ArchetypeUse {
    /// Creates a use of `archetype_id` that fills no slot and narrows nothing.
    #[must_use]
    pub fn new(archetype_id: ArchetypeId) -> Self {
        Self {
            archetype_id,
            fills: Vec::new(),
            narrowings: Vec::new(),
        }
    }
}

/// One archetype placed in an `ARCHETYPE_SLOT`, or in an attribute the
/// archetype leaves open.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotFill {
    /// The path of the `ARCHETYPE_SLOT` node, ending in its node id; or the
    /// path of an attribute the archetype leaves open, ending in the
    /// attribute name, such as `/content` of a composition.
    pub path: String,
    /// The occurrences of the placed archetype: within the slot's own, and
    /// the slot's when absent; `{1..1}` in an open attribute when absent.
    pub occurrences: Option<Intervalofinteger>,
    /// The archetype placed in the slot.
    pub filler: ArchetypeUse,
}

/// One constraint a template narrows on an archetype.
#[derive(Debug, Clone, PartialEq)]
pub enum Narrowing {
    /// Narrows the occurrences of the object node at `path` (ending in its
    /// node id); `{0}` removes the node.
    Occurrences {
        /// The object node's path.
        path: String,
        /// The new occurrences, within the archetype's.
        occurrences: Intervalofinteger,
    },
    /// Narrows the existence of the attribute at `path` (ending in the
    /// attribute name); `{0}` removes the attribute.
    Existence {
        /// The attribute's path.
        path: String,
        /// The new existence, within the archetype's.
        existence: Intervalofinteger,
    },
    /// Removes the optional object node at `path`; on a slot it closes the
    /// slot after its fillers are placed.
    Exclude {
        /// The object node's path.
        path: String,
    },
    /// Constrains an attribute the archetype leaves open on the complex
    /// object at `path` (`/` for the archetype root), such as the `name` of a
    /// node the template renames. A [`SlotFill`] may then place archetypes in
    /// the same attribute beside the constraint's own objects.
    Constrain {
        /// The complex object's path.
        path: String,
        /// The attribute constraint, written as given.
        attribute: CAttribute,
    },
    /// Redefines child objects of the attribute at `path` (ending in the
    /// attribute name), such as a `DV_TEXT` value narrowed to a
    /// `DV_CODED_TEXT` with a value set.
    ///
    /// Each object redefines the archetype child with its node id, or, when
    /// it has none, the child without a node id of its exact reference-model
    /// type, else the only such child its type conforms to. Its type must
    /// conform to the redefined child's and its occurrences lie within. Over
    /// a complex child its attributes overlay the child's: an attribute the
    /// child constrains is narrowed the same way (existence, cardinality,
    /// children), an attribute at existence `{0}` is removed, and the child's
    /// other attributes are kept. An object at occurrences `{0}` removes the
    /// child. When any object has no node id, the objects without one are the
    /// attribute's whole set of alternatives, and the archetype's other
    /// alternatives are removed.
    Redefine {
        /// The attribute's path.
        path: String,
        /// The redefining objects, in the order they replace the archetype's.
        objects: Vec<CObject>,
    },
    /// Clones an object node under its own node id, distinguished by a
    /// `name` constraint.
    Clone(NodeClone),
}

/// One copy of an archetype node that may occur more than once, placed after
/// the node and the copies before it.
///
/// The copy starts from the archetype's node as it stands before the
/// template's other narrowings and keeps its node id; the node itself stays
/// unless a narrowing excludes it, in which case the copies take its place.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeClone {
    /// The path of the object node cloned, ending in its node id.
    pub path: String,
    /// The copy's constraint on the `name` attribute, which must differ from
    /// the node's own and from every other copy's.
    pub name: CAttribute,
    /// The copy's occurrences; the node's own when absent.
    pub occurrences: Option<Intervalofinteger>,
    /// The slot fillers in the copy, by paths relative to the copy (`/` is
    /// the copy itself).
    pub fills: Vec<SlotFill>,
    /// The constraints the template narrows on the copy, by paths relative to
    /// the copy.
    pub narrowings: Vec<Narrowing>,
}

impl NodeClone {
    /// Creates a copy of the node at `path` named by `name`, narrowing
    /// nothing else.
    #[must_use]
    pub fn new(path: impl Into<String>, name: CAttribute) -> Self {
        Self {
            path: path.into(),
            name,
            occurrences: None,
            fills: Vec::new(),
            narrowings: Vec::new(),
        }
    }
}

/// A refusal of the OPT 1.4 writer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WriteError {
    /// The library holds no archetype with this id.
    #[error("archetype {archetype_id} is not in the archetype library")]
    ArchetypeNotFound {
        /// The archetype id the definition names.
        archetype_id: String,
    },
    /// A constituent archetype defines no terms in the template language.
    #[error("archetype {archetype_id} has no term definitions in language {language}")]
    MissingLanguage {
        /// The archetype.
        archetype_id: String,
        /// The template language.
        language: String,
    },
    /// A path is not an ADL 1.4 path of the form the writer reads.
    #[error("{path:?} in {archetype_id} is not an ADL path: {reason}")]
    MalformedPath {
        /// The archetype the path addresses.
        archetype_id: String,
        /// The path as given.
        path: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A path addresses no node of the archetype, or more than one.
    #[error("{path:?} addresses {found} nodes in {archetype_id}, not exactly one")]
    PathNotFound {
        /// The archetype the path addresses.
        archetype_id: String,
        /// The path as given.
        path: String,
        /// How many nodes the path matched.
        found: usize,
    },
    /// A slot fill names a node that is not an `ARCHETYPE_SLOT`.
    #[error("{path:?} in {archetype_id} is not an archetype slot")]
    NotASlot {
        /// The archetype.
        archetype_id: String,
        /// The node's path.
        path: String,
    },
    /// A slot carries an assertion the writer cannot evaluate.
    #[error(
        "the slot {slot_path:?} in {archetype_id} carries an assertion other than \
         `archetype_id/value matches {{/regex/}}`"
    )]
    UnsupportedSlotAssertion {
        /// The archetype that defines the slot.
        archetype_id: String,
        /// The slot's path.
        slot_path: String,
    },
    /// A slot assertion's pattern is not a regular expression.
    #[error("the slot {slot_path:?} in {archetype_id} has the invalid pattern {pattern:?}")]
    InvalidSlotPattern {
        /// The archetype that defines the slot.
        archetype_id: String,
        /// The slot's path.
        slot_path: String,
        /// The pattern as written.
        pattern: String,
        /// The regular-expression error.
        #[source]
        source: Box<regex::Error>,
    },
    /// A slot's include and exclude assertions refuse the filler.
    #[error("the slot {slot_path:?} in {archetype_id} does not admit {filler}")]
    SlotRefusesFiller {
        /// The archetype that defines the slot.
        archetype_id: String,
        /// The slot's path.
        slot_path: String,
        /// The filler's archetype id.
        filler: String,
    },
    /// The filler's reference-model type does not conform to the slot's.
    #[error(
        "the slot {slot_path:?} in {archetype_id} takes {slot_type}, and {filler} is a {filler_type}"
    )]
    FillerTypeMismatch {
        /// The archetype that defines the slot.
        archetype_id: String,
        /// The slot's path.
        slot_path: String,
        /// The slot's reference-model type.
        slot_type: String,
        /// The filler's archetype id.
        filler: String,
        /// The filler root's reference-model type.
        filler_type: String,
    },
    /// A template constraint widens the archetype's instead of narrowing it.
    #[error("the {what} {given} at {path:?} in {archetype_id} is not within {archetype}")]
    Widening {
        /// The archetype.
        archetype_id: String,
        /// The node's path.
        path: String,
        /// Which constraint: `occurrences`, `existence`, `cardinality`, or the
        /// leaf value constraint a redefinition narrows (`code list`, `magnitude`).
        what: &'static str,
        /// The template's constraint.
        given: String,
        /// The archetype's constraint.
        archetype: String,
    },
    /// A `Constrain` narrowing names a node that is not a complex object.
    #[error("{path:?} in {archetype_id} is not a complex object")]
    NotAComplexObject {
        /// The archetype.
        archetype_id: String,
        /// The node's path.
        path: String,
    },
    /// A `Constrain` narrowing names an attribute the node's reference-model
    /// type does not have.
    #[error("{rm_type} at {path:?} in {archetype_id} has no attribute {attribute}")]
    AttributeNotInModel {
        /// The archetype.
        archetype_id: String,
        /// The node's path.
        path: String,
        /// The node's reference-model type.
        rm_type: String,
        /// The attribute name.
        attribute: String,
    },
    /// A `Constrain` narrowing names an attribute the archetype constrains
    /// already; a template narrows that constraint instead.
    #[error("{attribute} at {path:?} in {archetype_id} is constrained by the archetype already")]
    AttributeConstrained {
        /// The archetype.
        archetype_id: String,
        /// The node's path.
        path: String,
        /// The attribute name.
        attribute: String,
    },
    /// A redefined object's reference-model type does not conform to the
    /// type of the archetype node it redefines.
    #[error("{given} does not conform to {archetype} at {path:?} in {archetype_id}")]
    TypeNotConforming {
        /// The archetype.
        archetype_id: String,
        /// The redefined attribute's path.
        path: String,
        /// The archetype node's reference-model type, or the types of its
        /// alternatives.
        archetype: String,
        /// The redefining object's reference-model type.
        given: String,
    },
    /// A redefining object matches no archetype node, or more than one, or a
    /// redefined attribute changes its kind.
    #[error("{node} at {path:?} in {archetype_id} redefines no single archetype node")]
    RedefinitionUnmatched {
        /// The archetype.
        archetype_id: String,
        /// The redefined attribute's path.
        path: String,
        /// The redefining object or attribute.
        node: String,
    },
    /// The archetype does not admit a clone of the node.
    #[error("{path:?} in {archetype_id} admits no clone: {reason}")]
    CloneNotAdmitted {
        /// The archetype.
        archetype_id: String,
        /// The cloned node's path.
        path: String,
        /// Why the clone is refused.
        reason: &'static str,
    },
    /// Two copies of a node, or a copy and the node, carry the same `name`
    /// constraint, so data cannot tell them apart.
    #[error("the clones of {path:?} in {archetype_id} share a name constraint")]
    CloneNameNotDistinct {
        /// The archetype.
        archetype_id: String,
        /// The cloned node's path.
        path: String,
    },
    /// An internal reference points at its own ancestor, which an inline
    /// copy cannot terminate.
    #[error(
        "the internal reference at {path:?} in {archetype_id} targets its own ancestor {target}"
    )]
    RecursiveInternalRef {
        /// The archetype.
        archetype_id: String,
        /// The reference's own location.
        path: String,
        /// The reference's target path.
        target: String,
    },
    /// The operational template does not serialize.
    #[error("the operational template does not serialize")]
    Xml(#[from] XmlError),
}

/// Builds the operational template of `definition` over `library`.
///
/// The root archetype becomes the OPT `definition`, a `C_ARCHETYPE_ROOT`
/// carrying the template id; every filler becomes a `C_ARCHETYPE_ROOT` placed
/// directly after its slot, or in the open attribute it names. Each root carries its archetype's term
/// definitions in the template language and its term bindings, and
/// `component_ontologies` carries every constituent's whole ontology, once
/// per archetype id, in first-use order.
///
/// # Errors
///
/// Returns a [`WriteError`] when an archetype is missing from the library or
/// lacks the template language, a path does not resolve, a slot refuses its
/// filler, a constraint widens the archetype's or constrains an attribute
/// that is taken or absent from the reference model, a redefinition names a
/// type that does not conform or no single node, a clone is not admitted or
/// not told apart by its name, or an internal reference is recursive.
pub fn build(
    definition: &TemplateDefinition,
    library: &ArchetypeLibrary,
) -> Result<OperationalTemplate, WriteError> {
    let mut flattener = assemble::Flattener {
        library,
        language: &definition.language,
        components: Vec::new(),
    };
    let mut root = flattener.archetype_root(&definition.root, None)?;
    // NOTE: Template.xsd declares C_ARCHETYPE_ROOT.template_id with no semantics; no openEHR
    // spec governs it, and the writer names the template on the definition root (our design).
    root.template_id = Some(definition.template_id.clone());
    let component_ontologies = flattener
        .components
        .iter()
        .filter_map(|id| library.get(id))
        .map(assemble::flat_ontology)
        .collect();
    Ok(OperationalTemplate {
        language: definition.language.clone(),
        is_controlled: None,
        description: definition.description.clone(),
        revision_history: None,
        uid: definition.uid.clone(),
        template_id: definition.template_id.clone(),
        concept: definition.concept.clone(),
        definition: root,
        ontology: None,
        component_ontologies,
        annotations: Vec::new(),
        constraints: None,
        view: None,
    })
}

/// Builds the operational template of `definition` and serializes it as OPT
/// 1.4 XML (`Template.xsd`, root `<template>` in `http://schemas.openehr.org/v1`).
///
/// # Errors
///
/// Returns every refusal of [`build`], and [`WriteError::Xml`] when the
/// template does not serialize.
pub fn write(
    definition: &TemplateDefinition,
    library: &ArchetypeLibrary,
) -> Result<String, WriteError> {
    let template = build(definition, library)?;
    Ok(crate::opt14::to_xml(&template)?)
}
