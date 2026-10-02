// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    let_underscore_drop,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
//! Tests for the generated static RM attribute/type model (`openehr_rm::v1_2::model`,
//! the AQL planner's oracle). These assert behaviour the
//! planner relies on: inheritance-aware attribute resolution, descendant/ancestor
//! sets, container multiplicity, and the node-codec structure classification.

use openehr_rm::v1_2::model::{
    Container, PRIMITIVE_TYPES_PACKAGE, ancestors, attribute, attributes, class, classes,
    conforms_to_ordered, descendants, is_a, is_foundation_type, is_primitive, is_structure_root,
};

#[test]
fn inherited_attribute_resolves_through_the_hierarchy() {
    // LOCATABLE.name resolves for OBSERVATION (ancestor-flattened attributes).
    let name = attribute("OBSERVATION", "name").expect("OBSERVATION.name");
    assert_eq!(name.declared_type, "DV_TEXT");
    assert_eq!(name.container, Container::None);
    assert!(name.is_mandatory);

    // OBSERVATION.data : HISTORY (single, mandatory).
    let data = attribute("OBSERVATION", "data").expect("OBSERVATION.data");
    assert_eq!(data.declared_type, "HISTORY");
    assert_eq!(data.container, Container::None);

    // The flattened attribute set includes both own and inherited attributes.
    let names: Vec<&str> = attributes("OBSERVATION").map(|a| a.name).collect();
    assert!(names.contains(&"name")); // inherited from LOCATABLE
    assert!(names.contains(&"data")); // own
}

#[test]
fn event_context_is_pathable_not_locatable() {
    // EVENT_CONTEXT inherits PATHABLE, not LOCATABLE, so `name` does NOT resolve.
    assert!(class("EVENT_CONTEXT").is_some());
    assert!(
        attribute("EVENT_CONTEXT", "name").is_none(),
        "EVENT_CONTEXT must not inherit LOCATABLE.name"
    );
    assert!(is_a("EVENT_CONTEXT", "PATHABLE"));
    assert!(!is_a("EVENT_CONTEXT", "LOCATABLE"));
}

#[test]
fn entry_descendants_are_the_concrete_entry_subtypes() {
    let d = descendants("ENTRY");
    for c in [
        "OBSERVATION",
        "EVALUATION",
        "INSTRUCTION",
        "ACTION",
        "ADMIN_ENTRY",
    ] {
        assert!(d.contains(&c), "descendants(ENTRY) missing {c}: {d:?}");
    }
    // ENTRY and CARE_ENTRY are abstract → excluded from the concrete descendant set.
    assert!(!d.contains(&"ENTRY"));
    assert!(!d.contains(&"CARE_ENTRY"));
}

#[test]
fn is_a_walks_the_ancestor_chain() {
    assert!(is_a("DV_CODED_TEXT", "DATA_VALUE"));
    assert!(is_a("DV_CODED_TEXT", "DV_TEXT"));
    assert!(is_a("DV_CODED_TEXT", "DV_CODED_TEXT")); // reflexive
    assert!(!is_a("DV_TEXT", "DV_CODED_TEXT")); // not the other way

    // ancestors() exposes the transitive chain.
    let anc = ancestors("OBSERVATION");
    for a in [
        "CARE_ENTRY",
        "ENTRY",
        "CONTENT_ITEM",
        "LOCATABLE",
        "PATHABLE",
    ] {
        assert!(
            anc.contains(&a),
            "ancestors(OBSERVATION) missing {a}: {anc:?}"
        );
    }
}

#[test]
fn container_multiplicity_is_recorded() {
    let content = attribute("COMPOSITION", "content").expect("COMPOSITION.content");
    assert_eq!(content.container, Container::List);
    assert_eq!(content.declared_type, "CONTENT_ITEM");

    // A generic list attribute reduces to its item root, tagged List.
    let events = attribute("HISTORY", "events").expect("HISTORY.events");
    assert_eq!(events.container, Container::List);
    assert_eq!(events.declared_type, "EVENT");
}

#[test]
fn generic_parameters_resolve_to_their_bound() {
    // DV_INTERVAL<T: DV_ORDERED>: `lower`/`upper` are the bare param T → DV_ORDERED.
    let lower = attribute("DV_INTERVAL", "lower").expect("DV_INTERVAL.lower");
    assert_eq!(lower.declared_type, "DV_ORDERED");
    let upper = attribute("DV_INTERVAL", "upper").expect("DV_INTERVAL.upper");
    assert_eq!(upper.declared_type, "DV_ORDERED");
}

#[test]
fn is_structure_root_matches_the_node_codec() {
    // Mirrors ferroehr::storage::codec::STRUCTURE_TYPES.
    for t in [
        "COMPOSITION",
        "EHR_STATUS",
        "FOLDER",
        "EVENT_CONTEXT",
        "SECTION",
        "OBSERVATION",
        "EVALUATION",
        "INSTRUCTION",
        "ACTION",
        "ACTIVITY",
        "HISTORY",
        "POINT_EVENT",
        "INTERVAL_EVENT",
        "ITEM_TREE",
        "CLUSTER",
        "ELEMENT",
        "FEEDER_AUDIT",
    ] {
        assert!(is_structure_root(t), "{t} should be a structure root");
    }
    // Data values, references, demographics, and EHR itself are NOT node roots.
    for t in [
        "DV_TEXT",
        "DV_CODED_TEXT",
        "CODE_PHRASE",
        "PARTY_IDENTIFIED",
        "EHR",
        "DV_QUANTITY",
    ] {
        assert!(!is_structure_root(t), "{t} should NOT be a structure root");
    }
}

/// The primitive types are model classes in the BASE `primitive_types` package:
/// the BASE `foundation_types` master03 §Overview table (`Octet`, `Character`,
/// `Boolean`, `Integer`, `Integer64`, `Real`, `Double`, `String`) plus `Uri`,
/// the `String` subtype the package also declares. The abstract markers of the
/// same package (`Ordered`, `Numeric`, …) are not primitives.
#[test]
fn the_primitives_are_the_concrete_classes_of_the_primitive_types_package() {
    let mut primitives: Vec<&str> = classes()
        .filter(|c| is_primitive(c.name))
        .map(|c| c.name)
        .collect();
    primitives.sort_unstable();
    assert_eq!(
        primitives,
        [
            "Boolean",
            "Character",
            "Double",
            "Integer",
            "Integer64",
            "Octet",
            "Real",
            "String",
            "Uri"
        ]
    );
    for marker in ["Ordered", "Numeric", "Ordered_Numeric", "Comparable"] {
        let c = class(marker).expect("the marker is a model class");
        assert_eq!(c.package, PRIMITIVE_TYPES_PACKAGE, "{marker}");
        assert!(c.is_abstract && !is_primitive(marker), "{marker}");
    }
    assert!(!is_primitive("DV_TEXT") && !is_primitive("Iso8601_date_time"));
    assert_eq!(
        class("OBSERVATION").map(|c| c.package),
        Some("org.openehr.rm.composition.content.entry")
    );
}

/// `Ordered` conformance follows the BMM ancestry: `DV_ORDERED` and every
/// subtype, the ordered primitives (`Integer` through `Ordered_Numeric`), and
/// the ISO 8601 types; never `Boolean`, `DV_TEXT` or `DV_BOOLEAN`.
#[test]
fn ordered_conformance_follows_the_bmm_ancestry() {
    for ordered in [
        "Ordered",
        "DV_ORDERED",
        "DV_QUANTITY",
        "DV_COUNT",
        "DV_DATE_TIME",
        "DV_ORDINAL",
        "DV_SCALE",
        "DV_PROPORTION",
        "Integer",
        "Integer64",
        "Real",
        "Double",
        "String",
        "Uri",
        "Character",
        "Octet",
        "Iso8601_date_time",
    ] {
        assert!(conforms_to_ordered(ordered), "{ordered}");
    }
    for unordered in ["Boolean", "DV_TEXT", "DV_BOOLEAN", "DV_CODED_TEXT", "Any"] {
        assert!(!conforms_to_ordered(unordered), "{unordered}");
    }
    assert!(ancestors("DV_ORDERED").contains(&"Ordered"));
    assert!(ancestors("Integer").contains(&"Ordered_Numeric"));
    // Every concrete descendant of `DV_ORDERED` conforms, by construction.
    for d in descendants("DV_ORDERED") {
        assert!(conforms_to_ordered(d), "{d}");
    }
}

/// A reference-typed attribute names the class the RM text says it points to,
/// through inheritance, and none where the text leaves the target open.
#[test]
fn reference_attributes_name_their_rm_target() {
    let target = |class: &str, attr: &str| {
        attribute(class, attr)
            .unwrap_or_else(|| panic!("{class}.{attr} is in the model"))
            .ref_target
    };
    assert_eq!(target("EHR", "ehr_status"), Some("EHR_STATUS"));
    assert_eq!(target("EHR", "ehr_access"), Some("EHR_ACCESS"));
    assert_eq!(target("EHR", "compositions"), Some("VERSIONED_COMPOSITION"));
    assert_eq!(target("EHR", "contributions"), Some("CONTRIBUTION"));
    assert_eq!(target("EHR", "directory"), Some("FOLDER"));
    assert_eq!(target("EHR", "tags"), Some("ITEM_TAG"));
    // Inherited from VERSION.
    assert_eq!(
        target("ORIGINAL_VERSION", "contribution"),
        Some("CONTRIBUTION")
    );
    assert_eq!(
        target("PARTY_RELATIONSHIP", "target"),
        Some("VERSIONED_PARTY")
    );
    assert_eq!(
        target("INSTRUCTION_DETAILS", "instruction_id"),
        Some("INSTRUCTION")
    );
    // The text names no single class.
    assert_eq!(target("VERSIONED_COMPOSITION", "owner_id"), None);
    assert_eq!(target("FOLDER", "items"), None);
    // Not a reference.
    assert_eq!(target("EHR", "ehr_id"), None);
    // Every named target is a model class, and only reference-typed
    // attributes carry one.
    for c in classes() {
        for a in c.attributes {
            if let Some(t) = a.ref_target {
                assert!(class(t).is_some(), "{}.{} → {t}", c.name, a.name);
                assert!(is_a(a.declared_type, "OBJECT_REF"), "{}.{}", c.name, a.name);
            }
        }
    }
}

/// The foundation types are the classes of the BASE `foundation_types`
/// packages: value types, never RM content.
#[test]
fn foundation_types_are_the_base_foundation_packages() {
    for t in [
        "Any",
        "Integer",
        "String",
        "Ordered",
        "List",
        "Interval",
        "Iso8601_date_time",
    ] {
        assert!(is_foundation_type(t), "{t}");
    }
    for t in [
        "DV_TEXT",
        "OBJECT_REF",
        "CODE_PHRASE",
        "PROPORTION_KIND",
        "OBSERVATION",
    ] {
        assert!(!is_foundation_type(t), "{t}");
    }
}
