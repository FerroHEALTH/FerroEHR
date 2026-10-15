// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The JSON serialisation of a `P_BMM` schema reads to the same `P_BMM_SCHEMA`
//! value as its ODIN serialisation.
//!
//! `docs/specs/openehr/LANG/docs/bmm_persistence/master04-syntax.adoc`
//! §Serialisation Formats: "Whatever the format, the same serialisation rules
//! apply". The vendored component schemas publish both serialisations of one
//! release, so every pair is read both ways and compared; a hand-written pair
//! pins the stamping of `P_BMM_CLASS.uid` and `source_schema_id`, the interval
//! and positional-list mappings, and the JSON-only refusals.

#![expect(
    clippy::panic,
    clippy::panic_in_result_fn,
    reason = "test diagnostics: a fixture that does not read, or an assertion, is how these tests fail"
)]

use std::path::PathBuf;

use openehr_lang::v1_1::bmm_persistence::error::PBmmReadError;
use openehr_lang::v1_1::bmm_persistence::json_reader::read_schema_json;
use openehr_lang::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use openehr_lang::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use openehr_lang::v1_1::bmm_persistence::reader::read_schema;

/// The vendored component tree `openehr-codegen` reads.
fn components() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/openehr-codegen/vendor/bmm/components")
}

/// Every vendored component json BMM, relative to [`components`].
const JSON_SCHEMAS: &[&str] = &[
    "AM/json/openehr_am_1.4.0.bmm.json",
    "AM/json/openehr_am_2.2.0.bmm.json",
    "AM/json/openehr_am_2.3.0.bmm.json",
    "AM/json/openehr_am_2.4.0.bmm.json",
    "BASE/json/openehr_base_1.0.4.bmm.json",
    "BASE/json/openehr_base_1.1.0.bmm.json",
    "BASE/json/openehr_base_1.2.0.bmm.json",
    "BASE/json/openehr_base_1.3.0.bmm.json",
    "LANG/json/openehr_lang_1.0.0.bmm.json",
    "LANG/json/openehr_lang_1.1.0-bmm3.bmm.json",
    "LANG/json/openehr_lang_1.1.0.bmm.json",
    "RM/json/openehr_rm_1.0.2.bmm.json",
    "RM/json/openehr_rm_1.0.3.bmm.json",
    "RM/json/openehr_rm_1.0.4.bmm.json",
    "RM/json/openehr_rm_1.1.0.bmm.json",
    "RM/json/openehr_rm_1.2.0.bmm.json",
    "TERM/json/openehr_term_3.0.0.bmm.json",
    "TERM/json/openehr_term_3.1.0.bmm.json",
];

/// The vendored json schemas the strict reader refuses, each with the element
/// its message names.
///
/// AM 2.4.0's json gives the `P_BMM_GENERIC_PROPERTY` `value_sets` a
/// `cardinality`, which only `P_BMM_CONTAINER_PROPERTY` declares
/// (`LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_generic_property.adoc`
/// §Attributes); its ODIN twin carries none.
const REFUSED_JSON: &[(&str, &str)] = &[(
    "AM/json/openehr_am_2.4.0.bmm.json",
    "ARCHETYPE_TERMINOLOGY/properties/value_sets: attribute `cardinality`",
)];

/// The adjudicated refusal of `file`, when [`REFUSED_JSON`] names it.
fn refused(file: &str) -> Option<&'static str> {
    REFUSED_JSON
        .iter()
        .find(|(refused, _)| *refused == file)
        .map(|(_, detail)| *detail)
}

/// The classes whose json documentation keeps the leading white space of a
/// multi-line string's continuation lines, which the ODIN reading removes.
///
/// LANG `odin/master07-leaf_data.adoc` §Strings: the contents of a multi-line
/// ODIN string are computed "with the removal of white space leaders up to the
/// left-most character of the first line of the string". JSON has no such rule,
/// so the published json carries the leaders the ODIN text drops.
const DEDENTED_IN_ODIN: &[(&str, &str)] = &[(
    "AM/json/openehr_am_2.3.0.bmm.json",
    "C_TEMPORAL_DEFINITIONS",
)];

/// `debug` with the white space after every escaped newline removed.
fn without_leaders(debug: &str) -> String {
    let mut out = String::with_capacity(debug.len());
    let mut rest = debug;
    while let Some(at) = rest.find("\\n") {
        let (head, tail) = rest.split_at(at + 2);
        out.push_str(head);
        rest = tail.trim_start_matches(' ');
    }
    out.push_str(rest);
    out
}

/// Reads `file` under [`components`] as text.
fn source(file: &str) -> String {
    let path = components().join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Compares two readings class by class before the whole value, so a
/// difference names the class it is in.
fn assert_same_reading(label: &str, json: &PBmmSchema, odin: &PBmmSchema) {
    for (list, json_list, odin_list) in [
        (
            "primitive_types",
            &json.primitive_types,
            &odin.primitive_types,
        ),
        (
            "class_definitions",
            &json.class_definitions,
            &odin.class_definitions,
        ),
    ] {
        let json_list = json_list.as_deref().unwrap_or_default();
        let odin_list = odin_list.as_deref().unwrap_or_default();
        assert_eq!(
            json_list.iter().map(PBmmClass::name).collect::<Vec<_>>(),
            odin_list.iter().map(PBmmClass::name).collect::<Vec<_>>(),
            "{label}: the {list} differ in membership or order"
        );
        for (json_class, odin_class) in json_list.iter().zip(odin_list) {
            let mut json_text = format!("{json_class:#?}");
            let mut odin_text = format!("{odin_class:#?}");
            if DEDENTED_IN_ODIN.contains(&(label, json_class.name())) {
                json_text = without_leaders(&json_text);
                odin_text = without_leaders(&odin_text);
            }
            assert!(
                json_text == odin_text,
                "{label}: {list}/{} reads differently: {}",
                json_class.name(),
                first_difference(&json_text, &odin_text)
            );
        }
    }
    assert_eq!(
        json.packages, odin.packages,
        "{label}: the package trees differ"
    );
    let headers = |schema: &PBmmSchema| PBmmSchema {
        primitive_types: None,
        class_definitions: None,
        ..schema.clone()
    };
    assert_eq!(
        headers(json),
        headers(odin),
        "{label}: the schema headers differ"
    );
}

/// The first line where two pretty-printed readings part, with the field path
/// leading to it, so a parity failure names the field that differs.
fn first_difference(json: &str, odin: &str) -> String {
    let mut open: Vec<&str> = Vec::new();
    for (json_line, odin_line) in json.lines().zip(odin.lines()) {
        if json_line != odin_line {
            return format!(
                "under {}:\n  json: {json_line}\n  odin: {odin_line}",
                open.join(" > ")
            );
        }
        let trimmed = json_line.trim();
        if trimmed.ends_with('{') || trimmed.ends_with('[') || trimmed.ends_with('(') {
            open.push(trimmed);
        } else if trimmed.starts_with('}') || trimmed.starts_with(']') || trimmed.starts_with(')') {
            open.pop();
        }
    }
    format!(
        "one reading is a prefix of the other ({} vs {} lines)",
        json.lines().count(),
        odin.lines().count()
    )
}

#[test]
fn every_component_schema_published_in_both_serialisations_reads_to_one_value() {
    let mut pairs = 0usize;
    for file in JSON_SCHEMAS {
        let odin_file = file
            .replace("/json/", "/odin/")
            .replace(".bmm.json", ".bmm");
        if !components().join(&odin_file).exists() || refused(file).is_some() {
            continue;
        }
        let json = read_schema_json(&source(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let odin = read_schema(&source(&odin_file)).unwrap_or_else(|e| panic!("{odin_file}: {e}"));
        assert_same_reading(file, &json, &odin);
        pairs += 1;
    }
    // Eighteen json schemas; the ODIN twins of BASE 1.3.0, RM 1.2.0 and
    // TERM 3.1.0 are not vendored, since only their json carries the release,
    // and AM 2.4.0's json does not read (`REFUSED_JSON`).
    assert_eq!(pairs, 14);
}

#[test]
fn every_component_json_schema_reads() {
    for file in JSON_SCHEMAS {
        let read = read_schema_json(&source(file));
        if let Some(detail) = refused(file) {
            let Err(error @ PBmmReadError::UnknownAttribute { .. }) = read else {
                panic!("{file}: expected the adjudicated refusal, got {read:?}");
            };
            let message = error.to_string();
            assert!(message.contains(detail), "{file}: {message}");
            continue;
        }
        let schema = read.unwrap_or_else(|e| panic!("{file}: {e}"));
        assert!(
            schema.class_definitions.is_some(),
            "{file}: no class definitions were read"
        );
    }
}

/// A small schema in ODIN, exercising both class lists, an interface, a
/// cardinality range and a positional generic-parameter list.
const ODIN_SCHEMA: &str = r#"
bmm_version = <"2.4">
rm_publisher = <"test">
schema_name = <"json_reader">
rm_release = <"1.0.0">
packages = <
    ["org.test"] = <
        name = <"org.test">
        classes = <"Any", "Integer", "WHOLE", "ACCESS">
    >
>
primitive_types = <
    ["Any"] = <
        name = <"Any">
        is_abstract = <True>
    >
    ["Integer"] = <
        name = <"Integer">
        ancestors = <"Any">
    >
>
class_definitions = <
    ["ACCESS"] = (P_BMM_INTERFACE) <
        name = <"ACCESS">
        functions = <
            ["count"] = <
                name = <"count">
                result = (P_BMM_SIMPLE_TYPE) <
                    type = <"Integer">
                >
            >
        >
    >
    ["WHOLE"] = <
        name = <"WHOLE">
        ancestors = <"Any">
        invariants = <
            ["Parts_valid"] = <"not parts.is_empty">
        >
        properties = <
            ["parts"] = (P_BMM_CONTAINER_PROPERTY) <
                name = <"parts">
                type_def = <
                    container_type = <"List">
                    type = <"Integer">
                >
                cardinality = <|>=1|>
            >
            ["table"] = (P_BMM_GENERIC_PROPERTY) <
                name = <"table">
                type_def = <
                    root_type = <"Hash">
                    generic_parameters = <
                        [1] = <"Integer">
                        [2] = (P_BMM_GENERIC_TYPE) <
                            root_type = <"List">
                            generic_parameters = <"Integer">
                        >
                    >
                >
            >
            ["size"] = (P_BMM_SINGLE_PROPERTY) <
                name = <"size">
                type = <"Integer">
                is_mandatory = <True>
            >
        >
    >
>
"#;

/// [`ODIN_SCHEMA`] in JSON, as `bmm-publisher` writes it.
const JSON_SCHEMA: &str = r#"{
    "bmm_version": "2.4",
    "rm_publisher": "test",
    "schema_name": "json_reader",
    "rm_release": "1.0.0",
    "packages": {
        "org.test": {"name": "org.test", "classes": ["Any", "Integer", "WHOLE", "ACCESS"]}
    },
    "primitive_types": {
        "Any": {"name": "Any", "is_abstract": true},
        "Integer": {"name": "Integer", "ancestors": ["Any"]}
    },
    "class_definitions": {
        "ACCESS": {
            "_type": "P_BMM_INTERFACE",
            "name": "ACCESS",
            "functions": {
                "count": {"name": "count", "result": {"_type": "P_BMM_SIMPLE_TYPE", "type": "Integer"}}
            }
        },
        "WHOLE": {
            "name": "WHOLE",
            "ancestors": ["Any"],
            "invariants": {"Parts_valid": "not parts.is_empty"},
            "properties": {
                "parts": {
                    "_type": "P_BMM_CONTAINER_PROPERTY",
                    "name": "parts",
                    "type_def": {"container_type": "List", "type": "Integer"},
                    "cardinality": {"lower": 1, "upper_unbounded": true}
                },
                "table": {
                    "_type": "P_BMM_GENERIC_PROPERTY",
                    "name": "table",
                    "type_def": {
                        "root_type": "Hash",
                        "generic_parameters": [
                            "Integer",
                            {"_type": "P_BMM_GENERIC_TYPE", "root_type": "List", "generic_parameters": ["Integer"]}
                        ]
                    }
                },
                "size": {"_type": "P_BMM_SINGLE_PROPERTY", "name": "size", "type": "Integer", "is_mandatory": true}
            }
        }
    }
}"#;

/// The class named `name` in either class list of `schema`.
fn class<'a>(schema: &'a PBmmSchema, name: &str) -> &'a PBmmClass {
    schema
        .primitive_types
        .iter()
        .flatten()
        .chain(schema.class_definitions.iter().flatten())
        .find(|class| class.name() == name)
        .unwrap_or_else(|| panic!("{name} was not read"))
}

#[test]
fn the_json_form_reads_to_the_odin_value() -> Result<(), PBmmReadError> {
    let json = read_schema_json(JSON_SCHEMA)?;
    let odin = read_schema(ODIN_SCHEMA)?;
    assert_same_reading("json_reader", &json, &odin);
    Ok(())
}

#[test]
fn the_json_reader_stamps_uid_and_source_schema_id_in_document_order() -> Result<(), PBmmReadError>
{
    let schema = read_schema_json(JSON_SCHEMA)?;
    let id = schema.schema_id();
    // Numbering runs over every entry in document order; the interface keeps its
    // position (3) but carries no uid.
    for (name, uid) in [("Any", 1), ("Integer", 2), ("WHOLE", 4)] {
        let class = class(&schema, name);
        assert_eq!(class.uid(), Some(uid), "{name}: uid");
        assert_eq!(
            class.source_schema_id(),
            Some(id.as_str()),
            "{name}: source_schema_id"
        );
    }
    // A P_BMM_INTERFACE declares neither attribute (p_bmm_interface.adoc §Attributes).
    let access = class(&schema, "ACCESS");
    assert_eq!(access.uid(), None);
    assert_eq!(access.source_schema_id(), None);
    Ok(())
}

/// [`JSON_SCHEMA`] with `from` replaced by `to`, asserting the edit applies.
fn edited(from: &str, to: &str) -> String {
    assert!(
        JSON_SCHEMA.contains(from),
        "the fixture no longer contains {from:?}"
    );
    JSON_SCHEMA.replacen(from, to, 1)
}

#[test]
fn a_repeated_member_name_is_refused() {
    let text = edited(
        r#""rm_release": "1.0.0","#,
        r#""rm_release": "1.0.0", "rm_release": "1.0.1","#,
    );
    assert!(matches!(
        read_schema_json(&text),
        Err(PBmmReadError::Json { .. })
    ));
}

#[test]
fn malformed_json_is_refused() {
    assert!(matches!(
        read_schema_json("{\"bmm_version\": "),
        Err(PBmmReadError::Json { .. })
    ));
}

#[test]
fn a_null_member_is_refused() {
    let text = edited(
        r#""type": "Integer", "is_mandatory": true"#,
        r#""type": null"#,
    );
    assert!(matches!(
        read_schema_json(&text),
        Err(PBmmReadError::WrongValueShape { found: "null", .. })
    ));
}

#[test]
fn an_inconsistent_cardinality_is_refused() {
    // Lower_included_valid: an unbounded side is never included
    // (BASE org.openehr.base.foundation_types.interval.adoc §Invariants).
    for limits in [
        r#"{"lower": 1, "lower_unbounded": true}"#,
        r#"{"upper_unbounded": true, "upper_included": true}"#,
        r#"{"lower": "1"}"#,
    ] {
        let text = edited(r#"{"lower": 1, "upper_unbounded": true}"#, limits);
        assert!(
            matches!(
                read_schema_json(&text),
                Err(PBmmReadError::UnsupportedCardinality { .. })
            ),
            "{limits} must be refused"
        );
    }
    let text = edited(
        r#""upper_unbounded": true}"#,
        r#""upper_unbounded": true, "width": 2}"#,
    );
    assert!(matches!(
        read_schema_json(&text),
        Err(PBmmReadError::UnknownAttribute { .. })
    ));
}

#[test]
fn a_member_the_p_bmm_model_does_not_declare_is_refused() {
    let text = edited(
        r#""name": "WHOLE","#,
        r#""name": "WHOLE", "colour": "blue","#,
    );
    assert!(matches!(
        read_schema_json(&text),
        Err(PBmmReadError::UnknownAttribute { .. })
    ));
}
