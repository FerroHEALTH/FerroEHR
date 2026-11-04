// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 → ADL 2 code shift: a 1.4 at-code becomes the ADL 2 code one
//! number up, and the converter allocates new value-set codes above every
//! code the source already uses. No openEHR spec governs the numbering: it is
//! pinned by the ADL 2 reference conversion fixtures.

use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

use crate::adl14::walk::walk_constraints;

/// The highest ac-number the source already uses, across its terminology
/// entries and its bare constraint refs.
///
/// Synthesised and minted acs must allocate ABOVE the shifted range, so a
/// fresh `acN` never collides with an existing `ac000(N-1)` → `acN`.
pub(super) fn highest_ac_number(
    definition: &CComplexObject,
    terminology: &ArchetypeTerminology,
) -> i64 {
    let mut highest = 0i64;
    let mut track = |code: &str| {
        if AdlCodeDefinitionsData::is_value_set_code(code)
            && !code.contains('.')
            && let Some(n) = first_num(&shift_code(code, "ac"))
        {
            highest = highest.max(n);
        }
    };
    for terms in terminology.term_definitions.values() {
        for code in terms.keys() {
            track(code);
        }
    }
    walk_constraints(definition, &mut |raw, _| {
        let body = raw.split_once(';').map_or(raw, |(b, _)| b);
        if !body.contains("::") {
            track(body.trim());
        }
    });
    highest
}

/// Shift a 1.4 code to an ADL2 code with the given `prefix` (`"id"`/`"at"`).
/// The first segment's number is incremented by one (`at0000`→`id1`,
/// `at0003.1`→`id4.1`), except the specialisation new-code prefix `at0`
/// (`at0.89`→`id0.89` — number kept). No openEHR spec governs this — fixture-pinned.
pub(super) fn shift_code(code: &str, prefix: &str) -> String {
    let bare = code
        .trim_start_matches("at")
        .trim_start_matches("id")
        .trim_start_matches("ac");
    let mut segs = bare.split('.');
    let Some(first) = segs.next() else {
        return code.to_owned();
    };
    let rest: Vec<&str> = segs.collect();
    let has_suffix = !rest.is_empty();
    let shifted_first = if first == "0" && has_suffix {
        // `at0.K` new-at-level code: keep the 0.
        "0".to_owned()
    } else {
        match first.parse::<i64>() {
            Ok(n) => (n + 1).to_string(),
            Err(_) => first.to_owned(),
        }
    };
    if has_suffix {
        format!("{prefix}{shifted_first}.{}", rest.join("."))
    } else {
        format!("{prefix}{shifted_first}")
    }
}

pub(super) fn first_num(code: &str) -> Option<i64> {
    let bare = code
        .trim_start_matches("at")
        .trim_start_matches("id")
        .trim_start_matches("ac");
    bare.split('.').next()?.parse::<i64>().ok()
}
