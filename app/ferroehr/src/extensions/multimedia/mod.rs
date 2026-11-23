// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[record.multimedia]` config section + the feature-gated glue onto the
//! extension crate's externalization engine.
//!
//! **No openEHR spec governs this — our own design/extension.** The engine,
//! blob store, and canonical-JSON transforms live in
//! `ferroehr_ext::multimedia` (the `multimedia` cargo feature); this module
//! keeps the serde config section in the platform's one config tree and maps
//! it onto the extension's runtime parameters.

pub mod config;
#[cfg(feature = "multimedia")]
pub mod sweep;

#[cfg(feature = "multimedia")]
use ferroehr_ext::multimedia::blob::{BackendSettings, BlobStoreParams, Limits, S3Settings};
#[cfg(feature = "multimedia")]
use ferroehr_ext::multimedia::{MultimediaEngine, MultimediaError};
#[cfg(feature = "multimedia")]
use ferrohealth_config::units::ByteSize;

#[cfg(feature = "multimedia")]
use crate::extensions::multimedia::config::Backend;
use crate::extensions::multimedia::config::MultimediaConfig;

/// Builds the externalization engine from configuration.
///
/// The engine is built whenever a store is reachable, and `enabled` governs
/// only whether new content is externalized, so switching it off strands no
/// blob already offloaded. `None` means no store can be reached, and the read
/// path then refuses an expansion request loudly.
///
/// # Errors
/// Returns `MultimediaError::Endpoint` when
/// [`config::S3Config::endpoint_url`] refuses the endpoint, and
/// `MultimediaError::Config` or `ConfigFailed` when the blob store cannot be
/// built, or offload is enabled with a zero `orphan_grace`. A read-back-only store (disabled, but
/// an endpoint left configured) never fails the boot: turning an integration
/// off must not be able to stop the server starting, so an unbuildable client
/// there degrades to `None` — and the read path is loud about it per request.
#[cfg(feature = "multimedia")]
pub fn engine_from_config(
    cfg: &MultimediaConfig,
) -> Result<Option<MultimediaEngine>, MultimediaError> {
    if !cfg.names_a_store() {
        return Ok(None);
    }
    let backend = match cfg.backend {
        Backend::S3 => BackendSettings::S3(S3Settings {
            endpoint: cfg
                .s3
                .endpoint_url()
                .map_err(|e| MultimediaError::Endpoint(Box::new(e)))?,
            bucket: cfg.s3.bucket.clone(),
            region: cfg.s3.region.clone(),
            access_key_id: cfg.s3.access_key_id.clone(),
            secret_access_key: cfg
                .s3
                .secret_access_key
                .as_ref()
                .map(|s| secrecy::SecretString::from(s.expose().to_owned())),
            allow_http: cfg.allows_http(),
        }),
    };
    // NOTE: #5957, our own design: a zero grace lets the sweep take a blob whose commit still runs.
    if cfg.enabled && cfg.orphan_grace.get().is_zero() {
        return Err(MultimediaError::Config(String::from(
            "record.multimedia.orphan_grace = \"0s\" would let the orphan sweep delete a blob a \
             running commit has just uploaded; set it well above the longest commit",
        )));
    }
    let size = |what: &str, value: ByteSize| {
        usize::try_from(value.bytes()).map_err(|error| {
            MultimediaError::Config(format!(
                "record.multimedia.{what} = \"{value}\" is larger than this platform can address: \
                 {error}"
            ))
        })
    };
    let params = BlobStoreParams {
        backend,
        limits: Limits {
            max_concurrent_requests: cfg.max_concurrent_requests,
            request_timeout: cfg.request_timeout.get(),
            part_size: size("part_size", cfg.part_size)?,
            part_concurrency: cfg.part_concurrency,
            refresh_after: cfg.orphan_grace.get() / 2,
        },
    };
    let threshold = size("threshold", cfg.threshold)?;
    MultimediaEngine::from_params(params, threshold)
        .map(|engine| Some(engine.with_offload_enabled(cfg.enabled)))
}

/// The loud slim-build refusal: a configuration that enables externalization
/// on a binary compiled without the `multimedia` feature is a boot error,
/// never a silent ignore.
///
/// # Errors
/// The refusal message when `record.multimedia.enabled` is set.
#[cfg(not(feature = "multimedia"))]
pub fn require_disabled(cfg: &MultimediaConfig) -> Result<(), String> {
    if cfg.enabled {
        return Err(
            "record.multimedia.enabled = true, but this binary was built without the \
             `multimedia` cargo feature"
                .to_owned(),
        );
    }
    Ok(())
}
