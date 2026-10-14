// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! A `StringDictionaryItem` element must carry its `id` attribute.
//!
//! The ITS-XML schemas type every `Hash<String, String>` entry as
//! `StringDictionaryItem`, whose `id` is `use="required"`
//! (`schemas/xml/its-xml-2.0.0-nsv2/AM/Release-1.4/Resource.xsd`,
//! `schemas/xml/its-xml-2.0.0-nsv2/BASE/Release-1.2.0/BaseTypes.xsd`). An element
//! without it is refused naming the attribute; an empty `id=""` is a present
//! `xs:string` and is accepted (the valid twin).

#![expect(
    clippy::expect_used,
    reason = "fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_base::prelude::TranslationDetails;
use openehr_its::xml::runtime::{XmlError, from_xml};

/// A vendored SDK template whose description names its original author.
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sdk/ips.v0.opt");

/// The fixture's first `original_author` start tag.
const AUTHOR_TAG: &str = "<original_author id=\"name\">";

/// The fixture with its first `original_author` start tag replaced by `tag`.
fn with_first_author_tag(tag: &str) -> String {
    let source = std::fs::read_to_string(FIXTURE).expect("the vendored SDK template");
    assert!(source.contains(AUTHOR_TAG), "the fixture names its author");
    source.replacen(AUTHOR_TAG, tag, 1)
}

#[test]
fn an_opt_original_author_without_id_is_refused() {
    let opt = with_first_author_tag("<original_author>");
    let err = openehr_its::opt14::from_xml(&opt)
        .expect_err("original_author without its required id is not a valid OPT");
    match &err {
        XmlError::MissingAttribute {
            owner, attribute, ..
        } => {
            assert_eq!(owner, "StringDictionaryItem");
            assert_eq!(attribute, "id");
        }
        other => panic!("expected MissingAttribute, got {other:?}"),
    }
}

#[test]
fn an_opt_original_author_with_an_empty_id_is_accepted() {
    let opt = with_first_author_tag("<original_author id=\"\">");
    let parsed = openehr_its::opt14::from_xml(&opt).expect("an empty id is a present xs:string");
    let description = parsed.description.expect("the fixture has a description");
    assert_eq!(
        description.original_author.get("").map(String::as_str),
        Some("Heather Leslie")
    );
}

#[test]
fn an_rm_translation_author_without_id_is_refused() {
    let xml = "<translations xmlns=\"http://schemas.openehr.org/v1\">\
               <language><terminology_id><value>ISO_639-1</value></terminology_id>\
               <code_string>de</code_string></language>\
               <author>Dr. Ada</author></translations>";
    let err = from_xml::<TranslationDetails>(xml)
        .expect_err("author without its required id is not a valid TRANSLATION_DETAILS");
    assert!(
        matches!(&err, XmlError::MissingAttribute { attribute, .. } if attribute == "id"),
        "got {err:?}"
    );
}
