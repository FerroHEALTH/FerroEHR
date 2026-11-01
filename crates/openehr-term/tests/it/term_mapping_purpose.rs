// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The vendored Support Terminology carries SPECTERM-31: `interoperability`
//! (code 672) in the term mapping purpose vocabulary.
//!
//! `docs/specs/openehr/TERM/docs/SupportTerminology/master00-amendment_record.adoc`
//! records the change, and `codesets/openehr_terminology-vocabularies.adoc`
//! lists the `term_mapping_purpose` row. The bundle reads it out of the
//! embedded computable assets in every shipped language.

use openehr_term::bundle::openehr;

/// Code 672 is a valid `TERM_MAPPING.purpose`, with its rubric in each
/// language the assets ship.
#[test]
fn specterm_31_interoperability_is_a_term_mapping_purpose() {
    let bundle = openehr();
    assert!(bundle.is_valid_code("term_mapping_purpose", "672"));
    assert_eq!(
        bundle.rubric("term_mapping_purpose", "672", "en"),
        Some("interoperability")
    );
    for language in ["es", "ja", "pt", "zh"] {
        assert!(
            bundle
                .rubric("term_mapping_purpose", "672", language)
                .is_some_and(|rubric| !rubric.is_empty()),
            "672 has a rubric in {language}"
        );
    }
    assert!(!bundle.is_valid_code("term_mapping_purpose", "673"));
}
