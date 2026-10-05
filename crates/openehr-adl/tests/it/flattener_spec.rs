// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Specialisation-flattener conformance harness.
//!
//! Flattens each `tests/corpus/flattener/{specexamples,siblingorder}` child
//! against its parent and asserts against HAND-AUTHORED expectations derived
//! from the vendored spec text (`docs/specs/openehr/AM/docs/AOM2/master08`
//! §Flattening + `ADL2/master09.02`–`master09.10`). No flat golden is vendored
//! (the archie fixtures keep expected flats in Java test code); every assertion
//! below cites the master09.x section it encodes. The `siblingorder` fixtures
//! carry their expected order in a `--resulting order:` comment authored by
//! openEHR; those orders are reproduced here and cross-checked against the
//! `master09.04` §Ordering of Sibling Nodes rules.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::PathBuf;

use openehr_adl::aom::access::{complex_attributes, object_node_id, object_rm_type};
use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::flatten::flat_form;
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

const FLATTENER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/flattener");

// ── fixture loading ─────────────────────────────────────────────────────────

fn read(dir: &str, file: &str) -> Archetype {
    let path = PathBuf::from(format!("{FLATTENER}/{dir}/{file}"));
    let src =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    parse_artefact(&src, Dialect::Adl2).unwrap_or_else(|e| panic!("parse {file}: {e:?}"))
}

/// A repository over an entire flattener subdirectory, so multi-level lineage
/// resolves.
fn repo_of(dir: &str) -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::new();
    let base = PathBuf::from(format!("{FLATTENER}/{dir}"));
    for entry in std::fs::read_dir(&base).unwrap().flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "adls")
            && let Ok(src) = std::fs::read_to_string(&p)
            && let Ok(art) = parse_artefact(&src, Dialect::Adl2)
        {
            repo.insert(art);
        }
    }
    repo
}

/// Flatten the child fixture `child_file` (in `dir`) against its lineage.
fn flatten(dir: &str, child_file: &str) -> Archetype {
    let repo = repo_of(dir);
    let child = read(dir, child_file);
    flat_form(&child, &repo).unwrap_or_else(|e| panic!("flatten {child_file}: {e}"))
}

// ── flat-form navigation helpers ─────────────────────────────────────────────

fn root_def(a: &Archetype) -> &CComplexObject {
    match a {
        Archetype::AuthoredArchetype(inner) => match inner.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => &d.definition,
            AuthoredArchetype::Template(t) => &t.definition,
            AuthoredArchetype::OperationalTemplate(o) => &o.definition,
        },
        Archetype::TemplateOverlay(t) => &t.definition,
    }
}

fn root_node_id(a: &Archetype) -> &str {
    match root_def(a) {
        CComplexObject::CComplexObject(d) => &d.node_id,
        CComplexObject::CArchetypeRoot(r) => &r.node_id,
    }
}

/// Navigate to a nested complex object by a slash path of `attr[id]` segments
/// (empty path = the root), returning `None` if it does not resolve.
fn object_at<'a>(root: &'a CComplexObject, path: &str) -> Option<&'a CComplexObject> {
    let mut current = root;
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        let (attr_name, id) = match seg.split_once('[') {
            Some((a, rest)) => (a, Some(rest.trim_end_matches(']'))),
            None => (seg, None),
        };
        let attr = complex_attributes(current)
            .iter()
            .find(|a| a.rm_attribute_name == attr_name)?;
        let child = match id {
            Some(id) => attr
                .children
                .iter()
                .flatten()
                .find(|c| object_node_id(c) == id)?,
            None => attr.children.iter().flatten().next()?,
        };
        current = match child {
            CObject::CComplexObject(cco) => cco,
            _ => return None,
        };
    }
    Some(current)
}

/// The ordered node ids of the children under `obj_path`'s `attr` attribute.
fn ids(a: &Archetype, obj_path: &str, attr: &str) -> Vec<String> {
    let obj = object_at(root_def(a), obj_path).expect("object path resolves");
    complex_attributes(obj)
        .iter()
        .find(|x| x.rm_attribute_name == attr)
        .map(|x| {
            x.children
                .iter()
                .flatten()
                .map(|c| object_node_id(c).to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The child `CObject` with node id `nid` under `obj_path`'s `attr`.
fn child<'a>(a: &'a Archetype, obj_path: &str, attr: &str, nid: &str) -> &'a CObject {
    let obj = object_at(root_def(a), obj_path).expect("object path resolves");
    complex_attributes(obj)
        .iter()
        .find(|x| x.rm_attribute_name == attr)
        .and_then(|x| {
            x.children
                .iter()
                .flatten()
                .find(|c| object_node_id(c) == nid)
        })
        .unwrap_or_else(|| panic!("no child {nid} under {obj_path}/{attr}"))
}

fn occurrences_prohibited(obj: &CObject) -> bool {
    openehr_adl::aom::access::child_occurrences(obj)
        .is_some_and(openehr_base::prelude::MultiplicityInterval::is_prohibited)
}

// ── siblingorder: exact order (master09.04 §Ordering of Sibling Nodes) ────────
//
// order-parent items = [id2, id3, id4, id5] (id2, id3 occurrences {*}; id4
// {0..1}; id5 default). Each expected order is the fixture's own `--resulting
// order:` comment, cross-checked against master09.04: a marker anchors the run
// of nodes following it until the next marker; `before [X]` inserts before the
// first node conforming to X, `after [X]` after the last; a redefinition with
// no marker replaces its parent in place; extensions with no marker go last.

const SO: &str = "siblingorder";

#[test]
fn siblingorder_reorder_parent_nodes() {
    // after[id5] id2.1 ; before[id5] id3.1 → id4, id3.1, id5, id2.1
    let flat = flatten(SO, "openEHR-EHR-CLUSTER.reorder_parent_nodes.v1.0.0.adls");
    assert_eq!(ids(&flat, "", "items"), ["id4", "id3.1", "id5", "id2.1"]);
}

#[test]
fn siblingorder_test_anchoring() {
    // after[id3] {id0.1,id0.2} ; before[id5] {id0.3,id0.4}
    // → id2, id3, id0.1, id0.2, id4, id0.3, id0.4, id5
    let flat = flatten(SO, "openEHR-EHR-CLUSTER.test_anchoring.v1.0.0.adls");
    assert_eq!(
        ids(&flat, "", "items"),
        [
            "id2", "id3", "id0.1", "id0.2", "id4", "id0.3", "id0.4", "id5"
        ]
    );
}

#[test]
fn siblingorder_redefinition_at_same_place() {
    // No markers: id2.1 replaces id2 in place; id3 is multiple-occurrence and is
    // cloned (kept) with id3.1, id3.2 added (master09.05 §Single and Multiple
    // Specialisation — clone_not_needed is false: parent max occ > 1, not a sole
    // max-1 redef); id0.6 appended at end.
    // → id2.1, id3, id3.1, id3.2, id4, id5, id0.6
    let flat = flatten(
        SO,
        "openEHR-EHR-CLUSTER.redefinition_at_same_place.v1.0.0.adls",
    );
    assert_eq!(
        ids(&flat, "", "items"),
        ["id2.1", "id3", "id3.1", "id3.2", "id4", "id5", "id0.6"]
    );
}

#[test]
fn siblingorder_specialise_first_element() {
    // id2.1 redefines id2 with no explicit max-1 occurrences; id2 is multiple-
    // occurrence so cloning applies (master09.05) — id2 is kept, id2.1 added.
    let flat = flatten(
        SO,
        "openEHR-EHR-CLUSTER.specialise_first_element.v1.0.0.adls",
    );
    assert_eq!(
        ids(&flat, "", "items"),
        ["id2", "id2.1", "id3", "id4", "id5"]
    );
}

#[test]
fn siblingorder_redefined_node_id() {
    // after[id3.1] {id0.5} ; before[id2] {id0.6, id3.1(redef), id0.7}
    // → id0.6, id3.1, id0.5, id0.7, id2, id4, id5
    let flat = flatten(
        SO,
        "openEHR-EHR-CLUSTER.sibling_order_redefined_node_id.v1.0.0.adls",
    );
    assert_eq!(
        ids(&flat, "", "items"),
        ["id0.6", "id3.1", "id0.5", "id0.7", "id2", "id4", "id5"]
    );
}

#[test]
fn siblingorder_redefined_node_id_2() {
    // after[id3.1] {id0.5, id0.8} ; before[id2] {id0.6, id3.1(redef), id0.7}
    // → id0.6, id3.1, id0.5, id0.8, id0.7, id2, id4, id5
    let flat = flatten(
        SO,
        "openEHR-EHR-CLUSTER.sibling_order_redefined_node_id_2.v1.0.0.adls",
    );
    assert_eq!(
        ids(&flat, "", "items"),
        [
            "id0.6", "id3.1", "id0.5", "id0.8", "id0.7", "id2", "id4", "id5"
        ]
    );
}

#[test]
fn siblingorder_redefined_node_id_3() {
    // before[id3.1] {id0.5, id0.8} ; before[id2] {id0.6, id3.1(redef), id0.7}
    // → id0.6, id0.5, id0.8, id3.1, id0.7, id2, id4, id5
    let flat = flatten(
        SO,
        "openEHR-EHR-CLUSTER.sibling_order_redefined_node_id_3.v1.0.0.adls",
    );
    assert_eq!(
        ids(&flat, "", "items"),
        [
            "id0.6", "id0.5", "id0.8", "id3.1", "id0.7", "id2", "id4", "id5"
        ]
    );
}

#[test]
fn siblingorder_tricky_edge_case() {
    // after[id3] {id0.5} — anchor id3 is redefined away to id3.1, so `after`
    // resolves to the last conforming sibling id3.1 (master09.04 anchor-loss);
    // before[id2] {id0.6, id3.1(redef), id0.7}
    // → id0.6, id3.1, id0.5, id0.7, id2, id4, id5
    let flat = flatten(SO, "openEHR-EHR-CLUSTER.tricky_edge_case.v1.0.0.adls");
    assert_eq!(
        ids(&flat, "", "items"),
        ["id0.6", "id3.1", "id0.5", "id0.7", "id2", "id4", "id5"]
    );
}

#[test]
fn siblingorder_child_new_nodes_anchored() {
    // siblingorderparent items = [id5, id6, id7]. Child redefines id6 (occ 1),
    // prohibits id7 (occ 0 — kept visible in plain flat), and anchors new nodes.
    // No `--resulting order` comment is vendored, so assert the anchor relations
    // each marker guarantees (master09.04 §Ordering of Sibling Nodes).
    let flat = flatten(SO, "openEHR-EHR-CLUSTER.siblingorderchild.v1.0.0.adls");
    let order = ids(&flat, "", "items");
    let pos = |id: &str| {
        order
            .iter()
            .position(|x| x == id)
            .unwrap_or_else(|| panic!("{id} missing in {order:?}"))
    };
    assert!(pos("id0.2") < pos("id5"), "before[id5]: {order:?}");
    assert!(pos("id0.5") < pos("id6"), "before[id6]: {order:?}");
    assert!(pos("id0.8") < pos("id7"), "before[id7]: {order:?}");
    assert!(pos("id0.9") > pos("id6"), "after[id6]: {order:?}");
    // id7 prohibited but retained (plain flat keeps prohibited nodes visible).
    assert!(order.contains(&"id7".to_owned()));
}

#[test]
fn siblingorder_archetype_slot_filled() {
    // archetype_slot_parent has an open slot id2; the child closes id2 and adds
    // two C_ARCHETYPE_ROOT fillers id2.1, id2.2 (use_archetype). All three
    // survive in the plain flat form.
    let flat = flatten(SO, "openEHR-EHR-CLUSTER.archetype_slot_filled.v1.0.0.adls");
    let order = ids(&flat, "", "items");
    for id in ["id2", "id2.1", "id2.2"] {
        assert!(order.contains(&id.to_owned()), "{id} missing in {order:?}");
    }
}

// ── specexamples: structural expectations (master09.05 object redefinition) ───

const SX: &str = "specexamples";

#[test]
fn specexample_occurrences_prohibition_kept_visible() {
    // occurrences_parent value = [id4(DV_QUANTITY), id5, id6, id7]. The child
    // prohibits id4 and id7 (occurrences {0}). Value is single-valued so each is
    // a max-1 in-place replacement (master09.05); plain flat keeps the prohibited
    // nodes visible, stripped of children (module NOTE), id5/id6 inherited.
    let flat = flatten(
        SX,
        "openEHR-EHR-CLUSTER.occurrences_specialized.v1.0.0.adls",
    );
    assert_eq!(root_node_id(&flat), "id1.1");
    let value = ids(&flat, "items[id3]", "value");
    for id in ["id4", "id5", "id6", "id7"] {
        assert!(value.contains(&id.to_owned()), "{id} missing in {value:?}");
    }
    assert!(occurrences_prohibited(child(
        &flat,
        "items[id3]",
        "value",
        "id4"
    )));
    assert!(occurrences_prohibited(child(
        &flat,
        "items[id3]",
        "value",
        "id7"
    )));
    assert!(!occurrences_prohibited(child(
        &flat,
        "items[id3]",
        "value",
        "id5"
    )));
}

#[test]
fn specexample_type_refinement_replaces_single_node() {
    // type_refinement_parent value = DV_AMOUNT[id4] (single-valued). The child
    // redefines it into three alternatives id4.1/id4.2/id4.3 — a max-1 parent is
    // replaced in place (master09.05: clone_not_needed, so no clone), id4 gone.
    let flat = flatten(
        SX,
        "openEHR-EHR-ELEMENT.type_refinement_specialized.v1.0.0.adls",
    );
    let value = ids(&flat, "", "value");
    assert_eq!(
        value,
        ["id4.1", "id4.2", "id4.3"],
        "id4 replaced by its alternatives"
    );
    assert_eq!(
        object_rm_type(child(&flat, "", "value", "id4.1")),
        "DV_QUANTITY"
    );
    assert_eq!(
        object_rm_type(child(&flat, "", "value", "id4.2")),
        "DV_PROPORTION"
    );
    assert_eq!(
        object_rm_type(child(&flat, "", "value", "id4.3")),
        "DV_AMOUNT"
    );
}

#[test]
fn specexample_cardinality_clones_multiple_occurrence_node() {
    // cardinality_parent has ELEMENT[id12] occurrences {0..*} (multiple). The
    // child splits it into id12.1..id12.10 — a multiple-occurrence parent redefined
    // by more than one child is CLONED (master09.05 §Single and Multiple
    // Specialisation): the parent id12 is retained and the ten clones added. The
    // child also restates the container cardinality to {3..10}.
    let flat = flatten(
        SX,
        "openEHR-EHR-CLUSTER.cardinality_specialized.v1.0.0.adls",
    );
    let items = ids(&flat, "items[id3]", "items");
    for n in 1..=10 {
        let id = format!("id12.{n}");
        assert!(items.contains(&id), "{id} missing in {items:?}");
    }
    assert!(
        items.contains(&"id12".to_owned()),
        "cloned parent id12 retained: {items:?}"
    );
}

#[test]
fn specexample_reference_redefinition_expands_proxy_inline() {
    // reference_redefinition_parent data = { CLUSTER[id2]{items{ELEMENT[id4]}},
    // use_node CLUSTER[id3] /data[id2] }. The child overrides through the proxy
    // path (/data[id3]/items) adding ELEMENT[id0.1]; the proxy is expanded inline
    // to a copy of its target, then the addition is overlaid (master09.05
    // §Internal Reference (Proxy Object) Redefinition).
    let flat = flatten(
        SX,
        "openEHR-EHR-ENTRY.reference_redefinition_specialized.v1.0.0.adls",
    );
    let data = ids(&flat, "", "data");
    assert!(data.contains(&"id2".to_owned()) && data.contains(&"id3".to_owned()));
    // id3 is now a complex object (expanded proxy), not a proxy.
    assert!(matches!(
        child(&flat, "", "data", "id3"),
        CObject::CComplexObject(_)
    ));
    let id3_items = ids(&flat, "data[id3]", "items");
    assert!(
        id3_items.contains(&"id4".to_owned()),
        "target ELEMENT[id4] inlined: {id3_items:?}"
    );
    assert!(
        id3_items.contains(&"id0.1".to_owned()),
        "added ELEMENT[id0.1]: {id3_items:?}"
    );
}

#[test]
fn specexample_reference_redefinition_no_replacement_keeps_proxy() {
    // The child does NOT override the proxy id3, so in the plain flat form the
    // proxy is retained as a proxy (un-overridden use_node kept — module NOTE;
    // OPT-form inlining is A8).
    let flat = flatten(
        SX,
        "openEHR-EHR-ENTRY.reference_redefinition_no_replacement.v1.0.0.adls",
    );
    assert!(matches!(
        child(&flat, "", "data", "id3"),
        CObject::CComplexObjectProxy(_)
    ));
}

#[test]
fn specexample_diagnosis_adds_and_orders_status_nodes() {
    // The AOM2 worked example (diagnosis → problem). The child redefines
    // /data[id2]/items[id3]/value's DV_TEXT to DV_CODED_TEXT[id4], and under
    // /data/items inserts new nodes: id0.32 before id5, and id0.35/id0.37 after
    // id31 (master09.04 sibling order; master09.05 object redefinition). The
    // ac0.1 value set is added to the flat terminology (master09.09).
    let flat = flatten(SX, "diagnosis.adls");
    assert_eq!(root_node_id(&flat), "id1.1");
    let items = ids(&flat, "data[id2]", "items");
    for id in ["id0.32", "id0.35", "id0.37"] {
        assert!(items.contains(&id.to_owned()), "{id} missing in {items:?}");
    }
    let pos = |id: &str| {
        items
            .iter()
            .position(|x| x == id)
            .unwrap_or_else(|| panic!("{id} in {items:?}"))
    };
    assert!(pos("id0.32") < pos("id5"), "id0.32 before id5: {items:?}");
    assert!(pos("id0.35") > pos("id31"), "id0.35 after id31: {items:?}");
    assert!(pos("id0.37") > pos("id31"), "id0.37 after id31: {items:?}");
    // DV_CODED_TEXT redefinition of the value leaf.
    assert_eq!(
        object_rm_type(child(&flat, "data[id2]/items[id3]", "value", "id4")),
        "DV_CODED_TEXT"
    );
}

#[test]
fn specexamples_all_flatten_clean() {
    // Every specialised specexample flattens without error and stamps the child
    // root id (master08 §Flattening). Parent-only fixtures flatten to themselves.
    for entry in std::fs::read_dir(format!("{FLATTENER}/{SX}"))
        .unwrap()
        .flatten()
    {
        let p = entry.path();
        if p.extension().is_none_or(|e| e != "adls") {
            continue;
        }
        let file = p.file_name().unwrap().to_string_lossy().to_string();
        let repo = repo_of(SX);
        let child = parse_artefact(&std::fs::read_to_string(&p).unwrap(), Dialect::Adl2).unwrap();
        let flat = flat_form(&child, &repo).unwrap_or_else(|e| panic!("flatten {file}: {e}"));
        // A flat form is non-differential.
        assert!(
            !is_differential(&flat),
            "{file} flat form is non-differential"
        );
    }
}

#[test]
fn flattener_corpus_coverage_gate() {
    // Every `.adls` file under `tests/corpus/flattener/**` is claimed: a
    // specialised fixture is a flatten subject; a level-0 parent-only fixture
    // flattens to itself (used as a parent input). No dead fixtures
    // (a HARD REQUIREMENT: every vendored corpus file is exercised).
    let mut claimed = 0usize;
    for dir in [SX, SO] {
        let repo = repo_of(dir);
        for entry in std::fs::read_dir(format!("{FLATTENER}/{dir}"))
            .unwrap()
            .flatten()
        {
            let p = entry.path();
            if p.extension().is_none_or(|e| e != "adls") {
                continue;
            }
            let file = p.file_name().unwrap().to_string_lossy().to_string();
            let child = parse_artefact(&std::fs::read_to_string(&p).unwrap(), Dialect::Adl2)
                .unwrap_or_else(|e| panic!("parse {file}: {e:?}"));
            flat_form(&child, &repo).unwrap_or_else(|e| panic!("flatten {file}: {e}"));
            claimed += 1;
        }
    }
    // 25 specexamples + 13 siblingorder fixtures (INVENTORY §1).
    assert_eq!(claimed, 38, "every flattener fixture is exercised");
}

fn is_differential(a: &Archetype) -> bool {
    match a {
        Archetype::AuthoredArchetype(inner) => match inner.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => d.is_differential,
            AuthoredArchetype::Template(t) => t.is_differential,
            AuthoredArchetype::OperationalTemplate(o) => o.is_differential,
        },
        Archetype::TemplateOverlay(t) => t.is_differential,
    }
}

// ── round-trip: a printed flat form re-parses (master07.04 flat keyword) ──────

#[test]
fn flat_form_reprints_and_reparses() {
    // The flat form prints with the `flat archetype` header (master07.04
    // §Artefact declaration) and re-parses to a structurally-equal definition.
    let flat = flatten(
        SX,
        "openEHR-EHR-CLUSTER.cardinality_specialized.v1.0.0.adls",
    );
    let text = openehr_adl::print::print(&flat).expect("print the flat form");
    assert!(
        text.starts_with("flat archetype"),
        "flat header:\n{}",
        text.get(..40.min(text.len())).unwrap_or(&text)
    );
    let reparsed =
        parse_artefact(&text, Dialect::Adl2).unwrap_or_else(|e| panic!("reparse flat: {e:?}"));
    assert_eq!(root_node_id(&reparsed), root_node_id(&flat));
    // The ten specialised element ids survive the print→parse round-trip.
    let items = ids(&reparsed, "items[id3]", "items");
    for n in 1..=10 {
        assert!(
            items.contains(&format!("id12.{n}")),
            "id12.{n} survived round-trip: {items:?}"
        );
    }
}

// ── tuple overlay: merge by member-attribute group ──────────────────────────
//
// NOTE: no released text states whether a child node's `C_ATTRIBUTE_TUPLE` set
// replaces or merges with the flat parent's (`AOM2/master08` §Flattening names
// no second-order case) — group-keyed merge is our own design.

/// A level-0 `ELEMENT` whose `DV_QUANTITY` value carries two tuples over
/// disjoint member-attribute groups.
const TUPLE_PARENT: &str = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-ELEMENT.tuple_overlay_parent.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"unmanaged\">

definition
\tELEMENT[id1] matches {
\t\tvalue matches {
\t\t\tDV_QUANTITY[id2] matches {
\t\t\t\t[magnitude, units] matches {
\t\t\t\t\t[{|>=50.0|}, {\"mm[Hg]\"}],
\t\t\t\t\t[{|>=68.0|}, {\"cm[H20]\"}]
\t\t\t\t}
\t\t\t\t[precision, magnitude_status] matches {
\t\t\t\t\t[{2}, {\"=\"}]
\t\t\t\t}
\t\t\t}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"element\"> description=<\"element\">>
\t\t\t[\"id2\"] = <text=<\"quantity\"> description=<\"quantity\">>
\t\t>
\t>
";

/// A level-1 child of [`TUPLE_PARENT`] restating ONLY the `[magnitude, units]`
/// group (narrowed to one row).
const TUPLE_CHILD_SAME_GROUP: &str = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-ELEMENT.tuple_overlay_child.v1.0.0

specialize
\topenEHR-EHR-ELEMENT.tuple_overlay_parent.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"unmanaged\">

definition
\tELEMENT[id1.1] matches {
\t\t/value matches {
\t\t\tDV_QUANTITY[id2] matches {
\t\t\t\t[magnitude, units] matches {
\t\t\t\t\t[{|>=50.0|}, {\"mm[Hg]\"}]
\t\t\t\t}
\t\t\t}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1.1\"] = <text=<\"element\"> description=<\"element\">>
\t\t>
\t>
";

/// A level-1 child of [`TUPLE_PARENT`] adding a group the parent's `id2` node
/// carries no tuple for.
const TUPLE_CHILD_NEW_GROUP: &str = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-ELEMENT.tuple_overlay_child2.v1.0.0

specialize
\topenEHR-EHR-ELEMENT.tuple_overlay_parent.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"unmanaged\">

definition
\tELEMENT[id1.1] matches {
\t\t/value matches {
\t\t\tDV_QUANTITY[id2] matches {
\t\t\t\t[accuracy, accuracy_is_percent] matches {
\t\t\t\t\t[{|0.0..1.0|}, {True}]
\t\t\t\t}
\t\t\t}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1.1\"] = <text=<\"element\"> description=<\"element\">>
\t\t>
\t>
";

/// Flatten `child_src` against `parent_src` alone.
fn flatten_pair(parent_src: &str, child_src: &str) -> Archetype {
    let mut repo = ArchetypeRepository::new();
    repo.insert(parse_artefact(parent_src, Dialect::Adl2).expect("parent parses"));
    let child = parse_artefact(child_src, Dialect::Adl2).expect("child parses");
    flat_form(&child, &repo).expect("flatten")
}

/// The `(sorted member names, row count)` of every tuple on the flat form's
/// `value` node, sorted by group for a stable comparison.
fn value_tuple_groups(flat: &Archetype) -> Vec<(Vec<String>, usize)> {
    let obj = object_at(root_def(flat), "value").expect("value node resolves");
    let CComplexObject::CComplexObject(data) = obj else {
        panic!("value is a plain complex object");
    };
    let mut groups: Vec<(Vec<String>, usize)> = data
        .attribute_tuples
        .iter()
        .flatten()
        .map(|t| {
            let mut names: Vec<String> = t
                .members
                .iter()
                .flatten()
                .map(|m| m.rm_attribute_name.clone())
                .collect();
            names.sort();
            (names, t.tuples.iter().flatten().count())
        })
        .collect();
    groups.sort();
    groups
}

#[test]
fn tuple_overlay_retains_a_parent_tuple_over_a_disjoint_group() {
    // The child restates only `[magnitude, units]`; the parent's
    // `[precision, magnitude_status]` tuple constrains a disjoint attribute
    // group, so nothing in the child redefines it and it survives the overlay
    // with its row intact.
    let flat = flatten_pair(TUPLE_PARENT, TUPLE_CHILD_SAME_GROUP);
    let groups = value_tuple_groups(&flat);
    assert!(
        groups
            .iter()
            .any(|(names, rows)| names == &["magnitude_status", "precision"] && *rows == 1),
        "the disjoint parent group survives: {groups:?}"
    );
    assert_eq!(groups.len(), 2, "both groups present: {groups:?}");
}

#[test]
fn tuple_overlay_replaces_the_parent_tuple_over_the_same_group() {
    // `ADL2/master09.05` §Tuple Redefinition: a child narrows a tuple by
    // restating that group's whole row list, so the parent's two
    // `[magnitude, units]` rows are replaced by the child's single row.
    let flat = flatten_pair(TUPLE_PARENT, TUPLE_CHILD_SAME_GROUP);
    let groups = value_tuple_groups(&flat);
    assert!(
        groups
            .iter()
            .any(|(names, rows)| names == &["magnitude", "units"] && *rows == 1),
        "the same group is replaced, not unioned: {groups:?}"
    );
}

#[test]
fn tuple_overlay_appends_a_group_the_parent_does_not_carry() {
    // A child tuple over a group with no parent counterpart is a new
    // second-order constraint the conformance functions leave unrefuted
    // (`AOM2/master04.5` §Conformance semantics: `C_ATTRIBUTE_TUPLE` — no
    // corresponding parent tuple), so it joins the inherited groups.
    let flat = flatten_pair(TUPLE_PARENT, TUPLE_CHILD_NEW_GROUP);
    let groups = value_tuple_groups(&flat);
    assert_eq!(
        groups
            .iter()
            .map(|(names, _)| names.join(","))
            .collect::<Vec<_>>(),
        [
            "accuracy,accuracy_is_percent",
            "magnitude,units",
            "magnitude_status,precision"
        ],
        "all three groups present: {groups:?}"
    );
}
