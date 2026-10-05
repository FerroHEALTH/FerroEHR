// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! A mandatory element that is absent cannot be searched for, so its refusal
//! names the element that should hold it, where that element sits in the
//! document, and which class attribute the child realises (#3067). The shape
//! Archetype Designer emits on its own is the fixture: a template overlay
//! (`T_COMPLEX_OBJECT`, the OPT `constraints` element) whose typed
//! `default_value` has no content.

#![expect(
    clippy::expect_used,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_its::xml::runtime::XmlError;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sdk/ips.v0.opt");

/// The vendored IPS OPT with a `constraints` overlay appended before the
/// closing `</template>`, carrying an empty `default_value` of `xsi_type`.
/// Returns the document and the 1-based line the `default_value` element
/// landed on (it is indented eight spaces, so column 9).
fn ips_with_empty_default_value(xsi_type: &str) -> (String, usize) {
    let source = std::fs::read_to_string(FIXTURE).expect("the vendored IPS OPT");
    let closing = source
        .lines()
        .position(|l| l.trim() == "</template>")
        .expect("the fixture closes its template element");
    let block = format!(
        "  <constraints>\n    <attributes>\n      <rm_attribute_name>value</rm_attribute_name>\n      \
         <children>\n        <default_value xsi:type=\"{xsi_type}\"/>\n      </children>\n      \
         <differential_path>/content[openEHR-EHR-SECTION.medications_ips.v0]</differential_path>\n    \
         </attributes>\n  </constraints>\n"
    );
    let mut out = String::with_capacity(source.len() + block.len());
    for (index, line) in source.lines().enumerate() {
        if index == closing {
            out.push_str(&block);
        }
        out.push_str(line);
        out.push('\n');
    }
    // `closing` is 0-based; the block's fifth line is the `default_value`.
    (out, closing + 5)
}

#[test]
fn an_absent_mandatory_child_names_the_element_its_position_and_its_class() {
    let (opt, line) = ips_with_empty_default_value("DV_IDENTIFIER");
    let err = openehr_its::opt14::from_xml(&opt)
        .expect_err("an empty DV_IDENTIFIER default value lacks its mandatory id");
    assert!(matches!(err, XmlError::MissingChild { .. }), "got {err:?}");
    let message = err.to_string();
    for expected in [
        r#"element <default_value xsi:type="DV_IDENTIFIER">"#,
        &format!("at line {line}, column 9"),
        "(/template/constraints[1]/attributes[1]/children[1]/default_value[1])",
        "is missing mandatory child <id> (DV_IDENTIFIER.id)",
    ] {
        assert!(
            message.contains(expected),
            "{expected:?} not in {message:?}"
        );
    }
    assert!(
        !message.starts_with("xml parse error"),
        "a cardinality refusal is not a well-formedness failure: {message}"
    );
}

#[test]
fn the_owning_class_follows_the_xsi_type() {
    // The first mandatory child in the class's field order is the one named;
    // for DV_CODED_TEXT that is the inherited `value`, attributed to the
    // concrete class the `xsi:type` names.
    for (xsi_type, child) in [("DV_QUANTITY", "magnitude"), ("DV_CODED_TEXT", "value")] {
        let (opt, _) = ips_with_empty_default_value(xsi_type);
        let message = openehr_its::opt14::from_xml(&opt)
            .expect_err("an empty typed default value lacks its mandatory child")
            .to_string();
        assert!(
            message.contains(&format!(
                "is missing mandatory child <{child}> ({xsi_type}.{child})"
            )),
            "{message}"
        );
    }
}
