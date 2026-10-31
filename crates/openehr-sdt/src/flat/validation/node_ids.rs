// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype roots an instance records by the complete identifier an OPT2
//! inlined (BASE `artefact_identification` `master07-referencing.adoc`
//! §Reconstitutability), read under the interface id their Web Template node
//! carries ([`WebTemplate::archetype_refs`]). The archetype-conformance walk
//! and the FLAT and STRUCTURED read ([`crate::flat::flatten`]) both match an
//! instance to the template through [`as_template_node_ids`].

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON is the instance the archetype-conformance walk reads (#1694)"
)]

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde_json::Value;

use super::Validator;
use crate::flat::webtemplate::model::WebTemplate;

/// Walk `composition` against `wt` with the configured validator.
///
/// An archetype root the instance records by the full reference the template
/// inlined is read under the interface id its template node carries
/// ([`WebTemplate::archetype_refs`]), so the walk matches it to its node.
pub(super) fn run(mut v: Validator, composition: &Value, wt: &WebTemplate) -> Validator {
    v.slot_fills_enumerated = wt.archetype_refs.is_some();
    v.walk(&as_template_node_ids(composition, wt), &wt.tree);
    v
}

/// `composition` with every archetype root it records by a full reference in
/// [`WebTemplate::archetype_refs`] read under the interface id of its template
/// node; borrowed unchanged when it records none.
pub(crate) fn as_template_node_ids<'a>(composition: &'a Value, wt: &WebTemplate) -> Cow<'a, Value> {
    match &wt.archetype_refs {
        Some(refs) if records_any_ref(composition, refs) => {
            let mut instance = composition.clone();
            to_template_node_ids(&mut instance, refs);
            Cow::Owned(instance)
        }
        _ => Cow::Borrowed(composition),
    }
}

/// Whether any LOCATABLE in `value` records one of `refs` as its
/// `archetype_node_id`.
fn records_any_ref(value: &Value, refs: &BTreeMap<String, String>) -> bool {
    match value {
        Value::Object(o) => {
            o.get("archetype_node_id")
                .and_then(Value::as_str)
                .is_some_and(|id| refs.contains_key(id))
                || o.values().any(|v| records_any_ref(v, refs))
        }
        Value::Array(items) => items.iter().any(|v| records_any_ref(v, refs)),
        _ => false,
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
