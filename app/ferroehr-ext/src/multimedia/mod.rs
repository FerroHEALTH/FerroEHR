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
//! seam and calls it on the commit and read paths. `blob` is the
//! content-addressed [`BlobStore`] over one `OpenDAL` operator, `offload` the
//! canonical-JSON transforms.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): external FHIR resources, tenancy/event CRUD rows, \
              multimedia offload over stored fragments (families 3/6/8)"
)]

use std::collections::BTreeMap;

use serde_json::Value;

pub mod blob;
mod offload;

use crate::multimedia::blob::{BlobError, BlobKey, BlobStore, BlobStoreParams};
use crate::multimedia::offload::{Base64Text, DV_MULTIMEDIA, Fetched};

/// A failure in the multimedia externalization path.
#[derive(Debug, thiserror::Error)]
pub enum MultimediaError {
    /// The blob store failed an operation; the error names the operation and
    /// the key, with the backend's error as its source.
    #[error(transparent)]
    Store(#[from] BlobError),
    /// A blob's bytes do not hash to the key that names them: the integrity
    /// check failed, and nothing was stored or inlined.
    #[error("multimedia integrity check failed: blob {expected} holds bytes that hash to {actual}")]
    Integrity {
        /// The key, the SHA-256 the bytes should have.
        expected: BlobKey,
        /// The SHA-256 of the bytes actually seen.
        actual: BlobKey,
    },
    /// A blob is not the length its record declares (`DV_MULTIMEDIA.size`,
    /// RM `Size_valid`), and nothing was stored or inlined.
    #[error("multimedia blob {key} is {actual} bytes, but its record declares {declared}")]
    Size {
        /// The blob.
        key: BlobKey,
        /// The length the record or the caller declared.
        declared: u64,
        /// The length of the bytes actually seen.
        actual: u64,
    },
    /// A `DV_MULTIMEDIA` value was structurally malformed (e.g. `data` not
    /// base64, or size out of range).
    #[error("malformed multimedia value: {0}")]
    Malformed(String),
    /// The blob store could not be built from its configuration.
    #[error("multimedia store configuration: {0}")]
    Config(String),
    /// The blob store's HTTP client or backend would not build, with the cause
    /// as the source (RFC 0201).
    #[error("the multimedia store client could not be built from its configuration")]
    ConfigFailed(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The platform's validation refused the configured endpoint; the cause is
    /// the platform's own error type, and its message masks any `userinfo`.
    #[error("the multimedia store endpoint is not valid")]
    Endpoint(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A commit reached its transaction with an inline value that should have
    /// been externalized before it: a write path skipped
    /// [`MultimediaEngine::offload`].
    #[error(
        "a DV_MULTIMEDIA value above the offload threshold reached the commit transaction \
         inline; externalization must run before the transaction opens"
    )]
    NotExternalized,
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
    /// Build the engine from the store's parameters and the offload threshold.
    ///
    /// # Errors
    /// [`MultimediaError::Config`] or [`MultimediaError::ConfigFailed`] when
    /// the blob store cannot be built.
    pub fn from_params(params: BlobStoreParams, threshold: usize) -> Result<Self, MultimediaError> {
        Ok(Self::from_parts(BlobStore::from_params(params)?, threshold))
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

    /// The engine over an already-built store, with offload on.
    #[must_use]
    pub const fn from_parts(store: BlobStore, threshold: usize) -> Self {
        Self {
            store,
            threshold,
            offload_enabled: true,
        }
    }

    /// The underlying content-addressed blob store.
    #[must_use]
    pub const fn store(&self) -> &BlobStore {
        &self.store
    }

    /// Externalize every qualifying inline `DV_MULTIMEDIA` in `canonical`,
    /// uploading the blobs. Rewrites `canonical` in place; a below-threshold or
    /// already-external value is left untouched.
    ///
    /// Runs before the commit's transaction opens, so no database connection
    /// waits on an upload; a commit that fails after it leaves an unreferenced
    /// blob, which costs storage and never correctness.
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
        for blob in pending {
            self.store
                .write_if_absent(&blob.key, blob.size, offload::decoded_chunks(&blob.data))
                .await?;
        }
        Ok(())
    }

    /// Refuses a body that still holds a value [`Self::offload`] would have
    /// externalized: the commit transaction's check that externalization ran
    /// before it.
    ///
    /// # Errors
    /// [`MultimediaError::NotExternalized`] for such a body, and
    /// [`MultimediaError::Malformed`] when an inline `data` is not base64.
    pub fn ensure_externalized(&self, canonical: &Value) -> Result<(), MultimediaError> {
        if self.offload_enabled && offload::needs_offload(canonical, self.threshold)? {
            return Err(MultimediaError::NotExternalized);
        }
        Ok(())
    }

    /// Re-inline every externalized `DV_MULTIMEDIA` (of *our* blobs) in
    /// `canonical`, each blob streamed from the store and proven against its
    /// key and its declared size before anything is inlined. A value already
    /// inline, or referencing a foreign URI, is left as-is.
    ///
    /// # Errors
    /// [`MultimediaError::Integrity`] or [`MultimediaError::Size`] when a blob
    /// is not what its record says, or [`MultimediaError::Store`] when a
    /// referenced blob is missing or the backend fails.
    pub async fn expand(&self, canonical: &mut Value) -> Result<(), MultimediaError> {
        let keys = offload::collect_expand_keys(canonical, &self.store);
        if keys.is_empty() {
            return Ok(());
        }
        let mut fetched = BTreeMap::new();
        for key in keys {
            let mut text = Base64Text::default();
            let size = self
                .store
                .read_verified(&key, |chunk| {
                    text.push(chunk);
                    Ok::<(), MultimediaError>(())
                })
                .await?;
            fetched.insert(
                key,
                Fetched {
                    data: text.finish(),
                    size,
                },
            );
        }
        offload::apply_expand(canonical, &fetched, &self.store)
    }

    /// The keys of *our* externalized media referenced anywhere in
    /// `canonical`, sorted and each once (for GC and dump/load).
    #[must_use]
    pub fn referenced_keys(&self, canonical: &Value) -> Vec<BlobKey> {
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
