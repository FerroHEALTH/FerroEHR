// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The legal marks against a real `PostgreSQL` 18 (shared testkit harness):
//! restriction of processing, the research objection, and the retention
//! register.
//!
//! **No openEHR spec governs any of this — our own design/extension.** What the
//! RM does settle is what these marks may NOT be built out of, and the suite
//! asserts the consequences: `EHR_STATUS.is_queryable` limits population
//! queries only (RM ehr `master04-ehr_package.adoc` §EHR Status), so it cannot
//! stand in for a restriction that also refuses a point read; and content is
//! indelible (RM common `master06-change_control_package.adoc` §Logical
//! Deletion), so a retention period yields a LIST and never a deletion.
//!
//! The obligations under test are GDPR Art. 4(3) and Art. 18 (the mark, and
//! storage as the only processing left), Art. 18(3) (a lift is a later act on
//! a register that still holds the request), Art. 21(6) (the research
//! objection, which reaches research processing and not the care record), and
//! Art. 5(1)(e) with Art. 30(1)(f) (the periods and what has fallen due) —
//! `docs/law/eu/gdpr/text.html`.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use serde_json::json;
use sqlx::PgPool;

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::status::CallStatusType;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::sender;

use crate::fixtures::uv;

/// A fresh repository plus one EHR carrying an `EHR_STATUS` and a directory
/// `FOLDER`, and the `EHR_STATUS` version-container id.
async fn seed(svc: &FerroEhrService) -> (EhrId, String) {
    let ehr_id = svc.create_ehr(None).await.expect("ehr");
    let status = svc
        .get_ehr_status_at_time(ehr_id, None)
        .await
        .expect("status get");
    let status_ovid = status["uid"]["value"].as_str().expect("uid").to_owned();
    let status_vo = status_ovid
        .split("::")
        .next()
        .expect("object id")
        .to_owned();
    svc.create_directory(
        ehr_id,
        uv(
            &json!({
                "_type": "FOLDER",
                "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                "name": { "_type": "DV_TEXT", "value": "root" }
            }),
            "249",
            None,
        ),
    )
    .await
    .expect("directory");
    (ehr_id, status_vo)
}

/// The number of `EHR_STATUS` rows a population query (no `ehr_id` scope) can
/// see.
async fn population_rows(svc: &FerroEhrService) -> usize {
    let result = svc
        .execute_ad_hoc_query(
            "SELECT e/ehr_id/value FROM EHR e".to_owned(),
            AqlQueryRequest::default(),
        )
        .await
        .expect("population query")
        .result_set;
    result["rows"].as_array().map_or(0, Vec::len)
}

/// The rows a query scoped to one EHR can see.
async fn scoped_rows(svc: &FerroEhrService, ehr_id: EhrId) -> usize {
    let result = svc
        .execute_ad_hoc_query(
            "SELECT e/ehr_id/value FROM EHR e".to_owned(),
            AqlQueryRequest {
                ehr_ids: vec![ehr_id.to_string()],
                ..AqlQueryRequest::default()
            },
        )
        .await
        .expect("scoped query")
        .result_set;
    result["rows"].as_array().map_or(0, Vec::len)
}

/// The pending (unpublished) outbox rows of one EHR the drainer would take.
async fn drainable_rows(pool: &PgPool, ehr_id: EhrId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox o \
         WHERE o.ehr_id = $1 AND o.published_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM ehr e WHERE e.id = o.ehr_id \
               AND (e.restricted_at IS NOT NULL \
                    OR (e.research_objected_at IS NOT NULL \
                        AND e.research_objection_ground IS NULL)))",
    )
    .bind(ehr_id)
    .fetch_one(pool)
    .await
    .expect("drainable count")
}

#[tokio::test]
async fn a_restricted_object_is_refused_on_every_read_and_write_path() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let (ehr_id, status_vo) = seed(&svc).await;

    // Before the mark: the EHR_STATUS reads, and the population query sees it.
    let mut update_body = svc
        .get_ehr_status_at_time(ehr_id, None)
        .await
        .expect("an unrestricted status reads");
    let status_ovid = update_body["uid"]["value"]
        .as_str()
        .expect("status uid")
        .to_owned();
    update_body
        .as_object_mut()
        .expect("status object")
        .remove("uid");
    update_body["is_queryable"] = json!(false);
    assert_eq!(
        population_rows(&svc).await,
        1,
        "the EHR is in the population"
    );

    svc.restrict_processing(&ehr_id.to_string(), Some(&status_vo), "gdpr-18-1-a", None)
        .await
        .expect("restrict the EHR_STATUS");

    // The point read is a 403 that NAMES the restriction — not a 404, which
    // would deny the record exists, and not a 403 from the access layer, which
    // would send the caller to fix their credentials.
    let refusal = svc
        .get_ehr_status_at_time(ehr_id, None)
        .await
        .expect_err("a restricted status is refused");
    assert_eq!(refusal.status, CallStatusType::ProcessingRestricted);
    assert!(
        refusal.message.contains("restricted") && refusal.message.contains("Art. 18"),
        "the refusal names the restriction: {}",
        refusal.message
    );

    // The revision history is the same object's change record, refused alike.
    let history = svc
        .ehr_status_revision_history(ehr_id)
        .await
        .expect_err("a restricted object's revision history is refused");
    assert_eq!(history.status, CallStatusType::ProcessingRestricted);

    // A write to it is refused: Art. 18(2) leaves only storage. The body is
    // the one read back before the mark, so the refusal is the restriction's
    // and not a validation failure standing in for it.
    let write = svc
        .replace_ehr_status(ehr_id, uv(&update_body, "251", Some(&status_ovid)))
        .await
        .expect_err("a write to a restricted object is refused");
    assert_eq!(write.status, CallStatusType::ProcessingRestricted);

    // The stored rows are untouched: a restriction limits processing, it does
    // not remove anything.
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM version WHERE vo_id = $1")
        .bind(status_vo.parse::<uuid::Uuid>().expect("vo uuid"))
        .fetch_one(&pool)
        .await
        .expect("version count");
    assert!(versions >= 1, "the stored versions stay in storage");

    // The lift is a later act on the same register, and the read works again.
    svc.lift_restriction(&ehr_id.to_string(), Some(&status_vo))
        .await
        .expect("lift");
    svc.get_ehr_status_at_time(ehr_id, None)
        .await
        .expect("a lifted status reads again");

    // Art. 18(3) needs the sequence, so the register still carries the request.
    let register = svc
        .restriction_register(&ehr_id.to_string())
        .await
        .expect("register");
    assert_eq!(register.len(), 1, "the lift did not erase the request");
    assert_eq!(register[0].ground, "gdpr-18-1-a");
    assert!(
        register[0].lifted_at.is_some(),
        "the lift is recorded on the row it lifted"
    );
}

#[tokio::test]
async fn a_whole_ehr_restriction_reaches_every_object_and_the_event_stream() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let (ehr_id, _status_vo) = seed(&svc).await;

    svc.restrict_processing(
        &ehr_id.to_string(),
        None,
        "gdpr-18-1-b",
        Some("under review"),
    )
    .await
    .expect("restrict the whole EHR");

    assert!(
        svc.get_ehr_status_at_time(ehr_id, None).await.is_err(),
        "the status of a restricted EHR is refused"
    );
    assert!(
        svc.get_directory_at_time(ehr_id, None, None).await.is_err(),
        "the directory of a restricted EHR is refused"
    );
    assert_eq!(
        population_rows(&svc).await,
        0,
        "a restricted EHR contributes no row to a population query"
    );
    assert_eq!(
        scoped_rows(&svc, ehr_id).await,
        0,
        "the restriction is not a population-only gate: it holds under an \
         ehr_id scope too, unlike is_queryable"
    );
    let extract = svc
        .extract_ehrs(ehr_id)
        .await
        .expect_err("an Extract of a restricted EHR is refused");
    assert_eq!(extract.status, CallStatusType::ProcessingRestricted);
    assert_eq!(
        drainable_rows(&pool, ehr_id).await,
        0,
        "no event of a restricted EHR is drainable"
    );

    // Lifting the whole-EHR restriction restores every path.
    svc.lift_restriction(&ehr_id.to_string(), None)
        .await
        .expect("lift");
    svc.get_ehr_status_at_time(ehr_id, None)
        .await
        .expect("the status reads again");
    assert_eq!(population_rows(&svc).await, 1);
    assert!(
        drainable_rows(&pool, ehr_id).await > 0,
        "the withheld events publish once the restriction is lifted"
    );
}

#[tokio::test]
async fn an_object_restriction_survives_a_whole_ehr_lift() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr_id, status_vo) = seed(&svc).await;

    svc.restrict_processing(&ehr_id.to_string(), None, "gdpr-18-1-c", None)
        .await
        .expect("whole-EHR restriction");
    svc.restrict_processing(&ehr_id.to_string(), Some(&status_vo), "gdpr-18-1-a", None)
        .await
        .expect("object restriction");

    svc.lift_restriction(&ehr_id.to_string(), None)
        .await
        .expect("lift the whole-EHR grain only");

    // The directory is free again; the separately restricted status is not.
    svc.get_directory_at_time(ehr_id, None, None)
        .await
        .expect("the directory is no longer restricted");
    assert!(
        svc.get_ehr_status_at_time(ehr_id, None).await.is_err(),
        "the object-scoped restriction stands on its own"
    );
}

#[tokio::test]
async fn an_unknown_target_or_ground_is_refused_before_anything_is_recorded() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr_id, status_vo) = seed(&svc).await;

    let bad_ground = svc
        .restrict_processing(&ehr_id.to_string(), None, "because-we-felt-like-it", None)
        .await
        .expect_err("a ground outside the closed list is refused");
    assert_eq!(bad_ground.status, CallStatusType::PreconditionViolation);

    let unknown_ehr = svc
        .restrict_processing(&uuid::Uuid::now_v7().to_string(), None, "national", None)
        .await
        .expect_err("an unknown EHR is refused");
    assert_eq!(unknown_ehr.status, CallStatusType::EhrIdDoesNotExist);

    let foreign = svc
        .restrict_processing(
            &svc.create_ehr(None).await.expect("second ehr").to_string(),
            Some(&status_vo),
            "national",
            None,
        )
        .await
        .expect_err("an object in another EHR is refused");
    assert_eq!(foreign.status, CallStatusType::VersionedObjectDoesNotExist);

    assert!(
        svc.restriction_register(&ehr_id.to_string())
            .await
            .expect("register")
            .is_empty(),
        "a refused request records nothing"
    );
}

#[tokio::test]
async fn a_research_objection_leaves_the_population_and_the_care_record_alone() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let (ehr_id, _status_vo) = seed(&svc).await;

    assert_eq!(population_rows(&svc).await, 1);

    svc.set_research_objection(&ehr_id.to_string(), true, None)
        .await
        .expect("record the objection");

    assert_eq!(
        population_rows(&svc).await,
        0,
        "an objected EHR leaves the research population (Art. 21(6))"
    );
    assert_eq!(
        scoped_rows(&svc, ehr_id).await,
        1,
        "a query naming the ehr_id is care, not research, and is untouched"
    );
    svc.get_ehr_status_at_time(ehr_id, None)
        .await
        .expect("the treating clinician's read is untouched");
    let extract = svc
        .extract_ehrs(ehr_id)
        .await
        .expect_err("an Extract of an objected EHR is refused");
    assert_eq!(extract.status, CallStatusType::ProcessingRestricted);
    assert!(
        extract.message.contains("Art. 21(6)"),
        "the refusal names the objection rather than a restriction: {}",
        extract.message
    );
    assert_eq!(
        drainable_rows(&pool, ehr_id).await,
        0,
        "no event of an objected EHR is drainable"
    );

    // The controller's Art. 21(6) public-interest override puts it back, and
    // says on whose authority.
    svc.set_research_objection(
        &ehr_id.to_string(),
        true,
        Some("public-health surveillance"),
    )
    .await
    .expect("record the override");
    assert_eq!(population_rows(&svc).await, 1);
    svc.extract_ehrs(ehr_id)
        .await
        .expect("an overridden objection exports again");
    assert!(drainable_rows(&pool, ehr_id).await > 0);

    // Withdrawing carries no ground: there is nothing left to override.
    let bad = svc
        .set_research_objection(&ehr_id.to_string(), false, Some("still important"))
        .await
        .expect_err("a withdrawal with a ground is refused");
    assert_eq!(bad.status, CallStatusType::PreconditionViolation);
    svc.set_research_objection(&ehr_id.to_string(), false, None)
        .await
        .expect("withdraw");
    assert_eq!(population_rows(&svc).await, 1);
}

#[tokio::test]
async fn the_retention_register_lists_what_is_due_and_deletes_nothing() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let (ehr_id, status_vo) = seed(&svc).await;

    // EPDV Art. 10 Abs. 1 lit. d asks a certified community to destroy the
    // medical data of the dossier after twenty years, so the category is the
    // whole EHR and the due row counts every object of it.
    svc.put_retention_policy(
        "EHR",
        "CH",
        "20 years",
        "last_commit",
        "EPDV Art. 10 Abs. 1 lit. d",
    )
    .await
    .expect("declare the CH period");
    let policies = svc.retention_policies().await.expect("register");
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0].source, "EPDV Art. 10 Abs. 1 lit. d");

    // A period the register refuses stays out of it.
    let bad = svc
        .put_retention_policy("SOMETHING_ELSE", "CH", "20 years", "last_commit", "x")
        .await
        .expect_err("an unknown content category is refused");
    assert_eq!(bad.status, CallStatusType::PreconditionViolation);

    // Anchored in the past, the EHR falls due.
    svc.put_retention_anchor(
        &ehr_id.to_string(),
        "CH",
        Some("1990-01-01T00:00:00Z"),
        None,
    )
    .await
    .expect("anchor");
    let due = svc.retention_due(100).await.expect("due list");
    assert_eq!(due.len(), 1, "one category in force, one row");
    assert_eq!(due[0].ehr_id, ehr_id);
    assert_eq!(due[0].source, "EPDV Art. 10 Abs. 1 lit. d");
    assert_eq!(due[0].objects_held, 0);

    // Nothing was deleted by the listing: the record is indelible and disposal
    // is the controller's act.
    svc.get_ehr_status_at_time(ehr_id, None)
        .await
        .expect("the due EHR still serves its content");

    // A per-object hold moves the object from due to held (EPDV Art. 10
    // Abs. 2 lit. b).
    let due_before = due[0].objects_due;
    svc.set_retention_hold(&status_vo, true)
        .await
        .expect("hold");
    let held = svc.retention_due(100).await.expect("due list");
    assert_eq!(held[0].objects_held, 1);
    assert_eq!(held[0].objects_due, due_before - 1);

    // An EHR-wide hold takes the EHR off the list entirely.
    svc.put_retention_anchor(
        &ehr_id.to_string(),
        "CH",
        Some("1990-01-01T00:00:00Z"),
        Some(("2026-09-15T00:00:00Z", "litigation hold")),
    )
    .await
    .expect("EHR-wide hold");
    assert!(
        svc.retention_due(100).await.expect("due list").is_empty(),
        "a held EHR is not due whatever its period says"
    );

    // A hold instant with no ground is refused: a hold nobody can account for
    // is not a record of anything.
    let bad_hold: Result<i64, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO retention_anchor (ehr_id, jurisdiction, hold_at) \
         VALUES ($1, 'CH', now()) RETURNING 1",
    )
    .bind(EhrId(uuid::Uuid::now_v7()))
    .fetch_one(&pool)
    .await;
    assert!(bad_hold.is_err(), "the CHECK refuses a groundless hold");
}

#[tokio::test]
async fn setting_and_lifting_a_mark_is_recorded_in_the_access_trail() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            sgb_v_309_controller: false,
        },
        ..AuditConfig::default()
    };
    let (sender, _handle) = sender::start(config, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender);
    let (ehr_id, _status_vo) = seed(&svc).await;

    svc.restrict_processing(&ehr_id.to_string(), None, "gdpr-18-1-d", None)
        .await
        .expect("restrict");
    svc.lift_restriction(&ehr_id.to_string(), None)
        .await
        .expect("lift");
    svc.set_research_objection(&ehr_id.to_string(), true, None)
        .await
        .expect("object");

    // The sender drains in the background, so the trail is polled rather than
    // read once. These three acts are exactly what a supervisory authority asks
    // about after the fact, so their absence would be the one thing the
    // repository could not account for.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let recorded: Vec<String> = sqlx::query_scalar(
            "SELECT resource_id FROM audit.audit_event \
             WHERE resource_id IS NOT NULL ORDER BY recorded_at",
        )
        .fetch_all(&pool)
        .await
        .expect("audit read");
        let marks: Vec<&String> = recorded
            .iter()
            .filter(|id| id.starts_with("restriction:") || id.starts_with("research-objection:"))
            .collect();
        if marks.len() >= 3 {
            assert_eq!(
                marks
                    .iter()
                    .filter(|id| id.as_str() == "restriction:ehr-wide")
                    .count(),
                2,
                "the restriction and its lift are two acts, both recorded: {marks:?}"
            );
            assert!(
                marks
                    .iter()
                    .any(|id| id.as_str() == "research-objection:ehr-wide"),
                "the objection is recorded: {marks:?}"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the three administrative acts never reached the trail: {recorded:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
