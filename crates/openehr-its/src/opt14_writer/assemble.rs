// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The template flattener: one `C_ARCHETYPE_ROOT` per archetype use.

use std::collections::BTreeMap;

use openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase;

use super::path::{self, Step, Unresolved};
use super::{ArchetypeLibrary, ArchetypeUse, Narrowing, SlotFill, TemplateDefinition, WriteError};
use super::{interval, slot};
use crate::opt14::types::{
    Archetype, CArchetypeRoot, CAttribute, CComplexObject, CMultipleAttribute, CObject,
    CSingleAttribute, Cardinality, FlatArchetypeOntology, Intervalofinteger, OperationalTemplate,
};

/// Builds the operational template; see [`super::build`].
pub(super) fn build(
    definition: &TemplateDefinition,
    library: &ArchetypeLibrary,
) -> Result<OperationalTemplate, WriteError> {
    let mut flattener = Flattener {
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
        .map(flat_ontology)
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

/// A constituent's whole ontology, every language, as `component_ontologies`
/// carries it.
fn flat_ontology(archetype: &Archetype) -> FlatArchetypeOntology {
    // NOTE: OPT2 master03-opt_raw.adoc §Terminology gathers each constituent's flat
    // terminology; Template.xsd's only home for all its languages is component_ontologies.
    FlatArchetypeOntology {
        archetype_id: archetype.archetype_id.value.clone(),
        term_definitions: archetype.ontology.term_definitions.clone(),
        constraint_definitions: archetype.ontology.constraint_definitions.clone(),
        term_bindings: archetype.ontology.term_bindings.clone(),
        constraint_bindings: archetype.ontology.constraint_bindings.clone(),
    }
}

/// The state of one build: the inputs and the constituents seen so far.
struct Flattener<'a> {
    library: &'a ArchetypeLibrary,
    language: &'a CodePhrase,
    /// Archetype ids in first-use order, each once.
    components: Vec<String>,
}

fn lookup<'a>(
    library: &'a ArchetypeLibrary,
    archetype_id: &str,
) -> Result<&'a Archetype, WriteError> {
    library
        .get(archetype_id)
        .ok_or_else(|| WriteError::ArchetypeNotFound {
            archetype_id: archetype_id.to_owned(),
        })
}

impl Flattener<'_> {
    /// Flattens one archetype use into its `C_ARCHETYPE_ROOT`.
    ///
    /// The steps run in a fixed order: internal references inline, then the
    /// attribute constraints the template adds, then the placements, then the
    /// occurrence and existence narrowings, then every removal, so a slot can
    /// be filled and closed in one use.
    fn archetype_root(
        &mut self,
        archetype_use: &ArchetypeUse,
        occurrences: Option<Intervalofinteger>,
    ) -> Result<CArchetypeRoot, WriteError> {
        let id = archetype_use.archetype_id.value.as_str();
        let archetype = lookup(self.library, id)?;
        if !self.components.iter().any(|seen| seen == id) {
            self.components.push(id.to_owned());
        }
        let terms = archetype
            .ontology
            .term_definitions
            .iter()
            .find(|set| set.language == self.language.code_string)
            .ok_or_else(|| WriteError::MissingLanguage {
                archetype_id: id.to_owned(),
                language: self.language.code_string.clone(),
            })?;

        let root_type = archetype.definition.rm_type_name.as_str();
        let mut attributes = archetype.definition.attributes.clone();
        inline_internal_refs(
            &mut attributes,
            &Inlining {
                original: &archetype.definition,
                archetype_id: id,
            },
            &[],
            &[],
            &mut Vec::new(),
        )?;
        for narrowing in &archetype_use.narrowings {
            if let Narrowing::Constrain { path, attribute } = narrowing {
                constrain(&mut attributes, id, root_type, path, attribute)?;
            }
        }
        let mut placed: BTreeMap<String, usize> = BTreeMap::new();
        for fill in &archetype_use.fills {
            self.place(&mut attributes, id, root_type, fill, &mut placed)?;
        }
        for narrowing in &archetype_use.narrowings {
            narrow(&mut attributes, id, narrowing)?;
        }
        for narrowing in &archetype_use.narrowings {
            remove(&mut attributes, id, narrowing)?;
        }

        Ok(CArchetypeRoot {
            rm_type_name: root_type.to_owned(),
            occurrences: occurrences.unwrap_or_else(|| archetype.definition.occurrences.clone()),
            // NOTE: Template.xsd is silent on a C_ARCHETYPE_ROOT's node_id; the root keeps its
            // archetype's own root code, which its term_definitions define (our design).
            node_id: archetype.definition.node_id.clone(),
            attributes,
            archetype_id: archetype.archetype_id.clone(),
            template_id: None,
            term_definitions: terms.items.clone(),
            term_bindings: archetype.ontology.term_bindings.clone(),
        })
    }

    /// Places one archetype: after its slot when the path ends in a node id,
    /// in the open attribute when it ends in an attribute name.
    fn place(
        &mut self,
        attributes: &mut Vec<CAttribute>,
        archetype_id: &str,
        root_type: &str,
        fill: &SlotFill,
        placed: &mut BTreeMap<String, usize>,
    ) -> Result<(), WriteError> {
        let steps = steps(archetype_id, &fill.path)?;
        match steps.last() {
            Some(last) if last.node_id.is_some() => {
                self.place_in_slot(attributes, archetype_id, fill, &steps, placed)
            }
            Some(_) => self.place_in_attribute(attributes, archetype_id, root_type, fill, &steps),
            None => Err(WriteError::MalformedPath {
                archetype_id: archetype_id.to_owned(),
                path: fill.path.clone(),
                reason: "a placement names a slot or an attribute",
            }),
        }
    }

    /// Places one filler directly after its slot, after the fillers already
    /// placed there.
    fn place_in_slot(
        &mut self,
        attributes: &mut Vec<CAttribute>,
        archetype_id: &str,
        fill: &SlotFill,
        steps: &[Step],
        placed: &mut BTreeMap<String, usize>,
    ) -> Result<(), WriteError> {
        let slot = {
            let (siblings, index) = resolve_object(attributes, archetype_id, &fill.path, steps)?;
            match siblings.get(index) {
                Some(CObject::ArchetypeSlot(slot)) => slot.clone(),
                _ => {
                    return Err(WriteError::NotASlot {
                        archetype_id: archetype_id.to_owned(),
                        path: fill.path.clone(),
                    });
                }
            }
        };
        let filler_id = fill.filler.archetype_id.value.as_str();
        let filler = lookup(self.library, filler_id)?;
        let admitted = slot::admits(&slot, filler_id).map_err(|e| match e {
            slot::SlotError::Unsupported => WriteError::UnsupportedSlotAssertion {
                archetype_id: archetype_id.to_owned(),
                slot_path: fill.path.clone(),
            },
            slot::SlotError::Pattern { pattern, source } => WriteError::InvalidSlotPattern {
                archetype_id: archetype_id.to_owned(),
                slot_path: fill.path.clone(),
                pattern,
                source: Box::new(source),
            },
        })?;
        if !admitted {
            return Err(WriteError::SlotRefusesFiller {
                archetype_id: archetype_id.to_owned(),
                slot_path: fill.path.clone(),
                filler: filler_id.to_owned(),
            });
        }
        // NOTE: ADL1.4 master05-cadl.adoc §Archetype Slots: "archetype slots are typed", so a
        // filler's root type is the slot's type or a subtype of it in the reference model.
        check_type(
            archetype_id,
            &fill.path,
            &slot.rm_type_name,
            filler_id,
            filler,
        )?;
        let occurrences = match &fill.occurrences {
            Some(given) => {
                check_within(
                    archetype_id,
                    &fill.path,
                    "occurrences",
                    given,
                    &slot.occurrences,
                )?;
                given.clone()
            }
            None => slot.occurrences.clone(),
        };
        let root = self.archetype_root(&fill.filler, Some(occurrences))?;

        let (siblings, index) = resolve_object(attributes, archetype_id, &fill.path, steps)?;
        let offset = placed.entry(path::render(steps)).or_insert(0);
        let at = index.saturating_add(1).saturating_add(*offset);
        siblings.insert(at.min(siblings.len()), CObject::CArchetypeRoot(root));
        *offset = offset.saturating_add(1);
        Ok(())
    }

    /// Places one archetype in an attribute the archetype leaves open,
    /// constraining the attribute as the reference model declares it.
    fn place_in_attribute(
        &mut self,
        attributes: &mut Vec<CAttribute>,
        archetype_id: &str,
        root_type: &str,
        fill: &SlotFill,
        steps: &[Step],
    ) -> Result<(), WriteError> {
        let (owner_steps, name) = match steps.split_last() {
            Some((last, parents)) => (parents, last.attribute.as_str()),
            None => (steps, ""),
        };
        let owner_type =
            owner_rm_type(attributes, archetype_id, root_type, &fill.path, owner_steps)?;
        let class = owner_type.split('<').next().unwrap_or(&owner_type);
        let model = openehr_rm::v1_2::model::attribute(class, name).ok_or_else(|| {
            WriteError::AttributeNotInModel {
                archetype_id: archetype_id.to_owned(),
                path: fill.path.clone(),
                rm_type: owner_type.clone(),
                attribute: name.to_owned(),
            }
        })?;
        let constrained = owner_attributes(attributes, archetype_id, &fill.path, owner_steps)?
            .iter()
            .find(|a| path::attribute_name(a) == name)
            .is_some_and(|a| {
                path::children(a)
                    .iter()
                    .any(|c| !matches!(c, CObject::CArchetypeRoot(_)))
            });
        if constrained {
            return Err(WriteError::AttributeConstrained {
                archetype_id: archetype_id.to_owned(),
                path: fill.path.clone(),
                attribute: name.to_owned(),
            });
        }
        let filler_id = fill.filler.archetype_id.value.as_str();
        let filler = lookup(self.library, filler_id)?;
        check_type(
            archetype_id,
            &fill.path,
            model.declared_type,
            filler_id,
            filler,
        )?;
        // NOTE: ADL1.4 master05-cadl.adoc §Occurrences: "the default occurrences, if none is
        // mentioned, is {1..1}"; no slot states a wider one here.
        let occurrences = fill
            .occurrences
            .clone()
            .unwrap_or_else(|| interval(1, Some(1)));
        let root = self.archetype_root(&fill.filler, Some(occurrences))?;

        let owner = owner_attributes(attributes, archetype_id, &fill.path, owner_steps)?;
        let index = owner
            .iter()
            .position(|a| path::attribute_name(a) == name)
            .unwrap_or_else(|| {
                owner.push(open_attribute(name, model));
                owner.len().saturating_sub(1)
            });
        if let Some(attribute) = owner.get_mut(index) {
            path::children_mut(attribute).push(CObject::CArchetypeRoot(root));
        }
        Ok(())
    }
}

/// The attribute constraint the reference model itself states: its existence,
/// and for a container its cardinality.
fn open_attribute(name: &str, model: &openehr_rm::v1_2::model::RmAttribute) -> CAttribute {
    use openehr_rm::v1_2::model::Container;
    // NOTE: no openEHR spec states how a template constrains an open attribute; the writer
    // writes the RM's existence and container semantics (ADL1.4 master05 §Cardinality, our design).
    let existence = if model.is_mandatory {
        interval(1, Some(1))
    } else {
        interval(0, Some(1))
    };
    match model.container {
        Container::None => CAttribute::CSingleAttribute(CSingleAttribute {
            rm_attribute_name: name.to_owned(),
            existence,
            children: Vec::new(),
        }),
        container => {
            let (lower, upper) = model.cardinality.map_or((0, None), |c| (c.lower, c.upper));
            CAttribute::CMultipleAttribute(CMultipleAttribute {
                rm_attribute_name: name.to_owned(),
                existence,
                children: Vec::new(),
                cardinality: Cardinality {
                    is_ordered: container == Container::List,
                    is_unique: matches!(container, Container::Set | Container::Hash),
                    interval: interval(
                        i32::try_from(lower).unwrap_or(i32::MAX),
                        upper.map(|u| i32::try_from(u).unwrap_or(i32::MAX)),
                    ),
                },
            })
        }
    }
}

/// A closed integer interval, or one open above when `upper` is `None`.
fn interval(lower: i32, upper: Option<i32>) -> Intervalofinteger {
    Intervalofinteger {
        lower_included: Some(true),
        upper_included: upper.map(|_| true),
        lower_unbounded: false,
        upper_unbounded: upper.is_none(),
        lower: Some(lower),
        upper,
    }
}

fn check_type(
    archetype_id: &str,
    path: &str,
    slot_type: &str,
    filler_id: &str,
    filler: &Archetype,
) -> Result<(), WriteError> {
    if openehr_rm::v1_2::model::is_a(&filler.definition.rm_type_name, slot_type) {
        Ok(())
    } else {
        Err(WriteError::FillerTypeMismatch {
            archetype_id: archetype_id.to_owned(),
            slot_path: path.to_owned(),
            slot_type: slot_type.to_owned(),
            filler: filler_id.to_owned(),
            filler_type: filler.definition.rm_type_name.clone(),
        })
    }
}

/// The reference-model type of the complex object at `steps` (the root when
/// empty).
fn owner_rm_type(
    attributes: &mut Vec<CAttribute>,
    archetype_id: &str,
    root_type: &str,
    path: &str,
    steps: &[Step],
) -> Result<String, WriteError> {
    if steps.is_empty() {
        return Ok(root_type.to_owned());
    }
    let (siblings, index) = resolve_object(attributes, archetype_id, path, steps)?;
    let object = siblings
        .get_mut(index)
        .ok_or_else(|| not_found(archetype_id, path, Unresolved { found: 0 }))?;
    if path::attributes_mut(object).is_none() {
        return Err(WriteError::NotAComplexObject {
            archetype_id: archetype_id.to_owned(),
            path: path.to_owned(),
        });
    }
    Ok(path::rm_type_name(object).to_owned())
}

/// The attribute list of the complex object at `steps` (the root when empty).
fn owner_attributes<'a>(
    attributes: &'a mut Vec<CAttribute>,
    archetype_id: &str,
    path: &str,
    steps: &[Step],
) -> Result<&'a mut Vec<CAttribute>, WriteError> {
    if steps.is_empty() {
        return Ok(attributes);
    }
    let (siblings, index) = resolve_object(attributes, archetype_id, path, steps)?;
    siblings
        .get_mut(index)
        .and_then(path::attributes_mut)
        .ok_or_else(|| WriteError::NotAComplexObject {
            archetype_id: archetype_id.to_owned(),
            path: path.to_owned(),
        })
}

/// What an inline copy reads from: the archetype's own definition.
struct Inlining<'a> {
    original: &'a CComplexObject,
    archetype_id: &'a str,
}

/// Replaces every `ARCHETYPE_INTERNAL_REF` below `attributes` by an inline
/// copy of its target (OPT2 `master03-opt_raw.adoc` §Flattening).
///
/// `location` is where the attributes' owner sits now and `origin` where its
/// content comes from in the archetype; `active` holds the targets being
/// copied, so a reference that would copy itself forever is refused.
fn inline_internal_refs(
    attributes: &mut [CAttribute],
    inlining: &Inlining<'_>,
    location: &[Step],
    origin: &[Step],
    active: &mut Vec<Vec<Step>>,
) -> Result<(), WriteError> {
    for attribute in attributes.iter_mut() {
        let name = path::attribute_name(attribute).to_owned();
        for child in path::children_mut(attribute).iter_mut() {
            let step = |node_id: &str| Step {
                attribute: name.clone(),
                node_id: (!node_id.is_empty()).then(|| node_id.to_owned()),
            };
            if let CObject::ArchetypeInternalRef(reference) = child {
                let here = [location, &[step(&reference.node_id)]].concat();
                let target = steps(inlining.archetype_id, &reference.target_path)?;
                if path::is_prefix(&target, origin) || active.contains(&target) {
                    return Err(WriteError::RecursiveInternalRef {
                        archetype_id: inlining.archetype_id.to_owned(),
                        path: path::render(&here),
                        target: reference.target_path.clone(),
                    });
                }
                let mut copy = path::object(inlining.original, &target)
                    .map_err(|e| not_found(inlining.archetype_id, &reference.target_path, e))?
                    .clone();
                // NOTE: ADL1.4 master05-cadl.adoc §Internal References: the referring node's
                // occurrences override the target's; a node id of its own names the copy.
                *path::occurrences_mut(&mut copy) = reference.occurrences.clone();
                if !reference.node_id.is_empty() {
                    reference.node_id.clone_into(path::node_id_mut(&mut copy));
                }
                active.push(target.clone());
                if let Some(inner) = path::attributes_mut(&mut copy) {
                    inline_internal_refs(inner, inlining, &here, &target, active)?;
                }
                active.pop();
                *child = copy;
            } else {
                let here = [location, &[step(path::node_id(child))]].concat();
                let from = [origin, &[step(path::node_id(child))]].concat();
                if let Some(inner) = path::attributes_mut(child) {
                    inline_internal_refs(inner, inlining, &here, &from, active)?;
                }
            }
        }
    }
    Ok(())
}

/// Applies an occurrence or existence narrowing in place.
fn narrow(
    attributes: &mut Vec<CAttribute>,
    archetype_id: &str,
    narrowing: &Narrowing,
) -> Result<(), WriteError> {
    match narrowing {
        Narrowing::Occurrences { path, occurrences } => {
            let steps = steps(archetype_id, path)?;
            let (siblings, index) = resolve_object(attributes, archetype_id, path, &steps)?;
            let object = siblings.get_mut(index).ok_or(WriteError::PathNotFound {
                archetype_id: archetype_id.to_owned(),
                path: path.clone(),
                found: 0,
            })?;
            let current = path::occurrences_mut(object);
            check_within(archetype_id, path, "occurrences", occurrences, current)?;
            occurrences.clone_into(current);
        }
        Narrowing::Existence { path, existence } => {
            let steps = steps(archetype_id, path)?;
            let (owner, index) = path::attribute_slot(attributes, &steps)
                .map_err(|e| not_found(archetype_id, path, e))?;
            let attribute = owner.get_mut(index).ok_or(WriteError::PathNotFound {
                archetype_id: archetype_id.to_owned(),
                path: path.clone(),
                found: 0,
            })?;
            let current = path::existence_mut(attribute);
            check_within(archetype_id, path, "existence", existence, current)?;
            existence.clone_into(current);
        }
        Narrowing::Constrain { .. } | Narrowing::Exclude { .. } => {}
    }
    Ok(())
}

/// Adds an attribute constraint on an attribute the archetype leaves open.
fn constrain(
    root: &mut Vec<CAttribute>,
    archetype_id: &str,
    root_type: &str,
    path: &str,
    attribute: &CAttribute,
) -> Result<(), WriteError> {
    let steps = steps(archetype_id, path)?;
    let name = path::attribute_name(attribute);
    let rm_type = owner_rm_type(root, archetype_id, root_type, path, &steps)?;
    let class = rm_type.split('<').next().unwrap_or(&rm_type);
    if openehr_rm::v1_2::model::attribute(class, name).is_none() {
        return Err(WriteError::AttributeNotInModel {
            archetype_id: archetype_id.to_owned(),
            path: path.to_owned(),
            rm_type,
            attribute: name.to_owned(),
        });
    }
    let owner = owner_attributes(root, archetype_id, path, &steps)?;
    if owner.iter().any(|a| path::attribute_name(a) == name) {
        return Err(WriteError::AttributeConstrained {
            archetype_id: archetype_id.to_owned(),
            path: path.to_owned(),
            attribute: name.to_owned(),
        });
    }
    owner.push(attribute.clone());
    Ok(())
}

/// Removes what a narrowing deletes: an excluded node, or a node or
/// attribute narrowed to `{0}` (OPT2 `master03-opt_raw.adoc` §Flattening).
fn remove(
    attributes: &mut Vec<CAttribute>,
    archetype_id: &str,
    narrowing: &Narrowing,
) -> Result<(), WriteError> {
    match narrowing {
        Narrowing::Exclude { path } => {
            let steps = steps(archetype_id, path)?;
            let (siblings, index) = resolve_object(attributes, archetype_id, path, &steps)?;
            if let Some(object) = siblings.get_mut(index) {
                let current = path::occurrences_mut(object);
                if interval::is_mandatory(current) {
                    return Err(WriteError::Widening {
                        archetype_id: archetype_id.to_owned(),
                        path: path.clone(),
                        what: "occurrences",
                        given: "{0..0}".to_owned(),
                        archetype: interval::display(current),
                    });
                }
            }
            siblings.remove(index);
        }
        Narrowing::Occurrences { path, occurrences } if interval::is_zero(occurrences) => {
            let steps = steps(archetype_id, path)?;
            let (siblings, index) = resolve_object(attributes, archetype_id, path, &steps)?;
            siblings.remove(index);
        }
        Narrowing::Existence { path, existence } if interval::is_zero(existence) => {
            let steps = steps(archetype_id, path)?;
            let (owner, index) = path::attribute_slot(attributes, &steps)
                .map_err(|e| not_found(archetype_id, path, e))?;
            owner.remove(index);
        }
        Narrowing::Occurrences { .. }
        | Narrowing::Existence { .. }
        | Narrowing::Constrain { .. } => {}
    }
    Ok(())
}

fn check_within(
    archetype_id: &str,
    path: &str,
    what: &'static str,
    given: &Intervalofinteger,
    archetype: &Intervalofinteger,
) -> Result<(), WriteError> {
    // NOTE: AM Overview master04-semantic_overview.adoc: templates narrow "existing constraints, in
    // the same way as specialised archetypes", so a template interval lies within the archetype's.
    if interval::is_within(given, archetype) {
        Ok(())
    } else {
        Err(WriteError::Widening {
            archetype_id: archetype_id.to_owned(),
            path: path.to_owned(),
            what,
            given: interval::display(given),
            archetype: interval::display(archetype),
        })
    }
}

fn steps(archetype_id: &str, path: &str) -> Result<Vec<Step>, WriteError> {
    path::parse(path).map_err(|reason| WriteError::MalformedPath {
        archetype_id: archetype_id.to_owned(),
        path: path.to_owned(),
        reason,
    })
}

fn not_found(archetype_id: &str, path: &str, unresolved: Unresolved) -> WriteError {
    WriteError::PathNotFound {
        archetype_id: archetype_id.to_owned(),
        path: path.to_owned(),
        found: unresolved.found,
    }
}

fn resolve_object<'a>(
    attributes: &'a mut Vec<CAttribute>,
    archetype_id: &str,
    path: &str,
    steps: &[Step],
) -> Result<(&'a mut Vec<CObject>, usize), WriteError> {
    path::object_slot(attributes, steps).map_err(|e| not_found(archetype_id, path, e))
}
