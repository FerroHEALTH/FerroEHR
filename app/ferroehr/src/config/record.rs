// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[record]` table: the record itself, how it is indexed, versioned,
//! signed, queried, fed out and where its large media live.
//!
//! No openEHR spec governs configuration: our own design. Each sub-table is
//! owned by the module that consumes it and referenced here.

use serde::{Deserialize, Serialize};

/// The `[record]` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecordConfig {
    /// `subject_index`: who holds the subject-to-EHR cross-reference, the
    /// IHE patient identity source of `[pixm]` (`pixm`) or the in-instance
    /// map (`local`). No default: an unset index refuses the start.
    pub subject_index: Option<crate::service::ehr_index::config::SubjectIndex>,
    /// `[record.versioning]`: the answer to an identical re-send.
    pub versioning: crate::config::versioning::VersioningConfig,
    /// `[record.signing]`: VERSION signing.
    pub signing: crate::versioning::signature::config::SigningConfig,
    /// `[record.query]`: AQL execution knobs.
    pub query: crate::service::query::config::QueryConfig,
    /// `[record.change_feed]`: the commit-event outbox readers and their
    /// retention.
    pub change_feed: crate::extensions::outbox::config::OutboxConfig,
    /// `[record.multimedia]`: `DV_MULTIMEDIA` externalization to object
    /// storage. It keeps `enabled`, because its store applies while it is
    /// off: blobs already offloaded are read back from `url`.
    pub multimedia: crate::extensions::multimedia::config::MultimediaConfig,
}
