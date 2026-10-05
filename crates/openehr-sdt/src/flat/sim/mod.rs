// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The internal simplified-instance model.
//!
//! Both wire variants of the Simplified Formats (ITS-REST
//! `simplified_formats/master04-basic_concepts.adoc` §Format variants) are
//! views of one tree: FLAT is the key/value serialization, STRUCTURED the
//! nested one, and `master04 §Conversion Between Formats` defines the exact
//! transform between them. [`SimNode`] is that tree, so each wire format is
//! a pure codec ([`flat`], [`structured`]) and the template-driven RM
//! conversion is written once against this model.
//!
//! Shape:
//!
//! - **Datum parts** (`attrs`): the `|suffix` values of a node, keyed by the
//!   printed suffix chain (`"magnitude"`, `"meaning|code"`, `"issuer:0"`).
//!   The bare, suffix-less value uses the `""` key. NOTE: the wire
//!   convention for a bare value alongside suffixed parts in STRUCTURED (an
//!   `""` property) is not stated by the spec — no openEHR spec governs
//!   that corner; the `""` key matches the ecosystem's established wire.
//! - **Children** (`children`): segment name → [`SimChild`]: the ordered
//!   occurrence list plus whether occurrences are `:i`-indexed on the FLAT
//!   wire (`master04 §Instance Indexing`: indices appear when a node
//!   repeats; single-valued nodes carry none).

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

pub mod flat;
pub mod structured;

use indexmap::IndexMap;
use serde_json::Value;

/// The occurrences of one child segment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimChild {
    /// Whether FLAT keys for this child carry `:i` instance indices
    /// (`master04 §Instance Indexing` — repeating nodes). Parsing sets it
    /// from the input; the RM flattener sets it from the template
    /// multiplicity (`max > 1 || max == -1`).
    pub indexed: bool,
    /// The occurrences, in instance order.
    pub occurrences: Vec<SimNode>,
}

/// One node of a simplified data instance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimNode {
    /// Datum parts keyed by printed suffix chain; `""` is the bare value.
    pub attrs: IndexMap<String, Value>,
    /// Child segments in first-seen (or template) order.
    pub children: IndexMap<String, SimChild>,
}

impl SimNode {
    /// Whether this node carries nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty() && self.children.is_empty()
    }

    /// Whether this node is a bare-only leaf (a single `""` datum, no
    /// suffixed parts, no children) — serialized as a plain scalar in
    /// STRUCTURED (`master04 §Structured format` example: `"time":
    /// ["2024-01-15T10:30:00Z"]`).
    #[must_use]
    pub fn is_bare_leaf(&self) -> bool {
        self.children.is_empty() && self.attrs.len() == 1 && self.attrs.contains_key("")
    }

    /// The bare (suffix-less) datum, if any.
    #[must_use]
    pub fn bare(&self) -> Option<&Value> {
        self.attrs.get("")
    }

    /// The child occurrence at `name`/`index`, created (with any
    /// intermediate placeholder occurrences) if absent. An explicit index
    /// marks the child as indexed on the FLAT wire; use [`Self::place_mut`]
    /// for pure positional placement without that marking.
    pub fn occurrence_mut(&mut self, name: &str, index: Option<u32>) -> &mut SimNode {
        if index.is_some() {
            self.children.entry(name.to_owned()).or_default().indexed = true;
        }
        self.place_mut(name, index.unwrap_or(0))
    }

    /// The child occurrence at position `i`, created (with any intermediate
    /// placeholder occurrences) if absent, without touching the child's
    /// `indexed` marking.
    #[expect(
        clippy::as_conversions,
        clippy::indexing_slicing,
        reason = "the loop immediately below grows `occurrences` until `len() > i`, so the index is in bounds by construction (the u32 → usize widening is lossless on every supported target); the fn returns `&mut SimNode`, not an Option"
    )]
    pub fn place_mut(&mut self, name: &str, i: u32) -> &mut SimNode {
        let child = self.children.entry(name.to_owned()).or_default();
        let i = i as usize;
        while child.occurrences.len() <= i {
            child.occurrences.push(SimNode::default());
        }
        &mut child.occurrences[i]
    }

    /// The single occurrence of child `name`, if present.
    #[must_use]
    pub fn child(&self, name: &str) -> Option<&SimNode> {
        self.children.get(name).and_then(|c| c.occurrences.first())
    }

    /// Trim trailing empty occurrences and drop fully-empty children.
    ///
    /// Interior holes (a `:0`/`:2` input with nothing at `:1`) are KEPT as
    /// empty placeholders and skipped at serialization, so original indices
    /// survive — compacting them would silently renumber index-correlated
    /// key families (`master06 §Participation` correlates
    /// `participation_name:i` / `participation_identifiers:i` by index).
    /// NOTE: no openEHR spec addresses index gaps — preserving the
    /// client's numbering is our own, least-surprising posture.
    pub fn prune_empty(&mut self) {
        for child in self.children.values_mut() {
            for occ in &mut child.occurrences {
                occ.prune_empty();
            }
            while child.occurrences.last().is_some_and(SimNode::is_empty) {
                child.occurrences.pop();
            }
        }
        self.children.retain(|_, c| !c.occurrences.is_empty());
    }
}

/// A complete simplified data instance: the virtual root above the
/// composition's root segment and the `ctx` namespace. `children` holds at
/// most one `ctx` child plus the data root(s).
pub type SimDocument = SimNode;

/// Whether a JSON value counts as absent on the simplified wire.
///
/// Datum values that are `null` or empty are skipped on read and never
/// emitted (`master04 §Structured format` rule 6: empty objects SHOULD be
/// omitted; the same reading is applied to FLAT values).
#[must_use]
pub fn is_present(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Number(_) | Value::Bool(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn occurrence_mut_fills_and_marks_indexed() {
        let mut root = SimNode::default();
        root.occurrence_mut("event", Some(2))
            .attrs
            .insert(String::new(), json!("x"));
        let child = &root.children["event"];
        assert!(child.indexed);
        assert_eq!(child.occurrences.len(), 3);
        assert!(child.occurrences[0].is_empty());
        assert_eq!(child.occurrences[2].bare(), Some(&json!("x")));
    }

    #[test]
    fn prune_empty_keeps_interior_holes_and_trims_trailing() {
        let mut root = SimNode::default();
        root.occurrence_mut("event", Some(0))
            .attrs
            .insert(String::new(), json!("a"));
        root.occurrence_mut("event", Some(2))
            .attrs
            .insert(String::new(), json!("b"));
        // A trailing placeholder from a value that was skipped as absent.
        let _ = root.occurrence_mut("event", Some(4));
        let _ = root.occurrence_mut("dropped", Some(1));
        root.prune_empty();
        let child = &root.children["event"];
        assert_eq!(child.occurrences.len(), 3);
        assert!(child.occurrences[1].is_empty());
        assert_eq!(child.occurrences[2].bare(), Some(&json!("b")));
        assert!(!root.children.contains_key("dropped"));
    }

    #[test]
    fn bare_leaf_detection() {
        let mut n = SimNode::default();
        n.attrs.insert(String::new(), json!("2024-01-15T10:30:00Z"));
        assert!(n.is_bare_leaf());
        n.attrs.insert("magnitude".to_owned(), json!(1));
        assert!(!n.is_bare_leaf());
    }
}
