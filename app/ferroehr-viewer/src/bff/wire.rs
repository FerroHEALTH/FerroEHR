// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The wire shapes the shared component kit renders.
//!
//! A screen's own flattened types live with that screen. These two carry what
//! SEVERAL screens read from the CDR's versioned surfaces, and what the shared
//! kit ([`version_history`](crate::components::version_history)) renders: a
//! `components` module reaching into `pages` for a type would invert the crate's
//! one dependency arrow.
//!
//! Both are flattened BFF-side so the browser never re-models the RM, and both
//! carry fixed-size-safe fields only — no `usize` crosses a server-function
//! boundary on a 32-bit target.

use serde::{Deserialize, Serialize};

/// One entry in a versioned object's revision history, flattened for the
/// version selector, the history table and the audit card.
///
/// The attributes are `AUDIT_DETAILS`'s own
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.audit_details.adoc`)
/// beside the VERSION's `uid`; a revision history is
/// `List<REVISION_HISTORY_ITEM>`, each item a version id plus its audits
/// (`org.openehr.rm.common.revision_history_item.adoc`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionEntry {
    /// The `OBJECT_VERSION_ID` value (`uuid::system::version`).
    pub version_id: String,
    /// `AUDIT_DETAILS.time_committed` value.
    pub committed: String,
    /// `AUDIT_DETAILS.change_type` value (the `DV_CODED_TEXT` label).
    pub change_type: String,
    /// `AUDIT_DETAILS.committer` name.
    pub committer: String,
}

/// A versioned object's container facts plus one of its VERSIONs' envelope
/// facts — the shape every History tab's versioned-object card is built from.
///
/// One type for every family: the `VERSIONED_EHR_STATUS` container and the
/// demographic ones carry the same eight facts, differing only in which of them
/// a given family populates and in how each screen lays them out.
///
/// The attributes are the RM classes' own (files under
/// `docs/specs/openehr/RM/docs/UML/classes/`): `VERSIONED_OBJECT._uid_`,
/// `_owner_id_` and `_time_created_`
/// (`org.openehr.rm.common.versioned_object.adoc`); `VERSION._contribution_`,
/// `_signature_` and `_preceding_version_uid_`, whose invariant
/// `Preceding_version_uid_validity` makes it absent exactly for a first version
/// (`org.openehr.rm.common.version.adoc`); and
/// `ORIGINAL_VERSION._lifecycle_state_`
/// (`org.openehr.rm.common.original_version.adoc`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VersionedObjectFacts {
    /// `VERSIONED_OBJECT.uid.value` — the versioned-object id.
    pub object_uid: String,
    /// `VERSIONED_OBJECT.owner_id.id.value` — the owning EHR; empty on a
    /// demographic container, which has no owning EHR.
    pub owner_id: String,
    /// `VERSIONED_OBJECT.time_created.value` — when the first version was
    /// committed.
    pub time_created: String,
    /// The read VERSION's `uid.value` (`OBJECT_VERSION_ID`).
    pub version_id: String,
    /// `ORIGINAL_VERSION.lifecycle_state.value`.
    pub lifecycle_state: String,
    /// `VERSION.preceding_version_uid.value` — empty for a first version.
    pub preceding_version_uid: String,
    /// `VERSION.contribution.id.value`.
    pub contribution_uid: String,
    /// Whether the VERSION carries a `signature`.
    pub signed: bool,
}
