// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Archetype path utilities over the parsed AOM2 constraint model.
//!
//! Path resolution, enumeration, and existence checks against a
//! [`CComplexObject`] tree — the model-level half of the ADL path grammar
//! (`docs/specs/openehr/AM/docs/ADL2/master05-paths.adoc`; the outer lexer
//! already tokenises paths). These are used by the phase-1 validation
//! catalogue (VRANP annotation paths, VTTBK binding paths, VRRLP rule paths)
//! and by the VUNP `C_COMPLEX_OBJECT_PROXY`-target check, which resolves and
//! compares proxy targets through [`locate`], [`resolve`] and
//! [`is_ancestor_path`] rather than any grammar of its own.
//!
//! Model-level only: this walks attribute + node-id predicates over the
//! archetype's own constraint tree and knows **nothing** about the reference
//! model. A path segment carrying only an RM attribute name (no `[id…]`
//! predicate) that leaves the archetype tree is an RM-path extension whose
//! validity is a reference-model question — resolution stops and reports
//! "left the archetype" rather than guessing (RM path validity is a
//! reference-model concern, `crate::validate::rm`). See [`resolve`].

use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

use crate::aom::access::{complex_attributes, object_node_id};
use crate::codes::code_prefix;

/// One segment of an archetype path: an RM attribute name plus an optional
/// node-id predicate (`items[id15]` → attribute `items`, node id `id15`).
///
/// Non-code predicates (a meaning name, a position integer, a `[at0003|label|]`
/// display form) contribute their code part to `node_id` where one is present
/// (`at0003`) and otherwise leave `node_id` `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSegment {
    /// The RM attribute name (the un-predicated part of the segment).
    pub attribute: String,
    /// The node-id predicate (`idN`/`atN`/`acN`) if the segment carries one.
    pub node_id: Option<String>,
}

/// Parses an archetype path string into its ordered [`PathSegment`]s.
///
/// Grammar (master05): `'/'? segment ('/' segment)+` with
/// `segment = attr ('[' id ']')?`. A leading `/` is ignored; the empty /
/// root path yields an empty segment list.
#[must_use]
pub fn parse_path(path: &str) -> Vec<PathSegment> {
    let trimmed = path.trim();
    let body = trimmed.strip_prefix('/').unwrap_or(trimmed);
    if body.is_empty() {
        return Vec::new();
    }
    split_top_level(body)
        .into_iter()
        .map(|seg| parse_segment(&seg))
        .collect()
}

/// Split a path body on `/` separators that are **not** inside a `[…]`
/// predicate (predicates may contain `/` inside a quoted display form).
fn split_top_level(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for ch in body.chars() {
        match ch {
            '[' => {
                depth += 1;
                cur.push(ch);
            }
            ']' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            '/' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Parse a single `attr[predicate]` segment.
fn parse_segment(seg: &str) -> PathSegment {
    if let Some(open) = seg.find('[') {
        let attribute = seg.get(..open).unwrap_or("").trim().to_owned();
        let inner = seg
            .get(open + 1..)
            .and_then(|s| s.strip_suffix(']').or(Some(s)))
            .unwrap_or("");
        PathSegment {
            attribute,
            node_id: predicate_code(inner),
        }
    } else {
        PathSegment {
            attribute: seg.trim().to_owned(),
            node_id: None,
        }
    }
}

/// Extract the leading local-code part of a predicate body, or `None` if the
/// predicate is a name / position (`items[id15]` → `id15`;
/// `items[at0003|label|]` → `at0003`; `items[some name]` → `None`).
fn predicate_code(inner: &str) -> Option<String> {
    let head = inner.split('|').next().unwrap_or(inner).trim();
    if code_prefix(head).is_some()
        && head
            .bytes()
            .skip(2)
            .all(|b| b.is_ascii_digit() || b == b'.')
    {
        Some(head.to_owned())
    } else {
        None
    }
}

/// The outcome of resolving a path against an archetype tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The path resolves to an object node inside the archetype.
    Found,
    /// A segment carrying a node-id predicate did not match any node — the
    /// path is invalid with respect to the archetype.
    NotFound,
    /// Resolution left the archetype tree at a pure-RM (un-predicated) segment
    /// whose validity is a reference-model question, not an archetype one.
    LeftArchetype,
}

/// Resolve `path` against the `root` object tree.
///
/// - [`Resolution::Found`] — every segment matched an attribute + object node.
/// - [`Resolution::NotFound`] — a segment with a node-id predicate found no
///   matching node (the path is archetype-invalid).
/// - [`Resolution::LeftArchetype`] — a pure-RM (un-predicated) segment could
///   not continue within the archetype tree; whether it is a legal RM
///   extension is a reference-model question (`crate::validate::rm`).
#[must_use]
pub fn resolve(root: &CComplexObject, path: &str) -> Resolution {
    let segments = parse_path(path);
    if segments.is_empty() {
        return Resolution::Found; // the root path
    }
    let mut current: &CComplexObject = root;
    for (idx, seg) in segments.iter().enumerate() {
        let Some(attr) = complex_attributes(current)
            .iter()
            .find(|a| a.rm_attribute_name == seg.attribute)
        else {
            // No such constrained attribute here.
            return if seg.node_id.is_some() {
                Resolution::NotFound
            } else {
                Resolution::LeftArchetype
            };
        };
        // Choose the child object.
        let child = match &seg.node_id {
            Some(nid) => attr
                .children
                .iter()
                .flatten()
                .find(|c| object_node_id(c) == nid),
            None if attr.children.as_ref().map_or(0, Vec::len) == 1 => {
                attr.children.iter().flatten().next()
            }
            None => None,
        };
        let Some(child) = child else {
            return if seg.node_id.is_some() {
                Resolution::NotFound
            } else {
                Resolution::LeftArchetype
            };
        };
        let is_last = idx + 1 == segments.len();
        match child {
            CObject::CComplexObject(cco) => current = cco,
            _ if is_last => return Resolution::Found,
            // A non-complex child (primitive / slot / proxy) has no further
            // attributes to walk into.
            _ => return Resolution::LeftArchetype,
        }
    }
    Resolution::Found
}

/// True iff `path` resolves to an object node inside the archetype
/// ([`Resolution::Found`]).
#[must_use]
pub fn path_exists(root: &CComplexObject, path: &str) -> bool {
    resolve(root, path) == Resolution::Found
}

/// Locate the object node `path` addresses inside `root`, returning it when
/// the path resolves to a node within the archetype ([`Resolution::Found`]),
/// else `None`.
///
/// Mirrors [`resolve`] but yields the target [`CObject`] (used to resolve an
/// `EXPR_ARCHETYPE_REF` proxy to its target node — `AOM2` master05).
#[must_use]
pub fn locate<'a>(root: &'a CComplexObject, path: &str) -> Option<&'a CObject> {
    let segments = parse_path(path);
    if segments.is_empty() {
        return None;
    }
    let mut current: &'a CComplexObject = root;
    for (idx, seg) in segments.iter().enumerate() {
        let attr = complex_attributes(current)
            .iter()
            .find(|a| a.rm_attribute_name == seg.attribute)?;
        let child = match &seg.node_id {
            Some(nid) => attr
                .children
                .iter()
                .flatten()
                .find(|c| object_node_id(c) == nid),
            None if attr.children.as_ref().map_or(0, Vec::len) == 1 => {
                attr.children.iter().flatten().next()
            }
            None => None,
        }?;
        if idx + 1 == segments.len() {
            return Some(child);
        }
        match child {
            CObject::CComplexObject(cco) => current = cco,
            _ => return None,
        }
    }
    None
}

/// True when `target` addresses an ancestor of (or the same node as) `proxy`.
///
/// An internal reference whose target lies on its own ancestor path expands
/// into an infinitely recursive deep copy, which is why
/// `AM/docs/ADL2/master04.3-cadl_complex_types.adoc` §Internal References
/// requires that "the path must not be in the parent path of the proxy object
/// itself, but may be a sibling".
///
/// Both sides go through [`parse_path`], so the ancestor relation is decided
/// segment-wise on attribute name + node-id predicate rather than on the raw
/// string — a string prefix would false-positive on `/items[id2]` against
/// `/items[id22]`. A target segment carrying no node-id predicate names the
/// attribute only and so cannot exclude the proxy's own branch; a root target
/// (zero segments) is the ultimate ancestor. Sibling and cross-branch targets
/// differ in some segment and stay legal.
#[must_use]
pub fn is_ancestor_path(target: &str, proxy: &str) -> bool {
    let target_segments = parse_path(target);
    let proxy_segments = parse_path(proxy);
    if target_segments.len() > proxy_segments.len() {
        return false;
    }
    target_segments
        .iter()
        .zip(&proxy_segments)
        .all(|(t, p)| t.attribute == p.attribute && (t.node_id.is_none() || t.node_id == p.node_id))
}

/// True if `path` carries at least one node-id predicate (`[idN]`/`[atN]`) — an
/// archetype-specific path that must resolve within the archetype, as opposed
/// to a pure reference-model path.
#[must_use]
pub fn has_node_id_predicate(path: &str) -> bool {
    parse_path(path).iter().any(|s| s.node_id.is_some())
}

/// Enumerate every object-node path in the archetype (each identified object
/// node reached by attribute + node-id predicates), rooted at `/`.
#[must_use]
pub fn enumerate_paths(root: &CComplexObject) -> Vec<String> {
    let mut out = Vec::new();
    walk_paths(root, "", &mut out);
    out
}

fn walk_paths(node: &CComplexObject, prefix: &str, out: &mut Vec<String>) {
    for attr in complex_attributes(node) {
        for child in attr.children.iter().flatten() {
            let nid = object_node_id(child);
            let seg = if nid.is_empty() {
                format!("/{}", attr.rm_attribute_name)
            } else {
                format!("/{}[{}]", attr.rm_attribute_name, nid)
            };
            let child_path = format!("{prefix}{seg}");
            out.push(child_path.clone());
            if let CObject::CComplexObject(cco) = child {
                walk_paths(cco, &child_path, out);
            }
        }
    }
}

/// The path string of a child object under `attr_path`: `attr_path[node_id]`,
/// or bare `attr_path` when the child carries no node id
/// (`ADL2/master05-paths.adoc` §Overview — the `attribute[predicate]` segment
/// form [`parse_path`] reads).
pub(crate) fn child_path(attr_path: &str, node_id: &str) -> String {
    if node_id.is_empty() {
        attr_path.to_owned()
    } else {
        format!("{attr_path}[{node_id}]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::parse_artefact;
    use crate::parse::Dialect;
    use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
    use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;

    #[test]
    fn parse_path_splits_segments_and_predicates() {
        let segs = parse_path("/data[id2]/items[id15]");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].attribute, "data");
        assert_eq!(segs[0].node_id.as_deref(), Some("id2"));
        assert_eq!(segs[1].node_id.as_deref(), Some("id15"));
        // pure RM path
        let rm = parse_path("/context/start_time");
        assert_eq!(rm.len(), 2);
        assert!(rm.iter().all(|s| s.node_id.is_none()));
        assert!(!has_node_id_predicate("/context/start_time"));
        assert!(has_node_id_predicate("/data[id2]"));
    }

    #[test]
    fn display_predicate_yields_code() {
        let segs = parse_path("/items[at0003|blood pressure|]");
        assert_eq!(segs[0].node_id.as_deref(), Some("at0003"));
    }

    fn root_of(src: &str) -> CComplexObject {
        match parse_artefact(src, Dialect::Adl2).unwrap() {
            Archetype::AuthoredArchetype(a) => match *a {
                AuthoredArchetype::AuthoredArchetype(d) => d.definition,
                AuthoredArchetype::Template(t) => t.definition,
                AuthoredArchetype::OperationalTemplate(o) => o.definition,
            },
            Archetype::TemplateOverlay(t) => t.definition,
        }
    }

    #[test]
    fn resolve_over_a_real_tree() {
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-ENTRY.paths_test.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"draft\">

definition
    ENTRY[id1] matches {
        element_attr matches {
            ELEMENT[id2]
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = < text = <\"\"> description = <\"\"> >
            [\"id2\"] = < text = <\"\"> description = <\"\"> >
        >
    >
";
        let root = root_of(src);
        assert_eq!(resolve(&root, "/element_attr[id2]"), Resolution::Found);
        assert_eq!(resolve(&root, "/element_attr[id99]"), Resolution::NotFound);
        assert_eq!(resolve(&root, "/no_such_attr[id2]"), Resolution::NotFound);
        assert_eq!(resolve(&root, "/context/foo"), Resolution::LeftArchetype);
        assert!(path_exists(&root, "/element_attr[id2]"));
        let paths = enumerate_paths(&root);
        assert!(paths.contains(&"/element_attr[id2]".to_owned()));
    }
}
