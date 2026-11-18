// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The typed released-generation check (`openehr_rm::v1_2::released`) gives
//! the released strict reader's answer on every composition of the corpus and
//! on variants the released model cannot hold (#6002).

#![expect(
    clippy::expect_used,
    reason = "fixture plumbing outside #[test] fns: a refused fixture is a test failure"
)]

use openehr_rm::prelude::Composition;
use openehr_rm::v1_2::released::expressible_in_released;
use serde_json::{Value, json};

fn compositions() -> Vec<(String, Value)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = [
        "corpus",
        "crates/openehr-its/tests",
        "docs/specs/openehr/CNF",
    ]
    .iter()
    .map(|d| root.join(d))
    .collect();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("corpus dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "json")
                && let Ok(text) = std::fs::read_to_string(&path)
                && text.contains("\"COMPOSITION\"")
                && let Ok(v) = serde_json::from_str::<Value>(&text)
                && v.get("_type").and_then(Value::as_str) == Some("COMPOSITION")
            {
                out.push((path.display().to_string(), v));
            }
        }
    }
    out
}

/// A `GENERIC_ENTRY` whose `data` is a `CLUSTER`: an `ITEM` in RM 1.2.0, where
/// RM 1.1.0 declares `ITEM_TREE`.
fn generic_entry_over_a_cluster() -> Value {
    json!({
        "_type": "GENERIC_ENTRY",
        "name": {"_type": "DV_TEXT", "value": "generic"},
        "archetype_node_id": "openEHR-EHR-GENERIC_ENTRY.adhoc.v1",
        "data": {
            "_type": "CLUSTER",
            "name": {"_type": "DV_TEXT", "value": "data"},
            "archetype_node_id": "at0001",
            "items": [{
                "_type": "ELEMENT",
                "name": {"_type": "DV_TEXT", "value": "e"},
                "archetype_node_id": "at0002",
                "value": {"_type": "DV_BOOLEAN", "value": true}
            }]
        }
    })
}

#[test]
fn the_typed_check_answers_as_the_released_reader() {
    let docs = compositions();
    assert!(docs.len() > 150, "found {} compositions", docs.len());
    let mut compared = 0;
    let mut inexpressible = 0;
    for (path, doc) in &docs {
        let mut variants = vec![doc.clone()];
        let mut with_generic = doc.clone();
        if let Some(Value::Array(content)) = with_generic.get_mut("content") {
            content.push(generic_entry_over_a_cluster());
            variants.push(with_generic);
        }
        for variant in variants {
            let Ok(typed) = openehr_its::json::from_canonical_value::<Composition>(&variant) else {
                continue;
            };
            let canonical = openehr_its::json::to_canonical_value(&typed);
            let reader = openehr_its::json::from_canonical_value::<
                openehr_rm::v1_1::composition::composition::Composition,
            >(&canonical)
            .is_ok();
            let typed_answer = expressible_in_released(&typed);
            assert_eq!(typed_answer, reader, "{path}");
            compared += 1;
            if !reader {
                inexpressible += 1;
            }
        }
    }
    assert!(compared > 300, "compared {compared}");
    assert!(
        inexpressible > 100,
        "only {inexpressible} documents were inexpressible"
    );
}

/// Every other resource kind a commit stamps, read the same way.
#[test]
fn the_typed_check_answers_as_the_released_reader_for_every_kind() {
    use openehr_rm::v1_1 as old;
    use openehr_rm::v1_2 as new;
    fn agree<New, Old>(doc: &Value) -> Option<bool>
    where
        New: serde::de::DeserializeOwned
            + serde::Serialize
            + openehr_rm::v1_2::released::Expressible,
        Old: serde::de::DeserializeOwned,
    {
        let typed = openehr_its::json::from_canonical_value::<New>(doc).ok()?;
        let canonical = openehr_its::json::to_canonical_value(&typed);
        let reader = openehr_its::json::from_canonical_value::<Old>(&canonical).is_ok();
        Some(expressible_in_released(&typed) == reader)
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut stack: Vec<std::path::PathBuf> = [
        "corpus",
        "crates/openehr-its/tests",
        "docs/specs/openehr/CNF",
    ]
    .iter()
    .map(|d| root.join(d))
    .collect();
    let mut compared = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("corpus dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(doc) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let verdict = match doc.get("_type").and_then(Value::as_str) {
                Some("EHR_STATUS") => {
                    agree::<new::ehr::ehr_status::EhrStatus, old::ehr::ehr_status::EhrStatus>(&doc)
                }
                Some("EHR_ACCESS") => {
                    agree::<new::ehr::ehr_access::EhrAccess, old::ehr::ehr_access::EhrAccess>(&doc)
                }
                Some("FOLDER") => agree::<
                    new::common::directory::folder::Folder,
                    old::common::directory::folder::Folder,
                >(&doc),
                Some("PERSON") => agree::<
                    new::demographic::person::Person,
                    old::demographic::person::Person,
                >(&doc),
                Some("ORGANISATION") => agree::<
                    new::demographic::organisation::Organisation,
                    old::demographic::organisation::Organisation,
                >(&doc),
                Some("GROUP") => {
                    agree::<new::demographic::group::Group, old::demographic::group::Group>(&doc)
                }
                Some("AGENT") => {
                    agree::<new::demographic::agent::Agent, old::demographic::agent::Agent>(&doc)
                }
                Some("ROLE") => {
                    agree::<new::demographic::role::Role, old::demographic::role::Role>(&doc)
                }
                Some("PARTY_RELATIONSHIP") => agree::<
                    new::demographic::party_relationship::PartyRelationship,
                    old::demographic::party_relationship::PartyRelationship,
                >(&doc),
                _ => None,
            };
            if let Some(same) = verdict {
                assert!(same, "{}", path.display());
                compared += 1;
            }
        }
    }
    assert!(compared > 20, "compared {compared}");
}
