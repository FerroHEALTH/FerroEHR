// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end tests for `DV_MULTIMEDIA` externalization against a real
//! S3 backend — a `SeaweedFS` S3 gateway in a testcontainer — plus a real
//! `PostgreSQL` 18.
//!
//! Spec basis: RM 1.2.0 `DV_MULTIMEDIA`
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_multimedia.adoc`):
//! `uri`/`data` are alternatives under `is_inline or is_external`; an
//! externalized value carries `integrity_check` + `integrity_check_algorithm`
//! (openEHR `Integrity check algorithms` code set, code `SHA-256`) and the
//! mandatory unencoded `size`. Server-side blob storage is spec-silent — this
//! is our design, and these tests are its acceptance instrument.
//!
//! Each test owns its S3 container (Drop removes it); the PostgreSQL database
//! comes from the shared testkit harness. Requires Docker. `SeaweedFS` with no credentials runs in unauthenticated
//! "allow-all" mode (dev/test only).

#![expect(
    clippy::expect_used,
    let_underscore_drop,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use testcontainers::core::IntoContainerPort;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use uuid::Uuid;

use ferroehr::extensions::multimedia::config::MultimediaConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::types::ExportSpec;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr_ext::multimedia::MultimediaEngine;
use ferroehr_ext::multimedia::store::BlobStore;

const BUCKET: &str = "openehr-multimedia";
/// S3-gateway port `SeaweedFS` listens on.
const S3_PORT: u16 = 8333;

// ── containers ───────────────────────────────────────────────────────────────

struct Seaweed {
    _container: ContainerAsync<GenericImage>,
    endpoint: String,
}

impl Seaweed {
    async fn start() -> Self {
        // `weed server -s3` brings up master+volume+filer+S3. testcontainers'
        // `http` wait is feature-gated, so readiness is gated by the
        // bucket-create retry loop below (it polls until the S3 gateway answers)
        // rather than a WaitFor.
        let container = GenericImage::new("chrislusf/seaweedfs", "latest")
            .with_exposed_port(S3_PORT.tcp())
            .with_cmd(["server", "-s3", "-dir=/data"])
            .with_startup_timeout(Duration::from_secs(90))
            .start()
            .await
            .expect("start seaweedfs (is Docker running?)");
        let host = container.get_host().await.expect("host").to_string();
        let port = container
            .get_host_port_ipv4(S3_PORT)
            .await
            .expect("mapped s3 port");
        let endpoint = format!("http://{host}:{port}");
        // The gateway is up but the filer may need a moment; create the bucket
        // with a short retry (S3 CreateBucket = PUT /<bucket>).
        create_bucket(&endpoint, BUCKET).await;
        // CreateBucket succeeding proves only FILER metadata readiness —
        // object writes additionally need a VOLUME server registered with the
        // master, which lags behind on a loaded host ("no writable volumes").
        // That gap was the #541 flake: the first offload write of the test
        // proper failed the commit, and the nextest retry masked it. Gate on
        // the capability the tests actually use: a probe object round-trip.
        probe_object_write(&endpoint, BUCKET).await;
        Self {
            _container: container,
            endpoint,
        }
    }

    fn config(&self) -> MultimediaConfig {
        MultimediaConfig {
            enabled: true,
            threshold_bytes: 256,
            endpoint: Some(self.endpoint.clone()),
            bucket: BUCKET.to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: None,
            secret_access_key: None,
            secret_access_key_file: None,
            allow_http: true,
        }
    }

    fn engine(&self) -> Arc<MultimediaEngine> {
        Arc::new(
            ferroehr::extensions::multimedia::engine_from_config(&self.config())
                .expect("build engine")
                .expect("engine enabled"),
        )
    }
}

/// Create the bucket via an anonymous S3 `CreateBucket` (PUT /<bucket>), retrying
/// briefly while the filer finishes coming up.
async fn create_bucket(endpoint: &str, bucket: &str) {
    let client = reqwest::Client::new();
    let url = format!("{endpoint}/{bucket}");
    for attempt in 0..90 {
        match client.put(&url).send().await {
            Ok(resp)
                if resp.status().is_success() || resp.status() == reqwest::StatusCode::CONFLICT =>
            {
                return;
            }
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
        assert!(attempt < 89, "seaweedfs bucket creation never succeeded");
    }
}

/// Gate on OBJECT-write readiness: PUT a probe object until the write (and its
/// read-back) succeeds, then delete it. Bucket creation alone only proves the
/// filer is up; the first object write needs a volume server registered with
/// the master, which arrives later under host contention (#541).
async fn probe_object_write(endpoint: &str, bucket: &str) {
    let client = reqwest::Client::new();
    let url = format!("{endpoint}/{bucket}/.readiness-probe");
    for attempt in 0..90 {
        let wrote = matches!(
            client.put(&url).body("probe").send().await,
            Ok(resp) if resp.status().is_success()
        );
        if wrote
            && matches!(
                client.get(&url).send().await,
                Ok(resp) if resp.status().is_success()
            )
        {
            let _deleted = client.delete(&url).send().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(attempt < 89, "seaweedfs object writes never became ready");
    }
}

// ── fixtures ───────────────────────────────────────────────────────────────

/// A canonical `DV_MULTIMEDIA` node with `n` bytes of inline data.
fn multimedia(n: usize) -> Value {
    let payload = vec![0x42u8; n];
    json!({
        "_type": "DV_MULTIMEDIA",
        "media_type": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_media-types" },
            "code_string": "application/octet-stream"
        },
        "size": n,
        "data": base64::engine::general_purpose::STANDARD.encode(&payload),
    })
}

/// A demographic `PERSON` carrying `media` as the value of an identity ELEMENT.
///
/// The party path externalizes exactly as the clinical one does — the
/// versioning layer is shared — so this is how a blob comes to be referenced
/// only by a party.
fn person_with_media(name: &str, media: &Value) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "portrait" },
                    "value": media.clone()
                }]
            }
        }]
    })
}

/// Create a PERSON and return its bare versioned-object id.
async fn make_person_with_media(svc: &FerroEhrService, name: &str, media: &Value) -> String {
    let created = svc
        .party_create(
            PartyKind::Person,
            openehr_its::json::from_canonical_value(&person_with_media(name, media))
                .expect("the PERSON decodes"),
            None,
        )
        .await
        .expect("create person");
    created
        .body
        .pointer("/uid/value")
        .and_then(Value::as_str)
        .expect("uid")
        .split("::")
        .next()
        .expect("vo uuid")
        .to_owned()
}

/// The blob key referenced by a stored PERSON's identity ELEMENT.
fn party_blob_key(person: &Value) -> String {
    let uri = person
        .pointer("/identities/0/details/items/0/value/uri/value")
        .and_then(Value::as_str)
        .expect("the externalized media carries a uri");
    uri.rsplit('/').next().expect("hex").to_owned()
}

/// A valid `EHR_STATUS` carrying `media` inside `other_details` (an ELEMENT value).
#[expect(
    clippy::needless_pass_by_value,
    reason = "the helper takes an owned Value so call sites can pass a json! \
              literal directly"
)]
fn status_with_media(media: Value) -> openehr_rm::prelude::EhrStatus {
    openehr_its::json::from_canonical_value(&status_with_media_value(&media))
        .expect("the fixture EHR_STATUS decodes")
}

/// The same fixture as its canonical JSON (what a client would post).
fn status_with_media_value(media: &Value) -> Value {
    json!({
        "_type": "EHR_STATUS",
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        // Roots carry ARCHETYPED (LOCATABLE.Archetyped_valid).
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "subject": { "_type": "PARTY_SELF" },
        "is_queryable": true,
        "is_modifiable": true,
        "other_details": {
            "_type": "ITEM_TREE",
            "name": { "_type": "DV_TEXT", "value": "tree" },
            "archetype_node_id": "at0001",
            "items": [ {
                "_type": "ELEMENT",
                "name": { "_type": "DV_TEXT", "value": "media" },
                "archetype_node_id": "at0002",
                "value": media
            } ]
        }
    })
}

/// The `DV_MULTIMEDIA` node inside an `EHR_STATUS`'s `other_details`.
fn media_node(status: &Value) -> &Value {
    status
        .pointer("/other_details/items/0/value")
        .expect("multimedia node in other_details")
}

/// The blob key (hex) referenced by an externalized `DV_MULTIMEDIA` node.
fn blob_key(status: &Value) -> String {
    let uri = media_node(status)
        .pointer("/uri/value")
        .and_then(Value::as_str)
        .expect("uri.value");
    uri.rsplit('/').next().expect("hex").to_owned()
}

// ── tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn blob_store_round_trips_against_seaweedfs() {
    let sw = Seaweed::start().await;
    let store = BlobStore::from_params(ferroehr_ext::multimedia::store::BlobStoreParams {
        endpoint: sw.config().endpoint_url().expect("endpoint"),
        bucket: sw.config().bucket,
        region: sw.config().region,
        access_key_id: None,
        secret_access_key: None,
        allow_http: true,
    })
    .expect("store");

    assert!(!store.exists("k1").await.unwrap());
    store.put_if_absent("k1", b"hello".to_vec()).await.unwrap();
    assert!(store.exists("k1").await.unwrap());
    assert_eq!(&*store.get("k1").await.unwrap(), b"hello");
    // put_if_absent on an existing key is a no-op (content-addressed dedup).
    store.put_if_absent("k1", b"hello".to_vec()).await.unwrap();
    store.delete("k1").await.unwrap();
    assert!(!store.exists("k1").await.unwrap());
    // delete of an absent key is idempotent.
    store.delete("k1").await.unwrap();
}

#[tokio::test]
async fn commit_offloads_large_multimedia_and_expands() {
    let db = testkit::db().await.expect("testkit database");
    let sw = Seaweed::start().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_multimedia(sw.engine());

    // Commit an EHR whose EHR_STATUS carries a >threshold inline multimedia.
    let ehr = svc
        .create_ehr(Some(status_with_media(multimedia(1000))))
        .await
        .expect("create ehr");

    // The stored/served form is externalized: data gone, uri + integrity + size
    // present (RM invariants honoured), and the blob exists in the store.
    let stored = svc
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("read status");
    let node = media_node(&stored);
    assert!(node.get("data").is_none(), "inline data must be gone");
    assert_eq!(
        node.pointer("/uri/value").unwrap(),
        &json!(format!("s3://{BUCKET}/{}", blob_key(&stored)))
    );
    assert!(node.get("integrity_check").unwrap().is_string());
    assert_eq!(
        node.pointer("/integrity_check_algorithm/code_string")
            .unwrap(),
        "SHA-256"
    );
    assert_eq!(
        node.pointer("/integrity_check_algorithm/terminology_id/value")
            .unwrap(),
        "openehr_integrity_check_algorithms"
    );
    assert_eq!(node.get("size").unwrap(), &json!(1000));

    let key = blob_key(&stored);
    assert!(
        sw.engine().store().exists(&key).await.unwrap(),
        "blob must exist in the store"
    );

    // expand_multimedia re-inlines the verified bytes.
    let expanded = svc.expand_multimedia(stored.clone()).await.expect("expand");
    let enode = media_node(&expanded);
    let data = enode
        .get("data")
        .and_then(Value::as_str)
        .expect("inline data");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap(),
        vec![0x42u8; 1000]
    );

    // Without the flag the default read keeps the externalized form untouched.
    let unexpanded = svc
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("read again");
    assert!(media_node(&unexpanded).get("data").is_none());
}

#[tokio::test]
async fn small_multimedia_stays_inline() {
    let db = testkit::db().await.expect("testkit database");
    let sw = Seaweed::start().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_multimedia(sw.engine());

    let media = multimedia(100); // below the 256-byte threshold
    let inline_data = media.get("data").cloned();
    let ehr = svc
        .create_ehr(Some(status_with_media(media)))
        .await
        .expect("create ehr");
    let stored = svc
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("read status");
    let node = media_node(&stored);
    assert_eq!(
        node.get("data").cloned(),
        inline_data,
        "below-threshold media must stay inline byte-identical"
    );
    assert!(
        node.get("uri").is_none(),
        "no externalization for small media"
    );
}

#[tokio::test]
async fn corrupted_blob_fails_integrity_on_expand() {
    use object_store::{ObjectStoreExt, aws::AmazonS3Builder, path::Path};

    let db = testkit::db().await.expect("testkit database");
    let sw = Seaweed::start().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_multimedia(sw.engine());

    let ehr = svc
        .create_ehr(Some(status_with_media(multimedia(1000))))
        .await
        .expect("create ehr");
    let stored = svc.get_ehr_status_at_time(ehr, None).await.expect("read");
    let key = blob_key(&stored);

    // Tamper with the stored bytes directly (overwrite the object at its key).
    let raw = AmazonS3Builder::new()
        .with_bucket_name(BUCKET)
        .with_region("us-east-1")
        .with_endpoint(&sw.endpoint)
        .with_allow_http(true)
        .with_skip_signature(true)
        .build()
        .expect("raw s3");
    raw.put(&Path::from(key), b"tampered bytes".to_vec().into())
        .await
        .expect("overwrite");

    // Expansion recomputes the SHA-256, detects the mismatch, and errors (500),
    // never serving corrupted data.
    let err = svc.expand_multimedia(stored).await.expect_err("must fail");
    assert_eq!(
        err.status,
        ferroehr::service::status::CallStatusType::Exception,
        "integrity failure maps to a server fault (500)"
    );
}

#[tokio::test]
async fn gc_removes_unreferenced_but_keeps_shared_blobs() {
    let db = testkit::db().await.expect("testkit database");
    let sw = Seaweed::start().await;
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_multimedia(sw.engine());

    // Two EHRs carrying the *same* bytes → the same content-addressed blob.
    let ehr1 = svc
        .create_ehr(Some(status_with_media(multimedia(1000))))
        .await
        .expect("ehr1");
    let ehr2 = svc
        .create_ehr(Some(status_with_media(multimedia(1000))))
        .await
        .expect("ehr2");
    let s1 = svc.get_ehr_status_at_time(ehr1, None).await.expect("s1");
    let s2 = svc.get_ehr_status_at_time(ehr2, None).await.expect("s2");
    let key = blob_key(&s1);
    assert_eq!(key, blob_key(&s2), "identical media dedups to one blob");
    assert!(sw.engine().store().exists(&key).await.unwrap());

    // Deleting ehr1 leaves the blob (still referenced by ehr2).
    svc.admin_ehr_delete(ehr1.to_string())
        .await
        .expect("delete ehr1");
    assert!(
        sw.engine().store().exists(&key).await.unwrap(),
        "a shared blob must survive while another EHR references it"
    );

    // Deleting ehr2 removes the last reference → the blob is GC'd.
    svc.admin_ehr_delete(ehr2.to_string())
        .await
        .expect("delete ehr2");
    assert!(
        !sw.engine().store().exists(&key).await.unwrap(),
        "an unreferenced blob must be GC'd on physical delete"
    );
}

/// #3180: a physically deleted party takes its blobs with it, and leaves a
/// blob anything else still references alone.
#[tokio::test]
async fn physical_party_delete_collects_its_blobs_but_keeps_shared_ones() {
    let db = testkit::db().await.expect("testkit database");
    let sw = Seaweed::start().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_multimedia(sw.engine());

    // One party holds a blob nothing else references; a second holds one it
    // shares with a clinical record (identical bytes dedup to one blob).
    let solo_media = multimedia(1100);
    let shared_media = multimedia(1200);
    let solo = make_person_with_media(&svc, "Solo", &solo_media).await;
    let sharer = make_person_with_media(&svc, "Sharer", &shared_media).await;
    let ehr = svc
        .create_ehr(Some(status_with_media(shared_media.clone())))
        .await
        .expect("ehr sharing the blob");

    let solo_key = party_blob_key(
        &svc.party_get(PartyKind::Person, solo.clone(), None)
            .await
            .expect("read solo")
            .body,
    );
    let shared_key = party_blob_key(
        &svc.party_get(PartyKind::Person, sharer.clone(), None)
            .await
            .expect("read sharer")
            .body,
    );
    assert_eq!(
        shared_key,
        blob_key(
            &svc.get_ehr_status_at_time(ehr, None)
                .await
                .expect("read the sharing EHR")
        ),
        "identical media must dedup to one blob for the cross-domain case to mean anything"
    );
    assert_ne!(solo_key, shared_key);
    assert!(sw.engine().store().exists(&solo_key).await.unwrap());
    assert!(sw.engine().store().exists(&shared_key).await.unwrap());

    svc.physical_party_delete(solo.clone())
        .await
        .expect("delete the solo party");
    assert!(
        !sw.engine().store().exists(&solo_key).await.unwrap(),
        "a blob only the deleted party referenced must be collected — a blob is \
         content, and the operator asked for a physical delete"
    );

    svc.physical_party_delete(sharer.clone())
        .await
        .expect("delete the sharing party");
    assert!(
        sw.engine().store().exists(&shared_key).await.unwrap(),
        "a blob a clinical record still references must survive the party delete"
    );
}

#[tokio::test]
async fn dump_load_carries_blobs() {
    let source_db = testkit::db().await.expect("testkit database");
    let sw = Seaweed::start().await;
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ))
    .with_multimedia(sw.engine());
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_db.pool(),
    ))
    .with_multimedia(sw.engine());

    let ehr = source
        .create_ehr(Some(status_with_media(multimedia(1000))))
        .await
        .expect("seed ehr");
    let key = blob_key(
        &source
            .get_ehr_status_at_time(ehr, None)
            .await
            .expect("read"),
    );

    let dir = std::env::temp_dir()
        .join(format!("ferroehr-mm-dump-{}", Uuid::now_v7()))
        .to_string_lossy()
        .into_owned();
    let reports = source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1024))
        .await
        .expect("export");
    assert!(reports.is_empty(), "clean export, got {reports:?}");

    // The archive carries the blob in a blobs/<hex> file.
    let blob_path = std::path::Path::new(&dir).join("blobs").join(&key);
    assert!(blob_path.exists(), "export writes blobs/<hex>");

    // Load into the fresh target: the EHR + its blob re-populate.
    let loaded = target.load_ehrs(dir.clone()).await.expect("load");
    assert!(loaded.is_empty(), "clean load, got {loaded:?}");

    let stored = target
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("read target");
    assert_eq!(blob_key(&stored), key, "external reference round-trips");
    assert!(
        sw.engine().store().exists(&key).await.unwrap(),
        "the blob is present in the target's store"
    );
    // And expansion works end-to-end on the target.
    let expanded = target.expand_multimedia(stored).await.expect("expand");
    assert!(media_node(&expanded).get("data").is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

/// The default-mode zero-drift proof: with externalization OFF (no engine),
/// large inline multimedia is stored and read back byte-identical — no object
/// store is contacted, so this needs only Postgres.
#[tokio::test]
async fn disabled_by_default_stores_inline_verbatim() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let media = multimedia(4096); // well above any threshold
    let inline_data = media.get("data").cloned();
    let ehr = svc
        .create_ehr(Some(status_with_media(media)))
        .await
        .expect("create ehr");
    let stored = svc.get_ehr_status_at_time(ehr, None).await.expect("read");
    let node = media_node(&stored);
    assert_eq!(
        node.get("data").cloned(),
        inline_data,
        "with externalization off, media is stored inline byte-identical"
    );
    assert!(node.get("uri").is_none(), "no uri when the feature is off");

    // expand_multimedia is a transparent no-op passthrough when there is
    // nothing to expand: this record is inline, so the request has no work.
    let same = svc.expand_multimedia(stored.clone()).await.expect("no-op");
    assert_eq!(same, stored);
}

/// With NO store reachable, a record that references an externalized blob makes
/// `expand_multimedia` fail rather than answer with the compact reference.
///
/// The silent version of this was the defect: the caller asked for the bytes,
/// the server answered `200` with a `s3://` URI instead, and nothing said the
/// request had not been honoured. Switching an integration off may stop new
/// offloads; it may not quietly change what clinical content a read returns.
#[tokio::test]
async fn unreachable_store_refuses_expansion_instead_of_answering_silently() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // A body shaped like a stored, already-externalized record. Built by hand
    // because reaching this state through the service would require the very
    // store this test asserts is absent.
    let body = serde_json::json!({
        "_type": "EHR_STATUS",
        "data": {
            "_type": "DV_MULTIMEDIA",
            "uri": {"_type": "DV_URI", "value": "s3://openehr-multimedia/deadbeef"},
            "size": 4096
        }
    });

    let err = svc
        .expand_multimedia(body)
        .await
        .expect_err("an unserviceable expansion must fail, not answer");
    // The wire body stays the curated opaque 500 text; the actionable detail is
    // on the trace record, never on the wire.
    assert!(
        format!("{err:?}").contains("Exception"),
        "expected a server-fault status, got {err:?}"
    );
}
