// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `C_PRIMITIVE` leaves, temporal/duration pattern validity, and the
//! `C_DOMAIN_TYPE` assumed-value rules for the OPT 1.4 pass
//! (T9, T9b, T9c, T10, T12).
//!
//! The `C_PRIMITIVE` family (`AM/docs/ADL1.4/master05-cadl.adoc` §Primitive
//! Types; AOM1.4 `c_boolean`/`c_string`/`c_integer`/`c_real`/`c_date`/`c_time`/
//! `c_date_time`/`c_duration` class files) carries per-leaf invariants — boolean
//! satisfiability, `Assumed_value_valid`, and the ISO-8601 constraint-pattern
//! `Pattern_validity` — decidable structurally on the flattened OPT. The
//! openEHR-profile `C_DOMAIN_TYPE`s (`C_DV_QUANTITY`, `C_DV_ORDINAL`,
//! `C_CODE_PHRASE`, `C_CODE_REFERENCE`) add `property`/assumed-value checks
//! (UML `c_quantity`/`c_ordinal`/`c_coded_text`; ADL1.4 master09).

use openehr_base::prelude::CodePhrase;
use openehr_its::opt14::types::{
    CBoolean, CDvOrdinal, CDvQuantity, CInteger, CPrimitive, CReal, CString,
};

use super::RuleViolation;
use super::interval::{int_in_range, real_in_range};

/// The `C_PRIMITIVE`-level checks: `C_BOOLEAN` satisfiability, `C_DEFINED_OBJECT`
/// `Assumed_value_valid` for list/range-constrained primitives, and the
/// `C_DATE`/`C_TIME`/`C_DATE_TIME` `Pattern_validity` + `C_DURATION` pattern
/// syntax.
pub(super) fn check_primitive(p: &CPrimitive, node_id: &str) -> Result<(), RuleViolation> {
    match p {
        CPrimitive::CBoolean(c) => check_boolean(c, node_id),
        CPrimitive::CString(c) => check_string(c, node_id),
        CPrimitive::CInteger(c) => check_integer(c, node_id),
        CPrimitive::CReal(c) => check_real(c, node_id),
        // C_DATE/C_DATE_TIME/C_TIME invariant Pattern_validity:
        // `pattern /= Void implies valid_iso8601_*_constraint_pattern(pattern)`
        // (AOM1.4 c_date/c_date_time/c_time class files); the legal patterns
        // and the optional→optional/disallowed, disallowed→disallowed
        // field-ordering are cADL §Constraints on Dates/Times (ADL1.4 master05
        // lines 858–892).
        CPrimitive::CDate(c) => {
            check_pattern(c.pattern.as_deref(), node_id, "date", valid_date_pattern)
        }
        CPrimitive::CTime(c) => {
            check_pattern(c.pattern.as_deref(), node_id, "time", valid_time_pattern)
        }
        CPrimitive::CDateTime(c) => check_pattern(
            c.pattern.as_deref(),
            node_id,
            "date-time",
            valid_date_time_pattern,
        ),
        // C_DURATION: the pattern must be `P[Y][M][W][D][T[H][M][S]]` — openEHR
        // deviates from strict ISO 8601 by allowing `W` to be mixed with the
        // other designators (cADL §Duration Constraints, ADL1.4 master05 lines
        // 934–980).
        CPrimitive::CDuration(c) => check_pattern(
            c.pattern.as_deref(),
            node_id,
            "duration",
            valid_duration_pattern,
        ),
    }
}

/// `C_BOOLEAN` satisfiability and `Assumed_value_valid`.
///
/// `true_valid` and `false_valid` cannot both be False — the constraint would
/// be unsatisfiable (AOM1.4 `c_boolean` class file, Description).
fn check_boolean(c: &CBoolean, node_id: &str) -> Result<(), RuleViolation> {
    if !c.true_valid && !c.false_valid {
        return Err(RuleViolation::new(
            "C_BOOLEAN_validity",
            format!(
                "node '{node_id}': true_valid and false_valid are both false — the \
                 boolean constraint is unsatisfiable"
            ),
        ));
    }
    let Some(assumed) = c.assumed_value else {
        return Ok(());
    };
    let permitted = if assumed { c.true_valid } else { c.false_valid };
    if permitted {
        return Ok(());
    }
    Err(RuleViolation::new(
        "Assumed_value_valid",
        format!(
            "node '{node_id}': the assumed boolean value {assumed} is not \
             permitted by the true_valid/false_valid flags"
        ),
    ))
}

/// `C_STRING` `Assumed_value_valid` against a closed value list (AOM1.4
/// `c_string` class file; cADL string constraints are case-sensitive).
fn check_string(c: &CString, node_id: &str) -> Result<(), RuleViolation> {
    if let Some(assumed) = &c.assumed_value
        && !c.list.is_empty()
        && c.list_open != Some(true)
        && !c.list.contains(assumed)
    {
        return Err(RuleViolation::new(
            "Assumed_value_valid",
            format!(
                "node '{node_id}': the assumed string '{assumed}' is not in the closed \
                 value list"
            ),
        ));
    }
    Ok(())
}

/// `C_INTEGER` `Assumed_value_valid` against the constrained list and range.
fn check_integer(c: &CInteger, node_id: &str) -> Result<(), RuleViolation> {
    let Some(assumed) = c.assumed_value else {
        return Ok(());
    };
    let list_ok = c.list.is_empty() || c.list.contains(&assumed);
    let range_ok = c.range.as_ref().is_none_or(|r| int_in_range(assumed, r));
    if list_ok && range_ok {
        return Ok(());
    }
    Err(RuleViolation::new(
        "Assumed_value_valid",
        format!(
            "node '{node_id}': the assumed integer {assumed} is outside the \
             constrained list/range"
        ),
    ))
}

/// `C_REAL` `Assumed_value_valid` against the constrained list and range.
fn check_real(c: &CReal, node_id: &str) -> Result<(), RuleViolation> {
    let Some(assumed) = c.assumed_value else {
        return Ok(());
    };
    let list_ok = c.list.is_empty() || c.list.contains(&assumed);
    let range_ok = c.range.as_ref().is_none_or(|r| real_in_range(assumed, r));
    if list_ok && range_ok {
        return Ok(());
    }
    Err(RuleViolation::new(
        "Assumed_value_valid",
        format!(
            "node '{node_id}': the assumed real {assumed} is outside the \
             constrained list/range"
        ),
    ))
}

/// The `Pattern_validity` invariant of one temporal or duration leaf: a
/// present pattern must satisfy its class's constraint-pattern grammar.
fn check_pattern(
    pattern: Option<&str>,
    node_id: &str,
    kind: &str,
    valid: fn(&str) -> bool,
) -> Result<(), RuleViolation> {
    match pattern {
        Some(pattern) if !valid(pattern) => Err(pattern_violation(node_id, pattern, kind)),
        _ => Ok(()),
    }
}

fn pattern_violation(node_id: &str, pattern: &str, kind: &str) -> RuleViolation {
    RuleViolation::new(
        "Pattern_validity",
        format!("node '{node_id}': '{pattern}' is not a valid {kind} constraint pattern"),
    )
}

// ─── C_DOMAIN_TYPE assumed-value + property ────────────────────────────────

/// `C_DEFINED_OBJECT` invariant `Assumed_value_valid` for the code-carrying
/// domain types (`C_CODE_PHRASE` / `C_CODE_REFERENCE`): the assumed code must be
/// one of the constrained codes when the code list is closed and non-empty.
pub(super) fn check_assumed_code(
    assumed: Option<&CodePhrase>,
    code_list: &[String],
    node_id: &str,
) -> Result<(), RuleViolation> {
    if let Some(assumed) = assumed
        && !code_list.is_empty()
        && !code_list.contains(&assumed.code_string)
    {
        return Err(RuleViolation::new(
            "Assumed_value_valid",
            format!(
                "node '{node_id}': the assumed code '{}' is not in the constrained code list",
                assumed.code_string
            ),
        ));
    }
    Ok(())
}

/// `C_DV_ORDINAL` `Assumed_value_valid` (AOM1.4 `c_defined_object` class file):
/// the assumed ordinal must be one of the constrained (symbol, value) pairs.
pub(super) fn check_dv_ordinal(c: &CDvOrdinal, node_id: &str) -> Result<(), RuleViolation> {
    if let Some(assumed) = &c.assumed_value
        && !c.list.is_empty()
        && !c.list.iter().any(|o| o.value == assumed.value)
    {
        return Err(RuleViolation::new(
            "Assumed_value_valid",
            format!(
                "node '{node_id}': the assumed DV_ORDINAL value {} is not one of the constrained \
                 ordinal values",
                assumed.value
            ),
        ));
    }
    Ok(())
}

/// `C_DV_QUANTITY` `Property_valid` + `Assumed_value_valid` (UML `c_quantity`;
/// RM support master05 §"Terms and Codes in the openEHR Reference Model",
/// `Group_id_property`).
pub(super) fn check_dv_quantity(c: &CDvQuantity, node_id: &str) -> Result<(), RuleViolation> {
    // The measurement property must be a member of the openEHR `property`
    // terminology group — checked when the constraint codes it from the openEHR
    // terminology.
    if let Some(property) = &c.property
        && property.terminology_id.value.eq_ignore_ascii_case("openehr")
        // NOTE (prior-art OPT tolerance): Ocean Template Designer emits the
        // placeholder property code "0" for an unconstrained property — a
        // placeholder is "no constraint", not a foreign code.
        && !property.code_string.is_empty()
        && property.code_string != "0"
        && !openehr_term::bundle::openehr().is_valid_property(&property.code_string)
    {
        return Err(RuleViolation::new(
            "Property_valid",
            format!(
                "node '{node_id}': DV_QUANTITY property code '{}' is not in the openEHR \
                 'property' terminology group",
                property.code_string
            ),
        ));
    }
    // Assumed_value_valid: the assumed quantity's units must be one of the
    // constrained unit items, and its magnitude inside that item's magnitude
    // range.
    if let Some(assumed) = &c.assumed_value
        && !c.list.is_empty()
    {
        let Some(item) = c.list.iter().find(|i| i.units == assumed.units) else {
            return Err(RuleViolation::new(
                "Assumed_value_valid",
                format!(
                    "node '{node_id}': the assumed DV_QUANTITY units '{}' are not among the \
                     constrained units",
                    assumed.units
                ),
            ));
        };
        if let Some(range) = &item.magnitude
            && !real_in_range(assumed.magnitude, range)
        {
            return Err(RuleViolation::new(
                "Assumed_value_valid",
                format!(
                    "node '{node_id}': the assumed DV_QUANTITY magnitude {} is outside the \
                     constrained magnitude range for units '{}'",
                    assumed.magnitude, assumed.units
                ),
            ));
        }
    }
    Ok(())
}

// ─── temporal + duration constraint-pattern validity (T9b, T9c) ──────────────────

/// `yyyy-<mm|??|XX>-<dd|??|XX>` with the field-ordering rule: optional (`??`)
/// may be followed only by optional/disallowed; disallowed (`XX`) only by
/// disallowed (ADL1.4 master05 lines 858–866).
pub(super) fn valid_date_pattern(p: &str) -> bool {
    let parts: Vec<&str> = p.split('-').collect();
    let [y, m, d] = parts.as_slice() else {
        return false;
    };
    *y == "yyyy" && field_chain_valid(&[(m, "mm"), (d, "dd")])
}

/// `<HH|??|XX>:<MM|??|XX>:<SS|??|XX>` with the same field-ordering rule and an
/// optional trailing timezone requirement (`Z` / `±hh` / `±hh:mm` / `±hhmm` —
/// ADL1.4 master05 lines 852–854, 896–910: a timezone can be required, never
/// prohibited).
pub(super) fn valid_time_pattern(p: &str) -> bool {
    let body = p
        .strip_suffix('Z')
        .or_else(|| strip_tz_offset(p))
        .unwrap_or(p);
    let parts: Vec<&str> = body.split(':').collect();
    let [h, m, s] = parts.as_slice() else {
        return false;
    };
    *h == "HH" && field_chain_valid(&[(m, "MM"), (s, "SS")])
}

/// `<date>T<time>` (ADL1.4 master05 lines 868–892). The date and time fields
/// form ONE monotonic ordering chain (Month→Day→Hour→Minute→Second — the
/// `C_DATE_TIME` `*_validity_optional`/`*_validity_disallowed` invariants).
/// Unlike `C_TIME`, `C_DATE_TIME` has an `hour_validity`, so `??`/`XX` hours are
/// legal here (e.g. `yyyy-??-??T??:??:??`, the CNF RIPPLE conformance
/// template).
pub(super) fn valid_date_time_pattern(pattern: &str) -> bool {
    let Some((date, time)) = pattern.split_once('T') else {
        return false;
    };
    let date_parts: Vec<&str> = date.split('-').collect();
    let [y, mo, dy] = date_parts.as_slice() else {
        return false;
    };
    let body = time
        .strip_suffix('Z')
        .or_else(|| strip_tz_offset(time))
        .unwrap_or(time);
    let time_parts: Vec<&str> = body.split(':').collect();
    let [h, mi, s] = time_parts.as_slice() else {
        return false;
    };
    *y == "yyyy" && field_chain_valid(&[(mo, "mm"), (dy, "dd"), (h, "HH"), (mi, "MM"), (s, "SS")])
}

/// One monotonic field chain: mandatory (`mm`/`dd`/…) → any; optional (`??`) →
/// optional or disallowed; disallowed (`XX`) → disallowed.
fn field_chain_valid(fields: &[(&&str, &str)]) -> bool {
    let mut state = 0u8; // 0 = mandatory so far, 1 = optional seen, 2 = disallowed seen
    for (actual, mandatory_form) in fields {
        let level = if **actual == *mandatory_form {
            0
        } else if **actual == "??" {
            1
        } else if **actual == "XX" {
            2
        } else {
            return false;
        };
        if level < state {
            return false;
        }
        state = state.max(level);
    }
    true
}

/// A time-pattern timezone suffix `±hh`, `±hh:mm`, or `±hhmm` — strip it if
/// present (the pattern grammar writes them literally, e.g. `HH:MM:SS+hh:mm`).
fn strip_tz_offset(p: &str) -> Option<&str> {
    for suffix in ["+hh:mm", "-hh:mm", "+hhmm", "-hhmm", "+hh", "-hh"] {
        if let Some(body) = p.strip_suffix(suffix) {
            return Some(body);
        }
    }
    None
}

/// `P` followed by an in-order subset of `Y M W D`, optionally `T` + an
/// in-order non-empty subset of `H M S`; at least one designator overall.
pub(super) fn valid_duration_pattern(p: &str) -> bool {
    let Some(rest) = p.strip_prefix('P') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };
    if !in_order_subset(date_part, &['Y', 'M', 'W', 'D']) {
        return false;
    }
    match time_part {
        Some(t) => !t.is_empty() && in_order_subset(t, &['H', 'M', 'S']),
        None => !date_part.is_empty(),
    }
}

/// `s` uses only characters from `order`, each at most once, in order.
fn in_order_subset(s: &str, order: &[char]) -> bool {
    let mut pos = 0usize;
    for c in s.chars() {
        let Some(i) = order
            .get(pos..)
            .and_then(|rest| rest.iter().position(|o| *o == c))
        else {
            return false;
        };
        pos += i + 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{
        valid_date_pattern, valid_date_time_pattern, valid_duration_pattern, valid_time_pattern,
    };

    #[test]
    fn duration_pattern_syntax() {
        // Every corpus form must validate; out-of-order or foreign designators must
        // not.
        for ok in [
            "PD", "PDTH", "PDTHM", "PDTHMS", "PMTS", "PTH", "PTHMS", "PTM", "PTS", "PWD", "PWDTH",
            "PY", "PYM", "PYMWD", "PYMWDTH",
        ] {
            assert!(valid_duration_pattern(ok), "{ok} must be valid");
        }
        for bad in ["P", "PT", "PDY", "PTMH", "PX", "YMD", "PYY"] {
            assert!(!valid_duration_pattern(bad), "{bad} must be invalid");
        }
    }

    #[test]
    fn temporal_pattern_validity_forms() {
        for ok in [
            ("yyyy-mm-dd", "date"),
            ("yyyy-??-??", "date"),
            ("yyyy-mm-XX", "date"),
            ("yyyy-??-XX", "date"),
        ] {
            assert!(valid_date_pattern(ok.0), "{} must be valid", ok.0);
        }
        assert!(!valid_date_pattern("yyyy-XX-??"));
        assert!(valid_time_pattern("HH:MM:SS"));
        assert!(valid_time_pattern("HH:??:XX"));
        assert!(!valid_time_pattern("??:MM:SS"));
        assert!(valid_date_time_pattern("yyyy-mm-ddTHH:MM:SS"));
        assert!(valid_date_time_pattern("yyyy-??-??T??:??:??"));
        assert!(!valid_date_time_pattern("yyyy-??-??THH:MM:SS"));
    }
}
