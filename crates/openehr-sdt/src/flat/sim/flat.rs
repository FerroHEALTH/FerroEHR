// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FLAT wire codec: `path[:i][|suffix]` key/value map ↔ [`SimNode`].
//!
//! Syntax per ITS-REST `simplified_formats/master04-basic_concepts.adoc`
//! §Flat format: single-level JSON object, fully-qualified keys, zero-based
//! instance indices, pipe suffixes, `ctx/` context keys, `_`-prefixed RM
//! attributes. The nesting/unnesting logic is the algorithm pair in
//! `master04 §Conversion Between Formats`, expressed against the shared
//! [`SimNode`] tree instead of a second ad-hoc structure.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::fmt::Write;

use serde_json::{Map, Value};

use crate::flat::error::FlatError;
use crate::flat::path::FlatKey;
use crate::flat::sim::{SimDocument, SimNode, is_present};

/// Parse a FLAT document into the simplified tree.
///
/// Absent-valued entries (`null`, `""`, `[]`, `{}`) are skipped; index gaps
/// are compacted (see [`SimNode::prune_empty`]).
///
/// # Errors
/// [`FlatError::MalformedPath`] for a syntactically invalid key.
pub fn parse_flat(map: &Map<String, Value>) -> Result<SimDocument, FlatError> {
    let mut root = SimNode::default();
    for (key, value) in map {
        if !is_present(value) && !is_empty_unit_datum(key, value) {
            continue;
        }
        let parsed = FlatKey::parse(key)?;
        let mut node = &mut root;
        for seg in &parsed.segments {
            node = node.occurrence_mut(&seg.name, seg.index);
        }
        node.attrs
            .insert(print_suffix_chain(&parsed), value.clone());
    }
    root.prune_empty();
    Ok(root)
}

/// Serialize the simplified tree as a FLAT document.
///
/// Children emit in tree order, a node's datum parts before its children.
/// A child marked `indexed` (repeating) always prints its `:i` index —
/// including a sole `:0` — matching `master04 §Instance Indexing`; an
/// unindexed child prints none.
#[must_use]
pub fn emit_flat(doc: &SimDocument) -> Map<String, Value> {
    let mut out = Map::new();
    for (name, child) in &doc.children {
        let force_index = child.indexed || child.occurrences.len() > 1;
        for (i, occ) in child.occurrences.iter().enumerate() {
            if occ.is_empty() {
                continue; // an interior index hole — preserved, not renumbered
            }
            let prefix = if force_index {
                format!("{name}:{i}")
            } else {
                name.clone()
            };
            emit_node(occ, &prefix, &mut out);
        }
    }
    out
}

fn emit_node(node: &SimNode, prefix: &str, out: &mut Map<String, Value>) {
    for (chain, value) in &node.attrs {
        let key = if chain.is_empty() {
            prefix.to_owned()
        } else {
            format!("{prefix}|{chain}")
        };
        out.insert(key, value.clone());
    }
    for (name, child) in &node.children {
        let force_index = child.indexed || child.occurrences.len() > 1;
        for (i, occ) in child.occurrences.iter().enumerate() {
            if occ.is_empty() {
                continue; // an interior index hole — preserved, not renumbered
            }
            let child_prefix = if force_index {
                format!("{prefix}/{name}:{i}")
            } else {
                format!("{prefix}/{name}")
            };
            emit_node(occ, &child_prefix, out);
        }
    }
}

/// Whether `value` is an explicitly-present **empty** `DV_QUANTITY` `|unit`
/// datum, which — unlike other empty datum values — must survive the parse.
///
/// `DV_QUANTITY.units` is a mandatory RM attribute (RM data_types
/// `master06-quantity_package.adoc` §DV_QUANTITY — "a real number magnitude,
/// precision, units and accuracy"), and an empty units string is the legitimate
/// value of a *dimensionless* quantity. The general absent-value rule
/// (`master04 §Structured format` rule 6 — empty datum values are omitted) would
/// otherwise drop the empty `|unit` (master05 §DV_QUANTITY: `units` → the
/// `|unit` suffix), leaving the rebuilt `DV_QUANTITY` with no `units` field and
/// violating the RM invariant. Only an empty `|unit` is rescued; every other
/// empty datum still drops.
fn is_empty_unit_datum(key: &str, value: &Value) -> bool {
    matches!(value, Value::String(s) if s.is_empty())
        && FlatKey::parse(key).is_ok_and(|k| k.suffixes.last().is_some_and(|s| s.name == "unit"))
}

/// The printed suffix chain of a parsed key (`""` when the key has none):
/// parts joined by `|`, each keeping its `:i` index.
fn print_suffix_chain(key: &FlatKey) -> String {
    let mut chain = String::new();
    for (i, suffix) in key.suffixes.iter().enumerate() {
        if i > 0 {
            chain.push('|');
        }
        chain.push_str(&suffix.name);
        if let Some(idx) = suffix.index {
            let _ = write!(chain, ":{idx}");
        }
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    #[test]
    fn round_trips_the_master04_flat_example() {
        // master04 §Flat format example (abridged to one observation).
        let input = doc(&[
            ("ctx/language", json!("en")),
            ("ctx/territory", json!("US")),
            ("ctx/composer_name", json!("Dr. Smith")),
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
        ]);
        let tree = parse_flat(&input).unwrap();
        assert_eq!(emit_flat(&tree), input);
    }

    #[test]
    fn keeps_sole_index_on_repeating_and_none_on_single() {
        let input = doc(&[
            ("root/repeating:0/leaf", json!("a")),
            ("root/single/leaf", json!("b")),
        ]);
        let out = emit_flat(&parse_flat(&input).unwrap());
        assert_eq!(out, input);
    }

    #[test]
    fn preserves_index_gaps() {
        // master06 §Participation correlates key families by index — a hole
        // must never renumber later occurrences.
        let input = doc(&[("root/e:0/v", json!("a")), ("root/e:2/v", json!("b"))]);
        let out = emit_flat(&parse_flat(&input).unwrap());
        assert_eq!(out, input);
    }

    #[test]
    fn skips_absent_values_and_keeps_suffix_chains() {
        let input = doc(&[
            ("root/o:0/_link:0|meaning|code", json!("related_to")),
            ("root/o:0/skipped", json!(null)),
            ("root/o:0/empty", json!("")),
            ("ctx/participation_identifiers:1|issuer:0", json!("issuer3")),
        ]);
        let out = emit_flat(&parse_flat(&input).unwrap());
        assert_eq!(
            out,
            doc(&[
                ("root/o:0/_link:0|meaning|code", json!("related_to")),
                ("ctx/participation_identifiers:1|issuer:0", json!("issuer3")),
            ])
        );
    }
}
