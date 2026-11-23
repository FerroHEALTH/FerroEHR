// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The content-addressed blob store, over one shared OpenDAL [`Operator`].
//!
//! No openEHR spec governs where externalized `DV_MULTIMEDIA` bytes live: our
//! own design. A blob is keyed by the SHA-256 of its unencoded bytes, the value
//! its record carries as `integrity_check` (RM 1.2.0 `DV_MULTIMEDIA`, algorithm
//! `SHA-256`), so identical media is stored once and every read can prove what
//! it returns.
//!
//! Reads and writes stream, so memory follows the part size, not the blob size.

#![expect(
    clippy::doc_markdown,
    reason = "product identifiers (OpenDAL, MinIO, SeaweedFS, …) read as prose in \
              this module's docs"
)]

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::TryStreamExt as _;
use opendal::layers::{ConcurrentLimitLayer, RetryLayer, TimeoutLayer};
use opendal::services::{Memory, S3};
use opendal::{ErrorKind, HttpTransporter, OperationContext, Operator};
use opendal_http_transport_reqwest::ReqwestTransport;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tracing::Instrument as _;

use super::MultimediaError;

/// The URI scheme our externalized `DV_MULTIMEDIA.uri` values use.
pub const URI_SCHEME: &str = "s3";

/// The key of one blob: the SHA-256 of its unencoded bytes.
///
/// Rendered as 64 lowercase hex characters, which is the object name in the
/// bucket and the last segment of the record's `s3://<bucket>/<key>` URI.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobKey([u8; 32]);

impl BlobKey {
    /// The key of the bytes whose SHA-256 is `digest`.
    #[must_use]
    pub const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    /// The key of `bytes`, hashed in one pass.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// The SHA-256 digest the key is.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for BlobKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for BlobKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlobKey({self})")
    }
}

/// A string that is not 64 lowercase hex characters, so it names no blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a blob key is 64 lowercase hex characters (a SHA-256 digest)")]
pub struct NotABlobKey;

impl FromStr for BlobKey {
    type Err = NotABlobKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 64 {
            return Err(NotABlobKey);
        }
        let mut digest = [0u8; 32];
        let (pairs, _) = s.as_bytes().as_chunks::<2>();
        for (byte, [high, low]) in digest.iter_mut().zip(pairs) {
            *byte = (nibble(*high)? << 4) | nibble(*low)?;
        }
        Ok(Self(digest))
    }
}

/// The value of one lowercase hex digit.
fn nibble(digit: u8) -> Result<u8, NotABlobKey> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        _ => Err(NotABlobKey),
    }
}

/// The operation a [`BlobError`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobOp {
    /// Asking whether a blob exists.
    Exists,
    /// Writing a blob.
    Write,
    /// Reading a blob.
    Read,
    /// Deleting a blob.
    Delete,
    /// Listing the bucket.
    List,
}

impl BlobOp {
    /// The name the span and the error carry.
    const fn name(self) -> &'static str {
        match self {
            Self::Exists => "exists",
            Self::Write => "write",
            Self::Read => "read",
            Self::Delete => "delete",
            Self::List => "list",
        }
    }
}

impl fmt::Display for BlobOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A blob-store operation the backend failed, with the backend's error as its
/// source.
#[derive(Debug, thiserror::Error)]
#[error("the multimedia blob store failed to {op}{}", key.map(|k| format!(" blob {k}")).unwrap_or_default())]
pub struct BlobError {
    op: BlobOp,
    key: Option<BlobKey>,
    #[source]
    source: Box<opendal::Error>,
}

impl BlobError {
    /// The operation that failed.
    #[must_use]
    pub const fn op(&self) -> BlobOp {
        self.op
    }

    /// The blob the operation addressed; `None` for a listing.
    #[must_use]
    pub const fn key(&self) -> Option<BlobKey> {
        self.key
    }

    /// Whether the backend answered that no such blob exists.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        self.source.kind() == ErrorKind::NotFound
    }
}

/// How the S3 backend is reached; built by the platform from
/// `[record.multimedia.s3]`.
#[derive(Debug)]
pub struct S3Settings {
    /// The endpoint, already parsed by the platform's one validation of the
    /// key; `None` is the AWS endpoint of `region`.
    pub endpoint: Option<url::Url>,
    /// The bucket the blobs live in.
    pub bucket: String,
    /// The signing region (S3 requires one even for non-AWS endpoints).
    pub region: String,
    /// The access key id; `None` with no secret sends requests unsigned.
    pub access_key_id: Option<String>,
    /// The secret access key, paired with `access_key_id`; never rendered.
    pub secret_access_key: Option<SecretString>,
    /// Whether a plain-HTTP endpoint is accepted (dev and test).
    pub allow_http: bool,
}

/// The storage backend, one variant per service the build offers.
#[derive(Debug)]
pub enum BackendSettings {
    /// An S3-compatible object store (AWS S3, MinIO, SeaweedFS and others).
    S3(S3Settings),
}

/// How hard the store may drive its backend.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The most backend requests in flight at once, across all callers.
    pub max_concurrent_requests: usize,
    /// How long one backend request, or one chunk of its body, may take.
    pub request_timeout: Duration,
    /// The size of one multipart part on write and one ranged chunk on read;
    /// a blob larger than this is written as a multipart upload.
    pub part_size: usize,
    /// How many parts of one blob move at once.
    pub part_concurrency: usize,
    /// How old an existing blob may be before a write of the same bytes
    /// rewrites it instead of skipping, which renews its age. Half the orphan
    /// sweep's grace period, so a blob a commit is about to reference is never
    /// old enough for the sweep to take.
    pub refresh_after: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_concurrent_requests: 64,
            request_timeout: Duration::from_secs(30),
            part_size: 8 * 1024 * 1024,
            part_concurrency: 4,
            refresh_after: Duration::from_mins(30),
        }
    }
}

/// Everything [`BlobStore::from_params`] needs.
#[derive(Debug)]
pub struct BlobStoreParams {
    /// The backend and how to reach it.
    pub backend: BackendSettings,
    /// The request, timeout and part limits.
    pub limits: Limits,
}

/// How long establishing a connection to the backend may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How many times a request that failed with an error OpenDAL marks temporary
/// is sent again, with jittered exponential backoff, before its failure is
/// reported.
const RETRIES: usize = 2;

/// The content-addressed blob store: one [`Operator`] shared by every request,
/// with the bucket name our URIs carry.
#[derive(Clone)]
pub struct BlobStore {
    op: Operator,
    bucket: String,
    backend: &'static str,
    limits: Limits,
}

impl fmt::Debug for BlobStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlobStore")
            .field("backend", &self.backend)
            .field("bucket", &self.bucket)
            .finish_non_exhaustive()
    }
}

impl BlobStore {
    /// Builds the store and its one [`Operator`]: the backend, then the
    /// concurrency limit, the per-request timeout and the bounded retry, with
    /// the requests sent through one pooled `reqwest` client.
    ///
    /// The S3 backend reads no environment variable, profile file or instance
    /// metadata: its credentials come from the configuration or it runs
    /// unsigned, the mode a keyless dev SeaweedFS accepts.
    ///
    /// # Errors
    /// [`MultimediaError::Config`] for an endpoint whose scheme is not `http`
    /// or `https`; [`MultimediaError::ConfigFailed`] when the HTTP client or
    /// the backend cannot be built. No message quotes the endpoint.
    pub fn from_params(params: BlobStoreParams) -> Result<Self, MultimediaError> {
        let BackendSettings::S3(s3) = params.backend;
        let mut builder = S3::default()
            .bucket(&s3.bucket)
            .region(&s3.region)
            .disable_config_load()
            .disable_ec2_metadata();
        let endpoint = match s3.endpoint {
            Some(mut endpoint) => {
                // The scheme is judged again because the type does not carry it; the
                // refusal names the scheme alone, so it can never quote `userinfo`.
                if !matches!(endpoint.scheme(), "http" | "https") {
                    return Err(MultimediaError::Config(format!(
                        "record.multimedia.s3.url has scheme {:?}: an S3 endpoint must be http or https",
                        endpoint.scheme()
                    )));
                }
                // NOTE: a URL whose scheme is http(s) always takes empty userinfo, so both results are Ok.
                let _cleared = (endpoint.set_username(""), endpoint.set_password(None));
                endpoint.to_string()
            }
            None => format!("https://s3.{}.amazonaws.com", s3.region),
        };
        builder = builder.endpoint(&endpoint);
        builder = match (&s3.access_key_id, &s3.secret_access_key) {
            (Some(id), Some(secret)) => builder
                .access_key_id(id)
                .secret_access_key(secret.expose_secret()),
            _ => builder.skip_signature(),
        };
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .https_only(!s3.allow_http)
            .build()
            .map_err(|e| MultimediaError::ConfigFailed(Box::new(e.without_url())))?;
        let op = Operator::new(builder)
            .map_err(|e| MultimediaError::ConfigFailed(Box::new(e)))?
            .with_context(
                OperationContext::new()
                    .with_http_transport(HttpTransporter::new(ReqwestTransport::new(http))),
            );
        Ok(Self::layered(op, s3.bucket, "s3", params.limits))
    }

    /// A store held in this process's memory, for tests.
    ///
    /// # Panics
    /// Never: the memory service takes no configuration that can fail.
    #[must_use]
    pub fn in_memory(bucket: impl Into<String>) -> Self {
        #[expect(
            clippy::expect_used,
            reason = "Memory::default() has no field a build can refuse; OpenDAL's own \
                      examples build it the same way"
        )]
        let op = Operator::new(Memory::default()).expect("the memory service should always build");
        Self::layered(op, bucket.into(), "memory", Limits::default())
    }

    /// Puts the limit, timeout and retry layers on `op`; the retry is outermost,
    /// so every attempt has its own timeout and its own concurrency permit.
    fn layered(op: Operator, bucket: String, backend: &'static str, limits: Limits) -> Self {
        let op = op
            .layer(ConcurrentLimitLayer::new(limits.max_concurrent_requests))
            .layer(
                TimeoutLayer::new()
                    .with_timeout(limits.request_timeout)
                    .with_io_timeout(limits.request_timeout),
            )
            .layer(RetryLayer::new().with_max_times(RETRIES).with_jitter());
        Self {
            op,
            bucket,
            backend,
            limits,
        }
    }

    /// The bucket our URIs name.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The canonical externalized URI of a blob: `s3://<bucket>/<key>`.
    #[must_use]
    pub fn uri_for(&self, key: &BlobKey) -> String {
        format!("{URI_SCHEME}://{}/{key}", self.bucket)
    }

    /// The key of one of *our* externalized URIs (`s3://<our-bucket>/<key>`);
    /// `None` for any other URI, a foreign reference we never fetch.
    #[must_use]
    pub fn key_from_uri(&self, uri: &str) -> Option<BlobKey> {
        uri.strip_prefix(URI_SCHEME)
            .and_then(|rest| rest.strip_prefix("://"))
            .and_then(|rest| rest.strip_prefix(self.bucket.as_str()))
            .and_then(|rest| rest.strip_prefix('/'))
            // NOTE: a last segment that is not a digest names a blob we never minted, which is absence.
            .and_then(|name| name.parse().ok())
    }

    /// The span one operation runs in.
    fn span(&self, op: BlobOp, key: Option<&BlobKey>) -> tracing::Span {
        tracing::info_span!(
            "multimedia.blob",
            op = op.name(),
            backend = self.backend,
            key = key.map(tracing::field::display),
            bytes = tracing::field::Empty,
            elapsed_ms = tracing::field::Empty,
        )
    }

    /// Whether a blob exists under `key`.
    ///
    /// # Errors
    /// [`BlobError`] when the backend cannot answer.
    pub async fn exists(&self, key: &BlobKey) -> Result<bool, BlobError> {
        let span = self.span(BlobOp::Exists, Some(key));
        let started = Instant::now();
        let answer = self
            .op
            .exists(&key.to_string())
            .instrument(span.clone())
            .await
            .map_err(|source| BlobError {
                op: BlobOp::Exists,
                key: Some(*key),
                source: Box::new(source),
            });
        span.record("elapsed_ms", elapsed_ms(started));
        answer
    }

    /// Writes the blob `key` names from `chunks` unless it already exists
    /// and is younger than [`Limits::refresh_after`], checking while it writes
    /// that the bytes are `size` long and hash to `key`.
    ///
    /// An existing blob older than that is written again, which renews its
    /// age, so the orphan sweep never takes a blob a commit is about to
    /// reference by deduplication. A backend that records no write time keeps
    /// what it has.
    ///
    /// A blob larger than the part size goes up as a multipart upload with
    /// parts sent concurrently. A length or digest mismatch, or a failing
    /// chunk, aborts the upload before it completes, so nothing is stored
    /// under `key` unless its bytes are the ones `key` names.
    ///
    /// # Errors
    /// [`MultimediaError::Store`] on a backend failure;
    /// [`MultimediaError::Integrity`] or [`MultimediaError::Size`] when the
    /// bytes are not the ones `key` and `size` describe; the error a chunk
    /// carries.
    pub async fn write_if_absent<I>(
        &self,
        key: &BlobKey,
        size: u64,
        chunks: I,
    ) -> Result<(), MultimediaError>
    where
        I: IntoIterator<Item = Result<Bytes, MultimediaError>>,
    {
        if self.fresh(key).await? {
            return Ok(());
        }
        let span = self.span(BlobOp::Write, Some(key));
        let started = Instant::now();
        let written = self
            .write_verified(key, size, chunks)
            .instrument(span.clone())
            .await;
        span.record("bytes", size);
        span.record("elapsed_ms", elapsed_ms(started));
        written
    }

    /// Whether a blob exists under `key` and was written within
    /// [`Limits::refresh_after`] (or its backend records no write time).
    async fn fresh(&self, key: &BlobKey) -> Result<bool, BlobError> {
        let span = self.span(BlobOp::Exists, Some(key));
        let started = Instant::now();
        let stat = self
            .op
            .stat(&key.to_string())
            .instrument(span.clone())
            .await;
        span.record("elapsed_ms", elapsed_ms(started));
        match stat {
            Ok(meta) => Ok(meta.last_modified().is_none_or(|written| {
                !older_than(written.into_inner(), self.limits.refresh_after)
            })),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(false),
            Err(source) => Err(BlobError {
                op: BlobOp::Exists,
                key: Some(*key),
                source: Box::new(source),
            }),
        }
    }

    /// The body of [`Self::write_if_absent`], inside its span.
    async fn write_verified<I>(
        &self,
        key: &BlobKey,
        size: u64,
        chunks: I,
    ) -> Result<(), MultimediaError>
    where
        I: IntoIterator<Item = Result<Bytes, MultimediaError>>,
    {
        let failed = |source| BlobError {
            op: BlobOp::Write,
            key: Some(*key),
            source: Box::new(source),
        };
        let mut writer = self
            .op
            .writer_with(&key.to_string())
            .chunk(self.limits.part_size)
            .concurrent(self.limits.part_concurrency)
            .await
            .map_err(failed)?;
        let mut hasher = Sha256::new();
        let mut length: u64 = 0;
        for chunk in chunks {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => return Err(abort(&mut writer, e, failed).await),
            };
            hasher.update(&chunk);
            length = length.saturating_add(len64(chunk.len()));
            if let Err(e) = writer.write(chunk).await {
                return Err(abort(&mut writer, failed(e).into(), failed).await);
            }
        }
        let actual = BlobKey::from_digest(hasher.finalize().into());
        if length != size {
            let refused = MultimediaError::Size {
                key: *key,
                declared: size,
                actual: length,
            };
            return Err(abort(&mut writer, refused, failed).await);
        }
        if actual != *key {
            let refused = MultimediaError::Integrity {
                expected: *key,
                actual,
            };
            return Err(abort(&mut writer, refused, failed).await);
        }
        writer.close().await.map_err(failed)?;
        Ok(())
    }

    /// Reads the blob `key` names into `sink` chunk by chunk, hashing and
    /// counting on the way, and returns its length once the digest is proven.
    ///
    /// `sink` sees bytes before the digest is known; a caller that must never
    /// act on unproven bytes keeps them until this returns `Ok` and discards
    /// them on `Err`.
    ///
    /// # Errors
    /// [`MultimediaError::Store`] when the blob is missing or the backend
    /// fails; [`MultimediaError::Integrity`] when the bytes do not hash to
    /// `key`, each converted into `E`; the error `sink` returns.
    pub async fn read_verified<F, E>(&self, key: &BlobKey, sink: F) -> Result<u64, E>
    where
        F: FnMut(&[u8]) -> Result<(), E>,
        E: From<MultimediaError>,
    {
        let span = self.span(BlobOp::Read, Some(key));
        let started = Instant::now();
        let read = self.read_into(key, sink).instrument(span.clone()).await;
        if let Ok(length) = &read {
            span.record("bytes", length);
        }
        span.record("elapsed_ms", elapsed_ms(started));
        read
    }

    /// The body of [`Self::read_verified`], inside its span.
    async fn read_into<F, E>(&self, key: &BlobKey, mut sink: F) -> Result<u64, E>
    where
        F: FnMut(&[u8]) -> Result<(), E>,
        E: From<MultimediaError>,
    {
        let failed = |source| {
            E::from(MultimediaError::Store(BlobError {
                op: BlobOp::Read,
                key: Some(*key),
                source: Box::new(source),
            }))
        };
        let reader = self
            .op
            .reader_with(&key.to_string())
            .chunk(self.limits.part_size)
            .concurrent(self.limits.part_concurrency)
            .await
            .map_err(failed)?;
        let mut stream = reader.into_bytes_stream(..).await.map_err(failed)?;
        let mut hasher = Sha256::new();
        let mut length: u64 = 0;
        while let Some(chunk) = stream.try_next().await.map_err(|e| {
            failed(opendal::Error::new(ErrorKind::Unexpected, "read a blob chunk").set_source(e))
        })? {
            hasher.update(&chunk);
            length = length.saturating_add(len64(chunk.len()));
            sink(&chunk)?;
        }
        let actual = BlobKey::from_digest(hasher.finalize().into());
        if actual != *key {
            return Err(E::from(MultimediaError::Integrity {
                expected: *key,
                actual,
            }));
        }
        Ok(length)
    }

    /// Deletes the blob `key` names; deleting an absent blob succeeds.
    ///
    /// # Errors
    /// [`BlobError`] on a backend failure other than not-found.
    pub async fn delete(&self, key: &BlobKey) -> Result<(), BlobError> {
        let span = self.span(BlobOp::Delete, Some(key));
        let started = Instant::now();
        let deleted = match self
            .op
            .delete(&key.to_string())
            .instrument(span.clone())
            .await
        {
            Err(source) if source.kind() != ErrorKind::NotFound => Err(BlobError {
                op: BlobOp::Delete,
                key: Some(*key),
                source: Box::new(source),
            }),
            Ok(()) | Err(_) => Ok(()),
        };
        span.record("elapsed_ms", elapsed_ms(started));
        deleted
    }

    /// The key of every blob this store writes, sorted: each object at the
    /// bucket root whose name is a SHA-256 digest.
    ///
    /// An object of any other name, or under any prefix, was not written by
    /// this store and is left out, so a bucket shared with other content lists
    /// only ours.
    ///
    /// # Errors
    /// [`BlobError`] when the backend cannot list the bucket.
    pub async fn content_keys(&self) -> Result<Vec<BlobKey>, BlobError> {
        let listing = self.content_listing().await?;
        Ok(listing.into_iter().map(|(key, _)| key).collect())
    }

    /// [`Self::content_keys`] with each blob's last write time, where the
    /// backend records one.
    ///
    /// # Errors
    /// [`BlobError`] when the backend cannot list the bucket.
    pub async fn content_listing(
        &self,
    ) -> Result<Vec<(BlobKey, Option<jiff::Timestamp>)>, BlobError> {
        let span = self.span(BlobOp::List, None);
        let started = Instant::now();
        let listed = self.op.list("/").instrument(span.clone()).await;
        span.record("elapsed_ms", elapsed_ms(started));
        let entries = listed.map_err(|source| BlobError {
            op: BlobOp::List,
            key: None,
            source: Box::new(source),
        })?;
        let mut keys: Vec<(BlobKey, Option<jiff::Timestamp>)> = entries
            .iter()
            .filter(|entry| entry.metadata().mode().is_file())
            // NOTE: an object whose name is not a digest is someone else's, which is absence from our set.
            .filter_map(|entry| {
                let key = entry.path().parse().ok()?;
                Some((
                    key,
                    entry
                        .metadata()
                        .last_modified()
                        .map(opendal::raw::Timestamp::into_inner),
                ))
            })
            .collect();
        keys.sort_unstable_by_key(|(key, _)| *key);
        Ok(keys)
    }

    /// Deletes every blob [`Self::content_keys`] lists and returns how many.
    ///
    /// Objects this store did not write are left in place. Deleting is
    /// idempotent per key, so a failure part-way leaves a store a second call
    /// finishes.
    ///
    /// # Errors
    /// [`BlobError`] when the listing or a delete fails.
    pub async fn delete_all_content(&self) -> Result<usize, BlobError> {
        let keys = self.content_keys().await?;
        for key in &keys {
            self.delete(key).await?;
        }
        Ok(keys.len())
    }
}

/// Aborts `writer` after `cause`, so no partial upload completes, and returns
/// `cause`; an abort failure is folded in as the store error it is.
async fn abort(
    writer: &mut opendal::Writer,
    cause: MultimediaError,
    failed: impl Fn(opendal::Error) -> BlobError,
) -> MultimediaError {
    match writer.abort().await {
        Ok(()) => cause,
        Err(e) => {
            tracing::warn!(error = %e, cause = %cause, "a refused blob upload could not be aborted");
            MultimediaError::Store(failed(e))
        }
    }
}

/// Whether `written` lies more than `age` before now. A write time in the
/// future (clock skew) is not old.
#[must_use]
pub fn older_than(written: jiff::Timestamp, age: Duration) -> bool {
    let elapsed = jiff::Timestamp::now().duration_since(written);
    jiff::SignedDuration::try_from(age).is_ok_and(|age| elapsed > age)
}

/// A byte count as `u64`; saturates on a platform whose `usize` is wider.
pub(super) fn len64(n: usize) -> u64 {
    // NOTE: usize is at most 64 bits on every target Rust supports today, so this never saturates there.
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Milliseconds since `started`, for a span field.
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
