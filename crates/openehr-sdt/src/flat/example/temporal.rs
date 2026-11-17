// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The example value of a `DV_DATE`, `DV_TIME` or `DV_DATE_TIME` leaf.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use serde_json::Value;

use crate::flat::validation::leaf::temporal_in_range;
use crate::flat::webtemplate::model::WebTemplateNode;

/// A deterministic temporal example honouring the leaf's AOM 1.4 temporal
/// pattern (`yyyy-??-XX` etc.: literal = mandatory, `??` = optional/kept,
/// `XX` = prohibited/omitted — mirroring the validator's presence-only
/// judgement) and the `timezone_validity` constraint (`1003` = the designator
/// must be absent; the fixed example instants already carry `Z` for the rest).
pub(super) fn example_temporal(node: &WebTemplateNode, rm: &str) -> String {
    let pattern = node
        .inputs
        .first()
        .and_then(|i| i.validation.as_ref())
        .and_then(|v| v.pattern.as_deref());
    let (pat_date, pat_time) = match pattern {
        Some(p) if p.contains('T') => {
            let (d, t) = p.split_once('T').unwrap_or((p, ""));
            (d, t)
        }
        Some(p) if p.contains(':') => ("", p),
        Some(p) => (p, ""),
        None => ("", ""),
    };
    let tz = if node.tz_validity == Some(1003) {
        ""
    } else {
        "Z"
    };
    let date = temporal_segments(pat_date, DATE_SEGMENTS, '-');
    let value = match rm {
        "DV_DATE" => date,
        "DV_TIME" => format!("{}{tz}", temporal_segments(pat_time, TIME_SEGMENTS, ':')),
        // DV_DATE_TIME: a time part prohibited outright (pattern `…TXX:…`)
        // truncates to the date.
        _ if pat_time.starts_with("XX") => date,
        _ => format!(
            "{date}T{}{tz}",
            temporal_segments(pat_time, TIME_SEGMENTS, ':')
        ),
    };
    // NOTE: AOM 1.4 `C_DATE_TIME.range`: outside the template's range, the example takes
    // its included lower bound, else its included upper bound.
    let range = node
        .inputs
        .first()
        .and_then(|i| i.validation.as_ref())
        .and_then(|v| v.range.as_ref());
    match range {
        Some(r) if !temporal_in_range(&value, r) => {
            [(&r.min, &r.min_op, ">"), (&r.max, &r.max_op, "<")]
                .into_iter()
                .find_map(|(b, op, open)| {
                    b.as_ref()
                        .and_then(Value::as_str)
                        .filter(|_| op.as_deref() != Some(open))
                })
                .map_or(value, str::to_owned)
        }
        _ => value,
    }
}

/// The example date's segments, coarsest first.
const DATE_SEGMENTS: [&str; 3] = ["2022", "02", "03"];

/// The example time's segments, coarsest first.
const TIME_SEGMENTS: [&str; 3] = ["04", "05", "06"];

/// Renders one temporal half, dropping every segment the pattern prohibits.
///
/// The leading segment is always present; the rest are kept until the pattern
/// marks one `XX`, since ISO 8601 partial precision truncates right-to-left —
/// an omitted month forces the day off with it.
fn temporal_segments(pattern: &str, full: [&str; 3], separator: char) -> String {
    let segments: Vec<&str> = if pattern.is_empty() {
        Vec::new()
    } else {
        pattern.splitn(3, separator).collect()
    };
    let mut out = String::from(full[0]);
    for (i, part) in full.iter().enumerate().skip(1) {
        if segments.get(i).is_some_and(|s| *s == "XX") {
            break;
        }
        out.push(separator);
        out.push_str(part);
    }
    out
}
