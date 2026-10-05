// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariants for `DV_PERIODIC_TIME_SPECIFICATION`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_periodic_time_specification.adoc`
//! — `Value_valid: value.formalism.is_equal("HL7:PIVL") or
//! value.formalism.is_equal("HL7:EIVL")`.

use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
use crate::v1_2::data_types::time_specification::dv_periodic_time_specification::DvPeriodicTimeSpecification;
use crate::v1_2::validate::valid_iso8601_duration;
use openehr_base::validate::{InvariantViolation, Validate};

impl DvPeriodicTimeSpecification {
    /// The period of the repetition, or `None` for a specification that states
    /// none.
    ///
    /// Spec: `dv_periodic_time_specification.adoc` §Functions `period` — "The
    /// period of the repetition, computationally derived from the syntax
    /// representation. Extracted from the `value` attribute", over the
    /// phase-linked grammar of `master08-time_specification_package.adoc`
    /// §Phase-linked Time Specification Syntax: `phase = interval "/" "("
    /// difference ")"`.
    ///
    /// The class types this `1..1`, but only the phase-linked (`HL7:PIVL`)
    /// grammar has a `difference` production at all: the event-linked
    /// (`HL7:EIVL`) one is `event [ offset ]`, an offset from a real-world
    /// event rather than a period. An event-linked specification therefore has
    /// no period to return, and saying so is more honest than manufacturing
    /// one.
    #[must_use]
    pub fn period(&self) -> Option<DvDuration> {
        let value = self.phase_linked()?;
        let difference = value
            .split_once("/(")
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(difference, _)| difference)?;
        let value = iso_duration(difference)?;
        Some(DvDuration {
            value,
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            normal_range: None,
            normal_status: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
        })
    }

    /// Calendar alignment extracted from `value`, or `None` when none is
    /// stated.
    ///
    /// Spec: `dv_periodic_time_specification.adoc` §Functions
    /// `calendar_alignment`, over `pure_phase_linked_time_spec = phase [ "@"
    /// alignment ]` — the alignment is a term from the `HL7::CalendarCycle`
    /// domain (`DW` = day of week, `DM` = day of month, …), and the production
    /// makes it optional.
    #[must_use]
    pub fn calendar_alignment(&self) -> Option<String> {
        let value = self.phase_linked()?;
        let alignment = value.rsplit_once('@').map(|(_, alignment)| alignment)?;
        let alignment = alignment
            .strip_suffix(INSTITUTION_SPECIFIED)
            .unwrap_or(alignment);
        (!alignment.is_empty()).then(|| alignment.to_owned())
    }

    /// Event alignment extracted from `value`, or `None` when none is stated.
    ///
    /// Spec: `dv_periodic_time_specification.adoc` §Functions
    /// `event_alignment`, over `event_linked_time_spec = event | event offset`
    /// — the event is a term from the `HL7::TimingEvent` domain (`PC` = after
    /// meal, `HS` = bedtime, …), and it is the leading token, before any `+`
    /// or `-` offset.
    #[must_use]
    pub fn event_alignment(&self) -> Option<String> {
        let value = self.event_linked()?;
        let event = value.split(['+', '-']).next().unwrap_or(value).trim();
        (!event.is_empty()).then(|| event.to_owned())
    }

    /// Whether the timing is institution-specified.
    ///
    /// Spec: `dv_periodic_time_specification.adoc` §Functions
    /// `institution_specified` — "Extracted from value", over
    /// `phase_linked_time_spec = pure_phase_linked_time_spec [ "IST" ]`. The
    /// flag exists only in the phase-linked grammar, so its absence — including
    /// on every event-linked specification — is `false`.
    #[must_use]
    pub fn institution_specified(&self) -> bool {
        self.phase_linked()
            .is_some_and(|value| value.ends_with(INSTITUTION_SPECIFIED))
    }

    /// This specification's `value` when it is phase-linked, trimmed.
    fn phase_linked(&self) -> Option<&str> {
        (self.value.formalism == PHASE_LINKED).then(|| self.value.value.trim())
    }

    /// This specification's `value` when it is event-linked, trimmed.
    fn event_linked(&self) -> Option<&str> {
        (self.value.formalism == EVENT_LINKED).then(|| self.value.value.trim())
    }
}

/// The `HL7:PIVL` formalism: a phase-linked periodic time specification.
const PHASE_LINKED: &str = "HL7:PIVL";

/// The `HL7:EIVL` formalism: an event-linked periodic time specification.
const EVENT_LINKED: &str = "HL7:EIVL";

/// The trailing flag marking a phase-linked timing as institution-specified.
const INSTITUTION_SPECIFIED: &str = "IST";

/// A syntax `difference` as an ISO-8601 duration value.
///
/// The vendored grammar annotates `difference` as "ISO 8601 for time
/// difference", while the spec's own examples in the same section spell it
/// `7d` and `1mo`. Both are accepted: refusing the second would refuse the
/// specification's own worked examples, and inventing a third form would serve
/// nobody. A difference in neither form has no duration.
fn iso_duration(difference: &str) -> Option<String> {
    let difference = difference.trim();
    if valid_iso8601_duration(difference) {
        return Some(difference.to_owned());
    }
    let boundary = difference.find(|c: char| !c.is_ascii_digit() && c != '-')?;
    let (count, unit) = difference.split_at_checked(boundary)?;
    if count.is_empty()
        || !count
            .trim_start_matches('-')
            .chars()
            .all(|c| c.is_ascii_digit())
    {
        return None;
    }
    // The HL7 unit abbreviations the section's examples use, mapped onto the
    // ISO-8601 designator each one names.
    let (designator, timed) = match unit {
        "a" => ("Y", false),
        "mo" => ("M", false),
        "wk" => ("W", false),
        "d" => ("D", false),
        "h" => ("H", true),
        "min" => ("M", true),
        "s" => ("S", true),
        _ => return None,
    };
    let separator = if timed { "PT" } else { "P" };
    Some(format!("{separator}{count}{designator}"))
}

impl Validate for DvPeriodicTimeSpecification {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.value.formalism != "HL7:PIVL" && self.value.formalism != "HL7:EIVL" {
            out.push(InvariantViolation::here(
                "Invariant Value_valid failed on type DV_PERIODIC_TIME_SPECIFICATION \
                 (value.formalism must be HL7:PIVL or HL7:EIVL)",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::encapsulated::dv_parsable::DvParsable;

    fn spec(formalism: &str) -> DvPeriodicTimeSpecification {
        DvPeriodicTimeSpecification {
            value: DvParsable {
                charset: None,
                language: None,
                value: "[20260711T1000]".to_owned(),
                formalism: formalism.to_owned(),
            },
        }
    }

    /// `Value_valid` (`dv_periodic_time_specification.adoc`): the inner
    /// parsable's formalism must be HL7:PIVL or HL7:EIVL.
    #[test]
    fn formalism_is_constrained() {
        let mut out = Vec::new();
        spec("HL7:PIVL").validate_invariants(&mut out);
        assert!(out.is_empty(), "{out:?}");
        let mut out = Vec::new();
        spec("HL7:EIVL").validate_invariants(&mut out);
        assert!(out.is_empty(), "{out:?}");
        let mut out = Vec::new();
        spec("ISO8601").validate_invariants(&mut out);
        assert!(
            out.iter().any(|m| m.message.contains("Value_valid")),
            "{out:?}"
        );
    }

    fn parsable(formalism: &str, value: &str) -> DvPeriodicTimeSpecification {
        DvPeriodicTimeSpecification {
            value: DvParsable {
                charset: None,
                language: None,
                value: value.to_owned(),
                formalism: formalism.to_owned(),
            },
        }
    }

    /// The section's own first worked example: "[200004181100;200004181110]/(7d)@DW
    /// = every Tuesday from 11:00 to 11:10 AM."
    #[test]
    fn the_specs_weekly_example_parses() {
        let weekly = parsable("HL7:PIVL", "[200004181100;200004181110]/(7d)@DW");
        assert_eq!(
            weekly.period().expect("a phase has a difference").value,
            "P7D"
        );
        assert_eq!(weekly.calendar_alignment().as_deref(), Some("DW"));
        assert!(!weekly.institution_specified());
        assert!(weekly.event_alignment().is_none(), "not event-linked");
    }

    /// The section's second example: "[200004181100;200004181110]/(1mo)@DM =
    /// every 18th of the month 11:00 to 11:10 AM."
    #[test]
    fn the_specs_monthly_example_parses() {
        let monthly = parsable("HL7:PIVL", "[200004181100;200004181110]/(1mo)@DM");
        assert_eq!(monthly.period().expect("a difference").value, "P1M");
        assert_eq!(monthly.calendar_alignment().as_deref(), Some("DM"));
    }

    /// The grammar annotates `difference` as ISO-8601 while the same section's
    /// examples spell it `7d`; both are accepted, and the alignment is optional.
    #[test]
    fn an_iso_difference_and_a_missing_alignment_are_both_accepted() {
        let iso = parsable("HL7:PIVL", "[;]/(P7D)");
        assert_eq!(iso.period().expect("a difference").value, "P7D");
        assert!(iso.calendar_alignment().is_none(), "no @ alignment given");

        let hourly = parsable("HL7:PIVL", "[;]/(6h)");
        assert_eq!(hourly.period().expect("a difference").value, "PT6H");
        let minutes = parsable("HL7:PIVL", "[;]/(50min)");
        assert_eq!(minutes.period().expect("a difference").value, "PT50M");
    }

    /// `phase_linked_time_spec = pure_phase_linked_time_spec [ "IST" ]` — the
    /// flag is trailing, and it does not become part of the alignment.
    #[test]
    fn the_institution_specified_flag_is_read_off_the_end() {
        let flagged = parsable("HL7:PIVL", "[;]/(7d)@DWIST");
        assert!(flagged.institution_specified());
        assert_eq!(
            flagged.calendar_alignment().as_deref(),
            Some("DW"),
            "the flag is not part of the alignment term"
        );
        assert!(!parsable("HL7:PIVL", "[;]/(7d)@DW").institution_specified());
    }

    /// The section's event-linked examples: "PC+[1h;1h]" = one hour after meal,
    /// "HS-[50min;1h]" = one hour before bedtime. The event is the leading term
    /// and there is no period — the event-linked grammar has no `difference`.
    #[test]
    fn the_specs_event_linked_examples_parse() {
        let after_meal = parsable("HL7:EIVL", "PC+[1h;1h]");
        assert_eq!(after_meal.event_alignment().as_deref(), Some("PC"));
        assert!(after_meal.period().is_none(), "an event has no period");
        assert!(after_meal.calendar_alignment().is_none());
        assert!(!after_meal.institution_specified());

        let bedtime = parsable("HL7:EIVL", "HS-[50min;1h]");
        assert_eq!(bedtime.event_alignment().as_deref(), Some("HS"));

        // `event_linked_time_spec = event | event offset` — a bare event.
        assert_eq!(
            parsable("HL7:EIVL", "AC").event_alignment().as_deref(),
            Some("AC")
        );
    }

    /// A difference in neither form yields no period, rather than a duration
    /// invented from text nobody can parse.
    #[test]
    fn an_unreadable_difference_has_no_period() {
        assert!(parsable("HL7:PIVL", "[;]/(every week)").period().is_none());
        assert!(parsable("HL7:PIVL", "[;]/(7furlongs)").period().is_none());
        assert!(parsable("HL7:PIVL", "[;]").period().is_none(), "no phase");
    }
}
