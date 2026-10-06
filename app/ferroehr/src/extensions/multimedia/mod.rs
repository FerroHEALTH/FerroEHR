// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[multimedia]` config section + the feature-gated glue onto the
//! extension crate's externalization engine.
//!
//! **No openEHR spec governs this — our own design/extension.** The engine,
//! blob store, and canonical-JSON transforms live in
//! `ferroehr_ext::multimedia` (the `multimedia` cargo feature); this module
//! keeps the serde config section in the platform's one config tree and maps
//! it onto the extension's runtime parameters.

pub mod config;

use crate::extensions::multimedia::config::MultimediaConfig;

/// Builds the externalization engine from configuration.
///
/// The engine is built whenever a store is REACHABLE — `multimedia.enabled`,
/// or an explicit `endpoint` — and `enabled` then governs only whether NEW
/// content is externalized. Switching the integration off therefore stops
/// offloading without stranding the blobs already offloaded: a stored record
/// referencing one is clinical content this server put there, and a read that
/// asks for it back must not silently answer with the reference instead.
///
/// `None` means no store can be reached at all, which is the one case where an
/// expansion request cannot be honoured; the read path refuses loudly there
/// rather than quietly serving the compact form.
///
/// # Errors
/// Returns `MultimediaError::Config` when the integration is ENABLED and its
/// object-store client cannot be built. A read-back-only store (disabled, but
/// an endpoint left configured) never fails the boot: turning an integration
/// off must not be able to stop the server starting, so an unbuildable client
/// there degrades to `None` — and the read path is loud about it per request.
#[cfg(feature = "multimedia")]
pub fn engine_from_config(
    cfg: &MultimediaConfig,
) -> Result<
    Option<ferroehr_ext::multimedia::MultimediaEngine>,
    ferroehr_ext::multimedia::MultimediaError,
> {
    if !cfg.enabled && cfg.endpoint.is_none() {
        return Ok(None);
    }
    let params = ferroehr_ext::multimedia::store::BlobStoreParams {
        endpoint: cfg.endpoint.as_ref().map(|e| e.expose().to_owned()),
        bucket: cfg.bucket.clone(),
        region: cfg.region.clone(),
        access_key_id: cfg.access_key_id.clone(),
        secret_access_key: cfg
            .secret_access_key
            .as_ref()
            .map(|s| secrecy::SecretString::from(s.expose().to_owned())),
        allow_http: cfg.allow_http,
    };
    ferroehr_ext::multimedia::MultimediaEngine::from_params(params, cfg.threshold_bytes)
        .map(|engine| Some(engine.with_offload_enabled(cfg.enabled)))
}

/// The loud slim-build refusal: a configuration that enables externalization
/// on a binary compiled without the `multimedia` feature is a boot error,
/// never a silent ignore.
///
/// # Errors
/// The refusal message when `multimedia.enabled` is set.
#[cfg(not(feature = "multimedia"))]
pub fn require_disabled(cfg: &MultimediaConfig) -> Result<(), String> {
    if cfg.enabled {
        return Err(
            "multimedia.enabled = true, but this binary was built without the \
             `multimedia` cargo feature"
                .to_owned(),
        );
    }
    Ok(())
}
