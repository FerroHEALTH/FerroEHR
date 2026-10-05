// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Party / reference / participation / identifier codecs.
//!
//! Covers the ITS-REST `simplified_formats/master05-rm_mapping.adoc` tables for
//! `PARTY_SELF`, `PARTY_IDENTIFIED`, `PARTY_RELATED` (the `PARTY_PROXY` subtypes),
//! `OBJECT_REF`, `PARTICIPATION`, and `DV_IDENTIFIER`. These shapes are shared by
//! the ENTRY / EVENT_CONTEXT `_`-attribute families in [`super::structures`] and
//! by the `_identifier:i` / performer inlining rules.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::data_values;
use crate::flat::sim::SimNode;

// ── DV_IDENTIFIER (master05 §DV_IDENTIFIER) ───────────────────────────────────

/// DV_IDENTIFIER → `|id`/`|issuer`/`|assigner`/`|type` attrs on `out`.
pub(super) fn emit_identifier(rm: &Value, out: &mut SimNode) {
    for (field, suffix) in [
        ("id", "id"),
        ("type", "type"),
        ("issuer", "issuer"),
        ("assigner", "assigner"),
    ] {
        if let Some(v) = rm.get(field).filter(|v| !v.is_null()) {
            out.attrs.insert(suffix.to_owned(), v.clone());
        }
    }
}

/// A DV_IDENTIFIER from `|id` (or, per master05 §DV_IDENTIFIER "for the input
/// |id might be left out", the bare value) plus the optional `|issuer` /
/// `|assigner` / `|type`.
pub(super) fn build_identifier(node: &SimNode) -> Value {
    let id = node
        .attrs
        .get("id")
        .or_else(|| node.bare())
        .cloned()
        .unwrap_or_else(|| json!(""));
    let mut o = Map::new();
    o.insert("_type".to_owned(), json!("DV_IDENTIFIER"));
    o.insert("id".to_owned(), id);
    // `issuer`/`assigner`/`type` are optional; only set when present so an empty
    // string is never fabricated (which would break the round-trip).
    for field in ["type", "issuer", "assigner"] {
        if let Some(v) = node.attrs.get(field) {
            o.insert(field.to_owned(), v.clone());
        }
    }
    Value::Object(o)
}

// ── OBJECT_REF (master05 §OBJECT_REF) ─────────────────────────────────────────

/// OBJECT_REF → `|type`/`|id`/`|id_scheme`/`|namespace` attrs on `out`.
///
/// NOTE: master05 §OBJECT_REF's table row is `|scheme` (→ `id.scheme`), but
/// every spec example carries `|id_scheme` (e.g. INSTRUCTION `_guideline_id`,
/// master05 §INSTRUCTION). The example blocks are the wire authority, so this
/// emits `|id_scheme`; [`build_object_ref`] accepts either.
pub(super) fn emit_object_ref(oref: &Value, out: &mut SimNode) {
    if let Some(v) = oref.get("type").filter(|v| !v.is_null()) {
        out.attrs.insert("type".to_owned(), v.clone());
    }
    if let Some(v) = oref.pointer("/id/value") {
        out.attrs.insert("id".to_owned(), v.clone());
    }
    if let Some(v) = oref.pointer("/id/scheme") {
        out.attrs.insert("id_scheme".to_owned(), v.clone());
    }
    if let Some(v) = oref.get("namespace").filter(|v| !v.is_null()) {
        out.attrs.insert("namespace".to_owned(), v.clone());
    }
}

/// An OBJECT_REF from `|id` (+ `|type`/`|id_scheme`(or `|scheme`)/`|namespace`).
/// `None` when no `|id` is present. master05 §OBJECT_REF: `type`, `id.value`,
/// `id.scheme`, `namespace` are all required, so absent parts default.
pub(super) fn build_object_ref(node: &SimNode) -> Option<Value> {
    let id = node.attrs.get("id").and_then(Value::as_str)?;
    let ty = node
        .attrs
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("ANY");
    let scheme = node
        .attrs
        .get("id_scheme")
        .or_else(|| node.attrs.get("scheme"))
        .and_then(Value::as_str)
        .unwrap_or("id_scheme");
    let ns = node
        .attrs
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or("EHR");
    Some(json!({
        "_type": "OBJECT_REF",
        "namespace": ns,
        "type": ty,
        "id": {"_type": "GENERIC_ID", "value": id, "scheme": scheme},
    }))
}

// ── PARTY_PROXY family (master05 §§PARTY_SELF, PARTY_IDENTIFIED, PARTY_RELATED) ─

/// PARTY_PROXY (`PARTY_SELF`/`PARTY_IDENTIFIED`/`PARTY_RELATED`) → inlined
/// `|name`/`|id`/`|id_scheme`/`|id_namespace` attrs on `out`, plus `_identifier:i`
/// (PARTY_IDENTIFIED) and a `/relationship` DV_CODED_TEXT child (PARTY_RELATED).
///
/// The three subtype tables (master05 §§PARTY_SELF, PARTY_IDENTIFIED,
/// PARTY_RELATED) share the `|id`/`|id_scheme`/`|id_namespace` rows, so the
/// concrete subtype is carried by two discriminators: a `/relationship` child
/// means PARTY_RELATED (master05 §PARTY_RELATED), and `|_type` marks a
/// PARTY_SELF (master05 §FEEDER_AUDIT_DETAILS, `/subject` row Note: "add
/// /subject|_type: PARTY_SELF"). Without the marker a PARTY_SELF would rebuild
/// as a PARTY_IDENTIFIED, so it is emitted whenever the value is one.
///
/// NOTE: master05 §PARTY_RELATED spells the relationship sub-path two ways —
/// both example blocks of the section write
/// `…/composer/relationship|code`/`|value`/`|terminology` (and the
/// §"PARTY_RELATED performer" table row is likewise `/relationship`), while
/// the §PARTY_RELATED mapping table's Flat Path column reads
/// `/_relationship`. The example spelling is the emitted one (it is the form
/// two tables and every example agree on); the underscore spelling is
/// accepted on input as an alias by [`relationship_child`].
///
/// This is the **sole** emission site of the party `_identifier:i` family
/// (master05 §§PARTY_IDENTIFIED, PARTY_RELATED `/_identifier:i`):
/// [`super::structures::emit_rm_attrs`] deliberately carries no PARTY arm, so
/// the family is emitted exactly once however a party node is reached — a
/// PARTY_PROXY datum leaf through [`super::data_values::emit_leaf`], or a
/// `_provider` / `_health_care_facility` / FEEDER_AUDIT_DETAILS party through
/// [`super::structures`].
pub(super) fn emit_party(p: &Value, out: &mut SimNode) {
    if p.get("_type").and_then(Value::as_str) == Some("PARTY_SELF") {
        out.attrs.insert("_type".to_owned(), json!("PARTY_SELF"));
    }
    if let Some(name) = p.get("name").filter(|v| !v.is_null()) {
        out.attrs.insert("name".to_owned(), name.clone());
    }
    if let Some(id) = p.pointer("/external_ref/id/value") {
        out.attrs.insert("id".to_owned(), id.clone());
    }
    if let Some(s) = p.pointer("/external_ref/id/scheme") {
        out.attrs.insert("id_scheme".to_owned(), s.clone());
    }
    if let Some(ns) = p.pointer("/external_ref/namespace") {
        out.attrs.insert("id_namespace".to_owned(), ns.clone());
    }
    if let Some(ids) = p.get("identifiers").and_then(Value::as_array) {
        for (i, id) in ids.iter().enumerate() {
            emit_identifier(
                id,
                out.occurrence_mut(
                    IDENTIFIER_SEGMENT,
                    Some(u32::try_from(i).unwrap_or(u32::MAX)),
                ),
            );
        }
    }
    if let Some(rel) = p.get("relationship").filter(|v| !v.is_null()) {
        data_values::emit_leaf(
            rel,
            "DV_CODED_TEXT",
            None,
            out.occurrence_mut("relationship", None),
        );
    }
}

/// Build a PARTY_PROXY from a party sim node. `PARTY_SELF` when `|_type` is
/// `"PARTY_SELF"` (master05 §FEEDER_AUDIT_DETAILS note: "add /subject|_type:
/// PARTY_SELF"); `PARTY_RELATED` when a `/relationship` child is present
/// (master05 §PARTY_RELATED); otherwise `PARTY_IDENTIFIED` (master05
/// §PARTY_IDENTIFIED). `default_party_type` is the `PARTY_REF.type` used when an
/// `|id` is present but no explicit ref type (`PERSON`/`ORGANISATION`).
///
/// Both master05 spellings of the relationship sub-path are accepted here (see
/// [`relationship_child`]); consuming it in this builder — rather than routing
/// `_relationship` through [`super::build_rm_attr`] like the other
/// `_`-prefixed families — is what makes the alias correct: the child is the
/// PARTY_PROXY subtype discriminator, so it must be read *before* the concrete
/// party type is decided, not attached afterwards to an already-typed
/// PARTY_IDENTIFIED.
pub(super) fn build_party(node: &SimNode, default_party_type: &str) -> Value {
    let explicit_self = node
        .attrs
        .get("_type")
        .and_then(Value::as_str)
        .is_some_and(|t| t == "PARTY_SELF");
    let relationship = relationship_child(node);
    let identifiers = build_party_identifiers(node);

    let mut p = Map::new();
    if explicit_self {
        p.insert("_type".to_owned(), json!("PARTY_SELF"));
        if let Some(ext) = external_ref(node, default_party_type) {
            p.insert("external_ref".to_owned(), ext);
        }
        return Value::Object(p);
    }

    let ty = if relationship.is_some() {
        "PARTY_RELATED"
    } else {
        "PARTY_IDENTIFIED"
    };
    p.insert("_type".to_owned(), json!(ty));
    if let Some(name) = node.attrs.get("name") {
        p.insert("name".to_owned(), name.clone());
    }
    if let Some(ext) = external_ref(node, default_party_type) {
        p.insert("external_ref".to_owned(), ext);
    }
    if !identifiers.is_empty() {
        p.insert("identifiers".to_owned(), Value::Array(identifiers));
    }
    if let Some(rel) = relationship.and_then(|r| data_values::build_leaf(r, "DV_CODED_TEXT", None))
    {
        p.insert("relationship".to_owned(), rel);
    }
    Value::Object(p)
}

/// The `PARTY_REF` external reference for a party sim node, when it carries an
/// `|id` (master05 §PARTY_IDENTIFIED: `|id_namespace` is required when `|id` is
/// set). A `|id_scheme` makes the id a `GENERIC_ID`; without one it is a
/// `HIER_OBJECT_ID` (so a scheme is never fabricated).
fn external_ref(node: &SimNode, default_party_type: &str) -> Option<Value> {
    let id = node.attrs.get("id").and_then(Value::as_str)?;
    let namespace = node
        .attrs
        .get("id_namespace")
        .and_then(Value::as_str)
        .unwrap_or("EHR");
    let id_obj = match node.attrs.get("id_scheme").and_then(Value::as_str) {
        Some(scheme) => json!({"_type": "GENERIC_ID", "value": id, "scheme": scheme}),
        None => json!({"_type": "HIER_OBJECT_ID", "value": id}),
    };
    Some(json!({
        "_type": "PARTY_REF",
        "id": id_obj,
        "namespace": namespace,
        "type": default_party_type,
    }))
}

/// The `_identifier:i` DV_IDENTIFIER array on a PARTY node (master05
/// §PARTY_IDENTIFIED `/_identifier:i`).
fn build_party_identifiers(node: &SimNode) -> Vec<Value> {
    node.children
        .get(IDENTIFIER_SEGMENT)
        .map(|c| {
            c.occurrences
                .iter()
                .filter(|o| !o.is_empty())
                .map(build_identifier)
                .collect()
        })
        .unwrap_or_default()
}

// ── PARTICIPATION (master05 §PARTICIPATION) ───────────────────────────────────

/// PARTICIPATION → `|function`/`|mode`, the inlined performer
/// (`|name`/`|id`/`|id_scheme`/`|id_namespace`, `|identifiers_*:i`), and a
/// PARTY_RELATED `/relationship` child.
pub(super) fn emit_participation(p: &Value, out: &mut SimNode) {
    if let Some(f) = p.pointer("/function/value") {
        out.attrs.insert("function".to_owned(), f.clone());
    }
    if let Some(m) = p.pointer("/mode/value") {
        out.attrs.insert("mode".to_owned(), m.clone());
    }
    if let Some(performer) = p.get("performer").filter(|v| !v.is_null()) {
        emit_performer(performer, out);
    }
}

/// The inlined performer of a PARTICIPATION: its `|name`/`|id`/`|id_scheme`/
/// `|id_namespace` suffixes, the `|identifiers_<field>:i` list, and a
/// PARTY_RELATED `/relationship` child.
///
/// Performer identifiers are inlined as `|identifiers_<field>:i` suffixes
/// (master05 §PARTICIPATION), not the `_identifier:i` sub-path form.
fn emit_performer(performer: &Value, out: &mut SimNode) {
    if let Some(name) = performer.get("name").filter(|v| !v.is_null()) {
        out.attrs.insert("name".to_owned(), name.clone());
    }
    for (pointer, suffix) in [
        ("/external_ref/id/value", "id"),
        ("/external_ref/id/scheme", "id_scheme"),
        ("/external_ref/namespace", "id_namespace"),
    ] {
        if let Some(v) = performer.pointer(pointer) {
            out.attrs.insert(suffix.to_owned(), v.clone());
        }
    }
    for (i, id) in performer
        .get("identifiers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (field, suffix) in [
            ("id", "identifiers_id"),
            ("issuer", "identifiers_issuer"),
            ("assigner", "identifiers_assigner"),
            ("type", "identifiers_type"),
        ] {
            if let Some(v) = id.get(field).filter(|v| !v.is_null()) {
                out.attrs.insert(format!("{suffix}:{i}"), v.clone());
            }
        }
    }
    if let Some(rel) = performer.get("relationship").filter(|v| !v.is_null()) {
        data_values::emit_leaf(
            rel,
            "DV_CODED_TEXT",
            None,
            out.occurrence_mut("relationship", None),
        );
    }
}

/// Build a PARTICIPATION: `function` (DV_TEXT), the inlined `performer`, and an
/// optional coded `mode` resolved against the openEHR `participation mode` group
/// (master05 §PARTICIPATION note "ValueSet openEHR participation mode group").
pub(super) fn build_participation(node: &SimNode) -> Value {
    let mut p = Map::new();
    p.insert("_type".to_owned(), json!("PARTICIPATION"));
    p.insert(
        "function".to_owned(),
        json!({
            "_type": "DV_TEXT",
            "value": node.attrs.get("function").and_then(Value::as_str).unwrap_or(""),
        }),
    );
    p.insert("performer".to_owned(), build_performer(node));
    if let Some(mode) = node.attrs.get("mode").and_then(Value::as_str) {
        p.insert(
            "mode".to_owned(),
            super::coded_from_group("participation_mode", mode),
        );
    }
    Value::Object(p)
}

/// The inlined performer PARTY of a PARTICIPATION (`PARTY_IDENTIFIED`, or
/// `PARTY_RELATED` when a `/relationship` child is present), including the
/// `|identifiers_*:i` performer identifiers.
fn build_performer(node: &SimNode) -> Value {
    let relationship = single(node, "relationship");
    let identifiers = build_inline_identifiers(node);
    let mut p = Map::new();
    let ty = if relationship.is_some() {
        "PARTY_RELATED"
    } else {
        "PARTY_IDENTIFIED"
    };
    p.insert("_type".to_owned(), json!(ty));
    if let Some(name) = node.attrs.get("name") {
        p.insert("name".to_owned(), name.clone());
    }
    if let Some(ext) = external_ref(node, "PERSON") {
        p.insert("external_ref".to_owned(), ext);
    }
    if !identifiers.is_empty() {
        p.insert("identifiers".to_owned(), Value::Array(identifiers));
    }
    if let Some(rel) = relationship.and_then(|r| data_values::build_leaf(r, "DV_CODED_TEXT", None))
    {
        p.insert("relationship".to_owned(), rel);
    }
    Value::Object(p)
}

/// Rebuild the performer `identifiers` from the inlined `|identifiers_<field>:i`
/// attrs, ordered by `i` (master05 §PARTICIPATION).
fn build_inline_identifiers(node: &SimNode) -> Vec<Value> {
    let mut by_index: BTreeMap<u32, Map<String, Value>> = BTreeMap::new();
    for (key, value) in &node.attrs {
        let Some(rest) = key.strip_prefix("identifiers_") else {
            continue;
        };
        let Some((field, idx)) = rest.split_once(':') else {
            continue;
        };
        let Ok(idx) = idx.parse::<u32>() else {
            continue;
        };
        if !matches!(field, "id" | "issuer" | "assigner" | "type") {
            continue;
        }
        by_index
            .entry(idx)
            .or_default()
            .insert(field.to_owned(), value.clone());
    }
    by_index
        .into_values()
        .map(|mut fields| {
            fields.insert("_type".to_owned(), json!("DV_IDENTIFIER"));
            fields.entry("id".to_owned()).or_insert_with(|| json!(""));
            Value::Object(fields)
        })
        .collect()
}

/// The single (index-0) occurrence of a sub-path child of a party node.
fn single<'a>(node: &'a SimNode, name: &str) -> Option<&'a SimNode> {
    node.children.get(name).and_then(|c| c.occurrences.first())
}

/// The PARTY_RELATED `relationship` DV_CODED_TEXT child of a party node, under
/// either spelling master05 §PARTY_RELATED gives it.
///
/// The section's two example blocks write
/// `"…/composer/relationship|code": "10"` (and §"PARTY_RELATED performer"
/// repeats `/relationship` in both its table and its example), while the
/// §PARTY_RELATED mapping table's Flat Path column reads `/_relationship`.
/// The spec contradicts itself, so both are read on input; only the example
/// spelling is ever emitted ([`emit_party`]), keeping RM → FLAT → RM stable.
fn relationship_child(node: &SimNode) -> Option<&SimNode> {
    single(node, "relationship").or_else(|| single(node, RELATIONSHIP_TABLE_SEGMENT))
}

/// The master05 §PARTY_RELATED *table* spelling of the relationship sub-path,
/// accepted on input as an alias of the example spelling `/relationship`.
/// [`crate::flat::build`] excludes this segment from the generic `_`-family
/// routing on a PARTY leaf, because [`build_party`] consumes it.
pub(crate) const RELATIONSHIP_TABLE_SEGMENT: &str = "_relationship";

/// The party identifier family's segment (master05 §§PARTY_IDENTIFIED,
/// PARTY_RELATED `/_identifier:i`). [`build_party`] is its single consumption
/// site — [`crate::flat::build`] excludes it from the generic `_`-family
/// routing on a PARTY leaf, whose `identifier` arm would otherwise rebuild the
/// same children over the key and, on an all-empty family, write the
/// `identifiers: []` that RM `Identifiers_valid` forbids.
pub(crate) const IDENTIFIER_SEGMENT: &str = "_identifier";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node_of(attrs: &[(&str, Value)]) -> SimNode {
        let mut n = SimNode::default();
        for (k, v) in attrs {
            n.attrs.insert((*k).to_owned(), v.clone());
        }
        n
    }

    // master05 §DV_IDENTIFIER: `|id` (+ optional issuer/assigner/type) round-trip,
    // and the bare-input tolerance.
    #[test]
    fn identifier_roundtrip_and_bare_input() {
        let rm = json!({"_type": "DV_IDENTIFIER", "id": "A123", "issuer": "Issuer", "type": "Prescription"});
        let mut out = SimNode::default();
        emit_identifier(&rm, &mut out);
        assert_eq!(out.attrs.get("id"), Some(&json!("A123")));
        assert_eq!(out.attrs.get("type"), Some(&json!("Prescription")));

        let bare = node_of(&[("", json!("A123"))]);
        let dv = build_identifier(&bare);
        assert_eq!(dv["id"], json!("A123"));
    }

    // master05 §PARTY_IDENTIFIED: `|name`/`|id`/`|id_scheme`/`|id_namespace` +
    // `_identifier:i` round-trip.
    #[test]
    fn party_identified_roundtrip() {
        let rm = json!({
            "_type": "PARTY_IDENTIFIED", "name": "Silvia Blake",
            "external_ref": {"_type": "PARTY_REF",
                "namespace": "EHR.NETWORK", "type": "PERSON",
                "id": {"_type": "GENERIC_ID", "value": "1234-5678", "scheme": "UUID"}},
            "identifiers": [{"_type": "DV_IDENTIFIER", "id": "122", "issuer": "issuer"}]
        });
        let mut out = SimNode::default();
        emit_party(&rm, &mut out);
        assert_eq!(out.attrs.get("name"), Some(&json!("Silvia Blake")));
        assert_eq!(out.attrs.get("id"), Some(&json!("1234-5678")));
        assert_eq!(out.attrs.get("id_scheme"), Some(&json!("UUID")));

        let built = build_party(&out, "PERSON");
        assert_eq!(built["_type"], json!("PARTY_IDENTIFIED"));
        assert_eq!(built["name"], json!("Silvia Blake"));
        assert_eq!(built["external_ref"]["id"]["scheme"], json!("UUID"));
        assert_eq!(built["identifiers"][0]["id"], json!("122"));
    }

    // master05 §PARTY_SELF: `/subject|_type: PARTY_SELF` marks a PARTY_SELF.
    #[test]
    fn party_self_via_type_marker() {
        let node = node_of(&[
            ("_type", json!("PARTY_SELF")),
            ("id", json!("x")),
            ("id_namespace", json!("EHR")),
        ]);
        let built = build_party(&node, "PERSON");
        assert_eq!(built["_type"], json!("PARTY_SELF"));
        assert_eq!(built["external_ref"]["id"]["value"], json!("x"));
    }

    // master05 §PARTICIPATION: inlined performer + `|identifiers_*:i`; the mode is
    // coded against the openEHR participation-mode group.
    #[test]
    fn participation_inlined_performer_and_identifiers() {
        let node = node_of(&[
            ("function", json!("requester")),
            ("mode", json!("face-to-face communication")),
            ("name", json!("Dr. Marcus Johnson")),
            ("id", json!("199")),
            ("id_namespace", json!("HOSPITAL-NS")),
            ("identifiers_id:0", json!("122")),
            ("identifiers_issuer:0", json!("issuer")),
        ]);
        let p = build_participation(&node);
        assert_eq!(p["function"]["value"], json!("requester"));
        assert_eq!(p["performer"]["name"], json!("Dr. Marcus Johnson"));
        assert_eq!(p["performer"]["identifiers"][0]["id"], json!("122"));
        assert_eq!(p["mode"]["_type"], json!("DV_CODED_TEXT"));
        assert_eq!(p["mode"]["value"], json!("face-to-face communication"));
        // resolved to a real openEHR participation-mode code (not fabricated).
        assert!(p["mode"]["defining_code"]["code_string"].is_string());
    }

    // master05 §OBJECT_REF: `|id_scheme` accepted (spec example form).
    #[test]
    fn object_ref_builds_from_id_scheme() {
        let node = node_of(&[
            ("type", json!("GUIDELINE")),
            ("id", json!("3445")),
            ("id_scheme", json!("HOSPITAL-NS")),
            ("namespace", json!("HOSPITAL-NS")),
        ]);
        let o = build_object_ref(&node).unwrap();
        assert_eq!(o["type"], json!("GUIDELINE"));
        assert_eq!(o["id"]["value"], json!("3445"));
        assert_eq!(o["id"]["scheme"], json!("HOSPITAL-NS"));
        assert_eq!(o["namespace"], json!("HOSPITAL-NS"));
    }

    /// A party node carrying the relationship sub-path under `name`.
    fn party_with_relationship(segment: &str) -> SimNode {
        let mut node = node_of(&[("name", json!("Susan Doe"))]);
        let child = node.occurrence_mut(segment, None);
        child.attrs.insert("code".to_owned(), json!("10"));
        child.attrs.insert("value".to_owned(), json!("mother"));
        child
            .attrs
            .insert("terminology".to_owned(), json!("openehr"));
        node
    }

    // master05 §PARTY_RELATED gives the relationship sub-path TWO spellings:
    // the mapping table's Flat Path column reads `/_relationship`, while both
    // example blocks of the same section (and the §"PARTY_RELATED performer"
    // table + example) write `…/relationship|code`. Both are accepted on
    // input and both make the party a PARTY_RELATED — the child is the
    // subtype discriminator, so `build_party` must consume it whichever way
    // it is spelled.
    #[test]
    fn party_related_accepts_both_master05_relationship_spellings() {
        let example = build_party(&party_with_relationship("relationship"), "PERSON");
        let table = build_party(
            &party_with_relationship(RELATIONSHIP_TABLE_SEGMENT),
            "PERSON",
        );
        assert_eq!(example["_type"], json!("PARTY_RELATED"));
        assert_eq!(
            table, example,
            "the `/_relationship` table spelling must build the same \
             PARTY_RELATED as the `/relationship` example spelling"
        );
        assert_eq!(table["relationship"]["defining_code"]["code_string"], "10");
    }

    // The emitted spelling stays the example one — the alias is input-only, so
    // an RM → FLAT → RM round-trip never starts producing `/_relationship`.
    #[test]
    fn party_related_emits_only_the_example_relationship_spelling() {
        let rm = json!({
            "_type": "PARTY_RELATED", "name": "Susan Doe",
            "relationship": {
                "_type": "DV_CODED_TEXT", "value": "mother",
                "defining_code": {"_type": "CODE_PHRASE",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"},
                    "code_string": "10"},
            },
        });
        let mut out = SimNode::default();
        emit_party(&rm, &mut out);
        assert!(
            out.children.contains_key("relationship"),
            "master05 §PARTY_RELATED example blocks: `/relationship` is emitted"
        );
        assert!(
            !out.children.contains_key(RELATIONSHIP_TABLE_SEGMENT),
            "the `/_relationship` table spelling is an INPUT alias only; \
             emitting it would change the wire: {:?}",
            out.children.keys().collect::<Vec<_>>()
        );
        let rebuilt = build_party(&out, "PERSON");
        assert_eq!(rebuilt["_type"], json!("PARTY_RELATED"));
        assert_eq!(rebuilt["name"], json!("Susan Doe"));
        assert_eq!(rebuilt["relationship"]["value"], json!("mother"));
        assert_eq!(
            rebuilt["relationship"]["defining_code"]["code_string"],
            json!("10")
        );
    }

    // The party `_identifier:i` family (master05 §§PARTY_IDENTIFIED,
    // PARTY_RELATED `/_identifier:i`) has exactly ONE emission site,
    // `emit_party`. This pins the ownership split in both directions, so a
    // second emission site fails here rather than silently.
    #[test]
    fn party_identifiers_have_exactly_one_emission_site() {
        let rm = json!({
            "_type": "PARTY_IDENTIFIED", "name": "Silvia Blake",
            "identifiers": [{"_type": "DV_IDENTIFIER", "id": "122", "issuer": "issuer"}],
        });

        let mut from_party = SimNode::default();
        emit_party(&rm, &mut from_party);
        assert_eq!(
            from_party
                .child("_identifier")
                .and_then(|o| o.attrs.get("id")),
            Some(&json!("122")),
            "`emit_party` owns the `_identifier:i` family"
        );

        let mut from_rm_attrs = SimNode::default();
        crate::flat::map::structures::emit_rm_attrs(&rm, "PARTY_IDENTIFIED", &mut from_rm_attrs);
        assert!(
            !from_rm_attrs.children.contains_key("_identifier"),
            "the `_`-attribute dispatcher must NOT re-emit the party \
             `_identifier:i` family — `emit_party` already did: {:?}",
            from_rm_attrs.children.keys().collect::<Vec<_>>()
        );
    }
    // …and exactly ONE CONSUMPTION site on the way back: `build_party`. The
    // generic `_`-family router still owns an `identifier` arm for hosts that
    // are not parties, so the split is enforced by
    // `crate::flat::build::leaf_consumes_segment`. This pins the edge the two
    // sites do NOT agree on: an `_identifier` family whose occurrences are all
    // empty. `build_party` omits the key; the router's arm would insert
    // `identifiers: []`, which RM `PARTY_IDENTIFIED`'s `Identifiers_valid`
    // forbids and ITS-REST Resources.md §JSON Format keeps off the wire.
    #[test]
    fn an_all_empty_party_identifier_family_yields_no_identifiers_key() {
        let mut party = SimNode::default();
        party.attrs.insert("name".to_owned(), json!("Silvia Blake"));
        // The parsers prune an all-empty family away, so the disagreement is
        // only reachable here — which is exactly why it needs a pin.
        let _ = party.occurrence_mut(IDENTIFIER_SEGMENT, Some(0));

        let built = build_party(&party, "PERSON");
        assert_eq!(built["_type"], json!("PARTY_IDENTIFIED"));
        assert!(
            built.get("identifiers").is_none(),
            "an empty identifier family is absent, never `identifiers: []`: {built}"
        );

        // A non-empty family still lands, so the omission is about emptiness
        // and not about the segment being dropped.
        let mut with_id = SimNode::default();
        with_id
            .occurrence_mut(IDENTIFIER_SEGMENT, Some(0))
            .attrs
            .insert("id".to_owned(), json!("122"));
        let built = build_party(&with_id, "PERSON");
        assert_eq!(built["identifiers"][0]["id"], json!("122"));
    }
}
