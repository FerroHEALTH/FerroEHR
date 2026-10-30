// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Flat documents of the Simplified Formats mapping tables (ITS-REST
//! `docs/simplified_formats/master05-rm_mapping.adoc`) and context defaults
//! (`master06-context_information.adoc`), on the CNF Robot `test_all_types`
//! and `minimal_action` templates: `ACTIVITY/action_archetype_id`,
//! `ctx/action_ism_transition_current_state`, `ctx/instruction_narrative` and
//! the `DV_INTERVAL` `|lower_unbounded` and `|upper_unbounded` flags.
#![expect(clippy::panic, reason = "test fixture plumbing")]

use openehr_its::opt14;
use openehr_sdt::flat::convert::{composition_from_flat, composition_to_flat};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::{Map, Value, json};

const TEMPLATES: &str =
    "../../docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets/valid_templates";
const NOW: &str = "2024-01-01T00:00:00Z";
const INSTRUCTION: &str = "test_all_types/test_all_types_2:0/section_2/section_3/test_all_types:0";
const INTERVAL: &str = "test_all_types/test_all_types_1:0/interval_quantity";

fn web_template(file: &str) -> WebTemplate {
    let opt_xml = std::fs::read_to_string(format!("{TEMPLATES}/{file}"))
        .unwrap_or_else(|e| panic!("read {file}: {e}"));
    let opt = opt14::from_xml(&opt_xml).unwrap_or_else(|e| panic!("parse {file}: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build WT {file}: {e}"))
}

fn flat(pairs: &[(&str, Value)]) -> Map<String, Value> {
    let mut map = Map::new();
    for (key, value) in [
        ("ctx/language", json!("en")),
        ("ctx/territory", json!("US")),
        ("ctx/composer_name", json!("CNF Composer")),
    ] {
        map.insert(key.to_owned(), value);
    }
    for (key, value) in pairs {
        map.insert((*key).to_owned(), value.clone());
    }
    map
}

/// Every object in `value` whose `_type` is `rm_type`, depth first.
fn objects_of<'a>(value: &'a Value, rm_type: &str, out: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some(rm_type) {
                out.push(value);
            }
            for child in map.values() {
                objects_of(child, rm_type, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                objects_of(child, rm_type, out);
            }
        }
        _ => {}
    }
}

fn the_only<'a>(composition: &'a Value, rm_type: &str) -> &'a Value {
    let mut found = Vec::new();
    objects_of(composition, rm_type, &mut found);
    match found.as_slice() {
        [only] => only,
        other => panic!(
            "expected one {rm_type}, found {}: {composition}",
            other.len()
        ),
    }
}

/// master05 §ACTIVITY: `/action_archetype_id` is a `STRING` mapped to the
/// String attribute `ACTIVITY.action_archetype_id`.
#[test]
fn activity_action_archetype_id_is_the_rm_string() {
    let wt = web_template("all_types/Test_all_types.opt");
    let doc = flat(&[
        (
            &format!("{INSTRUCTION}/narrative"),
            json!("Human readable instruction narrative"),
        ),
        (
            &format!("{INSTRUCTION}/current_activity/action_archetype_id"),
            json!("openEHR-EHR-ACTION.test_all_types.v1"),
        ),
        (
            &format!("{INSTRUCTION}/current_activity/partial_date"),
            json!("2019-03"),
        ),
    ]);
    let composition =
        composition_from_flat(&doc, &wt, NOW).unwrap_or_else(|e| panic!("convert: {e}"));
    let activity = the_only(&composition, "ACTIVITY");
    assert_eq!(
        activity["action_archetype_id"],
        json!("openEHR-EHR-ACTION.test_all_types.v1"),
        "{activity}"
    );
    let back = composition_to_flat(&composition, &wt).unwrap_or_else(|e| panic!("flatten: {e}"));
    assert_eq!(
        back.get(&format!(
            "{INSTRUCTION}/current_activity/action_archetype_id"
        )),
        Some(&json!("openEHR-EHR-ACTION.test_all_types.v1")),
        "the flat output carries the string"
    );

    let doc = flat(&[(
        &format!("{INSTRUCTION}/current_activity/partial_date"),
        json!("2019-03"),
    )]);
    let composition = composition_from_flat(&doc, &wt, NOW)
        .unwrap_or_else(|e| panic!("convert without the id: {e}"));
    assert_eq!(
        the_only(&composition, "ACTIVITY")["action_archetype_id"],
        json!("/.*/"),
        "an absent id is the match-all form"
    );
}

/// master06 §`action_ism_transition_current_state`: "set the default for
/// `ACTION.ism_transition.current_state`", "either value or code is
/// accepted"; both forms name `completed` (532).
#[test]
fn ctx_action_ism_transition_current_state_sets_the_state() {
    let wt = web_template("minimal/minimal_action.opt");
    for given in ["completed", "532"] {
        let doc = flat(&[
            ("ctx/action_ism_transition_current_state", json!(given)),
            (
                "minimal/minimal:0/multimedia",
                json!("http://med.tube.com/sample"),
            ),
            (
                "minimal/minimal:0/multimedia|mediatype",
                json!("application/pdf"),
            ),
            ("minimal/minimal:0/multimedia|size", json!(504_903_212)),
            (
                "minimal/minimal:0/time",
                json!("2019-03-22T22:26:01.127+01:00"),
            ),
        ]);
        let composition = composition_from_flat(&doc, &wt, NOW)
            .unwrap_or_else(|e| panic!("convert ({given}): {e}"));
        let action = the_only(&composition, "ACTION");
        assert_eq!(
            action["ism_transition"]["current_state"]["defining_code"]["code_string"],
            json!("532"),
            "{given}: {action}"
        );
    }
}

/// master06 §`instruction_narrative`: "set the default for
/// `INSTRUCTION.narrative`".
#[test]
fn ctx_instruction_narrative_sets_the_narrative() {
    let wt = web_template("all_types/Test_all_types.opt");
    let doc = flat(&[
        (
            "ctx/instruction_narrative",
            json!("Human readable instruction narrative"),
        ),
        (
            &format!("{INSTRUCTION}/_wf_definition|value"),
            json!("wf_definition"),
        ),
        (
            &format!("{INSTRUCTION}/_wf_definition|formalism"),
            json!("formalism"),
        ),
    ]);
    let composition =
        composition_from_flat(&doc, &wt, NOW).unwrap_or_else(|e| panic!("convert: {e}"));
    let instruction = the_only(&composition, "INSTRUCTION");
    assert_eq!(
        instruction["narrative"]["value"],
        json!("Human readable instruction narrative"),
        "{instruction}"
    );
}

/// master05 §`DV_INTERVAL`: `|lower_unbounded` and `|upper_unbounded` are
/// BOOLEAN flags, "defaults to false. only in output if true".
#[test]
fn dv_interval_unbounded_flags_round_trip() {
    let wt = web_template("all_types/Test_all_types.opt");
    let cases = [
        (
            "upper",
            vec![
                (format!("{INTERVAL}/lower|magnitude"), json!(65.42)),
                (format!("{INTERVAL}/lower|unit"), json!("mg")),
                (format!("{INTERVAL}|upper_unbounded"), json!(true)),
                (format!("{INTERVAL}|upper_included"), json!(false)),
            ],
        ),
        (
            "lower",
            vec![
                (format!("{INTERVAL}/upper|magnitude"), json!(65.42)),
                (format!("{INTERVAL}/upper|unit"), json!("mg")),
                (format!("{INTERVAL}|lower_unbounded"), json!(true)),
                (format!("{INTERVAL}|lower_included"), json!(false)),
            ],
        ),
    ];
    for (side, pairs) in cases {
        let pairs: Vec<(&str, Value)> =
            pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let composition = composition_from_flat(&flat(&pairs), &wt, NOW)
            .unwrap_or_else(|e| panic!("convert ({side}): {e}"));
        let interval = the_only(&composition, "DV_INTERVAL");
        assert_eq!(
            interval[format!("{side}_unbounded")],
            json!(true),
            "{interval}"
        );
        let other = if side == "upper" { "lower" } else { "upper" };
        assert_eq!(
            interval[format!("{other}_unbounded")],
            json!(false),
            "{interval}"
        );
        assert!(
            interval.get(side).is_none(),
            "an unbounded side has no limit: {interval}"
        );

        let back =
            composition_to_flat(&composition, &wt).unwrap_or_else(|e| panic!("flatten: {e}"));
        assert_eq!(
            back.get(&format!("{INTERVAL}|{side}_unbounded")),
            Some(&json!(true)),
            "written when true"
        );
        assert!(
            !back.contains_key(&format!("{INTERVAL}|{other}_unbounded")),
            "only in output if true"
        );
    }
}
