// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Leaf domain-constraint validation — the `C_DV_*` / `C_PRIMITIVE` leaf
//! constraint checks (AOM 1.4 `master04-constraint_model_package.adoc`) applied
//! to a `DATA_VALUE`, approximated from the WebTemplate `inputs`.
//!
//! For each input we validate the corresponding datum of the instance value,
//! keyed by the input `suffix` / `type`: coded-value membership (unless the list
//! is open or the code is from an external terminology), numeric and temporal
//! ranges (honoring `minOp`/`maxOp`), decimal-precision intervals on
//! `DV_QUANTITY`/`DV_PROPORTION` (#3027), string patterns, duration
//! field/range constraints, and timezone validity. RM well-formedness of
//! date/time/duration VALUES stays with the RM-invariant pass.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use openehr_base::v1_3::base_types::definitions::definitions_impl::LOCAL_TERMINOLOGY_ID;
use serde_json::Value;

use super::{ValidationKind, Validator};
use crate::flat::webtemplate::model::{
    WebTemplateCodedValue, WebTemplateInput, WebTemplateInputType, WebTemplateNode,
    WebTemplateRange,
};

pub(super) fn check_inputs(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let rm = wt.rm_type.split('<').next().unwrap_or(&wt.rm_type);
    match rm {
        "DV_CODED_TEXT" | "DV_STATE" => check_coded_text(v, instance, wt),
        "CODE_PHRASE" => check_code_phrase(v, instance, wt),
        "DV_ORDINAL" | "DV_SCALE" => check_ordinal(v, instance, wt),
        "DV_QUANTITY" => check_quantity(v, instance, wt),
        "DV_COUNT" => check_count(v, instance, wt),
        "DV_PROPORTION" => check_proportion(v, instance, wt),
        "DV_TEXT" | "DV_URI" | "DV_EHR_URI" => check_text(v, instance, wt),
        "DV_BOOLEAN" => check_boolean(v, instance, wt),
        "DV_IDENTIFIER" => check_identifier(v, instance, wt),
        "DV_PARSABLE" => check_parsable(v, instance, wt),
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

/// `C_INTEGER.range` / `C_REAL.range` on a leaf's numeric datum the `inputs`
/// builders do not otherwise carry (`DV_MULTIMEDIA.size` — AOM 1.4
/// `master04-constraint_model_package.adoc` §`C_INTEGER`; RM `data_types`
/// §`DV_MULTIMEDIA`): the instance datum must lie within the declared interval.
fn check_numeric_ranges(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    for (attr, range) in &wt.numeric_ranges {
        let Some(value) = instance.get(attr).and_then(as_f64) else {
            continue;
        };
        if !in_range(value, range) {
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
fn check_numeric_lists(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    for (attr, list) in &wt.numeric_lists {
        let Some(value) = instance.get(attr).and_then(as_f64) else {
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

/// `C_CODE_PHRASE` code-list membership on a coded RM attribute the `inputs`
/// mapping does not model (e.g. `DV_MULTIMEDIA.media_type` — AOM 1.4
/// §`C_CODE_PHRASE`). Enforced only when the instance's terminology matches the
/// constraint's (or both are `local`), biasing toward confident violations.
fn check_code_lists(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    for cl in &wt.code_lists {
        let Some(cp) = instance.get(&cl.attr) else {
            continue;
        };
        let code = cp.get("code_string").and_then(Value::as_str);
        let term = cp
            .get("terminology_id")
            .and_then(|t| t.get("value"))
            .and_then(Value::as_str);
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

fn check_coded_text(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let Some(code_input) = input_with_suffix(wt, "code") else {
        return;
    };
    let code = instance
        .get("defining_code")
        .and_then(|c| c.get("code_string"))
        .and_then(Value::as_str);
    let terminology = instance
        .get("defining_code")
        .and_then(|c| c.get("terminology_id"))
        .and_then(|t| t.get("value"))
        .and_then(Value::as_str);
    check_code_membership(v, wt, code_input, code, terminology);
}

fn check_code_phrase(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let Some(code_input) = input_with_suffix(wt, "code").or_else(|| wt.inputs.first()) else {
        return;
    };
    let code = instance.get("code_string").and_then(Value::as_str);
    let terminology = instance
        .get("terminology_id")
        .and_then(|t| t.get("value"))
        .and_then(Value::as_str);
    check_code_membership(v, wt, code_input, code, terminology);
}

/// `DV_ORDINAL` / `DV_SCALE` against a `C_DV_ORDINAL` / `C_DV_SCALE` list.
///
/// The constraint is a list of `ORDINAL{symbol: CODE_PHRASE, value:
/// Integer}` (Real for `DV_SCALE`) entries, and validity requires the instance's
/// **(symbol, value) PAIR** to match one entry — not the symbol alone nor the
/// value alone (AOM 1.4 `AM/docs/UML/classes/org.openehr.am.aom14.ordinal.adoc`
/// §ORDINAL / §`C_ORDINAL`; the pairing is normative per
/// `AOM2/master04.3-constraint_model-second_order.adoc` §Tuple Constraints L19-21
/// — "as pairs not just as allowable alternatives … which would incorrectly
/// allow any mixing of the Integer and code values"; CNF
/// `master17.3-content_tc_data_types-quantity.adoc` CONT-DV_ORDINAL/DV_SCALE-
/// `validate_constraint`: a right symbol with a wrong value, or a right value with
/// a wrong symbol, both reject). The `WebTemplate` `ordinal_input` carries each
/// entry as a coded value whose `value` is the symbol code and whose `ordinal`
/// (or `scale`) is the paired numeric value.
fn check_ordinal(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let Some(input) = wt.inputs.first() else {
        return;
    };
    if input.list.is_empty() || input.list_open == Some(true) {
        return;
    }
    let Some(code) = checkable_symbol_code(instance, input) else {
        return;
    };

    // Entries whose symbol matches the instance code.
    let same_symbol: Vec<&WebTemplateCodedValue> =
        input.list.iter().filter(|cv| cv.value == code).collect();
    if same_symbol.is_empty() {
        v.push(
            &wt.aql_path,
            format!("ordinal symbol '{code}' is not in the constrained value set"),
            ValidationKind::CodedValue,
        );
        return;
    }
    // Generic C_COMPLEX_OBJECT form: AOM 1.4 has no `C_DV_SCALE` constrainer, so
    // a DV_SCALE constrains its coded `symbol` through `symbol.defining_code` as a
    // `C_CODE_PHRASE` `code_list` with no paired numeric (the builder records the
    // codes with `ordinal`/`scale` unset). Symbol membership alone is then the
    // coded constraint — the numeric `value` set is enforced separately via
    // `numeric_lists` — so the (symbol, value) pair check below applies only to
    // the C_DV_ORDINAL form, which does pin the pair (AOM 1.4 §`C_CODE_PHRASE`;
    // RM `data_types` §`DV_SCALE`).
    if same_symbol
        .iter()
        .all(|cv| cv.scale.is_none() && cv.ordinal.is_none())
    {
        return;
    }
    // The symbol is known; require the (symbol, value) pair to match one entry.
    let is_scale = wt.rm_type.starts_with("DV_SCALE");
    let Some(inst_value) = instance.get("value") else {
        return; // value absent — the RM-invariant pass owns structural presence.
    };
    let pair_ok = same_symbol
        .iter()
        .any(|cv| paired_value_matches(cv, is_scale, inst_value));
    if !pair_ok {
        v.push(
            &wt.aql_path,
            format!(
                "ordinal (symbol '{code}', value {inst_value}) does not match a \
                 constrained (symbol, value) pair"
            ),
            ValidationKind::CodedValue,
        );
    }
}

/// The instance's ordinal symbol code, when the constraint is checkable at all.
///
/// DV_ORDINAL/DV_SCALE encode their coded symbol under `symbol/defining_code`;
/// a code whose terminology differs from the constraint's is not checkable here
/// and yields [`None`].
fn checkable_symbol_code<'a>(instance: &'a Value, input: &WebTemplateInput) -> Option<&'a str> {
    let defining_code = instance.get("symbol").and_then(|s| s.get("defining_code"));
    let terminology = defining_code
        .and_then(|c| c.get("terminology_id"))
        .and_then(|t| t.get("value"))
        .and_then(Value::as_str);
    if !terminology_matches(input.terminology.as_deref(), terminology) {
        return None;
    }
    defining_code
        .and_then(|c| c.get("code_string"))
        .and_then(Value::as_str)
}

/// Whether one constrained entry's paired numeric equals the instance value.
fn paired_value_matches(entry: &WebTemplateCodedValue, is_scale: bool, inst_value: &Value) -> bool {
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

/// Report a coded value that is not among the constrained options. Checked for
/// a non-open, non-empty list whose terminology matches the instance code's —
/// the archetype-`local` terminology when the input names none, or the exact
/// external terminology the `C_CODE_PHRASE` names (AOM 1.4 §`C_CODE_PHRASE`: an
/// enumerated external code list constrains membership just like a local one —
/// master17.2 `CONT-DV_CODED_TEXT-validate_ext_term`).
///
/// When the instance code's terminology *differs* from the constraint's, the
/// behaviour depends on how the constraint is scoped: a constraint that
/// **explicitly** binds to the archetype-`local` terminology
/// ([`WebTemplateNode::coded_terminology_local`]) admits ONLY local codes in its
/// closed list, so a code from any other terminology is a violation — the
/// C_CODE_PHRASE `code_list` is "a list of codes FROM the terminology"
/// (`AM/docs/UML/classes/org.openehr.am.aom14.c_coded_text.adoc` §C_CODED_TEXT,
/// the AOM1.4 form of C_CODE_PHRASE), scoped to that one terminology. For a
/// constraint that does NOT name a terminology (or names an external one), a
/// differently-scoped instance code is left to the terminology pass
/// (confident-violations bias).
fn check_code_membership(
    v: &mut Validator,
    wt: &WebTemplateNode,
    input: &WebTemplateInput,
    code: Option<&str>,
    terminology: Option<&str>,
) {
    if input.list.is_empty() || input.list_open == Some(true) {
        return;
    }
    let Some(code) = code else { return };
    if terminology_matches(input.terminology.as_deref(), terminology) {
        if !input.list.iter().any(|cv| cv.value == code) {
            v.push(
                &wt.aql_path,
                format!("coded value '{code}' is not in the constrained value set"),
                ValidationKind::CodedValue,
            );
        }
        return;
    }
    // Terminology mismatch: reject only when the constraint EXPLICITLY declared
    // the archetype-`local` terminology with a closed list (the builder strips
    // the implicit/default `local` from `input.terminology`, so an unset
    // terminology cannot be distinguished there — the explicit-local signal is
    // carried separately on the node).
    if wt.coded_terminology_local {
        v.push(
            &wt.aql_path,
            format!(
                "coded value '{code}' (terminology '{}') is not in the constrained local value set",
                terminology.unwrap_or("")
            ),
            ValidationKind::CodedValue,
        );
    }
}

// ── quantity / count / proportion ──────────────────────────────────────────────

fn check_quantity(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let unit = instance.get("units").and_then(Value::as_str);
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

    // C_QUANTITY.property membership: AOM 1.4 defines `property` only as
    // "Name of physical property for Quantities being constrained"
    // (`AM/docs/UML/classes/org.openehr.am.aom14.c_quantity.adoc` §C_QUANTITY)
    // with no formal valid_value (the constraint semantics live in the
    // non-vendored Archetype Profile), so this check is our own extension on
    // the openEHR property↔unit asset (`PropertyUnitData.xml` via
    // `openehr_term::bundle`): a unit is rejected only on a confident
    // dimensional mismatch; a unit absent from the table is tolerated.
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
    if let Some(mag) = instance.get("magnitude").and_then(as_f64) {
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

    // DV_QUANTITY.precision against the constrained precision interval
    // (`OpenehrProfile.xsd` `C_QUANTITY_ITEM.precision : IntervalOfInteger`;
    // the Web Template surfaces it as `inputs[].validation.precision` — the
    // ITS-REST `simplified_formats` `master04` model). The RM attribute is
    // 0..1, so an instance that omits it violates nothing (#3027).
    if let Some(prec) = instance.get("precision").and_then(as_f64) {
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

fn check_count(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let Some(mag) = instance.get("magnitude").and_then(as_f64) else {
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

fn check_proportion(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    // `type` kind membership (C_INTEGER.list on DV_PROPORTION.type, surfaced as
    // the node's `proportionTypes` — AOM 1.4 §C_INTEGER; master17.3
    // CONT-DV_PROPORTION-validate_* truth tables). An empty set means the
    // template did not constrain the kind.
    if !wt.proportion_types.is_empty()
        && let Some(kind) = instance.get("type").and_then(Value::as_i64)
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
    if let Some(prec) = instance.get("precision").and_then(as_f64)
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
        let Some(value) = instance.get(part).and_then(as_f64) else {
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

fn check_text(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let Some(input) = wt.inputs.first() else {
        return;
    };
    let Some(value) = instance.get("value").and_then(Value::as_str) else {
        return;
    };
    check_string_constraints(v, wt, input, value);
}

fn check_boolean(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let Some(input) = wt.inputs.first() else {
        return;
    };
    let Some(value) = instance.get("value").and_then(Value::as_bool) else {
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

fn check_identifier(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    for part in ["id", "type", "issuer", "assigner"] {
        let Some(input) = input_with_suffix(wt, part) else {
            continue;
        };
        let Some(value) = instance.get(part).and_then(Value::as_str) else {
            continue;
        };
        check_string_constraints(v, wt, input, value);
    }
}

/// `DV_PARSABLE`: `C_STRING` list/pattern on `value` and `formalism`
/// (AOM 1.4 §`C_STRING`; master17.6 `CONT-DV_PARSABLE-validate_value_formalism`).
fn check_parsable(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    for part in ["value", "formalism"] {
        let Some(input) = input_with_suffix(wt, part) else {
            continue;
        };
        let Some(value) = instance.get(part).and_then(Value::as_str) else {
            continue;
        };
        check_string_constraints(v, wt, input, value);
    }
}

// ── temporal (C_DATE / C_TIME / C_DATE_TIME / C_DURATION, AOM 1.4) ────────────

/// `DV_DATE`/`DV_TIME`/`DV_DATE_TIME`: the `C_*` pattern (mandatory/optional/
/// prohibited field parts, AOM 1.4 `master04-constraint_model_package.adoc`
/// §"Constraints on dates and times") and the ISO range, both surfaced in the
/// leaf input's `validation` by the `WebTemplate` builder.
fn check_temporal(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let Some(value) = instance.get("value").and_then(Value::as_str) else {
        return;
    };
    if let Some(validation) = wt.inputs.first().and_then(|i| i.validation.as_ref()) {
        if let Some(pattern) = &validation.pattern
            && !temporal_pattern_ok(pattern, value)
        {
            v.push(
                &wt.aql_path,
                format!("temporal value '{value}' does not satisfy the pattern '{pattern}'"),
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

/// `C_TIME`/`C_DATE_TIME` `timezone_validity` (`VALIDITY_KIND`): the instance's
/// timezone designator must be present (`1001` mandatory), may be present
/// (`1002` optional — no check), or must be absent (`1003` disallowed). Normative
/// and CNF-tested (CNF `master17.4-content_tc_data_types-date_time.adoc`
/// CONT-DV_TIME-validate_constraint; AOM 1.4
/// `AM/docs/UML/classes/org.openehr.am.aom14.c_time.adoc`/`…c_date_time.adoc`).
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
fn check_duration(v: &mut Validator, instance: &Value, wt: &WebTemplateNode) {
    let Some(value) = instance.get("value").and_then(Value::as_str) else {
        return;
    };
    let Some(fields) = duration_fields(value) else {
        return; // Malformed ISO duration — the RM-invariant pass owns that.
    };
    check_duration_fields_allowed(v, value, &fields, wt);
    check_duration_range(v, value, &fields, wt);
}

/// Reports every duration field the `C_DURATION` pattern does not allow.
///
/// The check runs only when the builder filtered the inputs: a full 7-field set
/// means the pattern allowed everything, or there was no pattern.
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

/// Reports a duration outside the node's constrained range.
///
/// Bound inclusivity follows the AOM interval flags carried in the range ops
/// (BASE `foundation_types` Interval — an exclusive `> PT0S` rejects `PT0S`).
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

/// Appends one half of an ISO-8601 duration to `out`.
///
/// `in_time` selects the time-half reading of the ambiguous `M` designator
/// (minutes rather than months). Returns [`None`] on an unparseable half.
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

/// Whether a date/time/datetime instance value satisfies an AOM 1.4 temporal
/// pattern (`yyyy-mm-dd`, `HH:MM:SS`, `yyyy-mm-ddTHH:MM:SS`, with `??` =
/// optional and `XX` = prohibited parts). Only field *presence* is judged —
/// value well-formedness is the RM-invariant pass's job.
fn temporal_pattern_ok(pattern: &str, value: &str) -> bool {
    let (pat_date, pat_time) = split_date_time(pattern);
    let (val_date, val_time) = split_date_time(value);
    // A colon-less bare DV_TIME value (e.g. "10" = hour only) has no `:` and is
    // classified as a date part by `split_date_time`; but a pure-time pattern
    // ("HH:??:??" / "HH:XX:XX") means the value IS a time — reclassify it so the
    // hour-only value is judged against the time segments, not demanded as a date.
    // ADL 1.4 `master05-cadl.adoc` §"Date, Time and Date/Time" Patterns (L847-910):
    // `?` = optional field, `X` = disallowed field, so a value carrying only the
    // hour satisfies both "HH:??:??" and "HH:XX:XX".
    let (val_date, val_time) = if pat_date.is_empty()
        && !pat_time.is_empty()
        && !val_date.is_empty()
        && val_time.is_empty()
    {
        ("", val_date)
    } else {
        (val_date, val_time)
    };
    // Date part: year is always required, so its segment is skipped;
    // month/day are judged per the pattern segment. Time part: hours, minutes
    // and seconds are all judged.
    segments_match(pat_date, val_date, '-', 1) && segments_match(pat_time, val_time, ':', 0)
}

/// Whether the field presence of one temporal half satisfies its pattern half.
///
/// The two are split on `sep` into at most three segments; `skip` drops the
/// leading segments that carry no optionality (the year). A `??` segment
/// accepts either presence, an `XX` segment requires absence, and any other
/// segment requires presence (ADL 1.4 `master05-cadl.adoc` §"Date, Time and
/// Date/Time" Patterns).
fn segments_match(pattern: &str, value: &str, sep: char, skip: usize) -> bool {
    let pat_segs: Vec<&str> = if pattern.is_empty() {
        Vec::new()
    } else {
        pattern.splitn(3, sep).collect()
    };
    let val_count = if value.is_empty() {
        0
    } else {
        value.splitn(3, sep).count()
    };
    for (i, seg) in pat_segs.iter().enumerate().skip(skip) {
        let present = val_count > i;
        let ok = match *seg {
            "??" => true,
            "XX" => !present,
            _ => present,
        };
        if !ok {
            return false;
        }
    }
    true
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

/// Whether a date/time/datetime value lies within an ISO-string range. The
/// comparison is lexicographic over the timezone-stripped value truncated to
/// the shorter precision — ISO-8601 date/time strings order lexicographically
/// at equal precision; a prefix-equal, mixed-precision pair is treated as
/// in-range (confident-violations bias).
fn temporal_in_range(value: &str, range: &WebTemplateRange) -> bool {
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
fn check_string_constraints(
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

/// Whether `value` matches an archetype `C_STRING` regex. ADL delimits regexes
/// with `/`…`/`; the match is full-string (anchored).
///
/// AOM 1.4 `C_STRING.valid_value` (`AM/docs/UML/classes/org.openehr.am.aom14.c_string.adoc`
/// §`valid_value`; `master04-constraint_model_package.adoc` §`Valid_value` L60-62) is
/// affirmative — a value is valid **iff** it matches the pattern, so a non-match
/// must be reported; a pattern that fails to *compile* must not be treated as a
/// silent pass (that would accept a value against a constraint never evaluated).
/// The ADL regex dialect is a proper subset of Perl (`ADL1.4/master05-cadl.adoc`
/// §Regular Expression L687) — the Rust `regex` crate covers it; the
/// `fancy-regex` PCRE engine is the fallback for real-world patterns using
/// features `regex` rejects (e.g. backreferences).
///
/// NOTE: fail-closed is spec-mandated for a *non-match*, but a
/// pattern **neither** engine can compile cannot be evaluated at all. Rather
/// than reject a value against an uninterpretable constraint (which would
/// over-reject valid data on an engine limitation, not a spec violation), such a
/// pattern is skipped — the residual gap is limited to patterns outside both the
/// Rust-regex and PCRE dialects, and is caught by the corpus round-trip gate.
pub(super) fn matches_pattern(pattern: &str, value: &str) -> bool {
    let body = pattern
        .strip_prefix('/')
        .and_then(|p| p.strip_suffix('/'))
        .unwrap_or(pattern);
    let anchored = format!("^(?:{body})$");
    if let Ok(re) = regex::Regex::new(&anchored) {
        return re.is_match(value);
    }
    // The Rust `regex` crate rejected the pattern (e.g. a backreference); try the
    // PCRE-capable `fancy-regex` before giving up.
    match fancy_regex::Regex::new(&anchored) {
        Ok(re) => re.is_match(value).unwrap_or(true),
        Err(_) => true,
    }
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

/// Whether `value` satisfies a `WebTemplate` numeric range (honoring the
/// inclusive/exclusive `minOp`/`maxOp`; missing bounds are unbounded).
// NOTE: realizes BASE `Interval.has(v)` semantics over `WebTemplateRange`'s
// `minOp`/`maxOp` boundary openness (foundation_types interval.adoc); the
// reference semantics + tests live in `openehr-base` `interval_impl.rs`.
fn in_range(value: f64, range: &WebTemplateRange) -> bool {
    if let Some(min) = range.min.as_ref().and_then(as_f64) {
        let ok = match range.min_op.as_deref() {
            Some(">") => value > min,
            _ => value >= min,
        };
        if !ok {
            return false;
        }
    }
    if let Some(max) = range.max.as_ref().and_then(as_f64) {
        let ok = match range.max_op.as_deref() {
            Some("<") => value < max,
            _ => value <= max,
        };
        if !ok {
            return false;
        }
    }
    true
}
