// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Object redefinition: a template's objects overlaid on an attribute's
//! children, as a specialised archetype redefines its parent's
//! (`AM/docs/ADL2/master09.05-spec_object_redef.adoc` §Object Redefinition).

use super::WriteError;
use super::interval;
use super::leaf;
use super::path;
use crate::opt14::types::{CAttribute, CObject, Intervalofinteger};

/// Where a redefinition applies, for its diagnostics.
pub(super) struct Site<'a> {
    /// The archetype.
    pub(super) archetype_id: &'a str,
    /// The redefined attribute's path as the definition gives it.
    pub(super) path: &'a str,
}

impl Site<'_> {
    fn unmatched(&self, node: String) -> WriteError {
        WriteError::RedefinitionUnmatched {
            archetype_id: self.archetype_id.to_owned(),
            path: self.path.to_owned(),
            node,
        }
    }

    fn within(
        &self,
        what: &'static str,
        given: &Intervalofinteger,
        archetype: &Intervalofinteger,
    ) -> Result<(), WriteError> {
        if interval::is_within(given, archetype) {
            Ok(())
        } else {
            Err(WriteError::Widening {
                archetype_id: self.archetype_id.to_owned(),
                path: self.path.to_owned(),
                what,
                given: interval::display(given),
                archetype: interval::display(archetype),
            })
        }
    }
}

/// Returns whether the reference-model type `given` conforms to `archetype`:
/// the same type, or a subtype with the same generic parameters.
pub(super) fn conforms(given: &str, archetype: &str) -> bool {
    // NOTE: AOM2 master04.5 VSONCT: a redefined node's RM type is identical to its parent's or
    // conforms "via an inheritance relationship in the relevant reference model".
    if given == archetype {
        return true;
    }
    let (given_class, given_params) = split_generic(given);
    let (archetype_class, archetype_params) = split_generic(archetype);
    openehr_rm::v1_2::model::is_a(given_class, archetype_class)
        && (archetype_params.is_none() || given_params == archetype_params)
}

fn split_generic(rm_type: &str) -> (&str, Option<&str>) {
    match rm_type.split_once('<') {
        Some((class, params)) => (class, Some(params)),
        None => (rm_type, None),
    }
}

/// Overlays `given` on the children of `attribute`.
///
/// Each object redefines one child (see [`super::Narrowing::Redefine`]); a
/// child without a node id that no object redefines is removed when any
/// object has no node id, since those objects restate the alternatives.
pub(super) fn overlay_children(
    attribute: &mut CAttribute,
    given: &[CObject],
    site: &Site<'_>,
) -> Result<(), WriteError> {
    let multiple = matches!(attribute, CAttribute::CMultipleAttribute(_));
    let children = path::children_mut(attribute);
    let mut redefined: Vec<Vec<CObject>> = vec![Vec::new(); children.len()];
    for object in given {
        let target = matched_child(children, object, site)?;
        let child = children
            .get(target)
            .ok_or_else(|| site.unmatched(describe(object)))?;
        let overlaid = overlay_object(child, object, site)?;
        let slot = redefined
            .get_mut(target)
            .ok_or_else(|| site.unmatched(describe(object)))?;
        if !path::node_id(object).is_empty() && !slot.is_empty() {
            return Err(site.unmatched(describe(object)));
        }
        slot.push(overlaid);
    }
    let restates_alternatives = given.iter().any(|o| path::node_id(o).is_empty());
    let mut kept = Vec::with_capacity(children.len());
    for (child, overlays) in children.drain(..).zip(redefined) {
        if !overlays.is_empty() {
            kept.extend(
                overlays
                    .into_iter()
                    .filter(|o| !interval::is_zero(path::occurrences(o))),
            );
            continue;
        }
        let alternative = path::node_id(&child).is_empty() && !is_root(&child);
        if !(restates_alternatives && alternative) {
            kept.push(child);
            continue;
        }
        // NOTE: OPT2 master03-opt_raw.adoc §Flattening states sibling sets in full; an alternative
        // the template does not restate is dropped, and dropping a mandatory member widens.
        if multiple && interval::is_mandatory(path::occurrences(&child)) {
            return Err(WriteError::Widening {
                archetype_id: site.archetype_id.to_owned(),
                path: site.path.to_owned(),
                what: "occurrences",
                given: "{0..0}".to_owned(),
                archetype: interval::display(path::occurrences(&child)),
            });
        }
    }
    *children = kept;
    Ok(())
}

fn is_root(object: &CObject) -> bool {
    matches!(object, CObject::CArchetypeRoot(_))
}

fn describe(object: &CObject) -> String {
    let node_id = path::node_id(object);
    if node_id.is_empty() {
        path::rm_type_name(object).to_owned()
    } else {
        format!("{}[{node_id}]", path::rm_type_name(object))
    }
}

/// The index of the child `object` redefines.
fn matched_child(
    children: &[CObject],
    object: &CObject,
    site: &Site<'_>,
) -> Result<usize, WriteError> {
    if is_root(object) {
        return Err(site.unmatched(describe(object)));
    }
    let node_id = path::node_id(object);
    let own = || {
        children
            .iter()
            .enumerate()
            .filter(|(_, child)| !is_root(child))
    };
    let candidates: Vec<usize> = if node_id.is_empty() {
        let rm_type = path::rm_type_name(object);
        let alternatives: Vec<(usize, &CObject)> = own()
            .filter(|(_, child)| path::node_id(child).is_empty())
            .collect();
        let exact: Vec<usize> = alternatives
            .iter()
            .filter(|(_, child)| path::rm_type_name(child) == rm_type)
            .map(|(index, _)| *index)
            .collect();
        if exact.is_empty() {
            let conforming: Vec<usize> = alternatives
                .iter()
                .filter(|(_, child)| conforms(rm_type, path::rm_type_name(child)))
                .map(|(index, _)| *index)
                .collect();
            if conforming.is_empty() && !alternatives.is_empty() {
                return Err(WriteError::TypeNotConforming {
                    archetype_id: site.archetype_id.to_owned(),
                    path: site.path.to_owned(),
                    archetype: alternatives
                        .iter()
                        .map(|(_, child)| path::rm_type_name(child))
                        .collect::<Vec<_>>()
                        .join(" | "),
                    given: rm_type.to_owned(),
                });
            }
            conforming
        } else {
            exact
        }
    } else {
        own()
            .filter(|(_, child)| path::node_id(child) == node_id)
            .map(|(index, _)| index)
            .collect()
    };
    match candidates.as_slice() {
        [index] => Ok(*index),
        _ => Err(site.unmatched(describe(object))),
    }
}

/// The child `archetype` redefined by `given`.
fn overlay_object(
    archetype: &CObject,
    given: &CObject,
    site: &Site<'_>,
) -> Result<CObject, WriteError> {
    let archetype_type = path::rm_type_name(archetype);
    let given_type = path::rm_type_name(given);
    if !conforms(given_type, archetype_type) {
        return Err(WriteError::TypeNotConforming {
            archetype_id: site.archetype_id.to_owned(),
            path: site.path.to_owned(),
            archetype: archetype_type.to_owned(),
            given: given_type.to_owned(),
        });
    }
    // NOTE: AOM2 master04.5 VSONCO: a node redefining a single node lies within its occurrences;
    // each alternative of a single-valued attribute is such a node.
    site.within(
        "occurrences",
        path::occurrences(given),
        path::occurrences(archetype),
    )?;
    let mut result = given.clone();
    let (Some(archetype_attributes), Some(given_attributes)) =
        (path::attributes(archetype), path::attributes(given))
    else {
        leaf::narrows(archetype, given, site)?;
        return Ok(result);
    };
    // NOTE: ADL2 master09.05 §Reference Model Type Refinement: on a subtype, "any existing
    // constraints on the original type in the parent archetype are respected".
    let mut merged = archetype_attributes.clone();
    for attribute in given_attributes {
        let name = path::attribute_name(attribute);
        if let Some(index) = merged.iter().position(|a| path::attribute_name(a) == name) {
            let keep = merged
                .get_mut(index)
                .map(|current| overlay_attribute(current, attribute, site))
                .transpose()?
                .unwrap_or(false);
            if !keep {
                merged.remove(index);
            }
        } else {
            let class = split_generic(given_type).0;
            if openehr_rm::v1_2::model::attribute(class, name).is_none() {
                return Err(WriteError::AttributeNotInModel {
                    archetype_id: site.archetype_id.to_owned(),
                    path: site.path.to_owned(),
                    rm_type: given_type.to_owned(),
                    attribute: name.to_owned(),
                });
            }
            merged.push(attribute.clone());
        }
    }
    if let Some(attributes) = path::attributes_mut(&mut result) {
        *attributes = merged;
    }
    Ok(result)
}

/// Narrows the archetype attribute `current` by `given`; returns `false`
/// when `given` removes it.
fn overlay_attribute(
    current: &mut CAttribute,
    given: &CAttribute,
    site: &Site<'_>,
) -> Result<bool, WriteError> {
    // NOTE: AOM2 master04.5 VSANCE, VSANCC and VSAM: a redefined attribute keeps its multiplicity,
    // and its existence and cardinality lie within the parent's.
    match (&mut *current, given) {
        (CAttribute::CSingleAttribute(_), CAttribute::CSingleAttribute(_)) => {}
        (CAttribute::CMultipleAttribute(current), CAttribute::CMultipleAttribute(given)) => {
            site.within(
                "cardinality",
                &given.cardinality.interval,
                &current.cardinality.interval,
            )?;
            current.cardinality = given.cardinality.clone();
        }
        _ => return Err(site.unmatched(path::attribute_name(given).to_owned())),
    }
    let existence = path::existence(given);
    site.within("existence", existence, path::existence_mut(current))?;
    if interval::is_zero(existence) {
        return Ok(false);
    }
    existence.clone_into(path::existence_mut(current));
    let children = path::children(given);
    if !children.is_empty() {
        overlay_children(current, children, site)?;
    }
    Ok(true)
}
