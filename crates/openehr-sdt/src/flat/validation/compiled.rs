// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A node's constraint paths compiled once per template: each step's name
//! fallback, each sibling group's selector, and the container prefixes the
//! constraints share, so the walk reaches a container once per instance node
//! and makes no lookup by name. No openEHR spec governs it: our own design.

use openehr_rm::v1_2::paths::{PathSegment, Predicate};

use super::SiblingNameIndex;
use crate::flat::instance::{Instance, matches};
use crate::flat::rmpath::{NameFallback, fallback_allowed};

/// The position and predicate of one path step, with `archetype_node_id`
/// split out so the id a [`Members`] list read once answers it.
#[derive(Debug, Clone)]
struct Filter {
    position: Option<usize>,
    id: Option<String>,
    /// The conjuncts besides the id and the position, when there are any.
    rest: Option<Predicate>,
}

impl Filter {
    fn new(predicate: &Predicate) -> Self {
        let mut rest = predicate.clone();
        rest.position = None;
        let id = rest.archetype_node_id.take();
        Self {
            position: predicate.position,
            id,
            rest: (!rest.is_empty()).then_some(rest),
        }
    }

    fn admits<'i, I: Instance<'i>>(&self, node: I, id: Option<&str>) -> bool {
        if let Some(want) = &self.id
            && id != Some(want.as_str())
        {
            return false;
        }
        self.rest.as_ref().is_none_or(|rest| matches(rest, node))
    }

    /// Appends what [`crate::flat::instance::select_children`] selects with
    /// this step from the attribute `members` lists.
    fn pick<'i, I: Instance<'i>>(&self, members: &Members<'i, I>, out: &mut Vec<I>) {
        let admitted = |(node, id): &&(I, Option<&'i str>)| self.admits(*node, *id);
        if let (None, Some(want), false) = (self.position, &self.id, members.by_id.is_empty()) {
            let want = Some(want.as_str());
            let start = members.by_id.partition_point(|&i| members.id(i) < want);
            let same = members.by_id.iter().skip(start);
            let picked = same.take_while(|&&i| members.id(i) == want);
            out.extend(
                picked
                    .filter_map(|&i| members.items.get(i))
                    .filter(admitted)
                    .map(|(node, _)| *node),
            );
            return;
        }
        match self.position {
            Some(position) if members.many => out.extend(
                position
                    .checked_sub(1)
                    .and_then(|i| members.items.get(i))
                    .filter(admitted)
                    .map(|(node, _)| *node),
            ),
            Some(position) if position != 1 => {}
            _ => out.extend(members.items.iter().filter(admitted).map(|(node, _)| *node)),
        }
    }
}

/// The values one attribute of a container holds, each with its
/// `archetype_node_id`.
#[derive(Debug)]
pub(crate) struct Members<'i, I> {
    /// Whether the attribute is a container.
    many: bool,
    items: Vec<(I, Option<&'i str>)>,
    /// The indices into `items` ordered by id, then by position; empty for a
    /// list short enough to scan.
    by_id: Vec<usize>,
}

/// The list length from which [`Members`] orders its ids for a lookup.
const INDEXED_FROM: usize = 8;

impl<'i, I: Instance<'i>> Members<'i, I> {
    /// Reads `attribute` of `container` once.
    pub(crate) fn of(container: I, attribute: &str) -> Self {
        let read = |node: I| (node, node.str_at("archetype_node_id"));
        let mut items = Vec::new();
        let many = container.each_member(attribute, &mut |node| items.push(read(node)));
        if !many {
            items.extend(container.attr(attribute).map(read));
        }
        let mut by_id = Vec::new();
        if items.len() >= INDEXED_FROM {
            by_id.extend(0..items.len());
            by_id.sort_by_key(|&i| items.get(i).and_then(|(_, id)| *id));
        }
        Self { many, items, by_id }
    }

    fn id(&self, index: usize) -> Option<&'i str> {
        self.items.get(index).and_then(|(_, id)| *id)
    }
}

/// One compiled path step or sibling identity.
#[derive(Debug, Clone)]
pub(crate) struct Selector {
    attribute: String,
    primary: Filter,
    /// The id-only retry when the primary filter selects nothing and the
    /// step's name is redundant ([`fallback_allowed`]).
    fallback: Option<Filter>,
    /// The names a same-id sibling claims, which this one leaves out.
    excluded: Vec<String>,
}

impl Selector {
    /// A path step under the name fallback `table`, as
    /// [`crate::flat::rmpath::navigate_matched`] takes it.
    fn step(seg: &PathSegment, table: &NameFallback) -> Self {
        let allow = fallback_allowed(table, seg);
        Self::with_fallback(seg, allow, Vec::new())
    }

    /// A sibling identity, same-`archetype_node_id` siblings routed by
    /// `name` under `names` (RM common `master03-archetyped_package.adoc`
    /// §"The `LOCATABLE` class"; BASE `master11-paths.adoc` §"Using a
    /// Name-based Predicate").
    ///
    /// A name-qualified sibling of a name-differentiated identity matches
    /// strictly, the unqualified one takes the residue, and an identity
    /// absent from `names` matches with the id-only fallback on.
    pub(crate) fn identity(seg: &PathSegment, names: &SiblingNameIndex) -> Self {
        let claimed = seg
            .predicate
            .archetype_node_id
            .as_ref()
            .and_then(|id| names.get(&(seg.attribute.clone(), id.clone())));
        match (seg.predicate.name_value.as_ref(), claimed) {
            (Some(_), Some(_)) => Self::with_fallback(seg, false, Vec::new()),
            (None, Some(claimed)) => Self::with_fallback(seg, false, claimed.clone()),
            (_, None) => Self::with_fallback(seg, true, Vec::new()),
        }
    }

    fn with_fallback(seg: &PathSegment, allow: bool, excluded: Vec<String>) -> Self {
        let fallback = (allow && seg.predicate.name_value.is_some()).then(|| {
            let mut id_only = seg.predicate.clone();
            id_only.name_value = None;
            Filter::new(&id_only)
        });
        Self {
            attribute: seg.attribute.clone(),
            primary: Filter::new(&seg.predicate),
            fallback,
            excluded,
        }
    }

    /// The attribute the selector reads.
    pub(crate) fn attribute(&self) -> &str {
        &self.attribute
    }

    /// Whether the selector names an `archetype_node_id`.
    pub(crate) fn has_id(&self) -> bool {
        self.primary.id.is_some()
    }

    /// The values the selector takes from `members`, in document order.
    pub(crate) fn select<'i, I: Instance<'i>>(&self, members: &Members<'i, I>) -> Vec<I> {
        let mut out = Vec::new();
        self.primary.pick(members, &mut out);
        if !self.excluded.is_empty() {
            out.retain(|node| {
                node.attr("name")
                    .and_then(|n| n.str_at("value"))
                    .is_none_or(|name| !self.excluded.iter().any(|e| e == name))
            });
        }
        if out.is_empty()
            && let Some(fallback) = &self.fallback
        {
            fallback.pick(members, &mut out);
        }
        out
    }
}

/// Compiles `segs` as steps under the name fallback `table`.
pub(crate) fn steps(segs: &[PathSegment], table: &NameFallback) -> Vec<Selector> {
    segs.iter().map(|seg| Selector::step(seg, table)).collect()
}

/// Appends every value `steps` reach from `node`, in document order.
pub(crate) fn reach<'i, I: Instance<'i>>(node: I, steps: &[Selector], out: &mut Vec<I>) {
    let Some((step, rest)) = steps.split_first() else {
        out.push(node);
        return;
    };
    for child in step.select(&Members::of(node, &step.attribute)) {
        reach(child, rest, out);
    }
}

/// A sibling group compiled: its container prefix, the member source its
/// identity reads, and its selectors.
#[derive(Debug, Clone)]
pub(crate) struct GroupPlan {
    /// The index of the identity segment in the group's segments.
    pub(crate) identity: usize,
    /// The member source of the identity ([`Compiled::sources`]).
    pub(crate) source: usize,
    /// The identity as the template writes it.
    pub(crate) raw: Selector,
    /// The identity with its predicate dropped, for a node only its position
    /// identifies (only `LOCATABLE` carries `archetype_node_id`).
    pub(crate) structural: Selector,
    /// The steps past the identity.
    pub(crate) trailing: Vec<Selector>,
    /// The RM type of the group's first member when compiled, and whether it
    /// is a `LOCATABLE`.
    first_type: (String, bool),
}

impl GroupPlan {
    /// Whether `rm_type`, the group's first member's type, is a `LOCATABLE`.
    pub(crate) fn is_locatable(&self, rm_type: &str) -> bool {
        if self.first_type.0 == rm_type {
            return self.first_type.1;
        }
        super::is_locatable(rm_type)
    }
}

/// A node's constraint paths compiled: index-aligned with the plan's child
/// groups, slot groups and the node's constraint vectors.
#[derive(Debug, Clone, Default)]
pub(crate) struct Compiled {
    /// The distinct container prefixes.
    pub(crate) prefixes: Vec<Vec<Selector>>,
    /// The distinct `(prefix, attribute)` pairs the group identities read.
    pub(crate) sources: Vec<(usize, String)>,
    /// Per child group, `None` for one the walk skips.
    pub(crate) groups: Vec<Option<GroupPlan>>,
    /// Per slot group, its prefix and identity selector.
    pub(crate) slots: Vec<Option<(usize, Selector)>>,
    /// Per existence constraint, its prefix.
    pub(crate) existence: Vec<Option<usize>>,
    /// Per cardinality constraint, its prefix.
    pub(crate) cardinalities: Vec<Option<usize>>,
    /// Per closed attribute, its prefix.
    pub(crate) closed: Vec<Option<usize>>,
}

/// Builds [`Compiled`], deduplicating the prefixes and member sources.
pub(crate) struct Compiler<'a> {
    table: &'a NameFallback,
    keys: Vec<&'a [PathSegment]>,
    out: Compiled,
}

impl<'a> Compiler<'a> {
    pub(crate) fn new(table: &'a NameFallback) -> Self {
        Self {
            table,
            keys: Vec::new(),
            out: Compiled::default(),
        }
    }

    /// The prefix index of `segs`.
    fn prefix(&mut self, segs: &'a [PathSegment]) -> usize {
        if let Some(i) = self.keys.iter().position(|k| *k == segs) {
            return i;
        }
        self.keys.push(segs);
        self.out.prefixes.push(steps(segs, self.table));
        self.keys.len() - 1
    }

    /// The prefix of each constraint path: everything before its last
    /// segment.
    pub(crate) fn constraints(
        &mut self,
        paths: &'a [Option<Vec<PathSegment>>],
    ) -> Vec<Option<usize>> {
        paths
            .iter()
            .map(|segs| {
                let (_, intermediate) = segs.as_deref()?.split_last()?;
                Some(self.prefix(intermediate))
            })
            .collect()
    }

    /// Compiles one child group over `segs` under the sibling index `names`,
    /// its first member of RM type `first_type`.
    pub(crate) fn group(
        &mut self,
        segs: &'a [PathSegment],
        names: &SiblingNameIndex,
        first_type: &str,
    ) {
        let plan = (!segs.is_empty()).then(|| {
            let identity = segs
                .iter()
                .rposition(|s| !s.predicate.is_empty())
                .unwrap_or(segs.len() - 1);
            let (head, tail) = segs.split_at(identity);
            let (id_seg, trailing) = tail.split_first()?;
            let prefix = self.prefix(head);
            let key = (prefix, id_seg.attribute.clone());
            let source = self
                .out
                .sources
                .iter()
                .position(|s| *s == key)
                .unwrap_or_else(|| {
                    self.out.sources.push(key);
                    self.out.sources.len() - 1
                });
            let mut stripped = id_seg.clone();
            stripped.predicate = Predicate::default();
            Some(GroupPlan {
                identity,
                source,
                raw: Selector::identity(id_seg, names),
                structural: Selector::identity(&stripped, names),
                trailing: steps(trailing, self.table),
                first_type: (first_type.to_owned(), super::is_locatable(first_type)),
            })
        });
        self.out.groups.push(plan.flatten());
    }

    /// Compiles one slot group over `segs` under the sibling index `names`.
    pub(crate) fn slot(&mut self, segs: &'a [PathSegment], names: &SiblingNameIndex) {
        let plan = segs.split_last().map(|(last, intermediate)| {
            (self.prefix(intermediate), Selector::identity(last, names))
        });
        self.out.slots.push(plan);
    }

    pub(crate) fn finish(self) -> Compiled {
        self.out
    }
}

/// The containers and member lists one instance node's constraints read,
/// each computed on first use.
pub(crate) struct Frame<'i, I> {
    containers: Vec<Option<Vec<I>>>,
    members: Vec<Option<Vec<Members<'i, I>>>>,
}

impl<'i, I: Instance<'i>> Frame<'i, I> {
    pub(crate) fn new(compiled: &Compiled) -> Self {
        Self {
            containers: std::iter::repeat_with(|| None)
                .take(compiled.prefixes.len())
                .collect(),
            members: std::iter::repeat_with(|| None)
                .take(compiled.sources.len())
                .collect(),
        }
    }

    /// Computes the containers of `prefix` below `node`.
    fn ensure_prefix(&mut self, compiled: &Compiled, node: I, prefix: usize) {
        let Some(slot) = self.containers.get_mut(prefix) else {
            return;
        };
        if slot.is_none() {
            let mut out = Vec::new();
            if let Some(steps) = compiled.prefixes.get(prefix) {
                reach(node, steps, &mut out);
            }
            *slot = Some(out);
        }
    }

    /// The containers of `prefix` below `node`.
    pub(crate) fn containers(&mut self, compiled: &Compiled, node: I, prefix: usize) -> &[I] {
        self.ensure_prefix(compiled, node, prefix);
        self.containers
            .get(prefix)
            .and_then(Option::as_deref)
            .unwrap_or_default()
    }

    /// The member lists of `source`, one per container of its prefix.
    pub(crate) fn members(
        &mut self,
        compiled: &Compiled,
        node: I,
        source: usize,
    ) -> &[Members<'i, I>] {
        let Some((prefix, attribute)) = compiled.sources.get(source) else {
            return &[];
        };
        if self.members.get(source).is_some_and(Option::is_none) {
            self.ensure_prefix(compiled, node, *prefix);
            let lists = self
                .containers
                .get(*prefix)
                .and_then(Option::as_deref)
                .unwrap_or_default()
                .iter()
                .map(|container| Members::of(*container, attribute))
                .collect();
            if let Some(slot) = self.members.get_mut(source) {
                *slot = Some(lists);
            }
        }
        self.members
            .get(source)
            .and_then(Option::as_deref)
            .unwrap_or_default()
    }
}
