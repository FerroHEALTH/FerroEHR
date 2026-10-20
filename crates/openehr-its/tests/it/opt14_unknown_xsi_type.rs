// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! An OPT 1.4 constraint node whose `xsi:type` the OPT schemas do not define is
//! refused, and the refusal names the schemas it was read against.
//!
//! The `C_OBJECT` subtypes of an OPT 1.4 document are the closed set
//! `its-xml-1.0.2-nsv1/ALL/Template.xsd`, `OpenehrProfile.xsd` and
//! `Archetype.xsd` define (the same set as
//! `its-xml-2.0.0-nsv2/AM/Release-1.4/Template.xsd`): an archetype root inside
//! a definition is a `C_ARCHETYPE_ROOT`, and no `T_ARCHETYPE_ROOT` exists in
//! any of them (#3892).

#![expect(
    clippy::expect_used,
    reason = "fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_its::xml::runtime::XmlError;

/// The schema closure the refusal names.
const SCHEMAS: &str = "the ITS-XML OPT 1.4 schemas (Template.xsd, OpenehrProfile.xsd, \
                       Archetype.xsd) define no C_OBJECT subtype of that name";

/// A synthetic operational template whose one nested archetype root carries
/// `xsi:type="{xsi_type}"`.
fn template(xsi_type: &str) -> String {
    let interval = "<lower_included>true</lower_included><upper_included>true</upper_included>\
                    <lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded>\
                    <lower>1</lower><upper>1</upper>";
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<template xmlns="http://schemas.openehr.org/v1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <language><terminology_id><value>ISO_639-1</value></terminology_id><code_string>en</code_string></language>
  <template_id><value>synthetic.xsi_type.v1</value></template_id>
  <concept>synthetic</concept>
  <definition>
    <rm_type_name>COMPOSITION</rm_type_name>
    <occurrences>{interval}</occurrences>
    <node_id>at0000</node_id>
    <attributes xsi:type="C_SINGLE_ATTRIBUTE">
      <rm_attribute_name>content</rm_attribute_name>
      <existence>{interval}</existence>
      <children xsi:type="{xsi_type}">
        <rm_type_name>OBSERVATION</rm_type_name>
        <occurrences>{interval}</occurrences>
        <node_id>at0000</node_id>
        <archetype_id><value>openEHR-EHR-OBSERVATION.synthetic.v1</value></archetype_id>
      </children>
    </attributes>
    <archetype_id><value>openEHR-EHR-COMPOSITION.synthetic.v1</value></archetype_id>
  </definition>
</template>
"#
    )
}

/// The refusal of `xsi_type`, which the OPT schemas do not define.
fn refusal(xsi_type: &str) -> String {
    let err = openehr_its::opt14::from_xml(&template(xsi_type))
        .expect_err("an xsi:type outside the OPT schemas is not a document the format admits");
    assert!(matches!(err, XmlError::Parse(_)), "got {err:?}");
    err.to_string()
}

#[test]
fn the_schema_defined_archetype_root_is_accepted() {
    let opt = openehr_its::opt14::from_xml(&template("C_ARCHETYPE_ROOT"))
        .expect("C_ARCHETYPE_ROOT is the archetype root Template.xsd defines");
    let round_trip = openehr_its::opt14::to_xml(&opt).expect("the template serializes");
    assert!(
        round_trip.contains("openEHR-EHR-OBSERVATION.synthetic.v1"),
        "the nested archetype root did not survive the read"
    );
}

#[test]
fn t_archetype_root_is_refused_naming_the_schemas() {
    let message = refusal("T_ARCHETYPE_ROOT");
    for expected in ["CObject: unknown xsi:type T_ARCHETYPE_ROOT", SCHEMAS] {
        assert!(
            message.contains(expected),
            "{expected:?} not in {message:?}"
        );
    }
}

#[test]
fn an_invented_type_is_refused_naming_the_schemas() {
    let message = refusal("NOT_AN_AOM_TYPE");
    for expected in ["CObject: unknown xsi:type NOT_AN_AOM_TYPE", SCHEMAS] {
        assert!(
            message.contains(expected),
            "{expected:?} not in {message:?}"
        );
    }
}
