// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Contribution-outbox eventing tests against a real `PostgreSQL` 18
//! (shared testkit harness) — the transactional-outbox half of the eventing extension (tasks 2/3).
//!
//! Proves: (1) every CONTRIBUTION commit path (a direct composition commit, a
//! CONTRIBUTION commit, and an EHR-Extract import) writes exactly one
//! `event_outbox` row whose envelope is PHI-free (identity + provenance only,
//! no clinical content); (2) a rolled-back commit writes none (the outbox row
//! shares the transaction's fate); (3) the outbox feed serves every row to a
//! reader in `seq` order until the reader acknowledges it.

use ferroehr::service::version_update::WriteRequest;

use serde_json::{Value, json};
use sqlx::{PgPool, Row};

use crate::fixtures::commit_audit;
use crate::fixtures::{committer, composition_by, uv};
use crate::typed_body::typed;
use ferroehr::extensions::outbox::{OutboxDomain, OutboxGate, reconcile};
use ferroehr::service::FerroEhrService;

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
    // The feed adds the delivery `seq` beside the stored envelope, which does
    // not carry it. Ignore it for the key check.
    let mut top: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "seq")
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

// ── (1) atomic writes on every commit path ───────────────────────────────────

#[tokio::test]
async fn composition_and_contribution_commits_each_write_one_phi_free_outbox_row() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // EHR creation is itself a CONTRIBUTION → its own outbox row (baseline).
    let ehr = crate::fixtures::create_ehr(&svc).await;
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
        "audit": commit_audit("251", "modification", "author")
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
}

#[tokio::test]
async fn outbox_disabled_writes_no_rows() {
    // With no change-feed grant naming a reader (`OutboxGate::WhileGranted`),
    // the per-commit `event_outbox` INSERT is skipped entirely — no consumer
    // will ever read it. No openEHR spec governs eventing (our own extension).
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_outbox_gate(OutboxGate::WhileGranted);

    // EHR creation, a direct composition commit, and a CONTRIBUTION commit —
    // none writes an outbox row.
    let ehr = crate::fixtures::create_ehr(&svc).await;
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
        "audit": commit_audit("251", "modification", "author")
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

/// A `read_change_feed` grant committed to the CDR turns the outbox on for
/// the next commit, with no restart, under the binary's gate (#5439).
#[tokio::test]
async fn a_change_feed_grant_turns_the_outbox_on() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = crate::policy_fixtures::granting(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_outbox_gate(OutboxGate::WhileGranted),
    )
    .await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    assert_eq!(total_count(&pool).await, 0, "no reader is granted yet");
    crate::policy_fixtures::granted(
        &svc,
        "ferrobridge",
        &[crate::policy_fixtures::capability(
            "read_change_feed",
            &[crate::policy_fixtures::scope("reader", "ferrobridge")],
        )],
    )
    .await;
    assert_eq!(
        svc.change_feed_readers().await.expect("the grants read"),
        ["ferrobridge"]
    );
    svc.create_composition(ehr, uv(&composition("v1"), "249", None))
        .await
        .expect("create_composition");
    assert_eq!(
        total_count(&pool).await,
        1,
        "the commit after the grant writes its outbox row"
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
    Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(typed(&status)))))
        .await
        .map(|(id, _)| id)
        .expect("first EHR for subject-42");
    let baseline = total_count(&pool).await;
    assert_eq!(baseline, 1, "first EHR creation wrote one row");

    // A second EHR for the SAME subject violates uq_ehr_subject during the
    // EHR_STATUS write, aborting the whole transaction (the commit path,
    // including its outbox insert) — a rolled-back commit.
    Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(typed(&status)))))
        .await
        .map(|(id, _)| id)
        .expect_err("duplicate subject must conflict");
    assert_eq!(
        total_count(&pool).await,
        baseline,
        "a rolled-back commit leaves the outbox unchanged"
    );
}

// ── (3) the feed serves every row once acknowledged, in seq order ────────────

#[tokio::test]
async fn the_feed_serves_every_committed_row_in_seq_order_until_acknowledged() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    reconcile(&pool, &["ferrobridge".to_owned()])
        .await
        .expect("register the reader");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // Commit some work: an EHR + two compositions ⇒ three outbox rows.
    let ehr = crate::fixtures::create_ehr(&svc).await;
    svc.create_composition(ehr, uv(&composition("v1"), "249", None))
        .await
        .expect("comp 1");
    svc.create_composition(ehr, uv(&composition("v2"), "249", None))
        .await
        .expect("comp 2");
    assert_eq!(total_count(&pool).await, 3);

    let events = svc
        .outbox_events(OutboxDomain::Clinical, "ferrobridge", 100)
        .await
        .expect("read the feed");
    assert_eq!(events.len(), 3, "every committed row is served");
    let mut last = 0;
    for event in &events {
        assert!(
            event.seq > last,
            "the feed ascends in seq: {} !> {last}",
            event.seq
        );
        last = event.seq;
        let served = serde_json::to_value(event).expect("serialize the event");
        assert_eq!(
            served["seq"],
            json!(event.seq),
            "seq sits beside the envelope"
        );
        assert_phi_free(&served);
    }

    // Reading again without an acknowledgement serves the same rows.
    let again = svc
        .outbox_events(OutboxDomain::Clinical, "ferrobridge", 100)
        .await
        .expect("read the feed again");
    assert_eq!(again, events, "a read moves no cursor");

    svc.outbox_acknowledge(OutboxDomain::Clinical, "ferrobridge", last)
        .await
        .expect("acknowledge the page");
    let after = svc
        .outbox_events(OutboxDomain::Clinical, "ferrobridge", 100)
        .await
        .expect("read after the acknowledgement");
    assert!(after.is_empty(), "an acknowledged row is not served again");
}

#[tokio::test]
async fn import_writes_one_phi_free_outbox_row() {
    let (_source_db, source) = crate::fixtures::service().await;
    let target_db = testkit::db().await.expect("testkit database");
    let target_pool = target_db.pool();
    let target = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &target_pool,
    ));

    // Seed a source EHR with an EHR_STATUS (EHR creation + the auto EHR_ACCESS).
    let ehr = crate::fixtures::create_ehr(&source).await;
    // Give it a second EHR_STATUS version so the import carries real content.
    let mut status = source
        .ehr_status(ehr, None)
        .await
        .map(|r| r.body)
        .expect("status");
    let ovid = status["uid"]["value"].as_str().expect("uid").to_owned();
    status.as_object_mut().unwrap().remove("uid");
    status["is_modifiable"] = json!(false);
    Box::pin(source.replace_ehr_status(ehr, uv(&status, "251", Some(&ovid))))
        .await
        .map(|m| m.uid)
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
