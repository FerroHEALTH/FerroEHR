// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The decommissioning erase against a real database: every domain written,
//! then erased behind its confirmation, until no schema of this build and no
//! row remains.
//!
//! NOTE: no openEHR spec governs this — our own design, for
//! `docs/law/eu/cra/text.html` Annex I Part I(2)(m).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              fixture helpers and async bodies; a failing fixture must panic at \
              the fixture (the Rust Book ch11)"
)]

use ferroehr::config::secret::SecretUrl;
use ferroehr::db::DbConfig;
use ferroehr::db::domain::{DomainPools, StorageConfig};
use ferroehr::decommission::{Decommission, DecommissionError, Erasure};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use sqlx::PgPool;
use uuid::Uuid;

use crate::fixtures::{composition, uv};
use crate::pseudonymisation_boundary::a_person;
use crate::typed_body::typed;

/// The five schemas the migrations create, as `erase_schema` drops them.
const SCHEMAS: [&str; 5] = ["audit", "linkage", "party", "clinical", "ext"];

/// A decommissioning over the testkit clone, every domain on its one DSN.
fn decommission(db: &testkit::TestDb) -> Decommission {
    Decommission::new(
        DbConfig {
            url: SecretUrl::new(db.url().to_owned()),
            ..DbConfig::default()
        },
        StorageConfig::default(),
    )
}

/// Writes one record into every domain: an EHR with a composition, a party,
/// a linkage row and an audit record.
async fn seed_every_domain(pool: &PgPool) {
    let service = FerroEhrService::new(&DomainPools::from_shared(pool));
    let ehr = service.create_ehr(None).await.expect("create an EHR");
    service
        .create_composition(ehr, uv(&composition("Encounter"), "249", None))
        .await
        .expect("commit a composition");
    service
        .party_create(PartyKind::Person, typed(&a_person()), None)
        .await
        .expect("create a party");
    sqlx::query("INSERT INTO linkage.subject_ehr (party_id, ehr_id) VALUES ($1, $2)")
        .bind(Uuid::now_v7())
        .bind(Uuid::now_v7())
        .execute(pool)
        .await
        .expect("write a linkage row");
    sqlx::query(
        "INSERT INTO audit.audit_event \
         (recorded_at, action, outcome, event_code, resource_class, fhir) \
         VALUES (now(), 'R', 0, '110110', 'composition', '{}'::jsonb)",
    )
    .execute(pool)
    .await
    .expect("write an audit record");
}

/// The rows each domain holds, summed over one relation per domain.
async fn rows_per_domain(pool: &PgPool) -> [i64; 5] {
    let mut counts = [0; 5];
    for (slot, sql) in counts.iter_mut().zip([
        "SELECT count(*) FROM clinical.ehr",
        "SELECT count(*) FROM clinical.version",
        "SELECT count(*) FROM party.version",
        "SELECT count(*) FROM linkage.subject_ehr",
        "SELECT count(*) FROM audit.audit_event",
    ]) {
        *slot = sqlx::query_scalar(sql)
            .fetch_one(pool)
            .await
            .expect("count a domain's rows");
    }
    counts
}

/// How many of this build's schemas, and relations in them, the database holds.
async fn remaining(pool: &PgPool) -> (i64, i64) {
    let names: Vec<String> = SCHEMAS.iter().map(|s| (*s).to_owned()).collect();
    let schemas: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_namespace WHERE nspname = ANY($1)")
            .bind(&names)
            .fetch_one(pool)
            .await
            .expect("count the schemas");
    let relations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = ANY($1)",
    )
    .bind(&names)
    .fetch_one(pool)
    .await
    .expect("count the relations");
    (schemas, relations)
}

/// A wrong confirmation changes nothing; the instance id erases every domain,
/// the instance id with them, and a second run is a no-op.
#[tokio::test]
async fn erase_removes_every_domain_behind_the_instance_id() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    seed_every_domain(&pool).await;
    let instance = ferroehr::usage_report::store::ensure_instance(&pool)
        .await
        .expect("store an instance id")
        .id;
    let before = rows_per_domain(&pool).await;
    assert!(
        before.iter().all(|rows| *rows > 0),
        "every domain must hold a row, else the erase is proven on nothing: {before:?}"
    );

    let decommission = decommission(&db);
    let inventory = decommission.inventory().await.expect("the dry run reads");
    assert_eq!(inventory.confirmation(), Some(instance.to_string()));
    assert_eq!(inventory.databases.instance_id, Some(instance));
    assert_eq!(
        inventory
            .databases
            .databases
            .iter()
            .flat_map(|d| d.schemas.iter().copied())
            .collect::<Vec<_>>(),
        SCHEMAS.to_vec()
    );
    assert_eq!(
        rows_per_domain(&pool).await,
        before,
        "the dry run writes nothing"
    );

    for wrong in [
        db.name().to_owned(),
        Uuid::now_v7().to_string(),
        String::new(),
    ] {
        let refused = decommission.erase(&wrong).await;
        assert!(
            matches!(refused, Err(DecommissionError::NotConfirmed)),
            "`{wrong}` must not confirm: {refused:?}"
        );
    }
    assert_eq!(
        rows_per_domain(&pool).await,
        before,
        "a refused erase changes nothing"
    );

    let erased = decommission
        .erase(&instance.to_string())
        .await
        .expect("the instance id confirms");
    let Erasure::Erased { databases, .. } = erased else {
        panic!("the erase must report what it dropped: {erased:?}");
    };
    assert_eq!(databases.len(), 1);
    assert_eq!(
        databases.first().map(|d| d.schemas.clone()),
        Some(SCHEMAS.to_vec())
    );
    assert_eq!(
        remaining(&pool).await,
        (0, 0),
        "no schema of this build, and so no row, remains"
    );
    let gist: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_extension WHERE extname = 'btree_gist'")
            .fetch_one(&pool)
            .await
            .expect("read the extensions");
    assert_eq!(gist, 0, "the extension installed in ext goes with it");

    let again = decommission
        .erase(&instance.to_string())
        .await
        .expect("a second run succeeds");
    assert_eq!(again, Erasure::NothingToErase);
}

/// A database that stores no instance id is confirmed by its own name.
#[tokio::test]
async fn without_an_instance_id_the_database_name_confirms() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let decommission = decommission(&db);
    let inventory = decommission.inventory().await.expect("the dry run reads");
    assert_eq!(inventory.databases.instance_id, None);
    assert_eq!(inventory.confirmation(), Some(db.name().to_owned()));

    let erased = decommission
        .erase(db.name())
        .await
        .expect("the database name confirms");
    assert!(matches!(erased, Erasure::Erased { .. }), "{erased:?}");
    assert_eq!(remaining(&pool).await, (0, 0));
}

/// The blobs are deleted with the schemas; an unreachable store refuses before
/// the database is touched.
#[cfg(feature = "multimedia")]
mod blobs {
    use std::sync::Arc;

    use ferroehr::decommission::{DecommissionError, Erasure};
    use ferroehr_ext::multimedia::store::BlobStore;
    use object_store::memory::InMemory;

    use super::{decommission, remaining, rows_per_domain, seed_every_domain};

    #[tokio::test]
    async fn erase_deletes_every_blob_before_the_schemas() {
        let db = testkit::db().await.expect("testkit database");
        let pool = db.pool();
        seed_every_domain(&pool).await;
        let store = BlobStore::from_parts(Arc::new(InMemory::new()), "media".to_owned());
        for key in ["a".repeat(64), "b".repeat(64), "c".repeat(64)] {
            store
                .put_if_absent(&key, b"blob".to_vec())
                .await
                .expect("store a blob");
        }

        let decommission = decommission(&db).with_blob_store(store.clone());
        let inventory = decommission.inventory().await.expect("the dry run reads");
        assert_eq!(inventory.blobs.as_ref().map(|b| b.objects), Some(3));

        let erased = decommission
            .erase(db.name())
            .await
            .expect("the database name confirms");
        let Erasure::Erased { blobs_deleted, .. } = erased else {
            panic!("the erase must report what it removed: {erased:?}");
        };
        assert_eq!(blobs_deleted, Some(3));
        assert!(
            store.content_keys().await.expect("list").is_empty(),
            "no blob remains"
        );
        assert_eq!(remaining(&pool).await, (0, 0));
    }

    #[tokio::test]
    async fn an_unreachable_store_refuses_before_the_database_is_touched() {
        let db = testkit::db().await.expect("testkit database");
        let pool = db.pool();
        seed_every_domain(&pool).await;
        let before = rows_per_domain(&pool).await;
        let store = BlobStore::from_parts(Arc::new(unreachable::Unreachable), "media".to_owned());

        let refused = decommission(&db)
            .with_blob_store(store)
            .erase(db.name())
            .await;
        assert!(
            matches!(refused, Err(DecommissionError::Blobs(_))),
            "{refused:?}"
        );
        assert_eq!(
            rows_per_domain(&pool).await,
            before,
            "the database is whole"
        );
    }

    /// An object store every call of which fails, as a store behind a closed
    /// port does once its retries run out.
    mod unreachable {
        use futures::StreamExt as _;
        use futures::stream::BoxStream;
        use object_store::path::Path;
        use object_store::{
            CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
            ObjectStore, PutMultipartOptions, PutOptions, PutPayload, PutResult, Result,
        };

        #[derive(Debug)]
        pub(super) struct Unreachable;

        impl std::fmt::Display for Unreachable {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("Unreachable")
            }
        }

        fn refused() -> object_store::Error {
            object_store::Error::Generic {
                store: "Unreachable",
                source: "connection refused".into(),
            }
        }

        #[async_trait::async_trait]
        impl ObjectStore for Unreachable {
            async fn put_opts(&self, _: &Path, _: PutPayload, _: PutOptions) -> Result<PutResult> {
                Err(refused())
            }

            async fn put_multipart_opts(
                &self,
                _: &Path,
                _: PutMultipartOptions,
            ) -> Result<Box<dyn MultipartUpload>> {
                Err(refused())
            }

            async fn get_opts(&self, _: &Path, _: GetOptions) -> Result<GetResult> {
                Err(refused())
            }

            fn delete_stream(
                &self,
                _: BoxStream<'static, Result<Path>>,
            ) -> BoxStream<'static, Result<Path>> {
                futures::stream::once(async { Err(refused()) }).boxed()
            }

            fn list(&self, _: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
                futures::stream::once(async { Err(refused()) }).boxed()
            }

            async fn list_with_delimiter(&self, _: Option<&Path>) -> Result<ListResult> {
                Err(refused())
            }

            async fn copy_opts(&self, _: &Path, _: &Path, _: CopyOptions) -> Result<()> {
                Err(refused())
            }
        }
    }
}
