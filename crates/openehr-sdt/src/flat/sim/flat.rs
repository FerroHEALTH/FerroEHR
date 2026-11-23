// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FLAT wire codec: `path[:i][|suffix]` key/value map ↔ [`SimNode`]
//! (`simplified_formats/master04-basic_concepts.adoc` §Flat format, §Conversion
//! Between Formats).

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
use crate::flat::sim::{RawDatum, SimDocument, SimNode, is_present};
use serde_json::value::RawValue;

/// Parse a FLAT document into the simplified tree; absent-valued entries are
/// skipped.
///
/// # Errors
/// [`FlatError::MalformedPath`] for a syntactically invalid key.
pub fn parse_flat(map: &Map<String, Value>) -> Result<SimDocument, FlatError> {
    let mut root = SimNode::default();
    for (key, value) in map {
        if !is_present(value) && !is_empty_unit_datum(key, value) {
            continue;
        }
        well_formed(key)?;
        let (segment_part, suffix_part) = match key.split_once('|') {
            Some((segments, suffixes)) => (segments, Some(suffixes)),
            None => (key.as_str(), None),
        };
        node_at(&mut root, segment_part).put_datum(chain_of(suffix_part), value.clone());
    }
    root.prune_empty();
    Ok(root)
}

/// The node the `/`-separated segments of a well-formed key name, created
/// with its path if absent.
fn node_at<'n>(root: &'n mut SimNode, segments: &str) -> &'n mut SimNode {
    let mut node = root;
    for raw in segments.split('/') {
        let (name, index) = crate::flat::path::split_name_index(raw);
        node = node.occurrence_mut(name, index);
    }
    node
}

/// The suffix chain a datum sits under on its node, from the raw suffix text
/// of a well-formed key.
fn chain_of(suffixes: Option<&str>) -> String {
    match suffixes {
        None => String::new(),
        // A suffix chain without an index prints back as written.
        Some(suffixes) if !suffixes.contains(':') => suffixes.to_owned(),
        Some(suffixes) => print_raw_suffix_chain(suffixes),
    }
}

/// Parse FLAT JSON text straight into the simplified tree, key by key as
/// [`parse_flat`]. The inner error is a key's FLAT refusal.
///
/// # Errors
/// The [`serde_json::Error`] of text that is no JSON object.
pub fn parse_flat_text(text: &[u8]) -> Result<Result<SimDocument, FlatError>, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(text);
    let parsed = serde::Deserializer::deserialize_map(&mut deserializer, FlatVisitor)?;
    deserializer.end()?;
    Ok(parsed.map(|mut root| {
        root.prune_empty();
        root
    }))
}

/// The visitor [`parse_flat_text`] reads the document's one object with.
struct FlatVisitor;

impl<'de> serde::de::Visitor<'de> for FlatVisitor {
    type Value = Result<SimDocument, FlatError>;

    // NOTE: the refusal of text that is no object reads as the map reader's did.
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a map")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut root = SimNode::default();
        let mut refusal = None;
        while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
            let (segment_part, suffix_part) = match key.split_once('|') {
                Some((segments, suffixes)) => (segments, Some(suffixes)),
                None => (&*key, None),
            };
            if suffix_part == Some("raw") {
                let raw: &'de RawValue = map.next_value()?;
                if refusal.is_some() || !raw_present(raw.get()) {
                    continue;
                }
                match well_formed(&key) {
                    Ok(()) => {
                        node_at(&mut root, segment_part).raw = Some(RawDatum::Text(raw.to_owned()));
                    }
                    Err(e) => refusal = Some(e),
                }
                continue;
            }
            let value: Value = map.next_value()?;
            if refusal.is_some() {
                continue;
            }
            if !is_present(&value) && !is_empty_unit_datum(&key, &value) {
                continue;
            }
            match well_formed(&key) {
                Ok(()) => node_at(&mut root, segment_part).put_datum(chain_of(suffix_part), value),
                Err(e) => refusal = Some(e),
            }
        }
        Ok(refusal.map_or(Ok(root), Err))
    }
}

/// The refusal of a key that is no well-formed FLAT path, as [`parse_flat`]
/// names it.
fn well_formed(key: &str) -> Result<(), FlatError> {
    // NOTE: a key the allocation-free scan refuses is parsed again for the refusal it names.
    if !crate::flat::path::well_formed(key) {
        FlatKey::parse(key)?;
    }
    Ok(())
}

/// Whether the JSON text of a `|raw` datum is present on the simplified wire
/// ([`is_present`]): no `null`, empty string, empty array or empty object.
fn raw_present(text: &str) -> bool {
    let mut chars = text.chars().filter(|c| !c.is_ascii_whitespace());
    !matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some('n'), Some('u'), Some('l'))
            | (Some('"'), Some('"'), None)
            | (Some('['), Some(']'), None)
            | (Some('{'), Some('}'), None)
    )
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

/// Serialize the simplified tree as a FLAT document; an indexed child always
/// prints its `:i` (`master04 §Instance Indexing`).
#[must_use]
pub fn emit_flat(doc: &SimDocument) -> Map<String, Value> {
    let mut out = Map::new();
    let Ok(()) = each_datum::<std::convert::Infallible>(doc, &mut |key, datum| {
        match datum {
            Datum::Value(value) => {
                out.insert(key, value.clone());
            }
            Datum::Raw(raw) => {
                if let Some(value) = raw.to_value() {
                    out.insert(key, value);
                }
            }
        }
        Ok(())
    });
    out
}

/// The FLAT JSON text of `doc`, in the order [`emit_flat`] gives its keys.
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
        each_datum(self.0, &mut |key, datum| match datum {
            Datum::Value(value) | Datum::Raw(RawDatum::Json(value)) => {
                map.serialize_entry(&key, value)
            }
            Datum::Raw(RawDatum::Rm(object)) => {
                map.serialize_entry(&key, &crate::flat::typed::RawJson(&**object))
            }
            Datum::Raw(RawDatum::Text(text)) => map.serialize_entry(&key, text),
        })?;
        map.end()
    }
}

/// One datum of the FLAT form: a key's value, or a node's `|raw` object.
enum Datum<'a> {
    Value(&'a Value),
    Raw(&'a RawDatum),
}

/// Calls `f` with the FLAT key and datum of every entry of `doc`: children in
/// tree order, a node's `|raw` and datum parts first (`master04 §Instance
/// Indexing`).
fn each_datum<E>(
    doc: &SimDocument,
    f: &mut impl FnMut(String, Datum<'_>) -> Result<(), E>,
) -> Result<(), E> {
    each_child_node(doc, "", &mut |prefix, node| {
        each_node_datum(node, prefix, f)
    })
}

/// Calls `f` with the key prefix and node of every non-empty child
/// occurrence of `node` under `prefix`.
fn each_child_node<'n, E>(
    node: &'n SimNode,
    prefix: &str,
    f: &mut impl FnMut(&str, &'n SimNode) -> Result<(), E>,
) -> Result<(), E> {
    for (name, child) in &node.children {
        let force_index = child.indexed || child.occurrences.len() > 1;
        for (i, occ) in child.occurrences.iter().enumerate() {
            if occ.is_empty() {
                continue; // an interior index hole — preserved, not renumbered
            }
            let child_prefix = match (prefix.is_empty(), force_index) {
                (true, true) => format!("{name}:{i}"),
                (true, false) => name.clone(),
                (false, true) => format!("{prefix}/{name}:{i}"),
                (false, false) => format!("{prefix}/{name}"),
            };
            f(&child_prefix, occ)?;
        }
    }
    Ok(())
}

/// [`each_datum`] over one node at `prefix` and the nodes below it.
fn each_node_datum<E>(
    node: &SimNode,
    prefix: &str,
    f: &mut impl FnMut(String, Datum<'_>) -> Result<(), E>,
) -> Result<(), E> {
    if let Some(raw) = &node.raw {
        f(format!("{prefix}|raw"), Datum::Raw(raw))?;
    }
    for (chain, value) in &node.attrs {
        let key = if chain.is_empty() {
            prefix.to_owned()
        } else {
            format!("{prefix}|{chain}")
        };
        f(key, Datum::Value(value))?;
    }
    each_child_node(node, prefix, &mut |child_prefix, child| {
        each_node_datum(child, child_prefix, f)
    })
}

/// Whether `value` is an explicitly present empty `DV_QUANTITY` `|unit`, which
/// survives the parse: an empty `units` is a dimensionless quantity's value
/// (RM `master06-quantity_package.adoc` §DV_QUANTITY).
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

    /// The text read gives the tree the map read gives, a `|raw` datum kept as
    /// its text (#6003).
    #[test]
    fn the_text_read_gives_the_map_reads_tree() {
        let input = doc(&[
            ("ctx/language", json!("en")),
            ("root/o:0/v|magnitude", json!(37.5)),
            ("root/o:0/v|unit", json!("")),
            ("root/o:0/skipped", json!(null)),
            ("root/o:2/v", json!("b")),
            (
                "root/composer|raw",
                json!({"_type": "PARTY_IDENTIFIED", "name": "Dr. Smith"}),
            ),
            ("root/gone|raw", json!({})),
        ]);
        let text = serde_json::to_vec(&input).unwrap();
        let from_text = parse_flat_text(&text).unwrap().unwrap();
        assert_eq!(from_text, parse_flat(&input).unwrap());
        let composer = from_text
            .child("root")
            .and_then(|r| r.child("composer"))
            .unwrap();
        let Some(RawDatum::Text(raw)) = &composer.raw else {
            panic!("the |raw datum keeps its text: {:?}", composer.raw);
        };
        assert_eq!(
            raw.get(),
            r#"{"_type":"PARTY_IDENTIFIED","name":"Dr. Smith"}"#
        );
        assert!(
            from_text
                .child("root")
                .and_then(|r| r.child("gone"))
                .is_none()
        );
        assert_eq!(
            write_flat(&from_text).unwrap(),
            serde_json::to_string(&emit_flat(&from_text)).unwrap()
        );
    }

    /// A malformed key is the FLAT refusal and text that is no JSON object the
    /// syntax refusal, which wins wherever it stands (#6003).
    #[test]
    fn the_text_read_refuses_as_the_map_read_does() {
        let malformed = br#"{"root//x": 1, "root/y": 2}"#;
        assert!(matches!(
            parse_flat_text(malformed).unwrap(),
            Err(FlatError::MalformedPath { .. })
        ));
        assert!(parse_flat_text(br#"{"root//x": 1, "root/y": }"#).is_err());
        let not_an_object = parse_flat_text(b"[1]").unwrap_err();
        let map_reader = serde_json::from_slice::<Map<String, Value>>(b"[1]").unwrap_err();
        assert_eq!(not_an_object.to_string(), map_reader.to_string());
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
