// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[audit.access_log]` section: the signed checkpoints of the access
//! log, the signature of each link, and the systems whose exported logs this
//! deployment accepts (#5129).
//!
//! The key that signs is the keyring's (`[record.signing]` in `pgp` mode), so
//! a deployment configures one key for versions, checkpoints and links. No
//! openEHR spec governs log signing or its configuration: our own design
//! (`docs/plans/access-log.md` §Integrity).

#![cfg_attr(
    feature = "schema",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 8): the configuration schema's \
                  extension literals the schemars derive builds"
    )
)]

use std::path::PathBuf;

use ferrohealth_config::units::Duration;
use serde::{Deserialize, Serialize};

/// The `[audit.access_log]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct AccessLogConfig {
    /// Seal the head of the chain in a signed `ACCESS_LOG_CHECKPOINT`
    /// (`FERROEHR__AUDIT__ACCESS_LOG__CHECKPOINTS`). Off by default; on, the
    /// keyring must hold an `OpenPGP` key (`record.signing.mode = "pgp"`), or
    /// the server refuses to start.
    pub checkpoints: bool,
    /// The time between two scheduled checkpoints
    /// (`FERROEHR__AUDIT__ACCESS_LOG__CHECKPOINT_INTERVAL`); a run writes one
    /// only when the chain has grown since the last. `"0s"` leaves
    /// checkpoints to `ferroehr access-log checkpoint` and
    /// `POST /admin/access-log/checkpoint`. Default one hour.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
    pub checkpoint_interval: Duration,
    /// Sign the digest of each new link with the keyring's key
    /// (`FERROEHR__AUDIT__ACCESS_LOG__SIGN_LINKS`), as the eHDSI Audit Trail
    /// Profile §2.2 asks of a national contact point. Off by default: it
    /// costs one signature per record. On, the keyring must hold an `OpenPGP`
    /// key.
    pub sign_links: bool,
    /// Armored `OpenPGP` public certificates of the systems whose exported
    /// logs this deployment imports
    /// (`FERROEHR__AUDIT__ACCESS_LOG__TRUSTED_KEY_FILES`). An import verifies
    /// the checkpoint against these and the keyring's own key, and is refused
    /// when none signed it.
    pub trusted_key_files: Vec<PathBuf>,
}

impl Default for AccessLogConfig {
    fn default() -> Self {
        Self {
            checkpoints: false,
            checkpoint_interval: Duration::from_hours(1),
            sign_links: false,
            trusted_key_files: Vec::new(),
        }
    }
}

impl AccessLogConfig {
    /// Whether this section asks the keyring for an `OpenPGP` key.
    #[must_use]
    pub fn needs_key(&self) -> bool {
        self.checkpoints || self.sign_links
    }
}
