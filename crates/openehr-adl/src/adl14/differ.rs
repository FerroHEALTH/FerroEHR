// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Re-differentialisation of a converted specialised 1.4 child against its
//! converted+flattened parent.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion — our own design (see the
//! [`crate::adl14`] flag). The *target* of the differ (a differential child
//! whose nodes are the differences vs the flat parent) is defined by
//! `docs/specs/openehr/AM/docs/ADL2/master09.02` §Differential authoring; the
//! 1.4-conversion *use* of it is ours.
//!
//! A 1.4 specialised source is authored *flat* (it repeats every inherited
//! node). After the base conversion renumbers it against the flat parent's
//! codes, the differ strips every child node that is inherited-unchanged —
//! structurally identical to the same-path node in the flat parent — leaving
//! only the genuine differences (`master09.02`: "child differences are vs the
//! flat parent").

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

/// The flat-parent definition, if `parent` is an authored archetype.
fn flat_definition(parent: &Archetype) -> Option<&CComplexObject> {
    match parent {
        Archetype::AuthoredArchetype(b) => match b.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => Some(&d.definition),
            _ => None,
        },
        Archetype::TemplateOverlay(_) => None,
    }
}

/// Strip inherited-unchanged nodes from a converted specialised `child`,
/// re-differentialising it against the flat `parent`.
///
/// The minimal-correct rule (`master09.02` §Differential): a child object node
/// whose specialisation depth is 0 (an inherited node id, not a `.N`-redefined
/// or new node) and which is structurally equal to the same node in the flat
/// parent is inherited-unchanged and removed; an attribute left with no
/// children after stripping is removed; the root is always kept.
pub fn differentiate(child: &mut Archetype, parent: &Archetype) {
    let Some(parent_def) = flat_definition(parent) else {
        return;
    };
    let Archetype::AuthoredArchetype(b) = child else {
        return;
    };
    let AuthoredArchetype::AuthoredArchetype(data) = b.as_mut() else {
        return;
    };
    strip_inherited(&mut data.definition, parent_def);
    prune_terminology(data);
}

fn strip_inherited(child: &mut CComplexObject, parent: &CComplexObject) {
    let (CComplexObject::CComplexObject(cd), CComplexObject::CComplexObject(_pd)) =
        (&mut *child, parent)
    else {
        return;
    };
    // Recurse into attributes; drop children that are inherited-unchanged.
    for attr in cd.attributes.iter_mut().flatten() {
        strip_attr(attr, parent);
    }
    if let Some(attributes) = cd.attributes.as_mut() {
        attributes.retain(|a| !attr_is_empty_inherited(a));
    }
    // Stripping every attribute makes the object's attribute list ABSENT, not
    // present-and-empty: ADL 2 writes a member list by writing its members
    // (`docs/specs/openehr/AM/docs/ADL2/master04-syntax.adoc` §Structure — a
    // `matches {…}` block is written only when it carries content), so an
    // object with nothing left states no block at all. Parsing the reference
    // differential form back yields `None`, and the two must agree.
    cd.attributes = cd
        .attributes
        .take()
        .and_then(openehr_base::containers::present);
}

fn strip_attr(attr: &mut CAttribute, parent_def: &CComplexObject) {
    if let Some(children) = attr.children.as_mut() {
        // Keep a node that is redefined/new (has a specialisation depth) or that
        // is not found unchanged in the flat parent.
        children.retain(|child| !is_inherited_unchanged(child, parent_def));
    }
    // Same rule as the attribute list above: no surviving child means the
    // attribute states no children, not an empty child list.
    attr.children = attr
        .children
        .take()
        .and_then(openehr_base::containers::present);
    for child in attr.children.iter_mut().flatten() {
        if let CObject::CComplexObject(cco) = child
            && let Some(pmatch) = find_by_node_id(parent_def, node_id_of_cco(cco))
        {
            strip_inherited(cco, pmatch);
        }
    }
}

fn attr_is_empty_inherited(attr: &CAttribute) -> bool {
    // An attribute with no surviving children (and no local existence/cardinality
    // override) carries nothing differential.
    attr.children.as_ref().is_none_or(Vec::is_empty)
        && attr.existence.is_none()
        && attr.cardinality.is_none()
        && attr.differential_path.is_none()
}

fn is_inherited_unchanged(child: &CObject, parent_def: &CComplexObject) -> bool {
    let CObject::CComplexObject(cco) = child else {
        return false;
    };
    let nid = node_id_of_cco(cco);
    // A `.N`-suffixed (redefined) or synthesised-new node is never
    // inherited-unchanged.
    if nid.contains('.') {
        return false;
    }
    let Some(pmatch) = find_by_node_id(parent_def, nid) else {
        return false;
    };
    structurally_equal(cco, pmatch)
}

fn node_id_of_cco(cco: &CComplexObject) -> &str {
    match cco {
        CComplexObject::CComplexObject(d) => &d.node_id,
        CComplexObject::CArchetypeRoot(r) => &r.node_id,
    }
}

/// Depth-first search for a node with `node_id` anywhere under `def`.
fn find_by_node_id<'a>(def: &'a CComplexObject, node_id: &str) -> Option<&'a CComplexObject> {
    let CComplexObject::CComplexObject(d) = def else {
        return None;
    };
    if d.node_id == node_id {
        return Some(def);
    }
    for attr in d.attributes.iter().flatten() {
        for child in attr.children.iter().flatten() {
            if let CObject::CComplexObject(cco) = child
                && let Some(found) = find_by_node_id(cco, node_id)
            {
                return Some(found);
            }
        }
    }
    None
}

/// Structural equality of two complex objects ignoring parent back-pointers
/// (which are always `None` after assembly). `PartialEq` on the generated model
/// already excludes nothing, but the back-pointers are `None` on both sides, so
/// a direct compare is sound here.
fn structurally_equal(a: &CComplexObject, b: &CComplexObject) -> bool {
    a == b
}

/// Drop terminology entries for codes no longer present in the differential
/// child: after re-differentialisation only the changed nodes remain, so only
/// their terms (and any value sets they reference, with those members and
/// bindings) are kept. The reference fixtures keep only new/changed terms.
fn prune_terminology(data: &mut AuthoredArchetypeData) {
    let mut referenced = std::collections::BTreeSet::new();
    collect_referenced_codes(&data.definition, &mut referenced);
    referenced.insert(data.terminology.concept_code.clone());

    // Keep value sets referenced in the definition; their members become
    // referenced (their term entries + bindings must survive).
    if let Some(vs) = data.terminology.value_sets.as_mut() {
        vs.retain(|id, _| referenced.contains(id));
        for set in vs.values() {
            for m in &set.members {
                referenced.insert(m.clone());
            }
        }
    }
    for terms in data.terminology.term_definitions.values_mut() {
        terms.retain(|code, _| referenced.contains(code));
    }
    if let Some(bindings) = data.terminology.term_bindings.as_mut() {
        for m in bindings.values_mut() {
            m.retain(|code, _| referenced.contains(code));
        }
        bindings.retain(|_, m| !m.is_empty());
    }
}

fn collect_referenced_codes(def: &CComplexObject, out: &mut std::collections::BTreeSet<String>) {
    let CComplexObject::CComplexObject(d) = def else {
        return;
    };
    out.insert(d.node_id.clone());
    for attr in d.attributes.iter().flatten() {
        for child in attr.children.iter().flatten() {
            collect_codes_obj(child, out);
        }
    }
    for tuple in d.attribute_tuples.iter().flatten() {
        for member in tuple.members.iter().flatten() {
            for child in member.children.iter().flatten() {
                collect_codes_obj(child, out);
            }
        }
    }
}

fn collect_codes_obj(obj: &CObject, out: &mut std::collections::BTreeSet<String>) {
    match obj {
        CObject::CComplexObject(cco) => collect_referenced_codes(cco, out),
        CObject::CComplexObjectProxy(p) => {
            out.insert(p.node_id.clone());
        }
        CObject::ArchetypeSlot(s) => {
            out.insert(s.node_id.clone());
        }
        CObject::CTerminologyCode(tc) => {
            out.insert(tc.constraint.clone());
            if let Some(a) = &tc.assumed_value {
                out.insert(a.code_string.clone());
            }
        }
        _ => {}
    }
}
