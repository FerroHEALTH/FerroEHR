// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Per-RM-type datum codecs for the Simplified Formats, one module per grouping
//! of the `simplified_formats/master05-rm_mapping.adoc` tables.
//!
//! The walkers call [`emit::emit_leaf`] / [`build_leaf`] for a `DATA_VALUE` leaf
//! and [`emit::emit_rm_attrs`] / [`build_rm_attr`] for the `_`-prefixed
//! RM-attribute families (master04 §"RM Attributes prefix"). A leaf's
//! value-internal families (`_normal_range`, `_mapping:i`, …) go to
//! [`build_leaf`]; a walker never routes one `_`-segment to both. A `DV_TEXT`
//! in an open coded slot is emitted via `|other` (master04 §"Open Value-Sets and
//! the `|other` Suffix").

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

pub(crate) mod data_values;
pub(crate) mod emit;
pub(crate) mod parties;
pub(crate) mod structures;

use openehr_rm::v1_2::access;
use openehr_rm::v1_2::access::{RmObject, RmValue};
use openehr_rm::v1_2::typing::base_name;
use serde_json::Value;

use crate::flat::error::FlatError;
use crate::flat::instance::Instance;
use crate::flat::sim::{SimChild, SimNode};
use crate::flat::typed::{self, Build, Obj, o};
use crate::flat::webtemplate::model::{WebTemplateCodedValue, WebTemplateNode};

/// Datum parts of `node` and its value-internal `_`-prefixed children → an RM
/// leaf. `wt_node` supplies the input metadata defaults; `path` is for
/// diagnostics.
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
) -> Result<Obj, FlatError> {
    let base = base_name(rm_type);

    // `|raw` (master04 §"Raw canonical JSON") is embedded canonical RM JSON and
    // must carry `_type`.
    if let Some(raw) = &node.raw {
        return typed::raw_object(raw, path);
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

    // `|other` (master04 §"Open Value-Sets and the `|other` Suffix") is a `DV_TEXT`,
    // exclusive with the coded suffixes and refused on a closed value-set.
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
        let mut text = access::construct("DV_TEXT")?;
        text.put_datum("value", other)?;
        return Ok(text);
    }

    let built = data_values::build_leaf(node, rm_type, wt_node)?.ok_or_else(|| {
        FlatError::InvalidValue {
            path: path.to_owned(),
            reason: format!("the datum parts do not form a valid {base}"),
        }
    })?;
    check_proportion_magnitude(node, &*built, path)?;
    Ok(built)
}

/// Refuses a bare DV_PROPORTION value that disagrees with its numerator and
/// denominator (master05 §DV_PROPORTION: "calculated on output"; RM
/// `data_types` §DV_PROPORTION).
// NOTE: no specification says how close an input magnitude must be; our own
// design accepts one that equals the quotient to the decimals the client wrote.
fn check_proportion_magnitude(
    node: &SimNode,
    built: &dyn RmObject,
    path: &str,
) -> Result<(), FlatError> {
    let Some(given) = node.attrs.get("") else {
        return Ok(());
    };
    if built.rm_type() != "DV_PROPORTION" {
        return Ok(());
    }
    let built = typed::view(built);
    let quotient = match (
        built.attr("numerator").and_then(Instance::as_f64),
        built.attr("denominator").and_then(Instance::as_f64),
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
/// [`FlatError::ValueWithoutInput`] for a bare value, which no container takes,
/// naming an ELEMENT's value alternatives; [`FlatError::UnknownSuffix`] for a
/// suffix the per-type tables do not place on this container.
pub(crate) fn apply_container_datum_attrs(
    node: &WebTemplateNode,
    sim: &SimNode,
    path: &str,
    obj: &mut dyn RmObject,
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
    for (chain, datum) in &sim.attrs {
        if let Some(attr) = container_datum_attr(host, chain) {
            obj.put_datum(attr, datum)?;
        }
    }
    Ok(())
}

/// The RM attribute a datum suffix sets on a container node rather than a
/// `DATA_VALUE` leaf (master05 §INTERVAL_EVENT `…/any_event:0|sample_count: 5`).
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

/// One `_`-segment child of a sim node → `(rm_attribute_name, value)`, or
/// `None` when the occurrences carry nothing usable. `seg` keeps its leading
/// underscore.
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
) -> Result<Option<(&'static str, Attr)>, FlatError> {
    structures::build_rm_attr(seg, occurrences, base_name(host_rm_type), path)
}

/// The value one `_`-segment family writes into its RM attribute.
#[derive(Debug)]
pub(crate) enum Attr {
    /// A single object or primitive.
    One(RmValue),
    /// The members of a container attribute, in order.
    Many(Vec<Obj>),
    /// A client datum the attribute's primitive reads.
    Datum(Value),
}

impl Attr {
    /// Write the value into `attr` of `obj`.
    ///
    /// # Errors
    /// [`FlatError::Rm`] when the class refuses the attribute or the value.
    pub(crate) fn write(self, obj: &mut dyn RmObject, attr: &str) -> Result<(), FlatError> {
        match self {
            Self::One(value) => obj.put(attr, value),
            Self::Many(members) => {
                for member in members {
                    obj.push_obj(attr, member)?;
                }
                Ok(())
            }
            Self::Datum(value) => obj.put_datum(attr, &value),
        }
    }

    /// A single object.
    pub(crate) fn object(object: Obj) -> Self {
        Self::One(o(object))
    }

    /// The canonical JSON of the value, for a test to read.
    #[cfg(test)]
    pub(crate) fn canonical(self) -> Value {
        let one = |value: RmValue| match value {
            RmValue::Object(object) => typed::canonical_of(&*object).unwrap_or(Value::Null),
            RmValue::String(s) => Value::from(s),
            RmValue::Integer(i) => Value::from(i),
            RmValue::Real(r) => Value::from(r),
            RmValue::Boolean(b) => Value::from(b),
            RmValue::Character(c) => Value::from(c.to_string()),
        };
        match self {
            Self::One(value) => one(value),
            Self::Many(members) => Value::Array(members.into_iter().map(|m| one(o(m))).collect()),
            Self::Datum(value) => value,
        }
    }
}

/// The canonical-JSON value `node` carries whole under `|raw` (master04 §Raw
/// canonical JSON), `None` when it carries none.
///
/// # Errors
/// [`FlatError::RawWithoutType`] for a value without `_type`;
/// [`FlatError::InvalidRaw`] for a `_type` outside `types`, or a `|raw` beside
/// other content.
pub(crate) fn raw_value(
    node: &SimNode,
    path: &str,
    types: &[&str],
) -> Option<Result<Obj, FlatError>> {
    let raw = node.raw.as_ref()?;
    let invalid = |reason: String| FlatError::InvalidRaw {
        path: path.to_owned(),
        reason,
    };
    let Some(rm_type) = typed::raw_type(raw) else {
        return Some(Err(FlatError::RawWithoutType {
            path: path.to_owned(),
        }));
    };
    if !types.contains(&&*rm_type) {
        return Some(Err(invalid(format!(
            "the |raw value is a {rm_type}, expected one of {}",
            types.join(", ")
        ))));
    }
    if !node.attrs.is_empty() || !node.children.is_empty() {
        return Some(Err(invalid(
            "|raw carries the whole value, so no other suffix or sub-path may stand beside it"
                .to_owned(),
        )));
    }
    Some(typed::raw_object(raw, path))
}

/// The concrete `PARTY_PROXY` classes (RM common `generic` package §PARTY_PROXY
/// and its descendants).
pub(crate) const PARTY_PROXY_CLASSES: &[&str] =
    &["PARTY_SELF", "PARTY_IDENTIFIED", "PARTY_RELATED"];

// ── shared helpers (visible to the sibling map submodules) ────────────────────

/// A `CODE_PHRASE` from an owned code value + terminology, with an optional
/// `preferred_term` (master05 §CODE_PHRASE).
pub(super) fn code_phrase_obj(
    code: &Value,
    terminology: &str,
    preferred_term: Option<&Value>,
) -> Result<Obj, FlatError> {
    let mut cp = typed::make(
        "CODE_PHRASE",
        [("terminology_id", o(typed::terminology_id(terminology)?))],
    )?;
    cp.put_datum("code_string", code)?;
    if let Some(pt) = preferred_term {
        cp.put_datum("preferred_term", pt)?;
    }
    Ok(cp)
}

/// The occurrences of the single `_`-family child `name` on `node`, if any.
pub(super) fn family<'a>(node: &'a SimNode, name: &str) -> Option<&'a SimChild> {
    node.children.get(name)
}

/// A `DV_CODED_TEXT` from an openEHR-terminology group, resolving a token that
/// may be a code or a display value (master06 §§setting,
/// action_ism_transition_current_state: "either value or code is accepted").
///
/// A matching code, else a matching rubric; an unknown token stands as both, for
/// the validation layer to flag.
pub(crate) fn coded_from_group(group_id: &str, token: &str) -> Result<Obj, FlatError> {
    let term = openehr_term::bundle::openehr();
    if term.is_valid_code(group_id, token) {
        let value = term.rubric(group_id, token, "en").unwrap_or(token);
        return typed::dv_coded_text(value, "openehr", token);
    }
    if let Some(concept) = term
        .concepts_in_group(group_id)
        .iter()
        .find(|c| c.rubric == token)
    {
        return typed::dv_coded_text(token, "openehr", &concept.id);
    }
    typed::dv_coded_text(token, "openehr", token)
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

/// The web-template coded value whose code is `code`, across every input list
/// (master05 §§DV_CODED_TEXT, DV_ORDINAL: "may be left out if symbol is defined
/// in template").
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
        // master05 §§PARTY_SELF, PARTY_IDENTIFIED, PARTY_RELATED; `_type` marks
        // PARTY_SELF (master05 §FEEDER_AUDIT_DETAILS `/subject` row).
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
/// tables); the bare datum and `other` are always tolerated here.
fn reject_unknown_suffixes(node: &SimNode, base: &str, path: &str) -> Result<(), FlatError> {
    for key in node.attrs.keys() {
        if key.is_empty() || key == "other" {
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
            n.put_datum((*k).to_owned(), v.clone());
        }
        n
    }

    // master04 §"Raw canonical JSON": a `|raw` value with `_type` bypasses
    // decomposition and is returned verbatim.
    #[test]
    fn raw_bypass_returns_verbatim() {
        let node = leaf(&[(
            "raw",
            json!({"_type": "DV_QUANTITY", "magnitude": 120.0, "units": "mm[Hg]"}),
        )]);
        let v = typed::canonical_of(&*build_leaf(&node, "DV_TEXT", None, "p").unwrap()).unwrap();
        assert_eq!(v["_type"], "DV_QUANTITY");
        assert_eq!(v["magnitude"], 120.0);
    }

    // master04 §"Raw canonical JSON": the payload MUST carry `_type`.
    #[test]
    fn raw_without_type_is_rejected() {
        let node = leaf(&[("raw", json!({"magnitude": 1}))]);
        assert!(matches!(
            build_leaf(&node, "DV_TEXT", None, "p"),
            Err(FlatError::RawWithoutType { .. })
        ));
    }

    /// A `|raw` datum read from FLAT text reads and refuses as its value does
    /// (#6003).
    #[test]
    fn raw_text_reads_as_its_value_does() {
        let text = |t: &str| SimNode {
            raw: Some(crate::flat::sim::RawDatum::Text(
                serde_json::value::RawValue::from_string(t.to_owned()).unwrap(),
            )),
            ..SimNode::default()
        };
        let quantity = r#"{"_type": "DV_QUANTITY", "magnitude": 120.0, "units": "mm[Hg]"}"#;
        let built = build_leaf(&text(quantity), "DV_TEXT", None, "p").unwrap();
        assert_eq!(
            typed::canonical_of(&*built).unwrap(),
            json!({"_type": "DV_QUANTITY", "magnitude": 120.0, "units": "mm[Hg]"})
        );
        for untyped in [r#"{"magnitude": 1}"#, "5", r#"["x"]"#] {
            assert!(matches!(
                build_leaf(&text(untyped), "DV_TEXT", None, "p"),
                Err(FlatError::RawWithoutType { .. })
            ));
        }
        assert!(matches!(
            build_leaf(
                &text(r#"{"_type": "DV_QUANTITY", "magnitude": "x"}"#),
                "DV_TEXT",
                None,
                "p"
            ),
            Err(FlatError::RawDecode { .. })
        ));
        assert!(matches!(
            build_leaf(
                &text(r#"{"_type": "HIER_OBJECT_ID", "value": "x"}"#),
                "DV_TEXT",
                None,
                "p"
            ),
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
