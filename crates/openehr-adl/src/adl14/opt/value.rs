// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The value helpers: occurrences, binding URIs, code constraints, terms,
//! multiplicities and intervals.

/// Build the 1.4 `C_TERMINOLOGY_CODE.constraint` encoding
/// (`terminology::code[,code…][;assumed]`) the converter rewrites. An empty code
/// list yields an empty (unconstrained) string.
use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
use openehr_base::prelude::{
    Cardinality, Interval, MultiplicityInterval, PointInterval, ProperInterval, ProperIntervalData,
    TerminologyCode,
};
use openehr_its::opt14;

pub(super) fn code_constraint(
    terminology: Option<&str>,
    codes: &[String],
    assumed: Option<&str>,
) -> String {
    if codes.is_empty() {
        return String::new();
    }
    let term = terminology.unwrap_or("local");
    let mut out = format!("{term}::{}", codes.join(","));
    if let Some(a) = assumed {
        out.push(';');
        out.push_str(a);
    }
    out
}

pub(super) fn map_term(t: &opt14::types::ArchetypeTerm) -> ArchetypeTerm {
    let text = t.items.get("text").cloned().unwrap_or_default();
    let description = t.items.get("description").cloned().unwrap_or_default();
    let other_items: BTreeMap<String, String> = t
        .items
        .iter()
        .filter(|(k, _)| k.as_str() != "text" && k.as_str() != "description")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // Keep the map absent rather than empty for the common no-extras case.
    let other_items = (!other_items.is_empty()).then_some(other_items);
    ArchetypeTerm {
        code: t.code.clone(),
        text,
        description,
        other_items,
    }
}

pub(super) fn map_mult(iv: &opt14::types::Intervalofinteger) -> MultiplicityInterval {
    MultiplicityInterval {
        lower: iv.lower,
        upper: iv.upper,
        lower_unbounded: iv.lower_unbounded,
        upper_unbounded: iv.upper_unbounded,
        lower_included: iv.lower_included.unwrap_or(!iv.lower_unbounded),
        upper_included: iv.upper_included.unwrap_or(!iv.upper_unbounded),
    }
}

pub(super) fn map_cardinality(c: &opt14::types::Cardinality) -> Cardinality {
    Cardinality {
        interval: map_mult(&c.interval),
        is_ordered: c.is_ordered,
        is_unique: c.is_unique,
    }
}

/// Returns the AOM2 interval list for an AOM 1.4 numeric constraint: each
/// enumerated `list` value as a point interval, then the `range`.
///
/// AOM 1.4 `C_INTEGER`/`C_REAL` carry a `list` and a `range`
/// (AM `UML/classes/org.openehr.am.aom14.c_integer.adoc`); AOM2 has one
/// `constraint: List<Interval<T>>`, any member of which admits a value
/// (`org.openehr.am.aom2.c_integer.adoc`).
pub(super) fn enumerated_and_range<T: Copy>(
    list: &[T],
    range: Option<Interval<T>>,
) -> Vec<Interval<T>> {
    list.iter()
        .map(|&v| {
            Interval::PointInterval(PointInterval {
                lower: Some(v),
                upper: Some(v),
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: true,
                upper_included: true,
            })
        })
        .chain(range)
        .collect()
}

pub(super) fn int_interval(iv: &opt14::types::Intervalofinteger) -> Interval<i32> {
    if iv.lower == iv.upper && iv.lower.is_some() {
        let v = iv.lower.unwrap_or_default();
        return Interval::PointInterval(PointInterval {
            lower: Some(v),
            upper: Some(v),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        });
    }
    Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
        lower: iv.lower,
        upper: iv.upper,
        lower_unbounded: iv.lower_unbounded,
        upper_unbounded: iv.upper_unbounded,
        lower_included: iv.lower_included.unwrap_or(!iv.lower_unbounded),
        upper_included: iv.upper_included.unwrap_or(!iv.upper_unbounded),
    }))
}

pub(super) fn real_interval(iv: &opt14::types::Intervalofreal) -> Interval<f64> {
    if iv.lower == iv.upper && iv.lower.is_some() {
        let v = iv.lower.unwrap_or_default();
        return Interval::PointInterval(PointInterval {
            lower: Some(v),
            upper: Some(v),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        });
    }
    Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
        lower: iv.lower,
        upper: iv.upper,
        lower_unbounded: iv.lower_unbounded,
        upper_unbounded: iv.upper_unbounded,
        lower_included: iv.lower_included.unwrap_or(!iv.lower_unbounded),
        upper_included: iv.upper_included.unwrap_or(!iv.upper_unbounded),
    }))
}

pub(super) fn term_code(terminology_id: &str, code: &str) -> TerminologyCode {
    TerminologyCode {
        terminology_id: terminology_id.to_owned(),
        terminology_version: None,
        code_string: code.to_owned(),
        uri: None,
    }
}
