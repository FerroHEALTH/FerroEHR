// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The date/time/duration constraint-pattern validators.
//!
//! The valid-pattern tables of `ADL2/master04.5-cadl_primitive_types.adoc`
//! (identically `ADL1.4/master05-cadl.adoc` §Patterns): field shapes, the
//! `??`/`XX` degradation chain, the duration designator order, and the
//! timezone-modifier handling the ISO-8601 pattern text helpers below
//! implement. One `impl` block over the `Parser` state of [`crate::parse`].

use crate::error::SyntaxErrorCode;
use crate::parse::{PResult, Parser};

impl Parser<'_> {
    // ── constraint-pattern validators (`master04.5` valid-pattern tables) ──

    pub(crate) fn validate_date_pattern(&mut self, p: &str, code: SyntaxErrorCode) -> PResult<()> {
        // Fields: year(4)-month(2)-day(2). Degradation: after a `??` field, only
        // `??`/`XX`; after `XX`, only `XX`. A date pattern carries NO timezone
        // modifier — `master05` §Patterns L896 admits one on "any of the time or
        // date/time (but not date) patterns".
        let fields: Vec<&str> = p.split('-').collect();
        let [year, month, day] = fields.as_slice() else {
            return self.pattern_err(code, p);
        };
        if !is_year_field(year) {
            return self.pattern_err(code, p);
        }
        self.validate_pattern_degradation(&[month, day], code, p)
    }

    pub(crate) fn validate_time_pattern(&mut self, p: &str, code: SyntaxErrorCode) -> PResult<()> {
        let fields: Vec<&str> = pattern_time_core(p).split(':').collect();
        let [hour, minute, second] = fields.as_slice() else {
            return self.pattern_err(code, p);
        };
        if !is_present_field(hour, "hh") {
            return self.pattern_err(code, p);
        }
        self.validate_pattern_degradation(&[minute, second], code, p)
    }

    pub(crate) fn validate_date_time_pattern(
        &mut self,
        p: &str,
        code: SyntaxErrorCode,
    ) -> PResult<()> {
        // The date/time separator is `T` or a space: the chapter's own
        // `V_ISO8601_DATE_TIME_CONSTRAINT_PATTERN` spells `…[dD?X][dD?X][ T]…`
        // (`ADL1.4/master05-cadl.adoc` §Symbols L1422) and its assumed-value
        // example uses the space form (`yyyy-mm-dd hh:mm:XX; 1800-01-01T00:00:00`,
        // §Assumed Values L1018).
        let Some((date, time)) = p.split_once(['T', ' ']) else {
            return self.pattern_err(code, p);
        };
        let date_fields: Vec<&str> = date.split('-').collect();
        let time_fields: Vec<&str> = pattern_time_core(time).split(':').collect();
        let [year, date_month, date_day] = date_fields.as_slice() else {
            return self.pattern_err(code, p);
        };
        let [hour, minute, second] = time_fields.as_slice() else {
            return self.pattern_err(code, p);
        };
        if !is_year_field(year) {
            return self.pattern_err(code, p);
        }
        // Degradation flows date → time as one chain (`master04.5`): the hour
        // field may itself be `??`/`XX` once the date has degraded.
        self.validate_pattern_degradation(&[date_month, date_day, hour, minute, second], code, p)
    }

    /// Duration designator-order check: `P[Y][M][W][D][T[H][M][S]]`
    /// (`master04.6` §SCDUPT). The lexer already enforces order; this catches
    /// an empty pattern.
    pub(crate) fn validate_duration_pattern(
        &mut self,
        p: &str,
        code: SyntaxErrorCode,
    ) -> PResult<()> {
        let up = p.to_ascii_uppercase();
        if !up.starts_with('P') || up == "P" || up == "PT" {
            return self.pattern_err(code, p);
        }
        Ok(())
    }

    /// After a `??` (optional) field only `??`/`XX` may follow; after an `XX`
    /// (not-allowed) field only `XX`.
    fn validate_pattern_degradation(
        &mut self,
        fields: &[&str],
        code: SyntaxErrorCode,
        full: &str,
    ) -> PResult<()> {
        let mut seen_optional = false;
        let mut seen_absent = false;
        for f in fields {
            let is_absent = f.eq_ignore_ascii_case("XX");
            let is_optional = *f == "??";
            let is_present = !is_absent && !is_optional;
            if seen_absent && !is_absent {
                return self.pattern_err(code, full);
            }
            if seen_optional && is_present {
                return self.pattern_err(code, full);
            }
            seen_optional |= is_optional;
            seen_absent |= is_absent;
        }
        Ok(())
    }

    fn pattern_err<T>(&mut self, code: SyntaxErrorCode, p: &str) -> PResult<T> {
        let span = self.span_at(self.pos.saturating_sub(1));
        self.push(code, format!("invalid constraint pattern {p:?}"), span);
        Err(())
    }
}

// ── ISO-8601 pattern text helpers ─────────────────────────────────────────

/// Whether a date/time pattern field is the "present" placeholder (e.g. `hh`)
/// or a literal date/time number substituted for it.
///
/// `ADL1.4/master05-cadl.adoc` §Patterns L894: "In the above patterns, the
/// 'yyyy' etc match strings can be replaced by literal date/time numbers. For
/// example, `yyyy-??-XX` could be transformed into `1995-??-XX`". A literal
/// field constrains the value to exactly that number, so it is "present" in
/// the same sense the placeholder is — which is what the degradation rules
/// (L860-861) range over.
fn is_present_field(f: &str, present: &str) -> bool {
    f.eq_ignore_ascii_case(present) || is_literal_field(f, 2)
}

/// Whether an ISO8601 time / date-time literal carries a timezone modifier
/// (`Z` or a `±hh[:mm]` offset). Only the part after the `T` is examined, so
/// the `-` separators of the date part are never mistaken for a sign
/// (`base_lexer.g4` `ISO8601_DATE_TIME` / `ISO8601_TIME`).
pub(crate) fn iso_has_timezone(v: &str) -> bool {
    let tail = v.split_once('T').map_or(v, |(_, t)| t);
    tail.ends_with('Z') || tail.contains('+') || tail.contains('-')
}

/// The year field: the `yyyy`/`yyy` placeholder or a literal 4-digit year
/// (`master05` §Patterns L894).
fn is_year_field(f: &str) -> bool {
    f.eq_ignore_ascii_case("yyyy") || f.eq_ignore_ascii_case("yyy") || is_literal_field(f, 4)
}

/// Whether `f` is exactly `width` ASCII digits — a literal-substituted field.
fn is_literal_field(f: &str, width: usize) -> bool {
    f.len() == width && f.bytes().all(|b| b.is_ascii_digit())
}

/// The time part of a pattern with its timezone modifier stripped.
///
/// The modifier is `Z` or a sign-led `hh`/`hh:mm`/`hhmm` group; the sign is
/// `+`, `-` or the literal `±` — `master05` §Patterns L852 ("the addition of a
/// patterns such as `+hh:mm`, `+hhmm`, and `-hh`") and the
/// `<<timezone_constraints>>` table L900-906, whose `±` rows are glossed
/// "commencing with '+' or '-'". A time never contains `+`/`-`/`Z` otherwise,
/// so the split is unambiguous.
fn pattern_time_core(t: &str) -> &str {
    t.split(['+', '-', '\u{00B1}', 'Z']).next().unwrap_or(t)
}
