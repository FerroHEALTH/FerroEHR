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
//! The RM⇄sim walkers call exactly [`emit_leaf`] / [`build_leaf`] for a single
//! `DATA_VALUE` leaf and [`emit_rm_attrs`] / [`build_rm_attr`] for the
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

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

pub(crate) mod data_values;
pub(crate) mod parties;
pub(crate) mod structures;

use serde_json::{Value, json};

use crate::flat::error::FlatError;
use crate::flat::sim::{SimChild, SimNode};
use crate::flat::webtemplate::model::{WebTemplateCodedValue, WebTemplateNode};

/// RM leaf value (canonical JSON) → datum parts on `out` (attrs only; the
/// `|suffix` keys of master05, plus the bare `""` value where the type has one).
///
/// `list_open` is the `DV_CODED_TEXT` open-value-set marker from the web-template
/// input (`None` = not coded / unknown): a `DV_TEXT` value in an *open*
/// `DV_CODED_TEXT` slot is emitted via `|other` (master04 §"Open Value-Sets and
/// the `|other` Suffix"). The `_`-prefixed sub-attribute families are emitted by
/// [`emit_rm_attrs`], not here.
pub(crate) fn emit_leaf(rm: &Value, rm_type: &str, list_open: Option<bool>, out: &mut SimNode) {
    data_values::emit_leaf(rm, rm_type, list_open, out);
}

/// Datum parts of `node` (+ its value-internal `_`-prefixed structural children:
/// `_normal_range`, `_other_reference_ranges:i`, `_accuracy`, `_language`,
/// `_encoding`, `_mapping:i`, `_thumbnail`, `_charset`) → an RM leaf.
///
/// `wt_node` supplies the input metadata this layer defaults from: unit lists,
/// `DV_ORDINAL` symbols, the local-terminology `code → rubric` lookup, and
/// `listOpen`. `path` is the printed simplified path, used only for diagnostics.
///
/// # Errors
/// - [`FlatError::InvalidRaw`] — a `|raw` payload without a `_type`
///   (master04 §"Raw canonical JSON").
/// - [`FlatError::OtherSuffixConflict`] — `|other` combined with
///   `|code`/`|value`/`|terminology`/`|preferred_term` (master04 §"Open
///   Value-Sets").
/// - [`FlatError::OtherOnClosedValueSet`] — `|other` on a closed value-set
///   (`listOpen: false`).
/// - [`FlatError::UnknownSuffix`] — a `|suffix` not defined for `rm_type`
///   (master05 per-type tables).
/// - [`FlatError::InvalidValue`] — the present datum parts do not form a value
///   of the leaf's type.
pub(crate) fn build_leaf(
    node: &SimNode,
    rm_type: &str,
    wt_node: Option<&WebTemplateNode>,
    path: &str,
) -> Result<Value, FlatError> {
    let base = base_type(rm_type);

    // `|raw` bypass (master04 §"Raw canonical JSON"): the value is embedded
    // canonical RM JSON and MUST carry `_type`. Write-only — RM→FLAT always
    // decomposes, so `emit_leaf` never produces `|raw`.
    if let Some(raw) = node.attrs.get("raw") {
        if raw.get("_type").and_then(Value::as_str).is_some() {
            return Ok(raw.clone());
        }
        return Err(FlatError::InvalidRaw {
            path: path.to_owned(),
            reason: "the |raw payload must carry a _type property".to_owned(),
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

    data_values::build_leaf(node, rm_type, wt_node).ok_or_else(|| FlatError::InvalidValue {
        path: path.to_owned(),
        reason: format!("the datum parts do not form a valid {base}"),
    })
}

/// The `_`-prefixed optional RM-attribute families present on RM value `rm` →
/// `_`-prefixed children/attrs on `out` (master04 §"RM Attributes prefix" and
/// the per-class `_`-rows of master05). Field presence + `_type` gate which
/// family applies, so one entry point serves every node kind (the LOCATABLE
/// family, the ENTRY types, the `DV_ORDERED`/`DV_TEXT` value-internal families,
/// PARTY, EVENT_CONTEXT).
pub(crate) fn emit_rm_attrs(rm: &Value, rm_type: &str, out: &mut SimNode) {
    structures::emit_rm_attrs(rm, rm_type, out);
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
    structures::build_rm_attr(seg, occurrences, base_type(host_rm_type), path)
}

// ── shared helpers (visible to the sibling map submodules) ────────────────────

/// The concrete base of a possibly-generic slot type (`DV_INTERVAL<DV_QUANTITY>`
/// → `DV_INTERVAL`).
pub(super) fn base_type(rm_type: &str) -> &str {
    rm_type.split('<').next().unwrap_or(rm_type)
}

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
    let mut o = serde_json::Map::new();
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
        "DV_CODED_TEXT" | "DV_STATE" => &[
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn leaf(pairs: &[(&str, Value)]) -> SimNode {
        let mut n = SimNode::default();
        for (k, v) in pairs {
            n.attrs.insert((*k).to_owned(), v.clone());
        }
        n
    }

    // master04 §"Raw canonical JSON": a `|raw` value with `_type` bypasses
    // decomposition and is returned verbatim.
    #[test]
    fn raw_bypass_returns_verbatim() {
        let node = leaf(&[(
            "raw",
            json!({"_type": "DV_QUANTITY", "magnitude": 120, "unit": "mm[Hg]"}),
        )]);
        let v = build_leaf(&node, "DV_TEXT", None, "p").unwrap();
        assert_eq!(v["_type"], "DV_QUANTITY");
        assert_eq!(v["magnitude"], 120);
    }

    // master04 §"Raw canonical JSON": the payload MUST carry `_type`.
    #[test]
    fn raw_without_type_is_rejected() {
        let node = leaf(&[("raw", json!({"magnitude": 1}))]);
        assert!(matches!(
            build_leaf(&node, "DV_TEXT", None, "p"),
            Err(FlatError::InvalidRaw { .. })
        ));
    }

    // master04 §"Open Value-Sets": `|other` is mutually exclusive with `|code`.
    #[test]
    fn other_conflicts_with_code() {
        let node = leaf(&[("other", json!("free")), ("code", json!("at0001"))]);
        assert!(matches!(
            build_leaf(&node, "DV_CODED_TEXT", None, "p"),
            Err(FlatError::OtherSuffixConflict(_))
        ));
    }

    // master05 per-type tables: an undefined suffix is rejected.
    #[test]
    fn unknown_suffix_rejected() {
        let node = leaf(&[("magnitude", json!(1)), ("frobnicate", json!(2))]);
        assert!(matches!(
            build_leaf(&node, "DV_QUANTITY", None, "p"),
            Err(FlatError::UnknownSuffix { .. })
        ));
    }
}
