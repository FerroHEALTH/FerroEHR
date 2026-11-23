// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The internal simplified-instance model: one tree of which FLAT and
//! STRUCTURED are views (`simplified_formats/master04-basic_concepts.adoc`
//! §Format variants, §Conversion Between Formats), each a codec ([`flat`],
//! [`structured`]).
//!
//! A node holds its `|suffix` datum parts (`""` for the bare value), its
//! children with their `:i` indexing, and an optional `|raw` datum.
//!
//! NOTE: no openEHR spec governs a bare STRUCTURED value beside suffixed parts:
//! the `""` key is our own design.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

pub mod flat;
pub mod structured;

use indexmap::IndexMap;
use openehr_rm::v1_2::access::RmObject;
use serde_json::Value;
use serde_json::value::RawValue;

/// The occurrences of one child segment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimChild {
    /// Whether FLAT keys for this child carry `:i` indices (`master04 §Instance
    /// Indexing`).
    pub indexed: bool,
    /// The occurrences, in instance order.
    pub occurrences: Vec<SimNode>,
}

impl SimChild {
    /// The occurrence at position `i`, created (with any intermediate
    /// placeholder occurrences) if absent.
    #[expect(
        clippy::as_conversions,
        clippy::indexing_slicing,
        reason = "the loop immediately below grows `occurrences` until `len() > i`, so the index is in bounds by construction (the u32 → usize widening is lossless on every supported target); the fn returns `&mut SimNode`, not an Option"
    )]
    fn place(&mut self, i: u32) -> &mut SimNode {
        let i = i as usize;
        while self.occurrences.len() <= i {
            self.occurrences.push(SimNode::default());
        }
        &mut self.occurrences[i]
    }
}

/// The `|raw` datum of a node (`master04 §Raw canonical JSON`): an RM object
/// carried whole, in the form the side that made it holds.
#[derive(Debug)]
pub enum RawDatum {
    /// A typed object, flattened from a typed COMPOSITION.
    Rm(Box<dyn RmObject>),
    /// The canonical JSON text a FLAT document carries, read where the build
    /// meets it.
    Text(Box<RawValue>),
    /// The canonical-JSON value a STRUCTURED document, a parsed FLAT map or a
    /// canonical-JSON flatten carries.
    Json(Value),
}

impl RawDatum {
    /// The canonical-JSON value of the datum, for a wire form that is a
    /// value; `None` for an object no `|raw` output carries, or text that
    /// does not parse.
    #[must_use]
    pub fn to_value(&self) -> Option<Value> {
        match self {
            Self::Rm(object) => crate::flat::typed::canonical_of(&**object).ok(),
            // NOTE: the text came from a parsed document, so it is JSON; a failure reads as absent.
            Self::Text(text) => serde_json::from_str(text.get()).ok(),
            Self::Json(value) => Some(value.clone()),
        }
    }
}

impl Clone for RawDatum {
    fn clone(&self) -> Self {
        match self {
            Self::Rm(object) => Self::Rm(object.clone_box()),
            Self::Text(text) => Self::Text(text.clone()),
            Self::Json(value) => Self::Json(value.clone()),
        }
    }
}

impl PartialEq for RawDatum {
    /// Two datums are equal when they carry the same canonical JSON, in
    /// whatever form each holds it.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.get() == b.get(),
            (Self::Json(a), Self::Json(b)) => a == b,
            _ => self.to_value() == other.to_value(),
        }
    }
}

/// One node of a simplified data instance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimNode {
    /// Datum parts keyed by printed suffix chain; `""` is the bare value.
    pub attrs: IndexMap<String, Value>,
    /// The `|raw` datum, the RM object the node carries whole.
    pub raw: Option<RawDatum>,
    /// Child segments in first-seen (or template) order.
    pub children: IndexMap<String, SimChild>,
}

impl SimNode {
    /// Whether this node carries nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty() && self.raw.is_none() && self.children.is_empty()
    }

    /// Whether this node is a bare-only leaf, serialized as a plain scalar in
    /// STRUCTURED (`master04 §Structured format`).
    #[must_use]
    pub fn is_bare_leaf(&self) -> bool {
        self.children.is_empty()
            && self.raw.is_none()
            && self.attrs.len() == 1
            && self.attrs.contains_key("")
    }

    /// Sets the datum part `chain` to `value`: the `|raw` datum when the
    /// chain is `raw`, a part of [`Self::attrs`] otherwise.
    pub fn put_datum(&mut self, chain: String, value: Value) {
        if chain == "raw" {
            self.raw = Some(RawDatum::Json(value));
        } else {
            self.attrs.insert(chain, value);
        }
    }

    /// The bare (suffix-less) datum, if any.
    #[must_use]
    pub fn bare(&self) -> Option<&Value> {
        self.attrs.get("")
    }

    /// The child occurrence at `name`/`index`, created if absent; an explicit index
    /// marks the child indexed ([`Self::place_mut`] does not).
    pub fn occurrence_mut(&mut self, name: &str, index: Option<u32>) -> &mut SimNode {
        let child = self.child_mut(name);
        if index.is_some() {
            child.indexed = true;
        }
        child.place(index.unwrap_or(0))
    }

    /// The child `name`, created empty if absent; the name is copied only on
    /// creation.
    #[expect(
        clippy::indexing_slicing,
        reason = "`i` is the index `get_index_of` or `insert_full` just returned for this map"
    )]
    fn child_mut(&mut self, name: &str) -> &mut SimChild {
        let i = match self.children.get_index_of(name) {
            Some(i) => i,
            None => {
                self.children
                    .insert_full(name.to_owned(), SimChild::default())
                    .0
            }
        };
        &mut self.children[i]
    }

    /// The child occurrence at position `i`, created (with any intermediate
    /// placeholder occurrences) if absent, without touching the child's
    /// `indexed` marking.
    pub fn place_mut(&mut self, name: &str, i: u32) -> &mut SimNode {
        self.child_mut(name).place(i)
    }

    /// The single occurrence of child `name`, if present.
    #[must_use]
    pub fn child(&self, name: &str) -> Option<&SimNode> {
        self.children.get(name).and_then(|c| c.occurrences.first())
    }

    /// Trim trailing empty occurrences and drop fully-empty children.
    ///
    /// NOTE: no openEHR spec addresses index gaps: our own design keeps interior
    /// holes, so index-correlated keys (`master06 §Participation`) keep their numbers.
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

/// Whether a JSON value counts as absent on the simplified wire (`master04
/// §Structured format` rule 6).
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
