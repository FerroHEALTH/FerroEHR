// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! `DV_TEXT` and `DV_CODED_TEXT` invariants.
//!
//! RM 1.2.0 `org.openehr.rm.data_types.dv_text.adoc`: `Valid_value` and
//! `Formatting_valid`. `Mappings_valid` is checked on the JSON body;
//! `Language_valid` and `Encoding_valid` are terminology-bound.

use crate::v1_2::data_types::text::dv_text::DvText;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for DvText {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        match self {
            DvText::DvText(t) => t.validate_invariants(out),
            DvText::DvCodedText(t) => t.validate_invariants(out),
        }
    }
}

impl Validate for crate::v1_2::data_types::text::dv_text::DvTextData {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        text_invariants("DV_TEXT", &self.value, self.formatting.as_deref(), out);
    }
}

impl Validate for crate::v1_2::data_types::text::dv_coded_text::DvCodedText {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        text_invariants("DV_CODED_TEXT", &self.value, self.formatting.as_deref(), out);
    }
}

/// The `DV_TEXT` invariants, which `DV_CODED_TEXT` inherits.
fn text_invariants(
    ty: &str,
    value: &str,
    formatting: Option<&str>,
    out: &mut Vec<InvariantViolation>,
) {
    crate::v1_2::validate::generated::dv_text_core(ty, value, formatting, out);
    // NOTE: RM data_types master05 §Text package, "plain_no_newlines": "newlines
    // are not allowed"; other values are open.
    if formatting == Some("plain_no_newlines") && value.contains(['\n', '\r']) {
        out.push(InvariantViolation::here(format!(
            "formatting \"plain_no_newlines\" forbids newlines in value on type {ty} \
             (RM data_types master05 §Text package)"
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::text::dv_text::DvTextData;

    fn text(value: &str) -> DvText {
        DvText::DvText(DvTextData {
            value: value.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
        })
    }

    /// `Valid_value`: an empty rubric is rejected; `Formatting_valid`: a
    /// present-empty formatting string is rejected (`dv_text.adoc`).
    #[test]
    fn value_and_formatting_invariants() {
        let mut out = Vec::new();
        text("ok").validate_invariants(&mut out);
        assert!(out.is_empty(), "non-empty value passes: {out:?}");

        let mut out = Vec::new();
        text("").validate_invariants(&mut out);
        assert!(
            out.iter().any(|m| m.message.contains("Valid_value")),
            "empty value must fail: {out:?}"
        );

        let mut with_fmt = text("ok");
        if let DvText::DvText(t) = &mut with_fmt {
            t.formatting = Some(String::new());
        }
        let mut out = Vec::new();
        with_fmt.validate_invariants(&mut out);
        assert!(
            out.iter().any(|m| m.message.contains("Formatting_valid")),
            "empty formatting must fail: {out:?}"
        );
    }

    fn formatted(value: &str, formatting: &str) -> DvText {
        let mut t = text(value);
        if let DvText::DvText(d) = &mut t {
            d.formatting = Some(formatting.to_owned());
        }
        t
    }

    /// `plain_no_newlines` refuses a newline; any other formatting allows one.
    #[test]
    fn plain_no_newlines_forbids_newlines_and_nothing_else() {
        let mut out = Vec::new();
        formatted("one line", "plain_no_newlines").validate_invariants(&mut out);
        assert!(out.is_empty(), "no newline passes: {out:?}");

        for value in ["two\nlines", "carriage\rreturn"] {
            let mut out = Vec::new();
            formatted(value, "plain_no_newlines").validate_invariants(&mut out);
            assert!(
                out.iter().any(|m| m.message.contains("plain_no_newlines")),
                "{value:?} must fail under plain_no_newlines: {out:?}"
            );
        }

        for formatting in ["plain", "markdown", "font-weight : bold;", "custom-name"] {
            let mut out = Vec::new();
            formatted("two\nlines", formatting).validate_invariants(&mut out);
            assert!(
                out.is_empty(),
                "the value space stays open — {formatting:?} with a newline passes: {out:?}"
            );
        }
        let mut out = Vec::new();
        text("two\nlines").validate_invariants(&mut out);
        assert!(out.is_empty(), "Void formatting allows newlines: {out:?}");
    }
}
