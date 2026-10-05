// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Builder-capture + walk tests for the AOM 1.4 `C_ATTRIBUTE.existence`
//! enforcement of a mandatory structural attribute whose node-identified child
//! is dropped by master04 §"Level Removal" compaction (so it never becomes a
//! walkable node the occurrence check could visit): `OBSERVATION.state`,
//! `.protocol`, `HISTORY.summary`, `EVENT.state` (issue #234, exposed by the CNF
//! content chapter). Governing spec: AM AOM1.4
//! `master04-constraint_model_package.adoc` §existence ("indicates whether its
//! target object exists or not, i.e. is mandatory or not").
//!
//! The OPT is synthesised by injecting a mandatory (`existence 1..1`) `state`
//! attribute — a node-identified `ITEM_TREE[at0005]` with no leaf content — into
//! the vendored CNF base OBSERVATION template, then asserting a committed EVENT
//! that omits `state` is rejected while one carrying it is not.

#![expect(clippy::expect_used, reason = "test assertions/diagnostics")]

use std::path::PathBuf;

use openehr_its::opt14;
use openehr_sdt::flat::validation::validate_archetype_conformance;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::ValidationKind;
use serde_json::{Value, json};

fn base_opt() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/templates/dt_identifier_c_string.opt");
    std::fs::read_to_string(path).expect("read base OPT")
}

/// Build the `WebTemplate` for an OBSERVATION whose EVENT mandates a `state`
/// (`existence 1..1`) constrained by a content-less `ITEM_TREE[at0005]`.
fn wt_with_mandatory_event_state() -> WebTemplate {
    let interval = "<lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper>";
    let state_attr = format!(
        "<attributes xsi:type=\"C_SINGLE_ATTRIBUTE\"><rm_attribute_name>state</rm_attribute_name><existence>{interval}</existence><children xsi:type=\"C_COMPLEX_OBJECT\"><rm_type_name>ITEM_TREE</rm_type_name><occurrences>{interval}</occurrences><node_id>at0005</node_id></children></attributes>"
    );
    // Inject the state attribute right after the EVENT node's id.
    let xml = base_opt().replace(
        "<node_id>at0002</node_id>",
        &format!("<node_id>at0002</node_id>{state_attr}"),
    );
    let opt = opt14::from_xml(&xml).expect("parse injected OPT");
    build_web_template(&opt).expect("build web template")
}

fn observation(state: Option<Value>) -> Value {
    let mut event = json!({
        "_type": "POINT_EVENT", "archetype_node_id": "at0002",
        "name": {"_type": "DV_TEXT", "value": "Any event"},
        "data": {
            "_type": "ITEM_TREE", "archetype_node_id": "at0003",
            "name": {"_type": "DV_TEXT", "value": "Tree"}, "items": []
        }
    });
    if let Some(state) = state
        && let Value::Object(m) = &mut event
    {
        m.insert("state".to_owned(), state);
    }
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.minimal.v1",
        "name": {"_type": "DV_TEXT", "value": "Minimal"},
        "content": [{
            "_type": "OBSERVATION",
            "archetype_node_id": "openEHR-EHR-OBSERVATION.minimal.v1",
            "name": {"_type": "DV_TEXT", "value": "Minimal"},
            "data": {
                "_type": "HISTORY", "archetype_node_id": "at0001",
                "name": {"_type": "DV_TEXT", "value": "Event Series"},
                "events": [event]
            }
        }]
    })
}

#[test]
fn mandatory_event_state_absence_is_rejected() {
    let wt = wt_with_mandatory_event_state();
    // state omitted → a Required violation referencing `state`.
    let absent = validate_archetype_conformance(&observation(None), &wt);
    assert!(
        absent
            .iter()
            .any(|m| m.kind == ValidationKind::Required && m.path.contains("state")),
        "an EVENT mandating state (existence 1..1) must reject an instance omitting it: {absent:?}"
    );
    // state present → no `state` Required violation.
    let present = observation(Some(json!({
        "_type": "ITEM_TREE", "archetype_node_id": "at0005",
        "name": {"_type": "DV_TEXT", "value": "State"}, "items": []
    })));
    let msgs = validate_archetype_conformance(&present, &wt);
    assert!(
        !msgs
            .iter()
            .any(|m| m.kind == ValidationKind::Required && m.path.contains("state")),
        "a committed state must satisfy the existence constraint: {msgs:?}"
    );
}
