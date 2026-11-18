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
        // NOTE: a key the allocation-free scan refuses is parsed again for the refusal it names.
        if !crate::flat::path::well_formed(key) {
            FlatKey::parse(key)?;
        }
        let (segment_part, suffix_part) = match key.split_once('|') {
            Some((segments, suffixes)) => (segments, Some(suffixes)),
            None => (key.as_str(), None),
        };
        let mut node = &mut root;
        for raw in segment_part.split('/') {
            let (name, index) = crate::flat::path::split_name_index(raw);
            node = node.occurrence_mut(name, index);
        }
        let chain = match suffix_part {
            None => String::new(),
            // A suffix chain without an index prints back as written.
            Some(suffixes) if !suffixes.contains(':') => suffixes.to_owned(),
            Some(suffixes) => print_raw_suffix_chain(suffixes),
        };
        node.attrs.insert(chain, value.clone());
    }
    root.prune_empty();
    Ok(root)
}

/// The suffix chain of the raw `|`-separated suffix text of a key
/// [`crate::flat::path::well_formed`] accepts, each index printed as a
/// decimal: the key under which the datum sits on its node.
fn print_raw_suffix_chain(suffixes: &str) -> String {
    let mut chain = String::new();
    for (i, raw) in suffixes.split('|').enumerate() {
        if i > 0 {
            chain.push('|');
        }
        let (name, index) = crate::flat::path::split_name_index(raw);
        chain.push_str(name);
        if let Some(idx) = index {
            let _ = write!(chain, ":{idx}");
        }
    }
    chain
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

/// The FLAT JSON text of `doc`, written straight from the tree in the order
/// [`emit_flat`] gives its keys, with no intermediate map (#6003).
///
/// # Errors
/// The serializer's refusal, which a JSON datum never raises.
pub fn write_flat(doc: &SimDocument) -> Result<String, serde_json::Error> {
    serde_json::to_string(&FlatText(doc))
}

/// [`write_flat`]'s serializer view of a tree.
struct FlatText<'a>(&'a SimDocument);

impl serde::Serialize for FlatText<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        for (name, child) in &self.0.children {
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
                write_node(occ, &prefix, &mut map)?;
            }
        }
        map.end()
    }
}

/// [`emit_node`] into a serializer map.
fn write_node<M: serde::ser::SerializeMap>(
    node: &SimNode,
    prefix: &str,
    map: &mut M,
) -> Result<(), M::Error> {
    for (chain, value) in &node.attrs {
        if chain.is_empty() {
            map.serialize_entry(prefix, value)?;
        } else {
            map.serialize_entry(&format!("{prefix}|{chain}"), value)?;
        }
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
            write_node(occ, &child_prefix, map)?;
        }
    }
    Ok(())
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
