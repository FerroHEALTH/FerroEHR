// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[audit.access_log]` section: where the log's checkpoints are
//! published and the systems whose exported logs this deployment accepts
//! (#5129, #5704).
//!
//! Signing the log needs no key of its own here: whenever the keyring
//! (`[record.signing]` in `pgp` mode) holds an `OpenPGP` key each seal is
//! signed as a checkpoint, and without one the seals are unsigned (#5562,
//! #6242). No openEHR spec governs log signing or its configuration: our own
//! design (the draft's semantics page, `docs/specifications/access-log`, §Integrity).

#![cfg_attr(
    feature = "schema",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 8): the configuration schema's \
                  extension literals the schemars derive builds"
    )
)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The `[audit.access_log]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = {"publish_directory": "infrastructure", "trusted_key_files": "trust"})))]
pub struct AccessLogConfig {
    /// A directory outside the database, ideally on write-once storage,
    /// where the server publishes the newest checkpoint as its own file every
    /// 30 seconds (`FERROEHR__AUDIT__ACCESS_LOG__PUBLISH_DIRECTORY`). Only a
    /// published checkpoint shows that the newest entries were removed
    /// together with the checkpoints that seal them; unset, the
    /// `access_log_checkpoints` indicator reads `DEGRADED`.
    pub publish_directory: Option<PathBuf>,
    /// Armored `OpenPGP` public certificates of the systems whose exported
    /// logs this deployment imports
    /// (`FERROEHR__AUDIT__ACCESS_LOG__TRUSTED_KEY_FILES`). An import verifies
    /// the checkpoint against these and the keyring's own key, and is refused
    /// when none signed it.
    pub trusted_key_files: Vec<PathBuf>,
}
