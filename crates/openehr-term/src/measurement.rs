// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Measurement-service helpers.
//!
//! `MEASUREMENT_SERVICE.is_valid_units_string`
//! (RM support `measurement_service.adoc`: "True if the units string is a
//! valid string according to the HL7 UCUM specification"; RM data_types
//! master06: `DV_QUANTITY.units` is a UCUM case-sensitive code by default).
//!
//! A SYNTAX validator over the UCUM term grammar (unitsofmeasure.org §2.2
//! "Syntax and Semantics"): it carries no UCUM atom table, so it validates the
//! shape, not whether an atom is a registered unit.
//!
//! NOTE: RM data_types master06 declares no `Units_valid` invariant on
//! `DV_QUANTITY`, so commit-time rejection of non-UCUM units is not wired;
//! template-declared unit constraints go through the conformance walker.

/// `MEASUREMENT_SERVICE.is_valid_units_string`: UCUM syntax validity
/// (case-sensitive form).
#[must_use]
pub fn is_valid_units_string(units: &str) -> bool {
    let bytes = units.as_bytes();
    if units.is_empty() || !units.is_ascii() {
        return false;
    }
    let mut pos = 0usize;
    // mainTerm = ['/'] term
    if bytes.first() == Some(&b'/') {
        pos += 1;
    }
    match parse_term(bytes, pos) {
        Some(end) => end == bytes.len(),
        None => false,
    }
}

/// term = component { ('.' | '/') component }
fn parse_term(b: &[u8], mut pos: usize) -> Option<usize> {
    pos = parse_component(b, pos)?;
    while matches!(b.get(pos), Some(b'.' | b'/')) {
        pos = parse_component(b, pos + 1)?;
    }
    Some(pos)
}

/// component = '(' term ')' \[exponent\] | annotation | annotatable \[annotation\]
fn parse_component(b: &[u8], mut pos: usize) -> Option<usize> {
    let &opener = b.get(pos)?;
    if opener == b'(' {
        pos = parse_term(b, pos + 1)?;
        if b.get(pos) != Some(&b')') {
            return None;
        }
        pos += 1;
        pos = parse_exponent_opt(b, pos);
        return Some(pos);
    }
    if opener == b'{' {
        return parse_annotation(b, pos);
    }
    // annotatable = simple-unit [exponent]; simple-unit = factor | atom
    pos = parse_atom_or_factor(b, pos)?;
    pos = parse_exponent_opt(b, pos);
    if b.get(pos) == Some(&b'{') {
        pos = parse_annotation(b, pos)?;
    }
    Some(pos)
}

/// An atom: printable ASCII excluding the operator/meta characters, with
/// balanced `[...]` segments; or a bare integer factor.
fn parse_atom_or_factor(b: &[u8], mut pos: usize) -> Option<usize> {
    let start = pos;
    while let Some(&c) = b.get(pos) {
        match c {
            b'[' => pos = parse_bracket_segment(b, pos)?,
            b'.' | b'/' | b'(' | b')' | b'{' | b'}' | b']' => break,
            // exponent digits/sign are consumed by parse_exponent_opt; inside
            // an atom, digits terminate the symbol part (e.g. `mm3`).
            c if (0x21..=0x7e).contains(&c) => pos += 1,
            _ => return None, // whitespace / non-printable / non-ASCII
        }
    }
    (pos > start).then_some(pos)
}

/// One balanced `[...]` segment of an atom, entered on its opening bracket.
///
/// Returns the position after the closing `]`, or `None` for an unbalanced or
/// non-printable segment.
fn parse_bracket_segment(b: &[u8], mut pos: usize) -> Option<usize> {
    pos += 1;
    while let Some(&inner) = b.get(pos) {
        if inner == b']' {
            break;
        }
        if !(0x21..=0x7e).contains(&inner) || inner == b'[' {
            return None;
        }
        pos += 1;
    }
    if pos >= b.len() {
        return None; // unbalanced '['
    }
    Some(pos + 1) // past ']'
}

/// exponent = ['+'|'-'] digits — but bare trailing digits are already consumed
/// by the atom scan (`mm3`), so this handles the signed form (`s-1`, `m+2`).
fn parse_exponent_opt(b: &[u8], mut pos: usize) -> usize {
    if matches!(b.get(pos), Some(b'+' | b'-')) {
        let mut digits = pos + 1;
        while b.get(digits).is_some_and(u8::is_ascii_digit) {
            digits += 1;
        }
        if digits > pos + 1 {
            pos = digits;
        }
    }
    pos
}

/// annotation = '{' printable-except-braces '}'
fn parse_annotation(b: &[u8], mut pos: usize) -> Option<usize> {
    debug_assert_eq!(
        b.get(pos),
        Some(&b'{'),
        "parse_annotation must be entered on the opening brace of an annotation"
    );
    pos += 1;
    while let Some(&c) = b.get(pos) {
        if c == b'}' {
            break;
        }
        if !(0x21..=0x7e).contains(&c) || c == b'{' {
            return None;
        }
        pos += 1;
    }
    (pos < b.len()).then_some(pos + 1)
}

#[cfg(test)]
mod tests {
    use super::is_valid_units_string;

    /// UCUM syntax forms (unitsofmeasure.org §2.2) — accepted and rejected.
    #[test]
    fn ucum_syntax() {
        for ok in [
            "kg",
            "mm[Hg]",
            "kg/m2",
            "ms-1",
            "km/h",
            "1",
            "1/d",
            "/min",
            "10*3/ul",
            "cm2",
            "mol/l",
            "%",
            "Cel",
            "m.s-2",
            "{beats}/min",
            "mg{total}",
        ] {
            assert!(is_valid_units_string(ok), "{ok} must be valid UCUM syntax");
        }
        for bad in ["", "°C", "kg m", "mm[Hg", "a{b{c}}", "kg//m", "(kg"] {
            assert!(!is_valid_units_string(bad), "{bad:?} must be invalid");
        }
    }
}
