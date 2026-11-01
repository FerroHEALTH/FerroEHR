// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RM-type → `inputs` mapping.
//!
//! Each leaf `DATA_VALUE` / PARTY node is given its `inputs` (and, for
//! `DV_PROPORTION`, the node's `proportionTypes`), the `inputs[]` array of
//! `ITS-REST simplified_formats master04 §"Web Template Metadata"`. The per-type
//! suffix set follows master04 §"Attribute Suffixes" and the class sections
//! (`DV_QUANTITY` → `magnitude`/`unit`, `DV_CODED_TEXT` → `code`/`value`/…), each
//! input carrying its `type`, coded `list`, and `validation`.
//!
//! The `inputs` describe the CONSTRAINT, not resolved runtime values, which
//! fixes three boundaries:
//!
//! * `defaultValue` comes only from an explicit archetype assumed value
//!   (`assumed_value` on `C_PRIMITIVE`, `C_CODE_PHRASE`, `C_DV_QUANTITY`,
//!   `C_DV_ORDINAL` and `C_DV_STATE`, AM `UML/classes/org.openehr.am.aom14.*`),
//!   rendered as the string the released `web_template/Input.yaml` types it
//!   as. An assumed value outside the constraint's own list or range is not
//!   offered (AOM 1.4 `Assumed_value_valid`). RM defaults are a
//!   composition-build concern.
//! * Only archetype-`local` value sets become coded `list` entries; bindings to
//!   external terminologies surface as `termBindings`, and validating an
//!   external code is the terminology service's job.
//! * Rubrics come from the archetype ontology ([`Labels`]); a code the archetype
//!   does not define falls back to the code string as its label — no openEHR
//!   spec governs that fallback (our own design/extension).

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use indexmap::IndexMap;
use openehr_base::v1_3::foundation_types::time::iso8601_duration::Iso8601Duration;
use openehr_its::opt14::types::{
    CAttribute, CObject, CPrimitive, Intervalofinteger, Intervalofreal,
};

use super::model::{
    WebTemplateBindingCodedValue, WebTemplateCodedValue, WebTemplateInput, WebTemplateInputType,
    WebTemplateRange, WebTemplateValidation,
};

/// Resolves rubric text for a `(terminology, code)` pair (the archetype
/// ontology, in the builder). Non-`local` terminologies and unknown codes yield
/// `None`, and the label falls back to the code string.
pub(crate) trait Labels {
    fn text(&self, terminology: &str, code: &str) -> Option<String>;
    fn localized(&self, terminology: &str, code: &str) -> IndexMap<String, String>;
    /// External terminology bindings for `code` (the archetype ontology's
    /// `term_bindings`), keyed by terminology — populating each coded value's
    /// `termBindings` (master04 §"Web Template Metadata").
    fn term_bindings(&self, code: &str) -> IndexMap<String, WebTemplateBindingCodedValue>;
}

/// The openEHR terminology *group* an `openehr`-terminology coded RM attribute
/// binds to, when the owning RM type fixes one (the `has_code_for_group_id` RM
/// invariants). Used to resolve a code's display rubric from the **correct**
/// group: openEHR concept codes are *not* globally unique across groups (TERM
/// 3.1.0 carries the SPECPR-51 defect — code `532` is `complete` in
/// `version_lifecycle_state` but `completed` in `instruction_states`; `253` and
/// `523` collide likewise), so a group-agnostic lookup can pick the wrong
/// rubric. Spec: the RM invariant tables under
/// `docs/specs/openehr/RM/docs/UML/classes/` (`ism_transition`, `composition`,
/// `event_context`, `participation`, `event`, `term_mapping`, `audit_details`,
/// `attestation`, `party_related`).
pub(crate) fn openehr_group(owning_rm_type: &str, attr: &str) -> Option<&'static str> {
    match (owning_rm_type, attr) {
        ("ISM_TRANSITION", "current_state") => Some("instruction_states"),
        ("ISM_TRANSITION", "transition") => Some("instruction_transitions"),
        ("COMPOSITION", "category") => Some("composition_category"),
        ("EVENT_CONTEXT", "setting") => Some("setting"),
        ("PARTICIPATION", "function") => Some("participation_function"),
        ("PARTICIPATION", "mode") => Some("participation_mode"),
        ("EVENT" | "POINT_EVENT" | "INTERVAL_EVENT", "math_function") => {
            Some("event_math_function")
        }
        ("TERM_MAPPING", "purpose") => Some("term_mapping_purpose"),
        ("AUDIT_DETAILS", "change_type") => Some("audit_change_type"),
        ("ATTESTATION", "reason") => Some("attestation_reason"),
        ("PARTY_RELATED", "relationship") => Some("subject_relationship"),
        _ => None,
    }
}

/// Build the `inputs` (and `proportion_types`) for a leaf node. `group` is the
/// openEHR terminology group the node's coded value binds to (see
/// [`openehr_group`]), used only to resolve `openehr`-terminology rubrics from
/// the correct group.
pub(crate) fn build_inputs(
    rm_type: &str,
    co: &CObject,
    labels: &dyn Labels,
    group: Option<&str>,
) -> (Vec<WebTemplateInput>, Vec<String>) {
    // Strip any generic argument (`DV_INTERVAL<DV_QUANTITY>` → `DV_INTERVAL`).
    let base = rm_type.split('<').next().unwrap_or(rm_type);
    let mut proportion_types = Vec::new();
    let inputs = match base {
        "DV_TEXT" | "DV_MULTIMEDIA" | "DV_URI" | "DV_EHR_URI" => {
            vec![text_input(primitive_under(co, "value"), None)]
        }
        "DV_CODED_TEXT" | "DV_STATE" | "CODE_PHRASE" => coded_text_inputs(co, labels, group),
        "DV_QUANTITY" => quantity_inputs(co, labels),
        "DV_COUNT" => vec![count_input(co)],
        "DV_PROPORTION" => proportion_inputs(co, &mut proportion_types),
        "DV_ORDINAL" => vec![ordinal_input(co, labels, false)],
        "DV_SCALE" => vec![ordinal_input(co, labels, true)],
        "DV_BOOLEAN" => vec![boolean_input(co)],
        "DV_DATE" => vec![temporal_input(co, WebTemplateInputType::Date)],
        "DV_DATE_TIME" => vec![temporal_input(co, WebTemplateInputType::Datetime)],
        "DV_TIME" => vec![temporal_input(co, WebTemplateInputType::Time)],
        "DV_DURATION" => duration_inputs(co),
        "DV_IDENTIFIER" => ["id", "type", "issuer", "assigner"]
            .into_iter()
            .map(|s| text_input(primitive_under(co, s), Some(s)))
            .collect(),
        "DV_PARSABLE" => ["value", "formalism"]
            .into_iter()
            .map(|s| text_input(primitive_under(co, s), Some(s)))
            .collect(),
        // The three PARTY_PROXY subtype tables share the
        // `|id`/`|id_scheme`/`|id_namespace` rows and the latter two add
        // `|name` — master05 §§PARTY_SELF, PARTY_IDENTIFIED, PARTY_RELATED. A
        // PARTY_RELATED's extra `relationship` is a DV_CODED_TEXT sub-path
        // (master05 §"PARTY_RELATED performer"), never a party suffix, so it
        // adds no input here.
        "PARTY_PROXY" | "PARTY_IDENTIFIED" | "PARTY_RELATED" => {
            ["id", "id_scheme", "id_namespace", "name"]
                .into_iter()
                .map(|s| text_input(primitive_under(co, s), Some(s)))
                .collect()
        }
        _ => Vec::new(),
    };
    (inputs, proportion_types)
}

// ── DV_TEXT / string ─────────────────────────────────────────────────────────

fn text_input(cstring: Option<&CPrimitive>, suffix: Option<&str>) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(WebTemplateInputType::Text, suffix);
    if let Some(CPrimitive::CString(cs)) = cstring {
        if let Some(pattern) = &cs.pattern {
            input.validation = Some(WebTemplateValidation {
                pattern: Some(pattern.clone()),
                ..Default::default()
            });
        }
        for item in &cs.list {
            input
                .list
                .push(WebTemplateCodedValue::new(item, Some(item.clone())));
        }
        input.list_open = Some(cs.list_open == Some(true));
        input.default_value = cs
            .assumed_value
            .as_deref()
            .filter(|v| cs.list.is_empty() || cs.list.iter().any(|item| item == v))
            .map(|v| default_string(&v));
    }
    input
}

/// The `defaultValue` wire form of an assumed value: a string, as the released
/// `web_template/Input.yaml` declares the member.
pub(super) fn default_string<T: ToString + ?Sized>(value: &T) -> serde_json::Value {
    serde_json::Value::String(value.to_string())
}

/// The `defaultValue` of a coded input: the assumed code when the constraint
/// lists it (or lists none).
pub(super) fn assumed_code(
    assumed: Option<&str>,
    code_list: &[String],
) -> Option<serde_json::Value> {
    assumed
        .filter(|code| code_list.is_empty() || code_list.iter().any(|c| c == code))
        .map(|v| default_string(&v))
}

// ── DV_CODED_TEXT ────────────────────────────────────────────────────────────

fn coded_text_inputs(
    co: &CObject,
    labels: &dyn Labels,
    group: Option<&str>,
) -> Vec<WebTemplateInput> {
    // The node's own CObject may be a CODE_PHRASE, else look under `defining_code`.
    let code_phrases: Vec<&CObject> = match co {
        CObject::CCodePhrase(_) | CObject::CCodeReference(_) => vec![co],
        _ => code_phrases_under(co, "defining_code"),
    };

    let mut inputs = Vec::new();
    match code_phrases.first() {
        Some(CObject::CCodeReference(r)) => {
            let uri = reference_set_uri(&r.referenceSetUri);
            inputs.extend(external_terminology_inputs(uri.as_deref()));
            set_code_default(
                &mut inputs,
                assumed_code(
                    r.assumed_value.as_ref().map(|a| a.code_string.as_str()),
                    &[],
                ),
            );
        }
        Some(CObject::CCodePhrase(cp)) => {
            let terminology = cp.terminology_id.as_ref().map(|t| t.value.clone());
            let codes = coded_values(terminology.as_deref(), &cp.code_list, labels, group);
            if codes.is_empty() {
                inputs.extend(external_terminology_inputs(terminology.as_deref()));
            } else {
                let mut input =
                    WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
                input.list = codes;
                if let Some(term) = &terminology
                    && !term.is_empty()
                    && term != "local"
                {
                    input.terminology = Some(term.clone());
                }
                inputs.push(input);
            }
            set_code_default(
                &mut inputs,
                assumed_code(
                    cp.assumed_value.as_ref().map(|a| a.code_string.as_str()),
                    &cp.code_list,
                ),
            );
        }
        // No `defining_code` constraint (or some other object): free-text coded
        // input pair (`code`/`value`, per master04 §"Attribute Suffixes").
        None | Some(_) => {
            inputs.extend(external_terminology_inputs(None));
            if let CObject::CDvState(state) = co {
                let assumed = state
                    .assumed_value
                    .as_ref()
                    .map(|s| s.value.defining_code.code_string.as_str());
                set_code_default(&mut inputs, assumed_code(assumed, &[]));
            }
        }
    }

    // A `value` C_STRING with listOpen opens every input's list.
    if let Some(CPrimitive::CString(cs)) = primitive_under(co, "value")
        && cs.list_open == Some(true)
    {
        for input in &mut inputs {
            input.list_open = Some(true);
        }
    }
    inputs
}

/// Sets `default` on the `code` input of a coded input set.
fn set_code_default(inputs: &mut [WebTemplateInput], default: Option<serde_json::Value>) {
    if let Some(code) = inputs
        .iter_mut()
        .find(|input| input.suffix.as_deref() == Some("code"))
    {
        code.default_value = default;
    }
}

fn external_terminology_inputs(terminology: Option<&str>) -> Vec<WebTemplateInput> {
    let mut code = WebTemplateInput::new(WebTemplateInputType::Text, Some("code"));
    code.terminology = terminology.map(str::to_owned);
    let mut value = WebTemplateInput::new(WebTemplateInputType::Text, Some("value"));
    value.terminology = terminology.map(str::to_owned);
    vec![code, value]
}

/// The TERMINOLOGY_ID a `C_CODE_REFERENCE.referenceSetUri` names.
///
/// The Web Template `terminology` field carries a terminology identifier
/// (`ITS-REST/specifications/schemas/web_template/Input3.yaml`, example
/// `openehr`), so only the identifying part of the URI belongs in it. The
/// bare form is `terminology:SNOMED-CT`
/// (`CNF/tests/platform/robot/_resources/test_data_sets/valid_templates/`);
/// the addressed form carries an authority, a path and a query —
/// `terminology://snomed-ct/hierarchy?rootConceptId=50043002`
/// (`QUERY/docs/AQL/master03-syntax.adoc`) — where the terminology is the
/// authority and the rest selects within it.
fn reference_set_uri(uri: &str) -> Option<String> {
    if uri.is_empty() {
        return None;
    }
    let Some(rest) = uri.strip_prefix("terminology:") else {
        return Some(uri.to_owned());
    };
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let id = rest.split(['/', '?', '#']).next().unwrap_or("");
    (!id.is_empty()).then(|| id.to_owned())
}

fn coded_values(
    terminology: Option<&str>,
    codes: &[String],
    labels: &dyn Labels,
    group: Option<&str>,
) -> Vec<WebTemplateCodedValue> {
    let term = terminology.unwrap_or("local");
    codes
        .iter()
        .map(|code| {
            let mut cv = coded_value(term, code, labels, group);
            // Per-coded-value external term bindings (master04 §"Web Template
            // Metadata": a `list[]` entry's `termBindings`): the coded-text path
            // adds them; ordinals/scales do not.
            cv.term_bindings = labels.term_bindings(code);
            cv
        })
        .collect()
}

fn coded_value(
    terminology: &str,
    code: &str,
    labels: &dyn Labels,
    group: Option<&str>,
) -> WebTemplateCodedValue {
    // Label resolution order: the artefact's own term definitions, then — for
    // `openehr`-terminology codes, which no archetype defines — the TERM 3.1.0
    // rubric (`433` → `event`); the bare code is the last resort (no openEHR
    // spec governs the label fallback). `DV_CODED_TEXT.value` is the displayable
    // text of the defining code, so a code-as-label leaks wrong instance data.
    // Where the owning RM attribute fixes a terminology group, the rubric
    // resolves from THAT group — openEHR concept codes are not globally unique
    // — with the group-agnostic search as the fallback.
    let label = labels
        .text(terminology, code)
        .or_else(|| {
            (terminology == "openehr")
                .then(|| {
                    let bundle = openehr_term::bundle::openehr();
                    group
                        .and_then(|g| bundle.rubric(g, code, "en"))
                        .or_else(|| bundle.concept_rubric(code, "en"))
                        .map(str::to_owned)
                })
                .flatten()
        })
        .unwrap_or_else(|| code.to_owned());
    let mut cv = WebTemplateCodedValue::new(code, Some(label));
    cv.localized_labels = labels.localized(terminology, code);
    cv
}

// ── DV_QUANTITY ──────────────────────────────────────────────────────────────

fn quantity_inputs(co: &CObject, labels: &dyn Labels) -> Vec<WebTemplateInput> {
    let mut magnitude = WebTemplateInput::new(WebTemplateInputType::Decimal, Some("magnitude"));
    if let CObject::CDvQuantity(q) = co {
        let mut units = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("unit"));
        for item in &q.list {
            let mut value = WebTemplateCodedValue::new(&item.units, Some(item.units.clone()));
            let mut validation = WebTemplateValidation::default();
            if let Some(p) = item.precision.as_ref().and_then(int_range) {
                validation.precision = Some(p);
            }
            if let Some(r) = item.magnitude.as_ref().and_then(decimal_range) {
                validation.range = Some(r);
            }
            if !validation.is_empty() {
                value.validation = Some(validation);
            }
            value.localized_labels = labels.localized("local", &item.units);
            units.list.push(value);
        }
        // A single allowed unit promotes its range/precision onto the magnitude.
        if let [only] = units.list.as_slice() {
            magnitude.validation.clone_from(&only.validation);
        }
        if let Some(assumed) = q.assumed_value.as_ref().filter(|a| {
            q.list.is_empty()
                || q.list.iter().any(|item| {
                    item.units == a.units
                        && item
                            .magnitude
                            .as_ref()
                            .is_none_or(|iv| real_admits(iv, a.magnitude))
                })
        }) {
            magnitude.default_value = Some(default_string(&assumed.magnitude));
            units.default_value = Some(default_string(&assumed.units));
        }
        vec![magnitude, units]
    } else {
        let units = WebTemplateInput::new(WebTemplateInputType::Text, Some("unit"));
        vec![magnitude, units]
    }
}

// ── DV_COUNT ─────────────────────────────────────────────────────────────────

fn count_input(co: &CObject) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(WebTemplateInputType::Integer, None);
    if let Some(CPrimitive::CInteger(ci)) = primitive_under(co, "magnitude") {
        if let Some(range) = ci.range.as_ref().and_then(int_range) {
            input.validation = Some(WebTemplateValidation {
                range: Some(range),
                ..Default::default()
            });
        }
        input.default_value = ci
            .assumed_value
            .filter(|v| ci.list.is_empty() || ci.list.contains(v))
            .filter(|v| ci.range.as_ref().is_none_or(|iv| int_admits(iv, *v)))
            .map(|v| default_string(&v));
    }
    input
}

// ── DV_ORDINAL / DV_SCALE ────────────────────────────────────────────────────

fn ordinal_input(co: &CObject, labels: &dyn Labels, scale: bool) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, None);
    let CObject::CDvOrdinal(ord) = co else {
        push_generic_ordinal_codes(co, labels, &mut input);
        return input;
    };
    for entry in &ord.list {
        let dc = &entry.symbol.defining_code;
        let mut cv = coded_value(&dc.terminology_id.value, &dc.code_string, labels, None);
        // The label should be the ordinal symbol rubric; prefer the symbol value.
        if !entry.symbol.value.is_empty() {
            cv.label = Some(entry.symbol.value.clone());
        }
        if scale {
            cv.scale = Some(f64::from(entry.value));
        } else {
            cv.ordinal = Some(entry.value);
        }
        input.list.push(cv);
    }
    let listed: Vec<String> = ord
        .list
        .iter()
        .map(|entry| entry.symbol.defining_code.code_string.clone())
        .collect();
    input.default_value = assumed_code(
        ord.assumed_value
            .as_ref()
            .map(|assumed| assumed.symbol.defining_code.code_string.as_str()),
        &listed,
    );
    input
}

/// The coded values of the generic `C_COMPLEX_OBJECT` ordinal/scale form.
///
/// AOM 1.4 has no `C_DV_SCALE` constrainer (AM
/// `masterAppA-domain_extension.adoc` defines an integer-valued `C_ORDINAL`
/// only), so a DV_SCALE constrains its coded `symbol` through
/// `symbol.defining_code` as a `C_CODE_PHRASE` `code_list` (RM `data_types`
/// §`DV_SCALE`). That code set surfaces as the coded `list` so the walk
/// enforces symbol membership; the numeric `value` set is captured separately.
/// No paired `(symbol, value)` numeric is recorded — the generic form loses
/// the pairing — so the coded values carry no `ordinal`/`scale`.
fn push_generic_ordinal_codes(co: &CObject, labels: &dyn Labels, input: &mut WebTemplateInput) {
    for symbol_child in attr_children(co, "symbol") {
        for dc_child in attr_children(symbol_child, "defining_code") {
            let CObject::CCodePhrase(cp) = dc_child else {
                continue;
            };
            let term = cp.terminology_id.as_ref().map(|t| t.value.clone());
            for code in &cp.code_list {
                let cv = coded_value(term.as_deref().unwrap_or("local"), code, labels, None);
                if let Some(t) = &term
                    && !t.is_empty()
                    && t != "local"
                {
                    input.terminology = Some(t.clone());
                }
                input.list.push(cv);
            }
            if input.default_value.is_none() {
                input.default_value = assumed_code(
                    cp.assumed_value.as_ref().map(|a| a.code_string.as_str()),
                    &cp.code_list,
                );
            }
        }
    }
}

// ── DV_BOOLEAN ───────────────────────────────────────────────────────────────

fn boolean_input(co: &CObject) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(WebTemplateInputType::Boolean, None);
    if let Some(CPrimitive::CBoolean(cb)) = primitive_under(co, "value") {
        if cb.false_valid && !cb.true_valid {
            input.list.push(WebTemplateCodedValue::new(
                "false",
                Some("false".to_owned()),
            ));
        } else if cb.true_valid && !cb.false_valid {
            input
                .list
                .push(WebTemplateCodedValue::new("true", Some("true".to_owned())));
        }
        input.default_value = cb
            .assumed_value
            .filter(|v| if *v { cb.true_valid } else { cb.false_valid })
            .map(|v| default_string(&v));
    }
    input
}

// ── temporal (DATE / DATETIME / TIME) ────────────────────────────────────────

fn temporal_input(co: &CObject, ty: WebTemplateInputType) -> WebTemplateInput {
    let mut input = WebTemplateInput::new(ty, None);
    let (pattern, range, assumed) = match primitive_under(co, "value") {
        Some(CPrimitive::CDate(c)) => (
            c.pattern.clone(),
            temporal_range(c.range.as_ref().map(iv_bounds_date)),
            c.assumed_value.as_deref(),
        ),
        Some(CPrimitive::CDateTime(c)) => (
            c.pattern.clone(),
            temporal_range(c.range.as_ref().map(iv_bounds_datetime)),
            c.assumed_value.as_deref(),
        ),
        Some(CPrimitive::CTime(c)) => (
            c.pattern.clone(),
            temporal_range(c.range.as_ref().map(iv_bounds_time)),
            c.assumed_value.as_deref(),
        ),
        _ => (None, None, None),
    };
    input.default_value = assumed.map(|v| default_string(&v));
    if pattern.is_some() || range.is_some() {
        input.validation = Some(WebTemplateValidation {
            pattern,
            range,
            ..Default::default()
        });
    }
    input
}

// ── DV_DURATION ──────────────────────────────────────────────────────────────

/// The `DV_DURATION` per-field input order (note DAY precedes WEEK). No openEHR
/// spec governs the per-field split or its order — our own design/extension.
const DURATION_FIELDS: [(&str, char); 7] = [
    ("year", 'Y'),
    ("month", 'M'),
    ("day", 'D'),
    ("week", 'W'),
    ("hour", 'H'),
    ("minute", 'm'),
    ("second", 'S'),
];

/// The per-field `DV_DURATION` inputs, each field's `defaultValue` taken from
/// the matching component of the constraint's assumed ISO 8601 duration.
fn duration_inputs(co: &CObject) -> Vec<WebTemplateInput> {
    let (pattern, assumed) = match primitive_under(co, "value") {
        Some(CPrimitive::CDuration(d)) => (
            d.pattern.clone(),
            d.assumed_value
                .clone()
                .map(|value| Iso8601Duration { value }),
        ),
        _ => (None, None),
    };
    let allowed = duration_allowed_fields(pattern.as_deref());
    DURATION_FIELDS
        .iter()
        .filter(|(name, _)| allowed.contains(name))
        .map(|(name, _)| {
            let mut input = WebTemplateInput::new(WebTemplateInputType::Integer, Some(name));
            input.default_value = assumed
                .as_ref()
                .and_then(|d| duration_component(d, name))
                .map(|v| default_string(&v));
            input
        })
        .collect()
}

/// The count of one named field of a duration, or `None` when it does not parse.
fn duration_component(duration: &Iso8601Duration, field: &str) -> Option<u64> {
    match field {
        "year" => duration.years(),
        "month" => duration.months(),
        "week" => duration.weeks(),
        "day" => duration.days(),
        "hour" => duration.hours(),
        "minute" => duration.minutes(),
        "second" => duration.seconds(),
        _ => None,
    }
}

/// The duration fields allowed by an ISO-8601 duration pattern (a `P…` string
/// with `?`/`{n}` occurrence markers); all fields when there is no pattern.
fn duration_allowed_fields(pattern: Option<&str>) -> Vec<&'static str> {
    let Some(pattern) = pattern else {
        return DURATION_FIELDS.iter().map(|(n, _)| *n).collect();
    };
    // Time-part letters follow `T`; `M` before `T` is month, after is minute.
    let mut in_time = false;
    let mut allowed = Vec::new();
    for ch in pattern.chars() {
        match ch {
            'T' => in_time = true,
            'Y' => allowed.push("year"),
            'M' => allowed.push(if in_time { "minute" } else { "month" }),
            'W' => allowed.push("week"),
            'D' => allowed.push("day"),
            'H' => allowed.push("hour"),
            'S' => allowed.push("second"),
            _ => {}
        }
    }
    if allowed.is_empty() {
        DURATION_FIELDS.iter().map(|(n, _)| *n).collect()
    } else {
        allowed
    }
}

// ── DV_PROPORTION ────────────────────────────────────────────────────────────

pub(crate) const PROPORTION_KINDS: [&str; 5] = [
    "ratio",
    "unitary",
    "percent",
    "fraction",
    "integer_fraction",
];

fn proportion_inputs(co: &CObject, proportion_types: &mut Vec<String>) -> Vec<WebTemplateInput> {
    let type_codes: Vec<i32> = match primitive_under(co, "type") {
        Some(CPrimitive::CInteger(ci)) => ci.list.clone(),
        _ => Vec::new(),
    };
    let is_integral = matches!(primitive_under(co, "is_integral"),
        Some(CPrimitive::CBoolean(b)) if b.true_valid && !b.false_valid);

    *proportion_types = if type_codes.is_empty() {
        PROPORTION_KINDS.iter().map(|s| (*s).to_owned()).collect()
    } else {
        type_codes
            .iter()
            .filter_map(|c| PROPORTION_KINDS.get(usize::try_from(*c).unwrap_or(usize::MAX)))
            .map(|s| (*s).to_owned())
            .collect()
    };

    ["numerator", "denominator"]
        .into_iter()
        .map(|suffix| proportion_part(co, suffix, is_integral))
        .collect()
}

fn proportion_part(co: &CObject, suffix: &str, is_integral: bool) -> WebTemplateInput {
    let ty = if is_integral {
        WebTemplateInputType::Integer
    } else {
        WebTemplateInputType::Decimal
    };
    let mut input = WebTemplateInput::new(ty, Some(suffix));
    if let Some(CPrimitive::CReal(cr)) = primitive_under(co, suffix) {
        if let Some(range) = cr.range.as_ref().and_then(decimal_range) {
            input.validation = Some(WebTemplateValidation {
                range: Some(range),
                ..Default::default()
            });
        }
        input.default_value = cr
            .assumed_value
            .filter(|v| cr.range.as_ref().is_none_or(|iv| real_admits(iv, *v)))
            .map(|v| default_string(&v));
    }
    input
}

// ── opt14 CObject navigation ─────────────────────────────────────────────────

pub(crate) fn attributes(co: &CObject) -> &[CAttribute] {
    match co {
        CObject::CComplexObject(c) => &c.attributes,
        CObject::CArchetypeRoot(c) => &c.attributes,
        CObject::TComplexObject(c) => &c.attributes,
        _ => &[],
    }
}

pub(crate) fn attr_children<'a>(co: &'a CObject, name: &str) -> impl Iterator<Item = &'a CObject> {
    attributes(co)
        .iter()
        .filter(move |a| attribute_name(a) == name)
        .flat_map(attribute_children)
}

pub(crate) fn attribute_name(a: &CAttribute) -> &str {
    match a {
        CAttribute::CSingleAttribute(s) => &s.rm_attribute_name,
        CAttribute::CMultipleAttribute(m) => &m.rm_attribute_name,
    }
}

pub(crate) fn attribute_children(a: &CAttribute) -> &[CObject] {
    match a {
        CAttribute::CSingleAttribute(s) => &s.children,
        CAttribute::CMultipleAttribute(m) => &m.children,
    }
}

/// The first `C_PRIMITIVE` constrained under the named attribute.
pub(crate) fn primitive_under<'a>(co: &'a CObject, name: &str) -> Option<&'a CPrimitive> {
    attr_children(co, name).find_map(|child| match child {
        CObject::CPrimitiveObject(p) => p.item.as_deref(),
        _ => None,
    })
}

/// The `C_CODE_PHRASE`/`C_CODE_REFERENCE` children under the named attribute.
fn code_phrases_under<'a>(co: &'a CObject, name: &str) -> Vec<&'a CObject> {
    attr_children(co, name)
        .filter(|c| matches!(c, CObject::CCodePhrase(_) | CObject::CCodeReference(_)))
        .collect()
}

// ── range helpers ────────────────────────────────────────────────────────────

fn json_f64(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v).map_or(serde_json::Value::Null, serde_json::Value::Number)
}

/// Whether a real interval admits `value`; an absent or unbounded side admits
/// everything and an absent `*_included` reads as included.
fn real_admits(iv: &Intervalofreal, value: f64) -> bool {
    let above = iv.lower_unbounded
        || iv.lower.is_none_or(|lower| {
            if iv.lower_included == Some(false) {
                value > lower
            } else {
                value >= lower
            }
        });
    let below = iv.upper_unbounded
        || iv.upper.is_none_or(|upper| {
            if iv.upper_included == Some(false) {
                value < upper
            } else {
                value <= upper
            }
        });
    above && below
}

/// Whether an integer interval admits `value`, under the [`real_admits`] reading.
fn int_admits(iv: &Intervalofinteger, value: i32) -> bool {
    let above = iv.lower_unbounded
        || iv.lower.is_none_or(|lower| {
            if iv.lower_included == Some(false) {
                value > lower
            } else {
                value >= lower
            }
        });
    let below = iv.upper_unbounded
        || iv.upper.is_none_or(|upper| {
            if iv.upper_included == Some(false) {
                value < upper
            } else {
                value <= upper
            }
        });
    above && below
}

/// `WebTemplateValidationIntegerRange`: min via lower, max via upper (minus one
/// when the upper is excluded); ops fixed to `>=`/`<=`.
fn int_range(iv: &Intervalofinteger) -> Option<WebTemplateRange> {
    let min = if iv.lower_unbounded { None } else { iv.lower };
    let max = if iv.upper_unbounded {
        None
    } else {
        iv.upper.map(|u| {
            if iv.upper_included == Some(false) {
                u - 1
            } else {
                u
            }
        })
    };
    if min.is_none() && max.is_none() {
        return None;
    }
    Some(WebTemplateRange {
        min_op: min.map(|_| ">=".to_owned()),
        min: min.map(serde_json::Value::from),
        max_op: max.map(|_| "<=".to_owned()),
        max: max.map(serde_json::Value::from),
    })
}

/// `WebTemplateDecimalRange`: bounds when not unbounded; ops from inclusivity.
fn decimal_range(iv: &Intervalofreal) -> Option<WebTemplateRange> {
    let min = if iv.lower_unbounded { None } else { iv.lower };
    let max = if iv.upper_unbounded { None } else { iv.upper };
    if min.is_none() && max.is_none() {
        return None;
    }
    let min_op = if iv.lower_included == Some(false) {
        ">"
    } else {
        ">="
    };
    let max_op = if iv.upper_included == Some(false) {
        "<"
    } else {
        "<="
    };
    Some(WebTemplateRange {
        min_op: min.map(|_| min_op.to_owned()),
        min: min.map(json_f64),
        max_op: max.map(|_| max_op.to_owned()),
        max: max.map(json_f64),
    })
}

/// Bounds of a temporal interval as `(min, minOp, max, maxOp)` ISO strings.
type TemporalBounds = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn temporal_range(bounds: Option<TemporalBounds>) -> Option<WebTemplateRange> {
    let (min, min_op, max, max_op) = bounds?;
    if min.is_none() && max.is_none() {
        return None;
    }
    Some(WebTemplateRange {
        min_op,
        min: min.map(serde_json::Value::String),
        max_op,
        max: max.map(serde_json::Value::String),
    })
}

macro_rules! iv_bounds {
    ($fn_name:ident, $ty:ty) => {
        fn $fn_name(iv: &$ty) -> TemporalBounds {
            let min = if iv.lower_unbounded {
                None
            } else {
                iv.lower.clone()
            };
            let max = if iv.upper_unbounded {
                None
            } else {
                iv.upper.clone()
            };
            let min_op = min.as_ref().map(|_| {
                if iv.lower_included == Some(false) {
                    ">"
                } else {
                    ">="
                }
                .to_owned()
            });
            let max_op = max.as_ref().map(|_| {
                if iv.upper_included == Some(false) {
                    "<"
                } else {
                    "<="
                }
                .to_owned()
            });
            (min, min_op, max, max_op)
        }
    };
}

iv_bounds!(iv_bounds_date, openehr_its::opt14::types::Intervalofdate);
iv_bounds!(
    iv_bounds_datetime,
    openehr_its::opt14::types::Intervalofdatetime
);
iv_bounds!(iv_bounds_time, openehr_its::opt14::types::Intervaloftime);

#[cfg(test)]
mod tests {
    use super::openehr_group;

    #[test]
    fn openehr_group_maps_coded_rm_slots_to_their_group() {
        // The RM `has_code_for_group_id` slots (RM invariant tables under
        // docs/specs/openehr/RM/docs/UML/classes/): each maps to exactly one
        // openEHR terminology group so rubrics resolve unambiguously despite
        // SPECPR-51 cross-group code collisions.
        assert_eq!(
            openehr_group("ISM_TRANSITION", "current_state"),
            Some("instruction_states")
        );
        assert_eq!(
            openehr_group("ISM_TRANSITION", "transition"),
            Some("instruction_transitions")
        );
        assert_eq!(
            openehr_group("COMPOSITION", "category"),
            Some("composition_category")
        );
        assert_eq!(openehr_group("EVENT_CONTEXT", "setting"), Some("setting"));
        assert_eq!(
            openehr_group("POINT_EVENT", "math_function"),
            Some("event_math_function")
        );
        // Unmapped: a plain attribute, or an attribute on a type that fixes no
        // group, has no group hint (rubric falls back to the global search).
        assert_eq!(openehr_group("ELEMENT", "value"), None);
        assert_eq!(openehr_group("ISM_TRANSITION", "careflow_step"), None);
    }
}

#[cfg(test)]
mod default_value_tests {
    use indexmap::IndexMap;
    use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
    use openehr_its::opt14::types::{
        CAttribute, CBoolean, CCodePhrase, CComplexObject, CDate, CDuration, CDvOrdinal,
        CDvQuantity, CInteger, CObject, CPrimitive, CPrimitiveObject, CQuantityItem,
        CSingleAttribute, CString, Intervalofinteger, Intervalofreal,
    };
    use openehr_rm::v1_2::data_types::quantity::dv_ordinal::DvOrdinal;
    use openehr_rm::v1_2::data_types::quantity::dv_quantity::DvQuantity;
    use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

    use super::{Labels, WebTemplateBindingCodedValue, WebTemplateInput, build_inputs};

    /// Labels that resolve nothing (every label falls back to its code).
    struct NoLabels;

    impl Labels for NoLabels {
        fn text(&self, _terminology: &str, _code: &str) -> Option<String> {
            None
        }
        fn localized(&self, _terminology: &str, _code: &str) -> IndexMap<String, String> {
            IndexMap::new()
        }
        fn term_bindings(&self, _code: &str) -> IndexMap<String, WebTemplateBindingCodedValue> {
            IndexMap::new()
        }
    }

    fn one() -> Intervalofinteger {
        Intervalofinteger {
            lower_included: Some(true),
            upper_included: Some(true),
            lower_unbounded: false,
            upper_unbounded: false,
            lower: Some(1),
            upper: Some(1),
        }
    }

    fn from_zero() -> Intervalofreal {
        Intervalofreal {
            lower_included: Some(true),
            upper_included: None,
            lower_unbounded: false,
            upper_unbounded: true,
            lower: Some(0.0),
            upper: None,
        }
    }

    /// A `C_COMPLEX_OBJECT` of `rm_type` whose `attribute` holds `child`.
    fn complex(rm_type: &str, attribute: &str, child: CObject) -> CObject {
        CObject::CComplexObject(CComplexObject {
            rm_type_name: rm_type.to_owned(),
            occurrences: one(),
            node_id: String::new(),
            attributes: vec![CAttribute::CSingleAttribute(CSingleAttribute {
                rm_attribute_name: attribute.to_owned(),
                existence: one(),
                children: vec![child],
            })],
        })
    }

    fn primitive(rm_type: &str, item: CPrimitive) -> CObject {
        CObject::CPrimitiveObject(CPrimitiveObject {
            rm_type_name: rm_type.to_owned(),
            occurrences: one(),
            node_id: String::new(),
            item: Some(Box::new(item)),
        })
    }

    fn defaults(rm_type: &str, co: &CObject) -> Vec<(Option<String>, Option<String>)> {
        let (inputs, _) = build_inputs(rm_type, co, &NoLabels, None);
        inputs
            .iter()
            .map(|input: &WebTemplateInput| {
                (
                    input.suffix.clone(),
                    input
                        .default_value
                        .as_ref()
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                )
            })
            .collect()
    }

    fn some(suffix: Option<&str>, value: Option<&str>) -> (Option<String>, Option<String>) {
        (suffix.map(str::to_owned), value.map(str::to_owned))
    }

    fn code_phrase(
        code: &str,
    ) -> openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase {
        openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase {
            terminology_id: TerminologyId {
                value: "local".to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        }
    }

    fn coded_text(assumed: &str) -> CObject {
        complex(
            "DV_CODED_TEXT",
            "defining_code",
            CObject::CCodePhrase(CCodePhrase {
                rm_type_name: "CODE_PHRASE".to_owned(),
                occurrences: one(),
                node_id: String::new(),
                assumed_value: Some(code_phrase(assumed)),
                terminology_id: Some(TerminologyId {
                    value: "local".to_owned(),
                }),
                code_list: vec!["at1".to_owned(), "at2".to_owned()],
            }),
        )
    }

    #[test]
    fn coded_text_offers_a_listed_assumed_code_only() {
        let co = coded_text("at2");
        assert_eq!(
            defaults("DV_CODED_TEXT", &co),
            vec![some(Some("code"), Some("at2"))]
        );
        let co = coded_text("at9");
        assert_eq!(
            defaults("DV_CODED_TEXT", &co),
            vec![some(Some("code"), None)]
        );
    }

    fn quantity(assumed_magnitude: f64) -> CObject {
        CObject::CDvQuantity(CDvQuantity {
            rm_type_name: "DV_QUANTITY".to_owned(),
            occurrences: one(),
            node_id: String::new(),
            assumed_value: Some(DvQuantity {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: None,
                magnitude_status: None,
                accuracy: None,
                accuracy_is_percent: None,
                magnitude: assumed_magnitude,
                precision: None,
                units: "mg".to_owned(),
                units_system: None,
                units_display_name: None,
            }),
            property: None,
            list: vec![CQuantityItem {
                magnitude: Some(from_zero()),
                precision: None,
                units: "mg".to_owned(),
            }],
        })
    }

    #[test]
    fn quantity_offers_an_admitted_assumed_value_only() {
        assert_eq!(
            defaults("DV_QUANTITY", &quantity(5.5)),
            vec![
                some(Some("magnitude"), Some("5.5")),
                some(Some("unit"), Some("mg"))
            ]
        );
        assert_eq!(
            defaults("DV_QUANTITY", &quantity(-1.0)),
            vec![some(Some("magnitude"), None), some(Some("unit"), None)]
        );
    }

    #[test]
    fn count_boolean_text_and_date_offer_their_assumed_values() {
        let count = complex(
            "DV_COUNT",
            "magnitude",
            primitive(
                "INTEGER",
                CPrimitive::CInteger(CInteger {
                    list: Vec::new(),
                    range: None,
                    assumed_value: Some(3),
                }),
            ),
        );
        assert_eq!(defaults("DV_COUNT", &count), vec![some(None, Some("3"))]);

        let boolean = complex(
            "DV_BOOLEAN",
            "value",
            primitive(
                "BOOLEAN",
                CPrimitive::CBoolean(CBoolean {
                    true_valid: true,
                    false_valid: true,
                    assumed_value: Some(true),
                }),
            ),
        );
        assert_eq!(
            defaults("DV_BOOLEAN", &boolean),
            vec![some(None, Some("true"))]
        );

        let text = complex(
            "DV_TEXT",
            "value",
            primitive(
                "STRING",
                CPrimitive::CString(CString {
                    pattern: None,
                    list: vec!["a".to_owned(), "b".to_owned()],
                    list_open: None,
                    assumed_value: Some("b".to_owned()),
                }),
            ),
        );
        assert_eq!(defaults("DV_TEXT", &text), vec![some(None, Some("b"))]);

        let date = complex(
            "DV_DATE",
            "value",
            primitive(
                "DATE",
                CPrimitive::CDate(CDate {
                    pattern: None,
                    timezone_validity: None,
                    range: None,
                    assumed_value: Some("2020-01-31".to_owned()),
                }),
            ),
        );
        assert_eq!(
            defaults("DV_DATE", &date),
            vec![some(None, Some("2020-01-31"))]
        );
    }

    #[test]
    fn duration_fields_take_the_assumed_components() {
        let duration = complex(
            "DV_DURATION",
            "value",
            primitive(
                "DURATION",
                CPrimitive::CDuration(CDuration {
                    pattern: Some("PYTH".to_owned()),
                    range: None,
                    assumed_value: Some("P2YT6H".to_owned()),
                }),
            ),
        );
        assert_eq!(
            defaults("DV_DURATION", &duration),
            vec![some(Some("year"), Some("2")), some(Some("hour"), Some("6"))]
        );
    }

    #[test]
    fn ordinal_offers_a_listed_assumed_symbol() {
        let entry = |code: &str, value: i32| DvOrdinal {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            symbol: DvCodedText {
                value: code.to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: None,
                language: None,
                encoding: None,
                defining_code: openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase {
                    terminology_id: TerminologyId {
                        value: "local".to_owned(),
                    },
                    code_string: code.to_owned(),
                    preferred_term: None,
                },
            },
            value,
        };
        let co = CObject::CDvOrdinal(CDvOrdinal {
            rm_type_name: "DV_ORDINAL".to_owned(),
            occurrences: one(),
            node_id: String::new(),
            assumed_value: Some(entry("at2", 2)),
            list: vec![entry("at1", 1), entry("at2", 2)],
        });
        assert_eq!(defaults("DV_ORDINAL", &co), vec![some(None, Some("at2"))]);
    }

    /// An externally bound coded text with an assumed code serializes the
    /// string `defaultValue` the released `web_template/Input.yaml` declares;
    /// the vendored EHRbase node vector
    /// (`corpus/fixtures/service/webtemplate_nodes`, prior art only) carries the
    /// same shape.
    #[test]
    fn external_coded_text_default_matches_the_vendored_node_vector() {
        let co = complex(
            "DV_CODED_TEXT",
            "defining_code",
            CObject::CCodePhrase(CCodePhrase {
                rm_type_name: "CODE_PHRASE".to_owned(),
                occurrences: one(),
                node_id: String::new(),
                assumed_value: Some(
                    openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase {
                        terminology_id: TerminologyId {
                            value: "ICD10".to_owned(),
                        },
                        code_string: "E10.0".to_owned(),
                        preferred_term: None,
                    },
                ),
                terminology_id: Some(TerminologyId {
                    value: "ICD10".to_owned(),
                }),
                code_list: Vec::new(),
            }),
        );
        let (inputs, _) = build_inputs("DV_CODED_TEXT", &co, &NoLabels, None);
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/fixtures/service/webtemplate_nodes/dv_codedtext_unsupported.json"
        );
        let text = std::fs::read_to_string(path).expect("vendored node vector");
        let node: serde_json::Value = serde_json::from_str(&text).expect("node JSON");
        assert_eq!(
            serde_json::to_value(&inputs).expect("serialize inputs"),
            node["inputs"]
        );
    }
}
