// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The blob store over the memory service, and the S3 configuration refusals.

use super::*;

fn key_of(bytes: &[u8]) -> BlobKey {
    BlobKey::of(bytes)
}

fn chunks(bytes: &[u8], size: usize) -> Vec<Result<Bytes, MultimediaError>> {
    bytes
        .chunks(size)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect()
}

async fn read_all(store: &BlobStore, key: &BlobKey) -> Result<Vec<u8>, MultimediaError> {
    let mut out = Vec::new();
    store
        .read_verified(key, |chunk| {
            out.extend_from_slice(chunk);
            Ok::<(), MultimediaError>(())
        })
        .await?;
    Ok(out)
}

#[test]
fn a_key_round_trips_through_its_hex_form() {
    let key = key_of(b"hello");
    let text = key.to_string();
    assert_eq!(text.len(), 64);
    assert_eq!(text.parse::<BlobKey>(), Ok(key));
    assert_eq!(
        text,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[test]
fn only_a_lowercase_digest_is_a_key() {
    for bad in [
        "",
        "abc123",
        &"A".repeat(64),
        &"g".repeat(64),
        &"a".repeat(63),
    ] {
        assert_eq!(bad.parse::<BlobKey>(), Err(NotABlobKey), "{bad:?}");
    }
}

#[test]
fn uri_round_trips_to_key() {
    let s = BlobStore::in_memory("test-bucket");
    let key = key_of(b"x");
    let uri = s.uri_for(&key);
    assert_eq!(uri, format!("s3://test-bucket/{key}"));
    assert_eq!(s.key_from_uri(&uri), Some(key));
}

#[test]
fn foreign_uri_is_not_our_key() {
    let s = BlobStore::in_memory("test-bucket");
    let key = key_of(b"x");
    assert_eq!(s.key_from_uri(&format!("s3://other-bucket/{key}")), None);
    assert_eq!(s.key_from_uri("https://example.org/img.png"), None);
    assert_eq!(
        s.key_from_uri(&format!("s3://test-bucket/nested/{key}")),
        None
    );
    assert_eq!(s.key_from_uri("s3://test-bucket/abc"), None);
    assert_eq!(s.key_from_uri(&format!("s3://test-bucketx/{key}")), None);
}

#[test]
fn a_write_time_is_old_only_once_more_than_the_age_has_passed() {
    let now = jiff::Timestamp::now();
    let hour = Duration::from_hours(1);
    let ago = |seconds: i64| now - jiff::SignedDuration::from_secs(seconds);
    assert!(older_than(ago(2 * 3600), hour), "two hours old");
    assert!(!older_than(ago(60), hour), "a minute old");
    assert!(
        older_than(ago(60), Duration::ZERO),
        "a zero age takes anything written"
    );
    // A write time ahead of this clock is never old, whatever the age.
    assert!(!older_than(ago(-60), Duration::ZERO), "clock skew");
}

#[tokio::test]
async fn the_listing_names_every_blob_of_ours_in_key_order() {
    let s = BlobStore::in_memory("b");
    let (one, two) = (key_of(b"one"), key_of(b"two"));
    for (key, bytes) in [(&one, b"one".as_slice()), (&two, b"two".as_slice())] {
        s.write_if_absent(key, 3, chunks(bytes, 3)).await.unwrap();
    }
    let listing = s.content_listing().await.unwrap();
    let mut keys = vec![one, two];
    keys.sort_unstable();
    assert_eq!(
        keys,
        listing.iter().map(|(key, _)| *key).collect::<Vec<_>>()
    );
    assert_eq!(keys, s.content_keys().await.unwrap());
}

#[tokio::test]
async fn write_read_exists_delete_round_trip() {
    let s = BlobStore::in_memory("b");
    let bytes = b"hello".to_vec();
    let key = key_of(&bytes);
    assert!(!s.exists(&key).await.unwrap());
    s.write_if_absent(&key, 5, chunks(&bytes, 2)).await.unwrap();
    assert!(s.exists(&key).await.unwrap());
    assert_eq!(read_all(&s, &key).await.unwrap(), bytes);
    // A second write of the same content is a no-op.
    s.write_if_absent(&key, 5, chunks(&bytes, 5)).await.unwrap();
    s.delete(&key).await.unwrap();
    assert!(!s.exists(&key).await.unwrap());
    // Deleting an absent blob succeeds.
    s.delete(&key).await.unwrap();
}

#[tokio::test]
async fn a_write_whose_bytes_do_not_hash_to_the_key_stores_nothing() {
    let s = BlobStore::in_memory("b");
    let key = key_of(b"the real bytes");
    let err = s
        .write_if_absent(&key, 8, chunks(b"tampered", 3))
        .await
        .unwrap_err();
    assert!(matches!(err, MultimediaError::Integrity { .. }), "{err:?}");
    assert!(
        !s.exists(&key).await.unwrap(),
        "nothing stored under the key"
    );
}

#[tokio::test]
async fn a_write_of_the_wrong_length_stores_nothing() {
    let s = BlobStore::in_memory("b");
    let key = key_of(b"hello");
    let err = s
        .write_if_absent(&key, 6, chunks(b"hello", 2))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            MultimediaError::Size {
                declared: 6,
                actual: 5,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(!s.exists(&key).await.unwrap());
}

#[tokio::test]
async fn a_failing_chunk_aborts_the_write() {
    let s = BlobStore::in_memory("b");
    let key = key_of(b"hello");
    let source = vec![
        Ok(Bytes::from_static(b"he")),
        Err(MultimediaError::Malformed("bad chunk".to_owned())),
    ];
    let err = s.write_if_absent(&key, 5, source).await.unwrap_err();
    assert!(matches!(err, MultimediaError::Malformed(_)), "{err:?}");
    assert!(!s.exists(&key).await.unwrap());
}

#[tokio::test]
async fn a_read_of_a_missing_blob_is_a_not_found_store_error() {
    let s = BlobStore::in_memory("b");
    let err = read_all(&s, &key_of(b"absent")).await.unwrap_err();
    assert!(
        matches!(&err, MultimediaError::Store(e) if e.is_not_found() && e.op() == BlobOp::Read),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_read_of_bytes_that_do_not_hash_to_the_key_is_an_integrity_error() {
    let s = BlobStore::in_memory("b");
    let key = key_of(b"original");
    s.op.write(&key.to_string(), b"tampered".to_vec())
        .await
        .unwrap();
    let err = read_all(&s, &key).await.unwrap_err();
    assert!(matches!(err, MultimediaError::Integrity { .. }), "{err:?}");
}

#[tokio::test]
async fn delete_all_content_removes_our_blobs_and_leaves_foreign_objects() {
    let s = BlobStore::in_memory("test-bucket");
    let ours = [b"one".as_slice(), b"two".as_slice()];
    for bytes in ours {
        s.write_if_absent(&key_of(bytes), len64(bytes.len()), chunks(bytes, 2))
            .await
            .unwrap();
    }
    let upper = "A".repeat(64);
    for foreign in [
        "README".to_owned(),
        format!("nested/{}", "b".repeat(64)),
        upper.clone(),
    ] {
        s.op.write(&foreign, b"x".to_vec()).await.unwrap();
    }

    let mut expected: Vec<BlobKey> = ours.iter().map(|b| key_of(b)).collect();
    expected.sort_unstable();
    assert_eq!(s.content_keys().await.unwrap(), expected);
    assert_eq!(s.delete_all_content().await.unwrap(), 2);
    assert_eq!(s.content_keys().await.unwrap(), Vec::<BlobKey>::new());
    assert!(s.op.exists("README").await.unwrap());
    assert!(s.op.exists(&upper).await.unwrap());
    // A second run finds nothing to delete.
    assert_eq!(s.delete_all_content().await.unwrap(), 0);
}

fn s3(endpoint: Option<&str>, allow_http: bool) -> BlobStoreParams {
    BlobStoreParams {
        backend: BackendSettings::S3(S3Settings {
            endpoint: endpoint.map(|e| url::Url::parse(e).expect("parses as a URL")),
            bucket: "b".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: None,
            secret_access_key: None,
            allow_http,
        }),
        limits: Limits::default(),
    }
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
        let err = BlobStore::from_params(s3(Some(bad), true))
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
    let store = BlobStore::from_params(s3(Some("https://user:hunter2@s3.example"), false))
        .expect("an https endpoint builds");
    assert!(!format!("{store:?}").contains("hunter2"));
}

/// An absolute http(s) endpoint still builds, so the check above is not a
/// blanket refusal.
#[test]
fn an_absolute_http_endpoint_still_builds() {
    assert!(BlobStore::from_params(s3(Some("http://seaweedfs:8333"), true)).is_ok());
}

/// With no endpoint, the store addresses the AWS endpoint of its region.
#[test]
fn no_endpoint_builds_against_the_regional_aws_endpoint() {
    let mut params = s3(None, false);
    let BackendSettings::S3(settings) = &mut params.backend;
    settings.access_key_id = Some("AKIDEXAMPLE".to_owned());
    settings.secret_access_key = Some(SecretString::from("secret"));
    assert!(BlobStore::from_params(params).is_ok());
}

/// A closed port is a typed store failure that names the operation and
/// the key, after the bounded retries, and never renders a credential.
#[tokio::test]
async fn an_unreachable_endpoint_is_a_store_error_without_credentials() {
    let mut params = s3(Some("http://127.0.0.1:9"), true);
    let BackendSettings::S3(settings) = &mut params.backend;
    settings.access_key_id = Some("AKIDEXAMPLE".to_owned());
    settings.secret_access_key = Some(SecretString::from("hunter2"));
    params.limits.request_timeout = Duration::from_secs(2);
    let store = BlobStore::from_params(params).expect("builds");
    let key = key_of(b"k");
    let err = store.exists(&key).await.unwrap_err();
    assert_eq!(err.op(), BlobOp::Exists);
    assert_eq!(err.key(), Some(key));
    let rendered = format!("{err:?} {err}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
}
