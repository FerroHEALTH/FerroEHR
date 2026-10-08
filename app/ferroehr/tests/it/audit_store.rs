// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for the local IHE ATNA Audit Record Repository
//! (`ferroehr::system_log::store`) against a real `PostgreSQL` 18
//! (shared testkit harness): the `audit` schema migrates cleanly alongside
//! `ext`/`ehr`, inserted records land with the promoted search columns and
//! the FHIR R4 `AuditEvent` payload (IHE BALP shape), and the retention
//! reaper deletes only rows older than the horizon (0 = keep forever).
//! No openEHR spec governs audit storage — our own design/extension (the
//! schema-separation rationale is BASE
//! `architecture_overview/master07-security.adoc` §Access logging).

use jiff::Timestamp;
use sqlx::Row;

use ferroehr::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::fhir;
use ferroehr::system_log::message::AuditContext;
use ferroehr::system_log::store::AuditStore;

fn ctx() -> AuditContext {
    AuditContext {
        source_id: "ferroehr".to_owned(),
        enterprise_site_id: "site-1".to_owned(),
        server_ip: "10.42.23.77".to_owned(),
        value_if_missing: "UNKNOWN".to_owned(),
    }
}

fn read_event(at: Timestamp) -> AuditEvent {
    let mut e = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut e.user_id);
    e.client_ip = Some("10.0.0.9".to_owned());
    e.object_id = Some("8fa1::ferroehr::1".to_owned());
    e.event_type = Some(EventType::RestOperation("composition_get"));
    e.token_id = Some("jti-1".to_owned());
    e.timestamp = at;
    e
}

#[tokio::test]
async fn insert_persists_promoted_columns_and_fhir_payload() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    let event = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    let rendered = fhir::to_fhir(&event, &ctx(), Some("patient-42")).expect("render");
    store
        .insert(&event, Some("patient-42"), &rendered)
        .await
        .expect("insert");

    let row = sqlx::query(
        "SELECT action, outcome, event_code, operation, principal, patient_id, \
         resource_class, resource_id, client_ip, token_id, fhir, \
         delivered_syslog_at, delivered_fhir_feed_at \
         FROM audit.audit_event",
    )
    .fetch_one(&pool)
    .await
    .expect("one row");

    assert_eq!(row.get::<String, _>("action"), "R");
    assert_eq!(row.get::<i16, _>("outcome"), 0);
    assert_eq!(row.get::<String, _>("event_code"), "110110");
    assert_eq!(
        row.get::<Option<String>, _>("operation").as_deref(),
        Some("composition_get")
    );
    assert_eq!(
        row.get::<Option<String>, _>("principal").as_deref(),
        Some("alice")
    );
    assert_eq!(
        row.get::<Option<String>, _>("patient_id").as_deref(),
        Some("patient-42")
    );
    assert_eq!(row.get::<String, _>("resource_class"), "composition");
    assert_eq!(
        row.get::<Option<String>, _>("resource_id").as_deref(),
        Some("8fa1::ferroehr::1")
    );
    assert_eq!(
        row.get::<Option<String>, _>("client_ip").as_deref(),
        Some("10.0.0.9")
    );
    assert_eq!(
        row.get::<Option<String>, _>("token_id").as_deref(),
        Some("jti-1")
    );
    // Both forwarding outbox stamps start pending.
    assert_eq!(
        row.get::<Option<jiff_sqlx::Timestamp>, _>("delivered_syslog_at")
            .map(jiff_sqlx::Timestamp::to_jiff),
        None
    );

    // The stored payload is the exact rendered BALP AuditEvent.
    let stored: serde_json::Value = row.get("fhir");
    assert_eq!(stored, serde_json::to_value(&rendered).expect("value"));
    assert_eq!(stored["resourceType"], "AuditEvent");
    assert_eq!(
        stored["meta"]["profile"][0],
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientRead"
    );
}

#[tokio::test]
async fn insert_batch_persists_every_record_with_identical_shape() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    // A drained batch: distinct events, one with a subject, one without.
    let first = read_event("2026-07-10T09:00:00Z".parse().unwrap());
    let mut second = read_event("2026-07-10T09:00:01Z".parse().unwrap());
    "bob".clone_into(&mut second.user_id);
    second.object_id = Some("9bb2::ferroehr::1".to_owned());
    let records = vec![
        (
            first.clone(),
            Some("patient-42".to_owned()),
            Some(fhir::to_fhir(&first, &ctx(), Some("patient-42")).expect("render")),
        ),
        (
            second.clone(),
            None,
            Some(fhir::to_fhir(&second, &ctx(), None).expect("render")),
        ),
    ];
    store.insert_batch(&records).await.expect("batch insert");

    let rows = sqlx::query(
        "SELECT principal, patient_id, resource_id, fhir \
         FROM audit.audit_event ORDER BY recorded_at",
    )
    .fetch_all(&pool)
    .await
    .expect("rows");
    assert_eq!(rows.len(), 2, "the whole batch landed");
    assert_eq!(
        rows[0].get::<Option<String>, _>("principal").as_deref(),
        Some("alice")
    );
    assert_eq!(
        rows[0].get::<Option<String>, _>("patient_id").as_deref(),
        Some("patient-42")
    );
    assert_eq!(
        rows[1].get::<Option<String>, _>("principal").as_deref(),
        Some("bob")
    );
    assert_eq!(rows[1].get::<Option<String>, _>("patient_id"), None);
    assert_eq!(
        rows[1].get::<Option<String>, _>("resource_id").as_deref(),
        Some("9bb2::ferroehr::1")
    );
    // The batch path stores the exact same canonical payload as the
    // per-event path.
    let stored: serde_json::Value = rows[0].get("fhir");
    assert_eq!(
        stored,
        serde_json::to_value(&records[0].2).expect("value"),
        "batch-stored FHIR document differs from the rendered one"
    );

    // An empty batch is a no-op, never an error.
    store.insert_batch(&[]).await.expect("empty batch");
}

#[tokio::test]
async fn reap_deletes_only_rows_past_the_horizon() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    // One fresh record and one 40 days old.
    let now = Timestamp::now();
    let old = now - jiff::SignedDuration::from_hours(40 * 24);
    for at in [now, old] {
        let event = read_event(at);
        let rendered = fhir::to_fhir(&event, &ctx(), None).expect("render");
        store.insert(&event, None, &rendered).await.expect("insert");
    }

    // retention 0 = keep forever.
    assert_eq!(store.reap(0).await.expect("reap 0"), 0);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 2);

    // A 30-day horizon reaps exactly the row older than the horizon.
    assert_eq!(store.reap(30).await.expect("reap 30"), 1);
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(remaining, 1);
}

/// A horizon in calendar years (`retention_years`, #3625) reaps what is older
/// than the same calendar date that many years ago, and keeps the chain
/// verifiable across the reaped span.
#[tokio::test]
async fn reap_years_deletes_only_rows_past_the_calendar_horizon() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    // One record 1000 days old (inside three calendar years) and one 1200 days
    // old (outside them).
    let now = Timestamp::now();
    for days in [1000_i64, 1200] {
        let at = now - jiff::SignedDuration::from_hours(days * 24);
        let event = read_event(at);
        let rendered = fhir::to_fhir(&event, &ctx(), None).expect("render");
        store.insert(&event, None, &rendered).await.expect("insert");
    }

    assert_eq!(store.reap_years(0).await.expect("reap 0 years"), 0);
    assert_eq!(store.reap_years(3).await.expect("reap 3 years"), 1);
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(remaining, 1, "the record inside three years survives");
    let findings: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.verify_audit_chain()")
        .fetch_one(&pool)
        .await
        .expect("verify");
    assert_eq!(
        findings, 0,
        "the reaped span is tombstoned, so the chain verifies"
    );
}

/// The access-logging fields (#3156) survive the round trip on both write
/// paths, and the domain is derived from the resource class rather than set by
/// each call site.
///
/// NEN 7513 asks who read which record on whose authority, and EHDS Art. 9
/// requires access to health data to be logged; a column that is written but
/// not read back proves neither.
#[tokio::test]
async fn access_fields_round_trip_on_both_write_paths() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    let mut single = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    single.purpose = Some("TREATMENT".to_owned());
    single.legal_basis = Some("gdpr-art-9-2-h".to_owned());
    single.result_count = Some(17);
    single.request_id = Some("req-single".to_owned());
    let rendered = fhir::to_fhir(&single, &ctx(), Some("patient-42")).expect("render");
    store
        .insert(&single, Some("patient-42"), &rendered)
        .await
        .expect("insert");

    let mut batched = read_event("2026-07-10T08:31:00Z".parse().unwrap());
    batched.object = ObjectClass::Demographic;
    batched.domain = AccessDomain::of(ObjectClass::Demographic);
    batched.purpose = Some("TREATMENT".to_owned());
    batched.result_count = Some(1);
    batched.request_id = Some("req-batch".to_owned());
    let batched_fhir = fhir::to_fhir(&batched, &ctx(), None).expect("render");
    store
        .insert_batch(&[(batched, None, Some(batched_fhir))])
        .await
        .expect("insert batch");

    let rows = sqlx::query(
        "SELECT domain, purpose, legal_basis, result_count, request_id \
         FROM audit.audit_event ORDER BY recorded_at",
    )
    .fetch_all(&pool)
    .await
    .expect("both rows");
    assert_eq!(rows.len(), 2, "one row per write path");

    assert_eq!(
        rows[0].get::<Option<String>, _>("domain").as_deref(),
        Some("clinical")
    );
    assert_eq!(
        rows[0].get::<Option<String>, _>("purpose").as_deref(),
        Some("TREATMENT")
    );
    assert_eq!(
        rows[0].get::<Option<String>, _>("legal_basis").as_deref(),
        Some("gdpr-art-9-2-h")
    );
    assert_eq!(rows[0].get::<Option<i64>, _>("result_count"), Some(17));
    assert_eq!(
        rows[0].get::<Option<String>, _>("request_id").as_deref(),
        Some("req-single")
    );

    // The batched path carries the same fields, and a demographic read is
    // recorded in the demographic domain — the separation the whole
    // pseudonymisation boundary exists to make answerable.
    assert_eq!(
        rows[1].get::<Option<String>, _>("domain").as_deref(),
        Some("party")
    );
    assert_eq!(rows[1].get::<Option<i64>, _>("result_count"), Some(1));
    assert_eq!(
        rows[1].get::<Option<String>, _>("request_id").as_deref(),
        Some("req-batch")
    );
}

/// The accessing organisation survives the round trip on both write paths.
///
/// EHDS Annex II 3.2(a) asks which healthcare provider the access happened on
/// behalf of, beside the natural person of 3.2(b)
/// (<https://eur-lex.europa.eu/eli/reg/2025/327/oj>). The two write paths bind
/// their columns independently, so a field added to one and forgotten in the
/// other is lost exactly on the batched path a loaded server uses.
#[tokio::test]
async fn the_accessing_organisation_round_trips_on_both_write_paths() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    let mut single = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    single.organisation = Some("zh-noordwest".to_owned());
    let rendered = fhir::to_fhir(&single, &ctx(), Some("patient-42")).expect("render");
    store
        .insert(&single, Some("patient-42"), &rendered)
        .await
        .expect("insert");

    let mut batched = read_event("2026-07-10T08:31:00Z".parse().unwrap());
    batched.organisation = Some("huisartsenpost-zuid".to_owned());
    let batched_fhir = fhir::to_fhir(&batched, &ctx(), None).expect("render");
    // A record with no organisation resolved: the column stays NULL rather
    // than carrying the previous row's value or a placeholder.
    let anonymous = read_event("2026-07-10T08:32:00Z".parse().unwrap());
    let anonymous_fhir = fhir::to_fhir(&anonymous, &ctx(), None).expect("render");
    store
        .insert_batch(&[
            (batched, None, Some(batched_fhir)),
            (anonymous, None, Some(anonymous_fhir)),
        ])
        .await
        .expect("insert batch");

    let rows = sqlx::query(
        "SELECT organisation, principal, fhir FROM audit.audit_event ORDER BY recorded_at",
    )
    .fetch_all(&pool)
    .await
    .expect("three rows");
    assert_eq!(rows.len(), 3, "one row per written record");
    assert_eq!(
        rows[0].get::<Option<String>, _>("organisation").as_deref(),
        Some("zh-noordwest"),
        "the per-event path records the organisation"
    );
    assert_eq!(
        rows[1].get::<Option<String>, _>("organisation").as_deref(),
        Some("huisartsenpost-zuid"),
        "the batched path records the organisation"
    );
    assert_eq!(
        rows[1].get::<Option<String>, _>("principal").as_deref(),
        Some("alice"),
        "the batched row's other columns stay aligned with their own record"
    );
    assert_eq!(
        rows[2].get::<Option<String>, _>("organisation"),
        None,
        "an unresolved organisation is recorded as absent, never guessed"
    );

    // The stored FHIR document carries the same fact as the column: the
    // organisation as a second agent referencing an Organization.
    let stored: serde_json::Value = rows[0].get("fhir");
    let agents = stored["agent"].as_array().expect("the agent list");
    assert!(
        agents
            .iter()
            .any(|agent| agent["who"]["reference"] == "Organization/zh-noordwest"),
        "the stored document must carry the organisation agent: {stored}"
    );
}

/// The caller's roles round-trip on both write paths as a JSON array, and an
/// unauthenticated record (no principal) records them as NULL rather than as
/// an empty list, which is the authenticated-but-roleless fact (#3239).
#[tokio::test]
async fn the_actor_roles_round_trip_on_both_write_paths() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    let mut single = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    single.roles = vec!["USER".to_owned(), "clinician".to_owned()];
    let rendered = fhir::to_fhir(&single, &ctx(), Some("patient-42")).expect("render");
    store
        .insert(&single, Some("patient-42"), &rendered)
        .await
        .expect("insert");

    let mut batched = read_event("2026-07-10T08:31:00Z".parse().unwrap());
    batched.roles = vec!["ADMIN".to_owned()];
    let batched_fhir = fhir::to_fhir(&batched, &ctx(), None).expect("render");
    let mut roleless = read_event("2026-07-10T08:32:00Z".parse().unwrap());
    roleless.roles.clear();
    let roleless_fhir = fhir::to_fhir(&roleless, &ctx(), None).expect("render");
    let mut anonymous = read_event("2026-07-10T08:33:00Z".parse().unwrap());
    anonymous.user_id.clear();
    anonymous.roles.clear();
    let anonymous_fhir = fhir::to_fhir(&anonymous, &ctx(), None).expect("render");
    store
        .insert_batch(&[
            (batched, None, Some(batched_fhir)),
            (roleless, None, Some(roleless_fhir)),
            (anonymous, None, Some(anonymous_fhir)),
        ])
        .await
        .expect("insert batch");

    let rows = sqlx::query("SELECT roles, fhir FROM audit.audit_event ORDER BY recorded_at")
        .fetch_all(&pool)
        .await
        .expect("four rows");
    assert_eq!(rows.len(), 4);
    let roles = |i: usize| rows[i].get::<Option<serde_json::Value>, _>("roles");
    assert_eq!(
        roles(0),
        Some(serde_json::json!(["USER", "clinician"])),
        "the per-event path"
    );
    assert_eq!(
        roles(1),
        Some(serde_json::json!(["ADMIN"])),
        "the batched path"
    );
    assert_eq!(
        roles(2),
        Some(serde_json::json!([])),
        "authenticated without roles is the empty list"
    );
    assert_eq!(roles(3), None, "no principal means no roles to record");

    let stored: serde_json::Value = rows[0].get("fhir");
    let requestor = &stored["agent"][0];
    assert_eq!(requestor["role"][1]["text"], "clinician", "{stored}");
}

/// The trail is append-only for the runtime role: the access-logging columns
/// cannot be rewritten, a record cannot be deleted outside the reaper, and the
/// table cannot be truncated.
///
/// A log that can be edited by the component it logs is not evidence. The
/// mechanism is the `audit_event_reject_*` trigger set, and this is the test
/// that it actually refuses — including the columns #3156 added, which the
/// whole-row comparison covers only because it subtracts the two delivery
/// stamps rather than listing what it protects.
#[tokio::test]
async fn the_access_trail_refuses_rewriting_deletion_and_truncation() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    let event = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    let rendered = fhir::to_fhir(&event, &ctx(), Some("patient-42")).expect("render");
    let id = store
        .insert(&event, Some("patient-42"), &rendered)
        .await
        .expect("insert");

    for (what, sql) in [
        (
            "the recorded purpose",
            "UPDATE audit.audit_event SET purpose = 'RESEARCH' WHERE id = $1",
        ),
        (
            "the recorded domain",
            "UPDATE audit.audit_event SET domain = 'system' WHERE id = $1",
        ),
        (
            "the recorded principal",
            "UPDATE audit.audit_event SET principal = 'someone-else' WHERE id = $1",
        ),
        (
            "the record itself",
            "DELETE FROM audit.audit_event WHERE id = $1",
        ),
    ] {
        let refused = sqlx::query(sql).bind(id).execute(&pool).await;
        assert!(
            refused.is_err(),
            "{what} was changed: the access trail must be append-only"
        );
    }

    let truncated = sqlx::query("TRUNCATE audit.audit_event")
        .execute(&pool)
        .await;
    assert!(
        truncated.is_err(),
        "the access trail was truncated: TRUNCATE bypasses row triggers and needs its own refusal"
    );

    // The one sanctioned mutation still works, so the refusals above are
    // specific rather than a blanket lock.
    store.mark_syslog_delivered(id).await;

    let survives: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(survives, 1, "the record survived every attempt");
}

/// The access record's authentication columns (EHDS Annex II 3.1 and 3.2(b),
/// `docs/law/eu/ehds/text.html Annex II 3`): the single and the batched insert
/// persist the acting mode, the assurance level and raw value, and the
/// professional; an event that establishes none of them stores NULLs; and the
/// chain verifies over records carrying them.
#[tokio::test]
async fn both_inserts_persist_the_authentication_columns() {
    use ferroehr::config::auth::AssuranceLevel;
    use ferroehr::system_log::event::{ActingMode, ActorAuthentication};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    let mut client = read_event("2026-07-10T10:00:00Z".parse().unwrap());
    "scheduler-app".clone_into(&mut client.user_id);
    client.actor = ActorAuthentication {
        mode: Some(ActingMode::Client),
        assurance_level: Some(AssuranceLevel::High),
        assurance_value: Some("http://eidas.europa.eu/LoA/high".to_owned()),
        professional_id: Some("prof-007".to_owned()),
    };
    let rendered = fhir::to_fhir(&client, &ctx(), None).expect("render");
    store
        .insert(&client, None, &rendered)
        .await
        .expect("insert");

    let mut person = read_event("2026-07-10T10:00:01Z".parse().unwrap());
    person.actor = ActorAuthentication {
        mode: Some(ActingMode::Person),
        assurance_level: Some(AssuranceLevel::Substantial),
        assurance_value: Some("loa-substantial".to_owned()),
        professional_id: Some("alice".to_owned()),
    };
    let plain = read_event("2026-07-10T10:00:02Z".parse().unwrap());
    let records = vec![
        (
            person.clone(),
            None,
            Some(fhir::to_fhir(&person, &ctx(), None).expect("render")),
        ),
        (
            plain.clone(),
            None,
            Some(fhir::to_fhir(&plain, &ctx(), None).expect("render")),
        ),
    ];
    store.insert_batch(&records).await.expect("batch insert");

    let rows = sqlx::query(
        "SELECT acting_mode, assurance_level, assurance_value, professional_id \
         FROM audit.audit_event ORDER BY chain_seq",
    )
    .fetch_all(&pool)
    .await
    .expect("rows");
    let columns: Vec<[Option<String>; 4]> = rows
        .iter()
        .map(|row| {
            [
                row.get("acting_mode"),
                row.get("assurance_level"),
                row.get("assurance_value"),
                row.get("professional_id"),
            ]
        })
        .collect();
    assert_eq!(
        columns,
        vec![
            [
                Some("client".to_owned()),
                Some("high".to_owned()),
                Some("http://eidas.europa.eu/LoA/high".to_owned()),
                Some("prof-007".to_owned()),
            ],
            [
                Some("person".to_owned()),
                Some("substantial".to_owned()),
                Some("loa-substantial".to_owned()),
                Some("alice".to_owned()),
            ],
            [None, None, None, None],
        ]
    );

    let findings: Vec<String> =
        sqlx::query_scalar("SELECT finding FROM audit.verify_audit_chain()")
            .fetch_all(&pool)
            .await
            .expect("verify");
    assert!(findings.is_empty(), "{findings:?}");
}

/// The emergency-access mark (EHDS Art. 11(5), `docs/law/eu/ehds/text.html
/// Art. 11(5)`): both inserts persist it, an unmarked record stores `false`,
/// the append-only trigger refuses flipping it afterwards, and the chain
/// verifies over marked records.
#[tokio::test]
async fn both_inserts_persist_the_emergency_mark_and_it_cannot_be_rewritten() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = AuditStore::new(pool.clone());

    let mut single = read_event("2026-07-10T11:00:00Z".parse().unwrap());
    single.purpose = Some("ETREAT".to_owned());
    single.emergency_access = true;
    let rendered = fhir::to_fhir(&single, &ctx(), None).expect("render");
    store
        .insert(&single, None, &rendered)
        .await
        .expect("insert");

    let mut marked = read_event("2026-07-10T11:00:01Z".parse().unwrap());
    marked.purpose = Some("ETREAT".to_owned());
    marked.emergency_access = true;
    let mut plain = read_event("2026-07-10T11:00:02Z".parse().unwrap());
    plain.purpose = Some("TREAT".to_owned());
    let records = vec![
        (
            marked.clone(),
            None,
            Some(fhir::to_fhir(&marked, &ctx(), None).expect("render")),
        ),
        (
            plain.clone(),
            None,
            Some(fhir::to_fhir(&plain, &ctx(), None).expect("render")),
        ),
    ];
    store.insert_batch(&records).await.expect("batch insert");

    let marks: Vec<bool> =
        sqlx::query_scalar("SELECT emergency_access FROM audit.audit_event ORDER BY chain_seq")
            .fetch_all(&pool)
            .await
            .expect("rows");
    assert_eq!(marks, vec![true, true, false]);

    let rewrite = sqlx::query(
        "UPDATE audit.audit_event SET emergency_access = NOT emergency_access \
         WHERE purpose = 'TREAT'",
    )
    .execute(&pool)
    .await;
    assert!(rewrite.is_err(), "the mark is append-only: {rewrite:?}");

    let findings: Vec<String> =
        sqlx::query_scalar("SELECT finding FROM audit.verify_audit_chain()")
            .fetch_all(&pool)
            .await
            .expect("verify");
    assert!(findings.is_empty(), "{findings:?}");
}
