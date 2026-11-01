// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype **specialisation-lineage** index the AQL planner folds into an
//! `archetype_node_id` predicate, and the one reading of a queried archetype
//! identifier that predicate and the index share.
//!
//! BASE Artefact Identification (Referencing §Supporting Archetype-based
//! Querying) fixes the matching set as X, its previous minor or patch variants,
//! its specialisation parents and their previous variants, and adds that "for
//! specialised archetypes, the specialisation lineage can only be obtained from
//! the operational form of the archetype, found in the template used to create
//! the data". BASE Artefact Identification (Source Artefact Identification
//! §Legacy ADL 1.4 Semantics) removes the lineage meaning of the `-` separator
//! for AOM2-era identifiers ("the level of specialisation can no longer be
//! determined from the identifier"), so their lineage is only what the stored
//! artefacts declare.
//!
//! This module is the read-only side of that: a resolved child-to-parent graph
//! over the stored ADL2 and OPT2 family, inverted into parent-to-children, which
//! answers which stored archetypes descend from the one a query names. Loading
//! and caching it is the definition service's `lineage` module; the planner
//! consumes it through [`crate::aql::sql::SqlCtx`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use openehr_adl::hrid::{parse_hrid, raw_id_version};

/// The **interface identity** of an archetype: the namespace, the qualified RM
/// entity, the domain concept and the major version.
///
/// The namespace is part of the key because BASE `artefact_identification`
/// master07 §Referencing makes it exact: "A namespace is never a wildcard - an
/// omitted namespace does not mean 'any namespace'". Minor and patch versions
/// are not part of the key: §Supporting Archetype-based Querying admits "any
/// previous minor or patch variant" of X into one matching set, which
/// [`ReferenceForm`] bounds. Every text part compares case-folded (BASE
/// `base_types` master05 §"Composite Identifiers and Case"), matching the
/// promoted `node.archetype`, `node.arch_entity` and `node.arch_concept`
/// columns, which are lowercased at write.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArchetypeKey {
    /// The namespace, lowercased; `None` for an identifier that carries none.
    pub namespace: Option<String>,
    /// `qualified_rm_entity`, lowercased (e.g. `openehr-ehr-observation`).
    pub entity: String,
    /// `domain_concept`, lowercased (e.g. `lipid_panel`).
    pub concept: String,
    /// The major version (`.vN` / `.vN.m.p` → `N`).
    pub major: i32,
}

impl ArchetypeKey {
    /// Builds the key for an already-decomposed identifier, case-folding every
    /// text part.
    #[must_use]
    pub fn new(namespace: Option<&str>, entity: &str, concept: &str, major: i32) -> Self {
        Self {
            namespace: namespace.map(str::to_ascii_lowercase),
            entity: entity.to_ascii_lowercase(),
            concept: concept.to_ascii_lowercase(),
            major,
        }
    }

    /// Reads the key of an archetype HRID in either era's form:
    /// `openEHR-EHR-OBSERVATION.lipid_panel.v1` (BASE `base_types` master05
    /// §Archetype Identifiers) or `[ns::]publisher-package-class.concept
    /// .vMAJOR.MINOR.PATCH` (AM `AOM2` master07.05 §Physical Archetype
    /// Identifier).
    ///
    /// `None` when the text is not an archetype HRID at all (an at/id-code, an
    /// arbitrary string).
    #[must_use]
    pub fn from_hrid(hrid: &str) -> Option<Self> {
        decompose_hrid(hrid).map(|q| q.key)
    }
}

/// The version precision of an archetype reference, BASE
/// `artefact_identification` master07 §Referencing: `ihrid_ref` names the
/// major version, `sihrid_ref` the major and minor, `phrid_ref` all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReferenceForm {
    /// `ihrid_ref`: the major version only (`.v1`).
    Interface,
    /// `sihrid_ref`: the major and minor versions (`.v1.2`).
    SpecificInterface {
        /// The minor version the reference names.
        minor: i64,
    },
    /// `phrid_ref`: the full version (`.v1.2.3`).
    Physical {
        /// The minor version the reference names.
        minor: i64,
        /// The patch version the reference names.
        patch: i64,
    },
}

impl ReferenceForm {
    /// Reads the numeric release version of an identifier (`1`, `1.2`,
    /// `1.2.3`) into its major version and its form.
    ///
    /// `None` when the text has no one to three numeric segments, or a segment
    /// does not fit the integer it is compared as.
    fn read(release: &str) -> Option<(i32, Self)> {
        let mut parts = release.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map(str::parse::<i64>);
        let patch = parts.next().map(str::parse::<i64>);
        if parts.next().is_some() {
            return None;
        }
        let form = match (minor, patch) {
            (None, _) => Self::Interface,
            (Some(minor), None) => Self::SpecificInterface { minor: minor.ok()? },
            (Some(minor), Some(patch)) => Self::Physical {
                minor: minor.ok()?,
                patch: patch.ok()?,
            },
        };
        Some((major, form))
    }

    /// Whether a query with this reference form selects data created with a
    /// release the parent reference `declared` names, within one major.
    ///
    /// A part `declared` leaves out names the latest release of the interface
    /// (BASE `artefact_identification` master07 §Referencing), so it is at or
    /// after any part the query names.
    #[must_use]
    pub fn admits_reference(self, declared: Self) -> bool {
        match (self, declared) {
            (Self::Interface, _) | (_, Self::Interface) => true,
            (
                Self::SpecificInterface { minor },
                Self::SpecificInterface { minor: d } | Self::Physical { minor: d, .. },
            )
            | (Self::Physical { minor, .. }, Self::SpecificInterface { minor: d }) => d >= minor,
            (
                Self::Physical { minor, patch },
                Self::Physical {
                    minor: dm,
                    patch: dp,
                },
            ) => (dm, dp) >= (minor, patch),
        }
    }
}

/// An archetype identifier decomposed for matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueriedArchetype {
    /// The interface key the identifier resolves to.
    pub key: ArchetypeKey,
    /// How much of the version the identifier names.
    pub form: ReferenceForm,
    /// True when the identifier is an ADL 1.4 id: no namespace and a
    /// major-only version (`.v1`).
    ///
    /// Only in the 1.4 form do the `-` segments of a `domain_concept` carry
    /// specialisation (AM `AOM2` master07.05 §Physical Archetype Identifier),
    /// so a concept-prefix match is meaningful there and nowhere else.
    pub legacy_form: bool,
}

/// Decomposes an archetype HRID in either era's form.
///
/// This is the ONE reading of an archetype identifier in the query path: the
/// lineage index and the SQL predicate both go through it, so a form one of
/// them accepts cannot be a form the other silently declines. The version form
/// comes from the identifier's own text, since `parse_hrid` normalises every
/// version to `major.minor.patch`; a version-status suffix (`-rc.1`) is not
/// part of the numeric release.
///
/// `None` when the text is not an archetype HRID at all (an at/id-code, an
/// arbitrary string).
#[must_use]
pub fn decompose_hrid(hrid: &str) -> Option<QueriedArchetype> {
    let parsed = parse_hrid(hrid).ok()?;
    let (major, form) = ReferenceForm::read(raw_id_version(hrid))?;
    let entity = format!(
        "{}-{}-{}",
        parsed.rm_publisher, parsed.rm_package, parsed.rm_class
    );
    let namespace = parsed.namespace.as_deref().filter(|ns| !ns.is_empty());
    Some(QueriedArchetype {
        key: ArchetypeKey::new(namespace, &entity, &parsed.concept_id, major),
        form,
        legacy_form: namespace.is_none() && form == ReferenceForm::Interface,
    })
}

/// A stored artefact's `(minor, patch)` release; `None` for a stored id
/// whose version is not the three-part physical form.
type Release = Option<(i64, i64)>;

/// The releases of one stored child identity, each with the forms of the
/// parent references the artefacts of that release declare.
type ChildReleases = BTreeMap<Release, BTreeSet<ReferenceForm>>;

/// The releases of a stored descendant a query admits, within the
/// descendant's own major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmittedReleases {
    /// Every release: no stored release of the descendant is refused, so data
    /// of any release of it (a legacy major-only id included) is selected.
    Every,
    /// Only these `(minor, patch)` releases.
    Only(BTreeSet<(i64, i64)>),
}

/// The stored specialisation graph: for every archetype identity, the stored
/// artefacts that declare it (transitively) as their specialisation parent,
/// release by release.
///
/// Built from the `specialize` parent references of the stored ADL2/OPT2
/// family, the only lineage source BASE Artefact Identification (Referencing
/// §Supporting Archetype-based Querying) recognises for specialised
/// archetypes. An empty index (nothing stored, or nothing specialised)
/// resolves every identifier to itself.
#[derive(Debug, Clone, Default)]
pub struct ArchetypeLineage {
    /// parent identity → the identities that directly specialise it → their
    /// stored releases with the parent references each declares.
    children: BTreeMap<ArchetypeKey, BTreeMap<ArchetypeKey, ChildReleases>>,
    /// Every stored release of each child identity, across all its parents.
    releases: BTreeMap<ArchetypeKey, BTreeSet<Release>>,
}

impl ArchetypeLineage {
    /// Builds the index from `(artefact HRID, declared parent HRID)` pairs, one
    /// pair per stored artefact that carries a `specialize` clause.
    ///
    /// A parent reference without a namespace names the child's own namespace
    /// (BASE `artefact_identification` master07 §Referencing, a reference
    /// within a source artefact). A pair whose either side is not a readable
    /// HRID is skipped: every stored artefact was engine-validated at upload,
    /// so this is defensive only, and a single unreadable row must never cost
    /// the whole index.
    #[must_use]
    pub fn from_parent_edges<I, C, P>(edges: I) -> Self
    where
        I: IntoIterator<Item = (C, P)>,
        C: AsRef<str>,
        P: AsRef<str>,
    {
        let mut index = Self::default();
        for (child, parent) in edges {
            let (Some(child), Some(parent)) = (
                decompose_hrid(child.as_ref()),
                decompose_hrid(parent.as_ref()),
            ) else {
                continue;
            };
            let mut parent_key = parent.key;
            if parent_key.namespace.is_none() {
                parent_key.namespace.clone_from(&child.key.namespace);
            }
            // A self-edge would make every query its own descendant query for
            // no gain; drop it at build time so the walk stays minimal.
            if child.key == parent_key {
                continue;
            }
            let release = match child.form {
                ReferenceForm::Physical { minor, patch } => Some((minor, patch)),
                _ => None,
            };
            index
                .releases
                .entry(child.key.clone())
                .or_default()
                .insert(release);
            index
                .children
                .entry(parent_key)
                .or_default()
                .entry(child.key)
                .or_default()
                .entry(release)
                .or_default()
                .insert(parent.form);
        }
        index
    }

    /// Whether the index carries no specialisation edge at all (no stored
    /// family, or none of the stored artefacts specialises another).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Every stored identity that descends from `root`, transitively,
    /// excluding `root` itself, for a query naming `root` in `form`, with the
    /// releases of each the query admits.
    ///
    /// Data of a child release may be queried with "any previous minor or
    /// patch variant of any of the specialisation parents" of it (BASE
    /// Artefact Identification, Referencing §Supporting Archetype-based
    /// Querying), so a child release joins when the parent reference it
    /// declares names a release the query admits. For a direct child of `root`
    /// that is [`ReferenceForm::admits_reference`]; below it, the declared
    /// reference resolves to the latest stored parent release it matches,
    /// which must be an admitted one.
    ///
    /// The walk is breadth-first and re-visits an identity only when its
    /// admitted set grows, so a cyclic `specialize` chain (which AOM2 forbids
    /// but a hostile upload could still attempt across two artefacts)
    /// terminates instead of looping.
    #[must_use]
    pub fn descendants(
        &self,
        root: &ArchetypeKey,
        form: ReferenceForm,
    ) -> BTreeMap<ArchetypeKey, AdmittedReleases> {
        let mut admitted: BTreeMap<ArchetypeKey, BTreeSet<Release>> = BTreeMap::new();
        let mut queue = VecDeque::from([root.clone()]);
        while let Some(parent) = queue.pop_front() {
            let Some(direct) = self.children.get(&parent) else {
                continue;
            };
            for (child, releases) in direct {
                if child == root {
                    continue;
                }
                let accepted: Vec<Release> = releases
                    .iter()
                    .filter(|(_, declared)| {
                        declared.iter().any(|d| {
                            if &parent == root {
                                form.admits_reference(*d)
                            } else {
                                self.resolves_into(&parent, *d, admitted.get(&parent))
                            }
                        })
                    })
                    .map(|(release, _)| *release)
                    .collect();
                if accepted.is_empty() {
                    continue;
                }
                let entry = admitted.entry(child.clone()).or_default();
                let before = entry.len();
                entry.extend(accepted);
                if entry.len() > before {
                    queue.push_back(child.clone());
                }
            }
        }
        admitted
            .into_iter()
            .map(|(key, set)| {
                let every = set.contains(&None) || self.releases.get(&key) == Some(&set);
                let releases = if every {
                    AdmittedReleases::Every
                } else {
                    AdmittedReleases::Only(set.into_iter().flatten().collect())
                };
                (key, releases)
            })
            .collect()
    }

    /// Whether the parent reference `declared` resolves to an admitted stored
    /// release of `parent`: an `ihrid_ref` to the latest release, an
    /// `sihrid_ref` to the latest release of its minor, a `phrid_ref` to
    /// itself (BASE `artefact_identification` master07 §Referencing).
    fn resolves_into(
        &self,
        parent: &ArchetypeKey,
        declared: ReferenceForm,
        admitted: Option<&BTreeSet<Release>>,
    ) -> bool {
        let (Some(stored), Some(admitted)) = (self.releases.get(parent), admitted) else {
            return false;
        };
        let mut stored = stored.iter().flatten().copied();
        let resolved = match declared {
            ReferenceForm::Interface => stored.max(),
            ReferenceForm::SpecificInterface { minor } => stored.filter(|r| r.0 == minor).max(),
            ReferenceForm::Physical { minor, patch } => stored.find(|r| *r == (minor, patch)),
        };
        resolved.is_some_and(|r| admitted.contains(&Some(r)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{AdmittedReleases, ArchetypeKey, ArchetypeLineage, ReferenceForm, decompose_hrid};

    const ANY: ReferenceForm = ReferenceForm::Interface;

    fn key(concept: &str, major: i32) -> ArchetypeKey {
        ArchetypeKey::new(None, "openEHR-EHR-OBSERVATION", concept, major)
    }

    /// The descendants of `root`, each of which the query admits whole.
    fn every(
        lineage: &ArchetypeLineage,
        root: &ArchetypeKey,
        form: ReferenceForm,
    ) -> BTreeSet<ArchetypeKey> {
        let found = lineage.descendants(root, form);
        for (key, releases) in &found {
            assert_eq!(releases, &AdmittedReleases::Every, "{key:?}");
        }
        found.into_keys().collect()
    }

    fn ns_key(namespace: &str, concept: &str, major: i32) -> ArchetypeKey {
        ArchetypeKey::new(Some(namespace), "openEHR-EHR-OBSERVATION", concept, major)
    }

    /// Both identifier eras read into a case-folded interface key that drops
    /// minor and patch and keeps the namespace (BASE `artefact_identification`
    /// master07 §Referencing: a namespace is never a wildcard).
    #[test]
    fn hrid_forms_read_into_one_interface_key_per_namespace() {
        let legacy = ArchetypeKey::from_hrid("openEHR-EHR-OBSERVATION.Lipid_Panel.v1");
        let physical = ArchetypeKey::from_hrid("openEHR-EHR-OBSERVATION.lipid_panel.v1.4.2");
        let namespaced =
            ArchetypeKey::from_hrid("org.openEHR::openEHR-EHR-OBSERVATION.lipid_panel.v1.0.0");
        assert_eq!(legacy.as_ref(), Some(&key("lipid_panel", 1)));
        assert_eq!(physical.as_ref(), Some(&key("lipid_panel", 1)));
        assert_eq!(
            namespaced.as_ref(),
            Some(&ns_key("org.openehr", "lipid_panel", 1))
        );
        assert_ne!(namespaced, physical, "a namespace is part of the identity");
        // A differing major is a different interface (master07 §Referencing).
        assert_ne!(
            ArchetypeKey::from_hrid("openEHR-EHR-OBSERVATION.lipid_panel.v2"),
            Some(key("lipid_panel", 1))
        );
        // An at-code is not an HRID.
        assert_eq!(ArchetypeKey::from_hrid("at0001"), None);
    }

    /// The version form comes from the identifier's own text: one, two or three
    /// numeric segments, with a version-status suffix left out.
    #[test]
    fn the_reference_form_follows_the_version_segments() {
        let form = |hrid: &str| decompose_hrid(hrid).map(|q| (q.form, q.legacy_form));
        assert_eq!(
            form("openEHR-EHR-OBSERVATION.x.v1"),
            Some((ReferenceForm::Interface, true))
        );
        assert_eq!(
            form("org.openehr::openEHR-EHR-OBSERVATION.x.v1"),
            Some((ReferenceForm::Interface, false)),
            "a namespaced id is no ADL 1.4 id"
        );
        assert_eq!(
            form("openEHR-EHR-OBSERVATION.x.v1.2"),
            Some((ReferenceForm::SpecificInterface { minor: 2 }, false))
        );
        assert_eq!(
            form("openEHR-EHR-OBSERVATION.x.v1.2.3"),
            Some((ReferenceForm::Physical { minor: 2, patch: 3 }, false))
        );
        assert_eq!(
            form("openEHR-EHR-OBSERVATION.x.v1.2.3-rc.4"),
            Some((ReferenceForm::Physical { minor: 2, patch: 3 }, false))
        );
    }

    /// A query's form admits a declared parent reference at or after it, a
    /// part the declaration leaves out naming the latest release.
    #[test]
    fn a_query_admits_the_parent_releases_at_or_after_it() {
        let interface = ReferenceForm::Interface;
        let specific = |minor| ReferenceForm::SpecificInterface { minor };
        let physical = |minor, patch| ReferenceForm::Physical { minor, patch };
        for declared in [interface, specific(0), physical(0, 0), physical(9, 9)] {
            assert!(interface.admits_reference(declared), "{declared:?}");
            assert!(declared.admits_reference(interface), "{declared:?}");
        }
        assert!(specific(2).admits_reference(specific(2)));
        assert!(specific(2).admits_reference(specific(3)));
        assert!(!specific(2).admits_reference(specific(1)));
        assert!(specific(2).admits_reference(physical(2, 0)));
        assert!(!specific(2).admits_reference(physical(1, 9)));
        assert!(physical(2, 3).admits_reference(physical(2, 3)));
        assert!(physical(2, 3).admits_reference(physical(2, 4)));
        assert!(physical(2, 3).admits_reference(physical(3, 0)));
        assert!(!physical(2, 3).admits_reference(physical(2, 2)));
        assert!(!physical(2, 3).admits_reference(physical(1, 9)));
        assert!(physical(2, 3).admits_reference(specific(2)));
        assert!(!physical(2, 3).admits_reference(specific(1)));
    }

    /// The lineage walk is transitive: a query naming the root reaches a
    /// grandchild, and each intermediate identity reaches only what is below
    /// it (BASE Artefact Identification, Referencing §Supporting
    /// Archetype-based Querying: "any of the specialisation parents of X").
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
            every(&lineage, &key("lipid_panel", 1), ANY),
            [key("hdl_result", 1), key("hdl_direct", 1)]
                .into_iter()
                .collect()
        );
        assert_eq!(
            every(&lineage, &key("hdl_result", 1), ANY),
            [key("hdl_direct", 1)].into_iter().collect()
        );
        assert!(every(&lineage, &key("hdl_direct", 1), ANY).is_empty());
        // An identity nothing descends from resolves to itself alone.
        assert!(every(&lineage, &key("lipid_panel", 2), ANY).is_empty());
    }

    /// A specialised archetype is reached from its parent only by a query at
    /// or before the parent release it declares, and only in its namespace; a
    /// parent reference without a namespace names the child's own.
    #[test]
    fn a_child_is_reached_by_its_parent_version_and_namespace() {
        let lineage = ArchetypeLineage::from_parent_edges([(
            "org.a::openEHR-EHR-OBSERVATION.hdl_result.v1.0.0",
            "openEHR-EHR-OBSERVATION.lipid_panel.v1.2.0",
        )]);
        let parent = ns_key("org.a", "lipid_panel", 1);
        let child: BTreeSet<_> = [ns_key("org.a", "hdl_result", 1)].into_iter().collect();
        for form in [
            ReferenceForm::Interface,
            ReferenceForm::SpecificInterface { minor: 1 },
            ReferenceForm::SpecificInterface { minor: 2 },
            ReferenceForm::Physical { minor: 2, patch: 0 },
        ] {
            assert_eq!(every(&lineage, &parent, form), child, "{form:?}");
        }
        for form in [
            ReferenceForm::SpecificInterface { minor: 3 },
            ReferenceForm::Physical { minor: 2, patch: 1 },
        ] {
            assert!(
                every(&lineage, &parent, form).is_empty(),
                "a later parent release than the child declares: {form:?}"
            );
        }
        assert!(
            every(&lineage, &key("lipid_panel", 1), ANY).is_empty(),
            "the parent without a namespace is another archetype"
        );
        assert!(
            every(&lineage, &ns_key("org.b", "lipid_panel", 1), ANY).is_empty(),
            "the parent in another namespace is another archetype"
        );
    }

    /// A child may sit in a different major line than the parent it declares:
    /// the edge is the declared parent reference, not a version relation.
    #[test]
    fn a_child_may_declare_a_parent_in_another_major() {
        let lineage = ArchetypeLineage::from_parent_edges([(
            "openEHR-EHR-OBSERVATION.genetic_diagnosis.v2.0.0",
            "openEHR-EHR-OBSERVATION.diagnosis.v1",
        )]);
        assert_eq!(
            every(&lineage, &key("diagnosis", 1), ANY),
            [key("genetic_diagnosis", 2)].into_iter().collect()
        );
    }

    /// A cyclic `specialize` chain terminates (and a self-edge is dropped at
    /// build time): a hostile or corrupt store must not hang the planner.
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
            every(&lineage, &key("a", 1), ANY),
            [key("b", 1)].into_iter().collect()
        );
        assert!(every(&lineage, &key("c", 1), ANY).is_empty());
    }

    /// Unreadable rows are skipped, never fatal; an empty index resolves every
    /// identifier to itself.
    #[test]
    fn unreadable_edges_are_skipped() {
        let lineage = ArchetypeLineage::from_parent_edges([("not an hrid", "at0001")]);
        assert!(lineage.is_empty());
        assert!(every(&lineage, &key("lipid_panel", 1), ANY).is_empty());
    }

    /// Two releases of one child declaring different parent releases: a
    /// query admits only the child release whose declared parent release is
    /// at or after its own, and every release once both are (AM
    /// `Identification` master07 §Supporting Archetype-based Querying).
    #[test]
    fn a_query_admits_only_the_child_releases_built_on_a_later_parent() {
        let lineage = ArchetypeLineage::from_parent_edges([
            (
                "openEHR-EHR-OBSERVATION.child.v1.0.0",
                "openEHR-EHR-OBSERVATION.parent.v1.1",
            ),
            (
                "openEHR-EHR-OBSERVATION.child.v1.1.0",
                "openEHR-EHR-OBSERVATION.parent.v1.3",
            ),
        ]);
        let parent = key("parent", 1);
        let only = |releases: &[(i64, i64)]| {
            [(
                key("child", 1),
                AdmittedReleases::Only(releases.iter().copied().collect()),
            )]
            .into_iter()
            .collect()
        };
        assert_eq!(
            lineage.descendants(&parent, ReferenceForm::SpecificInterface { minor: 2 }),
            only(&[(1, 0)])
        );
        assert_eq!(
            lineage.descendants(&parent, ReferenceForm::Physical { minor: 3, patch: 0 }),
            only(&[(1, 0)])
        );
        for form in [
            ReferenceForm::Interface,
            ReferenceForm::SpecificInterface { minor: 1 },
        ] {
            assert_eq!(
                every(&lineage, &parent, form),
                [key("child", 1)].into_iter().collect(),
                "{form:?}"
            );
        }
        assert!(
            lineage
                .descendants(&parent, ReferenceForm::SpecificInterface { minor: 4 })
                .is_empty()
        );
    }

    /// Below the queried parent, a grandchild release joins when the child
    /// reference it declares resolves to an admitted child release: an
    /// interface reference to the latest stored one, a full reference to
    /// itself.
    #[test]
    fn a_grandchild_follows_the_child_release_it_resolves_to() {
        let lineage = ArchetypeLineage::from_parent_edges([
            (
                "openEHR-EHR-OBSERVATION.child.v1.0.0",
                "openEHR-EHR-OBSERVATION.parent.v1.1",
            ),
            (
                "openEHR-EHR-OBSERVATION.child.v1.1.0",
                "openEHR-EHR-OBSERVATION.parent.v1.3",
            ),
            (
                "openEHR-EHR-OBSERVATION.grand.v1.0.0",
                "openEHR-EHR-OBSERVATION.child.v1.0.0",
            ),
            (
                "openEHR-EHR-OBSERVATION.grand.v1.1.0",
                "openEHR-EHR-OBSERVATION.child.v1",
            ),
        ]);
        let found = lineage.descendants(
            &key("parent", 1),
            ReferenceForm::SpecificInterface { minor: 2 },
        );
        assert_eq!(
            found.get(&key("grand", 1)),
            Some(&AdmittedReleases::Only([(1, 0)].into_iter().collect())),
            "grand v1.1.0 resolves `child.v1` to child v1.1.0, built on parent v1.3: {found:?}"
        );
        let found = lineage.descendants(
            &key("parent", 1),
            ReferenceForm::SpecificInterface { minor: 4 },
        );
        assert!(
            found.is_empty(),
            "no child release is built on parent v1.4: {found:?}"
        );
    }
}
