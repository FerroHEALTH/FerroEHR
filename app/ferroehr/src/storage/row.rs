// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Node-row shapes: the full write row produced by decomposition, and the lean
//! read row the repository fetches back.
//!
//! No openEHR spec governs the physical row layout — this is our own decomposed
//! node model. The promoted columns and the
//! nested-set index (`num`/`num_cap`/`parent_num`) exists to make AQL
//! CONTAINS an integer interval join, never a JSON walk.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;

/// One decomposed `node` row to write (content columns only — the storage
/// context `vo_id`/`sys_version`/`ehr_id` is added by
/// [`crate::storage::node_repo::write_nodes`]).
///
/// Carries the full set of promoted query columns (`rm_type`, `archetype`,
/// `arch_*`, `name`, `name_code`, `name_terminology`) alongside the nested-set
/// index and the pruned JSON fragment — everything the `node` table stores per
/// row.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRow {
    /// Pre-order number within the versioned object (root = 0).
    pub num: i32,
    /// Max `num` in this row's subtree: the subtree is `num..=num_cap`.
    pub num_cap: i32,
    /// `num` of the parent structure node (root points at itself/0).
    pub parent_num: i32,
    /// The RM `_type`, verbatim (e.g. `OBSERVATION`).
    pub rm_type: String,
    /// `archetype_node_id`, verbatim.
    pub archetype: Option<String>,
    /// `qualified_rm_entity` of a full archetype HRID, lowercased for
    /// comparison; `None` on at/id-code nodes (BASE `base_types` master05
    /// §Archetype Identifiers).
    pub arch_entity: Option<String>,
    /// Full `domain_concept` (incl. specialisation segments) of a full archetype
    /// HRID, lowercased; `None` on at/id-code nodes. A parent-archetype query
    /// matches a specialisation child via a `concept-%` prefix (BASE
    /// `architecture_overview` master10 §Design-time Relationships).
    pub arch_concept: Option<String>,
    /// Major version (`.v` major) of a full archetype HRID; `None` on at/id-code
    /// nodes. The interface-reference major boundary is hard (AM master07
    /// §Querying).
    pub arch_major: Option<i32>,
    /// `name/value`.
    pub name: Option<String>,
    /// `name/defining_code/code_string` when the name is a `DV_CODED_TEXT`;
    /// `None` on a plain `DV_TEXT` name. Promoted for the AQL node predicate
    /// (QUERY `master03-syntax.adoc` §Node predicate).
    pub name_code: Option<String>,
    /// `name/defining_code/terminology_id/value` when the name is coded;
    /// `None` otherwise.
    pub name_terminology: Option<String>,
    /// Materialized path from the root: full attribute names, array index
    /// appended, `.`-terminated steps (`content0.data.events1.`) so byte
    /// order under `COLLATE "C"` equals tree order.
    pub path: String,
    /// The node's canonical JSON fragment, structure children pruned.
    pub data: Value,
    /// The raw promoted-leaf text for this row, aligned to
    /// [`crate::storage::promoted::PROMOTED_LEAVES`] (entry `i` ↔ index `i`); `Some` only
    /// on a versioned-object root whose leaf is present, `None` elsewhere.
    /// [`crate::storage::node_repo::write_nodes`] converts and writes each into
    /// its promoted column. No openEHR spec governs promoted columns — our own
    /// storage design.
    pub promoted: Vec<Option<String>>,
}

/// The lean read row: only the columns [`crate::storage::codec::reassemble`]
/// and the nested-set contract need.
///
/// The read path fetches exactly these five columns — the promoted query
/// columns (`rm_type`/`archetype`/`arch_*`/`name`) live inside `data` and are
/// not needed to reconstruct the canonical tree.
///
/// `num_cap`/`parent_num` are surfaced (unused by reassembly) because they are
/// the nested-set interval contract the AQL engine consumes when it reads the
/// `node` table directly.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadRow {
    /// Pre-order number within the versioned object (root = 0).
    pub num: i32,
    /// Max `num` in this row's subtree (`num..=num_cap`).
    pub num_cap: i32,
    /// `num` of the parent structure node.
    pub parent_num: i32,
    /// Materialized path from the root (see [`NodeRow::path`]).
    pub path: String,
    /// The node's canonical JSON fragment.
    pub data: Value,
}

/// Read access to the three fields [`crate::storage::codec::reassemble`]
/// needs from a node row — its pre-order number, its materialized path, and
/// its JSON fragment.
///
/// Implemented by both the write [`NodeRow`] and the lean [`ReadRow`], so
/// reassembly works from either shape without forcing the read path to fetch
/// the promoted query columns.
pub trait NodeContent {
    /// Pre-order number within the versioned object (root = 0).
    fn num(&self) -> i32;
    /// Materialized path from the root.
    fn path(&self) -> &str;
    /// The node's canonical JSON fragment.
    fn data(&self) -> &Value;
}

impl NodeContent for NodeRow {
    fn num(&self) -> i32 {
        self.num
    }
    fn path(&self) -> &str {
        &self.path
    }
    fn data(&self) -> &Value {
        &self.data
    }
}

impl NodeContent for ReadRow {
    fn num(&self) -> i32 {
        self.num
    }
    fn path(&self) -> &str {
        &self.path
    }
    fn data(&self) -> &Value {
        &self.data
    }
}
