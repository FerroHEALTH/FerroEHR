// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM → simplified-tree emitters, read from any [`Instance`]: the
//! `DATA_VALUE` leaves of the `master05-rm_mapping.adoc` DV tables (the mirror
//! of [`super::data_values`]) and the `_`-prefixed optional RM-attribute
//! families with `LINK`, `FEEDER_AUDIT`, `ISM_TRANSITION` and
//! `INSTRUCTION_DETAILS` (the mirror of [`super::structures`]).

use openehr_rm::v1_2::typing::base_name;
use serde_json::json;

use super::data_values::{DV_ORDERED, interval_limit};
use super::parties;
use crate::flat::instance::{Instance, at};
use crate::flat::sim::SimNode;

/// Insert `out.attrs[suffix]` if `v` is present and non-null. `suffix == ""` is
/// the bare datum.
fn put<'a, I: Instance<'a>>(out: &mut SimNode, suffix: &str, v: Option<I>) {
    if let Some(v) = v.and_then(Instance::to_value) {
        out.attrs.insert(suffix.to_owned(), v);
    }
}

/// The `code_string` / `terminology_id.value` / `preferred_term` of a CODE_PHRASE.
fn code_phrase_parts<'a, I: Instance<'a>>(cp: I) -> (Option<I>, Option<I>, Option<I>) {
    (
        cp.attr("code_string"),
        at(cp, &["terminology_id", "value"]),
        cp.attr("preferred_term"),
    )
}

/// Emit the shared `DV_QUANTIFIED`/`DV_ORDERED` scalar extras (master05
/// §§DV_QUANTITY, DV_COUNT, DV_PROPORTION, DV_DURATION): `|magnitude_status`,
/// `|normal_status` (= `normal_status.code_string`; the CODE_PHRASE terminology
/// is implicitly `openehr`), `|accuracy`, `|accuracy_is_percent`.
/// `with_accuracy` is false for the temporal family, whose `accuracy` is a
/// `DV_DURATION` carried as `/_accuracy` (master05 §§DV_DATE, DV_DATE_TIME,
/// DV_TIME).
fn emit_quantified_extras<'a, I: Instance<'a>>(dv: I, out: &mut SimNode, with_accuracy: bool) {
    put(out, "magnitude_status", dv.attr("magnitude_status"));
    put(
        out,
        "normal_status",
        at(dv, &["normal_status", "code_string"]),
    );
    if with_accuracy {
        put(out, "accuracy", dv.attr("accuracy"));
        put(out, "accuracy_is_percent", dv.attr("accuracy_is_percent"));
    }
}

/// RM leaf value → datum attrs on `out` (the [`super`] module docs, `emit_leaf`).
#[expect(
    clippy::match_same_arms,
    reason = "the arms are kept explicit per RM type so each mapping stays readable next to its type name"
)]
pub(crate) fn emit_leaf<'a, I: Instance<'a>>(
    rm: I,
    rm_type: &str,
    list_open: Option<bool>,
    out: &mut SimNode,
) {
    let ty = rm.rm_type().unwrap_or_else(|| base_name(rm_type));
    match ty {
        // master05 §DV_TEXT (and DV_PARAGRAPH shares the shape). A DV_TEXT held
        // in an *open* DV_CODED_TEXT slot is the `|other` free-text branch
        // (master04 §"Open Value-Sets"); otherwise the bare value, or `|value`
        // when the node also carries metadata that keeps it from being a bare
        // leaf (`|formatting` / `_mapping`).
        "DV_TEXT" | "DV_PARAGRAPH" => {
            let coded_slot = base_name(rm_type) == "DV_CODED_TEXT";
            let open_slot = coded_slot && list_open != Some(false);
            let has_meta = rm.is_present("formatting") || rm.is_present("mappings");
            if open_slot {
                put(out, "other", rm.attr("value"));
            } else if has_meta {
                put(out, "value", rm.attr("value"));
            } else {
                put(out, "", rm.attr("value"));
            }
            put(out, "formatting", rm.attr("formatting"));
        }
        // master05 §DV_CODED_TEXT: `|value` (display), `|code`/`|terminology`
        // (defining_code), `|preferred_term`, `|formatting`.
        "DV_CODED_TEXT" => {
            put(out, "value", rm.attr("value"));
            if let Some(dc) = rm.attr("defining_code") {
                let (code, term, pref) = code_phrase_parts(dc);
                put(out, "code", code);
                put(out, "terminology", term);
                put(out, "preferred_term", pref);
            }
            put(out, "formatting", rm.attr("formatting"));
        }
        // master05 has no §DV_STATE table, so the value goes out whole as
        // master04 §Raw canonical JSON (`|raw`), the one specified carrier.
        "DV_STATE" => put(out, "raw", Some(rm)),
        // master05 §CODE_PHRASE.
        "CODE_PHRASE" => {
            let (code, term, pref) = code_phrase_parts(rm);
            put(out, "code", code);
            put(out, "terminology", term);
            put(out, "preferred_term", pref);
        }
        // master05 §DV_QUANTITY (RM `units` field → the `|unit` suffix).
        "DV_QUANTITY" => {
            put(out, "magnitude", rm.attr("magnitude"));
            put(out, "unit", rm.attr("units"));
            put(out, "precision", rm.attr("precision"));
            emit_quantified_extras(rm, out, true);
        }
        // master05 §DV_COUNT: the bare value is `magnitude`.
        "DV_COUNT" => {
            put(out, "", rm.attr("magnitude"));
            emit_quantified_extras(rm, out, true);
        }
        // master05 §DV_PROPORTION: numerator/denominator/type/precision, plus the
        // bare magnitude "calculated on output".
        "DV_PROPORTION" => {
            put(out, "numerator", rm.attr("numerator"));
            put(out, "denominator", rm.attr("denominator"));
            put(out, "type", rm.attr("type"));
            put(out, "precision", rm.attr("precision"));
            if let (Some(n), Some(d)) = (
                rm.attr("numerator").and_then(Instance::as_f64),
                rm.attr("denominator").and_then(Instance::as_f64),
            ) && d != 0.0
            {
                out.attrs.insert(String::new(), json!(n / d));
            }
            emit_quantified_extras(rm, out, true);
        }
        // master05 §DV_ORDINAL: `|value`/`|code` (the symbol), `|ordinal` (value).
        "DV_ORDINAL" => {
            put(out, "value", at(rm, &["symbol", "value"]));
            emit_symbol_code(rm, out);
            put(out, "ordinal", rm.attr("value"));
        }
        // DV_SCALE mirrors DV_ORDINAL with a Real `|scale` in place of `|ordinal`.
        "DV_SCALE" => {
            put(out, "value", at(rm, &["symbol", "value"]));
            emit_symbol_code(rm, out);
            put(out, "scale", rm.attr("value"));
        }
        // master05 §DV_BOOLEAN: bare boolean.
        "DV_BOOLEAN" => put(out, "", rm.attr("value")),
        // master05 §DV_DURATION: bare ISO-8601 duration + the amount extras.
        "DV_DURATION" => {
            put(out, "", rm.attr("value"));
            emit_quantified_extras(rm, out, true);
        }
        // master05 §§DV_DATE, DV_DATE_TIME, DV_TIME: bare ISO-8601 value; the
        // `accuracy` (a DV_DURATION) is the `/_accuracy` sub-path, emitted by
        // the `_`-attribute layer.
        "DV_DATE_TIME" | "DV_DATE" | "DV_TIME" => {
            put(out, "", rm.attr("value"));
            emit_quantified_extras(rm, out, false);
        }
        // master05 §§DV_URI, DV_EHR_URI: bare string.
        "DV_URI" | "DV_EHR_URI" => put(out, "", rm.attr("value")),
        // master05 §DV_PARSABLE: bare value + `|formalism`.
        "DV_PARSABLE" => {
            put(out, "", rm.attr("value"));
            put(out, "formalism", rm.attr("formalism"));
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
            parties::emit_party_as(rm, "PERSON", out);
        }
        // master05 §DV_MULTIMEDIA: bare = `uri.value`, plus the attribute set;
        // `/_thumbnail`, `/_charset`, `/_language` are `_`-attribute sub-paths.
        "DV_MULTIMEDIA" => emit_multimedia(rm, out),
        // master05 §DV_INTERVAL: a leaf interval whose `/lower`/`/upper`
        // endpoints are sub-paths (children). NOTE: this is the one leaf
        // type whose datum is not attrs-only — `emit_leaf` emits its endpoint
        // children here, since master05 §DV_INTERVAL models them as sub-paths.
        "DV_INTERVAL" => {
            emit_interval(rm, &interval_limit(rm_type), out);
        }
        // Any remaining `DV_*` leaf falls back to its scalar `value`.
        _ => put(out, "", rm.attr("value")),
    }
}

fn emit_symbol_code<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    if let Some(dc) = at(rm, &["symbol", "defining_code"]) {
        let (code, term, _) = code_phrase_parts(dc);
        put(out, "code", code);
        put(out, "terminology", term);
    }
}

fn emit_multimedia<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    if let Some(uri) = rm.attr("uri") {
        put(out, "", uri.attr("value"));
    }
    if let Some(mt) = rm.attr("media_type") {
        put(out, "mediatype", mt.attr("code_string"));
    }
    put(out, "alternatetext", rm.attr("alternate_text"));
    if let Some(size) = rm
        .attr("size")
        .filter(|s| s.as_i64().is_some_and(|n| n > 0))
    {
        put(out, "size", Some(size));
    }
    if let Some(ca) = rm.attr("compression_algorithm") {
        put(out, "compression_algorithm", ca.attr("code_string"));
    }
    put(out, "integrity_check", rm.attr("integrity_check"));
    if let Some(ica) = rm.attr("integrity_check_algorithm") {
        put(out, "integrity_check_algorithm", ica.attr("code_string"));
    }
    put(out, "data", rm.attr("data"));
}

/// Emit the value-internal `_`-families of a leaf value (master05: the
/// `_normal_range`/`_other_reference_ranges` of the `DV_ORDERED` family, the
/// `_accuracy` of the temporal family, the `_language`/`_encoding`/`_mapping`
/// of the text family, and the `_thumbnail`/`_charset`/`_language` of
/// DV_MULTIMEDIA / DV_PARSABLE). Called from [`emit_rm_attrs`].
pub(super) fn emit_value_internal<'a, I: Instance<'a>>(rm: I, ty: &str, out: &mut SimNode) {
    match ty {
        "DV_TEXT" | "DV_PARAGRAPH" | "DV_CODED_TEXT" => emit_text_meta(rm, out),
        "DV_PARSABLE" => {
            emit_code_phrase_sub(rm.attr("charset"), "_charset", out);
            emit_code_phrase_sub(rm.attr("language"), "_language", out);
        }
        "DV_MULTIMEDIA" => {
            if let Some(t) = rm.attr("thumbnail") {
                emit_leaf(
                    t,
                    "DV_MULTIMEDIA",
                    None,
                    out.occurrence_mut("_thumbnail", None),
                );
            }
            emit_code_phrase_sub(rm.attr("charset"), "_charset", out);
            emit_code_phrase_sub(rm.attr("language"), "_language", out);
        }
        _ => {}
    }
    if DV_ORDERED.contains(&ty) {
        emit_reference_ranges(rm, ty, out);
        if matches!(ty, "DV_DATE" | "DV_DATE_TIME" | "DV_TIME")
            && let Some(acc) = at(rm, &["accuracy", "value"]).and_then(Instance::to_value)
        {
            out.occurrence_mut("_accuracy", None)
                .attrs
                .insert(String::new(), acc);
        }
    }
}

/// `_language`/`_encoding` (CODE_PHRASE) + `_mapping:i` (TERM_MAPPING) on a
/// DV_TEXT-family value (master05 §§DV_TEXT, DV_CODED_TEXT, TERM_MAPPING).
fn emit_text_meta<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    emit_code_phrase_sub(rm.attr("language"), "_language", out);
    emit_code_phrase_sub(rm.attr("encoding"), "_encoding", out);
    if let Some(maps) = rm.members("mappings") {
        for (i, m) in maps.into_iter().enumerate() {
            let child = out.occurrence_mut("_mapping", Some(u32::try_from(i).unwrap_or(u32::MAX)));
            put(child, "match", m.attr("match"));
            emit_code_phrase_sub(m.attr("target"), "target", child);
            if let Some(purpose) = m.attr("purpose") {
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

fn emit_code_phrase_sub<'a, I: Instance<'a>>(cp: Option<I>, name: &str, out: &mut SimNode) {
    let Some(cp) = cp else {
        return;
    };
    emit_leaf(cp, "CODE_PHRASE", None, out.occurrence_mut(name, None));
}

/// `_normal_range` (DV_INTERVAL`<T>`) + `_other_reference_ranges:i`
/// (REFERENCE_RANGE`<T>`), endpoints emitted via [`emit_leaf`] for `T`
/// (master05 §§DV_INTERVAL, REFERENCE_RANGE).
fn emit_reference_ranges<'a, I: Instance<'a>>(rm: I, t: &str, out: &mut SimNode) {
    if let Some(nr) = rm.attr("normal_range") {
        emit_interval(nr, t, out.occurrence_mut("_normal_range", None));
    }
    if let Some(ranges) = rm.members("other_reference_ranges") {
        for (i, r) in ranges.into_iter().enumerate() {
            let child = out.occurrence_mut(
                "_other_reference_ranges",
                Some(u32::try_from(i).unwrap_or(u32::MAX)),
            );
            if let Some(range) = r.attr("range") {
                emit_interval(range, t, child);
            }
            if let Some(meaning) = r.attr("meaning") {
                let mty = meaning.rm_type().unwrap_or("DV_TEXT");
                emit_leaf(meaning, mty, None, child.occurrence_mut("meaning", None));
            }
        }
    }
}

/// A DV_INTERVAL: `/lower`, `/upper` endpoints of type `t` + the boundary flags,
/// each flag emitted only when it differs from its default (master05
/// §DV_INTERVAL: `|lower_unbounded`/`|upper_unbounded` only if true;
/// `|lower_included`/`|upper_included` only if false).
pub(super) fn emit_interval<'a, I: Instance<'a>>(iv: I, t: &str, out: &mut SimNode) {
    if let Some(lower) = iv.attr("lower") {
        emit_leaf(lower, t, None, out.occurrence_mut("lower", None));
    }
    if let Some(upper) = iv.attr("upper") {
        emit_leaf(upper, t, None, out.occurrence_mut("upper", None));
    }
    for (field, default) in [
        ("lower_unbounded", false),
        ("upper_unbounded", false),
        ("lower_included", true),
        ("upper_included", true),
    ] {
        if let Some(b) = iv.attr(field).and_then(Instance::as_bool)
            && b != default
        {
            out.attrs.insert(field.to_owned(), json!(b));
        }
    }
}

// ── the `_`-prefixed RM-attribute families ─────────────────────────────────────

/// The `_`-prefixed families present on `rm` (the [`super`] module docs, `emit_rm_attrs`).
pub(crate) fn emit_rm_attrs<'a, I: Instance<'a>>(rm: I, rm_type: &str, out: &mut SimNode) {
    let ty = rm.rm_type().unwrap_or_else(|| base_name(rm_type));

    if is_locatable(ty) {
        emit_locatable_attrs(rm, out);
    }
    match ty {
        "ELEMENT" => emit_element_attrs(rm, out),
        "OBSERVATION" | "EVALUATION" | "INSTRUCTION" | "ACTION" | "ADMIN_ENTRY" => {
            emit_entry_attrs(rm, ty, out);
        }
        // A PARTY arm here would emit the family a second time onto the same
        // node: idempotent, invisible on the wire, and a trap for the next
        // change to either site (pinned by
        // `parties::tests::party_identifiers_have_exactly_one_emission_site`).
        // NOTE: no PARTY arm — the party `_identifier:i` family (master05
        // §§PARTY_IDENTIFIED, PARTY_RELATED) is emitted by
        // [`parties::emit_party`], which every route to a party node uses.
        "ISM_TRANSITION" => emit_ism_transition_attrs(rm, out),
        "EVENT_CONTEXT" => emit_event_context_attrs(rm, out),
        _ => {}
    }

    // The value-internal families of a `DV_*` leaf (ranges / accuracy / text
    // meta / multimedia thumbnail+charset+language).
    emit_value_internal(rm, ty, out);
}

/// The LOCATABLE families: `_uid`, the `_link:i` list and `_feeder_audit`
/// (master05 §LOCATABLE).
fn emit_locatable_attrs<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    if let Some(uid) = at(rm, &["uid", "value"]).and_then(Instance::to_value) {
        out.occurrence_mut("_uid", None)
            .attrs
            .insert(String::new(), uid);
    }
    if let Some(links) = rm.members("links") {
        for (i, link) in links.into_iter().enumerate() {
            emit_link(
                link,
                out.occurrence_mut("_link", Some(u32::try_from(i).unwrap_or(u32::MAX))),
            );
        }
    }
    if let Some(fa) = rm.attr("feeder_audit") {
        let target = out.occurrence_mut("_feeder_audit", None);
        if audit_parties_survive(fa) {
            emit_feeder_audit(fa, target);
        } else if let Some(raw) = fa.to_value() {
            target.attrs.insert("raw".to_owned(), raw);
        }
    }
}

/// Whether every party of both FEEDER_AUDIT_DETAILS survives the master05
/// suffixes, whose builder fills `PERSON` in as the reference type
/// ([`parties::ref_type_survives`]); the feeder audit is written whole as
/// `|raw` otherwise (master04 §Raw canonical JSON).
fn audit_parties_survive<'a, I: Instance<'a>>(fa: I) -> bool {
    ["originating_system_audit", "feeder_system_audit"]
        .iter()
        .filter_map(|details| fa.attr(details))
        .flat_map(|details| {
            ["location", "subject", "provider"]
                .iter()
                .filter_map(move |party| details.attr(party))
        })
        .all(|party| parties::ref_type_survives(party, "PERSON"))
}

/// The ELEMENT families: `_null_flavour` and `_null_reason` (master05
/// §ELEMENT).
fn emit_element_attrs<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    if let Some(nf) = rm.attr("null_flavour") {
        emit_leaf(
            nf,
            "DV_CODED_TEXT",
            None,
            out.occurrence_mut("_null_flavour", None),
        );
    }
    if let Some(nr) = rm.attr("null_reason") {
        emit_leaf(
            nr,
            "DV_TEXT",
            None,
            out.occurrence_mut("_null_reason", None),
        );
    }
}

/// The ENTRY families, plus the INSTRUCTION and ACTION additions (master05
/// §§ENTRY, INSTRUCTION, ACTION).
fn emit_entry_attrs<'a, I: Instance<'a>>(rm: I, ty: &str, out: &mut SimNode) {
    if let Some(o) = rm.attr("guideline_id") {
        parties::emit_object_ref(o, out.occurrence_mut("_guideline_id", None));
    }
    if let Some(o) = rm.attr("workflow_id") {
        parties::emit_object_ref(o, out.occurrence_mut("_work_flow_id", None));
    }
    if let Some(p) = rm.attr("provider") {
        parties::emit_party_as(p, "PERSON", out.occurrence_mut("_provider", None));
    }
    if let Some(parts) = rm.members("other_participations") {
        for (i, p) in parts.into_iter().enumerate() {
            parties::emit_participation(
                p,
                out.occurrence_mut(
                    "_other_participation",
                    Some(u32::try_from(i).unwrap_or(u32::MAX)),
                ),
            );
        }
    }
    if ty == "INSTRUCTION" {
        if let Some(et) = rm.attr("expiry_time") {
            emit_leaf(
                et,
                "DV_DATE_TIME",
                None,
                out.occurrence_mut("_expiry_time", None),
            );
        }
        if let Some(wf) = rm.attr("wf_definition") {
            emit_leaf(
                wf,
                "DV_PARSABLE",
                None,
                out.occurrence_mut("_wf_definition", None),
            );
        }
    }
    if ty == "ACTION"
        && let Some(det) = rm.attr("instruction_details")
    {
        emit_instruction_details(det, out.occurrence_mut("_instruction_details", None));
    }
}

/// The ISM_TRANSITION family: the `_reason:i` DV_TEXT list (master05
/// §ISM_TRANSITION).
fn emit_ism_transition_attrs<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    let Some(reasons) = rm.members("reason") else {
        return;
    };
    for (i, r) in reasons.into_iter().enumerate() {
        emit_leaf(
            r,
            "DV_TEXT",
            None,
            out.occurrence_mut("_reason", Some(u32::try_from(i).unwrap_or(u32::MAX))),
        );
    }
}

/// The EVENT_CONTEXT families (master05 §EVENT_CONTEXT).
///
/// A composition's context is normally surfaced through the `ctx/` vocabulary
/// (see [`crate::flat::ctx`]); these emit only when a walker renders an
/// EVENT_CONTEXT node through the tree instead. `end_time`/`location` surface
/// as the lossless `ctx/` scalars (master06 §§end_time, location) and are not
/// re-emitted here; their `_end_time`/`_location` path forms stay accepted on
/// input.
fn emit_event_context_attrs<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    if let Some(hcf) = rm.attr("health_care_facility") {
        parties::emit_party_as(
            hcf,
            "ORGANISATION",
            out.occurrence_mut("_health_care_facility", None),
        );
    }
    if let Some(parts) = rm.members("participations") {
        for (i, p) in parts.into_iter().enumerate() {
            parties::emit_participation(
                p,
                out.occurrence_mut("_participation", Some(u32::try_from(i).unwrap_or(u32::MAX))),
            );
        }
    }
}

fn is_locatable(ty: &str) -> bool {
    matches!(
        ty,
        "COMPOSITION"
            | "SECTION"
            | "OBSERVATION"
            | "EVALUATION"
            | "INSTRUCTION"
            | "ACTION"
            | "ADMIN_ENTRY"
            | "GENERIC_ENTRY"
            | "CLUSTER"
            | "ELEMENT"
    )
}

/// LINK → `|type`/`|meaning`/`|target` (master05 §LINK).
fn emit_link<'a, I: Instance<'a>>(link: I, out: &mut SimNode) {
    for (attr, suffix) in [
        ("type", "type"),
        ("meaning", "meaning"),
        ("target", "target"),
    ] {
        if let Some(v) = at(link, &[attr, "value"]).and_then(Instance::to_value) {
            out.attrs.insert(suffix.to_owned(), v);
        }
    }
}

/// FEEDER_AUDIT (master05 §FEEDER_AUDIT): the two `*_system_audit`
/// FEEDER_AUDIT_DETAILS, `original_content`(`_multimedia`), and the
/// `*_system_item_id:i` DV_IDENTIFIER lists.
fn emit_feeder_audit<'a, I: Instance<'a>>(fa: I, out: &mut SimNode) {
    if let Some(d) = fa.attr("originating_system_audit") {
        emit_audit_details(d, out.occurrence_mut("originating_system_audit", None));
    }
    if let Some(d) = fa.attr("feeder_system_audit") {
        emit_audit_details(d, out.occurrence_mut("feeder_system_audit", None));
    }
    if let Some(oc) = fa.attr("original_content") {
        let ty = oc.rm_type().unwrap_or("DV_PARSABLE");
        let name = if ty == "DV_MULTIMEDIA" {
            "original_content_multimedia"
        } else {
            "original_content"
        };
        emit_leaf(oc, ty, None, out.occurrence_mut(name, None));
    }
    for (arr_key, seg) in [
        ("originating_system_item_ids", "originating_system_item_id"),
        ("feeder_system_item_ids", "feeder_system_item_id"),
    ] {
        if let Some(ids) = fa.members(arr_key) {
            for (i, id) in ids.into_iter().enumerate() {
                parties::emit_identifier(
                    id,
                    out.occurrence_mut(seg, Some(u32::try_from(i).unwrap_or(u32::MAX))),
                );
            }
        }
    }
}

/// FEEDER_AUDIT_DETAILS (master05 §FEEDER_AUDIT_DETAILS): `|system_id`,
/// `|version_id`, `|time`, and the `location`/`subject`/`provider` parties.
fn emit_audit_details<'a, I: Instance<'a>>(d: I, out: &mut SimNode) {
    if let Some(v) = d.attr("system_id").and_then(Instance::to_value) {
        out.attrs.insert("system_id".to_owned(), v);
    }
    if let Some(v) = d.attr("version_id").and_then(Instance::to_value) {
        out.attrs.insert("version_id".to_owned(), v);
    }
    if let Some(v) = at(d, &["time", "value"]).and_then(Instance::to_value) {
        out.attrs.insert("time".to_owned(), v);
    }
    for party in ["location", "subject", "provider"] {
        if let Some(p) = d.attr(party) {
            parties::emit_party(p, out.occurrence_mut(party, None));
        }
    }
}

/// INSTRUCTION_DETAILS (master05 §INSTRUCTION_DETAILS): exactly three STRING
/// suffixes on the `_instruction_details` node itself — `|path` →
/// `instruction_id.path`, `|composition_uid` → `instruction_id.id`,
/// `|activity_id` → `activity_id`. The table and the section's example block
/// agree; `instruction_id` is NOT a nested node on the simplified wire and the
/// generic OBJECT_REF suffixes (`|id`/`|type`/`|namespace`) are not defined here.
fn emit_instruction_details<'a, I: Instance<'a>>(det: I, out: &mut SimNode) {
    if let Some(iid) = det.attr("instruction_id") {
        if let Some(v) = iid.attr("path").and_then(Instance::to_value) {
            out.attrs.insert("path".to_owned(), v);
        }
        if let Some(v) = at(iid, &["id", "value"]).and_then(Instance::to_value) {
            out.attrs.insert("composition_uid".to_owned(), v);
        }
    }
    if let Some(aid) = det.attr("activity_id").and_then(Instance::to_value) {
        out.attrs.insert("activity_id".to_owned(), aid);
    }
}
