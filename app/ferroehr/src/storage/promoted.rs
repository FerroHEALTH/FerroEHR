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
//! One registry drives the write ([`crate::storage::render::nodes`]) and the
//! read (`crate::aql::sql::value`), so the two never disagree on the mapping.
//!
//! ## Invariant on `rm_type`
//!
//! A promoted leaf's value lives on the root node (`num = 0`) of its versioned
//! object and the column is populated only there. The read side identifies the
//! root by `rm_type`, so an entry's `rm_type` must occur only at `num = 0`, that
//! is, a versioned-object root that never nests inside its own tree.
//! `COMPOSITION`, `EHR_STATUS` and `EHR_ACCESS` satisfy this; `FOLDER` does not,
//! so a `FOLDER` leaf would need an explicit `num = 0` guard on the read side.

/// The physical type a promoted column carries.
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

/// The promoted leaves of a root row
/// ([`PROMOTED_LEAVES`]), read off the rendering of its body.
pub(crate) fn of_root(
    body: &crate::storage::render::index::Rendered,
    num: i32,
    rm_type: &str,
    node: crate::storage::render::index::NodeId,
) -> Vec<Option<String>> {
    if num != 0 {
        return Vec::new();
    }
    PROMOTED_LEAVES
        .iter()
        .map(|leaf| {
            if rm_type != leaf.rm_type {
                return None;
            }
            let mut at = node;
            for step in leaf.path {
                at = body.get(at, step)?;
            }
            match body.kind(at)? {
                crate::storage::render::index::Kind::String
                | crate::storage::render::index::Kind::EscapedString => {
                    body.str(at).map(std::borrow::Cow::into_owned)
                }
                crate::storage::render::index::Kind::Scalar if body.raw(at) != b"null" => {
                    Some(body.raw_text(at).to_owned())
                }
                crate::storage::render::index::Kind::Scalar
                | crate::storage::render::index::Kind::Object
                | crate::storage::render::index::Kind::Array => None,
            }
        })
        .collect()
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "the tests state the promoted leaves of canonical bodies"
)]
mod tests {
    use super::*;

    /// The promoted leaves of every row of the decomposition of `body`.
    fn promoted(body: &serde_json::Value) -> Vec<Vec<Option<String>>> {
        crate::storage::codec::decompose(body.clone())
            .unwrap()
            .into_iter()
            .map(|row| row.promoted)
            .collect()
    }
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
        // The EVENT_CONTEXT is a structure node of its own, and promotes nothing.
        assert_eq!(
            promoted(&comp),
            vec![vec![Some("2021-01-02T03:04:05Z".to_owned())], Vec::new()]
        );
    }

    #[test]
    fn no_promotion_off_the_root_or_wrong_type() {
        let comp = json!({
            "_type": "COMPOSITION",
            "context": {"start_time": {"value": "2021-01-02T03:04:05Z"}},
            "content": [{"_type": "OBSERVATION", "context": {"start_time": {"value": "x"}}}]
        });
        // Not the root (num != 0): the allocation-free empty vec, which the
        // node writer's positional `.get(i)` reads as all-`None`.
        assert_eq!(
            promoted(&comp),
            vec![vec![Some("2021-01-02T03:04:05Z".to_owned())], Vec::new()]
        );
        // Root, but not a registered rm_type.
        let status = json!({"_type": "EHR_STATUS", "context": {"start_time": {"value": "x"}}});
        assert_eq!(promoted(&status), vec![vec![None]]);
    }

    #[test]
    fn context_less_composition_is_none() {
        // A persistent COMPOSITION with no context (RM ehr master03
        // §COMPOSITION.context [0..1]).
        let comp = json!({"_type": "COMPOSITION", "name": {"value": "persistent"}});
        assert_eq!(promoted(&comp), vec![vec![None]]);
    }
}
