// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[licence]` configuration section.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Commercial licence configuration.
///
/// Optional in every respect: an unset, missing, malformed or expired token
/// is logged once at boot and the server runs unlicensed, identically in
/// every other way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LicenceConfig {
    /// Path of the licence token file the licensor issued.
    pub file: Option<PathBuf>,
}
