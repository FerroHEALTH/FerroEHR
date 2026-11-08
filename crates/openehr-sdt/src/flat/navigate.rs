// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Template-aware navigation of a composition: the instances of a Web Template
//! node, and the relative path between two template paths.
//!
//! A Web Template node's `aqlPath` names the node, and a composition may hold
//! it any number of times, under names and identifiers the template's path
//! does not spell. Two such differences are resolved here, each from what the
//! template itself records, so a template path reaches every instance it
//! describes:
//!
//! - a renamed instance: a template's `[atNNNN,'Name']` conjunct spells the
//!   name the archetype constrains, while an instance may carry another
//!   `LOCATABLE.name` (RM common `master03-archetyped_package.adoc` §"The
//!   `LOCATABLE` class"). The name is dropped exactly where it is redundant,
//!   when every template path under the same parent spells the same name for
//!   that attribute and node id.
//! - an archetype root an OPT2 inlined: data records the complete identifier
//!   (BASE `artefact_identification` `master07-referencing.adoc`
//!   §Reconstitutability) where the `aqlPath` carries the interface form, and
//!   [`WebTemplate::archetype_refs`] records which identifier each interface
//!   form stands for.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON is the composition this navigation reads (#1694)"
)]

use std::collections::BTreeMap;

use openehr_rm::v1_2::paths::{PathSegment, Predicate, RmPath};
use serde_json::Value;

use crate::flat::rmpath::{NameFallback, fallback_allowed, name_fallback_from_segments};
use crate::flat::webtemplate::model::{WebTemplate, WebTemplateNode};

/// Returns the path from `parent` down to `child`, `None` when `child` is not
/// under `parent`.
///
/// The paths compare step by step, so `/data` is no parent of `/data_x` and
/// a step's predicate must match whole. The result is relative (BASE
/// `architecture_overview/master11-paths.adoc` §Basic Syntax: only an absolute
/// path starts with `/`), and appending it to `parent` gives `child`.
#[must_use]
pub fn relative(parent: &RmPath, child: &RmPath) -> Option<RmPath> {
    let tail = child.segments.strip_prefix(parent.segments.as_slice())?;
    Some(RmPath {
        absolute: false,
        segments: tail.to_vec(),
    })
}

/// Navigates compositions of one Web Template.
///
/// Built once per template, it holds the name-fallback decision of every
/// parent the template's paths pass through and the OPT2 archetype
/// references, so each step decides in constant time.
#[derive(Debug, Clone)]
pub struct Navigator<'t> {
    /// The name-fallback table of each parent, keyed by the parent's template
    /// path rendered with no positions.
    fallbacks: BTreeMap<String, NameFallback>,
    /// [`WebTemplate::archetype_refs`].
    references: Option<&'t BTreeMap<String, String>>,
}

/// One step of an [`Instance`]: the template segment it followed, the node it
/// reached, and where that node sits.
#[derive(Debug, Clone)]
pub struct InstanceStep<'a> {
    segment: PathSegment,
    value: &'a Value,
    occurrence: usize,
    index: Option<usize>,
}

impl<'a> InstanceStep<'a> {
    /// The template segment the step followed, with no position.
    #[must_use]
    pub const fn segment(&self) -> &PathSegment {
        &self.segment
    }

    /// The node the step reached.
    #[must_use]
    pub const fn value(&self) -> &'a Value {
        self.value
    }

    /// The 1-based occurrence of the node among the siblings the segment
    /// selects under the same parent instance, the number a position on the
    /// segment names in [`Navigator::instances`].
    #[must_use]
    pub const fn occurrence(&self) -> usize {
        self.occurrence
    }

    /// The 0-based index of the node in its container attribute, `None` for a
    /// single-valued attribute.
    #[must_use]
    pub const fn index(&self) -> Option<usize> {
        self.index
    }
}

/// One instance of a template path in a composition: the node and the steps
/// that reached it from the root.
#[derive(Debug, Clone)]
pub struct Instance<'a> {
    root: &'a Value,
    steps: Vec<InstanceStep<'a>>,
}

impl<'a> Instance<'a> {
    /// The root of a composition as an instance with no steps.
    #[must_use]
    pub const fn root(composition: &'a Value) -> Self {
        Self {
            root: composition,
            steps: Vec::new(),
        }
    }

    /// The node this instance is.
    #[must_use]
    pub fn value(&self) -> &'a Value {
        self.steps.last().map_or(self.root, InstanceStep::value)
    }

    /// The steps from the root, in order.
    #[must_use]
    pub fn steps(&self) -> &[InstanceStep<'a>] {
        &self.steps
    }

    /// The instance `up` steps above this one, `None` past the root.
    #[must_use]
    pub fn ancestor(&self, up: usize) -> Option<Self> {
        let kept = self.steps.len().checked_sub(up)?;
        Some(Self {
            root: self.root,
            steps: self.steps.get(..kept)?.to_vec(),
        })
    }

    /// The template path this instance followed, with no positions.
    #[must_use]
    pub fn template_path(&self) -> RmPath {
        RmPath {
            absolute: true,
            segments: self.steps.iter().map(|s| s.segment.clone()).collect(),
        }
    }

    /// The path that names exactly this instance: each step into a container
    /// carries the 1-based position of the node in it, alone, and a step into
    /// a single-valued attribute carries no predicate (BASE
    /// `architecture_overview/master11-paths.adoc` §"Using Positional
    /// Parameters"), so the path navigates back to this node and no other.
    #[must_use]
    pub fn path(&self) -> RmPath {
        let segments = self
            .steps
            .iter()
            .map(|step| PathSegment {
                attribute: step.segment.attribute.clone(),
                predicate: Predicate {
                    position: step.index.and_then(|i| i.checked_add(1)),
                    ..Predicate::default()
                },
                descendant: false,
            })
            .collect();
        RmPath {
            absolute: true,
            segments,
        }
    }

    /// The JSON pointer of this instance in the composition (RFC 6901), the
    /// empty pointer for the root.
    #[must_use]
    pub fn pointer(&self) -> String {
        let mut pointer = String::new();
        for step in &self.steps {
            pointer.push('/');
            pointer.push_str(&step.segment.attribute.replace('~', "~0").replace('/', "~1"));
            if let Some(index) = step.index {
                pointer.push('/');
                pointer.push_str(&index.to_string());
            }
        }
        pointer
    }
}

impl<'t> Navigator<'t> {
    /// The navigator of `template`.
    #[must_use]
    pub fn new(template: &'t WebTemplate) -> Self {
        let mut tails: BTreeMap<String, Vec<Vec<PathSegment>>> = BTreeMap::new();
        let mut stack = vec![&template.tree];
        while let Some(node) = stack.pop() {
            register(node, &mut tails);
            stack.extend(node.children.iter().chain(&node.alternatives));
        }
        let fallbacks = tails
            .into_iter()
            .map(|(parent, paths)| {
                let table = name_fallback_from_segments(paths.iter().map(Vec::as_slice));
                (parent, table)
            })
            .collect();
        Self {
            fallbacks,
            references: template.archetype_refs.as_ref(),
        }
    }

    /// Every instance of the template path `path` in `composition`, in
    /// document order.
    ///
    /// A segment that carries a position selects that occurrence alone, as
    /// [`InstanceStep::occurrence`] counts it, and a segment without one
    /// selects every occurrence. NOTE: BASE `master11-paths.adoc` §"Using
    /// Positional Parameters" counts a container's elements, and no
    /// specification counts the instances of one template node, so that
    /// count is our own design.
    #[must_use]
    pub fn instances<'a>(&self, composition: &'a Value, path: &RmPath) -> Vec<Instance<'a>> {
        self.descend(&Instance::root(composition), &path.segments)
    }

    /// Every instance `relative` reaches from `from`, in document order, with
    /// the positions of [`Navigator::instances`].
    #[must_use]
    pub fn descend<'a>(&self, from: &Instance<'a>, relative: &[PathSegment]) -> Vec<Instance<'a>> {
        let mut current = vec![from.clone()];
        for segment in relative {
            current = current
                .iter()
                .flat_map(|instance| self.step(instance, segment))
                .collect();
        }
        current
    }

    /// The instances one segment reaches from `instance`.
    fn step<'a>(&self, instance: &Instance<'a>, segment: &PathSegment) -> Vec<Instance<'a>> {
        let mut identity = segment.clone();
        identity.predicate.position = None;
        let key = render(&instance.template_path().segments);
        let allow = self
            .fallbacks
            .get(&key)
            .is_some_and(|table| fallback_allowed(table, &identity));
        let children = children(instance.value(), &identity.attribute);
        let mut chosen = self.select(&children, &identity.predicate);
        if chosen.is_empty() && allow && identity.predicate.name_value.is_some() {
            let mut id_only = identity.predicate.clone();
            id_only.name_value = None;
            chosen = self.select(&children, &id_only);
        }
        chosen
            .into_iter()
            .enumerate()
            .map(|(at, found)| (at.saturating_add(1), found))
            .filter(|(occurrence, _)| segment.predicate.position.is_none_or(|p| p == *occurrence))
            .map(|(occurrence, (index, value))| {
                let mut steps = instance.steps.clone();
                steps.push(InstanceStep {
                    segment: identity.clone(),
                    value,
                    occurrence,
                    index,
                });
                Instance {
                    root: instance.root,
                    steps,
                }
            })
            .collect()
    }

    /// The children `predicate` selects, its node id matched exactly or
    /// through the OPT2 archetype references.
    fn select<'a>(
        &self,
        children: &[(Option<usize>, &'a Value)],
        predicate: &Predicate,
    ) -> Vec<(Option<usize>, &'a Value)> {
        let rest = Predicate {
            archetype_node_id: None,
            position: None,
            ..predicate.clone()
        };
        children
            .iter()
            .filter(|(_, child)| self.node_id_matches(predicate, child) && rest.matches(child))
            .copied()
            .collect()
    }

    /// Whether `child` carries the node id `predicate` names, or the complete
    /// identifier an OPT2 recorded for it.
    fn node_id_matches(&self, predicate: &Predicate, child: &Value) -> bool {
        let Some(wanted) = predicate.archetype_node_id.as_deref() else {
            return true;
        };
        let Some(carried) = child.get("archetype_node_id").and_then(Value::as_str) else {
            return false;
        };
        carried == wanted
            || self
                .references
                .and_then(|refs| refs.get(carried))
                .is_some_and(|interface| interface == wanted)
    }
}

/// The children an attribute holds, each with its container index.
fn children<'a>(node: &'a Value, attribute: &str) -> Vec<(Option<usize>, &'a Value)> {
    match node.get(attribute) {
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (Some(i), v))
            .collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(single) => vec![(None, single)],
    }
}

/// Files every tail of `node`'s `aqlPath` under the parent it hangs from.
fn register(node: &WebTemplateNode, tails: &mut BTreeMap<String, Vec<Vec<PathSegment>>>) {
    if node.aql_path.is_empty() {
        return;
    }
    let Ok(path) = node.aql_path.parse::<RmPath>() else {
        return;
    };
    for at in 0..path.segments.len() {
        let (head, tail) = path.segments.split_at(at);
        tails.entry(render(head)).or_default().push(tail.to_vec());
    }
}

/// The key of a parent: its template segments rendered as an absolute path.
fn render(segments: &[PathSegment]) -> String {
    RmPath {
        absolute: true,
        segments: segments.to_vec(),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn template(paths: &[&str], refs: Option<BTreeMap<String, String>>) -> WebTemplate {
        let mut tree = WebTemplateNode::new("COMPOSITION".to_owned(), String::new());
        tree.children = paths
            .iter()
            .map(|p| WebTemplateNode::new("EVALUATION".to_owned(), (*p).to_owned()))
            .collect();
        WebTemplate {
            template_id: "t".to_owned(),
            sem_ver: None,
            version: "2.3".to_owned(),
            default_language: "en".to_owned(),
            languages: Vec::new(),
            tree,
            other_details: indexmap::IndexMap::new(),
            archetype_refs: refs,
        }
    }

    fn path(text: &str) -> RmPath {
        text.parse().expect("a well-formed path")
    }

    #[test]
    fn a_renamed_instance_is_reached_where_the_name_is_redundant() {
        let wt = template(
            &["/content[openEHR-EHR-EVALUATION.a.v1,'Template Name']"],
            None,
        );
        let composition = json!({"content": [
            {"archetype_node_id": "openEHR-EHR-EVALUATION.a.v1", "name": {"value": "Runtime"}}
        ]});
        let found =
            Navigator::new(&wt).instances(&composition, &path(&wt.tree.children[0].aql_path));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pointer(), "/content/0");
    }

    #[test]
    fn a_discriminating_name_is_never_dropped() {
        let wt = template(
            &[
                "/content[openEHR-EHR-EVALUATION.a.v1,'First']",
                "/content[openEHR-EHR-EVALUATION.a.v1,'Second']",
            ],
            None,
        );
        let composition = json!({"content": [
            {"archetype_node_id": "openEHR-EHR-EVALUATION.a.v1", "name": {"value": "Neither"}}
        ]});
        let navigator = Navigator::new(&wt);
        assert!(
            navigator
                .instances(&composition, &path(&wt.tree.children[0].aql_path))
                .is_empty()
        );
    }

    #[test]
    fn an_opt2_root_is_reached_through_its_complete_identifier() {
        let refs = BTreeMap::from([(
            "org.openehr::openEHR-EHR-EVALUATION.a.v1.2.0".to_owned(),
            "openEHR-EHR-EVALUATION.a.v1".to_owned(),
        )]);
        let wt = template(&["/content[openEHR-EHR-EVALUATION.a.v1]"], Some(refs));
        let composition = json!({"content": [
            {"archetype_node_id": "org.openehr::openEHR-EHR-EVALUATION.a.v1.2.0", "name": {"value": "x"}},
            {"archetype_node_id": "org.other::openEHR-EHR-EVALUATION.a.v1.2.0", "name": {"value": "y"}}
        ]});
        let found = Navigator::new(&wt)
            .instances(&composition, &path("/content[openEHR-EHR-EVALUATION.a.v1]"));
        assert_eq!(
            found.len(),
            1,
            "an unrecorded namespace is another artefact"
        );
        assert_eq!(found[0].value()["name"]["value"], "x");
    }

    #[test]
    fn a_position_selects_one_occurrence_and_the_instance_names_it() {
        let wt = template(
            &["/content[openEHR-EHR-EVALUATION.a.v1]/data[at0001]/items[at0002]"],
            None,
        );
        let composition = json!({"content": [
            {"archetype_node_id": "openEHR-EHR-OBSERVATION.other.v1"},
            {"archetype_node_id": "openEHR-EHR-EVALUATION.a.v1", "data": {"archetype_node_id": "at0001", "items": [
                {"archetype_node_id": "at0002", "tag": 1},
                {"archetype_node_id": "at0003"},
                {"archetype_node_id": "at0002", "tag": 2}
            ]}}
        ]});
        let navigator = Navigator::new(&wt);
        let all = navigator.instances(&composition, &path(&wt.tree.children[0].aql_path));
        assert_eq!(all.len(), 2);
        let second = navigator.instances(
            &composition,
            &path("/content[openEHR-EHR-EVALUATION.a.v1]/data[at0001]/items[at0002 and 2]"),
        );
        assert_eq!(second.len(), 1);
        let step = second[0].steps().last().expect("a step");
        assert_eq!((step.occurrence(), step.index()), (2, Some(2)));
        assert_eq!(second[0].value()["tag"], 2);
        assert_eq!(second[0].path().to_string(), "/content[2]/data/items[3]");
        assert_eq!(second[0].pointer(), "/content/1/data/items/2");
        let parent = second[0].ancestor(2).expect("the entry");
        assert_eq!(parent.pointer(), "/content/1");
        let again = navigator.descend(&parent, &path("data[at0001]/items[at0002]").segments);
        assert_eq!(again.len(), 2);
        assert_eq!(again[1].template_path(), second[0].template_path());
    }

    #[test]
    fn relative_compares_paths_step_by_step() {
        assert_eq!(
            relative(&path("/data"), &path("/data/events[at0002]")).map(|p| p.to_string()),
            Some("events[at0002]".to_owned())
        );
        assert_eq!(relative(&path("/data"), &path("/data_x/events")), None);
        assert_eq!(
            relative(
                &path("/items[at0001]"),
                &path("/items[at0001,'Named']/value")
            ),
            None
        );
    }
}
