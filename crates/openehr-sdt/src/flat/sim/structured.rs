// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The STRUCTURED wire codec: nested JSON ↔ [`SimNode`].
//!
//! Shape per ITS-REST `simplified_formats/master04-basic_concepts.adoc`
//! §Structured format: hierarchy as nested objects; **arrays for data
//! values always**, even at `0..1`/`1..1` cardinality (rule 5); attribute
//! suffixes as `"|suffix"` properties (rule 3); context grouped under a
//! `ctx` object (rule 4); empty objects omitted (rule 6). The top-level
//! data root maps to a single object (the worked example nests
//! `"vital_signs": { … }`, not an array); everything below it is
//! array-wrapped.
//!
//! NOTE: the spec's STRUCTURED chapter shows no `ctx` entry with
//! instance indices or suffixed parts (participations, `work_flow_id`) —
//! no openEHR spec governs that nesting. Convention here: a `ctx` child is
//! a scalar (bare single), an object of `"|suffix"` properties (suffixed
//! single), or an array of those forms (indexed); this is lossless against
//! the FLAT `ctx/` forms of `master06-context_information.adoc`.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use serde_json::{Map, Value};

use crate::flat::error::FlatError;
use crate::flat::path::FlatKey;
use crate::flat::sim::{SimDocument, SimNode, is_present};

/// The `ctx` grouping property (`master04 §Structured format` rule 4).
const CTX: &str = "ctx";

/// Parse a STRUCTURED document into the simplified tree.
///
/// Lenient on input where the spec allows equivalent forms: a property name
/// may carry an explicit `:i` index (`master04 §Structured format` rule 2)
/// which folds into array position; a single occurrence may appear without
/// its array wrapper. Absent values are skipped.
///
/// # Errors
/// [`FlatError::Conversion`] when the document is not a JSON object;
/// [`FlatError::MalformedPath`] on an invalid property name.
pub fn parse_structured(doc: &Value) -> Result<SimDocument, FlatError> {
    let Value::Object(map) = doc else {
        return Err(FlatError::Conversion(
            "a STRUCTURED document must be a JSON object".to_owned(),
        ));
    };
    let mut root = SimNode::default();
    for (key, value) in map {
        if !is_present(value) {
            continue;
        }
        if key == CTX {
            parse_ctx(value, root.occurrence_mut(CTX, None))?;
        } else {
            parse_child_property(&mut root, key, value)?;
        }
    }
    root.prune_empty();
    Ok(root)
}

/// Serialize the simplified tree as a STRUCTURED document.
#[must_use]
pub fn emit_structured(doc: &SimDocument) -> Value {
    let mut out = Map::new();
    for (name, child) in &doc.children {
        if name == CTX {
            if let Some(ctx) = child.occurrences.first() {
                out.insert(CTX.to_owned(), emit_ctx(ctx));
            }
        } else if child.indexed || child.occurrences.len() > 1 {
            // Spec-silent corner: a repeating data root. Kept as an array so
            // nothing is dropped.
            out.insert(
                name.clone(),
                Value::Array(child.occurrences.iter().map(emit_node).collect()),
            );
        } else if let Some(occ) = child.occurrences.first() {
            // The data root maps to a single object (master04 §Structured
            // format worked example).
            out.insert(name.clone(), emit_node(occ));
        }
    }
    Value::Object(out)
}

// ── data subtree ───────────────────────────────────────────────────────────

/// One child property inside a data node: `name`, `name:i`, `|suffix`
/// (possibly chained/indexed), or the `""` bare-value property.
fn parse_child_property(node: &mut SimNode, key: &str, value: &Value) -> Result<(), FlatError> {
    if key.is_empty() {
        node.attrs.insert(String::new(), value.clone());
        return Ok(());
    }
    if let Some(chain) = key.strip_prefix('|') {
        // Validate the suffix-chain syntax by parsing it as a key tail.
        FlatKey::parse(&format!("x|{chain}"))?;
        node.attrs.insert(chain.to_owned(), value.clone());
        return Ok(());
    }
    // A segment name, optionally `:i`-indexed (rule 2 allows indices to
    // remain in property names).
    let parsed = FlatKey::parse(key)?;
    let [segment] = parsed.segments.as_slice() else {
        return Err(FlatError::MalformedPath {
            path: key.to_owned(),
            reason: "a STRUCTURED property must be a single segment".to_owned(),
        });
    };
    if !parsed.suffixes.is_empty() {
        return Err(FlatError::MalformedPath {
            path: key.to_owned(),
            reason: "attribute suffixes inside a STRUCTURED node use \"|suffix\" properties"
                .to_owned(),
        });
    }
    match value {
        Value::Array(items) => {
            let base = segment.index.unwrap_or(0);
            if items.len() > 1 || segment.index.is_some() {
                node.children
                    .entry(segment.name.clone())
                    .or_default()
                    .indexed = true;
            }
            for (i, item) in items.iter().enumerate() {
                if !is_present(item) {
                    continue;
                }
                let occ = node.place_mut(
                    &segment.name,
                    base.saturating_add(u32::try_from(i).unwrap_or(u32::MAX)),
                );
                parse_occurrence(occ, item)?;
            }
        }
        // Lenient: a single occurrence without its array wrapper.
        other => {
            let occ = node.occurrence_mut(&segment.name, segment.index);
            parse_occurrence(occ, other)?;
        }
    }
    Ok(())
}

/// One occurrence value: a scalar (bare-only leaf) or a nested object.
fn parse_occurrence(occ: &mut SimNode, value: &Value) -> Result<(), FlatError> {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if !is_present(val) {
                    continue;
                }
                parse_child_property(occ, key, val)?;
            }
            Ok(())
        }
        scalar => {
            occ.attrs.insert(String::new(), scalar.clone());
            Ok(())
        }
    }
}

fn emit_node(node: &SimNode) -> Value {
    if node.is_bare_leaf()
        && let Some(bare) = node.attrs.get("")
    {
        return bare.clone();
    }
    let mut out = Map::new();
    for (chain, value) in &node.attrs {
        let key = if chain.is_empty() {
            String::new()
        } else {
            format!("|{chain}")
        };
        out.insert(key, value.clone());
    }
    for (name, child) in &node.children {
        // Arrays always, even for single occurrences (master04 §Structured
        // format rule 5). Interior index holes stay as empty objects so
        // FLAT round-trips keep their numbering.
        out.insert(
            name.clone(),
            Value::Array(child.occurrences.iter().map(emit_node).collect()),
        );
    }
    Value::Object(out)
}

// ── ctx subtree ────────────────────────────────────────────────────────────

fn parse_ctx(value: &Value, ctx: &mut SimNode) -> Result<(), FlatError> {
    let Value::Object(map) = value else {
        return Err(FlatError::Conversion(
            "the STRUCTURED ctx property must be a JSON object".to_owned(),
        ));
    };
    for (key, val) in map {
        if !is_present(val) {
            continue;
        }
        let parsed = FlatKey::parse(key)?;
        let [segment] = parsed.segments.as_slice() else {
            return Err(FlatError::MalformedPath {
                path: format!("ctx/{key}"),
                reason: "a ctx property must be a single key".to_owned(),
            });
        };
        match val {
            Value::Array(items) => {
                ctx.children
                    .entry(segment.name.clone())
                    .or_default()
                    .indexed = true;
                for (i, item) in items.iter().enumerate() {
                    if !is_present(item) {
                        continue;
                    }
                    let occ = ctx.place_mut(&segment.name, u32::try_from(i).unwrap_or(u32::MAX));
                    parse_ctx_item(occ, item, key)?;
                }
            }
            other => {
                let occ = ctx.occurrence_mut(&segment.name, segment.index);
                parse_ctx_item(occ, other, key)?;
            }
        }
    }
    Ok(())
}

fn parse_ctx_item(occ: &mut SimNode, value: &Value, key: &str) -> Result<(), FlatError> {
    match value {
        Value::Object(map) => {
            for (prop, val) in map {
                if !is_present(val) {
                    continue;
                }
                let Some(chain) = prop.strip_prefix('|') else {
                    return Err(FlatError::MalformedPath {
                        path: format!("ctx/{key}"),
                        reason: format!(
                            "ctx object properties must be \"|suffix\" parts, got {prop:?}"
                        ),
                    });
                };
                occ.attrs.insert(chain.to_owned(), val.clone());
            }
            Ok(())
        }
        scalar => {
            occ.attrs.insert(String::new(), scalar.clone());
            Ok(())
        }
    }
}

fn emit_ctx(ctx: &SimNode) -> Value {
    let mut out = Map::new();
    for (name, child) in &ctx.children {
        if child.indexed || child.occurrences.len() > 1 {
            out.insert(
                name.clone(),
                Value::Array(child.occurrences.iter().map(emit_ctx_item).collect()),
            );
        } else if let Some(occ) = child.occurrences.first() {
            out.insert(name.clone(), emit_ctx_item(occ));
        }
    }
    Value::Object(out)
}

fn emit_ctx_item(occ: &SimNode) -> Value {
    if occ.is_bare_leaf()
        && let Some(bare) = occ.attrs.get("")
    {
        return bare.clone();
    }
    let mut out = Map::new();
    for (chain, value) in &occ.attrs {
        let key = if chain.is_empty() {
            String::new()
        } else {
            format!("|{chain}")
        };
        out.insert(key, value.clone());
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flat::sim::flat::{emit_flat, parse_flat};
    use serde_json::json;

    /// master04 §Structured format worked example (abridged): the same data
    /// as the FLAT example nests to this exact shape.
    #[test]
    fn emits_the_master04_structured_shape() {
        let flat: Map<String, Value> = [
            ("ctx/language", json!("en")),
            ("ctx/territory", json!("US")),
            (
                "vital_signs/body_temperature:0/any_event:0/temperature|magnitude",
                json!(37.5),
            ),
            (
                "vital_signs/body_temperature:0/any_event:0/temperature|unit",
                json!("°C"),
            ),
            (
                "vital_signs/body_temperature:0/any_event:0/time",
                json!("2024-01-15T10:30:00Z"),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let tree = parse_flat(&flat).unwrap();
        let structured = emit_structured(&tree);
        assert_eq!(
            structured,
            json!({
                "ctx": { "language": "en", "territory": "US" },
                "vital_signs": {
                    "body_temperature": [ {
                        "any_event": [ {
                            "temperature": [ { "|magnitude": 37.5, "|unit": "°C" } ],
                            "time": [ "2024-01-15T10:30:00Z" ]
                        } ]
                    } ]
                }
            })
        );
    }

    #[test]
    fn structured_round_trips_to_flat() {
        let structured = json!({
            "ctx": {
                "language": "en",
                "territory": "US",
                "participation_name": ["Dr. Marcus Johnson", "Lara Markham"],
                "work_flow_id": { "|id": "567", "|type": "ORGANISATION" }
            },
            "vital_signs": {
                "body_temperature": [
                    { "any_event": [
                        { "temperature": [ { "|magnitude": 37.5, "|unit": "°C" } ] },
                        { "temperature": [ { "|magnitude": 38.1, "|unit": "°C" } ] }
                    ] }
                ]
            }
        });
        let tree = parse_structured(&structured).unwrap();
        let flat = emit_flat(&tree);
        // A single-item STRUCTURED array carries no `:i` in its property name,
        // so the template-free transform cannot recover a repeating `:0`
        // (master04 §Conversion Between Formats — indices live in property
        // names; cardinality is a template fact). `any_event` (two items)
        // keeps its indices.
        let expect: Map<String, Value> = [
            ("ctx/language", json!("en")),
            ("ctx/territory", json!("US")),
            ("ctx/participation_name:0", json!("Dr. Marcus Johnson")),
            ("ctx/participation_name:1", json!("Lara Markham")),
            ("ctx/work_flow_id|id", json!("567")),
            ("ctx/work_flow_id|type", json!("ORGANISATION")),
            (
                "vital_signs/body_temperature/any_event:0/temperature|magnitude",
                json!(37.5),
            ),
            (
                "vital_signs/body_temperature/any_event:0/temperature|unit",
                json!("°C"),
            ),
            (
                "vital_signs/body_temperature/any_event:1/temperature|magnitude",
                json!(38.1),
            ),
            (
                "vital_signs/body_temperature/any_event:1/temperature|unit",
                json!("°C"),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        assert_eq!(flat, expect);
        // And back: STRUCTURED → tree → STRUCTURED is stable.
        assert_eq!(
            emit_structured(&parse_structured(&structured).unwrap()),
            structured
        );
    }

    #[test]
    fn bare_value_beside_suffixes_uses_the_empty_key() {
        let structured = json!({
            "media_doc": {
                "media_file": [ { "": "http://x/y.png", "|mediatype": "image/png", "|size": 1024 } ]
            }
        });
        let tree = parse_structured(&structured).unwrap();
        let flat = emit_flat(&tree);
        assert_eq!(
            flat.get("media_doc/media_file"),
            Some(&json!("http://x/y.png"))
        );
        assert_eq!(
            flat.get("media_doc/media_file|mediatype"),
            Some(&json!("image/png"))
        );
        assert_eq!(emit_structured(&tree), structured);
    }

    #[test]
    fn rejects_non_object_documents() {
        assert!(matches!(
            parse_structured(&json!([1, 2])),
            Err(FlatError::Conversion(_))
        ));
    }
}
