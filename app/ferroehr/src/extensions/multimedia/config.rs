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

use crate::config::secret::{Secret, SecretUrl, redact_userinfo};
use serde::{Deserialize, Serialize};
use url::Url;

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

/// Why [`MultimediaConfig::endpoint_url`] refused `multimedia.endpoint`.
///
/// Every message quotes the endpoint only through [`redact_userinfo`], so no
/// refusal carries a credential.
#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    /// The key is set to a blank string.
    #[error(
        "multimedia.endpoint is set but empty — give an absolute URL \
         (e.g. http://seaweedfs:8333) or remove the key to use default \
         AWS endpoint resolution"
    )]
    Empty,
    /// The value does not parse as an absolute URL.
    #[error("multimedia.endpoint {shown:?} is not an absolute URL: {reason}")]
    NotAbsolute {
        /// The endpoint, `userinfo` masked.
        shown: String,
        /// The parse failure.
        reason: url::ParseError,
    },
    /// The value parses, but its scheme is neither `http` nor `https`.
    #[error(
        "multimedia.endpoint {shown:?} has scheme {scheme:?} — an S3 endpoint \
         must be http or https (did you mean \"http://{shown}\"?)"
    )]
    NotHttp {
        /// The endpoint, `userinfo` masked.
        shown: String,
        /// The scheme found.
        scheme: String,
    },
}

impl MultimediaConfig {
    /// Whether the client should run unsigned/anonymous (no credentials given).
    #[must_use]
    pub fn is_anonymous(&self) -> bool {
        self.access_key_id.is_none() && self.secret_access_key.is_none()
    }

    /// Parses [`Self::endpoint`] into the absolute `http`/`https` URL the S3
    /// client connects to; `None` when the key is absent.
    ///
    /// The one validation of the key: boot validation and the object-store
    /// construction both call it. `seaweedfs:8333` parses as a URL (scheme
    /// `seaweedfs`, path `8333`), so the scheme is judged on top of the syntax.
    ///
    /// # Errors
    /// [`EndpointError`] for a blank value, a value that is not an absolute
    /// URL, or a scheme other than `http`/`https`.
    pub fn endpoint_url(&self) -> Result<Option<Url>, EndpointError> {
        let Some(endpoint) = &self.endpoint else {
            return Ok(None);
        };
        let trimmed = endpoint.expose().trim();
        if trimmed.is_empty() {
            return Err(EndpointError::Empty);
        }
        let url = Url::parse(trimmed).map_err(|reason| EndpointError::NotAbsolute {
            shown: redact_userinfo(trimmed),
            reason,
        })?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(EndpointError::NotHttp {
                shown: redact_userinfo(trimmed),
                scheme: url.scheme().to_owned(),
            });
        }
        Ok(Some(url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every refusal of a malformed endpoint carrying `userinfo` keeps the
    /// password out of its message, and an absent or absolute endpoint passes.
    #[test]
    fn endpoint_refusals_never_quote_userinfo() {
        let with = |endpoint: &str| MultimediaConfig {
            endpoint: Some(SecretUrl::new(endpoint)),
            ..MultimediaConfig::default()
        };
        for bad in ["", "   ", "seaweedfs:8333", "/bucket"] {
            assert!(with(bad).endpoint_url().is_err(), "{bad:?} must be refused");
        }
        for leaky in [
            "ftp://user:hunter2@s3.example",
            "s3://user:hunter2@s3.example/bucket",
            "https://user:hunter2@[::1",
        ] {
            let err = with(leaky).endpoint_url().expect_err("refused");
            assert!(!err.to_string().contains("hunter2"), "{err}");
            assert!(!format!("{err:?}").contains("hunter2"), "{err:?}");
        }
        assert!(matches!(
            MultimediaConfig::default().endpoint_url(),
            Ok(None)
        ));
        let url = with("https://user:hunter2@s3.example")
            .endpoint_url()
            .expect("an absolute https endpoint parses")
            .expect("set");
        assert_eq!(url.host_str(), Some("s3.example"));
    }

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
