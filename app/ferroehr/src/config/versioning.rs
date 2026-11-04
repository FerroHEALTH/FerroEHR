// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[versioning]`: what a commit stores and what a repeated commit answers.
//!
//! A version whose content equals its predecessor's stores that content
//! once (#4415); that is not a setting. The one setting here is the opt-in
//! answer to an identical re-send. ITS-REST does not govern skipping an
//! identical commit: our own design.

use serde::{Deserialize, Serialize};

/// The `operation` of the access record a skipped identical re-send writes
/// (`<version uid>#sha256:<content digest>` as its resource id).
pub const RESEND_OPERATION: &str = "resend";

/// The `[versioning]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VersioningConfig {
    /// `skip_identical_resend` (default `false`): when `true`, an update whose
    /// content equals the current version's commits nothing and answers with
    /// the existing version's `ETag` and `Location`, leaving one `resend`
    /// record on the access log. No `commit_audit` and no CONTRIBUTION then
    /// record the act (RM common master06 §Committal and Audits), which is
    /// why the default records every commit.
    pub skip_identical_resend: bool,
}
