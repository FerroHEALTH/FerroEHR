// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the versioned-object spine: `version`, `audit`,
//! `contribution`, `vo_attestation`, plus the folder-membership and
//! event-outbox writes that ride along inside the same commit transaction.
//!
//! No openEHR spec governs the SQL schema; this module is pure plumbing. Every
//! semantic decision (change classification, version-tree placement, lifecycle,
//! signing, attestation policy, import policy) stays in the versioning layer,
//! which hands these functions plain value inputs and consumes the
//! [`read::StoredVersion`] read shape. The change-control law these rows realize
//! is RM common master06 (§Contributions, §Committal and Audits, §The 'Virtual
//! Version Tree', §Copying), with `AUDIT_DETAILS` and `ATTESTATION` in master04.
//! Every write runs inside a caller-owned `sqlx` transaction so a version, its
//! nodes, contribution, audit and outbox row commit atomically (master06
//! §Committal and Audits: "similar to nested transactions").

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

//
// The versioning layer owns the value types (`AuditInput`, `Kind`, `TreeId`,
// `VersionRead`, `Committed`) and maps them onto the plain inputs and the
// [`StoredVersion`] output here (e.g. `Kind::as_str` → `kind: &str`,
// `TreeId::columns()` → the three tree ints). Storage never depends upward on
// versioning — this decoupling is deliberate and stays.

/// The `FROM` item that joins each version row `v` to its commit audit `a`,
/// with the six audit columns the reads project.
///
/// A plain join is costed over the statistics of `version`, and once they had
/// fallen behind the commits the planner rated a merge join that walks
/// `pk_commit_audit` from its first entry as cheaper than one probe per
/// version row (#4438). A subquery that carries a `LIMIT` is never flattened
/// into the outer join (PostgreSQL 18, `prepjointree.c` `is_simple_subquery`),
/// so this lateral one runs once per version row and probes the key whatever
/// the statistics say. `LIMIT 1` drops nothing: `a.id` is the primary key. No
/// openEHR spec governs the SQL: our own design.
macro_rules! audit_of_version {
    () => {
        concat!(
            "CROSS JOIN LATERAL (SELECT a.system_id, a.change_type, a.description, ",
            "a.committer, a.attestation, a.time_committed FROM commit_audit a ",
            "WHERE a.id = v.commit_audit_id LIMIT 1) a "
        )
    };
}

/// The `version`⋈`audit` column list every version read selects, as a
/// compile-time string concatenation so each query stays a static literal
/// (`sqlx` 0.9 `SqlSafeStr` — no runtime SQL assembly).
///
/// The version's `ATTESTATION`s (RM common master06 §Attestation) are folded in
/// as aggregated jsonb columns via a `LEFT JOIN LATERAL`, so one statement
/// carries the whole version read instead of a second round trip per versioned
/// read (empty in the common case → `[]`). The aggregates order by
/// `time_committed, id`: attestations committed in one transaction share
/// `now()`, so the `uuidv7()` `id` breaks ties in insertion order.
///
/// They arrive split on `at_committal`, the two halves standing in different
/// relations to `VERSION.signature`: an attestation present at the act of
/// committal is inside the version's signed canonical form ("serialising the
/// entire Version object", master06 §Digital Signature) while one added later is
/// not (§Attestation). The split uses the standard aggregate `FILTER` clause
/// (<https://www.postgresql.org/docs/18/sql-expressions.html#SYNTAX-AGGREGATES>).
///
/// The version's canonical body is the row's own `v.body` column, materialized
/// at write time from the same value the node rows decompose from, so a point
/// read is one statement and one TOAST detoast rather than a node-subtree
/// re-aggregation. `NULL` is a logically deleted version (RM common master06
/// §Logical Deletion).
macro_rules! version_select {
    ($tail:literal) => {
        version_select!(""; $tail)
    };
    ($head:expr; $tail:literal) => {
        concat!(
            "SELECT ",
            $head,
            "v.vo_id, v.kind, v.ehr_id, v.sys_version, v.trunk_version, v.branch_number, ",
            "v.branch_version, v.lifecycle_state, v.creating_system_id, v.preceding_version_uid, ",
            "v.other_input_version_uids, v.contribution_id, v.template_id, v.signature, ",
            "v.signature_client_supplied, v.wrapped_original, v.stable_compatible, v.origins, ",
            "v.body, h.restricted_at, h.committed_at AS head_committed_at, (h.person_withheld_until > now()) IS TRUE AS person_withheld, ",
            access_revision!("v.ehr_id"), ", ",
            "(SELECT COALESCE(n.data->>'archetype_node_id', ext.node_archetype_node_id(n.archetype, n.rm_version)) FROM node n WHERE n.tier = v.tier AND n.vo_id = v.vo_id ",
            "AND n.sys_version = v.content_key AND n.num = 0) AS root_archetype, ",
            "a.system_id, a.change_type, a.description, a.committer, a.attestation, ",
            "a.time_committed, ",
            "att.attestations_at_committal, att.attestations_after_committal ",
            "FROM version v JOIN vo_head h ON h.vo_id = v.vo_id ",
            audit_of_version!(),
            "LEFT JOIN LATERAL (",
            "SELECT coalesce(jsonb_agg(x.data ORDER BY x.time_committed, x.id) ",
            "FILTER (WHERE x.at_committal), '[]'::jsonb) AS attestations_at_committal, ",
            "coalesce(jsonb_agg(x.data ORDER BY x.time_committed, x.id) ",
            "FILTER (WHERE NOT x.at_committal), '[]'::jsonb) AS attestations_after_committal ",
            "FROM vo_attestation x ",
            "WHERE x.vo_id = v.vo_id AND x.sys_version = v.sys_version",
            ") att ON true ",
            $tail
        )
    };
}

pub mod attestation;
pub mod commit;
pub mod contribution;
pub mod ehr_access;
pub mod gate;
pub mod guarded;
pub mod import;
pub mod judged;
pub mod meta;
pub mod placement;
pub mod read;
pub mod tier;

/// `other_input_version_uids` stores NULL when empty (`Is_merged_validity`),
/// else the JSON array.
fn optional_json_array(uids: &[String]) -> Option<serde_json::Value> {
    if uids.is_empty() {
        None
    } else {
        Some(serde_json::json!(uids))
    }
}
