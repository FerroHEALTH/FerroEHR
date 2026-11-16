// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `C_STRING` pattern no regex engine compiles is refused when the template
//! loads, and a value checked against one anyway is refused (#5841; AOM 1.4
//! `C_STRING.valid_value`, ADL 1.4 `master05-cadl.adoc` §Regular Expression).
//! No vendored OPT constrains a text value with a pattern, so the probe gives
//! one open `DV_TEXT` of the vendored `Demo Vitals.opt` a pattern. The ADL2
//! parser already refuses an uncompilable regex (`SCSRE`), so the OPT 1.4
//! front end is where one can reach the Web Template.

#![expect(
    clippy::expect_used,
    reason = "fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests \
              scoping does not reach"
)]

use openehr_its::opt14;
use openehr_its::opt14::types::{
    CAttribute, CObject, CPrimitive, CPrimitiveObject, CSingleAttribute, CString,
    OperationalTemplate,
};
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};

/// The Web Template of the vendored `Demo Vitals.opt` with `pattern`
/// constraining the first open `DV_TEXT` element value it holds.
fn demo_vitals_with(pattern: &str) -> Result<WebTemplate, FlatError> {
    build_web_template(&demo_vitals_opt(pattern))
}

fn demo_vitals_opt(pattern: &str) -> OperationalTemplate {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/better/Demo Vitals.opt"
    );
    let xml = std::fs::read_to_string(path).expect("read the vendored OPT");
    let mut opt = opt14::from_xml(&xml).expect("the OPT parses");
    assert!(
        constrain_first_text(&mut opt.definition.attributes, pattern),
        "the OPT holds an open DV_TEXT value"
    );
    opt
}

/// Gives the first `DV_TEXT` with no constraint of its own a `value` pattern.
fn constrain_first_text(attributes: &mut [CAttribute], pattern: &str) -> bool {
    attributes.iter_mut().any(|attribute| {
        let children = match attribute {
            CAttribute::CMultipleAttribute(m) => &mut m.children,
            CAttribute::CSingleAttribute(s) => &mut s.children,
        };
        children.iter_mut().any(|child| match child {
            CObject::CComplexObject(c)
                if c.rm_type_name == "DV_TEXT" && c.attributes.is_empty() =>
            {
                c.attributes
                    .push(CAttribute::CSingleAttribute(CSingleAttribute {
                        rm_attribute_name: "value".to_owned(),
                        existence: c.occurrences.clone(),
                        children: vec![CObject::CPrimitiveObject(CPrimitiveObject {
                            rm_type_name: "STRING".to_owned(),
                            occurrences: c.occurrences.clone(),
                            node_id: String::new(),
                            item: Some(Box::new(CPrimitive::CString(CString {
                                pattern: Some(pattern.to_owned()),
                                list: Vec::new(),
                                list_open: None,
                                assumed_value: None,
                            }))),
                        })],
                    }));
                true
            }
            CObject::CComplexObject(c) => constrain_first_text(&mut c.attributes, pattern),
            CObject::CArchetypeRoot(r) => constrain_first_text(&mut r.attributes, pattern),
            _ => false,
        })
    })
}

fn pattern_of(node: &WebTemplateNode) -> Option<(String, String)> {
    node.inputs
        .iter()
        .find_map(|input| input.validation.as_ref()?.pattern.clone())
        .map(|pattern| (node.aql_path.clone(), pattern))
        .or_else(|| node.children.iter().find_map(pattern_of))
}

#[test]
fn loading_refuses_a_string_pattern_no_engine_compiles() {
    let built = demo_vitals_with("[A-Z]+").expect("a readable pattern builds");
    let (path, pattern) = pattern_of(&built.tree).expect("the text value carries its pattern");
    assert_eq!(pattern, "[A-Z]+");

    match demo_vitals_with("(") {
        Err(FlatError::UnevaluablePattern {
            path: refused,
            pattern,
        }) => {
            assert_eq!(refused, path, "the refusal names the node");
            assert_eq!(pattern, "(");
        }
        other => panic!("an unreadable pattern refuses the template, got {other:?}"),
    }
}

/// No vendored OPT carries a string pattern no engine compiles.
#[test]
fn every_vendored_template_has_evaluable_string_patterns() {
    for dir in crate::common::opt_dirs() {
        for path in crate::common::opt_files(&dir) {
            let Ok(xml) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(opt) = opt14::from_xml(&xml) else {
                continue;
            };
            if let Err(e @ FlatError::UnevaluablePattern { .. }) = build_web_template(&opt) {
                panic!("{}: {e}", path.display());
            }
        }
    }
}
