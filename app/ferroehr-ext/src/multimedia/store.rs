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
    /// S3-compatible endpoint URL, already parsed by the platform's one
    /// validation of the key; `None` uses default AWS resolution.
    pub endpoint: Option<url::Url>,
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
    /// Returns [`MultimediaError::Config`] for an endpoint whose scheme is not
    /// `http`/`https`, and [`MultimediaError::ConfigFailed`] if the
    /// object_store builder rejects the settings. No message quotes the
    /// endpoint.
    pub fn from_params(params: BlobStoreParams) -> Result<Self, MultimediaError> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&params.bucket)
            .with_region(&params.region)
            .with_allow_http(params.allow_http);
        if let Some(endpoint) = &params.endpoint {
            // The scheme is judged again because the type does not carry it; the
            // refusal names the scheme alone, so it can never quote `userinfo`.
            if !matches!(endpoint.scheme(), "http" | "https") {
                return Err(MultimediaError::Config(format!(
                    "multimedia.endpoint has scheme {:?} — an S3 endpoint must be http or https",
                    endpoint.scheme()
                )));
            }
            builder = builder.with_endpoint(endpoint.as_str());
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

    /// Lists the key of every blob this store writes: each object at the
    /// bucket root whose name is a lowercase hex SHA-256 (64 characters).
    ///
    /// An object of any other name or under any prefix was not written by this
    /// store and is left out, so a bucket shared with other content lists only
    /// ours. The keys come back sorted.
    ///
    /// # Errors
    /// Returns [`MultimediaError::Store`] when the backend cannot list the
    /// bucket (unreachable, refused, or no such bucket).
    pub async fn content_keys(&self) -> Result<Vec<String>, MultimediaError> {
        let listing = self
            .inner
            .list_with_delimiter(None)
            .await
            .map_err(MultimediaError::Store)?;
        let mut keys: Vec<String> = listing
            .objects
            .into_iter()
            .map(|object| object.location.to_string())
            .filter(|key| is_content_key(key))
            .collect();
        keys.sort_unstable();
        Ok(keys)
    }

    /// Deletes every blob [`Self::content_keys`] lists and returns how many it
    /// deleted.
    ///
    /// Objects this store did not write are left in place. Deleting is
    /// idempotent per key, so a failure part-way leaves a store that a second
    /// call finishes.
    ///
    /// # Errors
    /// Returns [`MultimediaError::Store`] when the listing or a delete fails.
    pub async fn delete_all_content(&self) -> Result<usize, MultimediaError> {
        let keys = self.content_keys().await?;
        for key in &keys {
            self.delete(key).await?;
        }
        Ok(keys.len())
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

/// Whether `key` has the shape of the keys this store writes: the lowercase hex
/// SHA-256 of a blob, 64 characters, with no path separator.
fn is_content_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::memory::InMemory;

    #[tokio::test]
    async fn delete_all_content_removes_our_blobs_and_leaves_foreign_objects() {
        let inner: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let s = BlobStore::from_parts(Arc::clone(&inner), "test-bucket".to_owned());
        let ours = ["a".repeat(64), "0123456789abcdef".repeat(4)];
        for key in &ours {
            s.put_if_absent(key, b"blob".to_vec()).await.unwrap();
        }
        for foreign in ["README".to_owned(), format!("nested/{}", "b".repeat(64))] {
            inner
                .put(&Path::from(foreign.as_str()), b"x".to_vec().into())
                .await
                .unwrap();
        }
        let upper = "A".repeat(64);
        inner
            .put(&Path::from(upper.as_str()), b"x".to_vec().into())
            .await
            .unwrap();

        let mut expected = ours.to_vec();
        expected.sort_unstable();
        assert_eq!(s.content_keys().await.unwrap(), expected);
        assert_eq!(s.delete_all_content().await.unwrap(), 2);
        assert!(s.content_keys().await.unwrap().is_empty());
        assert!(inner.head(&Path::from("README")).await.is_ok());
        assert!(inner.head(&Path::from(upper.as_str())).await.is_ok());
        // A second run finds nothing to delete.
        assert_eq!(s.delete_all_content().await.unwrap(), 0);
    }

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

    /// A non-http(s) endpoint is a typed configuration error, never a panic on
    /// the first request (#2167), and the refusal never quotes the endpoint's
    /// `userinfo` (#3656). Blank and relative endpoints cannot reach the store:
    /// the platform's one validation refuses them before a `Url` exists.
    #[test]
    fn a_non_http_endpoint_is_a_typed_error_that_never_quotes_userinfo() {
        for bad in [
            "seaweedfs:8333",
            "ftp://user:hunter2@s3.example",
            "s3://user:hunter2@s3.example/bucket",
        ] {
            let params = BlobStoreParams {
                endpoint: Some(url::Url::parse(bad).expect("parses as a URL")),
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
            assert!(!err.to_string().contains("hunter2"), "{err}");
            assert!(!format!("{err:?}").contains("hunter2"), "{err:?}");
        }
    }

    /// An https endpoint carrying `userinfo` builds, and the store's `Debug`
    /// never prints the endpoint.
    #[test]
    fn an_endpoint_with_userinfo_builds_without_rendering_it() {
        let params = BlobStoreParams {
            endpoint: Some(url::Url::parse("https://user:hunter2@s3.example").expect("url")),
            bucket: "b".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: None,
            secret_access_key: None,
            allow_http: false,
        };
        let store = BlobStore::from_params(params).expect("an https endpoint builds");
        assert!(!format!("{store:?}").contains("hunter2"));
    }

    /// An absolute http(s) endpoint still builds, so the check above is not a
    /// blanket refusal.
    #[test]
    fn an_absolute_http_endpoint_still_builds() {
        let params = BlobStoreParams {
            endpoint: Some(url::Url::parse("http://seaweedfs:8333").expect("url")),
            bucket: "b".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: None,
            secret_access_key: None,
            allow_http: true,
        };
        assert!(BlobStore::from_params(params).is_ok());
    }
}
