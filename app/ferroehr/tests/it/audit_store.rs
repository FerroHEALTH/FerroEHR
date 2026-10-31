// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for the local IHE ATNA Audit Record Repository
//! (`ferroehr::system_log::store`) against a real `PostgreSQL` 18
//! (shared testkit harness): the `audit` schema migrates cleanly alongside
//! `ext`/`ehr`, inserted records land with their columns and render the FHIR
//! R4 `AuditEvent` (IHE BALP shape) the drain would have rendered, and the retention
//! reaper deletes only rows older than the horizon (0 = keep forever).
//! No openEHR spec governs audit storage — our own design/extension (the
//! schema-separation rationale is BASE
//! `architecture_overview/master07-security.adoc` §Access logging).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only reaches \
              `#[test]`-annotated functions, so it misses this module's helpers; panicking \
              assertions are the intended shape here (the Rust Book ch11)"
)]

use jiff::Timestamp;
use sqlx::Row;

use ferroehr::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::fhir;
use ferroehr::system_log::message::AuditContext;
use ferroehr::system_log::render::RenderPolicy;
use ferroehr::system_log::store::{AuditRecord, AuditSearchFilter, AuditStore};

fn policy() -> RenderPolicy {
    RenderPolicy {
        source_id: "ferroehr".to_owned(),
        enterprise_site_id: "site-1".to_owned(),
        server_host: "10.42.23.77".to_owned(),
        value_if_missing: "UNKNOWN".to_owned(),
        ..RenderPolicy::default()
    }
}

/// The context the drain renders with under [`policy`].
fn ctx() -> AuditContext {
    policy().compile().expect("the policy compiles").context
}

/// A store writing under [`policy`].
fn store_over(pool: &sqlx::PgPool) -> AuditStore {
    AuditStore::new(pool.clone()).with_render_policy(policy())
}

/// Every stored record's FHIR document as the store serves it, oldest first.
async fn served(store: &AuditStore) -> Vec<serde_json::Value> {
    let (_, mut documents) = store
        .search(&AuditSearchFilter {
            count: 1000,
            ..AuditSearchFilter::default()
        })
        .await
        .expect("search");
    documents.reverse();
    documents
}

fn record(event: &AuditEvent, subject: Option<&str>) -> AuditRecord {
    AuditRecord {
        event: event.clone(),
        subject: subject.map(str::to_owned),
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
    e.event_type = Some(EventType::rest("composition_get"));
    e.token_id = Some("jti-1".to_owned());
    e.timestamp = at;
    e
}

#[tokio::test]
async fn insert_persists_the_columns_and_serves_the_rendered_document() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = store_over(&pool);

    let event = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    store
        .insert(&event, Some("patient-42"))
        .await
        .expect("insert");

    let row = sqlx::query(
        "SELECT action, outcome, event_code, operation, principal, patient_id, \
         resource_class, resource_id, client_ip, token_id, \
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

    // The served document is the BALP AuditEvent the drain renders.
    let rendered = fhir::to_fhir(&event, &ctx(), Some("patient-42")).expect("render");
    let stored = served(&store).await.remove(0);
    assert_eq!(stored, rendered);
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
    let store = store_over(&pool);

    // A drained batch: distinct events, one with a subject, one without.
    let first = read_event("2026-07-10T09:00:00Z".parse().expect("timestamp"));
    let mut second = read_event("2026-07-10T09:00:01Z".parse().unwrap());
    "bob".clone_into(&mut second.user_id);
    second.object_id = Some("9bb2::ferroehr::1".to_owned());
    let records = vec![record(&first, Some("patient-42")), record(&second, None)];
    store.insert_batch(&records).await.expect("batch insert");

    let rows = sqlx::query(
        "SELECT principal, patient_id, resource_id \
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
    // A batch-written record serves the document the drain renders, as a
    // record written by the per-event path does.
    let documents = served(&store).await;
    assert_eq!(
        documents,
        vec![
            fhir::to_fhir(&first, &ctx(), Some("patient-42")).expect("render"),
            fhir::to_fhir(&second, &ctx(), None).expect("render"),
        ],
        "a batch-written record serves a document other than the rendered one"
    );

    // An empty batch is a no-op, never an error.
    store.insert_batch(&[]).await.expect("empty batch");
}

/// A token the store already holds draws no new identity value, and the
/// search token index takes new entries through its bounded pending list
/// (PostgreSQL 18 docs, "GIN Fast Update Technique").
#[tokio::test]
async fn repeated_batches_store_each_token_once_and_index_through_the_pending_list() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = store_over(&pool);

    let event = read_event("2026-07-10T09:00:00Z".parse().expect("timestamp"));
    for _ in 0..3 {
        store
            .insert_batch(&[record(&event, Some("patient-42"))])
            .await
            .expect("batch insert");
    }

    let (count, highest): (i64, Option<i32>) =
        sqlx::query_as("SELECT count(*), max(id) FROM audit.audit_search_token")
            .fetch_one(&pool)
            .await
            .expect("tokens");
    assert!(count > 0, "the batch stored its search tokens");
    assert_eq!(
        highest.map(i64::from),
        Some(count),
        "a held token spent an identity value"
    );

    let options: Vec<String> = sqlx::query_scalar(
        "SELECT unnest(reloptions) FROM pg_class \
         WHERE oid = 'audit.ix_audit_event_search_tokens'::regclass ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .expect("index options");
    assert_eq!(options, ["fastupdate=on", "gin_pending_list_limit=512"]);
}

#[tokio::test]
async fn reap_deletes_only_rows_past_the_horizon() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = store_over(&pool);

    // One fresh record and one 40 days old.
    let now = Timestamp::now();
    let old = now - jiff::SignedDuration::from_hours(40 * 24);
    for at in [now, old] {
        let event = read_event(at);
        store.insert(&event, None).await.expect("insert");
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
    let store = store_over(&pool);

    // One record 1000 days old (inside three calendar years) and one 1200 days
    // old (outside them).
    let now = Timestamp::now();
    for days in [1000_i64, 1200] {
        let at = now - jiff::SignedDuration::from_hours(days * 24);
        let event = read_event(at);
        store.insert(&event, None).await.expect("insert");
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

/// Each record is deleted on or after its own retention end and never before
/// it (Regulation (EU) 2025/327 Annex II 3.4): two records written the same
/// moment with different ends part ways, a record kept without end stays, and
/// a record that states no end keeps the store horizon.
#[tokio::test]
async fn reap_due_deletes_each_record_at_its_own_end() {
    use ferroehr::system_log::config::Retention;
    use ferroehr::system_log::store::RecordEnd;

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = store_over(&pool);

    let today: jiff::civil::Date = "2026-10-26".parse().unwrap();
    let written: Timestamp = "2026-10-01T08:00:00Z".parse().unwrap();
    let records: Vec<_> = (0..3).map(|_| record(&read_event(written), None)).collect();
    let ends = [
        RecordEnd::On(today),
        RecordEnd::On(today.tomorrow().unwrap()),
        RecordEnd::Never,
    ];
    store
        .insert_batch_ending(&records, &ends)
        .await
        .expect("batch insert with ends");
    // A record from before per-record ends states none.
    let legacy = read_event("2020-01-01T00:00:00Z".parse().unwrap());
    store
        .insert_batch(&[record(&legacy, None)])
        .await
        .expect("legacy insert");

    // Kept forever, the horizon spares the record that states no end; the
    // record whose end is today goes, the one ending tomorrow and the one
    // kept without end stay.
    assert_eq!(
        store
            .reap_due(today, Retention::Forever)
            .await
            .expect("reap"),
        1
    );
    let ends_left: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT retention_ends::text FROM audit.audit_event \
         WHERE retention_stated ORDER BY chain_seq",
    )
    .fetch_all(&pool)
    .await
    .expect("the stated records");
    assert_eq!(ends_left, vec![Some("2026-10-27".to_owned()), None]);

    // Reaping again the same day removes nothing more; the next day the
    // second record's end has come, and a horizon reaps the unstated record.
    assert_eq!(
        store
            .reap_due(today, Retention::Forever)
            .await
            .expect("reap"),
        0
    );
    assert_eq!(
        store
            .reap_due(today.tomorrow().unwrap(), Retention::Days(30))
            .await
            .expect("reap"),
        2
    );
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(left, 1, "only the record kept without end remains");
    assert!(
        store.verify_chain().await.expect("verify").is_empty(),
        "the reaped positions are tombstoned, so the chain verifies"
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
    let store = store_over(&pool);

    let mut single = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    single.purpose = Some("TREATMENT".to_owned());
    single.legal_basis = Some("gdpr-art-9-2-h".to_owned());
    single.result_count = Some(17);
    single.request_id = Some("req-single".to_owned());
    store
        .insert(&single, Some("patient-42"))
        .await
        .expect("insert");

    let mut batched = read_event("2026-07-10T08:31:00Z".parse().unwrap());
    batched.object = ObjectClass::Demographic;
    batched.domain = AccessDomain::of(ObjectClass::Demographic);
    batched.purpose = Some("TREATMENT".to_owned());
    batched.result_count = Some(1);
    batched.request_id = Some("req-batch".to_owned());
    store
        .insert_batch(&[record(&batched, None)])
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
    let store = store_over(&pool);

    let mut single = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    single.organisation = Some("zh-noordwest".to_owned());
    store
        .insert(&single, Some("patient-42"))
        .await
        .expect("insert");

    let mut batched = read_event("2026-07-10T08:31:00Z".parse().unwrap());
    batched.organisation = Some("huisartsenpost-zuid".to_owned());
    // A record with no organisation resolved: the column stays NULL rather
    // than carrying the previous row's value or a placeholder.
    let anonymous = read_event("2026-07-10T08:32:00Z".parse().unwrap());
    store
        .insert_batch(&[record(&batched, None), record(&anonymous, None)])
        .await
        .expect("insert batch");

    let rows =
        sqlx::query("SELECT organisation, principal FROM audit.audit_event ORDER BY recorded_at")
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

    // The served FHIR document carries the same fact as the column: the
    // organisation as an agent of its own, a logical reference to an
    // Organization (FHIR R4 `Reference.type` and `Reference.identifier`).
    let stored = served(&store).await.remove(0);
    let agents = stored["agent"].as_array().expect("the agent list");
    assert!(
        agents.iter().any(|agent| {
            agent["who"]["type"] == "Organization"
                && agent["who"]["identifier"]["value"] == "zh-noordwest"
        }),
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
    let store = store_over(&pool);

    let mut single = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    single.roles = vec!["USER".to_owned(), "clinician".to_owned()];
    store
        .insert(&single, Some("patient-42"))
        .await
        .expect("insert");

    let mut batched = read_event("2026-07-10T08:31:00Z".parse().unwrap());
    batched.roles = vec!["ADMIN".to_owned()];
    let mut roleless = read_event("2026-07-10T08:32:00Z".parse().unwrap());
    roleless.roles.clear();
    let mut anonymous = read_event("2026-07-10T08:33:00Z".parse().unwrap());
    anonymous.user_id.clear();
    anonymous.roles.clear();
    store
        .insert_batch(&[
            record(&batched, None),
            record(&roleless, None),
            record(&anonymous, None),
        ])
        .await
        .expect("insert batch");

    let rows = sqlx::query("SELECT roles FROM audit.audit_event ORDER BY recorded_at")
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

    let stored = served(&store).await.remove(0);
    let requestor = stored["agent"]
        .as_array()
        .expect("the agent list")
        .iter()
        .find(|agent| agent["requestor"] == true)
        .expect("the agent:user");
    assert_eq!(
        requestor["role"][1]["coding"][0]["code"], "clinician",
        "{stored}"
    );
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
    let store = store_over(&pool);

    let event = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    let id = store
        .insert(&event, Some("patient-42"))
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
    let seq: i64 = sqlx::query_scalar("SELECT chain_seq FROM audit.audit_event WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("the record's chain position");
    store
        .mark_fhir_feed_delivered(seq)
        .await
        .expect("a delivery stamp is the one sanctioned mutation");

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
    use ferroehr::system_log::event::{ActingMode, ActorAuthentication};
    use ferrohealth_smart::assurance::AssuranceLevel;

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = store_over(&pool);

    let mut client = read_event("2026-07-10T10:00:00Z".parse().unwrap());
    "scheduler-app".clone_into(&mut client.user_id);
    client.actor = ActorAuthentication {
        mode: Some(ActingMode::Client),
        assurance_level: Some(AssuranceLevel::High),
        assurance_value: Some("http://eidas.europa.eu/LoA/high".to_owned()),
        professional_id: Some("prof-007".to_owned()),
    };
    store.insert(&client, None).await.expect("insert");

    let mut person = read_event("2026-07-10T10:00:01Z".parse().unwrap());
    person.actor = ActorAuthentication {
        mode: Some(ActingMode::Person),
        assurance_level: Some(AssuranceLevel::Substantial),
        assurance_value: Some("loa-substantial".to_owned()),
        professional_id: Some("alice".to_owned()),
    };
    let plain = read_event("2026-07-10T10:00:02Z".parse().unwrap());
    let records = vec![record(&person, None), record(&plain, None)];
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
    let store = store_over(&pool);

    let mut single = read_event("2026-07-10T11:00:00Z".parse().unwrap());
    single.purpose = Some("ETREAT".to_owned());
    single.emergency_access = true;
    store.insert(&single, None).await.expect("insert");

    let mut marked = read_event("2026-07-10T11:00:01Z".parse().unwrap());
    marked.purpose = Some("ETREAT".to_owned());
    marked.emergency_access = true;
    let mut plain = read_event("2026-07-10T11:00:02Z".parse().unwrap());
    plain.purpose = Some("TREAT".to_owned());
    let records = vec![record(&marked, None), record(&plain, None)];
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

/// The policy of [`a_record_renders_the_document_the_drain_rendered`]: a
/// category map, an emergency purpose and a ten-year horizon, so every part
/// of the EHDS access record has something to write.
fn classifying_policy() -> RenderPolicy {
    let config: ferroehr::system_log::config::AuditConfig = toml::from_str(
        r#"
        source_id = "ferroehr"
        enterprise_site_id = "site-1"
        server_host = "10.42.23.77"
        purpose_codes = ["TREAT", "ETREAT"]
        emergency_purpose_codes = ["ETREAT"]
        [store]
        retention_years = 10
        [categories.templates]
        "Lab Report" = ["Laboratory-Reports"]
        "#,
    )
    .expect("parse");
    RenderPolicy::of(&config)
}

/// One event of every rendering path, each field filled that the path reads.
fn every_kind_of_record(
    compiled: &ferroehr::system_log::render::CompiledPolicy,
) -> Vec<(AuditEvent, Option<String>)> {
    use ferroehr::system_log::categories::{AccessedContent, ContentIds, QueryConstraints};
    use ferroehr::system_log::event::{ActingMode, ActorAuthentication};
    use ferrohealth_smart::assurance::AssuranceLevel;

    let at = |text: &str| text.parse::<Timestamp>().expect("timestamp");
    let mut classified = read_event(at("2026-07-10T08:30:00.123456Z"));
    classified.issuer = Some("https://idp.example.org".to_owned());
    classified.client_id = Some("portal".to_owned());
    classified.organisation = Some("zh-noordwest".to_owned());
    classified.roles = vec!["clinician".to_owned()];
    classified.ehr_id = Some("7d44b88c-4199-4bad-97dc-d78268e01398".to_owned());
    classified.subject_namespace = Some("urn:oid:2.999.1.1".to_owned());
    classified.purpose = Some("ETREAT".to_owned());
    classified.emergency_access = true;
    classified.permit = Some("permit-1".to_owned());
    classified.legal_basis = Some("gdpr-art-9-2-h".to_owned());
    classified.request_id = Some("req-1".to_owned());
    classified.result_count = Some(1);
    classified.record_origins(vec!["lab-system".to_owned(), "gp-system".to_owned()], None);
    classified.actor = ActorAuthentication {
        mode: Some(ActingMode::Client),
        assurance_level: Some(AssuranceLevel::High),
        assurance_value: Some("http://eidas.europa.eu/LoA/high".to_owned()),
        professional_id: Some("prof-007".to_owned()),
    };
    classified.category = Some(compiled.categories.classify(
        &AccessedContent::Objects(vec![ContentIds {
            template_id: Some("Lab Report".to_owned()),
            archetype_id: Some("openEHR-EHR-COMPOSITION.report.v1".to_owned()),
            resource_kind: None,
        }]),
        false,
    ));
    classified.category_map_digest = Some(compiled.categories.digest());

    let mut query = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Query,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut query.user_id);
    query.event_type = Some(EventType::rest("query_execute_ad_hoc"));
    query.object_id = Some("SELECT c FROM EHR e CONTAINS COMPOSITION c".to_owned());
    query.ehr_id = Some("7d44b88c-4199-4bad-97dc-d78268e01398".to_owned());
    query.subject_lookup_failed = true;
    query.timestamp = at("2026-07-10T08:30:01.000001Z");
    query.category = Some(compiled.categories.classify(
        &AccessedContent::Query(QueryConstraints {
            reaches_content: true,
            ..QueryConstraints::default()
        }),
        false,
    ));
    query.category_map_digest = Some(compiled.categories.digest());

    let mut login = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Authentication,
        EventOutcome::MinorFailure,
    );
    login.event_type = Some(EventType::Login);
    login.client_ip = Some("10.0.0.9".to_owned());
    login.timestamp = at("2026-07-10T08:30:02Z");

    let mut failed = read_event(at("2026-07-10T08:30:03Z"));
    failed.outcome = EventOutcome::SeriousFailure;

    let mut template = AuditEvent::new(
        EventActionCode::Create,
        ObjectClass::Template,
        EventOutcome::Success,
    );
    "admin".clone_into(&mut template.user_id);
    template.roles = vec!["ADMIN".to_owned()];
    template.event_type = Some(EventType::rest("definition_template_adl1_4_upload"));
    template.object_id = Some("Lab Report".to_owned());
    template.timestamp = at("2026-07-10T08:30:04Z");

    let mut extract = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Extract,
        EventOutcome::Success,
    );
    extract.object_id = Some("extract-1".to_owned());
    extract.ehr_id = Some("7d44b88c-4199-4bad-97dc-d78268e01398".to_owned());
    extract.timestamp = at("2026-07-10T08:30:05Z");

    vec![
        (classified, Some("patient-42".to_owned())),
        (query, None),
        (login, None),
        (failed, Some("patient-42".to_owned())),
        (template, None),
        (extract, Some("patient-42".to_owned())),
    ]
}

/// A stored record renders, byte for byte, the FHIR `AuditEvent` the drain
/// rendered from the event before the store kept it (#4300), on both write
/// paths, and renders it again the same way on every read.
#[tokio::test]
async fn a_record_renders_the_document_the_drain_rendered() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let policy = classifying_policy();
    let compiled = policy.compile().expect("compile");
    let store = AuditStore::new(pool.clone()).with_render_policy(policy);

    let records = every_kind_of_record(&compiled);
    let (first, rest) = records.split_first().expect("records");
    store
        .insert(&first.0, first.1.as_deref())
        .await
        .expect("insert");
    let batch: Vec<AuditRecord> = rest
        .iter()
        .map(|(event, subject)| record(event, subject.as_deref()))
        .collect();
    store.insert_batch(&batch).await.expect("batch insert");

    let expected: Vec<Vec<u8>> = records
        .iter()
        .map(|(event, subject)| {
            let document =
                fhir::to_fhir(event, &compiled.context, subject.as_deref()).expect("render");
            serde_json::to_vec(&document).expect("bytes")
        })
        .collect();
    let bytes = |documents: Vec<serde_json::Value>| -> Vec<Vec<u8>> {
        documents
            .iter()
            .map(|document| serde_json::to_vec(document).expect("bytes"))
            .collect()
    };
    let first_read = bytes(served(&store).await);
    for (index, (served, rendered)) in first_read.iter().zip(&expected).enumerate() {
        assert_eq!(
            String::from_utf8_lossy(served),
            String::from_utf8_lossy(rendered),
            "record {index} serves a document other than the one the drain rendered"
        );
    }
    assert_eq!(first_read.len(), expected.len());

    // A second reader, with a cold policy cache, renders the same bytes.
    let reader = AuditStore::new(pool.clone());
    assert_eq!(
        bytes(served(&reader).await),
        first_read,
        "the rendering moved"
    );
}

/// A record renders under the policy it was written under, so a store that
/// now writes under another configuration still serves the old record's
/// document unchanged.
#[tokio::test]
async fn a_record_keeps_the_policy_it_was_written_under() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let before = store_over(&pool);
    let event = read_event("2026-07-10T08:30:00Z".parse().unwrap());
    before
        .insert(&event, Some("patient-42"))
        .await
        .expect("insert");

    let moved = AuditStore::new(pool.clone()).with_render_policy(RenderPolicy {
        source_id: "renamed".to_owned(),
        ..policy()
    });
    moved
        .insert(&read_event("2026-07-10T08:31:00Z".parse().unwrap()), None)
        .await
        .expect("insert under the new policy");

    let documents = served(&moved).await;
    assert_eq!(
        documents[0],
        fhir::to_fhir(&event, &ctx(), Some("patient-42")).expect("render")
    );
    let policies: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_render_policy")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(policies, 2, "each configuration is kept once");
}

/// Every value, system and system-value pair of `document`, as the token
/// candidates of [`every_search_parameter_matches_what_the_document_says`].
fn candidates(
    document: &serde_json::Value,
    values: &mut std::collections::BTreeSet<String>,
    pairs: &mut std::collections::BTreeSet<(String, String)>,
) {
    match document {
        serde_json::Value::Object(members) => {
            let text = |key: &str| members.get(key).and_then(serde_json::Value::as_str);
            for key in ["value", "code", "address"] {
                if let Some(value) = text(key) {
                    values.insert(value.to_owned());
                    if let Some(system) = text("system") {
                        pairs.insert((system.to_owned(), value.to_owned()));
                    }
                }
            }
            for member in members.values() {
                candidates(member, values, pairs);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                candidates(item, values, pairs);
            }
        }
        _ => {}
    }
}

/// Every ITI-81 parameter on an element of the document finds through the
/// search token index exactly the records whose served document the profile's
/// matching selects: the three token forms of FHIR R4 Search §3.1.1.4.10 and
/// the `address` substring of `RESTful` ATNA §3.81.4.1.2.2 (#4313).
#[tokio::test]
async fn every_search_parameter_matches_what_the_document_says() {
    use ferroehr::system_log::store::{AuditCriterion, AuditElement, AuditToken, AuditTokenSystem};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let policy = classifying_policy();
    let compiled = policy.compile().expect("compile");
    let store = AuditStore::new(pool.clone()).with_render_policy(policy);
    let records: Vec<AuditRecord> = every_kind_of_record(&compiled)
        .iter()
        .map(|(event, subject)| record(event, subject.as_deref()))
        .collect();
    store.insert_batch(&records).await.expect("batch insert");

    let documents = served(&store).await;
    let mut values = std::collections::BTreeSet::new();
    let mut pairs = std::collections::BTreeSet::new();
    for document in &documents {
        candidates(document, &mut values, &mut pairs);
    }
    values.insert("not-in-any-record".to_owned());
    let systems: std::collections::BTreeSet<String> =
        pairs.iter().map(|(system, _)| system.clone()).collect();

    let token = |system: AuditTokenSystem, code: Option<&str>| AuditToken {
        system,
        code: code.map(str::to_owned),
    };
    let mut criteria = Vec::new();
    for element in [
        AuditElement::AgentIdentifier,
        AuditElement::PatientIdentifier,
        AuditElement::EntityIdentifier,
        AuditElement::SourceIdentifier,
        AuditElement::EntityType,
        AuditElement::EntityRole,
        AuditElement::Type,
        AuditElement::Subtype,
    ] {
        for value in &values {
            criteria.push(AuditCriterion::Element(
                element,
                vec![token(AuditTokenSystem::Any, Some(value))],
            ));
            criteria.push(AuditCriterion::Element(
                element,
                vec![token(AuditTokenSystem::Absent, Some(value))],
            ));
        }
        for (system, value) in &pairs {
            criteria.push(AuditCriterion::Element(
                element,
                vec![token(AuditTokenSystem::Named(system.clone()), Some(value))],
            ));
        }
        for system in &systems {
            criteria.push(AuditCriterion::Element(
                element,
                vec![token(AuditTokenSystem::Named(system.clone()), None)],
            ));
        }
    }
    for value in [
        "10.0.0",
        "10.42.23.77",
        "UNKNOWN",
        "unknown",
        "no-such-host",
    ] {
        criteria.push(AuditCriterion::Address(vec![value.to_owned()]));
    }

    let mut matched = 0;
    for criterion in criteria {
        let mut expected: Vec<String> = documents
            .iter()
            .filter(|document| criterion.matches(document))
            .map(ToString::to_string)
            .collect();
        let (total, found) = store
            .search(&AuditSearchFilter {
                count: 1000,
                criteria: vec![criterion.clone()],
                ..AuditSearchFilter::default()
            })
            .await
            .expect("search");
        let mut found: Vec<String> = found.iter().map(ToString::to_string).collect();
        expected.sort();
        found.sort();
        assert_eq!(
            usize::try_from(total).expect("a count"),
            expected.len(),
            "{criterion:?}"
        );
        assert_eq!(found, expected, "{criterion:?}");
        matched += usize::from(!expected.is_empty());
    }
    assert!(
        matched > 40,
        "the candidates exercise matches, not only misses: {matched}"
    );
}

/// A classified access under [`classifying_policy`] with its own request id
/// and object, of `action` (a read or a create).
fn classified_access(
    compiled: &ferroehr::system_log::render::CompiledPolicy,
    action: EventActionCode,
    request_id: &str,
    object_id: &str,
) -> AuditEvent {
    use ferroehr::system_log::categories::{AccessedContent, ContentIds};

    let mut event = read_event("2026-07-10T09:00:00Z".parse().expect("timestamp"));
    event.action = action;
    event.request_id = Some(request_id.to_owned());
    event.object_id = Some(object_id.to_owned());
    event.category = Some(compiled.categories.classify(
        &AccessedContent::Objects(vec![ContentIds {
            template_id: Some("Lab Report".to_owned()),
            archetype_id: Some("openEHR-EHR-COMPOSITION.report.v1".to_owned()),
            resource_kind: None,
        }]),
        false,
    ));
    event.category_map_digest = Some(compiled.categories.digest());
    event
}

/// The ids of every stored search token.
async fn token_ids(pool: &sqlx::PgPool) -> Vec<i32> {
    sqlx::query_scalar("SELECT id FROM audit.audit_search_token ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("tokens")
}

/// Whether `criterion` finds any record, after asserting that the search finds
/// exactly the records whose served document the profile's matching selects.
async fn finds_what_the_documents_say(
    store: &AuditStore,
    documents: &[serde_json::Value],
    criterion: ferroehr::system_log::store::AuditCriterion,
) -> bool {
    let mut expected: Vec<String> = documents
        .iter()
        .filter(|document| criterion.matches(document))
        .map(ToString::to_string)
        .collect();
    let (total, found) = store
        .search(&AuditSearchFilter {
            count: 1000,
            criteria: vec![criterion.clone()],
            ..AuditSearchFilter::default()
        })
        .await
        .expect("search");
    let mut found: Vec<String> = found.iter().map(ToString::to_string).collect();
    expected.sort();
    found.sort();
    assert_eq!(
        usize::try_from(total).expect("a count"),
        expected.len(),
        "{criterion:?}"
    );
    assert_eq!(found, expected, "{criterion:?}");
    !expected.is_empty()
}

/// A record stores no search token for its request id or for the version uid
/// it created, so the token table stays flat across new requests (#4524), and
/// ITI-81 `entity.identifier` (`RESTful` ATNA §3.81.4.1.2.2) still finds each
/// record by both, in the three token forms of FHIR R4 Search §3.1.1.4.10.
#[tokio::test]
async fn request_ids_and_version_uids_add_no_token_and_stay_searchable() {
    use ferroehr::system_log::store::{AuditCriterion, AuditElement, AuditToken, AuditTokenSystem};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let policy = classifying_policy();
    let compiled = policy.compile().expect("compile");
    let store = AuditStore::new(pool.clone()).with_render_policy(policy);
    let access = |action, n: usize| {
        record(
            &classified_access(
                &compiled,
                action,
                &format!("req-{n}"),
                &format!("8fa1::ferroehr::{n}"),
            ),
            Some("patient-42"),
        )
    };

    store
        .insert_batch(&[
            access(EventActionCode::Read, 1),
            access(EventActionCode::Create, 2),
        ])
        .await
        .expect("first batch");
    let before = token_ids(&pool).await;
    store
        .insert_batch(&[
            access(EventActionCode::Read, 3),
            access(EventActionCode::Create, 4),
        ])
        .await
        .expect("second batch");
    store
        .insert(
            &classified_access(
                &compiled,
                EventActionCode::Read,
                "req-5",
                "8fa1::ferroehr::5",
            ),
            Some("patient-42"),
        )
        .await
        .expect("single insert");
    assert_eq!(
        token_ids(&pool).await,
        before,
        "a new request or version uid added a search token"
    );
    let named: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit.audit_search_token \
         WHERE code LIKE 'req-%' OR code LIKE '8fa1::ferroehr::%'",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(named, 0, "a request id or version uid is stored as a token");

    let documents = served(&store).await;
    let token = |system: AuditTokenSystem, code: Option<&str>| AuditToken {
        system,
        code: code.map(str::to_owned),
    };
    let object_ids = AuditTokenSystem::Named("urn:openehr:object-id".to_owned());
    let mut criteria = Vec::new();
    for n in 1..=5 {
        for value in [format!("req-{n}"), format!("8fa1::ferroehr::{n}")] {
            for system in [
                AuditTokenSystem::Any,
                AuditTokenSystem::Absent,
                object_ids.clone(),
            ] {
                criteria.push(vec![token(system, Some(&value))]);
            }
        }
    }
    criteria.push(vec![token(object_ids.clone(), None)]);
    criteria.push(vec![
        token(AuditTokenSystem::Any, Some("req-1")),
        token(object_ids, Some("8fa1::ferroehr::4")),
    ]);
    let mut matched = 0;
    for tokens in criteria {
        let criterion = AuditCriterion::Element(AuditElement::EntityIdentifier, tokens);
        matched += usize::from(finds_what_the_documents_say(&store, &documents, criterion).await);
    }
    // Of five records, each request id alone and without a system, each uid
    // alone and in its system; then the system alone and the pair.
    assert_eq!(matched, 5 * 4 + 2, "the searches exercise matches");
    assert!(store.verify_chain().await.expect("verify").is_empty());
}

/// Retention removes the search tokens no remaining record names, among them
/// the request id token of a record written before #4524, keeps a token a
/// live record names, and the chain verifies records written before and
/// after; outside reaping no token can be deleted, and within it no token a
/// record names.
#[tokio::test]
async fn retention_removes_the_tokens_only_its_records_named() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let store = store_over(&pool);
    store
        .insert(&read_event(Timestamp::now()), Some("patient-42"))
        .await
        .expect("insert");
    let live: i32 = sqlx::query_scalar(
        "SELECT t FROM audit.audit_event, unnest(search_tokens) AS t WHERE t > 0 LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("a token of the live record");

    // A record as 0017 wrote it: its request id is a token of its own.
    let own: i32 = sqlx::query_scalar(
        "INSERT INTO audit.audit_search_token (parameter, system, code, digest) \
         VALUES ('entity.identifier', NULL, 'req-before', \
                 audit.search_token_digest('entity.identifier', NULL, 'req-before')) \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("the old token");
    sqlx::query(
        "INSERT INTO audit.audit_event \
         (recorded_at, action, outcome, event_code, resource_class, request_id, \
          render_policy, search_tokens) \
         SELECT now() - interval '40 days', 'R', 0, '110110', 'composition', 'req-before', \
                render_policy, ARRAY[$1, $2] \
           FROM audit.audit_event LIMIT 1",
    )
    .bind(own)
    .bind(live)
    .execute(&pool)
    .await
    .expect("the old record");
    assert!(store.verify_chain().await.expect("verify").is_empty());

    let mut tx = pool.begin().await.expect("tx");
    let unreaped = sqlx::query("DELETE FROM audit.audit_search_token WHERE id = $1")
        .bind(own)
        .execute(&mut *tx)
        .await;
    assert!(unreaped.is_err(), "a token was deleted outside reaping");
    tx.rollback().await.expect("rollback");
    let mut tx = pool.begin().await.expect("tx");
    sqlx::query("SELECT set_config('ferroehr.audit_reaping', 'on', true)")
        .execute(&mut *tx)
        .await
        .expect("flag");
    let named = sqlx::query("DELETE FROM audit.audit_search_token WHERE id = $1")
        .bind(live)
        .execute(&mut *tx)
        .await;
    assert!(named.is_err(), "a token a live record names was deleted");
    tx.rollback().await.expect("rollback");

    assert_eq!(store.reap(30).await.expect("reap"), 1);
    let left = token_ids(&pool).await;
    assert!(!left.contains(&own), "the reaped record's own token stayed");
    assert!(
        left.contains(&live),
        "a token a live record names was removed"
    );
    assert!(store.verify_chain().await.expect("verify").is_empty());
}
