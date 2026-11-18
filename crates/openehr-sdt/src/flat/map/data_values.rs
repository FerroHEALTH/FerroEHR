// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `DATA_VALUE` leaf codecs + the reference-range families.
//!
//! Covers the `DV_*` mapping tables of ITS-REST
//! `simplified_formats/master05-rm_mapping.adoc` (DV_TEXT, DV_CODED_TEXT,
//! CODE_PHRASE, TERM_MAPPING, DV_ORDINAL, DV_SCALE, DV_BOOLEAN, DV_URI,
//! DV_EHR_URI, DV_QUANTITY, DV_PROPORTION, DV_COUNT, DV_DATE, DV_DATE_TIME,
//! DV_TIME, DV_DURATION, DV_PARSABLE, DV_MULTIMEDIA), plus the DV_INTERVAL and
//! REFERENCE_RANGE tables and the `DV_ORDERED` value-internal reference-range
//! families (`_normal_range`, `_other_reference_ranges:i`, `_accuracy`).
//!
//! `DV_IDENTIFIER` is a `DATA_VALUE` too but clusters with the identifier / party
//! shapes, so it lives in [`super::parties`]; this module delegates to it.

#![expect(
    clippy::disallowed_types,
    reason = "a datum of the simplified wire is a JSON scalar (ITS-REST Simplified Formats); \
              the builders read it into typed RM and the emitters write it (#1694, #6003)"
)]

use openehr_rm::v1_2::access::{RmObject, RmValue};
use openehr_rm::v1_2::typing::{RmType, base_name};
use serde_json::{Value, json};

use super::{code_phrase_obj, family, parties, wt_coded_value, wt_terminology};
use crate::flat::error::FlatError;
use crate::flat::sim::SimNode;
use crate::flat::typed::{self, Build, Obj, o, s};
use crate::flat::webtemplate::model::WebTemplateNode;

/// The `DV_ORDERED` concrete leaf types that carry `_normal_range` /
/// `_other_reference_ranges` (master05 per-type tables).
pub(super) const DV_ORDERED: &[&str] = &[
    "DV_QUANTITY",
    "DV_COUNT",
    "DV_PROPORTION",
    "DV_DURATION",
    "DV_DATE",
    "DV_DATE_TIME",
    "DV_TIME",
    "DV_ORDINAL",
    "DV_SCALE",
];

// ── sim → RM ────────────────────────────────────────────────────────────────

/// Build the RM leaf for `node` (see [`super::build_leaf`]). `Ok(None)` when
/// the present datum parts do not form a value of `rm_type`.
///
/// # Errors
/// [`FlatError::Rm`] for a datum the RM attribute refuses.
pub(super) fn build_leaf(
    node: &SimNode,
    rm_type: &str,
    wt_node: Option<&WebTemplateNode>,
) -> Result<Option<Obj>, FlatError> {
    let base = base_name(rm_type);
    let built = match build_core(base, rm_type, node, wt_node)? {
        Some(v) => Some(v),
        None => match infer_type(node).filter(|t| *t != base) {
            Some(t) => build_core(t, t, node, wt_node)?,
            None => None,
        },
    };
    let Some(mut value) = built else {
        return Ok(None);
    };
    attach_value_internal(&mut *value, base, node)?;
    Ok(Some(value))
}

/// Build the value-internal families onto an already-built core value. Used by
/// [`build_leaf`]; [`super::structures::build_rm_attr`] builds the same families
/// on demand when a walker routes a `_`-child to it instead.
fn attach_value_internal(
    map: &mut dyn RmObject,
    base: &str,
    node: &SimNode,
) -> Result<(), FlatError> {
    if is_text_family(base) {
        attach_text_internal(map, node)?;
    }
    if base == "DV_PARSABLE" || base == "DV_MULTIMEDIA" {
        put_code_phrase(map, node, "_charset", "charset")?;
        put_code_phrase(map, node, "_language", "language")?;
    }
    if base == "DV_MULTIMEDIA"
        && let Some(t) = single(node, "_thumbnail")
        && let Some(dv) = build_leaf(t, "DV_MULTIMEDIA", None)?
    {
        map.put("thumbnail", o(dv))?;
    }
    if DV_ORDERED.contains(&base) {
        attach_ordered_internal(map, base, node)?;
    }
    Ok(())
}

/// The text-family value-internal families: `language`, `encoding` and the
/// `_mapping:i` list.
fn attach_text_internal(map: &mut dyn RmObject, node: &SimNode) -> Result<(), FlatError> {
    put_code_phrase(map, node, "_language", "language")?;
    put_code_phrase(map, node, "_encoding", "encoding")?;
    for mapping in build_indexed(node, "_mapping", build_term_mapping)? {
        map.push_obj("mappings", mapping)?;
    }
    Ok(())
}

/// The DV_ORDERED value-internal families: the normal range, the other
/// reference ranges, and (temporal types only) the accuracy duration.
fn attach_ordered_internal(
    map: &mut dyn RmObject,
    base: &str,
    node: &SimNode,
) -> Result<(), FlatError> {
    if let Some(child) = single(node, "_normal_range") {
        map.put("normal_range", o(build_interval(child, base)?))?;
    }
    for range in build_indexed(node, "_other_reference_ranges", |n| {
        build_reference_range(n, base)
    })? {
        map.push_obj("other_reference_ranges", range)?;
    }
    if matches!(base, "DV_DATE" | "DV_DATE_TIME" | "DV_TIME")
        && let Some(acc) = single(node, "_accuracy").and_then(SimNode::bare)
    {
        map.put("accuracy", o(duration(acc)?))?;
    }
    Ok(())
}

/// A `DV_DURATION` whose value is the client's datum.
///
/// # Errors
/// [`FlatError::Rm`] for a datum that is no string.
pub(super) fn duration(value: &Value) -> Result<Obj, FlatError> {
    let mut d = typed::new("DV_DURATION")?;
    d.put_datum("value", value)?;
    Ok(d)
}

/// The CODE_PHRASE the single `_`-family child `family` of `node` builds,
/// written into `attr`.
fn put_code_phrase(
    map: &mut dyn RmObject,
    node: &SimNode,
    family: &str,
    attr: &str,
) -> Result<(), FlatError> {
    match single(node, family)
        .map(build_code_phrase)
        .transpose()?
        .flatten()
    {
        Some(cp) => map.put(attr, o(cp)),
        None => Ok(()),
    }
}

fn is_text_family(base: &str) -> bool {
    matches!(base, "DV_TEXT" | "DV_PARAGRAPH" | "DV_CODED_TEXT")
}

/// The single (index-0) occurrence of a `_`-family child, if present.
fn single<'a>(node: &'a SimNode, name: &str) -> Option<&'a SimNode> {
    node.children.get(name).and_then(|c| c.occurrences.first())
}

/// Build a `Vec` from every occurrence of the indexed `_`-family child `name`,
/// in occurrence order (skipping interior placeholder holes).
fn build_indexed<F>(node: &SimNode, name: &str, f: F) -> Result<Vec<Obj>, FlatError>
where
    F: Fn(&SimNode) -> Result<Obj, FlatError>,
{
    let Some(child) = family(node, name) else {
        return Ok(Vec::new());
    };
    child
        .occurrences
        .iter()
        .filter(|occ| !occ.is_empty())
        .map(f)
        .collect()
}

/// Infer a leaf's concrete type from the distinctive suffixes present, when the
/// declared slot type built nothing (Better emits no `_type`, so a choice
/// alternative whose concrete type differs from the constraint is recovered).
fn infer_type(node: &SimNode) -> Option<&'static str> {
    let has = |s: &str| node.attrs.contains_key(s);
    if has("magnitude") {
        Some("DV_QUANTITY")
    } else if has("numerator") {
        Some("DV_PROPORTION")
    } else if has("ordinal") {
        Some("DV_ORDINAL")
    } else if has("scale") {
        Some("DV_SCALE")
    } else if has("mediatype") {
        Some("DV_MULTIMEDIA")
    } else if has("id") {
        Some("DV_IDENTIFIER")
    } else if has("formalism") {
        Some("DV_PARSABLE")
    } else if has("code") {
        Some("DV_CODED_TEXT")
    } else {
        None
    }
}

#[expect(
    clippy::match_same_arms,
    reason = "arms are kept explicit per RM type so each mapping stays readable next to its type name"
)]
fn build_core(
    base: &str,
    rm_type: &str,
    node: &SimNode,
    wt: Option<&WebTemplateNode>,
) -> Result<Option<Obj>, FlatError> {
    match base {
        "DV_TEXT" | "DV_PARAGRAPH" => build_text(node, base),
        "DV_CODED_TEXT" => build_coded_text(node, base, wt),
        "CODE_PHRASE" => build_code_phrase(node),
        "DV_QUANTITY" => build_quantity(node),
        "DV_COUNT" => build_count(node),
        "DV_PROPORTION" => build_proportion(node),
        "DV_ORDINAL" => build_ordinal(node, "DV_ORDINAL", "ordinal", wt),
        "DV_SCALE" => build_ordinal(node, "DV_SCALE", "scale", wt),
        "DV_BOOLEAN" => bare_typed(node, "DV_BOOLEAN"),
        "DV_DURATION" => build_amount_temporal(node, base, true),
        "DV_DATE_TIME" | "DV_DATE" | "DV_TIME" => build_amount_temporal(node, base, false),
        "DV_URI" | "DV_EHR_URI" => bare_typed(node, base),
        "DV_IDENTIFIER" => parties::build_identifier(node).map(Some),
        // master05 §PARTY_PROXY → the three subtype tables. `PERSON` is the
        // `PARTY_REF.type` used when the datum carries an `|id` but no
        // explicit reference type: the carriers of a PARTY_PROXY leaf
        // (`ENTRY.subject`, `COMPOSITION.composer`) reference a person, and
        // the master05 tables define no flat row for `external_ref.type`.
        "PARTY_PROXY" | "PARTY_SELF" | "PARTY_IDENTIFIED" | "PARTY_RELATED" => {
            parties::build_party(node, "PERSON").map(Some)
        }
        "DV_MULTIMEDIA" => build_multimedia(node),
        "DV_PARSABLE" => build_parsable(node),
        "DV_INTERVAL" => build_interval(node, &interval_limit(rm_type)).map(Some),
        _ => bare_typed(node, base),
    }
}

/// The class of the `T` of a generic slot type (`DV_INTERVAL<DV_QUANTITY>` →
/// `DV_QUANTITY`), `DV_QUANTITY` for a slot type that names none.
pub(super) fn interval_limit(rm_type: &str) -> String {
    // NOTE: a slot type that is no generic RM type name (RM class tables) has no argument.
    let parsed = rm_type.parse::<RmType>().ok();
    let limit = parsed.as_ref().and_then(|t| t.params().first());
    limit.map_or_else(|| "DV_QUANTITY".to_owned(), |t| t.name().to_owned())
}

fn attr<'a>(node: &'a SimNode, key: &str) -> Option<&'a Value> {
    node.attrs.get(key)
}

fn attr_str<'a>(node: &'a SimNode, key: &str) -> Option<&'a str> {
    node.attrs.get(key).and_then(Value::as_str)
}

/// Write each present suffix of `node` into its attribute of `o`.
fn put_suffixes(
    o: &mut dyn RmObject,
    node: &SimNode,
    pairs: &[(&str, &str)],
) -> Result<(), FlatError> {
    for (suffix, field) in pairs {
        if let Some(v) = attr(node, suffix) {
            o.put_datum(field, v)?;
        }
    }
    Ok(())
}

fn build_text(node: &SimNode, ty: &str) -> Result<Option<Obj>, FlatError> {
    let Some(value) = node.bare().or_else(|| attr(node, "value")) else {
        return Ok(None);
    };
    let mut o = typed::new(ty)?;
    o.put_datum("value", value)?;
    put_suffixes(&mut *o, node, &[("formatting", "formatting")])?;
    Ok(Some(o))
}

/// master05 §DV_CODED_TEXT. `|value`/`|terminology` are only required for
/// external terminologies; a local/openehr code resolves its rubric and the
/// implicit terminology from the web-template input list when the suffixes are
/// omitted.
fn build_coded_text(
    node: &SimNode,
    ty: &str,
    wt: Option<&WebTemplateNode>,
) -> Result<Option<Obj>, FlatError> {
    let Some(code) = attr(node, "code") else {
        return Ok(None);
    };
    let code_str = code.as_str().unwrap_or_default();
    let terminology = attr_str(node, "terminology")
        .or_else(|| wt_terminology(wt))
        .unwrap_or("local");
    let mut o = typed::new(ty)?;
    match attr(node, "value") {
        Some(value) => o.put_datum("value", value)?,
        None => {
            if let Some(label) = wt_coded_value(wt, code_str).and_then(|cv| cv.label.as_deref()) {
                o.put("value", s(label))?;
            }
        }
    }
    o.put(
        "defining_code",
        typed::o(code_phrase_obj(
            code,
            terminology,
            attr(node, "preferred_term"),
        )?),
    )?;
    put_suffixes(&mut *o, node, &[("formatting", "formatting")])?;
    Ok(Some(o))
}

/// master05 §CODE_PHRASE. `pub(super)` — reused for `_language`/`_encoding`/
/// `_charset`/`target` sub-code-phrases.
pub(super) fn build_code_phrase(node: &SimNode) -> Result<Option<Obj>, FlatError> {
    let Some(code) = attr(node, "code") else {
        return Ok(None);
    };
    let terminology = attr_str(node, "terminology").unwrap_or("local");
    code_phrase_obj(code, terminology, attr(node, "preferred_term")).map(Some)
}

/// Apply the shared amount/ordered reverse extras (master05 note): rebuild
/// `normal_status` as a CODE_PHRASE in the implicit `openehr` terminology.
fn apply_amount_extras(
    o: &mut dyn RmObject,
    node: &SimNode,
    with_accuracy: bool,
) -> Result<(), FlatError> {
    put_suffixes(o, node, &[("magnitude_status", "magnitude_status")])?;
    if with_accuracy {
        put_suffixes(
            o,
            node,
            &[
                ("accuracy", "accuracy"),
                ("accuracy_is_percent", "accuracy_is_percent"),
            ],
        )?;
    }
    if let Some(code) = attr_str(node, "normal_status") {
        o.put(
            "normal_status",
            typed::o(typed::code_phrase("openehr", code)?),
        )?;
    }
    Ok(())
}

fn build_quantity(node: &SimNode) -> Result<Option<Obj>, FlatError> {
    let Some(magnitude) = attr(node, "magnitude") else {
        return Ok(None);
    };
    let mut o = typed::new("DV_QUANTITY")?;
    o.put_datum("magnitude", magnitude)?;
    put_suffixes(
        &mut *o,
        node,
        &[("unit", "units"), ("precision", "precision")],
    )?;
    apply_amount_extras(&mut *o, node, true)?;
    Ok(Some(o))
}

fn build_count(node: &SimNode) -> Result<Option<Obj>, FlatError> {
    let Some(mut o) = from_bare(node, "DV_COUNT", "magnitude")? else {
        return Ok(None);
    };
    apply_amount_extras(&mut *o, node, true)?;
    Ok(Some(o))
}

fn build_proportion(node: &SimNode) -> Result<Option<Obj>, FlatError> {
    let Some(numerator) = attr(node, "numerator") else {
        return Ok(None);
    };
    let mut o = typed::new("DV_PROPORTION")?;
    o.put_datum("numerator", numerator)?;
    put_suffixes(
        &mut *o,
        node,
        &[
            ("denominator", "denominator"),
            ("type", "type"),
            ("precision", "precision"),
        ],
    )?;
    // The bare value is the computed magnitude ("calculated on output"); it is
    // derived from numerator/denominator, so it is not stored back.
    apply_amount_extras(&mut *o, node, true)?;
    Ok(Some(o))
}

/// master05 §§DV_ORDINAL, DV_SCALE. `|value`/`|ordinal`(`|scale`) may be omitted
/// when the symbol is defined in the template — resolved from the web-template
/// coded value for `|code`.
fn build_ordinal(
    node: &SimNode,
    ty: &str,
    numeric_suffix: &str,
    wt: Option<&WebTemplateNode>,
) -> Result<Option<Obj>, FlatError> {
    let Some(code) = attr(node, "code") else {
        return Ok(None);
    };
    let code_str = code.as_str().unwrap_or_default();
    let template = wt_coded_value(wt, code_str);
    let Some(numeric) = attr(node, numeric_suffix).cloned().or_else(|| {
        template.and_then(|cv| match ty {
            "DV_SCALE" => cv.scale.map(|s| json!(s)),
            _ => cv.ordinal.map(|o| json!(o)),
        })
    }) else {
        return Ok(None);
    };
    let terminology = attr_str(node, "terminology").unwrap_or("local");
    let mut symbol = typed::new("DV_CODED_TEXT")?;
    match attr(node, "value") {
        Some(value) => symbol.put_datum("value", value)?,
        None => {
            if let Some(label) = template.and_then(|cv| cv.label.as_deref()) {
                symbol.put("value", s(label))?;
            }
        }
    }
    symbol.put(
        "defining_code",
        o(code_phrase_obj(code, terminology, None)?),
    )?;
    let mut o = typed::new(ty)?;
    o.put_datum("value", &numeric)?;
    o.put("symbol", typed::o(symbol))?;
    Ok(Some(o))
}

/// A temporal leaf (`DV_DATE`/`DV_DATE_TIME`/`DV_TIME`) or `DV_DURATION`: bare
/// value + the amount/ordered extras (master05 §§DV_DATE/DATE_TIME/TIME/DURATION).
fn build_amount_temporal(
    node: &SimNode,
    ty: &str,
    with_accuracy: bool,
) -> Result<Option<Obj>, FlatError> {
    let Some(mut o) = from_bare(node, ty, "value")? else {
        return Ok(None);
    };
    apply_amount_extras(&mut *o, node, with_accuracy)?;
    Ok(Some(o))
}

/// master05 §DV_MULTIMEDIA: bare = `uri`; `media_type` + `size` are RM-mandatory
/// (`size` defaults to 0 when the FLAT lacks `|size`).
fn build_multimedia(node: &SimNode) -> Result<Option<Obj>, FlatError> {
    let mut o = typed::new("DV_MULTIMEDIA")?;
    let mut carries_content = false;
    if let Some(uri) = node.bare() {
        let mut dv_uri = typed::new("DV_URI")?;
        dv_uri.put_datum("value", uri)?;
        o.put("uri", typed::o(dv_uri))?;
        carries_content = true;
    }
    if let Some(mt) = attr(node, "mediatype") {
        o.put(
            "media_type",
            typed::o(code_phrase_obj(mt, "IANA_media-types", None)?),
        )?;
        carries_content = true;
    }
    put_suffixes(&mut *o, node, &[("alternatetext", "alternate_text")])?;
    if let Some(ca) = attr_str(node, "compression_algorithm") {
        o.put(
            "compression_algorithm",
            typed::o(typed::code_phrase("openehr_compression_algorithms", ca)?),
        )?;
    }
    put_suffixes(&mut *o, node, &[("integrity_check", "integrity_check")])?;
    if let Some(ica) = attr_str(node, "integrity_check_algorithm") {
        o.put(
            "integrity_check_algorithm",
            typed::o(typed::code_phrase(
                "openehr_integrity_check_algorithms",
                ica,
            )?),
        )?;
    }
    if let Some(d) = attr(node, "data") {
        o.put_datum("data", d)?;
        carries_content = true;
    }
    let size = attr(node, "size")
        .and_then(Value::as_i64)
        .or_else(|| attr_str(node, "size").and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    o.put("size", RmValue::Integer(size))?;
    Ok(carries_content.then_some(o))
}

fn build_parsable(node: &SimNode) -> Result<Option<Obj>, FlatError> {
    let Some(value) = node.bare().or_else(|| attr(node, "value")) else {
        return Ok(None);
    };
    let mut o = typed::new("DV_PARSABLE")?;
    o.put_datum("value", value)?;
    match attr(node, "formalism") {
        Some(formalism) => o.put_datum("formalism", formalism)?,
        None => o.put("formalism", s(""))?,
    }
    Ok(Some(o))
}

fn bare_typed(node: &SimNode, ty: &str) -> Result<Option<Obj>, FlatError> {
    from_bare(node, ty, "value")
}

/// A `ty` whose `attr` is the bare datum of `node`; `None` without one.
fn from_bare(node: &SimNode, ty: &str, attr: &str) -> Result<Option<Obj>, FlatError> {
    let Some(value) = node.bare() else {
        return Ok(None);
    };
    let mut o = typed::new(ty)?;
    o.put_datum(attr, value)?;
    Ok(Some(o))
}

/// master05 §TERM_MAPPING. `pub(super)` — reused by [`super::structures`] for the
/// `_mapping:i` family via [`build_indexed`].
pub(super) fn build_term_mapping(node: &SimNode) -> Result<Obj, FlatError> {
    let mut o = typed::new("TERM_MAPPING")?;
    match attr(node, "match") {
        Some(m) if m.is_string() => o.put_datum("match", m)?,
        _ => o.put("match", RmValue::Character('='))?,
    }
    put_code_phrase(&mut *o, node, "target", "target")?;
    if let Some(p) = single(node, "purpose")
        && let Some(purpose) = build_leaf(p, "DV_CODED_TEXT", None)?
    {
        o.put("purpose", typed::o(purpose))?;
    }
    Ok(o)
}

/// master05 §DV_INTERVAL: `/lower`, `/upper` (of type `t`) + the boundary flags
/// (defaults: unbounded=false, included=true). `pub(super)` — reused by the
/// `_normal_range` family in [`super::structures`].
pub(super) fn build_interval(node: &SimNode, t: &str) -> Result<Obj, FlatError> {
    let mut iv = typed::new("DV_INTERVAL")?;
    for bound in ["lower", "upper"] {
        if let Some(n) = single(node, bound)
            && let Some(dv) = build_leaf(n, t, None)?
        {
            iv.put(bound, o(dv))?;
        }
    }
    let flag = |s: &str, default: bool| attr(node, s).and_then(Value::as_bool).unwrap_or(default);
    for (name, default) in [
        ("lower_unbounded", false),
        ("upper_unbounded", false),
        ("lower_included", true),
        ("upper_included", true),
    ] {
        iv.put(name, RmValue::Boolean(flag(name, default)))?;
    }
    Ok(iv)
}

/// master05 §REFERENCE_RANGE: a `range` (DV_INTERVAL`<T>`) + a `meaning` (DV_TEXT,
/// or DV_CODED_TEXT when coded). `pub(super)` — reused for the
/// `_other_reference_ranges:i` family in [`super::structures`].
pub(super) fn build_reference_range(node: &SimNode, t: &str) -> Result<Obj, FlatError> {
    let meaning = match single(node, "meaning") {
        None => typed::dv_text("")?,
        Some(m) => {
            let mty = if m.attrs.contains_key("code") {
                "DV_CODED_TEXT"
            } else {
                "DV_TEXT"
            };
            match build_leaf(m, mty, None)? {
                Some(built) => built,
                None => typed::dv_text("")?,
            }
        }
    };
    typed::make(
        "REFERENCE_RANGE",
        [
            ("meaning", o(meaning)),
            ("range", o(build_interval(node, t)?)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flat::map::emit::{emit_interval, emit_leaf};
    use serde_json::json;

    fn canon(object: &Obj) -> Value {
        typed::canonical_of(&**object).unwrap()
    }

    fn leaf(pairs: &[(&str, Value)]) -> SimNode {
        let mut n = SimNode::default();
        for (k, v) in pairs {
            n.attrs.insert((*k).to_owned(), v.clone());
        }
        n
    }

    // master05 §DV_QUANTITY: `|magnitude`/`|unit` + the amount extras round-trip.
    #[test]
    fn quantity_roundtrip() {
        let dv = json!({
            "_type": "DV_QUANTITY", "magnitude": 65.9, "units": "unit",
            "precision": 1, "magnitude_status": "~", "accuracy": 50.5,
            "accuracy_is_percent": true,
            "normal_status": {"_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"},
                "code_string": "N"}
        });
        let mut out = SimNode::default();
        emit_leaf(&dv, "DV_QUANTITY", None, &mut out);
        assert_eq!(out.attrs.get("magnitude"), Some(&json!(65.9)));
        assert_eq!(out.attrs.get("unit"), Some(&json!("unit")));
        assert_eq!(out.attrs.get("normal_status"), Some(&json!("N")));

        let node = leaf(&[
            ("magnitude", json!(65.9)),
            ("unit", json!("unit")),
            ("accuracy", json!(50.5)),
            ("normal_status", json!("N")),
        ]);
        let rm = canon(&build_leaf(&node, "DV_QUANTITY", None).unwrap().unwrap());
        assert_eq!(rm["units"], json!("unit"));
        assert_eq!(rm["normal_status"]["code_string"], json!("N"));
        assert_eq!(
            rm["normal_status"]["terminology_id"]["value"],
            json!("openehr")
        );
    }

    // master05 §DV_COUNT: bare integer = magnitude; numbers keep JSON type.
    #[test]
    fn count_bare_integer() {
        let mut out = SimNode::default();
        emit_leaf(
            &json!({"_type": "DV_COUNT", "magnitude": 7}),
            "DV_COUNT",
            None,
            &mut out,
        );
        assert_eq!(out.bare(), Some(&json!(7)));
        let node = leaf(&[("", json!(7))]);
        let rm = canon(&build_leaf(&node, "DV_COUNT", None).unwrap().unwrap());
        assert_eq!(rm["magnitude"], json!(7));
        assert!(rm["magnitude"].is_i64());
    }

    // master05 §DV_PROPORTION: the bare magnitude is calculated on output.
    #[test]
    fn proportion_computed_magnitude() {
        let dv = json!({"_type": "DV_PROPORTION", "numerator": 20.5,
            "denominator": 12.4, "type": 0, "precision": 1});
        let mut out = SimNode::default();
        emit_leaf(&dv, "DV_PROPORTION", None, &mut out);
        assert_eq!(out.attrs.get("numerator"), Some(&json!(20.5)));
        let mag = out.bare().and_then(Value::as_f64).unwrap();
        assert!((mag - 20.5 / 12.4).abs() < 1e-9);
    }

    // master05 §DV_CODED_TEXT: `|code` resolves `|value`/`|terminology` from the
    // template list when omitted.
    #[test]
    fn coded_text_defaults_value_from_template() {
        use crate::flat::webtemplate::model::{WebTemplateInput, WebTemplateInputType};
        let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
        input
            .list
            .push(crate::flat::webtemplate::model::WebTemplateCodedValue::new(
                "at0006",
                Some("Term One".to_owned()),
            ));
        input.terminology = Some("local".to_owned());
        let mut wt = WebTemplateNode::new("DV_CODED_TEXT".to_owned(), String::new());
        wt.inputs.push(input);
        let node = leaf(&[("code", json!("at0006"))]);
        let rm = canon(
            &build_leaf(&node, "DV_CODED_TEXT", Some(&wt))
                .unwrap()
                .unwrap(),
        );
        assert_eq!(rm["value"], json!("Term One"));
        assert_eq!(rm["defining_code"]["code_string"], json!("at0006"));
        assert_eq!(
            rm["defining_code"]["terminology_id"]["value"],
            json!("local")
        );
    }

    // master05 §DV_ORDINAL: `|ordinal`/`|value` default from the template symbol.
    #[test]
    fn ordinal_defaults_from_template() {
        use crate::flat::webtemplate::model::{WebTemplateInput, WebTemplateInputType};
        let mut cv = crate::flat::webtemplate::model::WebTemplateCodedValue::new(
            "at0015",
            Some("value1".to_owned()),
        );
        cv.ordinal = Some(1);
        let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
        input.list.push(cv);
        let mut wt = WebTemplateNode::new("DV_ORDINAL".to_owned(), String::new());
        wt.inputs.push(input);
        let node = leaf(&[("code", json!("at0015"))]);
        let rm = canon(&build_leaf(&node, "DV_ORDINAL", Some(&wt)).unwrap().unwrap());
        assert_eq!(rm["value"], json!(1));
        assert_eq!(rm["symbol"]["value"], json!("value1"));
        assert_eq!(
            rm["symbol"]["defining_code"]["code_string"],
            json!("at0015")
        );
    }

    // master05 §DV_INTERVAL: boundary flags only in output when non-default.
    #[test]
    fn interval_boundary_flags_only_when_non_default() {
        let iv = json!({
            "_type": "DV_INTERVAL",
            "lower": {"_type": "DV_QUANTITY", "magnitude": 72.83, "units": "Unit"},
            "lower_included": false, "upper_unbounded": true,
            "upper_included": true, "lower_unbounded": false,
        });
        let mut out = SimNode::default();
        emit_interval(&iv, "DV_QUANTITY", &mut out);
        assert_eq!(out.attrs.get("lower_included"), Some(&json!(false)));
        assert_eq!(out.attrs.get("upper_unbounded"), Some(&json!(true)));
        // defaults are omitted
        assert!(!out.attrs.contains_key("upper_included"));
        assert!(!out.attrs.contains_key("lower_unbounded"));
    }

    // master05 §DV_MULTIMEDIA: bare = uri.value; the attribute set round-trips.
    #[test]
    fn multimedia_roundtrip() {
        let dv = json!({
            "_type": "DV_MULTIMEDIA",
            "uri": {"_type": "DV_URI", "value": "http://x/s"},
            "media_type": {"_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "IANA_media-types"},
                "code_string": "video/H261"},
            "size": 504
        });
        let mut out = SimNode::default();
        emit_leaf(&dv, "DV_MULTIMEDIA", None, &mut out);
        assert_eq!(out.bare(), Some(&json!("http://x/s")));
        assert_eq!(out.attrs.get("mediatype"), Some(&json!("video/H261")));
        let node = leaf(&[
            ("", json!("http://x/s")),
            ("mediatype", json!("video/H261")),
            ("size", json!(504)),
        ]);
        let rm = canon(&build_leaf(&node, "DV_MULTIMEDIA", None).unwrap().unwrap());
        assert_eq!(rm["uri"]["value"], json!("http://x/s"));
        assert_eq!(rm["media_type"]["code_string"], json!("video/H261"));
        assert_eq!(rm["size"], json!(504));
    }

    // master05 §DV_QUANTITY `/_normal_range` value-internal family (build side).
    #[test]
    fn quantity_normal_range_attach() {
        let mut node = leaf(&[("magnitude", json!(65.9)), ("unit", json!("unit"))]);
        let nr = node.occurrence_mut("_normal_range", None);
        nr.occurrence_mut("lower", None)
            .attrs
            .insert("magnitude".to_owned(), json!(20.5));
        nr.occurrence_mut("lower", None)
            .attrs
            .insert("unit".to_owned(), json!("unit"));
        let rm = canon(&build_leaf(&node, "DV_QUANTITY", None).unwrap().unwrap());
        assert_eq!(rm["normal_range"]["lower"]["magnitude"], json!(20.5));
        assert_eq!(rm["normal_range"]["lower"]["units"], json!("unit"));
    }
}
