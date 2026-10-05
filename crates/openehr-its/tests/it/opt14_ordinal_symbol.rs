// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! What an ordinal `<symbol>` must carry in an OPT 1.4 document, and what the
//! refusal says when it does not.
//!
//! The ITS-XML profile schema types `C_DV_ORDINAL.list` as the RM `DV_ORDINAL`
//! (`components/AM/Release-1.4/OpenehrProfile.xsd`), whose `symbol` is a
//! `DV_CODED_TEXT`: `value` and `defining_code` are both mandatory. An empty
//! `<value/>` satisfies the schema and carries nothing, which is what the
//! exported templates in this corpus write, because AOM 1.4 models the symbol
//! as a `CODE_PHRASE` (`AM/docs/UML/classes/org.openehr.am.aom14.ordinal.adoc`)
//! and the rubric is resolved from `term_definitions`. A symbol without the
//! element is refused, and the refusal names that one-element fix (#3067).

#![expect(
    clippy::expect_used,
    reason = "fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_its::xml::runtime::XmlError;

/// A vendored SDK template whose first `C_DV_ORDINAL` constrains its list with
/// symbols in the exported shape: an empty `<value/>` beside the code.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/sdk/Test_all_types.opt"
);

/// The `defining_code` of the fixture's first ordinal symbol, written flat.
const DEFINING_CODE: &str = "<defining_code><terminology_id><value>local</value>\
                             </terminology_id><code_string>at0014</code_string></defining_code>";

/// The remedy the refusal for an absent ordinal symbol value appends.
const REMEDY: &str = ". Add <value/> (empty is valid; the rubric is resolved from \
                      term_definitions; the ITS-XML profile schema requires the element on an \
                      ordinal symbol, see #3401)";

/// The fixture with the content of its first ordinal `<symbol>` replaced by
/// `body`, leaving every other symbol in the document untouched.
fn with_first_symbol(body: &str) -> String {
    let source = std::fs::read_to_string(FIXTURE).expect("the vendored SDK template");
    let (head, rest) = source
        .split_once("<symbol>")
        .expect("the fixture constrains an ordinal");
    let (_, tail) = rest
        .split_once("</symbol>")
        .expect("the symbol element closes");
    format!("{head}<symbol>{body}</symbol>{tail}")
}

#[test]
fn a_symbol_without_its_value_is_refused_and_the_refusal_names_the_remedy() {
    let opt = with_first_symbol(DEFINING_CODE);
    let err = openehr_its::opt14::from_xml(&opt)
        .expect_err("an ordinal symbol without <value> is not a document the format admits");
    assert!(matches!(err, XmlError::MissingChild { .. }), "got {err:?}");
    let message = err.to_string();
    for expected in [
        "element <symbol>",
        "is missing mandatory child <value> (DV_CODED_TEXT.value)",
        REMEDY,
    ] {
        assert!(
            message.contains(expected),
            "{expected:?} not in {message:?}"
        );
    }
}

#[test]
fn a_symbol_with_an_empty_value_is_accepted() {
    let opt = with_first_symbol(&format!("<value/>{DEFINING_CODE}"));
    let template =
        openehr_its::opt14::from_xml(&opt).expect("an empty <value/> satisfies the schema");
    let round_trip = openehr_its::opt14::to_xml(&template).expect("the template serializes");
    assert!(
        round_trip.contains("<symbol><value></value><defining_code>")
            || round_trip.contains("<symbol><value/><defining_code>"),
        "the empty symbol value did not survive the read: {}",
        round_trip
            .split("<symbol>")
            .nth(1)
            .unwrap_or("no symbol in the output")
    );
}

#[test]
fn a_symbol_without_its_defining_code_is_refused_without_the_remedy() {
    let opt = with_first_symbol("<value/>");
    let err = openehr_its::opt14::from_xml(&opt)
        .expect_err("an ordinal symbol without <defining_code> has no code");
    assert!(matches!(err, XmlError::MissingChild { .. }), "got {err:?}");
    let message = err.to_string();
    assert!(
        message
            .contains("is missing mandatory child <defining_code> (DV_CODED_TEXT.defining_code)"),
        "{message}"
    );
    assert!(
        !message.contains("Add <value/>"),
        "the remedy belongs to the absent value alone: {message}"
    );
}

#[test]
fn an_inlined_symbol_value_keeps_its_text() {
    let opt = with_first_symbol(&format!(
        "<value>ordinal symbol rubric</value>{DEFINING_CODE}"
    ));
    let template =
        openehr_its::opt14::from_xml(&opt).expect("a symbol may carry the rubric inline");
    let round_trip = openehr_its::opt14::to_xml(&template).expect("the template serializes");
    assert!(
        round_trip.contains("<symbol><value>ordinal symbol rubric</value>"),
        "the inlined symbol text did not survive the read"
    );
}
