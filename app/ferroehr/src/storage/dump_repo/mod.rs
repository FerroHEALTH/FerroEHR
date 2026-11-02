// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O of the admin dump and load: the rows an archive carries, read
//! per EHR or per demographic container, and written back verbatim.
//!
//! Each row type is also the archive's on-disk shape, so its serde form is the
//! archive format; a field added here is a format change. No openEHR spec
//! defines an on-disk archive format or the SQL: our own design/extension.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 3): EHR-Extract/dump-load compose over \
              verbatim stored content (RM common master06 §Copying)"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};

pub mod read;
pub mod write;

/// What a dump read covers.
#[derive(Debug, Clone, Copy)]
pub enum Scope {
    /// One EHR and everything it holds.
    Ehr(EhrId),
    /// One ehr-less demographic versioned object.
    Container(VoId),
}

/// The archived `ehr` root row.
#[derive(Debug, Serialize, Deserialize)]
pub struct EhrRow {
    /// The EHR id.
    pub id: EhrId,
    /// The immutable `system_id`.
    pub system_id: String,
    /// The immutable `time_created`, as `PostgreSQL` renders it as text.
    pub time_created: String,
    /// The promoted subject id.
    pub subject_id: Option<String>,
    /// The promoted subject namespace.
    pub subject_namespace: Option<String>,
}

/// One archived `commit_audit` row.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuditRow {
    /// The row id.
    pub id: Uuid,
    /// `AUDIT_DETAILS.time_committed` as an RFC 3339 instant — `jiff`'s own
    /// rendering of the stored `timestamptz`, so the export can parse it back
    /// losslessly when it reassembles the version's `commit_audit`
    /// (RM common master04 §Audit Details). `PostgreSQL`'s `timestamptz` input
    /// accepts this form and the older `::text` rendering alike
    /// (<https://www.postgresql.org/docs/18/datatype-datetime.html>
    /// §Date/Time Input), so archives written before this projection still
    /// load.
    pub time_committed: String,
    /// `AUDIT_DETAILS.system_id`.
    pub system_id: String,
    /// The `change_type` code.
    pub change_type: String,
    /// The canonical `DV_TEXT` fragment of `AUDIT_DETAILS.description` (0..1).
    pub description: Option<Value>,
    /// The canonical committer fragment.
    pub committer: Value,
    /// The `ATTESTATION`-declared attributes when the audit is an `ATTESTATION`
    /// (RM common master06 §Attestation), else absent — so an archive
    /// round-trips the concrete audit class.
    #[serde(default)]
    pub attestation: Option<Value>,
}

/// One archived `contribution` row.
#[derive(Debug, Serialize, Deserialize)]
pub struct ContributionRow {
    /// The contribution id.
    pub id: Uuid,
    /// Its commit audit.
    pub commit_audit_id: Uuid,
}

/// One archived `version` row with its payload.
#[derive(Debug, Serialize, Deserialize)]
pub struct VersionRecord {
    /// The versioned object.
    pub vo_id: VoId,
    /// The RM type of the versioned content.
    pub kind: String,
    /// The storage sequence number within the object.
    pub sys_version: i32,
    /// `VERSION_TREE_ID` columns (0/0 = trunk row).
    pub trunk_version: i32,
    /// The branch number (0 on the trunk).
    pub branch_number: i32,
    /// The version on the branch (0 on the trunk).
    pub branch_version: i32,
    /// Stored `ORIGINAL_VERSION.preceding_version_uid` (`None` for a first
    /// version).
    pub preceding_version_uid: Option<String>,
    /// Merge provenance (`None` when not a merge).
    pub other_input_version_uids: Option<Value>,
    /// The stored commit instant; validity is derived from it, so no upper
    /// bound is dumped.
    pub committed_at: Option<String>,
    /// The lifecycle-state code.
    pub lifecycle_state: String,
    /// The committing contribution.
    pub contribution_id: Uuid,
    /// The commit audit.
    pub commit_audit_id: Uuid,
    /// The template the content was committed against.
    pub template_id: Option<String>,
    /// The version `signature`.
    pub signature: Option<String>,
    /// Whether `signature` was client-supplied (foreign — never re-verified at
    /// read; master06 §Digital Signature). `#[serde(default)]` so archives dumped
    /// before this field existed load as server-generated.
    #[serde(default)]
    pub signature_client_supplied: bool,
    /// The `creating_system_id` of the version.
    pub creating_system_id: String,
    /// The wrapped `ORIGINAL_VERSION`'s own `{contribution, commit_audit,
    /// signature?}` on an `IMPORTED_VERSION` row (RM common master06 §Committal
    /// and Audits), `None` on a locally created version. `#[serde(default)]` so
    /// archives dumped before this member existed load as local originals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapped_original: Option<Value>,
    /// The reassembled canonical openEHR JSON, INLINE — the
    /// `openehr_canonical_json` payload form. `Value::Null` (serialized as an
    /// absent key) for a logically-deleted version, which stores no node rows,
    /// and for every version of an `openehr_canonical_xml` archive, whose
    /// payload lives in [`VersionRecord::body_entry`] instead. A read leaves it
    /// `Null`; the export fills it.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub body: Value,
    /// The `versions/<version_uid>.xml` entry holding this version's complete
    /// `ORIGINAL_VERSION` document — the `openehr_canonical_xml` payload form.
    /// `None` for an inline-JSON archive and for a logically-deleted version
    /// (no content ⇒ no document). `#[serde(default)]` so archives written
    /// before the XML member existed load unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_entry: Option<String>,
}

/// One archived `vo_attestation` row.
#[derive(Debug, Serialize, Deserialize)]
pub struct AttestationRow {
    /// The row id.
    pub id: Uuid,
    /// The attested versioned object.
    pub vo_id: VoId,
    /// The attested version.
    pub sys_version: i32,
    /// The contribution that carried the attestation.
    pub contribution_id: Uuid,
    /// `ATTESTATION.time_committed` as an RFC 3339 instant (the same lossless
    /// `jiff` rendering [`AuditRow::time_committed`] uses).
    pub time_committed: String,
    /// Whether the ATTESTATION was on the VERSION at committal and is
    /// therefore inside its signed canonical form (RM common master06
    /// §Attestation / §Digital Signature).
    pub at_committal: bool,
    /// The canonical `ATTESTATION` fragment, verbatim.
    pub data: Value,
}

/// One archived `ehr_folder` membership row (RM ehr §EHR Class
/// `Directory_in_folders`; ranks are append-only).
#[derive(Debug, Serialize, Deserialize)]
pub struct FolderRankRow {
    /// The rank of the folder hierarchy.
    pub rank: i32,
    /// The folder hierarchy's versioned object.
    pub vo_id: VoId,
}

/// One archived `item_tag` row.
#[derive(Debug, Serialize, Deserialize)]
pub struct ItemTagRow {
    /// The row id.
    pub id: Uuid,
    /// The tagged versioned object.
    pub target_vo_id: VoId,
    /// The RM type of the tagged object.
    pub target_type: String,
    /// The tag key.
    pub key: String,
    /// The tag value.
    pub value: Option<String>,
    /// The path within the target, stored opaque.
    pub target_path: Option<String>,
    /// When the tag was created, as `PostgreSQL` renders it as text.
    pub created_at: String,
}

/// One archive marker of a head row.
#[derive(Debug, Serialize, Deserialize)]
pub struct ArchiveRow {
    /// The archived versioned object.
    pub vo_id: VoId,
    /// When it was archived, as `PostgreSQL` renders it as text.
    pub archived_at: String,
    /// The archive reason.
    pub reason: Option<String>,
}
