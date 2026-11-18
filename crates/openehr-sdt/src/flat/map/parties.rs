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
    reason = "a datum of the simplified wire is a JSON scalar (ITS-REST Simplified Formats); \
              the builders read it into typed RM and the emitters write it (#1694, #6003)"
)]

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{data_values, emit};
use crate::flat::error::FlatError;
use crate::flat::instance::{Instance, at};
use crate::flat::sim::SimNode;
use crate::flat::typed::{self, Build, Obj, o, s};
use openehr_rm::v1_2::access;

// ── DV_IDENTIFIER (master05 §DV_IDENTIFIER) ───────────────────────────────────

/// DV_IDENTIFIER → `|id`/`|issuer`/`|assigner`/`|type` attrs on `out`.
pub(super) fn emit_identifier<'a, I: Instance<'a>>(rm: I, out: &mut SimNode) {
    for (field, suffix) in [
        ("id", "id"),
        ("type", "type"),
        ("issuer", "issuer"),
        ("assigner", "assigner"),
    ] {
        if let Some(v) = rm.attr(field).and_then(Instance::to_value) {
            out.attrs.insert(suffix.to_owned(), v);
        }
    }
}

/// A DV_IDENTIFIER from `|id` (or, per master05 §DV_IDENTIFIER "for the input
/// |id might be left out", the bare value) plus the optional `|issuer` /
/// `|assigner` / `|type`.
pub(super) fn build_identifier(node: &SimNode) -> Result<Obj, FlatError> {
    let mut o = access::construct("DV_IDENTIFIER")?;
    match node.attrs.get("id").or_else(|| node.bare()) {
        Some(id) => o.put_datum("id", id)?,
        None => o.put("id", s(""))?,
    }
    // `issuer`/`assigner`/`type` are optional; only set when present so an empty
    // string is never fabricated (which would break the round-trip).
    for field in ["type", "issuer", "assigner"] {
        if let Some(v) = node.attrs.get(field) {
            o.put_datum(field, v)?;
        }
    }
    Ok(o)
}

/// A DV_IDENTIFIER from `(field, datum)` pairs of `id`/`issuer`/`assigner`/
/// `type`, with an empty `id` when none is given (master05 §PARTICIPATION,
/// master06 §Participation).
///
/// # Errors
/// [`FlatError::Rm`] for a datum that is no string.
pub(crate) fn identifier_from_fields(fields: &[(&str, &Value)]) -> Result<Obj, FlatError> {
    let mut o = access::construct("DV_IDENTIFIER")?;
    for (field, value) in fields {
        o.put_datum(field, value)?;
    }
    o.or_put("id", || Ok(s("")))?;
    Ok(o)
}

// ── OBJECT_REF (master05 §OBJECT_REF) ─────────────────────────────────────────

/// OBJECT_REF → `|type`/`|id`/`|id_scheme`/`|namespace` attrs on `out`.
///
/// NOTE: master05 §OBJECT_REF's table row is `|scheme` (→ `id.scheme`), but
/// every spec example carries `|id_scheme` (e.g. INSTRUCTION `_guideline_id`,
/// master05 §INSTRUCTION). The example blocks are the wire authority, so this
/// emits `|id_scheme`; [`build_object_ref`] accepts either.
pub(super) fn emit_object_ref<'a, I: Instance<'a>>(oref: I, out: &mut SimNode) {
    let mut put = |suffix: &str, v: Option<I>| {
        if let Some(v) = v.and_then(Instance::to_value) {
            out.attrs.insert(suffix.to_owned(), v);
        }
    };
    put("type", oref.attr("type"));
    put("id", at(oref, &["id", "value"]));
    put("id_scheme", at(oref, &["id", "scheme"]));
    put("namespace", oref.attr("namespace"));
}

/// An OBJECT_REF from `|id` (+ `|type`/`|id_scheme`(or `|scheme`)/`|namespace`).
/// `None` when no `|id` is present. master05 §OBJECT_REF: `type`, `id.value`,
/// `id.scheme`, `namespace` are all required, so absent parts default.
pub(super) fn build_object_ref(node: &SimNode) -> Result<Option<Obj>, FlatError> {
    let Some(id) = node.attrs.get("id").and_then(Value::as_str) else {
        return Ok(None);
    };
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
    typed::make(
        "OBJECT_REF",
        [
            ("namespace", s(ns)),
            ("type", s(ty)),
            (
                "id",
                o(typed::make(
                    "GENERIC_ID",
                    [("value", s(id)), ("scheme", s(scheme))],
                )?),
            ),
        ],
    )
    .map(Some)
}

// ── PARTY_PROXY family (master05 §§PARTY_SELF, PARTY_IDENTIFIED, PARTY_RELATED) ─

/// Emit `p` as the master05 suffixes, or as `|raw` when its reference type is
/// not the `default_party_type` the builder fills in ([`ref_type_survives`]).
pub(super) fn emit_party_as<'a, I: Instance<'a>>(
    p: I,
    default_party_type: &str,
    out: &mut SimNode,
) {
    if ref_type_survives(p, default_party_type) {
        emit_party(p, out);
    } else if let Some(raw) = p.to_raw() {
        out.raw = Some(raw);
    }
}

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
/// [`super::emit::emit_rm_attrs`] deliberately carries no PARTY arm, so
/// the family is emitted exactly once however a party node is reached — a
/// PARTY_PROXY datum leaf through [`super::emit::emit_leaf`], or a
/// `_provider` / `_health_care_facility` / FEEDER_AUDIT_DETAILS party through
/// [`super::structures`].
pub(super) fn emit_party<'a, I: Instance<'a>>(p: I, out: &mut SimNode) {
    if p.rm_type() == Some("PARTY_SELF") {
        out.attrs.insert("_type".to_owned(), json!("PARTY_SELF"));
    }
    let mut put = |suffix: &str, v: Option<I>| {
        if let Some(v) = v.and_then(Instance::to_value) {
            out.attrs.insert(suffix.to_owned(), v);
        }
    };
    put("name", p.attr("name"));
    put("id", at(p, &["external_ref", "id", "value"]));
    put("id_scheme", at(p, &["external_ref", "id", "scheme"]));
    put("id_namespace", at(p, &["external_ref", "namespace"]));
    if let Some(ids) = p.members("identifiers") {
        for (i, id) in ids.into_iter().enumerate() {
            emit_identifier(
                id,
                out.occurrence_mut(
                    IDENTIFIER_SEGMENT,
                    Some(u32::try_from(i).unwrap_or(u32::MAX)),
                ),
            );
        }
    }
    if let Some(rel) = p.attr("relationship") {
        emit::emit_leaf(
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
pub(super) fn build_party(node: &SimNode, default_party_type: &str) -> Result<Obj, FlatError> {
    let explicit_self = node
        .attrs
        .get("_type")
        .and_then(Value::as_str)
        .is_some_and(|t| t == "PARTY_SELF");
    let relationship = relationship_child(node);
    let identifiers = build_party_identifiers(node)?;

    if explicit_self {
        let mut p = access::construct("PARTY_SELF")?;
        if let Some(ext) = external_ref(node, default_party_type)? {
            p.put("external_ref", o(ext))?;
        }
        return Ok(p);
    }

    let ty = if relationship.is_some() {
        "PARTY_RELATED"
    } else {
        "PARTY_IDENTIFIED"
    };
    let mut p = access::construct(ty)?;
    if let Some(name) = node.attrs.get("name") {
        p.put_datum("name", name)?;
    }
    if let Some(ext) = external_ref(node, default_party_type)? {
        p.put("external_ref", o(ext))?;
    }
    for identifier in identifiers {
        p.push_obj("identifiers", identifier)?;
    }
    if let Some(r) = relationship
        && let Some(rel) = data_values::build_leaf(r, "DV_CODED_TEXT", None)?
    {
        p.put("relationship", o(rel))?;
    }
    Ok(p)
}

/// Whether the master05 suffixes carry `party` whole when the builder fills
/// `default_party_type` in as its `external_ref.type`.
///
/// The §§PARTY_IDENTIFIED and PARTY_PROXY tables of master05 define `|id`,
/// `|id_scheme`, `|id_namespace` and `|name` and no row for the reference
/// type, so a party whose `external_ref.type` is another type cannot survive
/// the suffixes; its emitter writes the party as `|raw` instead (master04
/// §Raw canonical JSON).
pub(crate) fn ref_type_survives<'a, I: Instance<'a>>(party: I, default_party_type: &str) -> bool {
    at(party, &["external_ref", "type"])
        .and_then(Instance::as_str)
        .is_none_or(|ty| ty == default_party_type)
}

/// Whether the inlined performer of `participation` survives the master05
/// suffixes, whose builder fills `PERSON` in as its reference type.
pub(crate) fn performer_survives<'a, I: Instance<'a>>(participation: I) -> bool {
    participation
        .attr("performer")
        .is_none_or(|performer| ref_type_survives(performer, PERFORMER_REF_TYPE))
}

/// The `PARTY_REF.type` the performer builder fills in: a participation's
/// performer is a person (master05 §PARTICIPATION defines no reference-type row).
const PERFORMER_REF_TYPE: &str = "PERSON";

/// The `PARTY_REF` external reference for a party sim node, when it carries an
/// `|id` (master05 §PARTY_IDENTIFIED: `|id_namespace` is required when `|id` is
/// set). A `|id_scheme` makes the id a `GENERIC_ID`; without one it is a
/// `HIER_OBJECT_ID` (so a scheme is never fabricated).
fn external_ref(node: &SimNode, default_party_type: &str) -> Result<Option<Obj>, FlatError> {
    let Some(id) = node.attrs.get("id").and_then(Value::as_str) else {
        return Ok(None);
    };
    let namespace = node
        .attrs
        .get("id_namespace")
        .and_then(Value::as_str)
        .unwrap_or("EHR");
    let id_obj = match node.attrs.get("id_scheme").and_then(Value::as_str) {
        Some(scheme) => typed::make("GENERIC_ID", [("value", s(id)), ("scheme", s(scheme))])?,
        None => typed::hier_object_id(id, "|id")?,
    };
    typed::make(
        "PARTY_REF",
        [
            ("id", o(id_obj)),
            ("namespace", s(namespace)),
            ("type", s(default_party_type)),
        ],
    )
    .map(Some)
}

/// The `_identifier:i` DV_IDENTIFIER array on a PARTY node (master05
/// §PARTY_IDENTIFIED `/_identifier:i`).
fn build_party_identifiers(node: &SimNode) -> Result<Vec<Obj>, FlatError> {
    node.children
        .get(IDENTIFIER_SEGMENT)
        .map_or_else(Vec::new, |c| {
            c.occurrences
                .iter()
                .filter(|o| !o.is_empty())
                .map(build_identifier)
                .collect()
        })
        .into_iter()
        .collect()
}

// ── PARTICIPATION (master05 §PARTICIPATION) ───────────────────────────────────

/// PARTICIPATION → `|function`/`|mode`, the inlined performer
/// (`|name`/`|id`/`|id_scheme`/`|id_namespace`, `|identifiers_*:i`), and a
/// PARTY_RELATED `/relationship` child.
pub(super) fn emit_participation<'a, I: Instance<'a>>(p: I, out: &mut SimNode) {
    if !performer_survives(p) {
        out.raw = p.to_raw();
        return;
    }
    if let Some(f) = at(p, &["function", "value"]).and_then(Instance::to_value) {
        out.attrs.insert("function".to_owned(), f);
    }
    if let Some(m) = at(p, &["mode", "value"]).and_then(Instance::to_value) {
        out.attrs.insert("mode".to_owned(), m);
    }
    if let Some(performer) = p.attr("performer") {
        emit_performer(performer, out);
    }
}

/// The inlined performer of a PARTICIPATION: its `|name`/`|id`/`|id_scheme`/
/// `|id_namespace` suffixes, the `|identifiers_<field>:i` list, and a
/// PARTY_RELATED `/relationship` child.
///
/// Performer identifiers are inlined as `|identifiers_<field>:i` suffixes
/// (master05 §PARTICIPATION), not the `_identifier:i` sub-path form.
fn emit_performer<'a, I: Instance<'a>>(performer: I, out: &mut SimNode) {
    if let Some(name) = performer.attr("name").and_then(Instance::to_value) {
        out.attrs.insert("name".to_owned(), name);
    }
    for (steps, suffix) in [
        (&["external_ref", "id", "value"][..], "id"),
        (&["external_ref", "id", "scheme"][..], "id_scheme"),
        (&["external_ref", "namespace"][..], "id_namespace"),
    ] {
        if let Some(v) = at(performer, steps).and_then(Instance::to_value) {
            out.attrs.insert(suffix.to_owned(), v);
        }
    }
    for (i, id) in performer
        .members("identifiers")
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
            if let Some(v) = id.attr(field).and_then(Instance::to_value) {
                out.attrs.insert(format!("{suffix}:{i}"), v);
            }
        }
    }
    if let Some(rel) = performer.attr("relationship") {
        emit::emit_leaf(
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
pub(super) fn build_participation(node: &SimNode) -> Result<Obj, FlatError> {
    let function = node
        .attrs
        .get("function")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut p = typed::make(
        "PARTICIPATION",
        [
            ("function", o(typed::dv_text(function)?)),
            ("performer", o(build_performer(node)?)),
        ],
    )?;
    if let Some(mode) = node.attrs.get("mode").and_then(Value::as_str) {
        p.put(
            "mode",
            o(super::coded_from_group("participation_mode", mode)?),
        )?;
    }
    Ok(p)
}

/// The inlined performer PARTY of a PARTICIPATION (`PARTY_IDENTIFIED`, or
/// `PARTY_RELATED` when a `/relationship` child is present), including the
/// `|identifiers_*:i` performer identifiers.
fn build_performer(node: &SimNode) -> Result<Obj, FlatError> {
    let relationship = single(node, "relationship");
    let identifiers = build_inline_identifiers(node)?;
    let ty = if relationship.is_some() {
        "PARTY_RELATED"
    } else {
        "PARTY_IDENTIFIED"
    };
    let mut p = access::construct(ty)?;
    if let Some(name) = node.attrs.get("name") {
        p.put_datum("name", name)?;
    }
    if let Some(ext) = external_ref(node, PERFORMER_REF_TYPE)? {
        p.put("external_ref", o(ext))?;
    }
    for identifier in identifiers {
        p.push_obj("identifiers", identifier)?;
    }
    if let Some(r) = relationship
        && let Some(rel) = data_values::build_leaf(r, "DV_CODED_TEXT", None)?
    {
        p.put("relationship", o(rel))?;
    }
    Ok(p)
}

/// Rebuild the performer `identifiers` from the inlined `|identifiers_<field>:i`
/// attrs, ordered by `i` (master05 §PARTICIPATION).
fn build_inline_identifiers(node: &SimNode) -> Result<Vec<Obj>, FlatError> {
    let mut by_index: BTreeMap<u32, Vec<(&str, &Value)>> = BTreeMap::new();
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
        by_index.entry(idx).or_default().push((field, value));
    }
    by_index
        .into_values()
        .map(|fields| identifier_from_fields(&fields))
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

    fn canon(object: &Obj) -> Value {
        typed::canonical_of(&**object).unwrap()
    }

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
        let dv = canon(&build_identifier(&bare).unwrap());
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

        let built = canon(&build_party(&out, "PERSON").unwrap());
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
        let built = canon(&build_party(&node, "PERSON").unwrap());
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
        let p = canon(&build_participation(&node).unwrap());
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
        let o = canon(&build_object_ref(&node).unwrap().unwrap());
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
        let example =
            canon(&build_party(&party_with_relationship("relationship"), "PERSON").unwrap());
        let table = canon(
            &build_party(
                &party_with_relationship(RELATIONSHIP_TABLE_SEGMENT),
                "PERSON",
            )
            .unwrap(),
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
        let rebuilt = canon(&build_party(&out, "PERSON").unwrap());
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
        emit::emit_rm_attrs(&rm, "PARTY_IDENTIFIED", &mut from_rm_attrs);
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

        let built = canon(&build_party(&party, "PERSON").unwrap());
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
        let built = canon(&build_party(&with_id, "PERSON").unwrap());
        assert_eq!(built["identifiers"][0]["id"], json!("122"));
    }
}
