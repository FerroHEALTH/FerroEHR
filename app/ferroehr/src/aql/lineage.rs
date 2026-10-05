// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype **specialisation-lineage** index the AQL planner folds into an
//! `archetype_node_id` predicate.
//!
//! AM `Identification` master07 §Supporting Archetype-based Querying fixes the
//! matching set as X, its previous minor or patch variants, its specialisation
//! parents and their previous variants, and adds that "for specialised
//! archetypes, the specialisation lineage can only be obtained from the
//! operational form of the archetype, found in the template used to create the
//! data". AM `Identification` master03 §Legacy ADL 1.4 Semantics removes the
//! lineage meaning of the `-` separator for AOM2-era identifiers ("the level of
//! specialisation can no longer be determined from the identifier"), so their
//! lineage is only what the stored artefacts declare.
//!
//! This module is the read-only side of that: a resolved child-to-parent graph
//! over the stored ADL2 and OPT2 family, inverted into parent-to-children, which
//! answers which stored archetypes descend from the one a query names. Loading
//! and caching it is the definition service's `lineage` module; the planner
//! consumes it through [`crate::aql::sql::SqlCtx`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use openehr_adl::hrid::parse_hrid;

/// The **interface identity** of an archetype: the three parts an
/// `archetype_node_id` is matched on — the qualified RM entity, the domain
/// concept, and the major version.
///
/// Minor/patch versions and the AOM2 namespace are deliberately *not* part of
/// the key: AM `Identification` master07 §Supporting Archetype-based Querying
/// admits "any previous minor or patch variant" into the same matching set,
/// while §Referencing keeps the major version a hard boundary. Both parts
/// compare case-folded (BASE `base_types` master05 §"Composite Identifiers and
/// Case"), matching the promoted `node.arch_entity` / `node.arch_concept`
/// columns, which are lowercased at write.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArchetypeKey {
    /// `qualified_rm_entity`, lowercased (e.g. `openehr-ehr-observation`).
    pub entity: String,
    /// `domain_concept`, lowercased (e.g. `lipid_panel`).
    pub concept: String,
    /// The major version (`.vN` / `.vN.m.p` → `N`).
    pub major: i32,
}

impl ArchetypeKey {
    /// The key for an already-decomposed identifier, case-folding both text
    /// parts.
    #[must_use]
    pub fn new(entity: &str, concept: &str, major: i32) -> Self {
        Self {
            entity: entity.to_ascii_lowercase(),
            concept: concept.to_ascii_lowercase(),
            major,
        }
    }

    /// The key for an archetype HRID in either era's form —
    /// `openEHR-EHR-OBSERVATION.lipid_panel.v1` (BASE `base_types` master05
    /// §Archetype Identifiers) or `[ns::]publisher-package-class.concept
    /// .vMAJOR.MINOR.PATCH` (AM `AOM2` master07.05 §Physical Archetype
    /// Identifier). `None` when the text is not an archetype HRID at all
    /// (an at/id-code, an arbitrary string).
    #[must_use]
    pub fn from_hrid(hrid: &str) -> Option<Self> {
        decompose_hrid(hrid).map(|q| q.key)
    }
}

/// An archetype identifier decomposed for matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueriedArchetype {
    /// The interface key the identifier resolves to.
    pub key: ArchetypeKey,
    /// True when the version is the ADL 1.4 major-only form (`.v1`) rather
    /// than the AOM2-era physical form (`.v1.0.0`).
    ///
    /// Only in the 1.4 form do the `-` segments of a `domain_concept` carry
    /// specialisation (AM `AOM2` master07.05 §Physical Archetype Identifier),
    /// so a concept-prefix match is meaningful there and nowhere else.
    pub legacy_form: bool,
}

/// Decompose an archetype HRID in either era's form.
///
/// This is the ONE reading of an archetype identifier in the query path: the
/// lineage index and the SQL predicate both go through it, so a form one of
/// them accepts cannot be a form the other silently declines.
///
/// `None` when the text is not an archetype HRID at all (an at/id-code, an
/// arbitrary string).
#[must_use]
pub fn decompose_hrid(hrid: &str) -> Option<QueriedArchetype> {
    let parsed = parse_hrid(hrid).ok()?;
    let major: i32 = parsed.release_version.split('.').next()?.parse().ok()?;
    let entity = format!(
        "{}-{}-{}",
        parsed.rm_publisher, parsed.rm_package, parsed.rm_class
    );
    Some(QueriedArchetype {
        key: ArchetypeKey::new(&entity, &parsed.concept_id, major),
        legacy_form: version_is_major_only(hrid),
    })
}

/// True when the identifier's own text carries a major-only version (`.v1`)
/// rather than the physical `.v1.0.0` form.
///
/// The distinction has to come from the source text: `parse_hrid` normalises
/// every version to `major.minor.patch`, so a parsed `.v1` and a parsed
/// `.v1.0.0` are indistinguishable afterwards — and it is exactly `.v1` that
/// licenses the ADL 1.4 concept-prefix match.
fn version_is_major_only(hrid: &str) -> bool {
    let Some(version) = hrid.rsplit_once(".v").map(|(_, v)| v) else {
        return false;
    };
    // A `-rc`/`-alpha`/`-beta` build suffix is not part of the version number.
    let numeric = version.split('-').next().unwrap_or_default();
    !numeric.is_empty() && !numeric.contains('.')
}

/// The stored specialisation graph: for every archetype identity, the stored
/// artefacts that declare it (transitively) as their specialisation parent.
///
/// Built from the `specialize` parent references of the stored ADL2/OPT2
/// family — the only lineage source AM `Identification` master07 §Supporting
/// Archetype-based Querying recognises for specialised archetypes. An empty
/// index (nothing stored, or nothing specialised) resolves every identifier to
/// itself, which is exactly the pre-lineage behaviour.
#[derive(Debug, Clone, Default)]
pub struct ArchetypeLineage {
    /// parent identity → the identities that directly specialise it.
    children: BTreeMap<ArchetypeKey, BTreeSet<ArchetypeKey>>,
}

impl ArchetypeLineage {
    /// Build the index from `(artefact HRID, declared parent HRID)` pairs — one
    /// pair per stored artefact that carries a `specialize` clause.
    ///
    /// A pair whose either side is not a readable HRID is skipped: every stored
    /// artefact was engine-validated at upload, so this is defensive only, and
    /// a single unreadable row must never cost the whole index.
    #[must_use]
    pub fn from_parent_edges<I, C, P>(edges: I) -> Self
    where
        I: IntoIterator<Item = (C, P)>,
        C: AsRef<str>,
        P: AsRef<str>,
    {
        let mut children: BTreeMap<ArchetypeKey, BTreeSet<ArchetypeKey>> = BTreeMap::new();
        for (child, parent) in edges {
            let (Some(child), Some(parent)) = (
                ArchetypeKey::from_hrid(child.as_ref()),
                ArchetypeKey::from_hrid(parent.as_ref()),
            ) else {
                continue;
            };
            // A self-edge would make every query its own descendant query for
            // no gain; drop it at build time so the walk stays minimal.
            if child != parent {
                children.entry(parent).or_default().insert(child);
            }
        }
        Self { children }
    }

    /// Whether the index carries no specialisation edge at all (no stored
    /// family, or none of the stored artefacts specialises another).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Every stored identity that descends from `root`, transitively —
    /// excluding `root` itself.
    ///
    /// The walk is breadth-first over the parent → children map with a visited
    /// set, so a cyclic `specialize` chain (which AOM2 forbids but a hostile
    /// upload could still attempt across two artefacts) terminates instead of
    /// looping.
    #[must_use]
    pub fn descendants(&self, root: &ArchetypeKey) -> BTreeSet<ArchetypeKey> {
        let mut found = BTreeSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(root.clone());
        while let Some(next) = queue.pop_front() {
            let Some(direct) = self.children.get(&next) else {
                continue;
            };
            for child in direct {
                if child != root && found.insert(child.clone()) {
                    queue.push_back(child.clone());
                }
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::{ArchetypeKey, ArchetypeLineage};

    fn key(concept: &str, major: i32) -> ArchetypeKey {
        ArchetypeKey::new("openEHR-EHR-OBSERVATION", concept, major)
    }

    /// Both identifier eras read into the same case-folded interface key: the
    /// BASE master05 major-only form and the AOM2 master07.05 physical form
    /// (namespace + minor/patch), which the key deliberately ignores.
    #[test]
    fn hrid_forms_share_one_interface_key() {
        let legacy = ArchetypeKey::from_hrid("openEHR-EHR-OBSERVATION.Lipid_Panel.v1");
        let physical = ArchetypeKey::from_hrid("openEHR-EHR-OBSERVATION.lipid_panel.v1.4.2");
        let namespaced =
            ArchetypeKey::from_hrid("org.openehr::openEHR-EHR-OBSERVATION.lipid_panel.v1.0.0");
        assert_eq!(legacy.as_ref(), Some(&key("lipid_panel", 1)));
        assert_eq!(physical.as_ref(), Some(&key("lipid_panel", 1)));
        assert_eq!(namespaced.as_ref(), Some(&key("lipid_panel", 1)));
        // A differing major is a different interface (master07 §Referencing).
        assert_ne!(
            ArchetypeKey::from_hrid("openEHR-EHR-OBSERVATION.lipid_panel.v2"),
            Some(key("lipid_panel", 1))
        );
        // An at-code is not an HRID.
        assert_eq!(ArchetypeKey::from_hrid("at0001"), None);
    }

    /// The lineage walk is transitive: a query naming the root reaches a
    /// grandchild, and each intermediate identity reaches only what is below
    /// it (AM master07 §Supporting Archetype-based Querying — "any of the
    /// specialisation parents of X").
    #[test]
    fn descendants_are_transitive() {
        let lineage = ArchetypeLineage::from_parent_edges([
            (
                "openEHR-EHR-OBSERVATION.hdl_result.v1.0.0",
                "openEHR-EHR-OBSERVATION.lipid_panel.v1",
            ),
            (
                "openEHR-EHR-OBSERVATION.hdl_direct.v1.0.0",
                "openEHR-EHR-OBSERVATION.hdl_result.v1",
            ),
            (
                "openEHR-EHR-OBSERVATION.unrelated.v1.0.0",
                "openEHR-EHR-OBSERVATION.other_root.v1",
            ),
        ]);
        assert!(!lineage.is_empty());
        assert_eq!(
            lineage.descendants(&key("lipid_panel", 1)),
            [key("hdl_result", 1), key("hdl_direct", 1)]
                .into_iter()
                .collect()
        );
        assert_eq!(
            lineage.descendants(&key("hdl_result", 1)),
            [key("hdl_direct", 1)].into_iter().collect()
        );
        assert!(lineage.descendants(&key("hdl_direct", 1)).is_empty());
        // An identity nothing descends from resolves to itself alone.
        assert!(lineage.descendants(&key("lipid_panel", 2)).is_empty());
    }

    /// A child may sit in a different major line than the parent it declares —
    /// the edge is the declared parent reference, not a version relation.
    #[test]
    fn a_child_may_declare_a_parent_in_another_major() {
        let lineage = ArchetypeLineage::from_parent_edges([(
            "openEHR-EHR-OBSERVATION.genetic_diagnosis.v2.0.0",
            "openEHR-EHR-OBSERVATION.diagnosis.v1",
        )]);
        assert_eq!(
            lineage.descendants(&key("diagnosis", 1)),
            [key("genetic_diagnosis", 2)].into_iter().collect()
        );
    }

    /// A cyclic `specialize` chain terminates (and a self-edge is dropped at
    /// build time) — a hostile or corrupt store must not hang the planner.
    #[test]
    fn cycles_terminate() {
        let lineage = ArchetypeLineage::from_parent_edges([
            (
                "openEHR-EHR-OBSERVATION.a.v1.0.0",
                "openEHR-EHR-OBSERVATION.b.v1",
            ),
            (
                "openEHR-EHR-OBSERVATION.b.v1.0.0",
                "openEHR-EHR-OBSERVATION.a.v1",
            ),
            (
                "openEHR-EHR-OBSERVATION.c.v1.0.0",
                "openEHR-EHR-OBSERVATION.c.v1",
            ),
        ]);
        assert_eq!(
            lineage.descendants(&key("a", 1)),
            [key("b", 1)].into_iter().collect()
        );
        assert!(lineage.descendants(&key("c", 1)).is_empty());
    }

    /// Unreadable rows are skipped, never fatal; an empty index resolves every
    /// identifier to itself.
    #[test]
    fn unreadable_edges_are_skipped() {
        let lineage = ArchetypeLineage::from_parent_edges([("not an hrid", "at0001")]);
        assert!(lineage.is_empty());
        assert!(lineage.descendants(&key("lipid_panel", 1)).is_empty());
    }
}
