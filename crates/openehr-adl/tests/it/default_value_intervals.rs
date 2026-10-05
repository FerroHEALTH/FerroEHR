// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ODIN interval `_default` values, end to end: source → AOM2 → printed ADL →
//! source again.
//!
//! `ADL2/master06-default_values.adoc` §Syntax states that a default value is
//! "expressed in any regular object instance syntax, including ODIN syntax",
//! and `LANG/docs/odin/master07-leaf_data.adoc` §Intervals of Ordered Primitive
//! Types lists intervals among ODIN's leaf DATA forms — so an interval is a
//! legal `_default` datum, at the top of the block or under an attribute of it.
//!
//! The intermediate canonical-JSON object it lands in is our own
//! design/extension (no openEHR spec governs the intermediate shape); it
//! mirrors the emitted `Point_interval`/`Proper_interval` encoding of
//! `crates/openehr-its/src/json_codec/generated/impls.rs`, so a stored default
//! reads back with the same field set the codec would have written.
//!
//! The ADL 1.4 twin of this behaviour is the refusal fixture
//! `tests/corpus/adl14-cadl/openEHR-EHR-OBSERVATION.SCOAT_adl2_default_value.v1.adl`
//! — `_default` is an ADL2-only construct (`ADL1.4/master05-cadl.adoc`
//! §Keywords), so the accepting cases below are ADL2 sources.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::print::print;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

/// An ADL2 archetype whose `ELEMENT[id2]` node carries `default` as its
/// `_default` block body.
fn source(default: &str) -> String {
    format!(
        "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
         \x20   openEHR-EHR-ENTRY.default_interval.v1.0.0\n\
         \n\
         language\n\
         \x20   original_language = <[ISO_639-1::en]>\n\
         \n\
         description\n\
         \x20   lifecycle_state = <\"draft\">\n\
         \n\
         definition\n\
         \x20   ENTRY[id1] matches {{\n\
         \x20       element_attr matches {{\n\
         \x20           ELEMENT[id2] matches {{\n\
         \x20               _default = {default}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20   }}\n\
         \n\
         terminology\n\
         \x20   term_definitions = <\n\
         \x20       [\"en\"] = <\n\
         \x20           [\"id1\"] = < text = <\"root\"> description = <\"root\"> >\n\
         \x20           [\"id2\"] = < text = <\"el\"> description = <\"el\"> >\n\
         \x20       >\n\
         \x20   >\n"
    )
}

/// The `default_value` of the `ELEMENT[id2]` node of an assembled archetype.
fn element_default(archetype: &Archetype) -> serde_json::Value {
    let Archetype::AuthoredArchetype(authored) = archetype else {
        panic!("expected an authored archetype")
    };
    let AuthoredArchetype::AuthoredArchetype(authored) = authored.as_ref() else {
        panic!("expected a plain authored archetype")
    };
    let CComplexObject::CComplexObject(root) = &authored.definition else {
        panic!("expected a plain complex-object root")
    };
    let element = root
        .attributes
        .iter()
        .flatten()
        .next()
        .and_then(|a| a.children.iter().flatten().next())
        .expect("the ELEMENT child must be present");
    let CObject::CComplexObject(CComplexObject::CComplexObject(element)) = element else {
        panic!("expected the ELEMENT complex object")
    };
    element
        .default_value
        .clone()
        .expect("the `_default` must be read")
}

/// Parse `default` as a `_default` body, and return its canonical JSON
/// together with the printed ADL of the whole archetype.
fn parse_and_print(default: &str) -> (serde_json::Value, String) {
    let src = source(default);
    let archetype = parse_artefact(&src, Dialect::Adl2)
        .unwrap_or_else(|e| panic!("`_default = {default}` must parse, got {e:?}"));
    let json = element_default(&archetype);
    let printed = print(&archetype).expect("print the parsed archetype");
    // The printed text must re-read as the very same artefact.
    let reparsed = parse_artefact(&printed, Dialect::Adl2)
        .unwrap_or_else(|e| panic!("the printed form of `{default}` must re-parse, got {e:?}"));
    assert_eq!(
        archetype, reparsed,
        "`_default = {default}` must round-trip through the printer"
    );
    assert_eq!(
        element_default(&reparsed),
        json,
        "`_default = {default}` must re-read to the same canonical JSON"
    );
    (json, printed)
}

#[test]
fn a_top_level_interval_default_encodes_and_round_trips() {
    let (json, printed) = parse_and_print("<|0..5|>");
    assert_eq!(
        json,
        serde_json::json!({
            "_type": "Proper_interval",
            "lower": 0,
            "upper": 5,
            "lower_unbounded": false,
            "upper_unbounded": false,
            "lower_included": true,
            "upper_included": true,
        })
    );
    assert!(
        printed.contains("_default = <|0..5|>"),
        "the printer must emit canonical ODIN interval syntax, got:\n{printed}"
    );
}

/// The `<`/`>` of the relational interval forms are bound operators, not block
/// delimiters (`LANG/docs/odin/master07-leaf_data.adoc` §Intervals of Ordered
/// Primitive Types), so a `_default` body carrying them is still delimited
/// correctly.
#[test]
fn relational_interval_forms_survive_the_default_block_capture() {
    for (src, literal) in [
        ("<|>0..<5|>", "|>0..<5|"),
        ("<|>=0|>", "|>=0|"),
        ("<|<5|>", "|<5|"),
        ("<|0..infinity|>", "|>=0|"),
    ] {
        let (_, printed) = parse_and_print(src);
        assert!(
            printed.contains(&format!("_default = <{literal}>")),
            "`_default = {src}` must print as `{literal}`, got:\n{printed}"
        );
    }
}

/// An interval nested under an attribute of a typed `_default` block keeps the
/// block's RM cast on the outside and its own `Proper_interval` tag inside —
/// the cast names the generic slot type, which is not a canonical-JSON class
/// tag.
#[test]
fn a_nested_interval_default_keeps_both_tags() {
    let (json, printed) = parse_and_print(
        "(DV_INTERVAL) <\n\
         \x20               lower = <|>=0.0|>\n\
         \x20           >",
    );
    assert_eq!(json["_type"], serde_json::json!("DV_INTERVAL"));
    assert_eq!(json["lower"]["_type"], serde_json::json!("Proper_interval"));
    assert_eq!(json["lower"]["lower"], serde_json::json!(0.0));
    assert!(
        printed.contains("lower = <|>=0.0|>"),
        "the nested interval must print as an ODIN interval, got:\n{printed}"
    );
}

/// A degenerate closed interval denotes a single value, so it carries the
/// `Point_interval` tag and prints back as the bare `|N|` form.
#[test]
fn a_point_interval_default_round_trips_as_a_single_value() {
    let (json, printed) = parse_and_print("<|5|>");
    assert_eq!(json["_type"], serde_json::json!("Point_interval"));
    assert!(
        printed.contains("_default = <|5|>"),
        "the point interval must print as `|5|`, got:\n{printed}"
    );
}

/// A `|centre +/- delta|` interval over temporal endpoints has no faithful
/// bound reduction, so it is refused at the parse (SDINV) rather than guessed.
#[test]
fn a_non_numeric_plus_minus_default_is_refused() {
    let src = source("<|2020-01-01 +/-P1D|>");
    let errors =
        parse_artefact(&src, Dialect::Adl2).expect_err("a temporal `+/-` interval must be refused");
    assert!(
        errors
            .iter()
            .any(|e| e.to_string().contains("no lower/upper reduction")),
        "expected the reduction limitation to be named, got {errors:?}"
    );
}
