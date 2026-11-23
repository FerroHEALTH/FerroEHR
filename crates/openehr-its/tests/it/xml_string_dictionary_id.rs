// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! A `StringDictionaryItem` element must carry its `id` attribute.

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

/// The `MissingAttribute` location of `xml` as an OPT, `None` for any other
/// outcome.
fn missing_id_location(xml: &str) -> Option<openehr_its::xml::runtime::Location> {
    match openehr_its::opt14::from_xml(xml) {
        Err(XmlError::MissingAttribute { location, .. }) => Some(*location),
        _ => None,
    }
}

/// The refusal names the element that lacks the attribute, never the one
/// closed before it (#5916): the check runs on a child's start tag, before the
/// child is read, so it describes the element just opened.
#[test]
fn a_missing_id_names_the_element_that_lacks_it() {
    let first = missing_id_location(&with_first_author_tag("<original_author>"))
        .expect("refused as MissingAttribute");
    assert_eq!(first.element, "original_author");
    assert_eq!(first.line, 11);
    assert_eq!(first.path, "/template/description[1]/original_author[1]");

    let source = std::fs::read_to_string(FIXTURE).expect("the vendored SDK template");
    let later = source.replacen(
        "<original_author id=\"organisation\">",
        "<original_author>",
        1,
    );
    let second = missing_id_location(&later).expect("refused as MissingAttribute");
    assert_eq!(second.element, "original_author");
    assert_eq!(second.line, 12);
    assert_eq!(second.path, "/template/description[1]/original_author[2]");

    let details = source.replacen("<other_details id=\"licence\"/>", "<other_details/>", 1);
    let other = missing_id_location(&details).expect("refused as MissingAttribute");
    assert_eq!(other.element, "other_details");
    assert_eq!(other.line, 16);
}
