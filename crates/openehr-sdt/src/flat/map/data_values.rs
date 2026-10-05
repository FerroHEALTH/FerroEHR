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
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use serde_json::{Map, Value, json};

use super::{base_type, code_phrase_obj, family, parties, wt_coded_value, wt_terminology};
use crate::flat::sim::SimNode;
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

// ── RM → sim ──────────────────────────────────────────────────────────────────

/// Insert `out.attrs[suffix]` if `v` is present and non-null. `suffix == ""` is
/// the bare datum.
fn put(out: &mut SimNode, suffix: &str, v: Option<&Value>) {
    if let Some(v) = v.filter(|v| !v.is_null()) {
        out.attrs.insert(suffix.to_owned(), v.clone());
    }
}

/// The `code_string` / `terminology_id.value` / `preferred_term` of a CODE_PHRASE.
fn code_phrase_parts(cp: &Value) -> (Option<&Value>, Option<&Value>, Option<&Value>) {
    (
        cp.get("code_string"),
        cp.pointer("/terminology_id/value"),
        cp.get("preferred_term").filter(|v| !v.is_null()),
    )
}

/// Emit the shared `DV_QUANTIFIED`/`DV_ORDERED` scalar extras (master05
/// §§DV_QUANTITY, DV_COUNT, DV_PROPORTION, DV_DURATION): `|magnitude_status`,
/// `|normal_status` (= `normal_status.code_string`; the CODE_PHRASE terminology
/// is implicitly `openehr`), `|accuracy`, `|accuracy_is_percent`.
/// `with_accuracy` is false for the temporal family, whose `accuracy` is a
/// `DV_DURATION` carried as `/_accuracy` (master05 §§DV_DATE, DV_DATE_TIME,
/// DV_TIME).
fn emit_quantified_extras(dv: &Value, out: &mut SimNode, with_accuracy: bool) {
    put(out, "magnitude_status", dv.get("magnitude_status"));
    put(
        out,
        "normal_status",
        dv.pointer("/normal_status/code_string"),
    );
    if with_accuracy {
        put(out, "accuracy", dv.get("accuracy"));
        put(out, "accuracy_is_percent", dv.get("accuracy_is_percent"));
    }
}

/// RM leaf value → datum attrs on `out` (see [`super::emit_leaf`]).
#[expect(
    clippy::match_same_arms,
    reason = "the arms are kept explicit per RM type so each mapping stays readable next to its type name"
)]
pub(super) fn emit_leaf(rm: &Value, rm_type: &str, list_open: Option<bool>, out: &mut SimNode) {
    let ty = rm
        .get("_type")
        .and_then(Value::as_str)
        .unwrap_or_else(|| base_type(rm_type));
    match ty {
        // master05 §DV_TEXT (and DV_PARAGRAPH shares the shape). A DV_TEXT held
        // in an *open* DV_CODED_TEXT slot is the `|other` free-text branch
        // (master04 §"Open Value-Sets"); otherwise the bare value, or `|value`
        // when the node also carries metadata that keeps it from being a bare
        // leaf (`|formatting` / `_mapping`).
        "DV_TEXT" | "DV_PARAGRAPH" => {
            let coded_slot = base_type(rm_type) == "DV_CODED_TEXT";
            let open_slot = coded_slot && list_open != Some(false);
            let has_meta = present(rm.get("formatting")) || present(rm.get("mappings"));
            if open_slot {
                put(out, "other", rm.get("value"));
            } else if has_meta {
                put(out, "value", rm.get("value"));
            } else {
                put(out, "", rm.get("value"));
            }
            put(out, "formatting", rm.get("formatting"));
        }
        // master05 §DV_CODED_TEXT: `|value` (display), `|code`/`|terminology`
        // (defining_code), `|preferred_term`, `|formatting`.
        "DV_CODED_TEXT" | "DV_STATE" => {
            put(out, "value", rm.get("value"));
            if let Some(dc) = rm.get("defining_code") {
                let (code, term, pref) = code_phrase_parts(dc);
                put(out, "code", code);
                put(out, "terminology", term);
                put(out, "preferred_term", pref);
            }
            put(out, "formatting", rm.get("formatting"));
        }
        // master05 §CODE_PHRASE.
        "CODE_PHRASE" => {
            let (code, term, pref) = code_phrase_parts(rm);
            put(out, "code", code);
            put(out, "terminology", term);
            put(out, "preferred_term", pref);
        }
        // master05 §DV_QUANTITY (RM `units` field → the `|unit` suffix).
        "DV_QUANTITY" => {
            put(out, "magnitude", rm.get("magnitude"));
            put(out, "unit", rm.get("units"));
            put(out, "precision", rm.get("precision"));
            emit_quantified_extras(rm, out, true);
        }
        // master05 §DV_COUNT: the bare value is `magnitude`.
        "DV_COUNT" => {
            put(out, "", rm.get("magnitude"));
            emit_quantified_extras(rm, out, true);
        }
        // master05 §DV_PROPORTION: numerator/denominator/type/precision, plus the
        // bare magnitude "calculated on output".
        "DV_PROPORTION" => {
            put(out, "numerator", rm.get("numerator"));
            put(out, "denominator", rm.get("denominator"));
            put(out, "type", rm.get("type"));
            put(out, "precision", rm.get("precision"));
            if let (Some(n), Some(d)) = (
                rm.get("numerator").and_then(Value::as_f64),
                rm.get("denominator").and_then(Value::as_f64),
            ) && d != 0.0
            {
                put(out, "", Some(&json!(n / d)));
            }
            emit_quantified_extras(rm, out, true);
        }
        // master05 §DV_ORDINAL: `|value`/`|code` (the symbol), `|ordinal` (value).
        "DV_ORDINAL" => {
            put(out, "value", rm.pointer("/symbol/value"));
            emit_symbol_code(rm, out);
            put(out, "ordinal", rm.get("value"));
        }
        // DV_SCALE mirrors DV_ORDINAL with a Real `|scale` in place of `|ordinal`.
        "DV_SCALE" => {
            put(out, "value", rm.pointer("/symbol/value"));
            emit_symbol_code(rm, out);
            put(out, "scale", rm.get("value"));
        }
        // master05 §DV_BOOLEAN: bare boolean.
        "DV_BOOLEAN" => put(out, "", rm.get("value")),
        // master05 §DV_DURATION: bare ISO-8601 duration + the amount extras.
        "DV_DURATION" => {
            put(out, "", rm.get("value"));
            emit_quantified_extras(rm, out, true);
        }
        // master05 §§DV_DATE, DV_DATE_TIME, DV_TIME: bare ISO-8601 value; the
        // `accuracy` (a DV_DURATION) is the `/_accuracy` sub-path, emitted by
        // the `_`-attribute layer.
        "DV_DATE_TIME" | "DV_DATE" | "DV_TIME" => {
            put(out, "", rm.get("value"));
            emit_quantified_extras(rm, out, false);
        }
        // master05 §§DV_URI, DV_EHR_URI: bare string.
        "DV_URI" | "DV_EHR_URI" => put(out, "", rm.get("value")),
        // master05 §DV_PARSABLE: bare value + `|formalism`.
        "DV_PARSABLE" => {
            put(out, "", rm.get("value"));
            put(out, "formalism", rm.get("formalism"));
        }
        // master05 §DV_IDENTIFIER — delegated to the party/identifier module.
        "DV_IDENTIFIER" => parties::emit_identifier(rm, out),
        // master05 §PARTY_PROXY → §§PARTY_SELF, PARTY_IDENTIFIED,
        // PARTY_RELATED: a PARTY_PROXY-typed leaf (`ENTRY.subject` —
        // master05 §§ADMIN_ENTRY/INSTRUCTION/ACTION/EVALUATION/OBSERVATION
        // `/subject` row) decomposes into the inlined
        // `|name`/`|id`/`|id_scheme`/`|id_namespace` suffixes plus the
        // `/_identifier:i` and `/relationship` sub-paths.
        "PARTY_PROXY" | "PARTY_SELF" | "PARTY_IDENTIFIED" | "PARTY_RELATED" => {
            parties::emit_party(rm, out);
        }
        // master05 §DV_MULTIMEDIA: bare = `uri.value`, plus the attribute set;
        // `/_thumbnail`, `/_charset`, `/_language` are `_`-attribute sub-paths.
        "DV_MULTIMEDIA" => emit_multimedia(rm, out),
        // master05 §DV_INTERVAL: a leaf interval whose `/lower`/`/upper`
        // endpoints are sub-paths (children). NOTE: this is the one leaf
        // type whose datum is not attrs-only — `emit_leaf` emits its endpoint
        // children here, since master05 §DV_INTERVAL models them as sub-paths.
        "DV_INTERVAL" => {
            let t = base_type(generic_arg(rm_type).unwrap_or("DV_QUANTITY"));
            emit_interval(rm, t, out);
        }
        // Any remaining `DV_*` leaf falls back to its scalar `value`.
        _ => put(out, "", rm.get("value")),
    }
}

fn emit_symbol_code(rm: &Value, out: &mut SimNode) {
    if let Some(dc) = rm.pointer("/symbol/defining_code") {
        let (code, term, _) = code_phrase_parts(dc);
        put(out, "code", code);
        put(out, "terminology", term);
    }
}

fn emit_multimedia(rm: &Value, out: &mut SimNode) {
    if let Some(uri) = rm.get("uri").filter(|u| !u.is_null()) {
        put(out, "", uri.get("value"));
    }
    if let Some(mt) = rm.get("media_type") {
        put(out, "mediatype", mt.get("code_string"));
    }
    put(out, "alternatetext", rm.get("alternate_text"));
    if let Some(size) = rm.get("size").filter(|s| s.as_i64().is_some_and(|n| n > 0)) {
        put(out, "size", Some(size));
    }
    if let Some(ca) = rm.get("compression_algorithm") {
        put(out, "compression_algorithm", ca.get("code_string"));
    }
    put(out, "integrity_check", rm.get("integrity_check"));
    if let Some(ica) = rm.get("integrity_check_algorithm") {
        put(out, "integrity_check_algorithm", ica.get("code_string"));
    }
    put(out, "data", rm.get("data"));
}

/// Emit the value-internal `_`-families of a leaf value (master05: the
/// `_normal_range`/`_other_reference_ranges` of the `DV_ORDERED` family, the
/// `_accuracy` of the temporal family, the `_language`/`_encoding`/`_mapping`
/// of the text family, and the `_thumbnail`/`_charset`/`_language` of
/// DV_MULTIMEDIA / DV_PARSABLE). Called from [`super::structures::emit_rm_attrs`].
pub(super) fn emit_value_internal(rm: &Value, ty: &str, out: &mut SimNode) {
    match ty {
        "DV_TEXT" | "DV_PARAGRAPH" | "DV_CODED_TEXT" | "DV_STATE" => emit_text_meta(rm, out),
        "DV_PARSABLE" => {
            emit_code_phrase_sub(rm.get("charset"), "_charset", out);
            emit_code_phrase_sub(rm.get("language"), "_language", out);
        }
        "DV_MULTIMEDIA" => {
            if let Some(t) = rm.get("thumbnail").filter(|v| !v.is_null()) {
                emit_leaf(
                    t,
                    "DV_MULTIMEDIA",
                    None,
                    out.occurrence_mut("_thumbnail", None),
                );
            }
            emit_code_phrase_sub(rm.get("charset"), "_charset", out);
            emit_code_phrase_sub(rm.get("language"), "_language", out);
        }
        _ => {}
    }
    if DV_ORDERED.contains(&ty) {
        emit_reference_ranges(rm, ty, out);
        if matches!(ty, "DV_DATE" | "DV_DATE_TIME" | "DV_TIME")
            && let Some(acc) = rm.pointer("/accuracy/value")
        {
            out.occurrence_mut("_accuracy", None)
                .attrs
                .insert(String::new(), acc.clone());
        }
    }
}

/// `_language`/`_encoding` (CODE_PHRASE) + `_mapping:i` (TERM_MAPPING) on a
/// DV_TEXT-family value (master05 §§DV_TEXT, DV_CODED_TEXT, TERM_MAPPING).
fn emit_text_meta(rm: &Value, out: &mut SimNode) {
    emit_code_phrase_sub(rm.get("language"), "_language", out);
    emit_code_phrase_sub(rm.get("encoding"), "_encoding", out);
    if let Some(maps) = rm.get("mappings").and_then(Value::as_array) {
        for (i, m) in maps.iter().enumerate() {
            let child = out.occurrence_mut("_mapping", Some(u32::try_from(i).unwrap_or(u32::MAX)));
            put(child, "match", m.get("match"));
            emit_code_phrase_sub(m.get("target"), "target", child);
            if let Some(purpose) = m.get("purpose").filter(|v| !v.is_null()) {
                emit_leaf(
                    purpose,
                    "DV_CODED_TEXT",
                    None,
                    child.occurrence_mut("purpose", None),
                );
            }
        }
    }
}

fn emit_code_phrase_sub(cp: Option<&Value>, name: &str, out: &mut SimNode) {
    let Some(cp) = cp.filter(|v| !v.is_null()) else {
        return;
    };
    emit_leaf(cp, "CODE_PHRASE", None, out.occurrence_mut(name, None));
}

/// `_normal_range` (DV_INTERVAL`<T>`) + `_other_reference_ranges:i`
/// (REFERENCE_RANGE`<T>`), endpoints emitted via [`emit_leaf`] for `T`
/// (master05 §§DV_INTERVAL, REFERENCE_RANGE).
fn emit_reference_ranges(rm: &Value, t: &str, out: &mut SimNode) {
    if let Some(nr) = rm.get("normal_range").filter(|v| !v.is_null()) {
        emit_interval(nr, t, out.occurrence_mut("_normal_range", None));
    }
    if let Some(ranges) = rm.get("other_reference_ranges").and_then(Value::as_array) {
        for (i, r) in ranges.iter().enumerate() {
            let child = out.occurrence_mut(
                "_other_reference_ranges",
                Some(u32::try_from(i).unwrap_or(u32::MAX)),
            );
            if let Some(range) = r.get("range") {
                emit_interval(range, t, child);
            }
            if let Some(meaning) = r.get("meaning").filter(|v| !v.is_null()) {
                let mty = meaning
                    .get("_type")
                    .and_then(Value::as_str)
                    .unwrap_or("DV_TEXT");
                emit_leaf(meaning, mty, None, child.occurrence_mut("meaning", None));
            }
        }
    }
}

/// A DV_INTERVAL: `/lower`, `/upper` endpoints of type `t` + the boundary flags,
/// each flag emitted only when it differs from its default (master05
/// §DV_INTERVAL: `|lower_unbounded`/`|upper_unbounded` only if true;
/// `|lower_included`/`|upper_included` only if false).
fn emit_interval(iv: &Value, t: &str, out: &mut SimNode) {
    if let Some(lower) = iv.get("lower").filter(|v| !v.is_null()) {
        emit_leaf(lower, t, None, out.occurrence_mut("lower", None));
    }
    if let Some(upper) = iv.get("upper").filter(|v| !v.is_null()) {
        emit_leaf(upper, t, None, out.occurrence_mut("upper", None));
    }
    for (field, default) in [
        ("lower_unbounded", false),
        ("upper_unbounded", false),
        ("lower_included", true),
        ("upper_included", true),
    ] {
        if let Some(b) = iv.get(field).and_then(Value::as_bool)
            && b != default
        {
            out.attrs.insert(field.to_owned(), json!(b));
        }
    }
}

fn present(v: Option<&Value>) -> bool {
    v.is_some_and(|v| !v.is_null())
}

// ── sim → RM ────────────────────────────────────────────────────────────────

/// Build the RM leaf for `node` (see [`super::build_leaf`]). Returns `None` when
/// the present datum parts do not form a value of `rm_type`.
pub(super) fn build_leaf(
    node: &SimNode,
    rm_type: &str,
    wt_node: Option<&WebTemplateNode>,
) -> Option<Value> {
    let base = base_type(rm_type);
    let mut value = build_core(base, rm_type, node, wt_node).or_else(|| {
        infer_type(node)
            .filter(|t| *t != base)
            .and_then(|t| build_core(t, t, node, wt_node))
    })?;
    if let Value::Object(map) = &mut value {
        attach_value_internal(map, base, node);
    }
    Some(value)
}

/// Build the value-internal families onto an already-built core value. Used by
/// [`build_leaf`]; [`super::structures::build_rm_attr`] builds the same families
/// on demand when a walker routes a `_`-child to it instead.
fn attach_value_internal(map: &mut Map<String, Value>, base: &str, node: &SimNode) {
    if is_text_family(base) {
        attach_text_internal(map, node);
    }
    if base == "DV_PARSABLE" || base == "DV_MULTIMEDIA" {
        if let Some(child) = single(node, "_charset").and_then(build_code_phrase) {
            map.insert("charset".to_owned(), child);
        }
        if let Some(child) = single(node, "_language").and_then(build_code_phrase) {
            map.insert("language".to_owned(), child);
        }
    }
    if base == "DV_MULTIMEDIA"
        && let Some(t) = single(node, "_thumbnail")
        && let Some(dv) = build_leaf(t, "DV_MULTIMEDIA", None)
    {
        map.insert("thumbnail".to_owned(), dv);
    }
    if DV_ORDERED.contains(&base) {
        attach_ordered_internal(map, base, node);
    }
}

/// The text-family value-internal families: `language`, `encoding` and the
/// `_mapping:i` list.
fn attach_text_internal(map: &mut Map<String, Value>, node: &SimNode) {
    if let Some(cp) = single(node, "_language").and_then(build_code_phrase) {
        map.insert("language".to_owned(), cp);
    }
    if let Some(cp) = single(node, "_encoding").and_then(build_code_phrase) {
        map.insert("encoding".to_owned(), cp);
    }
    let mappings = build_indexed(node, "_mapping", build_term_mapping);
    if !mappings.is_empty() {
        map.insert("mappings".to_owned(), Value::Array(mappings));
    }
}

/// The DV_ORDERED value-internal families: the normal range, the other
/// reference ranges, and (temporal types only) the accuracy duration.
fn attach_ordered_internal(map: &mut Map<String, Value>, base: &str, node: &SimNode) {
    if let Some(child) = single(node, "_normal_range") {
        map.insert("normal_range".to_owned(), build_interval(child, base));
    }
    let ranges = build_indexed(node, "_other_reference_ranges", |n| {
        build_reference_range(n, base)
    });
    if !ranges.is_empty() {
        map.insert("other_reference_ranges".to_owned(), Value::Array(ranges));
    }
    if matches!(base, "DV_DATE" | "DV_DATE_TIME" | "DV_TIME")
        && let Some(acc) = single(node, "_accuracy").and_then(SimNode::bare)
    {
        map.insert(
            "accuracy".to_owned(),
            json!({"_type": "DV_DURATION", "value": acc.clone()}),
        );
    }
}

fn is_text_family(base: &str) -> bool {
    matches!(
        base,
        "DV_TEXT" | "DV_PARAGRAPH" | "DV_CODED_TEXT" | "DV_STATE"
    )
}

/// The single (index-0) occurrence of a `_`-family child, if present.
fn single<'a>(node: &'a SimNode, name: &str) -> Option<&'a SimNode> {
    node.children.get(name).and_then(|c| c.occurrences.first())
}

/// Build a `Vec` from every occurrence of the indexed `_`-family child `name`,
/// in occurrence order (skipping interior placeholder holes).
fn build_indexed<F>(node: &SimNode, name: &str, f: F) -> Vec<Value>
where
    F: Fn(&SimNode) -> Value,
{
    let Some(child) = family(node, name) else {
        return Vec::new();
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
) -> Option<Value> {
    match base {
        "DV_TEXT" | "DV_PARAGRAPH" => build_text(node, base),
        "DV_CODED_TEXT" | "DV_STATE" => build_coded_text(node, base, wt),
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
        "DV_IDENTIFIER" => Some(parties::build_identifier(node)),
        // master05 §PARTY_PROXY → the three subtype tables. `PERSON` is the
        // `PARTY_REF.type` used when the datum carries an `|id` but no
        // explicit reference type: the carriers of a PARTY_PROXY leaf
        // (`ENTRY.subject`, `COMPOSITION.composer`) reference a person, and
        // the master05 tables define no flat row for `external_ref.type`.
        "PARTY_PROXY" | "PARTY_SELF" | "PARTY_IDENTIFIED" | "PARTY_RELATED" => {
            Some(parties::build_party(node, "PERSON"))
        }
        "DV_MULTIMEDIA" => build_multimedia(node),
        "DV_PARSABLE" => build_parsable(node),
        "DV_INTERVAL" => Some(build_interval(
            node,
            base_type(generic_arg(rm_type).unwrap_or("DV_QUANTITY")),
        )),
        _ => bare_typed(node, base),
    }
}

/// The `T` of a generic slot type (`DV_INTERVAL<DV_QUANTITY>` → `DV_QUANTITY`).
fn generic_arg(rm_type: &str) -> Option<&str> {
    let start = rm_type.find('<')? + 1;
    let end = rm_type.rfind('>')?;
    if start >= end {
        return None;
    }
    Some(rm_type.get(start..end)?.trim())
}

fn attr<'a>(node: &'a SimNode, key: &str) -> Option<&'a Value> {
    node.attrs.get(key)
}

fn attr_str<'a>(node: &'a SimNode, key: &str) -> Option<&'a str> {
    node.attrs.get(key).and_then(Value::as_str)
}

fn build_text(node: &SimNode, ty: &str) -> Option<Value> {
    let value = node.bare().or_else(|| attr(node, "value"))?.clone();
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!(ty));
    o.insert("value".to_owned(), value);
    if let Some(f) = attr(node, "formatting") {
        o.insert("formatting".to_owned(), f.clone());
    }
    Some(Value::Object(o))
}

/// master05 §DV_CODED_TEXT. `|value`/`|terminology` are only required for
/// external terminologies; a local/openehr code resolves its rubric and the
/// implicit terminology from the web-template input list when the suffixes are
/// omitted.
fn build_coded_text(node: &SimNode, ty: &str, wt: Option<&WebTemplateNode>) -> Option<Value> {
    let code = attr(node, "code")?.clone();
    let code_str = code.as_str().unwrap_or_default();
    let terminology = attr_str(node, "terminology")
        .or_else(|| wt_terminology(wt))
        .unwrap_or("local")
        .to_owned();
    let value = attr(node, "value").cloned().or_else(|| {
        wt_coded_value(wt, code_str)
            .and_then(|cv| cv.label.clone())
            .map(Value::String)
    });
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!(ty));
    if let Some(v) = value {
        o.insert("value".to_owned(), v);
    }
    o.insert(
        "defining_code".to_owned(),
        code_phrase_obj(code, &terminology, attr(node, "preferred_term").cloned()),
    );
    if let Some(f) = attr(node, "formatting") {
        o.insert("formatting".to_owned(), f.clone());
    }
    Some(Value::Object(o))
}

/// master05 §CODE_PHRASE. `pub(super)` — reused for `_language`/`_encoding`/
/// `_charset`/`target` sub-code-phrases.
pub(super) fn build_code_phrase(node: &SimNode) -> Option<Value> {
    let code = attr(node, "code")?.clone();
    let terminology = attr_str(node, "terminology").unwrap_or("local");
    Some(code_phrase_obj(
        code,
        terminology,
        attr(node, "preferred_term").cloned(),
    ))
}

/// Apply the shared amount/ordered reverse extras (master05 note): rebuild
/// `normal_status` as a CODE_PHRASE in the implicit `openehr` terminology.
fn apply_amount_extras(o: &mut Map<String, Value>, node: &SimNode, with_accuracy: bool) {
    if let Some(v) = attr(node, "magnitude_status") {
        o.insert("magnitude_status".to_owned(), v.clone());
    }
    if with_accuracy {
        if let Some(v) = attr(node, "accuracy") {
            o.insert("accuracy".to_owned(), v.clone());
        }
        if let Some(v) = attr(node, "accuracy_is_percent") {
            o.insert("accuracy_is_percent".to_owned(), v.clone());
        }
    }
    if let Some(code) = attr_str(node, "normal_status") {
        o.insert(
            "normal_status".to_owned(),
            code_phrase_obj(json!(code), "openehr", None),
        );
    }
}

fn build_quantity(node: &SimNode) -> Option<Value> {
    let magnitude = attr(node, "magnitude")?.clone();
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!("DV_QUANTITY"));
    o.insert("magnitude".to_owned(), magnitude);
    if let Some(u) = attr(node, "unit") {
        o.insert("units".to_owned(), u.clone());
    }
    if let Some(p) = attr(node, "precision") {
        o.insert("precision".to_owned(), p.clone());
    }
    apply_amount_extras(&mut o, node, true);
    Some(Value::Object(o))
}

fn build_count(node: &SimNode) -> Option<Value> {
    let magnitude = node.bare()?.clone();
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!("DV_COUNT"));
    o.insert("magnitude".to_owned(), magnitude);
    apply_amount_extras(&mut o, node, true);
    Some(Value::Object(o))
}

fn build_proportion(node: &SimNode) -> Option<Value> {
    let numerator = attr(node, "numerator")?.clone();
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!("DV_PROPORTION"));
    o.insert("numerator".to_owned(), numerator);
    for (suffix, field) in [
        ("denominator", "denominator"),
        ("type", "type"),
        ("precision", "precision"),
    ] {
        if let Some(v) = attr(node, suffix) {
            o.insert(field.to_owned(), v.clone());
        }
    }
    // The bare value is the computed magnitude ("calculated on output"); it is
    // derived from numerator/denominator, so it is not stored back.
    apply_amount_extras(&mut o, node, true);
    Some(Value::Object(o))
}

/// master05 §§DV_ORDINAL, DV_SCALE. `|value`/`|ordinal`(`|scale`) may be omitted
/// when the symbol is defined in the template — resolved from the web-template
/// coded value for `|code`.
fn build_ordinal(
    node: &SimNode,
    ty: &str,
    numeric_suffix: &str,
    wt: Option<&WebTemplateNode>,
) -> Option<Value> {
    let code = attr(node, "code")?.clone();
    let code_str = code.as_str().unwrap_or_default();
    let template = wt_coded_value(wt, code_str);
    let numeric = attr(node, numeric_suffix).cloned().or_else(|| {
        template.and_then(|cv| match ty {
            "DV_SCALE" => cv.scale.map(|s| json!(s)),
            _ => cv.ordinal.map(|o| json!(o)),
        })
    })?;
    let terminology = attr_str(node, "terminology").unwrap_or("local").to_owned();
    let value = attr(node, "value")
        .cloned()
        .or_else(|| template.and_then(|cv| cv.label.clone()).map(Value::String));
    let mut symbol = Map::new();
    symbol.insert("_type".to_owned(), json!("DV_CODED_TEXT"));
    if let Some(v) = value {
        symbol.insert("value".to_owned(), v);
    }
    symbol.insert(
        "defining_code".to_owned(),
        code_phrase_obj(code, &terminology, None),
    );
    Some(json!({"_type": ty, "value": numeric, "symbol": Value::Object(symbol)}))
}

/// A temporal leaf (`DV_DATE`/`DV_DATE_TIME`/`DV_TIME`) or `DV_DURATION`: bare
/// value + the amount/ordered extras (master05 §§DV_DATE/DATE_TIME/TIME/DURATION).
fn build_amount_temporal(node: &SimNode, ty: &str, with_accuracy: bool) -> Option<Value> {
    let value = node.bare()?.clone();
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!(ty));
    o.insert("value".to_owned(), value);
    apply_amount_extras(&mut o, node, with_accuracy);
    Some(Value::Object(o))
}

/// master05 §DV_MULTIMEDIA: bare = `uri`; `media_type` + `size` are RM-mandatory
/// (`size` defaults to 0 when the FLAT lacks `|size`).
fn build_multimedia(node: &SimNode) -> Option<Value> {
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!("DV_MULTIMEDIA"));
    if let Some(uri) = node.bare() {
        o.insert(
            "uri".to_owned(),
            json!({"_type": "DV_URI", "value": uri.clone()}),
        );
    }
    if let Some(mt) = attr(node, "mediatype") {
        o.insert(
            "media_type".to_owned(),
            code_phrase_obj(mt.clone(), "IANA_media-types", None),
        );
    }
    if let Some(a) = attr(node, "alternatetext") {
        o.insert("alternate_text".to_owned(), a.clone());
    }
    if let Some(ca) = attr_str(node, "compression_algorithm") {
        o.insert(
            "compression_algorithm".to_owned(),
            code_phrase_obj(json!(ca), "openehr_compression_algorithms", None),
        );
    }
    if let Some(ic) = attr(node, "integrity_check") {
        o.insert("integrity_check".to_owned(), ic.clone());
    }
    if let Some(ica) = attr_str(node, "integrity_check_algorithm") {
        o.insert(
            "integrity_check_algorithm".to_owned(),
            code_phrase_obj(json!(ica), "openehr_integrity_check_algorithms", None),
        );
    }
    if let Some(d) = attr(node, "data") {
        o.insert("data".to_owned(), d.clone());
    }
    let size = attr(node, "size")
        .and_then(Value::as_i64)
        .or_else(|| attr_str(node, "size").and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    o.insert("size".to_owned(), json!(size));
    if o.contains_key("media_type") || o.contains_key("uri") || o.contains_key("data") {
        Some(Value::Object(o))
    } else {
        None
    }
}

fn build_parsable(node: &SimNode) -> Option<Value> {
    let value = node.bare().or_else(|| attr(node, "value"))?.clone();
    let formalism = attr(node, "formalism")
        .cloned()
        .unwrap_or_else(|| json!(""));
    Some(json!({"_type": "DV_PARSABLE", "value": value, "formalism": formalism}))
}

fn bare_typed(node: &SimNode, ty: &str) -> Option<Value> {
    Some(json!({"_type": ty, "value": node.bare()?.clone()}))
}

/// master05 §TERM_MAPPING. `pub(super)` — reused by [`super::structures`] for the
/// `_mapping:i` family via [`build_indexed`].
pub(super) fn build_term_mapping(node: &SimNode) -> Value {
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!("TERM_MAPPING"));
    o.insert(
        "match".to_owned(),
        json!(attr_str(node, "match").unwrap_or("=")),
    );
    if let Some(target) = single(node, "target").and_then(build_code_phrase) {
        o.insert("target".to_owned(), target);
    }
    if let Some(purpose) =
        single(node, "purpose").and_then(|p| build_leaf(p, "DV_CODED_TEXT", None))
    {
        o.insert("purpose".to_owned(), purpose);
    }
    Value::Object(o)
}

/// master05 §DV_INTERVAL: `/lower`, `/upper` (of type `t`) + the boundary flags
/// (defaults: unbounded=false, included=true). `pub(super)` — reused by the
/// `_normal_range` family in [`super::structures`].
pub(super) fn build_interval(node: &SimNode, t: &str) -> Value {
    let mut iv = Map::new();
    iv.insert("_type".to_owned(), json!("DV_INTERVAL"));
    if let Some(dv) = single(node, "lower").and_then(|n| build_leaf(n, t, None)) {
        iv.insert("lower".to_owned(), dv);
    }
    if let Some(dv) = single(node, "upper").and_then(|n| build_leaf(n, t, None)) {
        iv.insert("upper".to_owned(), dv);
    }
    let flag = |s: &str, default: bool| attr(node, s).and_then(Value::as_bool).unwrap_or(default);
    iv.insert(
        "lower_unbounded".to_owned(),
        json!(flag("lower_unbounded", false)),
    );
    iv.insert(
        "upper_unbounded".to_owned(),
        json!(flag("upper_unbounded", false)),
    );
    iv.insert(
        "lower_included".to_owned(),
        json!(flag("lower_included", true)),
    );
    iv.insert(
        "upper_included".to_owned(),
        json!(flag("upper_included", true)),
    );
    Value::Object(iv)
}

/// master05 §REFERENCE_RANGE: a `range` (DV_INTERVAL`<T>`) + a `meaning` (DV_TEXT,
/// or DV_CODED_TEXT when coded). `pub(super)` — reused for the
/// `_other_reference_ranges:i` family in [`super::structures`].
pub(super) fn build_reference_range(node: &SimNode, t: &str) -> Value {
    let meaning = single(node, "meaning");
    let meaning_val = match meaning {
        None => json!({"_type": "DV_TEXT", "value": ""}),
        Some(m) => {
            let mty = if m.attrs.contains_key("code") {
                "DV_CODED_TEXT"
            } else {
                "DV_TEXT"
            };
            build_leaf(m, mty, None).unwrap_or_else(|| json!({"_type": "DV_TEXT", "value": ""}))
        }
    };
    json!({
        "_type": "REFERENCE_RANGE",
        "meaning": meaning_val,
        "range": build_interval(node, t),
    })
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
        let rm = build_leaf(&node, "DV_QUANTITY", None).unwrap();
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
        let rm = build_leaf(&node, "DV_COUNT", None).unwrap();
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
        let rm = build_leaf(&node, "DV_CODED_TEXT", Some(&wt)).unwrap();
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
        let rm = build_leaf(&node, "DV_ORDINAL", Some(&wt)).unwrap();
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
        let rm = build_leaf(&node, "DV_MULTIMEDIA", None).unwrap();
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
        let rm = build_leaf(&node, "DV_QUANTITY", None).unwrap();
        assert_eq!(rm["normal_range"]["lower"]["magnitude"], json!(20.5));
        assert_eq!(rm["normal_range"]["lower"]["units"], json!("unit"));
    }
}
