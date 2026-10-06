// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[multimedia]` section — `DV_MULTIMEDIA` externalization.
//!
//! **No openEHR spec governs this — our own design/extension.** A field of the
//! one config tree ([`crate::config::FerroEhrConfig`]); no loader of its own.
//!
//! **Off by default** (`enabled = false`): with externalization disabled the
//! commit/read paths are byte-identical to today's inline behaviour and no
//! object store is ever contacted. The secret access key is a shared
//! [`crate::config::secret::Secret`] (never rendered) with a `*_file` sibling.

#![expect(
    clippy::doc_markdown,
    reason = "product identifiers (SeaweedFS, object_store, …) read as prose in \
              this module's docs"
)]

use std::path::PathBuf;

use crate::config::secret::{Secret, SecretUrl};
use serde::{Deserialize, Serialize};

/// DV_MULTIMEDIA externalization settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MultimediaConfig {
    /// Master switch. `false` (default) = today's inline behaviour, byte for
    /// byte; no object store is built or contacted.
    pub enabled: bool,
    /// A decoded `DV_MULTIMEDIA.data` strictly larger than this many bytes is
    /// offloaded to the object store; at or below it stays inline. Default
    /// 256 `KiB`.
    pub threshold_bytes: usize,
    /// S3-compatible endpoint URL (e.g. a SeaweedFS S3 gateway in dev/test, or
    /// an AWS/MinIO endpoint in prod). `None` uses the object_store default AWS
    /// endpoint resolution. Any `userinfo` in it is masked in every rendering.
    pub endpoint: Option<SecretUrl>,
    /// Target bucket for content-addressed blobs.
    pub bucket: String,
    /// AWS region (S3 requires one even for non-AWS endpoints).
    pub region: String,
    /// Access key id. `None` (with `secret_access_key` also `None`) runs the
    /// client unsigned/anonymous — the mode a keyless dev SeaweedFS accepts.
    pub access_key_id: Option<String>,
    /// Secret access key (paired with `access_key_id`); never rendered.
    pub secret_access_key: Option<Secret>,
    /// File-based indirection for [`Self::secret_access_key`] (K8s/Docker
    /// secrets). Exactly one of the pair may be set; the loader reads and trims
    /// the file.
    pub secret_access_key_file: Option<PathBuf>,
    /// Allow plain-HTTP endpoints (dev/test only — a SeaweedFS container speaks
    /// HTTP). Production S3 is HTTPS, so this stays `false` there.
    pub allow_http: bool,
}

impl Default for MultimediaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_bytes: 256 * 1024,
            endpoint: None,
            bucket: "openehr-multimedia".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: None,
            secret_access_key: None,
            secret_access_key_file: None,
            allow_http: false,
        }
    }
}

impl MultimediaConfig {
    /// Whether the client should run unsigned/anonymous (no credentials given).
    #[must_use]
    pub fn is_anonymous(&self) -> bool {
        self.access_key_id.is_none() && self.secret_access_key.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled_with_256kib_threshold() {
        let c = MultimediaConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.threshold_bytes, 256 * 1024);
        assert_eq!(c.bucket, "openehr-multimedia");
        assert!(c.is_anonymous());
        assert!(!c.allow_http);
    }
}
