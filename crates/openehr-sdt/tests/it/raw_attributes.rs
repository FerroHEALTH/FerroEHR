// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `|raw` on the attributes no template node holds (master04 §Raw canonical
//! JSON): the COMPOSITION `language`, `territory` and `composer`, and the
//! `_feeder_audit`, `_link:i`, `_participation:i`, `_other_participation:i`,
//! `_provider` and `_health_care_facility` families.
//!
//! Each value is taken whole, survives a FLAT round trip, and a value of the
//! wrong class or one beside another suffix is refused. The carrier is the
//! CNF Robot `minimal_action.en.v1` template.
#![expect(clippy::panic, reason = "test fixture plumbing")]

use openehr_its::opt14;
use openehr_sdt::flat::convert::{composition_from_flat, composition_to_flat};
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::{Map, Value, json};

const CNF_CORPUS: &str =
    "../../docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets";
const NOW: &str = "2024-01-01T00:00:00Z";
const ENTRY: &str = "minimal/minimal:0";

fn web_template() -> WebTemplate {
    let opt_xml = std::fs::read_to_string(format!(
        "{CNF_CORPUS}/valid_templates/minimal/minimal_action.opt"
    ))
    .unwrap_or_else(|e| panic!("read minimal_action OPT: {e}"));
    let opt = opt14::from_xml(&opt_xml).unwrap_or_else(|e| panic!("parse OPT: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build WT: {e}"))
}

/// The ACTION's own data, without any `ctx/` key.
fn action_flat() -> Map<String, Value> {
    let mut flat = Map::new();
    flat.insert(
        format!("{ENTRY}/ism_transition/current_state|code"),
        json!("532"),
    );
    flat.insert(
        format!("{ENTRY}/ism_transition/current_state|value"),
        json!("completed"),
    );
    flat.insert(
        format!("{ENTRY}/ism_transition/current_state|terminology"),
        json!("openehr"),
    );
    flat.insert(format!("{ENTRY}/time"), json!("2019-03-22T22:26:01+01:00"));
    flat
}

fn code_phrase(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
        "code_string": code,
    })
}

fn party_identified(name: &str, id: &str) -> Value {
    party_of("PERSON", name, id)
}

fn party_of(party_type: &str, name: &str, id: &str) -> Value {
    json!({
        "_type": "PARTY_IDENTIFIED",
        "external_ref": {
            "_type": "PARTY_REF",
            "id": {"_type": "GENERIC_ID", "value": id, "scheme": "local"},
            "namespace": "demographic",
            "type": party_type,
        },
        "name": name,
    })
}

fn participation(function: &str, name: &str) -> Value {
    json!({
        "_type": "PARTICIPATION",
        "function": {"_type": "DV_TEXT", "value": function},
        "performer": party_identified(name, "p-1"),
    })
}

fn feeder_audit() -> Value {
    json!({
        "_type": "FEEDER_AUDIT",
        "originating_system_audit": {
            "_type": "FEEDER_AUDIT_DETAILS",
            "system_id": "lab-system",
        },
    })
}

fn link() -> Value {
    json!({
        "_type": "LINK",
        "meaning": {"_type": "DV_TEXT", "value": "follow up"},
        "type": {"_type": "DV_TEXT", "value": "issue"},
        "target": {"_type": "DV_EHR_URI", "value": "ehr:/compositions/123"},
    })
}

fn decodes(rm: &Value) {
    openehr_its::json::from_canonical_value::<openehr_rm::prelude::Composition>(rm)
        .unwrap_or_else(|e| panic!("the built composition decodes: {e}\n{rm}"));
}

/// Builds `flat`, flattens the result and builds that again, returning both
/// compositions.
fn round_trip(flat: &Map<String, Value>, wt: &WebTemplate) -> (Value, Value) {
    let first = composition_from_flat(flat, wt, NOW).unwrap_or_else(|e| panic!("builds: {e}"));
    let again = composition_to_flat(&first, wt).unwrap_or_else(|e| panic!("flattens: {e}"));
    let second =
        composition_from_flat(&again, wt, NOW).unwrap_or_else(|e| panic!("builds again: {e}"));
    (first, second)
}

#[test]
fn the_composition_language_territory_and_composer_are_taken_whole() {
    let wt = web_template();
    let mut flat = action_flat();
    let composer = party_identified("Dr. Marcus Johnson", "c-7");
    flat.insert(
        "minimal/language|raw".to_owned(),
        code_phrase("ISO_639-1", "nl"),
    );
    flat.insert(
        "minimal/territory|raw".to_owned(),
        code_phrase("ISO_3166-1", "NL"),
    );
    flat.insert("minimal/composer|raw".to_owned(), composer.clone());
    let (rm, again) = round_trip(&flat, &wt);
    decodes(&rm);
    assert_eq!(rm["language"], code_phrase("ISO_639-1", "nl"));
    assert_eq!(rm["territory"], code_phrase("ISO_3166-1", "NL"));
    assert_eq!(rm["composer"], composer);
    assert_eq!(
        rm.pointer("/content/0/language"),
        Some(&code_phrase("ISO_639-1", "nl")),
        "the entry language defaults from the composition's: {rm}"
    );
    for attribute in ["language", "territory", "composer"] {
        assert_eq!(rm[attribute], again[attribute], "{attribute} round-trips");
    }
}

/// An explicit `ctx/` key wins over a path spelling of the same attribute,
/// the rule `|code` already follows.
#[test]
fn a_ctx_language_wins_over_a_raw_language() {
    let wt = web_template();
    let mut flat = action_flat();
    flat.insert("ctx/language".to_owned(), json!("en"));
    flat.insert("ctx/territory".to_owned(), json!("US"));
    flat.insert("ctx/composer_name".to_owned(), json!("Composer"));
    flat.insert(
        "minimal/language|raw".to_owned(),
        code_phrase("ISO_639-1", "nl"),
    );
    let rm = composition_from_flat(&flat, &wt, NOW).unwrap_or_else(|e| panic!("builds: {e}"));
    assert_eq!(rm["language"], code_phrase("ISO_639-1", "en"));
}

#[test]
fn a_raw_territory_of_another_class_is_refused() {
    let wt = web_template();
    let mut flat = action_flat();
    flat.insert("ctx/language".to_owned(), json!("en"));
    flat.insert(
        "minimal/territory|raw".to_owned(),
        json!({"_type": "DV_TEXT", "value": "NL"}),
    );
    match composition_from_flat(&flat, &wt, NOW) {
        Err(FlatError::InvalidRaw { path, .. }) => assert_eq!(path, "minimal/territory"),
        other => panic!("expected InvalidRaw, got {other:?}"),
    }
}

#[test]
fn the_underscore_families_are_taken_whole() {
    let wt = web_template();
    let mut flat = action_flat();
    flat.insert("ctx/language".to_owned(), json!("en"));
    flat.insert("ctx/territory".to_owned(), json!("US"));
    flat.insert("ctx/composer_name".to_owned(), json!("Composer"));
    let provider = party_identified("Provider", "pr-1");
    let facility = party_of("ORGANISATION", "Hospital", "org-1");
    let entries = [
        (format!("{ENTRY}/_provider|raw"), provider.clone()),
        (format!("{ENTRY}/_feeder_audit|raw"), feeder_audit()),
        (format!("{ENTRY}/_link:0|raw"), link()),
        (
            format!("{ENTRY}/_other_participation:0|raw"),
            participation("witness", "Witness"),
        ),
        (
            "minimal/context/_participation:0|raw".to_owned(),
            participation("requester", "Requester"),
        ),
        (
            "minimal/context/_health_care_facility|raw".to_owned(),
            facility.clone(),
        ),
    ];
    for (key, value) in entries {
        flat.insert(key, value);
    }
    let (rm, again) = round_trip(&flat, &wt);
    decodes(&rm);
    let expected = [
        ("/content/0/provider", provider),
        ("/content/0/feeder_audit", feeder_audit()),
        ("/content/0/links/0", link()),
        (
            "/content/0/other_participations/0",
            participation("witness", "Witness"),
        ),
        (
            "/context/participations/0",
            participation("requester", "Requester"),
        ),
        ("/context/health_care_facility", facility),
    ];
    for (pointer, value) in expected {
        assert_eq!(rm.pointer(pointer), Some(&value), "{pointer} in {rm}");
        assert_eq!(
            again.pointer(pointer),
            Some(&value),
            "{pointer} round-trips"
        );
    }
}

#[test]
fn a_raw_family_beside_a_suffix_is_refused() {
    let wt = web_template();
    let mut flat = action_flat();
    flat.insert("ctx/language".to_owned(), json!("en"));
    flat.insert(
        format!("{ENTRY}/_provider|raw"),
        party_identified("Provider", "pr-1"),
    );
    flat.insert(format!("{ENTRY}/_provider|name"), json!("Another"));
    match composition_from_flat(&flat, &wt, NOW) {
        Err(FlatError::InvalidRaw { path, .. }) => assert_eq!(path, format!("{ENTRY}/_provider")),
        other => panic!("expected InvalidRaw, got {other:?}"),
    }
}

#[test]
fn a_raw_participation_of_another_class_is_refused() {
    let wt = web_template();
    let mut flat = action_flat();
    flat.insert("ctx/language".to_owned(), json!("en"));
    flat.insert(
        "minimal/context/_participation:0|raw".to_owned(),
        party_identified("Requester", "r-1"),
    );
    match composition_from_flat(&flat, &wt, NOW) {
        Err(FlatError::InvalidRaw { path, .. }) => {
            assert_eq!(path, "minimal/context:0/_participation:0");
        }
        other => panic!("expected InvalidRaw, got {other:?}"),
    }
}
