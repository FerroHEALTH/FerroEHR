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
//! §Committal: "similar to nested transactions").
//!
//! One file per concern; consumers import each item from its defining
//! submodule (no re-exports):
//! - [`commit`] — the local write path: audit/contribution inserts, the
//!   folded one-statement version commit, lineage-tip close, and the
//!   ride-along folder-membership + event-outbox writes.
//! - [`import`] — the EHR-Extract / archive-load write path: explicit
//!   `committed_at` version rows, the head sync, container-state read.
//! - [`read`] — the full version reads ([`read::StoredVersion`]: metadata + body +
//!   attestations) by current / ordinal / tree id / instant.
//! - [`placement`] — the version-tree placement reads (lineage tip, next
//!   ordinal, transaction timestamp); the placement *decision* stays in
//!   versioning.
//! - [`attestation`] — `vo_attestation` writes and reads.
//! - [`meta`] — the lean metadata-only reads (`ETag`/`If-Match`, revision
//!   history, existence/kind/count lookups) that skip node reassembly.
//! - [`contribution`] — CONTRIBUTION reads (audit, affected versions,
//!   listing/counting).
//! - [`tier`] — the cold archival storage tier: the physical move behind SM
//!   `I_ADMIN_ARCHIVE`, its reverse, and the primary-miss read fallback the
//!   reads above use.

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

pub mod attestation;
pub mod commit;
pub mod contribution;
pub mod import;
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
