// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Contribution-outbox eventing tests against a real `PostgreSQL` 18
//! (shared testkit harness) — the transactional-outbox half of the eventing extension (tasks 2/3).
//!
//! Proves: (1) every CONTRIBUTION commit path (a direct composition commit, a
//! CONTRIBUTION commit, and an EHR-Extract import) writes exactly one pending
//! `event_outbox` row whose envelope is PHI-free (identity + provenance only,
//! no clinical content); (2) a rolled-back commit writes none (the outbox row
//! shares the transaction's fate); (3) the drainer publishes at-least-once with
//! no loss — a "broker down" phase leaves rows pending, and once the broker is
//! back every row drains in order (driven here through the [`EventPublisher`]
//! seam with a toggled fake, so the retry/no-loss logic is deterministic; the
//! real-broker end-to-end lives in `events_amqp.rs`).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

use crate::fixtures::{committer, composition_by, uv};
use crate::typed_body::typed;
use ferroehr::extensions::events::config::EventsConfig;
use ferroehr::extensions::events::publisher::start_with_publisher;
use ferroehr::service::FerroEhrService;
use ferroehr_ext::events::{EventError, EventPublisher};

// ── fixtures ─────────────────────────────────────────────────────────────────

/// A minimal valid bare-RM COMPOSITION whose `composer` is the PHI probe.
///
/// The name is load-bearing: [`assert_phi_free`] asserts that exact string
/// never reaches an event envelope, so this suite cannot share the default
/// composer.
fn composition(name: &str) -> Value {
    composition_by(name, "secret clinician name")
}

// ── outbox helpers ───────────────────────────────────────────────────────────

async fn pending_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM clinical.event_outbox WHERE published_at IS NULL")
        .fetch_one(pool)
        .await
        .expect("pending count")
}

async fn total_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM clinical.event_outbox")
        .fetch_one(pool)
        .await
        .expect("total count")
}

/// The most-recently written outbox row's envelope.
async fn latest_envelope(pool: &PgPool) -> Value {
    sqlx::query("SELECT envelope FROM clinical.event_outbox ORDER BY seq DESC LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("latest row")
        .try_get::<Value, _>("envelope")
        .expect("envelope")
}

/// Assert an envelope carries only PHI-free identity/provenance keys — the
/// exact top-level and per-version key sets, and no clinical-content markers.
fn assert_phi_free(envelope: &Value) {
    let obj = envelope.as_object().expect("envelope is an object");
    // The published payload additionally carries the delivery `seq` and the
    // per-version fan-out `version_index` (both injected at publish time,
    // E1 task 4); the stored envelope carries neither. Ignore them
    // for the key check.
    let mut top: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "seq" && *k != "version_index")
        .collect();
    top.sort_unstable();
    assert_eq!(
        top,
        ["committed_at", "contribution_id", "ehr_id", "versions"],
        "envelope must carry only the PHI-free top-level keys"
    );

    let versions = obj["versions"].as_array().expect("versions array");
    assert!(!versions.is_empty(), "at least one version entry");
    for v in versions {
        let mut keys: Vec<&str> = v
            .as_object()
            .expect("version entry object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        // `version_tree_id` joined the envelope with version-tree branching
        // (RM common master06 §The 'Virtual Version Tree') — identity metadata, PHI-free.
        assert_eq!(
            keys,
            [
                "change_type",
                "kind",
                "sys_version",
                "template_id",
                "version_tree_id",
                "vo_id"
            ],
            "version entry must carry only PHI-free keys"
        );
    }

    // No clinical content whatsoever (belt-and-braces over the structural check).
    let text = serde_json::to_string(envelope).expect("serialize envelope");
    for forbidden in [
        "composer",
        "secret clinician name",
        "archetype_node_id",
        "archetype_details",
        "territory",
        "DV_TEXT",
        "DV_CODED_TEXT",
    ] {
        assert!(
            !text.contains(forbidden),
            "envelope leaked clinical content ({forbidden}): {text}"
        );
    }
}

async fn create_ehr(svc: &FerroEhrService) -> ferroehr::ids::EhrId {
    svc.create_ehr(None).await.expect("create_ehr")
}

// ── (1) atomic writes on every commit path ───────────────────────────────────

#[tokio::test]
async fn composition_and_contribution_commits_each_write_one_phi_free_outbox_row() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // EHR creation is itself a CONTRIBUTION → its own outbox row (baseline).
    let ehr = create_ehr(&svc).await;
    let after_ehr = total_count(&pool).await;
    assert_eq!(after_ehr, 1, "EHR creation writes one outbox row");

    // (a) A direct composition commit writes exactly one more row.
    svc.create_composition(ehr, uv(&composition("v1"), "249", None))
        .await
        .expect("create_composition");
    assert_eq!(
        total_count(&pool).await,
        after_ehr + 1,
        "a composition commit writes exactly one outbox row"
    );
    let env = latest_envelope(&pool).await;
    assert_phi_free(&env);
    assert_eq!(env["ehr_id"], json!(ehr.to_string()));
    assert_eq!(env["versions"][0]["kind"], json!("COMPOSITION"));
    assert_eq!(env["versions"][0]["change_type"], json!("249"));
    assert_eq!(env["versions"][0]["sys_version"], json!(1));

    // (b) A CONTRIBUTION commit (a bare-RM composition version) writes one row.
    let before_contrib = total_count(&pool).await;
    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [{
            "_type": "ORIGINAL_VERSION",
            "commit_audit": {
                "change_type": { "_type": "DV_CODED_TEXT", "value": "creation",
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                        "code_string": "249" } },
                "committer": committer("author")
            },
            "lifecycle_state": { "_type": "DV_CODED_TEXT", "value": "complete",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "532" } },
            "data": composition("c1")
        }],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
    });
    svc.create_ehr_contribution(ehr, contribution)
        .await
        .expect("create_ehr_contribution");
    assert_eq!(
        total_count(&pool).await,
        before_contrib + 1,
        "a CONTRIBUTION commit writes exactly one outbox row"
    );
    let env = latest_envelope(&pool).await;
    assert_phi_free(&env);
    assert_eq!(env["versions"][0]["kind"], json!("COMPOSITION"));

    // All rows are still pending (no publisher running).
    assert_eq!(pending_count(&pool).await, total_count(&pool).await);
}

#[tokio::test]
async fn outbox_disabled_writes_no_rows() {
    // With no eventing consumer configured (`with_outbox_enabled(false)`), the
    // per-commit `event_outbox` INSERT is skipped entirely — no consumer will
    // ever read it. No openEHR spec governs eventing (our own extension).
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_outbox_enabled(false);

    // EHR creation, a direct composition commit, and a CONTRIBUTION commit —
    // none writes an outbox row.
    let ehr = create_ehr(&svc).await;
    assert_eq!(total_count(&pool).await, 0, "EHR creation writes none");

    svc.create_composition(ehr, uv(&composition("v1"), "249", None))
        .await
        .expect("create_composition");
    assert_eq!(
        total_count(&pool).await,
        0,
        "composition commit writes none"
    );

    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [{
            "_type": "ORIGINAL_VERSION",
            "commit_audit": {
                "change_type": { "_type": "DV_CODED_TEXT", "value": "creation",
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                        "code_string": "249" } },
                "committer": committer("author")
            },
            "lifecycle_state": { "_type": "DV_CODED_TEXT", "value": "complete",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "532" } },
            "data": composition("c1")
        }],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },  "committer": committer("author") }
    });
    svc.create_ehr_contribution(ehr, contribution)
        .await
        .expect("create_ehr_contribution");
    assert_eq!(
        total_count(&pool).await,
        0,
        "CONTRIBUTION commit writes none when the outbox is disabled"
    );
}

#[tokio::test]
async fn rolled_back_commit_writes_no_outbox_row() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // First EHR with a subject: one outbox row for its creation.
    let status = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": "demographic",
                "type": "PERSON",
                "id": { "_type": "GENERIC_ID", "value": "patient-42", "scheme": "mpi" }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    });
    svc.create_ehr(Some(typed(&status)))
        .await
        .expect("first EHR for subject-42");
    let baseline = total_count(&pool).await;
    assert_eq!(baseline, 1, "first EHR creation wrote one row");

    // A second EHR for the SAME subject violates uq_ehr_subject during the
    // EHR_STATUS write, aborting the whole transaction (the commit path,
    // including its outbox insert) — a rolled-back commit.
    svc.create_ehr(Some(typed(&status)))
        .await
        .expect_err("duplicate subject must conflict");
    assert_eq!(
        total_count(&pool).await,
        baseline,
        "a rolled-back commit leaves the outbox unchanged"
    );
}

// ── (3) drainer at-least-once + no-loss, via the EventPublisher seam ──────────

/// A publisher that fails while `fail` is set (simulating a down broker) and
/// records what it publishes otherwise.
#[derive(Default)]
struct TogglePublisher {
    fail: AtomicBool,
    published: std::sync::Mutex<Vec<(String, Value)>>,
}

impl TogglePublisher {
    fn published(&self) -> Vec<(String, Value)> {
        self.published.lock().expect("mutex").clone()
    }
}

#[async_trait]
impl EventPublisher for TogglePublisher {
    async fn publish(&self, routing_key: &str, payload: &[u8]) -> Result<(), EventError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(EventError::Nack("simulated broker down".to_owned()));
        }
        let env: Value = serde_json::from_slice(payload).expect("payload json");
        self.published
            .lock()
            .expect("mutex")
            .push((routing_key.to_owned(), env));
        Ok(())
    }
}

/// Poll `f` until it returns true or the deadline elapses.
async fn wait_until<F, Fut>(mut f: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if f().await {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not met before deadline"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn drainer_holds_pending_while_broker_down_then_drains_without_loss() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // Commit some work: an EHR + two compositions ⇒ three outbox rows.
    let ehr = create_ehr(&svc).await;
    svc.create_composition(ehr, uv(&composition("v1"), "249", None))
        .await
        .expect("comp 1");
    svc.create_composition(ehr, uv(&composition("v2"), "249", None))
        .await
        .expect("comp 2");
    let committed = total_count(&pool).await;
    assert_eq!(committed, 3);

    let publisher = Arc::new(TogglePublisher::default());
    publisher.fail.store(true, Ordering::Relaxed); // broker "down"
    let config = EventsConfig {
        enabled: true,
        poll_interval_ms: 50,
        publish_max_retries: 0,
        prune_interval_secs: 3_600,
        ..EventsConfig::default()
    };
    // Bound separately so the Arc<TogglePublisher> → Arc<dyn EventPublisher>
    // unsizing happens on assignment, not inside a `.clone()`.
    let dyn_publisher: Arc<dyn EventPublisher> = Arc::<TogglePublisher>::clone(&publisher);
    let handle = start_with_publisher(
        config,
        pool.clone(),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
        dyn_publisher,
    );

    // Broker down: rows stay pending, nothing published.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        pending_count(&pool).await,
        committed,
        "no row may drain while the broker is down"
    );
    assert!(
        publisher.published().is_empty(),
        "nothing published while the broker is down"
    );

    // Broker back up: every row drains, in seq order, exactly once each.
    publisher.fail.store(false, Ordering::Relaxed);
    wait_until(|| async { pending_count(&pool).await == 0 }).await;

    let delivered = publisher.published();
    // Per-version fan-out: each version entry is its
    // own message. EHR creation commits EHR_STATUS + EHR_ACCESS under one
    // CONTRIBUTION (2 versions); each composition, 1 — so the 3 outbox rows fan
    // out to 4 messages.
    let expected_messages: usize = delivered_version_count(&pool).await;
    assert_eq!(
        expected_messages, 4,
        "EHR (2 versions) + 2 compositions (1 each)"
    );
    assert_eq!(
        delivered.len(),
        expected_messages,
        "every committed version was published exactly once (no loss)"
    );
    // Every published payload carries seq + version_index + is PHI-free; the
    // (seq, version_index) pairs strictly ascend in publish order (per-EHR
    // order, with a row's versions in index order — messages within a row share
    // the row's seq).
    let mut last = (0i64, -1i64);
    for (rk, env) in &delivered {
        assert!(rk.contains('.'), "routing key looks like a topic key: {rk}");
        assert_phi_free(env);
        let seq = env["seq"].as_i64().expect("seq in payload");
        let vi = env["version_index"]
            .as_i64()
            .expect("version_index in payload");
        assert!(
            (seq, vi) > last,
            "(seq, version_index) must ascend (per-EHR order): {:?} !> {last:?}",
            (seq, vi)
        );
        last = (seq, vi);
    }

    handle.shutdown(Duration::from_secs(2)).await;
}

/// The total number of version entries across all outbox rows — the number of
/// per-version messages the drainer publishes.
async fn delivered_version_count(pool: &PgPool) -> usize {
    let n: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(jsonb_array_length(envelope -> 'versions')), 0)::bigint \
         FROM clinical.event_outbox",
    )
    .fetch_one(pool)
    .await
    .expect("version count");
    usize::try_from(n).expect("fits usize")
}

#[tokio::test]
async fn import_writes_one_phi_free_outbox_row() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let target_pool = target_db.pool();
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_pool,
    ));

    // Seed a source EHR with an EHR_STATUS (EHR creation + the auto EHR_ACCESS).
    let ehr = source.create_ehr(None).await.expect("source ehr");
    // Give it a second EHR_STATUS version so the import carries real content.
    let mut status = source
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("status");
    let ovid = status["uid"]["value"].as_str().expect("uid").to_owned();
    status.as_object_mut().unwrap().remove("uid");
    status["is_modifiable"] = json!(false);
    source
        .replace_ehr_status(ehr, uv(&status, "251", Some(&ovid)))
        .await
        .expect("status update");

    let mut extracts = source.extract_ehrs(ehr).await.expect("export");
    let extract: openehr_rm::v1_2::ehr_extract::common::extract::Extract =
        openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("typed extract");

    // Import into the fresh (empty) target: exactly one import CONTRIBUTION.
    target.import_ehr(None, extract).await.expect("import_ehr");

    assert_eq!(
        total_count(&target_pool).await,
        1,
        "an EHR-Extract import writes exactly one outbox row for its CONTRIBUTION"
    );
    let env = latest_envelope(&target_pool).await;
    assert_phi_free(&env);
    // The import announces every imported version (EHR_STATUS ×2 + EHR_ACCESS + …).
    let kinds: Vec<&str> = env["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"EHR_STATUS"),
        "import envelope must announce the EHR_STATUS versions, got {kinds:?}"
    );
}
