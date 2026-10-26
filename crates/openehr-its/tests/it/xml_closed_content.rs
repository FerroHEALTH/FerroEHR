// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The XML readers hold a document to the schema's content model: an
//! operational template starts with the global element `template`
//! (ITS-XML `components/AM/Release-1.4/Template.xsd`), and a child element a
//! type's closed `xs:sequence` does not declare is refused (XML Schema 1.1
//! Part 1 §3.4.4.2), never skipped as if it were absent.

use openehr_its::xml::runtime::XmlError;

/// A synthetic operational template under the root `root`, with `extra`
/// inserted among the children of `<template>`.
fn template(root: &str, extra: &str) -> String {
    let interval = "<lower_included>true</lower_included><upper_included>true</upper_included>\
                    <lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded>\
                    <lower>1</lower><upper>1</upper>";
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<{root} xmlns="http://schemas.openehr.org/v1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <language><terminology_id><value>ISO_639-1</value></terminology_id><code_string>en</code_string></language>
  <template_id><value>synthetic.closed.v1</value></template_id>
  <concept>synthetic</concept>
  {extra}
  <definition>
    <rm_type_name>COMPOSITION</rm_type_name>
    <occurrences>{interval}</occurrences>
    <node_id>at0000</node_id>
    <archetype_id><value>openEHR-EHR-COMPOSITION.synthetic.v1</value></archetype_id>
  </definition>
</{root}>
"#
    )
}

#[test]
fn a_template_within_the_schema_is_read() {
    let opt = openehr_its::opt14::from_xml(&template("template", ""))
        .expect("a template inside the content model reads");
    assert_eq!(opt.concept, "synthetic");
}

#[test]
fn another_root_element_is_refused() {
    let err = openehr_its::opt14::from_xml(&template("archetype", ""))
        .expect_err("an OPT 1.4 document starts with <template>");
    assert!(
        matches!(&err, XmlError::UnexpectedRoot { expected, found } if expected == "template" && found == "archetype"),
        "got {err:?}"
    );
}

#[test]
fn an_undeclared_child_element_is_refused() {
    let err = openehr_its::opt14::from_xml(&template(
        "template",
        "<not_in_the_schema>x</not_in_the_schema>",
    ))
    .expect_err("an element outside OPERATIONAL_TEMPLATE's sequence is invalid");
    assert!(
        matches!(&err, XmlError::UndeclaredChild { child, .. } if child == "not_in_the_schema"),
        "got {err:?}"
    );
}

#[test]
fn a_child_of_a_text_element_is_refused() {
    let err = openehr_its::opt14::from_xml(&template("template", "").replace(
        "<concept>synthetic</concept>",
        "<concept>synthetic<b/></concept>",
    ))
    .expect_err("a text element has a simple type and no child elements");
    assert!(
        matches!(&err, XmlError::UndeclaredChild { child, .. } if child == "b"),
        "got {err:?}"
    );
}

/// `template` with `match_negated` set to `value` on its definition's one
/// attribute.
fn negated(value: &str) -> String {
    template("template", "").replace(
        "<node_id>at0000</node_id>\n    <archetype_id>",
        &format!(
            "<node_id>at0000</node_id>\n    <attributes xsi:type=\"C_SINGLE_ATTRIBUTE\">\
             <rm_attribute_name>category</rm_attribute_name>\
             <existence><lower_included>true</lower_included><upper_included>true</upper_included>\
             <lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded>\
             <lower>1</lower><upper>1</upper></existence>\
             <match_negated>{value}</match_negated></attributes>\n    <archetype_id>"
        ),
    )
}

/// `match_negated` is written by modelling tools and declared by no ITS-XML
/// schema (the CNF Robot `valid_templates/cache/case.opt` carries it): `false`
/// states the default and is read, `true` would negate a constraint no schema
/// defines and is refused.
#[test]
fn a_false_match_negated_is_read_and_a_true_one_refused() {
    let opt = openehr_its::opt14::from_xml(&negated("false"))
        .unwrap_or_else(|e| panic!("match_negated false reads: {e}"));
    assert_eq!(opt.definition.attributes.len(), 1);

    let err = openehr_its::opt14::from_xml(&negated("true"))
        .expect_err("a negated match no schema defines is refused");
    assert!(
        matches!(&err, XmlError::UnsupportedExtension { element, value, .. } if element == "match_negated" && value == "true"),
        "got {err:?}"
    );
}

/// A BASE 1.3.0 `TERMINOLOGY_CODE` reads and writes its `terminology_id` in the
/// `TERMINOLOGY_ID` shape ITS-XML 1.0.2 gives the `CODE_PHRASE` slot.
#[test]
fn a_terminology_code_takes_the_code_phrase_shape() {
    use openehr_base::prelude::TranslationDetails;
    let xml = "<translations xmlns=\"http://schemas.openehr.org/v1\">\
               <language><terminology_id><value>ISO_639-1</value></terminology_id>\
               <code_string>de</code_string></language>\
               <author id=\"name\">Dr. Ada</author></translations>";
    let details: TranslationDetails =
        openehr_its::xml::runtime::from_xml(xml).unwrap_or_else(|e| panic!("reads: {e}"));
    assert_eq!(details.language.terminology_id, "ISO_639-1");
    let written = openehr_its::xml::runtime::to_xml(
        &details,
        "translations",
        openehr_its::xml::runtime::Namespace::V1,
    )
    .unwrap_or_else(|e| panic!("writes: {e}"));
    assert!(
        written.contains("<terminology_id><value>ISO_639-1</value></terminology_id>"),
        "{written}"
    );
}
