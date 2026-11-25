// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Leaf domain-constraint validation: the AOM 1.4 `C_DV_*` and `C_PRIMITIVE`
//! leaf checks (`master04-constraint_model_package.adoc`), approximated from the
//! WebTemplate `inputs`. Value well-formedness stays with the RM-invariant pass.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::collections::BTreeSet;

use openehr_base::v1_3::base_types::definitions::definitions_impl::LOCAL_TERMINOLOGY_ID;
use serde_json::Value;

use super::{ValidationKind, Validator};
use crate::flat::instance::Instance;
use crate::flat::webtemplate::constraint::pattern_matches;
use crate::flat::webtemplate::model::{
    WebTemplateCodedValue, WebTemplateInput, WebTemplateInputType, WebTemplateNode,
    WebTemplateRange,
};

pub(super) fn check_inputs<'i, I: Instance<'i>>(
    v: &mut Validator,
    instance: I,
    wt: &WebTemplateNode,
) {
    let rm = wt.rm_type.split('<').next().unwrap_or(&wt.rm_type);
    match rm {
        "DV_CODED_TEXT" => check_coded_text(v, instance, wt),
        // RM `data_types` §DV_STATE: the state's code sits on its `value`.
        "DV_STATE" => {
            if let Some(value) = instance.attr("value") {
                check_coded_text(v, value, wt);
            }
        }
        "CODE_PHRASE" => check_code_phrase(v, instance, wt),
        "DV_ORDINAL" | "DV_SCALE" => check_ordinal(v, instance, wt),
        "DV_QUANTITY" => check_quantity(v, instance, wt),
        "DV_COUNT" => check_count(v, instance, wt),
        "DV_PROPORTION" => check_proportion(v, instance, wt),
        "DV_TEXT" | "DV_URI" | "DV_EHR_URI" => check_text(v, instance, wt),
        "DV_BOOLEAN" => check_boolean(v, instance, wt),
        "DV_IDENTIFIER" => {
            check_string_parts(v, instance, wt, &["id", "type", "issuer", "assigner"]);
        }
        // NOTE: AOM 1.4 §`C_STRING`; master17.6 `CONT-DV_PARSABLE-validate_value_formalism`.
        "DV_PARSABLE" => check_string_parts(v, instance, wt, &["value", "formalism"]),
        "DV_DATE" | "DV_TIME" | "DV_DATE_TIME" => check_temporal(v, instance, wt),
        "DV_DURATION" => check_duration(v, instance, wt),
        _ => {}
    }
    // Constraints captured outside the `inputs` mapping (builder
    // `capture_leaf_constraints`): C_INTEGER/C_REAL lists on numeric data and
    // C_CODE_PHRASE lists on coded attributes (e.g. DV_MULTIMEDIA.media_type).
    check_numeric_lists(v, instance, wt);
    check_numeric_ranges(v, instance, wt);
    check_code_lists(v, instance, wt);
}

/// `C_INTEGER`/`C_REAL` ranges the `inputs` do not carry (`DV_MULTIMEDIA.size`;
/// an ADL 2 union of ranges, AOM2 `master04.5` §C_ORDERED).
fn check_numeric_ranges<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let attrs: BTreeSet<&str> = wt.numeric_ranges.iter().map(|(a, _)| a.as_str()).collect();
    for attr in attrs {
        let Some(value) = instance.attr(attr).and_then(Instance::as_f64) else {
            continue;
        };
        let mut ranges = wt.numeric_ranges.iter().filter(|(a, _)| a == attr);
        if !ranges.any(|(_, range)| in_range(value, range)) {
            v.push(
                &wt.aql_path,
                format!("{attr} {value} is outside the constrained range"),
                ValidationKind::RangeError,
            );
        }
    }
}

/// `C_INTEGER.list` / `C_REAL.list` membership on the leaf's numeric data
/// (AOM 1.4 `master04-constraint_model_package.adoc` §`C_INTEGER/§C_REAL)`: the
/// instance datum must be one of the enumerated values.
fn check_numeric_lists<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    for (attr, list) in &wt.numeric_lists {
        let Some(value) = instance.attr(attr).and_then(Instance::as_f64) else {
            continue;
        };
        if !list.iter().any(|allowed| (allowed - value).abs() < 1e-9) {
            v.push(
                &wt.aql_path,
                format!("{attr} {value} is not in the constrained value list"),
                ValidationKind::CodedValue,
            );
        }
    }
}

/// `C_CODE_PHRASE` code-list membership on a coded RM attribute the `inputs` do
/// not model (AOM 1.4 §`C_CODE_PHRASE`), only when the terminologies match.
fn check_code_lists<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    for cl in &wt.code_lists {
        let Some(cp) = instance.attr(&cl.attr) else {
            continue;
        };
        let code = cp.str_at("code_string");
        let term = cp
            .attr("terminology_id")
            .and_then(|t| t.attr("value"))
            .and_then(Instance::as_str);
        let Some(code) = code else { continue };
        if !terminology_matches(cl.terminology.as_deref(), term) {
            continue;
        }
        if !cl.codes.iter().any(|c| c == code) {
            v.push(
                &wt.aql_path,
                format!(
                    "{} code '{code}' is not in the constrained code list",
                    cl.attr
                ),
                ValidationKind::CodedValue,
            );
        }
    }
}

/// Whether an instance code's terminology matches the constraint's terminology
/// (`None` on the constraint side means the archetype-`local` terminology).
pub(super) fn terminology_matches(constraint: Option<&str>, instance: Option<&str>) -> bool {
    match constraint {
        None | Some(LOCAL_TERMINOLOGY_ID | "") => {
            matches!(instance, None | Some(LOCAL_TERMINOLOGY_ID | ""))
        }
        Some(t) => instance == Some(t),
    }
}

// ── coded text / code phrase / ordinal ─────────────────────────────────────────

fn check_coded_text<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let Some(code_input) = input_with_suffix(wt, "code") else {
        return;
    };
    let code = instance
        .attr("defining_code")
        .and_then(|c| c.attr("code_string"))
        .and_then(Instance::as_str);
    let terminology = instance
        .attr("defining_code")
        .and_then(|c| c.attr("terminology_id"))
        .and_then(|t| t.attr("value"))
        .and_then(Instance::as_str);
    check_code_membership(
        v,
        &wt.aql_path,
        wt.coded_terminology_local,
        code_input,
        (code, terminology),
    );
}

fn check_code_phrase<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let Some(code_input) = input_with_suffix(wt, "code").or_else(|| wt.inputs.first()) else {
        return;
    };
    let code = instance.str_at("code_string");
    let terminology = instance
        .attr("terminology_id")
        .and_then(|t| t.attr("value"))
        .and_then(Instance::as_str);
    check_code_membership(
        v,
        &wt.aql_path,
        wt.coded_terminology_local,
        code_input,
        (code, terminology),
    );
}

/// `DV_ORDINAL` / `DV_SCALE` against a `C_DV_ORDINAL` / `C_DV_SCALE` list.
///
/// The instance's (symbol, value) pair must match one entry
/// (`org.openehr.am.aom14.ordinal.adoc` §ORDINAL; `AOM2/master04.3` §Tuple
/// Constraints: "as pairs not just as allowable alternatives").
fn check_ordinal<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let Some(input) = wt.inputs.first() else {
        return;
    };
    if input.list.is_empty() || input.list_open == Some(true) {
        return;
    }
    let Some((terminology, code)) = symbol_code(instance) else {
        return;
    };

    // Entries whose symbol is the instance's code phrase: the terminology and
    // the code together (RM `data_types` §CODE_PHRASE), so `external::at0003`
    // is no member of a list of `local::` codes.
    let same_symbol: Vec<&WebTemplateCodedValue> = input
        .list
        .iter()
        .filter(|cv| cv.value == code)
        .filter(|cv| {
            let listed = cv.terminology.as_deref().or(input.terminology.as_deref());
            terminology_matches(listed, terminology)
        })
        .collect();
    if same_symbol.is_empty() {
        let shown = terminology.map_or_else(|| code.to_owned(), |t| format!("{t}::{code}"));
        v.push(
            &wt.aql_path,
            format!("ordinal symbol '{shown}' is not in the constrained value set"),
            ValidationKind::CodedValue,
        );
        return;
    }
    // AOM 1.4 has no `C_DV_SCALE`, so a DV_SCALE constrains only `symbol` as a
    // `C_CODE_PHRASE` code list (RM `data_types` §`DV_SCALE`); the pair check below
    // is for the C_DV_ORDINAL form.
    if same_symbol
        .iter()
        .all(|cv| cv.scale.is_none() && cv.ordinal.is_none())
    {
        return;
    }
    // The symbol is known; require the (symbol, value) pair to match one entry.
    let is_scale = wt.rm_type.starts_with("DV_SCALE");
    let Some(inst_value) = instance.attr("value") else {
        return; // value absent — the RM-invariant pass owns structural presence.
    };
    let pair_ok = same_symbol
        .iter()
        .any(|cv| paired_value_matches(cv, is_scale, inst_value));
    if !pair_ok {
        v.push(
            &wt.aql_path,
            format!(
                "ordinal (symbol '{code}', value {}) does not match a \
                 constrained (symbol, value) pair",
                inst_value.render()
            ),
            ValidationKind::CodedValue,
        );
    }
}

/// The instance's ordinal symbol as its terminology id and code string
/// (`symbol/defining_code`), `None` when the symbol carries no code.
fn symbol_code<'i, I: Instance<'i>>(instance: I) -> Option<(Option<&'i str>, &'i str)> {
    let defining_code = instance
        .attr("symbol")
        .and_then(|s| s.attr("defining_code"))?;
    let terminology = defining_code
        .attr("terminology_id")
        .and_then(|t| t.attr("value"))
        .and_then(Instance::as_str);
    let code = defining_code.str_at("code_string")?;
    Some((terminology, code))
}

/// Whether one constrained entry's paired numeric equals the instance value.
fn paired_value_matches<'i, I: Instance<'i>>(
    entry: &WebTemplateCodedValue,
    is_scale: bool,
    inst_value: I,
) -> bool {
    if is_scale {
        return matches!(
            (entry.scale, inst_value.as_f64()),
            (Some(s), Some(iv)) if (s - iv).abs() < 1e-9
        );
    }
    matches!(
        (entry.ordinal, inst_value.as_i64()),
        (Some(o), Some(iv)) if i64::from(o) == iv
    )
}

/// Report a coded value outside a closed option list whose terminology matches
/// the instance code's (AOM 1.4 §`C_CODE_PHRASE`).
///
/// An explicitly `local` list (`local_only`,
/// [`WebTemplateNode::coded_terminology_local`]) also refuses a code from any
/// other terminology (`org.openehr.am.aom14.c_coded_text.adoc` §C_CODED_TEXT);
/// otherwise a differently scoped code is left to the terminology pass.
pub(super) fn check_code_membership(
    v: &mut Validator,
    path: &str,
    local_only: bool,
    input: &WebTemplateInput,
    (code, terminology): (Option<&str>, Option<&str>),
) {
    if input.list.is_empty() || input.list_open == Some(true) {
        return;
    }
    let Some(code) = code else { return };
    if terminology_matches(input.terminology.as_deref(), terminology) {
        if !input.list.iter().any(|cv| cv.value == code) {
            v.push(
                path,
                format!("coded value '{code}' is not in the constrained value set"),
                ValidationKind::CodedValue,
            );
        }
        return;
    }
    // Only an explicitly `local` closed list refuses a foreign terminology.
    if local_only {
        v.push(
            path,
            format!(
                "coded value '{code}' (terminology '{}') is not in the constrained local value set",
                terminology.unwrap_or("")
            ),
            ValidationKind::CodedValue,
        );
    }
}

// ── quantity / count / proportion ──────────────────────────────────────────────

fn check_quantity<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let unit = instance.str_at("units");
    let unit_input = input_with_suffix(wt, "unit");

    // Unit membership against an enumerated C_QUANTITY_ITEM unit list.
    let has_unit_list =
        unit_input.is_some_and(|ui| !ui.list.is_empty() && ui.list_open != Some(true));
    if let (Some(ui), Some(u)) = (unit_input, unit)
        && has_unit_list
        && !ui.list.iter().any(|cv| cv.value == u)
    {
        v.push(
            &wt.aql_path,
            format!("unit '{u}' is not among the constrained units"),
            ValidationKind::CodedValue,
        );
    }

    // NOTE: `org.openehr.am.aom14.c_quantity.adoc` gives `property` no valid_value:
    // our own check over `PropertyUnitData.xml`, refusing only a dimensional mismatch.
    if let (Some(property), Some(u)) = (&wt.quantity_property, unit)
        && !has_unit_list
    {
        let bundle = openehr_term::bundle::openehr();
        let unit_matches =
            |pu: &&openehr_term::bundle::Unit| pu.text == u || pu.ucum.as_deref() == Some(u);
        // Only enforce when the constrained property is known to the table
        // (otherwise we cannot say what its units are — tolerate).
        let allowed = bundle.units_for_property(property);
        let in_property = allowed.iter().any(unit_matches);
        let known_elsewhere = bundle
            .property_units()
            .units
            .iter()
            .any(|pu| unit_matches(&pu));
        if !allowed.is_empty() && !in_property && known_elsewhere {
            v.push(
                &wt.aql_path,
                format!("unit '{u}' is not a valid unit for the constrained property '{property}'"),
                ValidationKind::CodedValue,
            );
        }
    }

    // Magnitude range: prefer the magnitude input's own range, else the range on
    // the coded value for the instance's unit.
    if let Some(mag) = instance.attr("magnitude").and_then(Instance::as_f64) {
        let range = input_with_suffix(wt, "magnitude")
            .and_then(|i| i.validation.as_ref())
            .and_then(|val| val.range.as_ref())
            .or_else(|| unit_scoped_range(unit_input, unit));
        if let Some(range) = range
            && !in_range(mag, range)
        {
            v.push(
                &wt.aql_path,
                format!("magnitude {mag} is outside the constrained range"),
                ValidationKind::RangeError,
            );
        }
    }

    // `C_QUANTITY_ITEM.precision` (`OpenehrProfile.xsd`); an absent precision
    // violates nothing.
    if let Some(prec) = instance.attr("precision").and_then(Instance::as_f64) {
        let range = input_with_suffix(wt, "magnitude")
            .and_then(|i| i.validation.as_ref())
            .and_then(|val| val.precision.as_ref())
            .or_else(|| unit_scoped_precision(unit_input, unit));
        if let Some(range) = range
            && !in_range(prec, range)
        {
            v.push(
                &wt.aql_path,
                format!("precision {prec} is outside the constrained precision range"),
                ValidationKind::RangeError,
            );
        }
    }
}

fn unit_scoped_range<'a>(
    unit_input: Option<&'a WebTemplateInput>,
    unit: Option<&str>,
) -> Option<&'a WebTemplateRange> {
    let (ui, u) = (unit_input?, unit?);
    ui.list
        .iter()
        .find(|cv| cv.value == u)
        .and_then(|cv| cv.validation.as_ref())
        .and_then(|val| val.range.as_ref())
}

/// The precision interval scoped to the instance's unit, when the constraint
/// carries per-unit validation (the `master04` example model attaches both
/// `range` and `precision` to each coded unit value).
fn unit_scoped_precision<'a>(
    unit_input: Option<&'a WebTemplateInput>,
    unit: Option<&str>,
) -> Option<&'a WebTemplateRange> {
    let (ui, u) = (unit_input?, unit?);
    ui.list
        .iter()
        .find(|cv| cv.value == u)
        .and_then(|cv| cv.validation.as_ref())
        .and_then(|val| val.precision.as_ref())
}

fn check_count<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let Some(mag) = instance.attr("magnitude").and_then(Instance::as_f64) else {
        return;
    };
    if let Some(range) = wt
        .inputs
        .first()
        .and_then(|i| i.validation.as_ref())
        .and_then(|val| val.range.as_ref())
        && !in_range(mag, range)
    {
        v.push(
            &wt.aql_path,
            format!("count {mag} is outside the constrained range"),
            ValidationKind::RangeError,
        );
    }
}

fn check_proportion<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    // `proportionTypes` (AOM 1.4 §C_INTEGER on `DV_PROPORTION.type`); empty means
    // unconstrained.
    if !wt.proportion_types.is_empty()
        && let Some(kind) = instance.attr("type").and_then(Instance::as_i64)
    {
        let name = usize::try_from(kind)
            .ok()
            .and_then(|k| crate::flat::webtemplate::PROPORTION_KINDS.get(k));
        if name.is_none_or(|n| !wt.proportion_types.iter().any(|p| p == n)) {
            v.push(
                &wt.aql_path,
                format!(
                    "proportion type {kind} is not among the permitted kinds [{}]",
                    wt.proportion_types.join(", ")
                ),
                ValidationKind::CodedValue,
            );
        }
    }
    // DV_PROPORTION.precision against the constrained interval, the same
    // contract as DV_QUANTITY (the `|precision` suffix in `master05`'s
    // DV_PROPORTION table; RM attribute 0..1, absent violates nothing).
    if let Some(prec) = instance.attr("precision").and_then(Instance::as_f64)
        && let Some(range) = wt
            .inputs
            .iter()
            .find_map(|i| i.validation.as_ref().and_then(|val| val.precision.as_ref()))
        && !in_range(prec, range)
    {
        v.push(
            &wt.aql_path,
            format!("precision {prec} is outside the constrained precision range"),
            ValidationKind::RangeError,
        );
    }
    for part in ["numerator", "denominator"] {
        let Some(value) = instance.attr(part).and_then(Instance::as_f64) else {
            continue;
        };
        if let Some(range) = input_with_suffix(wt, part)
            .and_then(|i| i.validation.as_ref())
            .and_then(|val| val.range.as_ref())
            && !in_range(value, range)
        {
            v.push(
                &wt.aql_path,
                format!("{part} {value} is outside the constrained range"),
                ValidationKind::RangeError,
            );
        }
    }
}

// ── text / boolean / identifier ────────────────────────────────────────────────

fn check_text<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let Some(input) = wt.inputs.first() else {
        return;
    };
    let Some(value) = instance.str_at("value") else {
        return;
    };
    check_string_constraints(v, wt, input, value);
}

fn check_boolean<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let Some(input) = wt.inputs.first() else {
        return;
    };
    let Some(value) = instance.attr("value").and_then(Instance::as_bool) else {
        return;
    };
    // A list restricts booleans to a single literal ("true" / "false").
    if !input.list.is_empty() && !input.list.iter().any(|cv| cv.value == value.to_string()) {
        v.push(
            &wt.aql_path,
            format!("boolean value {value} is not permitted"),
            ValidationKind::CodedValue,
        );
    }
}

/// The `C_STRING` constraints of the suffixes `parts` on their string values
/// (`DV_IDENTIFIER` and `DV_PARSABLE`, AOM 1.4 §`C_STRING`).
fn check_string_parts<'i, I: Instance<'i>>(
    v: &mut Validator,
    instance: I,
    wt: &WebTemplateNode,
    parts: &[&str],
) {
    for part in parts {
        let Some(input) = input_with_suffix(wt, part) else {
            continue;
        };
        let Some(value) = instance.str_at(part) else {
            continue;
        };
        check_string_constraints(v, wt, input, value);
    }
}

// ── temporal (C_DATE / C_TIME / C_DATE_TIME / C_DURATION, AOM 1.4) ────────────

/// `DV_DATE`/`DV_TIME`/`DV_DATE_TIME` against the `C_*` pattern and ISO range
/// (AOM 1.4 §"Constraints on dates and times").
fn check_temporal<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let Some(value) = instance.str_at("value") else {
        return;
    };
    if let Some(validation) = wt.inputs.first().and_then(|i| i.validation.as_ref()) {
        if let Some(pattern) = &validation.pattern
            && let Some(breach) = temporal_pattern_breach(pattern, value)
        {
            v.push(
                &wt.aql_path,
                format!(
                    "temporal value '{value}' does not satisfy the pattern '{pattern}': {}. \
                     {PATTERN_KEY}",
                    breach.explain(pattern)
                ),
                ValidationKind::PatternError,
            );
        }
        if let Some(range) = &validation.range
            && !temporal_in_range(value, range)
        {
            v.push(
                &wt.aql_path,
                format!("temporal value '{value}' is outside the constrained range"),
                ValidationKind::RangeError,
            );
        }
    }
    check_timezone_validity(v, wt, value);
}

/// `C_TIME`/`C_DATE_TIME` `timezone_validity`: `1001` requires a timezone
/// designator, `1003` forbids one (`org.openehr.am.aom14.c_time.adoc`).
fn check_timezone_validity(v: &mut Validator, wt: &WebTemplateNode, value: &str) {
    let Some(tzv) = wt.tz_validity else { return };
    let has_tz = has_timezone(value);
    match tzv {
        1001 if !has_tz => v.push(
            &wt.aql_path,
            format!("temporal value '{value}' is missing a mandatory timezone"),
            ValidationKind::PatternError,
        ),
        1003 if has_tz => v.push(
            &wt.aql_path,
            format!("temporal value '{value}' carries a timezone the constraint disallows"),
            ValidationKind::PatternError,
        ),
        _ => {}
    }
}

/// Whether an ISO-8601 date-time / time value carries a timezone designator
/// (`Z`, or a `+hh:mm` / `-hh:mm` offset on the time part — the date part's `-`
/// separators are never a timezone).
fn has_timezone(value: &str) -> bool {
    let time = match value.split_once('T') {
        Some((_, t)) => t,
        None if value.contains(':') => value, // a bare DV_TIME
        None => return false,
    };
    time.ends_with('Z') || time.ends_with('z') || time.rfind(['+', '-']).is_some()
}

/// `DV_DURATION`: the `C_DURATION` pattern (allowed fields — encoded by the
/// builder as which per-field inputs exist) and the ISO-8601 duration range
/// (`WebTemplateNode::duration_range`), compared on total seconds.
fn check_duration<'i, I: Instance<'i>>(v: &mut Validator, instance: I, wt: &WebTemplateNode) {
    let Some(value) = instance.str_at("value") else {
        return;
    };
    let Some(fields) = duration_fields(value) else {
        return; // Malformed ISO duration — the RM-invariant pass owns that.
    };
    check_duration_fields_allowed(v, value, &fields, wt);
    check_duration_range(v, value, &fields, wt);
}

/// Reports every duration field the `C_DURATION` pattern does not allow; a
/// full 7-field input set means no restriction.
fn check_duration_fields_allowed(
    v: &mut Validator,
    value: &str,
    fields: &[(&'static str, f64)],
    wt: &WebTemplateNode,
) {
    let allowed: Vec<&str> = wt
        .inputs
        .iter()
        .filter_map(|i| i.suffix.as_deref())
        .collect();
    if allowed.is_empty() || allowed.len() >= 7 {
        return;
    }
    for (name, _) in fields.iter().filter(|(_, n)| *n != 0.0) {
        if !allowed.contains(name) {
            v.push(
                &wt.aql_path,
                format!(
                    "duration '{value}' uses the '{name}' field the C_DURATION pattern \
                     does not allow"
                ),
                ValidationKind::PatternError,
            );
        }
    }
}

/// Reports a duration outside the node's constrained range, honouring bound
/// inclusivity.
fn check_duration_range(
    v: &mut Validator,
    value: &str,
    fields: &[(&'static str, f64)],
    wt: &WebTemplateNode,
) {
    let Some(range) = &wt.duration_range else {
        return;
    };
    let secs = duration_seconds(fields);
    let bound = |b: &Option<Value>| -> Option<f64> {
        b.as_ref()
            .and_then(Value::as_str)
            .and_then(|s| duration_fields(s).map(|f| duration_seconds(&f)))
    };
    let min_ok = bound(&range.min).is_none_or(|b| {
        if range.min_op.as_deref() == Some(">") {
            secs > b
        } else {
            secs >= b
        }
    });
    let max_ok = bound(&range.max).is_none_or(|b| {
        if range.max_op.as_deref() == Some("<") {
            secs < b
        } else {
            secs <= b
        }
    });
    if !min_ok || !max_ok {
        v.push(
            &wt.aql_path,
            format!("duration '{value}' is outside the constrained range"),
            ValidationKind::RangeError,
        );
    }
}

/// Parse an ISO-8601 duration into its named fields (the RM `DV_DURATION` value
/// syntax); `None` when the string is not a duration.
fn duration_fields(value: &str) -> Option<Vec<(&'static str, f64)>> {
    let rest = value.strip_prefix('-').unwrap_or(value);
    let rest = rest.strip_prefix('P')?;
    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, t),
        None => (rest, ""),
    };
    let mut out = Vec::new();
    push_duration_part(date_part, false, &mut out)?;
    push_duration_part(time_part, true, &mut out)?;
    Some(out)
}

/// Appends one half of an ISO-8601 duration to `out`; `in_time` reads `M` as
/// minutes. [`None`] on an unparseable half.
fn push_duration_part(part: &str, in_time: bool, out: &mut Vec<(&'static str, f64)>) -> Option<()> {
    let mut num = String::new();
    for ch in part.chars() {
        if ch.is_ascii_digit() || ch == '.' || ch == ',' {
            num.push(if ch == ',' { '.' } else { ch });
            continue;
        }
        let n: f64 = num.parse().ok()?;
        num.clear();
        out.push((duration_field_name(ch, in_time)?, n));
    }
    if num.is_empty() { Some(()) } else { None }
}

/// The `DV_DURATION` field one ISO-8601 designator names, if it names one.
fn duration_field_name(designator: char, in_time: bool) -> Option<&'static str> {
    match (designator, in_time) {
        ('Y', false) => Some("year"),
        ('M', false) => Some("month"),
        ('W', false) => Some("week"),
        ('D', false) => Some("day"),
        ('H', true) => Some("hour"),
        ('M', true) => Some("minute"),
        ('S', true) => Some("second"),
        _ => None,
    }
}

/// Total seconds of a parsed duration, using the RM's nominal field lengths
/// (year = 365.25 d, month = 30.4375 d — RM `DV_DURATION.magnitude` semantics).
fn duration_seconds(fields: &[(&'static str, f64)]) -> f64 {
    fields
        .iter()
        .map(|(name, n)| {
            n * match *name {
                "year" => 31_557_600.0,
                "month" => 2_629_800.0,
                "week" => 604_800.0,
                "day" => 86_400.0,
                "hour" => 3_600.0,
                "minute" => 60.0,
                _ => 1.0,
            }
        })
        .sum()
}

/// The one-line key to the temporal pattern symbols a refusal quotes (ADL 1.4
/// `master05-cadl.adoc` §"Date, Time and Date/Time" Patterns).
const PATTERN_KEY: &str = "Pattern key: a letter field (yyyy, mm, dd, HH, MM, SS) is required, \
                           ?? is optional, XX is not allowed.";

/// The date and time fields of a temporal pattern, in pattern order.
const DATE_FIELDS: [&str; 3] = ["year", "month", "day"];
const TIME_FIELDS: [&str; 3] = ["hour", "minute", "second"];

/// How a temporal value breaks its pattern: the fields it carries that the
/// pattern marks `XX`, and the fields the pattern requires that it lacks.
#[derive(Debug, Default, PartialEq, Eq)]
struct PatternBreach {
    not_allowed: Vec<&'static str>,
    missing: Vec<&'static str>,
}

impl PatternBreach {
    /// Explains the breach in plain words.
    fn explain(&self, pattern: &str) -> String {
        let mut parts = Vec::new();
        let (pat_date, pat_time) = split_date_time(pattern);
        let time_disallowed = !pat_time.is_empty() && pat_time.split(':').all(|seg| seg == "XX");
        if time_disallowed && self.not_allowed.iter().any(|f| TIME_FIELDS.contains(f)) {
            if pat_date.is_empty() {
                parts.push("the template allows no time here".to_owned());
            } else {
                parts
                    .push("the template allows a date only; a time is not allowed here".to_owned());
            }
            let date_extra: Vec<&str> = self
                .not_allowed
                .iter()
                .copied()
                .filter(|f| DATE_FIELDS.contains(f))
                .collect();
            if !date_extra.is_empty() {
                parts.push(format!("{} not allowed here", fields_phrase(&date_extra)));
            }
        } else if !self.not_allowed.is_empty() {
            parts.push(format!(
                "the value carries {}, which the template does not allow here",
                fields_phrase(&self.not_allowed)
            ));
        }
        if !self.missing.is_empty() {
            parts.push(format!(
                "the template requires {}, which the value lacks",
                fields_phrase(&self.missing)
            ));
        }
        parts.join("; ")
    }
}

/// Joins field names as an English list: `hour`, `hour and minute`, `hour,
/// minute and second`.
fn fields_phrase(fields: &[&str]) -> String {
    match fields {
        [] => String::new(),
        [one] => format!("the {one}"),
        [init @ .., last] => format!("the {} and {last}", init.join(", ")),
    }
}

/// How a temporal value breaks an AOM 1.4 pattern (`??` optional, `XX` not
/// allowed), or `None`. Only field presence is judged.
fn temporal_pattern_breach(pattern: &str, value: &str) -> Option<PatternBreach> {
    let (pat_date, pat_time) = split_date_time(pattern);
    let (val_date, val_time) = split_date_time(value);
    // An hour-only time ("10") against a pure-time pattern is a time (ADL 1.4
    // `master05-cadl.adoc` §"Date, Time and Date/Time").
    let (val_date, val_time) = if pat_date.is_empty()
        && !pat_time.is_empty()
        && !val_date.is_empty()
        && val_time.is_empty()
    {
        ("", val_date)
    } else {
        (val_date, val_time)
    };
    let mut breach = PatternBreach::default();
    // Date part: year is always required, so its segment is skipped;
    // month/day are judged per the pattern segment. Time part: hours, minutes
    // and seconds are all judged.
    judge_segments(pat_date, val_date, '-', 1, &DATE_FIELDS, &mut breach);
    judge_segments(pat_time, val_time, ':', 0, &TIME_FIELDS, &mut breach);
    (breach != PatternBreach::default()).then_some(breach)
}

/// Records which fields of one temporal half break its pattern half (ADL 1.4
/// `master05-cadl.adoc` §"Date, Time and Date/Time"); `skip` drops the year.
fn judge_segments(
    pattern: &str,
    value: &str,
    sep: char,
    skip: usize,
    fields: &[&'static str; 3],
    breach: &mut PatternBreach,
) {
    if pattern.is_empty() {
        return;
    }
    let val_count = if value.is_empty() {
        0
    } else {
        value.splitn(3, sep).count()
    };
    for (i, (seg, field)) in pattern
        .splitn(3, sep)
        .zip(fields.iter().copied())
        .enumerate()
        .skip(skip)
    {
        let present = val_count > i;
        match (seg, present) {
            ("XX", true) => breach.not_allowed.push(field),
            ("??" | "XX", _) | (_, true) => {}
            (_, false) => breach.missing.push(field),
        }
    }
}

/// Split a temporal string/pattern into its date and time parts. A value with
/// no `T` that contains `:` is a pure time (`HH:MM:SS`); otherwise a pure date.
fn split_date_time(s: &str) -> (&str, &str) {
    if let Some((d, t)) = s.split_once('T') {
        (d, trim_timezone(t))
    } else if s.contains(':') {
        ("", trim_timezone(s))
    } else {
        (s, "")
    }
}

/// Strip a timezone designator from a time part (`Z`, `+hh:mm`, `-hh:mm`).
fn trim_timezone(t: &str) -> &str {
    let t = t.strip_suffix('Z').unwrap_or(t);
    match t.rfind(['+', '-']).and_then(|i| t.get(..i)) {
        Some(core) => core,
        None => t,
    }
}

/// Whether a temporal value lies within an ISO-string range, compared
/// lexicographically at the shorter precision; a mixed-precision prefix match
/// is in range.
pub(crate) fn temporal_in_range(value: &str, range: &WebTemplateRange) -> bool {
    let norm = |s: &str| -> String {
        let (d, t) = split_date_time(s);
        if t.is_empty() {
            d.to_owned()
        } else if d.is_empty() {
            t.to_owned()
        } else {
            format!("{d}T{t}")
        }
    };
    let val = norm(value);
    let cmp = |bound: &str| -> std::cmp::Ordering {
        let b = norm(bound);
        // Truncate both to the shorter precision. ISO 8601 date/time strings
        // are ASCII, so the byte prefix is always a character boundary.
        let n = val.len().min(b.len());
        val.get(..n).unwrap_or(&val).cmp(b.get(..n).unwrap_or(&b))
    };
    if let Some(min) = range.min.as_ref().and_then(Value::as_str) {
        let ok = match range.min_op.as_deref() {
            Some(">") => cmp(min) == std::cmp::Ordering::Greater,
            _ => cmp(min) != std::cmp::Ordering::Less,
        };
        if !ok {
            return false;
        }
    }
    if let Some(max) = range.max.as_ref().and_then(Value::as_str) {
        let ok = match range.max_op.as_deref() {
            Some("<") => cmp(max) == std::cmp::Ordering::Less,
            _ => cmp(max) != std::cmp::Ordering::Greater,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// A string datum against an enumerated value list and/or a regex pattern.
pub(super) fn check_string_constraints(
    v: &mut Validator,
    wt: &WebTemplateNode,
    input: &WebTemplateInput,
    value: &str,
) {
    if input.input_type == WebTemplateInputType::Text
        && !input.list.is_empty()
        && input.list_open != Some(true)
        && !input.list.iter().any(|cv| cv.value == value)
    {
        v.push(
            &wt.aql_path,
            format!("value '{value}' is not in the constrained value set"),
            ValidationKind::CodedValue,
        );
    }
    if let Some(pattern) = input
        .validation
        .as_ref()
        .and_then(|val| val.pattern.as_ref())
        && !matches_pattern(pattern, value)
    {
        v.push(
            &wt.aql_path,
            format!("value '{value}' does not match pattern /{pattern}/"),
            ValidationKind::PatternError,
        );
    }
}

/// Whether `value` matches an archetype `C_STRING` regex, anchored at both
/// ends. A pattern that cannot be evaluated matches no value.
pub(super) fn matches_pattern(pattern: &str, value: &str) -> bool {
    pattern_matches(pattern, value) == Some(true)
}

// ── numeric helpers ─────────────────────────────────────────────────────────────

fn input_with_suffix<'a>(wt: &'a WebTemplateNode, suffix: &str) -> Option<&'a WebTemplateInput> {
    wt.inputs
        .iter()
        .find(|i| i.suffix.as_deref() == Some(suffix))
}

fn as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// Whether `value` satisfies a `WebTemplate` numeric range; missing bounds are
/// unbounded.
// NOTE: BASE foundation_types `interval.adoc` `Interval.has` over `minOp`/`maxOp`.
fn in_range(value: f64, range: &WebTemplateRange) -> bool {
    let min = range.min.as_ref().and_then(as_f64);
    let max = range.max.as_ref().and_then(as_f64);
    let above = min.is_none_or(|m| match range.min_op.as_deref() {
        Some(">") => value > m,
        _ => value >= m,
    });
    above
        && max.is_none_or(|m| match range.max_op.as_deref() {
            Some("<") => value < m,
            _ => value <= m,
        })
}
