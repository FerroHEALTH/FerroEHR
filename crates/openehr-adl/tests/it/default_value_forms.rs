// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `_default` forms of `cadl2.g4` (SPECPR-146), against the id-coded
//! examples of `ADL2/master06-default_values.adoc` restated without the term
//! rubric the ODIN grammar refuses.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::aom::access::complex_attributes;
use openehr_adl::parse::{Dialect, parse_definition_body};
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

/// Every id-coded `cadl` listing of the default-values chapter that states a
/// `_default`, verbatim.
fn master06_listings() -> Vec<String> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/specs/openehr/AM/docs/ADL2/master06-default_values.adoc"
    );
    let text = std::fs::read_to_string(path).expect("read the vendored default-values chapter");
    let mut listings = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "[source, cadl]" {
            continue;
        }
        let _open = lines.next();
        let body: Vec<&str> = lines
            .by_ref()
            .take_while(|l| !l.starts_with("-----"))
            .collect();
        let body = body.join("\n");
        if body.contains("_default") && body.trim_start().starts_with("/data[id") {
            listings.push(body);
        }
    }
    listings
}

/// The `default_value` of the object a listing constrains, after wrapping the
/// listing's differential attribute in an OBSERVATION root.
fn default_of(listing: &str) -> serde_json::Value {
    let source = format!("OBSERVATION[id1] matches {{\n{listing}\n}}");
    let root = parse_definition_body(&source, Dialect::Adl2)
        .unwrap_or_else(|e| panic!("the listing parses: {e:?}\n{source}"));
    let attribute = complex_attributes(&root)
        .first()
        .expect("the listing's attribute");
    let [CObject::CComplexObject(CComplexObject::CComplexObject(object))] =
        attribute.children.as_deref().unwrap_or_default()
    else {
        panic!("the attribute holds one complex object");
    };
    object
        .default_value
        .clone()
        .expect("the object carries the default")
}

#[test]
fn every_master06_json_listing_parses_to_its_default() {
    let listings: Vec<String> = master06_listings()
        .into_iter()
        .filter(|l| l.contains("(json) <#"))
        .collect();
    assert!(
        !listings.is_empty(),
        "the chapter states an id-coded JSON default"
    );
    for listing in &listings {
        let value = default_of(listing);
        assert_eq!(value["_type"], "DV_CODED_TEXT", "{listing}");
        assert_eq!(
            value["defining_code"]["code_string"], "163033001",
            "{listing}"
        );
    }
}

/// The typed ODIN listing of master06 without the `|lying BP|` rubric, which
/// the ODIN `TERM_CODE_REF` has no production for (#5814). The shape the term
/// code lands in is #5817.
#[test]
fn a_typed_odin_default_may_be_the_only_content() {
    let value = default_of(
        "/data[id2]/events[id6]/state[id7]/items[id8]/value matches {
            DV_CODED_TEXT[id9] matches {
                _default = (DV_CODED_TEXT) <
                    defining_code = <[snomed_ct::163033001]>
                >
            }
        }",
    );
    assert_eq!(value["_type"], "DV_CODED_TEXT");
    assert!(value.get("defining_code").is_some(), "{value}");
}

#[test]
fn a_json_default_is_read_as_canonical_json() {
    let value = default_of(
        "/data[id2]/events[id6]/state[id7]/items[id8]/value matches {
            DV_TEXT[id9] matches {
                _default = (json) <# {\"_type\": \"DV_TEXT\", \"value\": \"lying\"} #>
            }
        }",
    );
    assert_eq!(
        value,
        serde_json::json!({"_type": "DV_TEXT", "value": "lying"})
    );
}

#[test]
fn a_default_in_another_syntax_is_refused() {
    let errors = parse_definition_body(
        "OBSERVATION[id1] matches {
            /data[id2]/events[id6]/state[id7]/items[id8]/value matches {
                DV_TEXT[id9] matches {
                    _default = (xml) <# <value>lying</value> #>
                }
            }
        }",
        Dialect::Adl2,
    )
    .expect_err("an xml block has no canonical JSON form");
    assert!(
        errors
            .iter()
            .any(|e| e.to_string().contains("syntax 'xml'")),
        "the refusal names the syntax, got {errors:?}"
    );
}

#[test]
fn malformed_json_in_a_default_is_refused() {
    let errors = parse_definition_body(
        "OBSERVATION[id1] matches {
            /data[id2]/events[id6]/state[id7]/items[id8]/value matches {
                DV_TEXT[id9] matches {
                    _default = (json) <# {\"value\": } #>
                }
            }
        }",
        Dialect::Adl2,
    )
    .expect_err("the block is not JSON");
    assert!(
        errors
            .iter()
            .any(|e| e.to_string().contains("invalid JSON default value")),
        "got {errors:?}"
    );
}

#[test]
fn a_plug_in_block_outside_a_default_is_refused() {
    parse_definition_body(
        "OBSERVATION[id1] matches {
            data matches (json) <# {} #>
        }",
        Dialect::Adl2,
    )
    .expect_err("only default_value takes an included language fragment");
}
