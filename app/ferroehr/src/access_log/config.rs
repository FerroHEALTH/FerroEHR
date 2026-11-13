// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[audit.access_log]` section: the systems whose exported logs this
//! deployment accepts (#5129).
//!
//! Sealing the log needs no key of its own here: whenever the keyring
//! (`[record.signing]` in `pgp` mode) holds an `OpenPGP` key the server seals
//! the chain in signed checkpoints on a built-in schedule, and without one it
//! seals nothing (#5562). No openEHR spec governs log signing or its
//! configuration: our own design (`docs/plans/access-log.md` §Integrity).

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
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct AccessLogConfig {
    /// Armored `OpenPGP` public certificates of the systems whose exported
    /// logs this deployment imports
    /// (`FERROEHR__AUDIT__ACCESS_LOG__TRUSTED_KEY_FILES`). An import verifies
    /// the checkpoint against these and the keyring's own key, and is refused
    /// when none signed it.
    pub trusted_key_files: Vec<PathBuf>,
}
