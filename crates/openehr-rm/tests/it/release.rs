// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! `Generation::earliest_for_composition`: the oldest RM generation whose
//! canonical-JSON decode accepts a composition.
//!
//! Spec authority: `ARCHETYPED.rm_version`
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.archetyped.adoc`)
//! and `GENERIC_ENTRY.data`, typed `ITEM_TREE` by the RM 1.1.0 BMM and `ITEM`
//! by RM 1.2.0
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.integration.generic_entry.adoc`).

#![allow(
    clippy::expect_used,
    reason = "fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::error::Error as _;

use openehr_rm::Generation;
use serde_json::{Value, json};

/// A composition of RM 1.1.0 elements only.
const PERSISTENT: &str =
    include_str!("../../../../corpus/fixtures/composition/minimal_persistent.v1.json");

/// A composition holding a `GENERIC_ENTRY` whose `data` is a `CLUSTER`.
const GENERIC_ENTRY_CLUSTER: &str =
    include_str!("../../../../corpus/fixtures/composition/generic_entry_dev_surface.json");

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).expect("the fixture should be JSON")
}

#[test]
fn a_composition_of_release_1_1_0_elements_answers_v1_1() {
    assert_eq!(
        Generation::earliest_for_composition(&parsed(PERSISTENT)).unwrap(),
        Generation::V1_1
    );
}

#[test]
fn a_generic_entry_whose_data_is_a_cluster_answers_v1_2() {
    assert_eq!(
        Generation::earliest_for_composition(&parsed(GENERIC_ENTRY_CLUSTER)).unwrap(),
        Generation::V1_2
    );
}

#[test]
fn a_generic_entry_whose_data_is_an_element_answers_v1_2() {
    let mut composition = parsed(GENERIC_ENTRY_CLUSTER);
    composition["content"][0]["data"] = json!({
        "_type": "ELEMENT",
        "name": {"_type": "DV_TEXT", "value": "leaf"},
        "archetype_node_id": "at0001",
        "value": {"_type": "DV_TEXT", "value": "x"}
    });
    assert_eq!(
        Generation::earliest_for_composition(&composition).unwrap(),
        Generation::V1_2
    );
}

/// `ITEM_SINGLE` inherits `ITEM_STRUCTURE`, a `DATA_STRUCTURE`
/// (`org.openehr.rm.data_structures.item_single.adoc`), so it is no `ITEM`
/// and no `ITEM_TREE`: neither generation admits it as `GENERIC_ENTRY.data`.
#[test]
fn a_generic_entry_whose_data_is_an_item_single_decodes_as_no_generation() {
    let mut composition = parsed(GENERIC_ENTRY_CLUSTER);
    composition["content"][0]["data"] = json!({
        "_type": "ITEM_SINGLE",
        "name": {"_type": "DV_TEXT", "value": "data"},
        "archetype_node_id": "at0000",
        "item": {
            "_type": "ELEMENT",
            "name": {"_type": "DV_TEXT", "value": "leaf"},
            "archetype_node_id": "at0001",
            "value": {"_type": "DV_TEXT", "value": "x"}
        }
    });
    assert!(Generation::earliest_for_composition(&composition).is_err());
}

#[test]
fn a_generic_entry_whose_data_is_an_item_tree_answers_v1_1() {
    let mut composition = parsed(GENERIC_ENTRY_CLUSTER);
    composition["content"][0]["data"] = json!({
        "_type": "ITEM_TREE",
        "name": {"_type": "DV_TEXT", "value": "data"},
        "archetype_node_id": "at0000",
        "items": [{
            "_type": "ELEMENT",
            "name": {"_type": "DV_TEXT", "value": "leaf"},
            "archetype_node_id": "at0001",
            "value": {"_type": "DV_TEXT", "value": "x"}
        }]
    });
    assert_eq!(
        Generation::earliest_for_composition(&composition).unwrap(),
        Generation::V1_1
    );
}

#[test]
fn a_value_no_generation_decodes_is_an_error_naming_every_refusal() {
    let error = Generation::earliest_for_composition(&json!({"_type": "OBSERVATION"}))
        .expect_err("an OBSERVATION is no COMPOSITION");
    let refused: Vec<Generation> = error.rejections().iter().map(|(g, _)| *g).collect();
    assert_eq!(refused, vec![Generation::V1_1, Generation::V1_2]);
    assert!(error.source().is_some(), "the newest refusal is the source");
    assert!(error.to_string().contains("v1_1: "), "{error}");
    assert!(error.to_string().contains("v1_2: "), "{error}");
}
