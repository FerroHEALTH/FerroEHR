// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Content-addressed blob store over `object_store`.
//!
//! **No openEHR spec governs this — our own design/extension.** Constructed
//! only when the platform enables externalization; off means nothing here is
//! ever built.
//!
//! Blobs are keyed by the lowercase hex SHA-256 of their (unencoded) bytes, so
//! identical media dedups naturally and a key is immutable — matching openEHR
//! version indelibility. The backend is any S3-compatible endpoint (SeaweedFS
//! in dev/test via its S3 gateway; AWS/MinIO/etc. in production).

#![expect(
    clippy::doc_markdown,
    reason = "product identifiers (SeaweedFS, object_store, …) read as prose in \
              this module's docs"
)]

use std::sync::Arc;

use bytes::Bytes;
use object_store::{ObjectStore, ObjectStoreExt, aws::AmazonS3Builder, path::Path};

use secrecy::{ExposeSecret as _, SecretString};

use super::MultimediaError;

/// Runtime connection parameters for the S3-compatible backend — supplied by
/// the platform's config glue (the serde config section stays in the
/// platform's one config tree).
#[derive(Debug)]
pub struct BlobStoreParams {
    /// S3-compatible endpoint URL; `None` uses default AWS resolution.
    pub endpoint: Option<String>,
    /// Target bucket for content-addressed blobs.
    pub bucket: String,
    /// AWS region (S3 requires one even for non-AWS endpoints).
    pub region: String,
    /// Access key id; `None` with no secret runs the client unsigned.
    pub access_key_id: Option<String>,
    /// Secret access key (paired with `access_key_id`); never rendered.
    pub secret_access_key: Option<SecretString>,
    /// Allow plain-HTTP endpoints (dev/test only).
    pub allow_http: bool,
}

/// The URI scheme our externalized `DV_MULTIMEDIA.uri` values use.
pub const URI_SCHEME: &str = "s3";

/// A content-addressed blob store: `put`/`get`/`delete`/`exists` keyed by the
/// hex SHA-256 of the blob's unencoded bytes.
#[derive(Clone)]
pub struct BlobStore {
    inner: Arc<dyn ObjectStore>,
    bucket: String,
}

impl std::fmt::Debug for BlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobStore")
            .field("bucket", &self.bucket)
            .finish_non_exhaustive()
    }
}

impl BlobStore {
    /// Build an S3-backed blob store from runtime parameters.
    ///
    /// A keyless parameter set runs the client unsigned — the mode a dev
    /// SeaweedFS accepts with no credentials configured.
    ///
    /// # Errors
    /// Returns [`MultimediaError::Config`] if the object_store builder rejects
    /// the settings.
    pub fn from_params(params: BlobStoreParams) -> Result<Self, MultimediaError> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&params.bucket)
            .with_region(&params.region)
            .with_allow_http(params.allow_http);
        if let Some(endpoint) = &params.endpoint {
            // Refused here rather than passed to the builder, which accepts it
            // and leaves `object_store` to panic on `RelativeUrlWithoutBase` at
            // the first request. `${VAR:-}` in a compose file and an empty Helm
            // value both produce exactly this.
            let endpoint = endpoint.trim();
            if endpoint.is_empty() {
                return Err(MultimediaError::Config(
                    "multimedia.endpoint is set but empty — give an absolute URL \
                     (e.g. http://seaweedfs:8333) or unset it to use default AWS \
                     endpoint resolution"
                        .to_owned(),
                ));
            }
            let parsed = url::Url::parse(endpoint).map_err(|e| {
                MultimediaError::Config(format!(
                    "multimedia.endpoint {endpoint:?} is not an absolute URL: {e}"
                ))
            })?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(MultimediaError::Config(format!(
                    "multimedia.endpoint {endpoint:?} has scheme {:?} — an S3 endpoint \
                     must be http or https",
                    parsed.scheme()
                )));
            }
            builder = builder.with_endpoint(endpoint);
        }
        match (&params.access_key_id, &params.secret_access_key) {
            (Some(id), Some(secret)) => {
                builder = builder
                    .with_access_key_id(id)
                    .with_secret_access_key(secret.expose_secret());
            }
            // No credentials → run unsigned/anonymous (dev SeaweedFS).
            _ => builder = builder.with_skip_signature(true),
        }
        let store = builder
            .build()
            .map_err(|e| MultimediaError::ConfigFailed(e.to_string(), e))?;
        Ok(Self {
            inner: Arc::new(store),
            bucket: params.bucket,
        })
    }

    /// Construct directly from an object store (test seam / non-S3 backends).
    #[must_use]
    pub fn from_parts(inner: Arc<dyn ObjectStore>, bucket: String) -> Self {
        Self { inner, bucket }
    }

    /// The configured bucket name.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The canonical externalized URI for a blob key: `s3://<bucket>/<hex>`.
    #[must_use]
    pub fn uri_for(&self, hex: &str) -> String {
        format!("{URI_SCHEME}://{}/{hex}", self.bucket)
    }

    /// If `uri` is one of *our* externalized URIs (`s3://<our-bucket>/<hex>`),
    /// return the blob key `<hex>`; otherwise `None` (a foreign/client-managed
    /// external reference we never fetch).
    #[must_use]
    pub fn key_from_uri<'a>(&self, uri: &'a str) -> Option<&'a str> {
        let prefix = format!("{URI_SCHEME}://{}/", self.bucket);
        uri.strip_prefix(&prefix)
            .filter(|k| !k.is_empty() && !k.contains('/'))
    }

    /// Store `bytes` under `hex` unless already present (content-addressed:
    /// identical bytes ⇒ identical key ⇒ the upload is a no-op).
    ///
    /// # Errors
    /// Returns [`MultimediaError::Store`] on a backend failure.
    pub async fn put_if_absent(&self, hex: &str, bytes: Vec<u8>) -> Result<(), MultimediaError> {
        if self.exists(hex).await? {
            return Ok(());
        }
        self.inner
            .put(&Path::from(hex.to_owned()), bytes.into())
            .await
            .map_err(MultimediaError::Store)?;
        Ok(())
    }

    /// Fetch the blob stored under `hex`.
    ///
    /// # Errors
    /// Returns [`MultimediaError::Store`] if the object is missing or the
    /// backend fails.
    pub async fn get(&self, hex: &str) -> Result<Bytes, MultimediaError> {
        let res = self
            .inner
            .get(&Path::from(hex.to_owned()))
            .await
            .map_err(MultimediaError::Store)?;
        res.bytes().await.map_err(MultimediaError::Store)
    }

    /// Delete the blob stored under `hex` (idempotent: deleting an absent key
    /// is not an error).
    ///
    /// # Errors
    /// Returns [`MultimediaError::Store`] on a backend failure other than
    /// not-found.
    pub async fn delete(&self, hex: &str) -> Result<(), MultimediaError> {
        match self.inner.delete(&Path::from(hex.to_owned())).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(MultimediaError::Store(e)),
        }
    }

    /// Whether a blob exists under `hex`.
    ///
    /// # Errors
    /// Returns [`MultimediaError::Store`] on a backend failure other than
    /// not-found.
    pub async fn exists(&self, hex: &str) -> Result<bool, MultimediaError> {
        match self.inner.head(&Path::from(hex.to_owned())).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(MultimediaError::Store(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::memory::InMemory;

    fn mem_store() -> BlobStore {
        BlobStore::from_parts(Arc::new(InMemory::new()), "test-bucket".to_owned())
    }

    #[test]
    fn uri_round_trips_to_key() {
        let s = mem_store();
        let hex = "abc123";
        let uri = s.uri_for(hex);
        assert_eq!(uri, "s3://test-bucket/abc123");
        assert_eq!(s.key_from_uri(&uri), Some("abc123"));
    }

    #[test]
    fn foreign_uri_is_not_our_key() {
        let s = mem_store();
        assert_eq!(s.key_from_uri("s3://other-bucket/abc"), None);
        assert_eq!(s.key_from_uri("https://example.org/img.png"), None);
        assert_eq!(s.key_from_uri("s3://test-bucket/nested/path"), None);
    }

    #[tokio::test]
    async fn put_get_exists_delete_round_trip() {
        let s = mem_store();
        assert!(!s.exists("k").await.unwrap());
        s.put_if_absent("k", b"hello".to_vec()).await.unwrap();
        assert!(s.exists("k").await.unwrap());
        assert_eq!(&*s.get("k").await.unwrap(), b"hello");
        // put_if_absent on an existing key is a no-op (no error).
        s.put_if_absent("k", b"hello".to_vec()).await.unwrap();
        s.delete("k").await.unwrap();
        assert!(!s.exists("k").await.unwrap());
        // delete of an absent key is idempotent.
        s.delete("k").await.unwrap();
    }

    /// A blank or scheme-less endpoint is a typed configuration error, never a
    /// panic on the first request (#2167). This is the second line of defence:
    /// the platform refuses it at boot, and this refuses it if it ever gets
    /// past that.
    #[test]
    fn a_blank_or_schemeless_endpoint_is_a_typed_error_not_a_panic() {
        for bad in ["", "   ", "seaweedfs:8333", "/bucket"] {
            let params = BlobStoreParams {
                endpoint: Some(bad.to_owned()),
                bucket: "b".to_owned(),
                region: "us-east-1".to_owned(),
                access_key_id: None,
                secret_access_key: None,
                allow_http: true,
            };
            let err = BlobStore::from_params(params)
                .err()
                .unwrap_or_else(|| panic!("endpoint {bad:?} must be refused"));
            assert!(
                matches!(err, MultimediaError::Config(_)),
                "endpoint {bad:?} must be a typed Config error, got {err:?}"
            );
        }
    }

    /// An absolute http(s) endpoint still builds, so the check above is not a
    /// blanket refusal.
    #[test]
    fn an_absolute_http_endpoint_still_builds() {
        let params = BlobStoreParams {
            endpoint: Some("http://seaweedfs:8333".to_owned()),
            bucket: "b".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: None,
            secret_access_key: None,
            allow_http: true,
        };
        assert!(BlobStore::from_params(params).is_ok());
    }
}
