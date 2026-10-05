// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The promoted-leaf registry.
//!
//! The single source of truth for which canonical leaves are lifted onto a
//! dedicated `node` column, so a hot AQL predicate / sort reads an indexed
//! column instead of re-extracting the value through a correlated subtree scan.
//!
//! No openEHR spec governs storage columns or their derivation — our own design;
//! openEHR defines the language (QUERY master03), not how a leaf is physically
//! materialized.
//!
//! One registry drives both directions, so the write side and the read side can
//! never disagree on the mapping:
//! - **Write** ([`crate::storage::codec`] → [`extract`]): at decomposition time,
//!   for a versioned-object root, the raw leaf text is captured onto the row and
//!   written into the column ([`crate::storage::node_repo::write_nodes`]).
//! - **Read** (`crate::aql::sql::value`): a lowered `LeafPath` whose flattened
//!   attribute path equals a registry entry's `path` (and whose source is the
//!   matching versioned-object root) substitutes `node.<column>` for the
//!   correlated-subquery lowering.
//!
//! ## Invariant on `rm_type`
//!
//! A promoted leaf's value lives on the root node (`num = 0`) of its versioned
//! object and the column is populated only there. The read side identifies the
//! root by `rm_type`, so an entry's `rm_type` must occur only at `num = 0`, that
//! is, a versioned-object root that never nests inside its own tree.
//! `COMPOSITION`, `EHR_STATUS` and `EHR_ACCESS` satisfy this; `FOLDER` does not,
//! so a `FOLDER` leaf would need an explicit `num = 0` guard on the read side.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;

/// The physical type a promoted column carries.
///
/// It selects the write-time SQL conversion ([`crate::storage::node_repo`]) and
/// the read-time coercion the column may substitute for
/// (`crate::aql::sql::value`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotedKind {
    /// An ISO-8601 date-time leaf. The column is `timestamptz`, populated via
    /// the fail-safe `ext.openehr_timestamp`, and serves AQL temporal
    /// comparison / ordering (QUERY master03 §Built-in Types/Dates and Times).
    Timestamp,
}

/// One promoted leaf: a canonical leaf path on a versioned-object root that is
/// materialized onto a dedicated `node` column.
#[derive(Debug, Clone, Copy)]
pub struct PromotedLeaf {
    /// The versioned-object root RM type the leaf is promoted for (see the
    /// module `rm_type` invariant).
    pub rm_type: &'static str,
    /// The attribute path from the root to the leaf — the flattened
    /// `anchor` attributes followed by the `fragment` names (the structural
    /// split is an analysis artifact this deliberately abstracts over).
    pub path: &'static [&'static str],
    /// The physical `node` column, named through the schema catalog.
    pub column: crate::db::iden::Node,
    /// The column's physical kind.
    pub kind: PromotedKind,
}

/// The registry. Extending it is: a migration (add the column + backfill +
/// index) plus one entry here — no change to the write or read matching code.
pub static PROMOTED_LEAVES: &[PromotedLeaf] = &[
    // COMPOSITION.context.start_time.value → node.context_start (ehr baseline).
    // COMPOSITION occurs only at num = 0, satisfying the module invariant.
    PromotedLeaf {
        rm_type: "COMPOSITION",
        path: &["context", "start_time", "value"],
        column: crate::db::iden::Node::ContextStart,
        kind: PromotedKind::Timestamp,
    },
];

/// The raw promoted-leaf text for one node row, aligned to
/// [`PROMOTED_LEAVES`] (entry `i` ↔ index `i`).
///
/// An entry is `Some` only on the versioned-object root (`num == 0`) whose
/// `rm_type` matches and whose leaf value is present; `None` everywhere else,
/// so non-root rows and context-less compositions carry all-`None`.
///
/// Called at decomposition time with the node's **pre-pruning** JSON, so the
/// leaf (which may sit inside an about-to-be-split structure child, e.g.
/// `EVENT_CONTEXT`) is still reachable.
#[must_use]
pub fn extract(num: i32, rm_type: &str, json: &Value) -> Vec<Option<String>> {
    // Only a versioned-object root carries promoted leaves; every other row
    // returns the EMPTY vec (allocation-free — `Vec::new` does not allocate),
    // which the node writer's positional `.get(i)` reads as all-`None`.
    if num != 0 {
        return Vec::new();
    }
    PROMOTED_LEAVES
        .iter()
        .map(|leaf| {
            if rm_type == leaf.rm_type {
                leaf_text(json, leaf.path)
            } else {
                None
            }
        })
        .collect()
}

/// Follow `path` into `json` and read the scalar leaf as canonical text (the
/// same text the AQL `#>> '{}'` extraction would read). Absent path, JSON null,
/// or a non-scalar (object/array) leaf yield `None`.
fn leaf_text(json: &Value, path: &[&str]) -> Option<String> {
    let mut cur = json;
    for step in path {
        cur = cur.get(step)?;
    }
    match cur {
        Value::String(s) => Some(s.clone()),
        Value::Number(_) | Value::Bool(_) => Some(cur.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every entry's `rm_type` must occur only at a versioned-object root that
    /// never nests (the module invariant): COMPOSITION / `EHR_STATUS` /
    /// `EHR_ACCESS`.
    #[test]
    fn entries_are_non_nesting_roots() {
        for leaf in PROMOTED_LEAVES {
            assert!(
                matches!(leaf.rm_type, "COMPOSITION" | "EHR_STATUS" | "EHR_ACCESS"),
                "{} nests inside its own tree — a promoted leaf on it needs a num=0 read guard",
                leaf.rm_type
            );
        }
    }

    #[test]
    fn extracts_context_start_on_composition_root() {
        let comp = json!({
            "_type": "COMPOSITION",
            "context": {
                "_type": "EVENT_CONTEXT",
                "start_time": {"_type": "DV_DATE_TIME", "value": "2021-01-02T03:04:05Z"}
            }
        });
        assert_eq!(
            extract(0, "COMPOSITION", &comp),
            vec![Some("2021-01-02T03:04:05Z".to_owned())]
        );
    }

    #[test]
    fn no_promotion_off_the_root_or_wrong_type() {
        let comp = json!({
            "_type": "COMPOSITION",
            "context": {"start_time": {"value": "2021-01-02T03:04:05Z"}}
        });
        // Not the root (num != 0): the allocation-free empty vec, which the
        // node writer's positional `.get(i)` reads as all-`None`.
        assert_eq!(
            extract(2, "COMPOSITION", &comp),
            Vec::<Option<String>>::new()
        );
        // Root, but not a registered rm_type.
        let obs = json!({"_type": "OBSERVATION"});
        assert_eq!(extract(0, "OBSERVATION", &obs), vec![None]);
    }

    #[test]
    fn context_less_composition_is_none() {
        // A persistent COMPOSITION with no context (RM ehr master03
        // §COMPOSITION.context [0..1]).
        let comp = json!({"_type": "COMPOSITION", "name": {"value": "persistent"}});
        assert_eq!(extract(0, "COMPOSITION", &comp), vec![None]);
    }
}
