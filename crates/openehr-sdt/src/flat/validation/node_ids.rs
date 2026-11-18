// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype roots an instance records by the complete identifier an OPT2
//! inlined (BASE `artefact_identification` `master07-referencing.adoc`
//! §Reconstitutability), read under the interface id their Web Template node
//! carries ([`WebTemplate::archetype_refs`]). The archetype-conformance walk
//! and the FLAT and STRUCTURED read ([`crate::flat::flatten`]) both match an
//! instance to the template through [`Mapped`].

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON is the instance the archetype-conformance walk reads (#1694)"
)]

use std::collections::BTreeMap;

use serde_json::Value;

use super::Validator;
use crate::flat::instance::Instance;
use crate::flat::webtemplate::model::WebTemplate;

/// Walk `composition` against `wt` with the configured validator.
///
/// An archetype root the instance records by the full reference the template
/// inlined is read under the interface id its template node carries
/// ([`WebTemplate::archetype_refs`]), so the walk matches it to its node.
pub(super) fn run<'i, I: Instance<'i>>(
    mut v: Validator,
    composition: I,
    wt: &'i WebTemplate,
) -> Validator {
    v.slot_fills_enumerated = wt.archetype_refs.is_some();
    match &wt.archetype_refs {
        Some(refs) => v.walk(Mapped::Node(composition, refs), &wt.tree),
        None => v.walk(composition, &wt.tree),
    }
    v
}

/// An instance read with every `archetype_node_id` in `refs` replaced by the
/// interface id it maps to, with no copy of the instance; a `|raw` rendering
/// carries the mapped ids too.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mapped<'i, I> {
    /// A node or value of the instance.
    Node(I, &'i BTreeMap<String, String>),
    /// A mapped `archetype_node_id`.
    Id(&'i str),
}

impl<'i, I: Instance<'i>> Instance<'i> for Mapped<'i, I> {
    fn rm_type(self) -> Option<&'i str> {
        match self {
            Self::Node(n, _) => n.rm_type(),
            Self::Id(_) => None,
        }
    }
    fn attr(self, name: &str) -> Option<Self> {
        let Self::Node(n, refs) = self else {
            return None;
        };
        if name == "archetype_node_id"
            && let Some(interface) = n.str_at(name).and_then(|id| refs.get(id))
        {
            return Some(Self::Id(interface));
        }
        n.attr(name).map(|v| Self::Node(v, refs))
    }
    fn members(self, name: &str) -> Option<Vec<Self>> {
        match self {
            Self::Node(n, refs) => n
                .members(name)
                .map(|m| m.into_iter().map(|v| Self::Node(v, refs)).collect()),
            Self::Id(_) => None,
        }
    }
    fn is_present(self, name: &str) -> bool {
        matches!(self, Self::Node(n, _) if n.is_present(name))
    }
    fn is_object(self) -> bool {
        matches!(self, Self::Node(n, _) if n.is_object())
    }
    fn as_str(self) -> Option<&'i str> {
        match self {
            Self::Node(n, _) => n.as_str(),
            Self::Id(s) => Some(s),
        }
    }
    fn as_f64(self) -> Option<f64> {
        match self {
            Self::Node(n, _) => n.as_f64(),
            Self::Id(_) => None,
        }
    }
    fn as_i64(self) -> Option<i64> {
        match self {
            Self::Node(n, _) => n.as_i64(),
            Self::Id(_) => None,
        }
    }
    fn as_bool(self) -> Option<bool> {
        match self {
            Self::Node(n, _) => n.as_bool(),
            Self::Id(_) => None,
        }
    }
    fn render(self) -> String {
        match self {
            Self::Node(n, _) => n.render(),
            Self::Id(s) => Value::from(s).to_string(),
        }
    }
    fn keys(self) -> Vec<&'i str> {
        match self {
            Self::Node(n, _) => n.keys(),
            Self::Id(_) => Vec::new(),
        }
    }
    fn to_value(self) -> Option<Value> {
        match self {
            Self::Node(n, refs) => n.to_value().map(|mut v| {
                to_template_node_ids(&mut v, refs);
                v
            }),
            Self::Id(s) => Some(Value::from(s)),
        }
    }
}

/// Rewrite every `archetype_node_id` found in `refs` to the interface id it
/// maps to.
fn to_template_node_ids(value: &mut Value, refs: &BTreeMap<String, String>) {
    match value {
        Value::Object(o) => {
            if let Some(Value::String(id)) = o.get_mut("archetype_node_id")
                && let Some(interface) = refs.get(id.as_str())
            {
                interface.clone_into(id);
            }
            for v in o.values_mut() {
                to_template_node_ids(v, refs);
            }
        }
        Value::Array(items) => {
            for v in items {
                to_template_node_ids(v, refs);
            }
        }
        _ => {}
    }
}
