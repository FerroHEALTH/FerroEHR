// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

use super::xsd_double_lexical;

/// Every `Real` shape against the `xs:double` lexical space (XML Schema
/// Part 2 §3.2.5), including the three special values a bare
/// `f64::to_string` spells wrongly.
#[test]
fn real_values_take_the_xsd_double_lexical_form() {
    // Whole reals keep the decimal point openEHR writes.
    assert_eq!(xsd_double_lexical(120.0), "120.0");
    assert_eq!(xsd_double_lexical(0.0), "0.0");
    assert_eq!(xsd_double_lexical(-0.0), "-0.0");
    assert_eq!(xsd_double_lexical(-7.0), "-7.0");
    // Fractional reals round-trip through the shortest form.
    assert_eq!(xsd_double_lexical(5.66), "5.66");
    assert_eq!(
        xsd_double_lexical(32.299_869_242_485_19),
        "32.29986924248519"
    );
    // The special values: `INF` / `-INF` / `NaN`, never Rust's spellings.
    assert_eq!(xsd_double_lexical(f64::INFINITY), "INF");
    assert_eq!(xsd_double_lexical(f64::NEG_INFINITY), "-INF");
    assert_eq!(xsd_double_lexical(f64::NAN), "NaN");
    assert_ne!(xsd_double_lexical(f64::INFINITY), f64::INFINITY.to_string());
}

/// The emitted lexeme must parse back to the same value — the fidelity
/// property the round-trip gates rest on.
#[test]
fn every_emitted_lexeme_parses_back() {
    for v in [
        120.0_f64,
        -0.0,
        5.66,
        32.299_869_242_485_19,
        1e21,
        1e-7,
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        let text = xsd_double_lexical(v);
        let back: f64 = text
            .parse()
            .unwrap_or_else(|e| panic!("{text:?} does not parse back: {e}"));
        assert_eq!(back.to_bits(), v.to_bits(), "{text:?}");
    }
    assert!(
        xsd_double_lexical(f64::NAN)
            .parse::<f64>()
            .is_ok_and(f64::is_nan)
    );
}
