// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Per-RM-type datum codecs for the Simplified Formats.
//!
//! One module per grouping of the mapping tables in ITS-REST
//! `simplified_formats/master05-rm_mapping.adoc`:
//!
//! - [`data_values`] — the `DV_*` leaf types, `CODE_PHRASE`, `TERM_MAPPING`,
//!   and the `DV_INTERVAL` / `REFERENCE_RANGE` reference-range families.
//! - [`parties`] — `PARTY_SELF` / `PARTY_IDENTIFIED` / `PARTY_RELATED` /
//!   `PARTY_PROXY`, `OBJECT_REF`, `PARTICIPATION`, `DV_IDENTIFIER`.
//! - [`structures`] — `LINK`, `FEEDER_AUDIT`, `FEEDER_AUDIT_DETAILS`,
//!   `ISM_TRANSITION`, `INSTRUCTION_DETAILS`, and the `_`-prefixed optional
//!   RM-attribute families (master04 §"RM Attributes prefix").
//!
//! The codecs read and write the canonical openEHR JSON
//! ([`serde_json::Value`], `_type`-self-tagged) that the rest of the platform
//! stores and serves — this layer never re-models an RM type.
//!
//! ## The four entry points
//!
//! The RM⇄sim walkers call exactly [`data_values::emit_leaf`] / [`build_leaf`] for a single
//! `DATA_VALUE` leaf and [`structures::emit_rm_attrs`] / [`build_rm_attr`] for the
//! `_`-prefixed optional RM-attribute families. The walker owns the tree
//! structure and the template resolution; this layer owns the per-datum shape.
//!
//! A leaf's value-internal families (`_normal_range`, `_accuracy`, `_mapping:i`,
//! … — the master05 per-type tables) belong to the value and go to
//! [`build_leaf`]; the LOCATABLE / ENTRY / EVENT_CONTEXT families (`_uid`,
//! `_link:i`, `_feeder_audit`, `_null_flavour`, …) go to [`build_rm_attr`], one
//! call per `_`-segment. [`build_rm_attr`] can build the value-internal families
//! too, so a walker may route them either way, but never the same `_`-segment to
//! both.
//!
//! [`data_values::emit_leaf`] writes a leaf's datum attrs only (the `|suffix`
//! keys of master05, plus the bare `""` value where the type has one).
//! `list_open` is the `DV_CODED_TEXT` open-value-set marker from the
//! web-template input (`None` = not coded or unknown): a `DV_TEXT` value in an
//! open slot is emitted via `|other` (master04 §"Open Value-Sets and the
//! `|other` Suffix"). [`structures::emit_rm_attrs`] writes the `_`-prefixed
//! families present on an RM value (master04 §"RM Attributes prefix" and the
//! per-class `_`-rows of master05); field presence and `_type` decide which
//! family applies, so one entry point serves every node kind.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

pub(crate) mod data_values;
pub(crate) mod parties;
pub(crate) mod structures;

use openehr_rm::v1_2::typing::base_name;
use serde_json::{Map, Value, json};

use crate::flat::error::FlatError;
use crate::flat::sim::{SimChild, SimNode};
use crate::flat::webtemplate::model::{WebTemplateCodedValue, WebTemplateNode};

/// Datum parts of `node` (+ its value-internal `_`-prefixed structural children:
/// `_normal_range`, `_other_reference_ranges:i`, `_accuracy`, `_language`,
/// `_encoding`, `_mapping:i`, `_thumbnail`, `_charset`) → an RM leaf.
///
/// `wt_node` supplies the input metadata this layer defaults from: unit lists,
/// `DV_ORDINAL` symbols, the local-terminology `code → rubric` lookup, and
/// `listOpen`. `path` is the printed simplified path, used only for diagnostics.
///
/// # Errors
/// - [`FlatError::RawWithoutType`] — a `|raw` payload without a `_type`
///   (master04 §"Raw canonical JSON").
/// - [`FlatError::OtherSuffixConflict`] — `|other` combined with
///   `|code`/`|value`/`|terminology`/`|preferred_term` (master04 §"Open
///   Value-Sets").
/// - [`FlatError::OtherOnClosedValueSet`] — `|other` on a closed value-set
///   (`listOpen: false`).
/// - [`FlatError::UnknownSuffix`] — a `|suffix` not defined for `rm_type`
///   (master05 per-type tables).
/// - [`FlatError::NoSimplifiedForm`] — anything but `|raw` on a `DV_STATE`,
///   a type master05 gives no mapping table.
/// - [`FlatError::InvalidValue`] — the present datum parts do not form a value
///   of the leaf's type.
pub(crate) fn build_leaf(
    node: &SimNode,
    rm_type: &str,
    wt_node: Option<&WebTemplateNode>,
    path: &str,
) -> Result<Value, FlatError> {
    let base = base_name(rm_type);

    // `|raw` bypass (master04 §"Raw canonical JSON"): the value is embedded
    // canonical RM JSON and MUST carry `_type`. RM→FLAT produces it only for
    // a value no master05 table can express (a `DV_STATE`, a non-conforming
    // stored value).
    if let Some(raw) = node.attrs.get("raw") {
        if raw.get("_type").and_then(Value::as_str).is_some() {
            return Ok(raw.clone());
        }
        return Err(FlatError::RawWithoutType {
            path: path.to_owned(),
        });
    }

    // NOTE: simplified_formats master05-rm_mapping has no DV_STATE table, so
    // only `|raw` carries one (#3936).
    if base == "DV_STATE" {
        return Err(FlatError::NoSimplifiedForm {
            rm_type: base.to_owned(),
            path: path.to_owned(),
        });
    }

    reject_unknown_suffixes(node, base, path)?;

    // `|other` (master04 §"Open Value-Sets and the `|other` Suffix"): a free-text
    // value where the constraint allows an open value-set. Mutually exclusive
    // with the coded suffixes; rejected on a closed value-set; serialised as a
    // `DV_TEXT`, not a `DV_CODED_TEXT` with an empty `defining_code`.
    if let Some(other) = node.attrs.get("other") {
        if ["code", "value", "terminology", "preferred_term"]
            .iter()
            .any(|s| node.attrs.contains_key(*s))
        {
            return Err(FlatError::OtherSuffixConflict(path.to_owned()));
        }
        if list_open_of(wt_node) == Some(false) {
            return Err(FlatError::OtherOnClosedValueSet(path.to_owned()));
        }
        return Ok(json!({"_type": "DV_TEXT", "value": other.clone()}));
    }

    let built =
        data_values::build_leaf(node, rm_type, wt_node).ok_or_else(|| FlatError::InvalidValue {
            path: path.to_owned(),
            reason: format!("the datum parts do not form a valid {base}"),
        })?;
    check_proportion_magnitude(node, &built, path)?;
    Ok(built)
}

/// Refuses a bare DV_PROPORTION value that disagrees with its numerator and
/// denominator.
///
/// master05 §DV_PROPORTION maps the bare value to `magnitude`, "calculated on
/// output", and RM `data_types` §DV_PROPORTION computes `magnitude` as
/// `numerator / denominator`, so the value is not stored. One that disagrees
/// would otherwise be dropped with no trace.
// NOTE: no specification says how close an input magnitude must be; our own
// design accepts one that equals the quotient to the decimals the client wrote.
fn check_proportion_magnitude(node: &SimNode, built: &Value, path: &str) -> Result<(), FlatError> {
    let Some(given) = node.attrs.get("") else {
        return Ok(());
    };
    if built.get("_type").and_then(Value::as_str) != Some("DV_PROPORTION") {
        return Ok(());
    }
    let quotient = match (
        built.get("numerator").and_then(Value::as_f64),
        built.get("denominator").and_then(Value::as_f64),
    ) {
        (Some(n), Some(d)) if d != 0.0 => n / d,
        _ => return Ok(()),
    };
    let refusal = |reason: String| FlatError::InvalidValue {
        path: path.to_owned(),
        reason,
    };
    let Some(magnitude) = given.as_f64() else {
        return Err(refusal(format!(
            "the bare DV_PROPORTION value is its magnitude, a number, not {given}"
        )));
    };
    let written = given.to_string();
    let decimals = written.split_once('.').map_or(0, |(_, fraction)| {
        fraction.split(['e', 'E']).next().unwrap_or("").len()
    });
    let tolerance = 0.5 * 10f64.powi(-i32::try_from(decimals).unwrap_or(300).min(300));
    if (magnitude - quotient).abs() <= tolerance.max(f64::EPSILON * quotient.abs()) {
        Ok(())
    } else {
        Err(refusal(format!(
            "the bare DV_PROPORTION value {written} is the magnitude, numerator / denominator \
             = {quotient}, which it does not match"
        )))
    }
}

/// Applies the master05 container-level datum suffixes (`|sample_count`, the
/// DV_INTERVAL flags) to a container node under construction.
///
/// # Errors
/// [`FlatError::ValueWithoutInput`] for a bare value, which no container takes
/// (master05 puts a value on a data value node), naming an ELEMENT's value
/// alternatives; [`FlatError::UnknownSuffix`] for a suffix the per-type tables
/// do not place on this container ([`container_datum_attr`]).
pub(crate) fn apply_container_datum_attrs(
    node: &WebTemplateNode,
    sim: &SimNode,
    path: &str,
    obj: &mut Map<String, Value>,
) -> Result<(), FlatError> {
    if sim.attrs.contains_key("") {
        let alternatives = if node.rm_type == "ELEMENT" {
            node.children.iter().map(|c| c.id.clone()).collect()
        } else {
            Vec::new()
        };
        return Err(FlatError::ValueWithoutInput {
            path: path.to_owned(),
            rm_type: node.rm_type.clone(),
            alternatives,
        });
    }
    let host = base_name(&node.rm_type);
    for (chain, datum) in &sim.attrs {
        if let Some(attr) = container_datum_attr(host, chain) {
            obj.insert(attr.to_owned(), datum.clone());
        }
    }
    if let Some((chain, _)) = sim
        .attrs
        .iter()
        .find(|(c, _)| container_datum_attr(host, c.as_str()).is_none())
    {
        return Err(FlatError::UnknownSuffix {
            rm_type: node.rm_type.clone(),
            suffix: chain.clone(),
            path: path.to_owned(),
        });
    }
    Ok(())
}

/// The RM attribute a datum suffix (`|suffix`) sets when the master05 per-type
/// tables place it on a **container** node rather than on a `DATA_VALUE` leaf.
///
/// `master05 §INTERVAL_EVENT`: `|sample_count` (INTEGER → `sample_count`, the
/// count of samples the interval summarises) is spelled directly on the event
/// node — the section's second example block is
/// `…/any_event:0|sample_count: 5`, not a sub-path. The mirror of
/// [`crate::flat::legacy::flatten`]'s INTERVAL_EVENT emission.
fn container_datum_attr(host: &str, suffix: &str) -> Option<&'static str> {
    match (host, suffix) {
        ("INTERVAL_EVENT", "sample_count") => Some("sample_count"),
        // master05 §DV_INTERVAL: the four BOOLEAN flags sit on the interval
        // itself when the template gives its bounds their own nodes.
        ("DV_INTERVAL", "lower_unbounded") => Some("lower_unbounded"),
        ("DV_INTERVAL", "upper_unbounded") => Some("upper_unbounded"),
        ("DV_INTERVAL", "lower_included") => Some("lower_included"),
        ("DV_INTERVAL", "upper_included") => Some("upper_included"),
        _ => None,
    }
}

/// One `_`-segment child of a sim node → `(rm_attribute_name, canonical JSON
/// value)` for the RM object under construction, or `None` when the occurrences
/// carry nothing usable.
///
/// `seg` is the segment name **with** its leading underscore (`"_link"`,
/// `"_normal_range"`); `occurrences` is the [`SimChild`] occurrence list (one
/// entry for a single-valued family, several for a `:i` family). `host_rm_type`
/// types the reference-range endpoints (`T`); `path` is for diagnostics.
///
/// # Errors
/// [`FlatError::UnknownSuffix`] — `seg` names no RM-attribute family known for
/// this layer. [`FlatError::MissingRequiredSuffix`] — a datum in the family
/// omits a `|suffix` master05 marks `Required: yes` (the three LINK
/// attributes).
pub(crate) fn build_rm_attr(
    seg: &str,
    occurrences: &[SimNode],
    host_rm_type: &str,
    path: &str,
) -> Result<Option<(String, Value)>, FlatError> {
    structures::build_rm_attr(seg, occurrences, base_name(host_rm_type), path)
}

/// The canonical-JSON value `node` carries whole under `|raw` (master04 §Raw
/// canonical JSON), `None` when it carries none.
///
/// The value must be an object whose `_type` is one of `types`, the RM
/// classes the attribute admits, and `|raw` must be the node's only content:
/// a suffix or a sub-path beside it would be dropped.
///
/// # Errors
/// [`FlatError::RawWithoutType`] for a value without `_type`;
/// [`FlatError::InvalidRaw`] for one of another class, or a `|raw` beside
/// other content.
pub(crate) fn raw_value(
    node: &SimNode,
    path: &str,
    types: &[&str],
) -> Option<Result<Value, FlatError>> {
    let raw = node.attrs.get("raw")?;
    let invalid = |reason: String| FlatError::InvalidRaw {
        path: path.to_owned(),
        reason,
    };
    let Some(rm_type) = raw.get("_type").and_then(Value::as_str) else {
        return Some(Err(FlatError::RawWithoutType {
            path: path.to_owned(),
        }));
    };
    if !types.contains(&rm_type) {
        return Some(Err(invalid(format!(
            "the |raw value is a {rm_type}, expected one of {}",
            types.join(", ")
        ))));
    }
    if node.attrs.len() > 1 || !node.children.is_empty() {
        return Some(Err(invalid(
            "|raw carries the whole value, so no other suffix or sub-path may stand beside it"
                .to_owned(),
        )));
    }
    Some(Ok(raw.clone()))
}

/// The concrete `PARTY_PROXY` classes (RM common `generic` package §PARTY_PROXY
/// and its descendants).
pub(crate) const PARTY_PROXY_CLASSES: &[&str] =
    &["PARTY_SELF", "PARTY_IDENTIFIED", "PARTY_RELATED"];

// ── shared helpers (visible to the sibling map submodules) ────────────────────

/// A `CODE_PHRASE` (RM data_types §CODE_PHRASE): `terminology_id.value` +
/// `code_string`.
pub(super) fn code_phrase(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
        "code_string": code,
    })
}

/// A `CODE_PHRASE` from an owned code value + terminology, with an optional
/// `preferred_term` (master05 §CODE_PHRASE).
pub(super) fn code_phrase_obj(
    code: Value,
    terminology: &str,
    preferred_term: Option<Value>,
) -> Value {
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!("CODE_PHRASE"));
    o.insert(
        "terminology_id".to_owned(),
        json!({"_type": "TERMINOLOGY_ID", "value": terminology}),
    );
    o.insert("code_string".to_owned(), code);
    if let Some(pt) = preferred_term {
        o.insert("preferred_term".to_owned(), pt);
    }
    Value::Object(o)
}

/// The occurrences of the single `_`-family child `name` on `node`, if any.
pub(super) fn family<'a>(node: &'a SimNode, name: &str) -> Option<&'a SimChild> {
    node.children.get(name)
}

/// A `DV_CODED_TEXT` from an openEHR-terminology group, resolving a token that
/// master06 accepts as **either a code or a display value** (`ctx/setting`,
/// `ctx/action_ism_transition_current_state` — master06 §§setting,
/// action_ism_transition_current_state: "either value or code is accepted"; the
/// `PARTICIPATION.mode` group). The `defining_code` terminology is `openehr`.
///
/// Resolution order: a matching concept code → its rubric is the value; else a
/// matching concept rubric → its id is the code; else the token stands as both
/// (an unknown code the validation layer will flag, never a fabricated valid
/// code).
pub(crate) fn coded_from_group(group_id: &str, token: &str) -> Value {
    let term = openehr_term::bundle::openehr();
    if term.is_valid_code(group_id, token) {
        let value = term.rubric(group_id, token, "en").unwrap_or(token);
        return coded_text("openehr", token, value);
    }
    if let Some(concept) = term
        .concepts_in_group(group_id)
        .iter()
        .find(|c| c.rubric == token)
    {
        return coded_text("openehr", &concept.id, token);
    }
    coded_text("openehr", token, token)
}

/// A `DV_CODED_TEXT` from its display value + `(terminology, code)`.
pub(super) fn coded_text(terminology: &str, code: &str, value: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT",
        "value": value,
        "defining_code": code_phrase(terminology, code),
    })
}

/// The `listOpen` marker from the first web-template input that carries one
/// (master04 §"Open Value-Sets").
fn list_open_of(wt_node: Option<&WebTemplateNode>) -> Option<bool> {
    wt_node?.inputs.iter().find_map(|i| i.list_open)
}

/// The web-template terminology id declared on the leaf's input, if any
/// (master04 basic_concepts Web-Template `terminology` on an input).
pub(super) fn wt_terminology(wt_node: Option<&WebTemplateNode>) -> Option<&str> {
    wt_node?
        .inputs
        .iter()
        .find_map(|i| i.terminology.as_deref())
}

/// The web-template coded value whose `value` (the code) is `code`, searched
/// across every input list — the source of the `code → rubric`/`ordinal`
/// defaulting (master05 §§DV_CODED_TEXT, DV_ORDINAL: `|value`/`|ordinal` "may be
/// left out if symbol is defined in template").
pub(super) fn wt_coded_value<'a>(
    wt_node: Option<&'a WebTemplateNode>,
    code: &str,
) -> Option<&'a WebTemplateCodedValue> {
    wt_node?
        .inputs
        .iter()
        .flat_map(|i| i.list.iter())
        .find(|cv| cv.value == code)
}

/// Whether `suffix` is a datum suffix defined for the leaf `base` in the
/// master05 per-type tables (the `""` bare datum and the master04 bypass
/// suffixes `raw`/`other` are handled by the caller).
fn is_known_suffix(base: &str, suffix: &str) -> bool {
    // Suffixes shared by the `DV_ORDERED`/`DV_AMOUNT` scalar families
    // (master05 §§DV_QUANTITY, DV_COUNT, DV_PROPORTION, DV_DURATION and the
    // temporal types): magnitude_status / normal_status / accuracy.
    const AMOUNT_EXTRAS: &[&str] = &[
        "magnitude_status",
        "normal_status",
        "accuracy",
        "accuracy_is_percent",
    ];
    let base_ok: &[&str] = match base {
        "DV_TEXT" | "DV_PARAGRAPH" => &["value", "formatting"],
        "DV_CODED_TEXT" => &[
            "code",
            "value",
            "terminology",
            "preferred_term",
            "formatting",
        ],
        "CODE_PHRASE" => &["code", "terminology", "preferred_term"],
        // master05 §DV_QUANTITY table + example block (the example carries
        // `|precision`, an RM 1.2.0 `DV_QUANTITY.precision` field).
        "DV_QUANTITY" => &[
            "magnitude",
            "unit",
            "precision",
            "magnitude_status",
            "normal_status",
            "accuracy",
            "accuracy_is_percent",
        ],
        "DV_COUNT" | "DV_DURATION" => AMOUNT_EXTRAS,
        "DV_PROPORTION" => &[
            "numerator",
            "denominator",
            "type",
            "precision",
            "magnitude_status",
            "normal_status",
            "accuracy",
            "accuracy_is_percent",
        ],
        // `|terminology` is not in the master05 §DV_ORDINAL examples but the
        // symbol is a full CODE_PHRASE — accepting (and emitting) it keeps a
        // non-`local` symbol terminology round-trip-safe.
        "DV_ORDINAL" => &["code", "value", "ordinal", "terminology"],
        "DV_SCALE" => &["code", "value", "scale", "terminology"],
        "DV_DATE" | "DV_DATE_TIME" | "DV_TIME" => &["magnitude_status", "normal_status"],
        "DV_IDENTIFIER" => &["id", "issuer", "assigner", "type"],
        // master05 §§PARTY_SELF, PARTY_IDENTIFIED, PARTY_RELATED — the three
        // PARTY_PROXY subtype tables share `|id`/`|id_scheme`/`|id_namespace`
        // and PARTY_IDENTIFIED/PARTY_RELATED add `|name`. `_type` is the
        // PARTY_SELF discriminator (master05 §FEEDER_AUDIT_DETAILS `/subject`
        // row Note: "add /subject|_type: PARTY_SELF"); the `/_identifier:i`
        // and `/relationship` rows are sub-paths, not suffixes.
        "PARTY_PROXY" | "PARTY_SELF" | "PARTY_IDENTIFIED" | "PARTY_RELATED" => {
            &["name", "id", "id_scheme", "id_namespace", "_type"]
        }
        "DV_PARSABLE" => &["value", "formalism"],
        "DV_MULTIMEDIA" => &[
            "mediatype",
            "size",
            "alternatetext",
            "compression_algorithm",
            "integrity_check",
            "integrity_check_algorithm",
            "data",
        ],
        "DV_INTERVAL" => &[
            "lower_unbounded",
            "upper_unbounded",
            "lower_included",
            "upper_included",
        ],
        // `DV_BOOLEAN`/`DV_URI`/`DV_EHR_URI` are bare-only (no `|suffix`).
        _ => &[],
    };
    base_ok.contains(&suffix)
}

/// Reject any `|suffix` on `node` not defined for `base` (master05 per-type
/// tables). The bare `""` datum and the master04 bypass suffixes `raw`/`other`
/// are always tolerated here (their own rules run in [`build_leaf`]).
fn reject_unknown_suffixes(node: &SimNode, base: &str, path: &str) -> Result<(), FlatError> {
    for key in node.attrs.keys() {
        if key.is_empty() || key == "raw" || key == "other" {
            continue;
        }
        // A leaf datum suffix is single-level; guard defensively against a
        // chained key by checking its leading part.
        let head = key.split('|').next().unwrap_or(key);
        if !is_known_suffix(base, head) {
            return Err(FlatError::UnknownSuffix {
                rm_type: base.to_owned(),
                suffix: key.clone(),
                path: path.to_owned(),
            });
        }
    }
    Ok(())
}
