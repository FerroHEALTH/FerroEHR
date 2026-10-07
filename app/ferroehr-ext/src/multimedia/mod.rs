// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `DV_MULTIMEDIA` externalization to S3-compatible object storage.
//!
//! No openEHR spec governs server-side blob storage — our own design/extension,
//! gated by the platform's `multimedia.enabled` switch and this crate's
//! `multimedia` cargo feature. Off by default: with the switch disabled nothing
//! here is constructed and the commit and read paths are byte-identical to
//! inline behaviour.
//!
//! On commit, an inline `DV_MULTIMEDIA.data` larger than a configured threshold
//! is written to a content-addressed blob store keyed by its SHA-256, and the
//! canonical JSON is rewritten to reference it by `uri`, carrying the RM
//! integrity fields and the mandatory unencoded `size`. On read it can be
//! re-inlined (`?expand_multimedia=true`), verifying the SHA-256 first. The data
//! shape it rewrites is RM 1.2.0 `DV_MULTIMEDIA`: `uri`/`data` alternatives
//! under `is_inline or is_external`, `integrity_check` implying
//! `integrity_check_algorithm`, and the mandatory unencoded `size`.
//!
//! This module owns only the engine and its transforms, built from
//! [`BlobStoreParams`]; the platform attaches it through its `with_multimedia`
//! seam and calls it on the commit and read paths. `store` is the
//! content-addressed [`BlobStore`] over `object_store`, `offload` the pure
//! canonical-JSON transforms.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): external FHIR resources, tenancy/event CRUD rows, \
              multimedia offload over stored fragments (families 3/6/8)"
)]

use std::collections::HashMap;

use serde_json::Value;

mod offload;
pub mod store;

use crate::multimedia::offload::DV_MULTIMEDIA;
use crate::multimedia::store::{BlobStore, BlobStoreParams};

/// A failure in the multimedia externalization path.
#[derive(Debug, thiserror::Error)]
pub enum MultimediaError {
    /// The object store rejected an operation.
    #[error("blob store: {0}")]
    Store(#[from] object_store::Error),
    /// A stored blob's bytes do not hash to their content-addressed key — the
    /// integrity check failed.
    #[error("multimedia integrity check failed: expected sha-256 {expected}, got {actual}")]
    Integrity {
        /// The expected SHA-256 (the blob key).
        expected: String,
        /// The SHA-256 actually computed over the fetched bytes.
        actual: String,
    },
    /// A `DV_MULTIMEDIA` value was structurally malformed (e.g. `data` not
    /// base64, or size out of range).
    #[error("malformed multimedia value: {0}")]
    Malformed(String),
    /// The object-store client could not be constructed from configuration.
    #[error("multimedia store configuration: {0}")]
    Config(String),
    /// The same failure with its cause intact (RFC 0201). A store that will not
    /// build is a credential problem, an endpoint problem or a TLS problem, and
    /// a caller that cannot tell them apart has to read prose.
    #[error("multimedia store configuration: {0}")]
    ConfigFailed(String, #[source] object_store::Error),
    /// The platform's validation refused the configured endpoint; the cause is
    /// the platform's own error type, and its message masks any `userinfo`.
    #[error("multimedia store configuration: {0}")]
    Endpoint(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Whether the tree references ANY externalized blob (`s3://…`), regardless of
/// bucket.
///
/// Deliberately bucket-blind, because its caller has no store to compare
/// against: it answers "would an expansion request have had work to do here?"
/// when no engine exists at all. A foreign `https://` reference is not
/// included — that was never expandable, so its presence is not a failure.
#[must_use]
pub fn references_external_blob(root: &Value) -> bool {
    match root {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some(DV_MULTIMEDIA)
                && map
                    .get("uri")
                    .and_then(|u| u.get("value"))
                    .and_then(Value::as_str)
                    .is_some_and(|uri| uri.starts_with("s3://"))
            {
                return true;
            }
            map.values().any(references_external_blob)
        }
        Value::Array(items) => items.iter().any(references_external_blob),
        _ => false,
    }
}

/// The runtime engine bundling the blob store with its offload threshold.
///
/// Held by the service whenever a store is REACHABLE, which is not the same as
/// "externalization is on": `offload_enabled` governs whether new blobs leave
/// the database, while the expand path stays available either way. A record
/// that already references a blob is clinical content this server put there,
/// and refusing to serve it back because a switch was flipped is data loss
/// dressed as a setting.
#[derive(Debug, Clone)]
pub struct MultimediaEngine {
    store: BlobStore,
    threshold: usize,
    offload_enabled: bool,
}

impl MultimediaEngine {
    /// Build the engine from runtime connection parameters + the offload
    /// threshold.
    ///
    /// # Errors
    /// Returns [`MultimediaError::Config`] if the object-store client cannot
    /// be built.
    pub fn from_params(params: BlobStoreParams, threshold: usize) -> Result<Self, MultimediaError> {
        Ok(Self {
            store: BlobStore::from_params(params)?,
            threshold,
            offload_enabled: true,
        })
    }

    /// Sets whether NEW content may be externalized. `false` leaves the expand
    /// path fully functional, which is what keeps already-offloaded records
    /// readable after the integration is switched off.
    #[must_use]
    pub const fn with_offload_enabled(mut self, enabled: bool) -> Self {
        self.offload_enabled = enabled;
        self
    }

    /// Whether new content may be externalized.
    #[must_use]
    pub const fn offload_enabled(&self) -> bool {
        self.offload_enabled
    }

    /// Construct directly from a store + threshold (test seam).
    #[must_use]
    pub fn from_parts(store: BlobStore, threshold: usize) -> Self {
        Self {
            store,
            threshold,
            offload_enabled: true,
        }
    }

    /// The underlying content-addressed blob store.
    #[must_use]
    pub fn store(&self) -> &BlobStore {
        &self.store
    }

    /// Externalize every qualifying inline `DV_MULTIMEDIA` in `canonical`,
    /// uploading the blobs. Rewrites `canonical` in place; a below-threshold or
    /// already-external value is left untouched.
    ///
    /// # Errors
    /// [`MultimediaError::Malformed`] when an inline `data` is not base64 or
    /// exceeds the i64 byte range, or [`MultimediaError::Store`] on an upload
    /// failure (the caller aborts the commit, so nothing is persisted on
    /// error).
    pub async fn offload(&self, canonical: &mut Value) -> Result<(), MultimediaError> {
        if !self.offload_enabled {
            return Ok(());
        }
        let pending = offload::plan_offload(canonical, self.threshold, &self.store)?;
        for (hex, bytes) in pending {
            self.store.put_if_absent(&hex, bytes).await?;
        }
        Ok(())
    }

    /// Re-inline every externalized `DV_MULTIMEDIA` (of *our* blobs) in
    /// `canonical`, verifying each blob's SHA-256 before use. A value already
    /// inline, or referencing a foreign URI, is left as-is.
    ///
    /// # Errors
    /// [`MultimediaError::Integrity`] on a hash mismatch, or
    /// [`MultimediaError::Store`] when a referenced blob is missing or the
    /// backend fails.
    pub async fn expand(&self, canonical: &mut Value) -> Result<(), MultimediaError> {
        let mut keys = offload::collect_expand_keys(canonical, &self.store);
        keys.sort_unstable();
        keys.dedup();
        if keys.is_empty() {
            return Ok(());
        }
        let mut fetched: HashMap<String, String> = HashMap::with_capacity(keys.len());
        for key in keys {
            let bytes = self.store.get(&key).await?;
            let b64_data = offload::verify_and_encode(&key, &bytes)?;
            fetched.insert(key, b64_data);
        }
        offload::apply_expand(canonical, &fetched, &self.store);
        Ok(())
    }

    /// The blob keys of *our* externalized media referenced anywhere in
    /// `canonical` (for GC and dump/load).
    #[must_use]
    pub fn referenced_keys(&self, canonical: &Value) -> Vec<String> {
        offload::referenced_keys(canonical, &self.store)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::multimedia::references_external_blob;

    /// The bucket-blind detector answers the question its one caller asks:
    /// "would an expansion request have had work to do here?"
    #[test]
    fn detects_only_externalized_references() {
        let inline = json!({
            "_type": "DV_MULTIMEDIA",
            "data": "AAEC",
            "size": 3
        });
        assert!(
            !references_external_blob(&inline),
            "inline content needs no expansion"
        );

        // A foreign URI was never expandable, so its presence is not a failure.
        let foreign = json!({
            "_type": "DV_MULTIMEDIA",
            "uri": {"_type": "DV_URI", "value": "https://example.test/scan.png"},
            "size": 3
        });
        assert!(!references_external_blob(&foreign));

        // Nested arbitrarily deep, because a composition is a tree.
        let external = json!({
            "_type": "COMPOSITION",
            "content": [{"items": [{
                "_type": "DV_MULTIMEDIA",
                "uri": {"_type": "DV_URI", "value": "s3://openehr-multimedia/abc123"},
                "size": 3
            }]}]
        });
        assert!(references_external_blob(&external));

        // A non-DV_MULTIMEDIA node carrying an s3 URI is not a blob reference.
        let decoy = json!({"_type": "DV_URI", "value": "s3://openehr-multimedia/abc123"});
        assert!(!references_external_blob(&decoy));
    }
}
