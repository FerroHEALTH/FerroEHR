// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which RM `_type`s become their own `node` row, and archetype-id part
//! extraction for the promoted subsumption columns.
//!
//! No openEHR spec governs the decomposition granularity — it is our own
//! storage design. The *composition-content*
//! structure set is NOT hand-maintained here: it is delegated to the single
//! BMM-generated oracle [`openehr_rm::v1_2::model::is_structure_root`], which the
//! codegen keeps in lockstep with this codec — never a local duplicate
//! constant. The local additions are the demographic types the RM model
//! deliberately excludes (a party is never composition content): the five
//! **party roots** and the four archetypable **containers** nested inside them.

use openehr_base::prelude::ArchetypeId;

/// The five concrete demographic **party roots** (RM demographic). Each is a
/// standalone versioned object that reuses the `node`/`version` machinery
/// (with a NULL `ehr_id`), so it must be accepted as a decomposition root — yet
/// it is intentionally NOT part of the composition-content structure set the RM
/// model tracks.
/// (The delta is pinned against [`crate::versioning::Kind`] by
/// `tests::demographic_party_roots_mirror_the_versioning_kinds` — the
/// versioned-object domain is the owner of "which RM types are party roots";
/// this list only records which of them the node codec also splits into rows.)
const DEMOGRAPHIC_PARTY_ROOTS: [&str; 5] = ["PERSON", "ORGANISATION", "GROUP", "AGENT", "ROLE"];

/// The four archetypable demographic **containers** nested inside a party,
/// reached through its `identities`, `contacts`, `contacts.addresses` and
/// `capabilities` attributes.
///
/// Each carries a `details`/`credentials` `ITEM_STRUCTURE` and is therefore "a
/// completely archetypable structure" in its own right (RM demographic
/// `master02-demographic_package.adoc` §Archetyping), so its content is
/// decomposed exactly like composition content: the container gets a row, its
/// `ITEM_TREE` and `ELEMENT` descendants get theirs, and a full archetype HRID
/// on the container becomes the archetyped ancestor its at-coded leaves are
/// scoped by. Leaving them inline on the party root put that content out of
/// reach of every row-level predicate.
///
/// `PARTY_RELATIONSHIP` is deliberately absent: it is a versioned object of its
/// own, and a relationship nested in a party's `relationships` list stays
/// inline verbatim (see [`is_versioned_root_type`]).
const DEMOGRAPHIC_CONTAINERS: [&str; 4] = ["CONTACT", "ADDRESS", "PARTY_IDENTITY", "CAPABILITY"];

/// Whether an RM `_type` gets its own `node` row: the BMM-generated
/// composition-content structure set, plus the demographic party roots and the
/// demographic containers nested inside them.
#[must_use]
pub fn is_structure_type(rm_type: &str) -> bool {
    openehr_rm::v1_2::model::is_structure_root(rm_type)
        || DEMOGRAPHIC_PARTY_ROOTS.contains(&rm_type)
        || DEMOGRAPHIC_CONTAINERS.contains(&rm_type)
}

/// Whether an RM `_type` may be the **root** of a versioned object handed to
/// [`crate::storage::codec::decompose`].
///
/// This is [`is_structure_type`] minus the demographic containers, plus
/// `PARTY_RELATIONSHIP`. A container is decomposed into rows but is never a
/// versioned object: only `PARTY` and its descendants are versioned in a
/// demographic system (RM demographic `master02-demographic_package.adoc`
/// §Versioning Semantics), alongside `PARTY_RELATIONSHIP` — which in turn is
/// deliberately **not** a structure type, so a relationship nested inside a
/// party's `relationships` attribute stays inline. Splitting the two predicates
/// gives all three behaviours from one codec.
#[must_use]
pub fn is_versioned_root_type(rm_type: &str) -> bool {
    (is_structure_type(rm_type) && !DEMOGRAPHIC_CONTAINERS.contains(&rm_type))
        || rm_type == crate::versioning::Kind::PartyRelationship.as_str()
}

/// Parses a full archetype HRID `archetype_node_id` into its identifying parts.
///
/// Yields the `(qualified_rm_entity, domain_concept, major)` parts, lowercased
/// for case-insensitive comparison (BASE `base_types` master05 §Archetype
/// Identifiers and §"Composite Identifiers and Case"). Reuses the shared
/// [`ArchetypeId`] parser (never a hand-rolled regex); returns `None` for
/// at/id-codes and any value that is not a full HRID with a numeric major.
#[must_use]
pub fn archetype_parts(node_id: &str) -> Option<(String, String, i32)> {
    let id: ArchetypeId = node_id.parse().ok()?;
    let major: i32 = id.major_version().parse().ok()?;
    Some((
        id.qualified_rm_entity().to_ascii_lowercase(),
        id.domain_concept().to_ascii_lowercase(),
        major,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The party-root delta is exactly the party kinds of the versioned-object
    /// domain — a kind added to [`crate::versioning::Kind`] without a row here
    /// fails this test.
    #[test]
    fn demographic_party_roots_mirror_the_versioning_kinds() {
        let mut from_kinds: Vec<&str> = crate::versioning::Kind::ALL
            .iter()
            .filter(|k| k.is_party())
            .map(|k| k.as_str())
            .collect();
        from_kinds.sort_unstable();
        let mut listed: Vec<&str> = DEMOGRAPHIC_PARTY_ROOTS.to_vec();
        listed.sort_unstable();
        assert_eq!(from_kinds, listed, "party roots must mirror `Kind`");
    }

    #[test]
    fn party_roots_are_structure_roots() {
        for t in DEMOGRAPHIC_PARTY_ROOTS {
            assert!(is_structure_type(t), "{t} must be a structure root");
            assert!(is_versioned_root_type(t));
        }
    }

    #[test]
    fn composition_content_delegates_to_the_rm_model() {
        // Every type the BMM-generated oracle calls a structure root is a
        // storage structure type — the single source of truth, no local dup.
        for t in [
            "COMPOSITION",
            "OBSERVATION",
            "SECTION",
            "CLUSTER",
            "ELEMENT",
            "EVENT_CONTEXT",
            "FEEDER_AUDIT",
            "ITEM_TREE",
        ] {
            assert!(openehr_rm::v1_2::model::is_structure_root(t));
            assert!(is_structure_type(t), "{t}");
        }
    }

    /// The storage set = the RM-model structure set ⊎ the party roots ⊎ the
    /// demographic containers, and the two additions are disjoint from the
    /// model's own set (which excludes the demographic hierarchy).
    ///
    /// The containers earn a row because each carries an archetypable
    /// `ITEM_STRUCTURE` (RM demographic `master02-demographic_package.adoc`
    /// §Archetyping); they are still never versioned objects, because only
    /// `PARTY` and its descendants are (§Versioning Semantics), so
    /// [`is_versioned_root_type`] excludes them again.
    #[test]
    fn party_roots_and_containers_are_the_storage_delta_from_the_rm_model() {
        for t in DEMOGRAPHIC_PARTY_ROOTS
            .iter()
            .chain(&DEMOGRAPHIC_CONTAINERS)
        {
            assert!(
                !openehr_rm::v1_2::model::is_structure_root(t),
                "{t} unexpectedly in the RM-model structure set"
            );
            assert!(is_structure_type(t), "{t} must get its own node row");
        }
        for t in DEMOGRAPHIC_PARTY_ROOTS {
            assert!(is_versioned_root_type(t), "{t} is a versioned object");
        }
        for t in DEMOGRAPHIC_CONTAINERS {
            assert!(
                !is_versioned_root_type(t),
                "{t} is decomposed but never a versioned object"
            );
        }
    }

    #[test]
    fn a_nested_relationship_stays_inline_and_is_still_a_versioned_root() {
        assert!(
            !is_structure_type("PARTY_RELATIONSHIP"),
            "a relationship nested in a party's `relationships` stays inline"
        );
        assert!(is_versioned_root_type("PARTY_RELATIONSHIP"));
    }

    #[test]
    fn parses_full_hrid_lowercased() {
        let (entity, concept, major) =
            archetype_parts("openEHR-EHR-OBSERVATION.laboratory-glucose.v2").unwrap();
        assert_eq!(entity, "openehr-ehr-observation");
        assert_eq!(concept, "laboratory-glucose");
        assert_eq!(major, 2);
    }

    #[test]
    fn at_codes_have_no_parts() {
        assert!(archetype_parts("at0001").is_none());
    }
}
