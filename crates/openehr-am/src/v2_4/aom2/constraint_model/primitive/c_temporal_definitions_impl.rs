// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_TEMPORAL_DEFINITIONS` spec functions and constant
//! tables.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_temporal_definitions.adoc`
//! §Attributes (the four constant tables, printed verbatim there) + §Functions,
//! `AM/docs/AOM2/master04.2-constraint_model-semantics.adoc` §Date/Time
//! Constraints (the lexical-element → `VALIDITY_KIND` mapping), and
//! `AM/docs/ADL2/master04.5-cadl_primitive_types.adoc` §Patterns (the pattern
//! table and the timezone-modifier rules).

use crate::v2_4::aom2::constraint_model::primitive::c_temporal_definitions::CTemporalDefinitions;
use openehr_base::containers::NonEmptyVec;
use openehr_base::v1_3::base_types::definitions::validity_kind::ValidityKind;
use std::collections::BTreeMap;

impl Default for CTemporalDefinitions {
    /// The constant tables `c_temporal_definitions.adoc` §Attributes prints.
    ///
    /// NOTE: that page spells two `valid_time_constraint_replacements` keys
    /// `"HH-??-??"`/`"HH-??-XX"` while its own `valid_time_constraint_patterns`
    /// list and every other table spell time slots with `':'`; the colon
    /// spelling is used here, since the hyphen form matches no declared pattern.
    fn default() -> Self {
        Self {
            valid_date_constraint_patterns: non_empty(&[
                "YYYY-MM-DD",
                "YYYY-MM-??",
                "YYYY-MM-XX",
                "YYYY-??-??",
                "YYYY-??-XX",
                "YYYY-XX-XX",
            ]),
            valid_date_constraint_replacements: replacements(&[
                ("YYYY-MM-DD", &[]),
                ("YYYY-MM-??", &["YYYY-MM-DD", "YYYY-MM-XX"]),
                ("YYYY-MM-XX", &[]),
                (
                    "YYYY-??-??",
                    &[
                        "YYYY-MM-??",
                        "YYYY-MM-DD",
                        "YYYY-MM-XX",
                        "YYYY-??-XX",
                        "YYYY-XX-XX",
                    ],
                ),
                ("YYYY-??-XX", &["YYYY-MM-XX", "YYYY-XX-XX"]),
                ("YYYY-XX-XX", &[]),
            ]),
            valid_time_constraint_patterns: non_empty(&[
                "HH:MM:SS", "HH:MM:??", "HH:MM:XX", "HH:??:??", "HH:??:XX",
            ]),
            valid_time_constraint_replacements: replacements(&[
                ("HH:MM:SS", &[]),
                ("HH:MM:??", &["HH:MM:SS", "HH:MM:XX"]),
                ("HH:MM:XX", &[]),
                (
                    "HH:??:??",
                    &["HH:MM:??", "HH:MM:SS", "HH:MM:XX", "HH:??:XX"],
                ),
                ("HH:??:XX", &["HH:MM:XX"]),
            ]),
            valid_date_time_constraint_patterns: non_empty(&[
                "YYYY-MM-DDTHH:MM:SS",
                "YYYY-MM-DDTHH:MM:??",
                "YYYY-MM-DDTHH:MM:XX",
                "YYYY-MM-DDTHH:??:??",
                "YYYY-MM-DDTHH:??:XX",
                "YYYY-??-??T??:??:??",
            ]),
            valid_date_time_constraint_replacements: replacements(&[
                ("YYYY-MM-DDTHH:MM:SS", &[]),
                (
                    "YYYY-MM-DDTHH:MM:??",
                    &["YYYY-MM-DDTHH:MM:SS", "YYYY-MM-DDTHH:MM:XX"],
                ),
                ("YYYY-MM-DDTHH:MM:XX", &[]),
                (
                    "YYYY-MM-DDTHH:??:??",
                    &[
                        "YYYY-MM-DDTHH:??:XX",
                        "YYYY-MM-DDTHH:MM:SS",
                        "YYYY-MM-DDTHH:MM:??",
                        "YYYY-MM-DDTHH:MM:XX",
                    ],
                ),
                ("YYYY-MM-DDTHH:??:XX", &["YYYY-MM-DDTHH:MM:XX"]),
                (
                    "YYYY-??-??T??:??:??",
                    &[
                        "YYYY-MM-DDTHH:MM:SS",
                        "YYYY-MM-DDTHH:MM:??",
                        "YYYY-MM-DDTHH:MM:XX",
                        "YYYY-MM-DDTHH:??:??",
                        "YYYY-MM-DDTHH:??:XX",
                    ],
                ),
            ]),
        }
    }
}

impl CTemporalDefinitions {
    /// Returns true if `s` is a declared date constraint pattern.
    ///
    /// `valid_iso8601_date_constraint_pattern`
    /// (`c_temporal_definitions.adoc` §Functions): true if the pattern "is in
    /// `valid_date_constraint_patterns`". Patterns are compared
    /// case-insensitively, since the sibling replacement functions on `C_DATE`
    /// compare `as_upper` forms and `ADL2 master04.5` §Patterns prints the same
    /// patterns in lower case.
    #[must_use]
    pub fn valid_iso8601_date_constraint_pattern(&self, s: &str) -> bool {
        contains_ignoring_case(&self.valid_date_constraint_patterns, s)
    }

    /// Returns true if `s` is a declared time constraint pattern.
    ///
    /// `valid_iso8601_time_constraint_pattern` (`c_temporal_definitions.adoc`
    /// §Functions): true if the pattern "is in `valid_time_constraint_patterns`".
    /// A timezone modifier may be appended to any time pattern
    /// (`ADL2 master04.5` §Patterns), so it is stripped before the lookup.
    #[must_use]
    pub fn valid_iso8601_time_constraint_pattern(&self, s: &str) -> bool {
        let (base, _) = split_timezone(s);
        contains_ignoring_case(&self.valid_time_constraint_patterns, base)
    }

    /// Returns true if `s` is a declared date/time constraint pattern.
    ///
    /// `valid_iso8601_date_time_constraint_pattern`
    /// (`c_temporal_definitions.adoc` §Functions): true if the pattern "is in
    /// `valid_date_time_constraint_patterns`", with any appended timezone
    /// modifier stripped first (`ADL2 master04.5` §Patterns).
    #[must_use]
    pub fn valid_iso8601_date_time_constraint_pattern(&self, s: &str) -> bool {
        let (base, _) = split_timezone(s);
        contains_ignoring_case(&self.valid_date_time_constraint_patterns, base)
    }

    /// Returns true if `s` is a well-formed duration constraint pattern.
    ///
    /// `valid_iso8601_duration_constraint_pattern`
    /// (`c_temporal_definitions.adoc` §Functions): true if the string is of the
    /// form `P[Y|y][M|m][W|w][D|d][T[H|h][M|m][S|s]]`. Duration patterns have
    /// no table of their own — the class page states the grammar inline, and
    /// notes that mixing `W` with the other designators is an openEHR deviation
    /// from ISO 8601.
    #[must_use]
    #[expect(
        clippy::unused_self,
        reason = "the class page declares this as an instance function of C_TEMPORAL_DEFINITIONS; duration patterns are the one family with no constant table, so the grammar is stated inline instead"
    )]
    pub fn valid_iso8601_duration_constraint_pattern(&self, s: &str) -> bool {
        if !matches!(s.chars().next(), Some('P' | 'p')) {
            return false;
        }
        let rest: String = s.chars().skip(1).collect::<String>().to_uppercase();
        let (date_part, time_part) = match rest.split_once('T') {
            Some((d, t)) => (d, Some(t)),
            None => (rest.as_str(), None),
        };
        if !is_designator_subsequence(date_part, "YMWD") {
            return false;
        }
        match time_part {
            Some(t) => is_designator_subsequence(t, "HMS"),
            None => true,
        }
    }

    /// Returns true if `other_dur` admits every duration element `a_dur` allows.
    ///
    /// `valid_duration_constraint_replacement` (`c_temporal_definitions.adoc`
    /// §Functions): "True if ISO8601 duration string `other_dur` contains every
    /// character element in `a_dur`. For example: 'PYD' … conforms to 'PYMD',
    /// but doesn't conform to 'PY'." Designators are compared case-insensitively,
    /// since the pattern grammar admits either case.
    #[must_use]
    #[expect(
        clippy::unused_self,
        reason = "the class page declares this as an instance function of C_TEMPORAL_DEFINITIONS; the duration family has no replacement table, so the rule is character containment between the two patterns"
    )]
    pub fn valid_duration_constraint_replacement(&self, a_dur: &str, other_dur: &str) -> bool {
        let other: String = other_dur.to_uppercase();
        a_dur
            .to_uppercase()
            .chars()
            .all(|element| other.contains(element))
    }
}

/// The `VALIDITY_KIND` a pattern slot denotes.
///
/// `master04.2-constraint_model-semantics.adoc` §Date/Time Constraints: `??` →
/// optional, `XX` → prohibited, and a field letter (or a literal value
/// substituted for it) → mandatory.
pub(crate) fn slot_validity(slot: &str) -> ValidityKind {
    match slot.to_uppercase().as_str() {
        "??" => ValidityKind::Optional,
        "XX" => ValidityKind::Prohibited,
        _ => ValidityKind::Mandatory,
    }
}

/// The `-`-separated slots of a date pattern (`YYYY`, `MM`, `DD`).
pub(crate) fn date_slots(pattern: &str) -> Vec<&str> {
    pattern.split('-').collect()
}

/// The `:`-separated slots of a time pattern (`HH`, `MM`, `SS`), timezone
/// modifier removed.
pub(crate) fn time_slots(pattern: &str) -> Vec<&str> {
    let (base, _) = split_timezone(pattern);
    base.split(':').collect()
}

/// Whether a time or date/time pattern requires a timezone.
///
/// `ADL2 master04.5` §Patterns: an appended `±hh` / `±hh:mm` / `±hhmm` / `Z`
/// modifier makes a timezone required, "the absence of a timezone constraint
/// indicates that a timezone modifier is optional", and "there is no way to
/// state that timezone information be prohibited".
pub(crate) fn timezone_validity(pattern: &str) -> ValidityKind {
    let (_, timezone) = split_timezone(pattern);
    if timezone.is_some() {
        ValidityKind::Mandatory
    } else {
        ValidityKind::Optional
    }
}

/// Splits a time or date/time pattern into its base and its timezone modifier.
fn split_timezone(pattern: &str) -> (&str, Option<&str>) {
    if let Some(base) = pattern
        .strip_suffix('Z')
        .or_else(|| pattern.strip_suffix('z'))
    {
        return (base, Some("Z"));
    }
    // A '+' never occurs in a bare pattern, and a '-' only inside the date part
    // (before the 'T'), so the timezone sign is the first such character after
    // the time part begins.
    let time_start = pattern.find('T').map_or(0, |i| i + 1);
    let tail = pattern.split_at_checked(time_start).map(|(_, t)| t);
    let Some(tail) = tail else {
        return (pattern, None);
    };
    match tail.find(['+', '-']) {
        Some(offset) => match pattern.split_at_checked(time_start + offset) {
            Some((base, tz)) => (base, Some(tz)),
            None => (pattern, None),
        },
        None => (pattern, None),
    }
}

/// Whether every character of `s` is a designator from `allowed`, in order and
/// without repetition.
fn is_designator_subsequence(s: &str, allowed: &str) -> bool {
    let mut remaining = allowed.chars();
    s.chars()
        .all(|c| remaining.any(|designator| designator == c))
}

/// Case-insensitive membership in a pattern list.
fn contains_ignoring_case(patterns: &NonEmptyVec<String>, s: &str) -> bool {
    let wanted = s.to_uppercase();
    patterns.iter().any(|p| p.to_uppercase() == wanted)
}

/// A non-empty pattern list from a literal slice.
fn non_empty(values: &[&str]) -> NonEmptyVec<String> {
    let owned: Vec<String> = values.iter().map(|v| (*v).to_owned()).collect();
    #[expect(
        clippy::expect_used,
        reason = "every call site passes a non-empty literal slice from the spec's own constant tables, so an EmptyContainer is unreachable"
    )]
    NonEmptyVec::new(owned).expect("a spec constant table should never be empty")
}

/// A replacement table from literal pairs.
fn replacements(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(key, values)| {
            (
                (*key).to_owned(),
                values.iter().map(|v| (*v).to_owned()).collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_declared_date_patterns_are_the_only_valid_ones() {
        let d = CTemporalDefinitions::default();
        assert!(d.valid_iso8601_date_constraint_pattern("YYYY-??-XX"));
        assert!(d.valid_iso8601_date_constraint_pattern("yyyy-??-xx"));
        assert!(!d.valid_iso8601_date_constraint_pattern("YYYY-XX-DD"));
    }

    #[test]
    fn a_timezone_modifier_does_not_invalidate_a_time_pattern() {
        let d = CTemporalDefinitions::default();
        assert!(d.valid_iso8601_time_constraint_pattern("HH:MM:SS"));
        assert!(d.valid_iso8601_time_constraint_pattern("HH:MM:SSZ"));
        assert!(d.valid_iso8601_time_constraint_pattern("HH:MM:SS+HH:MM"));
        assert!(!d.valid_iso8601_time_constraint_pattern("HH:XX:SS"));
    }

    #[test]
    fn date_time_patterns_are_matched_across_the_t_separator() {
        let d = CTemporalDefinitions::default();
        assert!(d.valid_iso8601_date_time_constraint_pattern("YYYY-??-??T??:??:??"));
        assert!(d.valid_iso8601_date_time_constraint_pattern("YYYY-MM-DDTHH:MM:XX+HH"));
        assert!(!d.valid_iso8601_date_time_constraint_pattern("YYYY-MM-DDTHH:XX:SS"));
    }

    #[test]
    fn duration_patterns_follow_the_designator_grammar() {
        let d = CTemporalDefinitions::default();
        assert!(d.valid_iso8601_duration_constraint_pattern("PYMWD"));
        assert!(d.valid_iso8601_duration_constraint_pattern("PDTHMS"));
        assert!(d.valid_iso8601_duration_constraint_pattern("P"));
        assert!(d.valid_iso8601_duration_constraint_pattern("Pwd"));
        assert!(!d.valid_iso8601_duration_constraint_pattern("PDY"));
        assert!(!d.valid_iso8601_duration_constraint_pattern("PX"));
        assert!(!d.valid_iso8601_duration_constraint_pattern("YMD"));
    }

    #[test]
    fn a_duration_replacement_must_carry_every_element_of_the_original() {
        let d = CTemporalDefinitions::default();
        assert!(d.valid_duration_constraint_replacement("PYD", "PYMD"));
        assert!(!d.valid_duration_constraint_replacement("PYD", "PY"));
        assert!(d.valid_duration_constraint_replacement("PYD", "PYD"));
    }

    #[test]
    fn slots_map_to_the_declared_validity_kinds() {
        assert_eq!(slot_validity("??"), ValidityKind::Optional);
        assert_eq!(slot_validity("XX"), ValidityKind::Prohibited);
        assert_eq!(slot_validity("MM"), ValidityKind::Mandatory);
        assert_eq!(slot_validity("1995"), ValidityKind::Mandatory);
    }

    #[test]
    fn timezone_is_optional_unless_a_modifier_is_appended() {
        assert_eq!(timezone_validity("HH:MM:SS"), ValidityKind::Optional);
        assert_eq!(timezone_validity("HH:MM:SSZ"), ValidityKind::Mandatory);
        assert_eq!(timezone_validity("HH:MM:SS+HH:MM"), ValidityKind::Mandatory);
        assert_eq!(
            timezone_validity("YYYY-MM-DDTHH:MM:SS"),
            ValidityKind::Optional
        );
        assert_eq!(
            timezone_validity("YYYY-MM-DDTHH:MM:SS-HH"),
            ValidityKind::Mandatory
        );
    }

    #[test]
    fn slot_splitting_ignores_the_timezone_modifier() {
        assert_eq!(date_slots("YYYY-??-XX"), vec!["YYYY", "??", "XX"]);
        assert_eq!(time_slots("HH:??:XX+HH:MM"), vec!["HH", "??", "XX"]);
    }
}
