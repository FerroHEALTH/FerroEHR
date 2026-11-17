// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[record.multimedia]` section: `DV_MULTIMEDIA` externalization.
//!
//! **No openEHR spec governs this: our own design/extension.** A field of the
//! one config tree ([`crate::config::FerroEhrConfig`]); no loader of its own.
//!
//! **Off by default** (`enabled = false`): with externalization disabled and
//! no endpoint the commit/read paths are byte-identical to today's inline
//! behaviour and no blob store is ever contacted. The section keeps
//! `enabled` where other optional features use the table's presence, because
//! the backend applies while it is off: the blobs already offloaded are read
//! back. `backend` picks the storage service and its own table carries that
//! service's settings (`[record.multimedia.s3]`), so a further service is a
//! further variant and table. The secret access key is a shared
//! [`ferrohealth_secret::secret::Secret`] (never rendered) with a `*_file`
//! sibling.
#![expect(
    clippy::doc_markdown,
    reason = "product identifiers (SeaweedFS, MinIO, …) read as prose in \
              this module's docs"
)]
#![cfg_attr(
    feature = "schema",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 8, OpenAPI schema literals): the \
                  schemars derive builds the JSON Schema of this module's configuration type \
                  with serde_json::Map, which no item-level attribute reaches"
    )
)]

use std::path::PathBuf;

use ferrohealth_config::units::{ByteSize, Duration};
use ferrohealth_secret::secret::Secret;
use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_smart::config::mask_userinfo;
use serde::{Deserialize, Serialize};
use url::Url;

/// DV_MULTIMEDIA externalization settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct MultimediaConfig {
    /// Master switch. `false` (default) = today's inline behaviour, byte for
    /// byte; no blob store is built or contacted.
    pub enabled: bool,
    /// A decoded `DV_MULTIMEDIA.data` strictly larger than this is offloaded
    /// to the blob store; at or below it stays inline. Default `"256KiB"`.
    pub threshold: ByteSize,
    /// The storage service the blobs live in. Default `"s3"`, the one service
    /// this build offers.
    pub backend: Backend,
    /// The most requests to the blob store in flight at once, across every
    /// request this server serves. Default `64`.
    pub max_concurrent_requests: usize,
    /// How long one request to the blob store, or one chunk of its body, may
    /// take before it is retried or fails. Default `"30s"`.
    pub request_timeout: Duration,
    /// The part size of a multipart upload and of a ranged read: a blob larger
    /// than this is uploaded in parts. Default `"8MiB"`.
    pub part_size: ByteSize,
    /// How many parts of one blob move at once. Default `4`.
    pub part_concurrency: usize,
    /// How old an unreferenced blob must be before `ferroehr db sweep-blobs`
    /// deletes it. A write of bytes already stored rewrites a blob older than
    /// half of this, so a blob a commit is about to reference is never that
    /// old. Keep it well above the longest commit. Default `"1h"`.
    pub orphan_grace: Duration,
    /// The S3 backend's settings (`[record.multimedia.s3]`).
    pub s3: S3Config,
}

impl Default for MultimediaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: ByteSize::from_kib(256),
            backend: Backend::S3,
            max_concurrent_requests: 64,
            request_timeout: Duration::from_secs(30),
            part_size: ByteSize::from_mib(8),
            part_concurrency: 4,
            orphan_grace: Duration::from_hours(1),
            s3: S3Config::default(),
        }
    }
}

/// The storage service `[record.multimedia]` stores blobs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// An S3-compatible object store (AWS S3, MinIO, SeaweedFS and others),
    /// configured by `[record.multimedia.s3]`.
    S3,
}

/// `[record.multimedia.s3]`: how the S3 backend is reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct S3Config {
    /// S3-compatible endpoint URL (e.g. a SeaweedFS S3 gateway in dev/test, or
    /// a MinIO endpoint). `None` uses the AWS endpoint of `region`. Any
    /// `userinfo` in it is masked in every rendering and never sent.
    /// An `http://` URL is plain HTTP, which a dev SeaweedFS speaks; a
    /// `production` deployment names it as a gap.
    pub url: Option<SecretUrl>,
    /// Target bucket for content-addressed blobs.
    pub bucket: String,
    /// AWS region (S3 requires one even for non-AWS endpoints).
    pub region: String,
    /// Access key id. `None` (with `secret_access_key` also `None`) sends the
    /// requests unsigned, the mode a keyless dev SeaweedFS accepts.
    pub access_key_id: Option<String>,
    /// Secret access key (paired with `access_key_id`); never rendered.
    pub secret_access_key: Option<Secret>,
    /// File-based indirection for [`Self::secret_access_key`] (K8s/Docker
    /// secrets). Exactly one of the pair may be set; the loader reads and trims
    /// the file.
    pub secret_access_key_file: Option<PathBuf>,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            url: None,
            bucket: "openehr-multimedia".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: None,
            secret_access_key: None,
            secret_access_key_file: None,
        }
    }
}

impl MultimediaConfig {
    /// Whether the configuration names a store at all: externalization on,
    /// or an endpoint left in place to read already-offloaded blobs back.
    #[must_use]
    pub fn names_a_store(&self) -> bool {
        self.enabled || self.s3.url.is_some()
    }

    /// The bucket the configured backend stores blobs in.
    #[must_use]
    pub fn bucket(&self) -> &str {
        match self.backend {
            Backend::S3 => &self.s3.bucket,
        }
    }

    /// Whether the store is reached over plain HTTP: an `http://` S3 `url`,
    /// which a dev SeaweedFS speaks.
    #[must_use]
    pub fn allows_http(&self) -> bool {
        match self.backend {
            Backend::S3 => self
                .s3
                .url
                .as_ref()
                .is_some_and(|url| url.expose().trim_start().starts_with("http://")),
        }
    }
}

/// Why [`S3Config::endpoint_url`] refused `record.multimedia.s3.url`.
///
/// Every message quotes the endpoint only through [`mask_userinfo`], so no
/// refusal carries a credential.
#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    /// The key is set to a blank string.
    #[error(
        "record.multimedia.s3.url is set but empty — give an absolute URL \
         (e.g. http://seaweedfs:8333) or remove the key to use default \
         AWS endpoint resolution"
    )]
    Empty,
    /// The value does not parse as an absolute URL.
    #[error("record.multimedia.s3.url {shown:?} is not an absolute URL: {reason}")]
    NotAbsolute {
        /// The endpoint, `userinfo` masked.
        shown: String,
        /// The parse failure.
        reason: url::ParseError,
    },
    /// The value parses, but its scheme is neither `http` nor `https`.
    #[error(
        "record.multimedia.s3.url {shown:?} has scheme {scheme:?} — an S3 endpoint \
         must be http or https (did you mean \"http://{shown}\"?)"
    )]
    NotHttp {
        /// The endpoint, `userinfo` masked.
        shown: String,
        /// The scheme found.
        scheme: String,
    },
}

impl S3Config {
    /// Whether the client sends unsigned requests (no credentials given).
    #[must_use]
    pub fn is_anonymous(&self) -> bool {
        self.access_key_id.is_none() && self.secret_access_key.is_none()
    }

    /// Parses [`Self::url`] into the absolute `http`/`https` URL the S3
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
        let Some(endpoint) = &self.url else {
            return Ok(None);
        };
        let trimmed = endpoint.expose().trim();
        if trimmed.is_empty() {
            return Err(EndpointError::Empty);
        }
        let url = Url::parse(trimmed).map_err(|reason| EndpointError::NotAbsolute {
            shown: mask_userinfo(trimmed),
            reason,
        })?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(EndpointError::NotHttp {
                shown: mask_userinfo(trimmed),
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
        let with = |endpoint: &str| S3Config {
            url: Some(SecretUrl::new(endpoint)),
            ..S3Config::default()
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
            S3Config::default().endpoint_url(),
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
        assert_eq!(c.threshold, ByteSize::from_kib(256));
        assert_eq!(c.backend, Backend::S3);
        assert_eq!(c.s3.bucket, "openehr-multimedia");
        assert!(c.s3.is_anonymous());
        assert!(!c.names_a_store());
    }
}
