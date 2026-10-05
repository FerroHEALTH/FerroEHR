// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test fixtures/diagnostics — a malformed vendored schema should fail loudly"
)]
//! The ITS-JSON oracle-age pin (#1697): the fidelity gate's validation oracle
//! (`openehr_its::json::RM_SCHEMA_JSON`, the vendored RM **1.1.0** ITS-JSON
//! all-schema) is a CLOSED schema (`additionalProperties: false` per class)
//! judging RM **1.2.0**-generation output. This module machine-derives the
//! full per-class attribute delta between that schema and the generated RM
//! 1.2.0 model (`openehr_rm::v1_2::model`) on every run and pins it — the XSD-gate
//! precedent (`xml_xsd_validity.rs`): the known 1.1.0↔1.2.0 divergence is
//! ADJUDICATED and visible, and any NEW divergence (a pin bump, an emitter
//! change, a re-vendored schema) fails loud instead of surfacing later as a
//! spurious wire-validation failure.

use std::collections::{BTreeMap, BTreeSet};

/// Attributes the generated RM 1.2.0 model declares that the 1.1.0 schema's
/// CLOSED class definition does not — an instance carrying one is REFUSED by
/// the oracle even though it is correct RM 1.2.0. Format: `CLASS.attribute`.
const MODEL_ONLY: &[&str] = &[
    // RM 1.2.0 addition (RM ehr master04 §EHR: `tags`): the first RM
    // 1.2.0-only attribute; a served EHR carrying tags fails the 1.1.0 oracle.
    "EHR.tags",
    // The RM 1.2.0 development text's misspelled attribute (upstream report
    // #1849 — SPECPUB-6 was applied to BASE's resource copy only); the 1.1.0
    // schema spells `accreditation`.
    "TRANSLATION_DETAILS.accreditaton",
];

/// Properties the 1.1.0 schema declares that the RM 1.2.0 model has dropped —
/// the oracle ACCEPTS shapes that are no longer valid RM 1.2.0. Format:
/// `CLASS.attribute`.
const SCHEMA_ONLY: &[&str] = &[
    // RM 1.2.0 removed PARTY.reverse_relationships (RM demographic master05
    // §PARTY class — the attribute is gone from the 1.2.0 generation); the
    // 1.1.0 schema still declares it on every concrete PARTY subtype.
    "AGENT.reverse_relationships",
    "GROUP.reverse_relationships",
    "ORGANISATION.reverse_relationships",
    "PERSON.reverse_relationships",
    "ROLE.reverse_relationships",
    // RM 1.2.0 removed DV_QUANTITY.property (RM data_types master04
    // §DV_QUANTITY — the 1.1.0-era attribute is gone from the class table).
    "DV_QUANTITY.property",
    // The 1.1.0 schema derives SYNC_EXTRACT(_REQUEST) from LOCATABLE; the RM
    // 1.2.0 sync package no longer does, so the LOCATABLE member set is
    // schema-only there.
    "SYNC_EXTRACT.archetype_details",
    "SYNC_EXTRACT.archetype_node_id",
    "SYNC_EXTRACT.feeder_audit",
    "SYNC_EXTRACT.links",
    "SYNC_EXTRACT.name",
    "SYNC_EXTRACT.uid",
    "SYNC_EXTRACT_REQUEST.archetype_details",
    "SYNC_EXTRACT_REQUEST.archetype_node_id",
    "SYNC_EXTRACT_REQUEST.feeder_audit",
    "SYNC_EXTRACT_REQUEST.links",
    "SYNC_EXTRACT_REQUEST.name",
    "SYNC_EXTRACT_REQUEST.uid",
    // The correctly-spelled 1.1.0 property — the twin of the MODEL_ONLY
    // `accreditaton` pin (upstream report #1849).
    "TRANSLATION_DETAILS.accreditation",
];

/// Schema class definitions with no generated RM 1.2.0 class of that name.
const SCHEMA_ONLY_CLASSES: &[&str] = &[
    // BASE foundation/base-types classes the schema embeds for reference —
    // they live in `openehr-base`, not the RM attribute model, so a name
    // lookup in `openehr_rm::v1_2::model` legitimately misses them. Their per-class
    // property shape is exercised structurally through the RM classes that
    // embed them.
    "ARCHETYPE_HRID",
    "ARRAY",
    "DATE",
    "DATE_TIME",
    "DURATION",
    "INTERVAL",
    "ISO8601_TYPE",
    "LIST",
    "SET",
    "TERMINOLOGY_CODE",
    "TERMINOLOGY_TERM",
    "TIME",
    "URI",
];

fn schema_definitions() -> BTreeMap<String, BTreeSet<String>> {
    let schema: serde_json::Value =
        serde_json::from_str(openehr_its::json::RM_SCHEMA_JSON).expect("vendored schema parses");
    let defs = schema["definitions"].as_object().expect("definitions");
    defs.iter()
        .filter_map(|(name, def)| {
            let props = def.get("properties")?.as_object()?;
            Some((
                name.clone(),
                props.keys().filter(|k| *k != "_type").cloned().collect(),
            ))
        })
        .collect()
}

#[test]
fn the_1_1_0_oracle_delta_is_exactly_the_pinned_set() {
    let defs = schema_definitions();
    let mut model_only = BTreeSet::new();
    let mut schema_only = BTreeSet::new();
    let mut schema_only_classes = BTreeSet::new();

    for (class, props) in &defs {
        let Some(rm) = openehr_rm::v1_2::model::class(class) else {
            schema_only_classes.insert(class.clone());
            continue;
        };
        let model_attrs: BTreeSet<&str> = rm.attributes.iter().map(|a| a.name).collect();
        for attr in &model_attrs {
            if !props.contains(*attr) {
                model_only.insert(format!("{class}.{attr}"));
            }
        }
        for prop in props {
            if !model_attrs.contains(prop.as_str()) {
                schema_only.insert(format!("{class}.{prop}"));
            }
        }
    }

    let pin = |set: &BTreeSet<String>, pinned: &[&str], what: &str| {
        let pinned: BTreeSet<String> = pinned.iter().map(|s| (*s).to_owned()).collect();
        let new: Vec<&String> = set.difference(&pinned).collect();
        let gone: Vec<&String> = pinned.difference(set).collect();
        assert!(
            new.is_empty() && gone.is_empty(),
            "{what} delta drifted from the adjudicated pin (#1697)\n  NEW (adjudicate + pin): {new:#?}\n  GONE (unpin): {gone:#?}"
        );
    };
    pin(
        &model_only,
        MODEL_ONLY,
        "model-only (RM 1.2.0 attribute the CLOSED 1.1.0 oracle refuses)",
    );
    pin(
        &schema_only,
        SCHEMA_ONLY,
        "schema-only (1.1.0 property RM 1.2.0 dropped)",
    );
    pin(
        &schema_only_classes,
        SCHEMA_ONLY_CLASSES,
        "schema-only classes",
    );
}
