// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! ADL 1.4 paths over the `Template.xsd` constraint tree.
//!
//! A path is a sequence of `/attribute[node_id]` steps from the archetype
//! root (`AM/docs/ADL1.4/master07-paths.adoc`). A step without a node id
//! descends only into an attribute with exactly one child object, so a path
//! never picks one of several siblings by position. A `C_ARCHETYPE_ROOT`
//! child belongs to another archetype and is never matched.

use crate::opt14::types::{CAttribute, CComplexObject, CObject, Intervalofinteger};

/// One `/attribute[node_id]` step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Step {
    /// The reference-model attribute name.
    pub(super) attribute: String,
    /// The node id in the predicate, if any.
    pub(super) node_id: Option<String>,
}

/// Parses an ADL path into its steps; `/` is the root and has none.
pub(super) fn parse(path: &str) -> Result<Vec<Step>, &'static str> {
    let Some(rest) = path.strip_prefix('/') else {
        return Err("an archetype path starts with `/`");
    };
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    rest.split('/').map(parse_step).collect()
}

fn parse_step(step: &str) -> Result<Step, &'static str> {
    let (attribute, node_id) = match step.split_once('[') {
        None => (step, None),
        Some((attribute, predicate)) => {
            let Some(node_id) = predicate.strip_suffix(']') else {
                return Err("a predicate ends with `]`");
            };
            if node_id.is_empty() || node_id.contains(['[', ']', ' ', '\'', '=']) {
                return Err("a predicate is a single node id");
            }
            (attribute, Some(node_id.to_owned()))
        }
    };
    if attribute.is_empty()
        || !attribute
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err("a step names a reference-model attribute");
    }
    Ok(Step {
        attribute: attribute.to_owned(),
        node_id,
    })
}

/// Renders steps back into the path form, for diagnostics and locations.
pub(super) fn render(steps: &[Step]) -> String {
    if steps.is_empty() {
        return "/".to_owned();
    }
    steps.iter().fold(String::new(), |mut out, step| {
        out.push('/');
        out.push_str(&step.attribute);
        if let Some(id) = &step.node_id {
            out.push('[');
            out.push_str(id);
            out.push(']');
        }
        out
    })
}

/// The node id of a `C_OBJECT`.
pub(super) fn node_id(object: &CObject) -> &str {
    match object {
        CObject::ArchetypeInternalRef(o) => &o.node_id,
        CObject::ArchetypeSlot(o) => &o.node_id,
        CObject::ConstraintRef(o) => &o.node_id,
        CObject::CArchetypeRoot(o) => &o.node_id,
        CObject::CCodePhrase(o) => &o.node_id,
        CObject::CCodeReference(o) => &o.node_id,
        CObject::CComplexObject(o) => &o.node_id,
        CObject::CDefinedObject(o) => &o.node_id,
        CObject::CDvOrdinal(o) => &o.node_id,
        CObject::CDvQuantity(o) => &o.node_id,
        CObject::CDvState(o) => &o.node_id,
        CObject::CPrimitiveObject(o) => &o.node_id,
        CObject::TComplexObject(o) => &o.node_id,
    }
}

/// The reference-model type of a `C_OBJECT`.
pub(super) fn rm_type_name(object: &CObject) -> &str {
    match object {
        CObject::ArchetypeInternalRef(o) => &o.rm_type_name,
        CObject::ArchetypeSlot(o) => &o.rm_type_name,
        CObject::ConstraintRef(o) => &o.rm_type_name,
        CObject::CArchetypeRoot(o) => &o.rm_type_name,
        CObject::CCodePhrase(o) => &o.rm_type_name,
        CObject::CCodeReference(o) => &o.rm_type_name,
        CObject::CComplexObject(o) => &o.rm_type_name,
        CObject::CDefinedObject(o) => &o.rm_type_name,
        CObject::CDvOrdinal(o) => &o.rm_type_name,
        CObject::CDvQuantity(o) => &o.rm_type_name,
        CObject::CDvState(o) => &o.rm_type_name,
        CObject::CPrimitiveObject(o) => &o.rm_type_name,
        CObject::TComplexObject(o) => &o.rm_type_name,
    }
}

/// The node id of a `C_OBJECT`, mutable.
pub(super) fn node_id_mut(object: &mut CObject) -> &mut String {
    match object {
        CObject::ArchetypeInternalRef(o) => &mut o.node_id,
        CObject::ArchetypeSlot(o) => &mut o.node_id,
        CObject::ConstraintRef(o) => &mut o.node_id,
        CObject::CArchetypeRoot(o) => &mut o.node_id,
        CObject::CCodePhrase(o) => &mut o.node_id,
        CObject::CCodeReference(o) => &mut o.node_id,
        CObject::CComplexObject(o) => &mut o.node_id,
        CObject::CDefinedObject(o) => &mut o.node_id,
        CObject::CDvOrdinal(o) => &mut o.node_id,
        CObject::CDvQuantity(o) => &mut o.node_id,
        CObject::CDvState(o) => &mut o.node_id,
        CObject::CPrimitiveObject(o) => &mut o.node_id,
        CObject::TComplexObject(o) => &mut o.node_id,
    }
}

/// The occurrences of a `C_OBJECT`, mutable.
pub(super) fn occurrences_mut(object: &mut CObject) -> &mut Intervalofinteger {
    match object {
        CObject::ArchetypeInternalRef(o) => &mut o.occurrences,
        CObject::ArchetypeSlot(o) => &mut o.occurrences,
        CObject::ConstraintRef(o) => &mut o.occurrences,
        CObject::CArchetypeRoot(o) => &mut o.occurrences,
        CObject::CCodePhrase(o) => &mut o.occurrences,
        CObject::CCodeReference(o) => &mut o.occurrences,
        CObject::CComplexObject(o) => &mut o.occurrences,
        CObject::CDefinedObject(o) => &mut o.occurrences,
        CObject::CDvOrdinal(o) => &mut o.occurrences,
        CObject::CDvQuantity(o) => &mut o.occurrences,
        CObject::CDvState(o) => &mut o.occurrences,
        CObject::CPrimitiveObject(o) => &mut o.occurrences,
        CObject::TComplexObject(o) => &mut o.occurrences,
    }
}

/// The occurrences of a `C_OBJECT`.
pub(super) fn occurrences(object: &CObject) -> &Intervalofinteger {
    match object {
        CObject::ArchetypeInternalRef(o) => &o.occurrences,
        CObject::ArchetypeSlot(o) => &o.occurrences,
        CObject::ConstraintRef(o) => &o.occurrences,
        CObject::CArchetypeRoot(o) => &o.occurrences,
        CObject::CCodePhrase(o) => &o.occurrences,
        CObject::CCodeReference(o) => &o.occurrences,
        CObject::CComplexObject(o) => &o.occurrences,
        CObject::CDefinedObject(o) => &o.occurrences,
        CObject::CDvOrdinal(o) => &o.occurrences,
        CObject::CDvQuantity(o) => &o.occurrences,
        CObject::CDvState(o) => &o.occurrences,
        CObject::CPrimitiveObject(o) => &o.occurrences,
        CObject::TComplexObject(o) => &o.occurrences,
    }
}

/// The attributes a path descends through: those of a complex object of
/// this archetype. A `C_ARCHETYPE_ROOT` is another archetype's.
pub(super) fn attributes(object: &CObject) -> Option<&Vec<CAttribute>> {
    match object {
        CObject::CComplexObject(o) => Some(&o.attributes),
        CObject::TComplexObject(o) => Some(&o.attributes),
        _ => None,
    }
}

/// The attributes of a complex object of this archetype, mutable.
pub(super) fn attributes_mut(object: &mut CObject) -> Option<&mut Vec<CAttribute>> {
    match object {
        CObject::CComplexObject(o) => Some(&mut o.attributes),
        CObject::TComplexObject(o) => Some(&mut o.attributes),
        _ => None,
    }
}

/// The attribute name of a `C_ATTRIBUTE`.
pub(super) fn attribute_name(attribute: &CAttribute) -> &str {
    match attribute {
        CAttribute::CMultipleAttribute(a) => &a.rm_attribute_name,
        CAttribute::CSingleAttribute(a) => &a.rm_attribute_name,
    }
}

/// The child objects of a `C_ATTRIBUTE`.
pub(super) fn children(attribute: &CAttribute) -> &Vec<CObject> {
    match attribute {
        CAttribute::CMultipleAttribute(a) => &a.children,
        CAttribute::CSingleAttribute(a) => &a.children,
    }
}

/// The child objects of a `C_ATTRIBUTE`, mutable.
pub(super) fn children_mut(attribute: &mut CAttribute) -> &mut Vec<CObject> {
    match attribute {
        CAttribute::CMultipleAttribute(a) => &mut a.children,
        CAttribute::CSingleAttribute(a) => &mut a.children,
    }
}

/// The existence of a `C_ATTRIBUTE`.
pub(super) fn existence(attribute: &CAttribute) -> &Intervalofinteger {
    match attribute {
        CAttribute::CMultipleAttribute(a) => &a.existence,
        CAttribute::CSingleAttribute(a) => &a.existence,
    }
}

/// The existence of a `C_ATTRIBUTE`, mutable.
pub(super) fn existence_mut(attribute: &mut CAttribute) -> &mut Intervalofinteger {
    match attribute {
        CAttribute::CMultipleAttribute(a) => &mut a.existence,
        CAttribute::CSingleAttribute(a) => &mut a.existence,
    }
}

/// Returns whether a child object is the one a step's predicate names.
fn matches_step(object: &CObject, node_id: Option<&str>) -> bool {
    !matches!(object, CObject::CArchetypeRoot(_))
        && node_id.is_none_or(|id| self::node_id(object) == id)
}

/// The indices of the children a step selects: the one its predicate names,
/// or the only own child when it names none.
fn select(children: &[CObject], node_id: Option<&str>) -> Vec<usize> {
    let own: Vec<usize> = children
        .iter()
        .enumerate()
        .filter(|(_, child)| matches_step(child, node_id))
        .map(|(index, _)| index)
        .collect();
    if node_id.is_none() && own.len() > 1 {
        return Vec::new();
    }
    own
}

/// A resolution failure: how many nodes the path matched (zero or several).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Unresolved {
    /// The number of nodes matched where exactly one was required.
    pub(super) found: usize,
}

/// Finds the object node at `steps` below `root`, read-only.
pub(super) fn object<'a>(
    root: &'a CComplexObject,
    steps: &[Step],
) -> Result<&'a CObject, Unresolved> {
    let Some((last, parents)) = steps.split_last() else {
        return Err(Unresolved { found: 0 });
    };
    let mut attrs = &root.attributes;
    for step in parents {
        let child = only(attrs, step)?;
        attrs = attributes(child).ok_or(Unresolved { found: 0 })?;
    }
    only(attrs, last)
}

fn only<'a>(attrs: &'a [CAttribute], step: &Step) -> Result<&'a CObject, Unresolved> {
    let attribute = attrs
        .iter()
        .find(|a| attribute_name(a) == step.attribute)
        .ok_or(Unresolved { found: 0 })?;
    let picked = select(children(attribute), step.node_id.as_deref());
    match picked.as_slice() {
        [index] => children(attribute)
            .get(*index)
            .ok_or(Unresolved { found: 0 }),
        other => Err(Unresolved { found: other.len() }),
    }
}

/// Descends to the attribute list that holds the last step's attribute.
fn parent_attributes_mut<'a>(
    root: &'a mut Vec<CAttribute>,
    parents: &[Step],
) -> Result<&'a mut Vec<CAttribute>, Unresolved> {
    let mut attrs = root;
    for step in parents {
        let attribute = attrs
            .iter_mut()
            .find(|a| attribute_name(a) == step.attribute)
            .ok_or(Unresolved { found: 0 })?;
        let picked = select(children(attribute), step.node_id.as_deref());
        let [index] = picked.as_slice() else {
            return Err(Unresolved {
                found: picked.len(),
            });
        };
        let child = children_mut(attribute)
            .get_mut(*index)
            .ok_or(Unresolved { found: 0 })?;
        attrs = attributes_mut(child).ok_or(Unresolved { found: 0 })?;
    }
    Ok(attrs)
}

/// Finds the object node at `steps` below `root` as its parent attribute's
/// child list plus its index there, so a caller can replace, insert or remove.
pub(super) fn object_slot<'a>(
    root: &'a mut Vec<CAttribute>,
    steps: &[Step],
) -> Result<(&'a mut Vec<CObject>, usize), Unresolved> {
    let Some((last, parents)) = steps.split_last() else {
        return Err(Unresolved { found: 0 });
    };
    let attrs = parent_attributes_mut(root, parents)?;
    let attribute = attrs
        .iter_mut()
        .find(|a| attribute_name(a) == last.attribute)
        .ok_or(Unresolved { found: 0 })?;
    let picked = select(children(attribute), last.node_id.as_deref());
    let [index] = picked.as_slice() else {
        return Err(Unresolved {
            found: picked.len(),
        });
    };
    let index = *index;
    Ok((children_mut(attribute), index))
}

/// Finds the attribute at `steps` (the last step names it and carries no
/// predicate) as its owner's attribute list plus its index there.
pub(super) fn attribute_slot<'a>(
    root: &'a mut Vec<CAttribute>,
    steps: &[Step],
) -> Result<(&'a mut Vec<CAttribute>, usize), Unresolved> {
    let Some((last, parents)) = steps.split_last() else {
        return Err(Unresolved { found: 0 });
    };
    if last.node_id.is_some() {
        return Err(Unresolved { found: 0 });
    }
    let attrs = parent_attributes_mut(root, parents)?;
    let index = attrs
        .iter()
        .position(|a| attribute_name(a) == last.attribute)
        .ok_or(Unresolved { found: 0 })?;
    Ok((attrs, index))
}

/// Returns whether `ancestor` is a prefix of `steps` (or equal to it).
pub(super) fn is_prefix(ancestor: &[Step], steps: &[Step]) -> bool {
    ancestor.len() <= steps.len() && steps.iter().zip(ancestor).all(|(a, b)| a == b)
}

#[cfg(test)]
mod tests {
    use super::{Step, is_prefix, parse, render};

    #[test]
    fn a_path_parses_into_steps_and_renders_back() {
        let path = "/data[at0001]/events[at0002]/data/items[at0004]";
        let steps = parse(path).expect("a well-formed path");
        assert_eq!(steps.len(), 4);
        assert_eq!(
            steps.get(2),
            Some(&Step {
                attribute: "data".to_owned(),
                node_id: None
            })
        );
        assert_eq!(render(&steps), path);
        assert_eq!(parse("/").expect("the root"), Vec::new());
        assert_eq!(render(&[]), "/");
    }

    #[test]
    fn a_malformed_path_is_refused() {
        assert!(parse("data[at0001]").is_err());
        assert!(parse("/data[at0001").is_err());
        assert!(parse("/data[at0001 and name/value='x']").is_err());
        assert!(parse("/data[]").is_err());
        assert!(parse("//items").is_err());
    }

    #[test]
    fn a_prefix_is_an_ancestor() {
        let parent = parse("/data[at0001]").expect("path");
        let child = parse("/data[at0001]/items[at0002]").expect("path");
        assert!(is_prefix(&parent, &child));
        assert!(!is_prefix(&child, &parent));
    }
}
