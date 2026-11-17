// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The default values the Web Template inputs carry.

use indexmap::IndexMap;
use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_its::opt14::types::{
    CAttribute, CBoolean, CCodePhrase, CComplexObject, CDate, CDuration, CDvOrdinal, CDvQuantity,
    CInteger, CObject, CPrimitive, CPrimitiveObject, CQuantityItem, CSingleAttribute, CString,
    Intervalofinteger, Intervalofreal,
};
use openehr_rm::v1_2::data_types::quantity::dv_ordinal::DvOrdinal;
use openehr_rm::v1_2::data_types::quantity::dv_quantity::DvQuantity;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use super::{Labels, WebTemplateBindingCodedValue, WebTemplateInput, build_inputs};

/// Labels that resolve nothing (every label falls back to its code).
struct NoLabels;

impl Labels for NoLabels {
    fn text(&self, _terminology: &str, _code: &str) -> Option<String> {
        None
    }
    fn localized(&self, _terminology: &str, _code: &str) -> IndexMap<String, String> {
        IndexMap::new()
    }
    fn term_bindings(&self, _code: &str) -> IndexMap<String, WebTemplateBindingCodedValue> {
        IndexMap::new()
    }
}

fn one() -> Intervalofinteger {
    Intervalofinteger {
        lower_included: Some(true),
        upper_included: Some(true),
        lower_unbounded: false,
        upper_unbounded: false,
        lower: Some(1),
        upper: Some(1),
    }
}

fn from_zero() -> Intervalofreal {
    Intervalofreal {
        lower_included: Some(true),
        upper_included: None,
        lower_unbounded: false,
        upper_unbounded: true,
        lower: Some(0.0),
        upper: None,
    }
}

/// A `C_COMPLEX_OBJECT` of `rm_type` whose `attribute` holds `child`.
fn complex(rm_type: &str, attribute: &str, child: CObject) -> CObject {
    CObject::CComplexObject(CComplexObject {
        rm_type_name: rm_type.to_owned(),
        occurrences: one(),
        node_id: String::new(),
        attributes: vec![CAttribute::CSingleAttribute(CSingleAttribute {
            rm_attribute_name: attribute.to_owned(),
            existence: one(),
            children: vec![child],
        })],
    })
}

fn primitive(rm_type: &str, item: CPrimitive) -> CObject {
    CObject::CPrimitiveObject(CPrimitiveObject {
        rm_type_name: rm_type.to_owned(),
        occurrences: one(),
        node_id: String::new(),
        item: Some(Box::new(item)),
    })
}

fn defaults(rm_type: &str, co: &CObject) -> Vec<(Option<String>, Option<String>)> {
    let (inputs, _) = build_inputs(rm_type, co, &NoLabels, None);
    inputs
        .iter()
        .map(|input: &WebTemplateInput| {
            (
                input.suffix.clone(),
                input
                    .default_value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
            )
        })
        .collect()
}

fn some(suffix: Option<&str>, value: Option<&str>) -> (Option<String>, Option<String>) {
    (suffix.map(str::to_owned), value.map(str::to_owned))
}

fn code_phrase(code: &str) -> openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase {
    openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase {
        terminology_id: TerminologyId {
            value: "local".to_owned(),
        },
        code_string: code.to_owned(),
        preferred_term: None,
    }
}

fn coded_text(assumed: &str) -> CObject {
    complex(
        "DV_CODED_TEXT",
        "defining_code",
        CObject::CCodePhrase(CCodePhrase {
            rm_type_name: "CODE_PHRASE".to_owned(),
            occurrences: one(),
            node_id: String::new(),
            assumed_value: Some(code_phrase(assumed)),
            terminology_id: Some(TerminologyId {
                value: "local".to_owned(),
            }),
            code_list: vec!["at1".to_owned(), "at2".to_owned()],
        }),
    )
}

#[test]
fn coded_text_offers_a_listed_assumed_code_only() {
    let co = coded_text("at2");
    assert_eq!(
        defaults("DV_CODED_TEXT", &co),
        vec![some(Some("code"), Some("at2"))]
    );
    let co = coded_text("at9");
    assert_eq!(
        defaults("DV_CODED_TEXT", &co),
        vec![some(Some("code"), None)]
    );
}

fn quantity(assumed_magnitude: f64) -> CObject {
    CObject::CDvQuantity(CDvQuantity {
        rm_type_name: "DV_QUANTITY".to_owned(),
        occurrences: one(),
        node_id: String::new(),
        assumed_value: Some(DvQuantity {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            magnitude: assumed_magnitude,
            precision: None,
            units: "mg".to_owned(),
            units_system: None,
            units_display_name: None,
        }),
        property: None,
        list: vec![CQuantityItem {
            magnitude: Some(from_zero()),
            precision: None,
            units: "mg".to_owned(),
        }],
    })
}

#[test]
fn quantity_offers_an_admitted_assumed_value_only() {
    assert_eq!(
        defaults("DV_QUANTITY", &quantity(5.5)),
        vec![
            some(Some("magnitude"), Some("5.5")),
            some(Some("unit"), Some("mg"))
        ]
    );
    assert_eq!(
        defaults("DV_QUANTITY", &quantity(-1.0)),
        vec![some(Some("magnitude"), None), some(Some("unit"), None)]
    );
}

#[test]
fn count_boolean_text_and_date_offer_their_assumed_values() {
    let count = complex(
        "DV_COUNT",
        "magnitude",
        primitive(
            "INTEGER",
            CPrimitive::CInteger(CInteger {
                list: Vec::new(),
                range: None,
                assumed_value: Some(3),
            }),
        ),
    );
    assert_eq!(defaults("DV_COUNT", &count), vec![some(None, Some("3"))]);

    let boolean = complex(
        "DV_BOOLEAN",
        "value",
        primitive(
            "BOOLEAN",
            CPrimitive::CBoolean(CBoolean {
                true_valid: true,
                false_valid: true,
                assumed_value: Some(true),
            }),
        ),
    );
    assert_eq!(
        defaults("DV_BOOLEAN", &boolean),
        vec![some(None, Some("true"))]
    );

    let text = complex(
        "DV_TEXT",
        "value",
        primitive(
            "STRING",
            CPrimitive::CString(CString {
                pattern: None,
                list: vec!["a".to_owned(), "b".to_owned()],
                list_open: None,
                assumed_value: Some("b".to_owned()),
            }),
        ),
    );
    assert_eq!(defaults("DV_TEXT", &text), vec![some(None, Some("b"))]);

    let date = complex(
        "DV_DATE",
        "value",
        primitive(
            "DATE",
            CPrimitive::CDate(CDate {
                pattern: None,
                timezone_validity: None,
                range: None,
                assumed_value: Some("2020-01-31".to_owned()),
            }),
        ),
    );
    assert_eq!(
        defaults("DV_DATE", &date),
        vec![some(None, Some("2020-01-31"))]
    );
}

#[test]
fn duration_fields_take_the_assumed_components() {
    let duration = complex(
        "DV_DURATION",
        "value",
        primitive(
            "DURATION",
            CPrimitive::CDuration(CDuration {
                pattern: Some("PYTH".to_owned()),
                range: None,
                assumed_value: Some("P2YT6H".to_owned()),
            }),
        ),
    );
    assert_eq!(
        defaults("DV_DURATION", &duration),
        vec![some(Some("year"), Some("2")), some(Some("hour"), Some("6"))]
    );
}

#[test]
fn ordinal_offers_a_listed_assumed_symbol() {
    let entry = |code: &str, value: i32| DvOrdinal {
        normal_status: None,
        normal_range: None,
        other_reference_ranges: None,
        symbol: DvCodedText {
            value: code.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: None,
            language: None,
            encoding: None,
            defining_code: openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase {
                terminology_id: TerminologyId {
                    value: "local".to_owned(),
                },
                code_string: code.to_owned(),
                preferred_term: None,
            },
        },
        value,
    };
    let co = CObject::CDvOrdinal(CDvOrdinal {
        rm_type_name: "DV_ORDINAL".to_owned(),
        occurrences: one(),
        node_id: String::new(),
        assumed_value: Some(entry("at2", 2)),
        list: vec![entry("at1", 1), entry("at2", 2)],
    });
    assert_eq!(defaults("DV_ORDINAL", &co), vec![some(None, Some("at2"))]);
}

/// An externally bound coded text with an assumed code serializes the
/// string `defaultValue` the released `web_template/Input.yaml` declares;
/// the vendored EHRbase node vector
/// (`corpus/fixtures/service/webtemplate_nodes`, prior art only) carries the
/// same shape.
#[test]
fn external_coded_text_default_matches_the_vendored_node_vector() {
    let co = complex(
        "DV_CODED_TEXT",
        "defining_code",
        CObject::CCodePhrase(CCodePhrase {
            rm_type_name: "CODE_PHRASE".to_owned(),
            occurrences: one(),
            node_id: String::new(),
            assumed_value: Some(
                openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase {
                    terminology_id: TerminologyId {
                        value: "ICD10".to_owned(),
                    },
                    code_string: "E10.0".to_owned(),
                    preferred_term: None,
                },
            ),
            terminology_id: Some(TerminologyId {
                value: "ICD10".to_owned(),
            }),
            code_list: Vec::new(),
        }),
    );
    let (inputs, _) = build_inputs("DV_CODED_TEXT", &co, &NoLabels, None);
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/fixtures/service/webtemplate_nodes/dv_codedtext_unsupported.json"
    );
    let text = std::fs::read_to_string(path).expect("vendored node vector");
    let node: serde_json::Value = serde_json::from_str(&text).expect("node JSON");
    assert_eq!(
        serde_json::to_value(&inputs).expect("serialize inputs"),
        node["inputs"]
    );
}
