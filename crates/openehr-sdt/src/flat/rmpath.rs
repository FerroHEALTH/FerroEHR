// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RM-path parsing and navigation for the RM ⇄ simplified walkers and the
//! composition validator, over [`openehr_rm::v1_2::paths`] (BASE
//! `master11-paths`).
//!
//! It adds the relative path between a parent and child `aqlPath`, which
//! includes the structural nodes the web template compacted away, and
//! multi-root navigation.
#![allow(
    dead_code,
    reason = "consumed by the RM ⇄ sim walkers landing in this same rewrite; drop this allow with their arrival"
)]

use crate::flat::instance::{Instance, select_children};
use openehr_rm::v1_2::paths::{PathSegment, RmPath};

/// Parse a relative RM path (`/attr[pred]/attr2/…`) into its segments; a parse
/// error yields no segments, so the path locates nothing.
pub(crate) fn parse(rel: &str) -> Vec<PathSegment> {
    rel.parse::<RmPath>()
        .map(|p| p.segments)
        .unwrap_or_default()
}

/// The segments from `parent_aql` to `child_aql`, compared step by step as
/// [`crate::flat::navigate::relative`] does; the whole child path when it does
/// not extend the parent's.
pub(crate) fn relative(parent_aql: &str, child_aql: &str) -> Vec<PathSegment> {
    let parent = parse(parent_aql);
    let child = parse(child_aql);
    match child.strip_prefix(parent.as_slice()) {
        Some(tail) => tail.to_vec(),
        None => child,
    }
}

/// The segments from `parent` to its child `index` (`child`), as [`relative`]
/// gives them, read from the parent's prepared walk plan when it has one.
pub(crate) fn child_segments<'a>(
    parent: &'a crate::flat::webtemplate::model::WebTemplateNode,
    index: usize,
    child: &crate::flat::webtemplate::model::WebTemplateNode,
) -> std::borrow::Cow<'a, [PathSegment]> {
    match parent
        .walk
        .as_ref()
        .and_then(|plan| plan.segments_of(parent, index, child))
    {
        Some(cached) => std::borrow::Cow::Borrowed(cached),
        None => std::borrow::Cow::Owned(relative(&parent.aql_path, &child.aql_path)),
    }
}

/// Follow `segs` from each of `roots`, returning the reached nodes in document
/// order ([`openehr_rm::v1_2::paths::select_children`] per step).
///
pub(crate) fn navigate<'a, I: Instance<'a>>(roots: &[I], segs: &[PathSegment]) -> Vec<I> {
    let mut current: Vec<I> = roots.to_vec();
    for seg in segs {
        current = current
            .iter()
            .flat_map(|n| select_children(*n, seg))
            .collect();
    }
    current
}

/// One template-path step with a conditional name fallback.
///
/// NOTE: an instance may redefine `LOCATABLE.name` (RM common), so the id-only
/// fallback applies when the name is redundant among the siblings
/// (`allow_name_fallback`).
pub(crate) fn select_children_matched<'a, I: Instance<'a>>(
    container: I,
    seg: &PathSegment,
    allow_name_fallback: bool,
) -> Vec<I> {
    let strict = select_children(container, seg);
    if strict.is_empty() && allow_name_fallback && seg.predicate.name_value.is_some() {
        let mut id_only = seg.clone();
        id_only.predicate.name_value = None;
        return select_children(container, &id_only);
    }
    strict
}

/// Select the children matching an unqualified identity segment that shares its
/// `archetype_node_id` with name-qualified siblings, excluding the instances
/// those siblings claim (RM common `master03-archetyped_package.adoc` §"The
/// `LOCATABLE` class"; AOM 1.4 `master04-constraint_model_package.adoc`
/// §`node_id`).
pub(crate) fn select_children_excluding_names<'a, I: Instance<'a>>(
    container: I,
    seg: &PathSegment,
    excluded_names: &[String],
) -> Vec<I> {
    select_children(container, seg)
        .into_iter()
        .filter(|node| {
            node.attr("name")
                .and_then(|n| n.str_at("value"))
                .is_none_or(|name| !excluded_names.iter().any(|e| e == name))
        })
        .collect()
}

/// Per-parent decision table for the id-only name fallback of
/// [`select_children_matched`].
///
/// `Some(name)` for an identity every template path under the parent spells
/// identically, where dropping the name conjunct is sound; `None` for a
/// discriminating identity. An absent identity takes no fallback.
pub(crate) type NameFallback = std::collections::HashMap<(String, String), Option<String>>;

/// Build the [`NameFallback`] table from every predicate-bearing segment of a
/// parent's template paths (each already parsed relative to that parent).
pub(crate) fn name_fallback_from_segments<'a>(
    paths: impl Iterator<Item = &'a [PathSegment]>,
) -> NameFallback {
    let mut out: NameFallback = std::collections::HashMap::new();
    for segs in paths {
        for seg in segs {
            let Some(id) = &seg.predicate.archetype_node_id else {
                continue;
            };
            let key = (seg.attribute.clone(), id.clone());
            let name = seg.predicate.name_value.clone();
            match out.entry(key) {
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(name);
                }
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    if *o.get() != name {
                        o.insert(None);
                    }
                }
            }
        }
    }
    out
}

/// Build the [`NameFallback`] table from a parent's absolute template paths,
/// parsing each relative to `parent_aql` first.
pub(crate) fn name_fallback_from_paths<'a>(
    parent_aql: &str,
    paths: impl Iterator<Item = &'a str>,
) -> NameFallback {
    let parsed: Vec<Vec<PathSegment>> = paths.map(|p| relative(parent_aql, p)).collect();
    name_fallback_from_segments(parsed.iter().map(Vec::as_slice))
}

/// Whether `seg`'s name conjunct is redundant under `table` — the condition
/// [`NameFallback`] documents.
pub(crate) fn fallback_allowed(table: &NameFallback, seg: &PathSegment) -> bool {
    let Some(id) = &seg.predicate.archetype_node_id else {
        return false;
    };
    table
        .get(&(seg.attribute.clone(), id.clone()))
        .is_some_and(|uniform| uniform.as_ref() == seg.predicate.name_value.as_ref())
}

/// [`navigate`] with the id-only name fallback of [`select_children_matched`]
/// per step, as `table` permits, so a renamed instance node stays addressable.
pub(crate) fn navigate_matched<'a, I: Instance<'a>>(
    roots: &[I],
    segs: &[PathSegment],
    table: &NameFallback,
) -> Vec<I> {
    let mut current: Vec<I> = roots.to_vec();
    for seg in segs {
        let allow = fallback_allowed(table, seg);
        current = current
            .iter()
            .flat_map(|n| select_children_matched(*n, seg, allow))
            .collect();
    }
    current
}

/// Resolve the RM value(s) a full relative path reaches from `rm` (an empty
/// segment list resolves to `rm` itself).
pub(crate) fn resolve<'a, I: Instance<'a>>(rm: I, segs: &[PathSegment]) -> Vec<I> {
    navigate(&[rm], segs)
}

/// [`resolve`] with the per-step name fallback of [`navigate_matched`].
pub(crate) fn resolve_matched<'a, I: Instance<'a>>(
    rm: I,
    segs: &[PathSegment],
    table: &NameFallback,
) -> Vec<I> {
    navigate_matched(&[rm], segs, table)
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "the tests read the JSON form of an instance the 553 path keeps (#1694)"
)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn parses_segments() {
        let segs = parse(
            "/content[openEHR-EHR-SECTION.ispek_dialog.v1,'Vitals']/items[openEHR-EHR-OBSERVATION.lab_test-hba1c.v1]/data[at0001]/value",
        );
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0].attribute, "content");
        assert_eq!(
            segs[0].predicate.archetype_node_id.as_deref(),
            Some("openEHR-EHR-SECTION.ispek_dialog.v1")
        );
        assert_eq!(segs[0].predicate.name_value.as_deref(), Some("Vitals"));
        assert_eq!(segs[2].attribute, "data");
        assert_eq!(
            segs[2].predicate.archetype_node_id.as_deref(),
            Some("at0001")
        );
        assert_eq!(segs[3].attribute, "value");
        assert!(segs[3].predicate.is_empty());
    }

    #[test]
    fn relative_strips_parent() {
        let rel = relative(
            "/content[openEHR-EHR-OBSERVATION.x.v1]",
            "/content[openEHR-EHR-OBSERVATION.x.v1]/data[at0001]/events[at0002]",
        );
        assert_eq!(rel.len(), 2);
        assert_eq!(rel[0].attribute, "data");
        assert_eq!(rel[1].attribute, "events");
    }

    #[test]
    fn matched_step_falls_back_to_id_only_for_renamed_instances() {
        // The instance renamed the node; the fallback still locates it by
        // archetype_node_id.
        let rm = serde_json::json!({
            "content": [
                {"archetype_node_id": "openEHR-EHR-EVALUATION.a.v1",
                 "name": {"value": "Runtime Name"},
                 "data": {"archetype_node_id": "at0001", "_type": "ITEM_LIST"}}
            ]
        });
        let segs = parse("/content[openEHR-EHR-EVALUATION.a.v1,'Template Name']/data[at0001]");
        let step = select_children_matched(&rm, &segs[0], true);
        assert_eq!(step.len(), 1);
        assert_eq!(step[0]["name"]["value"], "Runtime Name");
        // Fallback disallowed (same-id siblings distinguished by name): strict
        // only, nothing matches.
        assert_eq!(
            select_children_matched(&rm, &segs[0], false),
            Vec::<&Value>::new()
        );
    }

    #[test]
    fn matched_step_still_disambiguates_matching_siblings() {
        // Two same-id siblings with distinct (template-constrained) names: the
        // strict match succeeds and must NOT widen to both, fallback or not.
        let rm = serde_json::json!({
            "content": [
                {"archetype_node_id": "openEHR-EHR-OBSERVATION.x.v1",
                 "name": {"value": "First"}, "tag": 1},
                {"archetype_node_id": "openEHR-EHR-OBSERVATION.x.v1",
                 "name": {"value": "Second"}, "tag": 2}
            ]
        });
        let segs = parse("/content[openEHR-EHR-OBSERVATION.x.v1,'Second']");
        let found = select_children_matched(&rm, &segs[0], true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["tag"], 2);
    }

    #[test]
    fn a_redundant_name_conjunct_permits_the_id_only_fallback() {
        // One sibling path spells `events[at0002,'Point in time']` and nothing
        // else claims `events[at0002]`, so the name identifies nothing the id
        // does not — an instance that renamed the node is still reached.
        let table = name_fallback_from_paths(
            "/content[openEHR-EHR-OBSERVATION.x.v1]",
            [
                "/content[openEHR-EHR-OBSERVATION.x.v1]/data[at0001]/events[at0002,'Point in time']/data[at0003]/items[at0004]/value",
                "/content[openEHR-EHR-OBSERVATION.x.v1]/language",
            ]
            .into_iter(),
        );
        let rm = serde_json::json!({
            "data": {"archetype_node_id": "at0001", "events": [{
                "archetype_node_id": "at0002",
                "name": {"value": "at0002"},
                "data": {"archetype_node_id": "at0003", "items": [{
                    "archetype_node_id": "at0004",
                    "value": {"_type": "DV_TEXT", "value": "hit"}
                }]}
            }]}
        });
        let segs =
            parse("/data[at0001]/events[at0002,'Point in time']/data[at0003]/items[at0004]/value");
        assert!(
            resolve(&rm, &segs).is_empty(),
            "the strict match locates nothing"
        );
        let found = resolve_matched(&rm, &segs, &table);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["value"], "hit");
    }

    #[test]
    fn a_discriminating_name_conjunct_refuses_the_id_only_fallback() {
        // Two sibling paths spell the same identity under different names, so
        // dropping the name would let one sibling claim the other's instances.
        let table = name_fallback_from_paths(
            "",
            [
                "/content[openEHR-EHR-SECTION.adhoc.v1,'Reporting']/items[at0001]/value",
                "/content[openEHR-EHR-SECTION.adhoc.v1,'Outcome']/items[at0001]/value",
            ]
            .into_iter(),
        );
        let rm = serde_json::json!({
            "content": [{
                "archetype_node_id": "openEHR-EHR-SECTION.adhoc.v1",
                "name": {"value": "Neither"},
                "items": [{"archetype_node_id": "at0001", "value": {"_type": "DV_TEXT", "value": "x"}}]
            }]
        });
        let segs = parse("/content[openEHR-EHR-SECTION.adhoc.v1,'Reporting']/items[at0001]/value");
        assert!(
            resolve_matched(&rm, &segs, &table).is_empty(),
            "a name-differentiated sibling never widens to the bare node id"
        );
    }

    #[test]
    fn an_unqualified_sibling_sharing_the_identity_refuses_the_fallback() {
        // One path names the identity and another leaves it unconstrained; the
        // unqualified sibling is the residual arm, so the named one must not
        // reach past its own name.
        let table = name_fallback_from_paths(
            "",
            ["/items[at0001,'Named']/value", "/items[at0001]/value"].into_iter(),
        );
        let segs = parse("/items[at0001,'Named']/value");
        let rm = serde_json::json!({
            "items": [{"archetype_node_id": "at0001", "name": {"value": "Other"},
                       "value": {"_type": "DV_TEXT", "value": "x"}}]
        });
        assert_eq!(resolve_matched(&rm, &segs, &table), Vec::<&Value>::new());
    }

    #[test]
    fn resolves_through_arrays_and_objects() {
        let rm = serde_json::json!({
            "data": {
                "events": [
                    {"archetype_node_id": "at0002", "value": {"_type": "DV_TEXT", "value": "a"}},
                    {"archetype_node_id": "at0009", "value": {"_type": "DV_TEXT", "value": "b"}}
                ]
            }
        });
        let segs = parse("/data/events[at0002]/value");
        let found = resolve(&rm, &segs);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["value"], "a");
    }
}
