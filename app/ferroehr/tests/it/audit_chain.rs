// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Tamper evidence on the local IHE ATNA Audit Record Repository, and the
//! privilege posture that backs it, against a real `PostgreSQL` 18 (shared
//! testkit harness).
//!
//! No openEHR spec governs audit storage mechanics, database roles or tamper
//! detection — our own design/extension. The control under test is the OWASP
//! Logging Cheat Sheet's §Log Integrity ("build in tamper detection so you know
//! if a record has been modified or deleted"), and these tests assert the
//! property rather than the mechanism: a record that changes is NAMED by
//! verification, a record that disappears is NAMED, and the role the server
//! actually connects as cannot do either — nor run any DDL.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use jiff::Timestamp;
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool, Row};
use uuid::Uuid;

use ferroehr::db;
use ferroehr::system_log::chain_check::{ChainCheck, ChainCheckState};
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::sender;
use ferroehr::system_log::store::{AuditRecord, AuditStore, VerifyScope};
use ferroehr::telemetry::indicators::AuditChainHealth;
use ferrohealth_health::document::State;
use ferrohealth_health::registry::HealthIndicator;

fn read_event(at: Timestamp, subject: &str) -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut event.user_id);
    event.client_ip = Some("10.0.0.9".to_owned());
    event.object_id = Some(subject.to_owned());
    event.event_type = Some(EventType::rest("composition_get"));
    event.timestamp = at;
    event
}

/// Write `count` records through the real store path, oldest first.
async fn seed(store: &AuditStore, count: u32) {
    for n in 0..count {
        let at = Timestamp::now() - jiff::SignedDuration::from_hours(i64::from(count - n));
        let event = read_event(at, &format!("8fa1::ferroehr::{n}"));
        store
            .insert(&event, Some("patient-42"))
            .await
            .expect("insert");
    }
}

/// Run `sql` with the append-only triggers off, which only the table's OWNER
/// can do — the attacker this mechanism is designed to catch rather than stop.
async fn with_triggers_disabled(pool: &PgPool, sql: &str) {
    sqlx::query("ALTER TABLE audit.audit_event DISABLE TRIGGER USER")
        .execute(pool)
        .await
        .expect("disable the append-only triggers");
    sqlx::query(AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .expect("the tampering statement itself must succeed");
    sqlx::query("ALTER TABLE audit.audit_event ENABLE TRIGGER USER")
        .execute(pool)
        .await
        .expect("re-enable the append-only triggers");
}

/// A modified record is detected and named, not silently accepted.
#[tokio::test]
async fn a_modified_audit_record_is_detected_and_named() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    seed(&store, 4).await;

    assert!(
        crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .is_empty(),
        "a freshly written trail must verify clean"
    );

    let target: Uuid = sqlx::query_scalar("SELECT id FROM audit.audit_event WHERE chain_seq = 2")
        .fetch_one(&pool)
        .await
        .expect("the second record");

    // Rewrite who accessed the patient — the exact silent edit the issue is
    // about.
    with_triggers_disabled(
        &pool,
        "UPDATE audit.audit_event SET principal = 'nobody' WHERE chain_seq = 2",
    )
    .await;

    let findings = crate::fixtures::verify_chain(&pool).await.expect("verify");
    assert_eq!(
        findings.len(),
        1,
        "exactly the tampered record must be reported: {findings:?}"
    );
    let finding = findings.first().expect("one finding");
    assert_eq!(finding.chain_seq, Some(2));
    assert_eq!(finding.record_id, Some(target));
    assert!(
        finding.finding.contains("modified"),
        "the finding must say the content changed: {}",
        finding.finding
    );
}

/// A record removed outside retention is detected — in the middle of the trail,
/// and at its end where no successor would notice.
#[tokio::test]
async fn a_deleted_audit_record_is_detected_in_the_middle_and_at_the_end() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    seed(&store, 5).await;

    with_triggers_disabled(&pool, "DELETE FROM audit.audit_event WHERE chain_seq = 3").await;
    let findings = crate::fixtures::verify_chain(&pool).await.expect("verify");
    assert_eq!(findings.len(), 1, "one gap must be reported: {findings:?}");
    let finding = findings.first().expect("one finding");
    assert_eq!(
        finding.chain_seq,
        Some(4),
        "the gap is reported at the record that should have followed the deleted one"
    );
    assert!(
        finding.finding.contains("deleted"),
        "the finding must say a record went missing: {}",
        finding.finding
    );

    // The newest record has no successor to notice its removal; the recorded
    // chain head does.
    with_triggers_disabled(&pool, "DELETE FROM audit.audit_event WHERE chain_seq = 5").await;
    let findings = crate::fixtures::verify_chain(&pool).await.expect("verify");
    assert!(
        findings
            .iter()
            .any(|f| f.finding.contains("end of the chain")),
        "removing the newest record must be reported: {findings:?}"
    );
}

/// Retention reaping stays legitimate: it deletes exactly what the horizon
/// says, and the surviving trail still verifies.
#[tokio::test]
async fn retention_reaping_leaves_a_verifiable_trail() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());

    let now = Timestamp::now();
    for at in [
        now - jiff::SignedDuration::from_hours(60 * 24),
        now - jiff::SignedDuration::from_hours(40 * 24),
        now,
    ] {
        let event = read_event(at, "8fa1::ferroehr::1");
        store.insert(&event, None).await.expect("insert");
    }

    assert_eq!(store.reap(30).await.expect("reap"), 2);
    assert!(
        crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .is_empty(),
        "a reaped trail must still verify: the removal is recorded"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(remaining, 1);

    // And a deletion on top of the reaping is still caught — the recorded
    // removal covers only what retention actually took.
    with_triggers_disabled(&pool, "DELETE FROM audit.audit_event WHERE chain_seq = 3").await;
    assert!(
        !crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .is_empty(),
        "an unrecorded deletion after reaping must still be detected"
    );
}

/// The triggers refuse the ordinary rewrite paths outright, while the one
/// permitted change — the forwarding delivery stamp — still works.
#[tokio::test]
async fn the_audit_table_refuses_mutation_deletion_and_truncation() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    seed(&store, 2).await;

    for statement in [
        "UPDATE audit.audit_event SET principal = 'nobody'",
        "UPDATE audit.audit_event SET category_content = '{}'::jsonb",
        "UPDATE audit.audit_render_policy SET policy = '{}'::jsonb",
        "DELETE FROM audit.audit_render_policy",
        "UPDATE audit.audit_event SET row_hash = '\\x00'::bytea",
        "DELETE FROM audit.audit_event",
        "TRUNCATE audit.audit_event",
    ] {
        let outcome = sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&pool)
            .await;
        assert!(
            outcome.is_err(),
            "the append-only table must refuse `{statement}`"
        );
    }

    sqlx::query("UPDATE audit.audit_event SET delivered_syslog_at = now()")
        .execute(&pool)
        .await
        .expect("stamping a record as forwarded is the one permitted change");
    assert!(
        crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .is_empty(),
        "a delivery stamp must not disturb the chain"
    );
}

/// The privilege posture of the role the running server connects as, over the
/// audit trail specifically: it can record an event and stamp it forwarded, and
/// it can do nothing else — no DDL, no rewrite, no deletion, and no disabling
/// of the triggers that enforce that.
///
/// Roles are cluster-global on the shared harness server, so the login role is
/// named off the clone database for the testkit sweep to reap.
#[tokio::test]
async fn the_application_role_cannot_run_ddl_or_rewrite_the_audit_trail() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let role = format!("{}_auditrole", testdb.name());
    sqlx::query(AssertSqlSafe(format!(
        "CREATE ROLE {role} LOGIN PASSWORD 'testpw' IN ROLE ferroehr_clinical"
    )))
    .execute(&pool)
    .await
    .expect("create the app-privilege role");

    let (scheme, rest) = testdb.url().split_once("://").expect("dsn scheme");
    let tail = rest.split_once('@').map_or(rest, |(_, t)| t);
    let mut conn = PgConnection::connect(&format!("{scheme}://{role}:testpw@{tail}"))
        .await
        .expect("connect as the app role");

    // DDL, in every direction an application-level SQL flaw could take it.
    for statement in [
        "CREATE TABLE audit.zq_probe (id int)",
        "DROP TABLE audit.audit_event",
        "ALTER TABLE audit.audit_event ADD COLUMN zq_probe int",
        "ALTER TABLE audit.audit_event DISABLE TRIGGER USER",
        "DROP TRIGGER audit_event_chain_link ON audit.audit_event",
        "CREATE INDEX zq_probe ON audit.audit_event (chain_seq)",
        "CREATE SCHEMA zq_probe",
        "DROP FUNCTION audit.verify_audit_chain()",
    ] {
        let outcome = sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&mut conn)
            .await;
        assert!(
            outcome.is_err(),
            "the application role must not be able to run `{statement}`"
        );
    }

    // Recording an event is exactly what it may do.
    sqlx::query(
        "INSERT INTO audit.audit_event \
         (recorded_at, action, outcome, event_code, resource_class, render_policy, \
          search_tokens) \
         VALUES (now(), 'R', 0, '110110', 'composition', \
                 audit.register_render_policy('{}'::jsonb), '{}')",
    )
    .execute(&mut conn)
    .await
    .expect("the application role must be able to record an audit event");
    sqlx::query(
        "INSERT INTO audit.audit_search_token (parameter, system, code, digest) \
         VALUES ('type', NULL, 'rest', audit.search_token_digest('type', NULL, 'rest'))",
    )
    .execute(&mut conn)
    .await
    .expect("the application role must be able to store a search token");

    // Rewriting or removing one is not.
    for statement in [
        "UPDATE audit.audit_event SET principal = 'nobody'",
        "UPDATE audit.audit_event SET row_hash = '\\x00'::bytea",
        "DELETE FROM audit.audit_event",
        "TRUNCATE audit.audit_event",
        "UPDATE audit.audit_chain_state SET head_hash = '\\x00'::bytea",
        "INSERT INTO audit.audit_chain_gap (from_seq, to_seq, link_hash) \
         VALUES (1, 99, '\\x00'::bytea)",
        "INSERT INTO audit.audit_render_policy (digest, policy) \
         VALUES (sha256(convert_to('{}', 'UTF8')), '{}'::jsonb)",
        "UPDATE audit.audit_render_policy SET policy = '{\"source_id\": \"x\"}'::jsonb",
        "DELETE FROM audit.audit_render_policy",
        "UPDATE audit.audit_search_token SET code = 'someone-else'",
        "DELETE FROM audit.audit_search_token",
        "INSERT INTO audit.audit_search_token (parameter, system, code, digest) \
         VALUES ('type', NULL, 'read', '\\x00'::bytea)",
        "DELETE FROM audit.audit_search_set",
    ] {
        let outcome = sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&mut conn)
            .await;
        assert!(
            outcome.is_err(),
            "the application role must not be able to run `{statement}`"
        );
    }

    // …while the forwarding stamp and the ITI-81 read the server performs work.
    sqlx::query("UPDATE audit.audit_event SET delivered_fhir_feed_at = now()")
        .execute(&mut conn)
        .await
        .expect("the application role must be able to stamp delivery");
    let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&mut conn)
        .await
        .expect("the application role must be able to read the audit trail");
    assert_eq!(recorded, 1);

    // And it can run the verification itself, which is how an operator asks the
    // question without a privileged credential.
    let findings: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.verify_audit_chain()")
        .fetch_one(&mut conn)
        .await
        .expect("the application role must be able to verify the chain");
    assert_eq!(findings, 0);
}

/// `[db].migrate = "verify"` issues no DDL: it accepts a database migrated to
/// exactly this build and refuses one that is not, naming the schema.
#[tokio::test]
async fn verify_mode_accepts_a_migrated_database_and_refuses_a_stale_one() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();

    let mut settings = db::DbConfig::new(testdb.url());
    settings.migrate = db::MigrationMode::Verify;
    db::prepare(
        &settings,
        &db::domain::StorageConfig::default(),
        &crate::fixtures::shared_pools(&pool),
        ferroehr::config::deployment::DeploymentProfile::Sandbox,
    )
    .await
    .expect("a fully migrated database must satisfy verify mode");

    // A schema this build owns but the database has never seen.
    sqlx::query("DROP SCHEMA audit CASCADE")
        .execute(&pool)
        .await
        .expect("drop one migration set");

    let error = db::prepare(
        &settings,
        &db::domain::StorageConfig::default(),
        &crate::fixtures::shared_pools(&pool),
        ferroehr::config::deployment::DeploymentProfile::Sandbox,
    )
    .await
    .expect_err("verify mode must refuse an unmigrated schema");
    assert!(
        matches!(
            error,
            db::DbError::SchemaNotReady(db::SchemaMismatch::NeverMigrated { .. })
        ),
        "the refusal must be the typed one: {error}"
    );
    let divergence = std::error::Error::source(&error)
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(
        divergence.contains("audit"),
        "the refusal's divergence must name the schema: {error}: {divergence}"
    );

    // Apply mode is the zero-config path, and it repairs what verify refused.
    settings.migrate = db::MigrationMode::Apply;
    db::prepare(
        &settings,
        &db::domain::StorageConfig::default(),
        &crate::fixtures::shared_pools(&pool),
        ferroehr::config::deployment::DeploymentProfile::Sandbox,
    )
    .await
    .expect("apply mode migrates");
    db::verify_migrations(&pool)
        .await
        .expect("and the database then verifies");
}

/// A migration recorded from different source text is a stale database, not an
/// acceptable one: verify mode refuses it rather than serving against a schema
/// it cannot reason about.
#[tokio::test]
async fn verify_mode_refuses_a_database_whose_migration_text_diverged() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();

    sqlx::query("UPDATE audit._sqlx_migrations SET checksum = '\\x00'::bytea WHERE version = 1")
        .execute(&pool)
        .await
        .expect("forge the recorded checksum");

    let error = db::verify_migrations(&pool)
        .await
        .expect_err("a diverged migration must be refused");
    assert!(
        matches!(
            error,
            db::DbError::SchemaNotReady(db::SchemaMismatch::ChecksumMismatch { .. })
        ),
        "the refusal must be the typed one: {error}"
    );
}

/// The batched drain path writes through the same trigger, so a batch is
/// chained record by record rather than left unprotected.
#[tokio::test]
async fn the_batched_write_path_is_chained_too() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());

    let mut batch = Vec::new();
    for n in 0..5_u32 {
        let event = read_event(Timestamp::now(), &format!("8fa1::ferroehr::{n}"));
        batch.push(AuditRecord {
            event,
            subject: None,
        });
    }
    store.insert_batch(&batch, &[]).await.expect("batch insert");

    let positions: Vec<i64> =
        sqlx::query("SELECT chain_seq FROM audit.audit_event ORDER BY chain_seq")
            .fetch_all(&pool)
            .await
            .expect("positions")
            .into_iter()
            .map(|row| row.get::<i64, _>("chain_seq"))
            .collect();
    assert_eq!(positions, vec![1, 2, 3, 4, 5]);
    assert!(
        crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .is_empty()
    );
}

/// The scheduled check reports a tampered record on the management health
/// surface (CRA Annex I Part I(2)(f), "report on corruptions"): an intact trail
/// reads `UP`, and once a record is rewritten the indicator reads `DOWN` and
/// names the damaged position.
#[tokio::test]
async fn the_chain_check_reports_a_tampered_record_on_the_health_surface() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    seed(&store, 3).await;

    let check = ChainCheck::default();
    let indicator = AuditChainHealth::new(check.clone());
    assert!(matches!(check.state(), ChainCheckState::Pending));
    assert_eq!(indicator.check().await.state(), State::Up);

    assert!(matches!(
        check.run_scope(&store, VerifyScope::Full).await,
        ChainCheckState::Intact { .. }
    ));
    assert_eq!(indicator.check().await.state(), State::Up);

    with_triggers_disabled(
        &pool,
        "UPDATE audit.audit_event SET principal = 'nobody' WHERE chain_seq = 2",
    )
    .await;

    let damaged = check.run_scope(&store, VerifyScope::Full).await;
    assert!(
        matches!(
            &damaged,
            ChainCheckState::Damaged { total: 1, findings, .. }
                if findings.first().and_then(|f| f.chain_seq) == Some(2)
        ),
        "exactly the tampered record must be reported: {damaged:?}"
    );
    let health = indicator.check().await;
    assert_eq!(
        health.state(),
        State::Degraded,
        "damage is reported and never flips readiness"
    );
    let detail = health.detail().expect("a damaged chain names the finding");
    assert!(detail.contains("position 2"), "{detail}");
    assert!(detail.contains("modified"), "{detail}");
}

/// The audit subsystem schedules the check whenever the local store is on, so
/// a deployment gets it without configuring anything beyond the interval.
#[tokio::test]
async fn the_audit_subsystem_schedules_the_chain_check() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            verify_interval_seconds: 1,
            ..StoreConfig::default()
        },
        ..AuditConfig::default()
    };
    let (sender, handle) = sender::start(config, None, Some(pool.clone()))
        .await
        .expect("start the audit subsystem");
    let check = handle
        .chain_check()
        .expect("the local store is on, so the check is scheduled");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while matches!(check.state(), ChainCheckState::Pending) {
        assert!(
            std::time::Instant::now() < deadline,
            "the scheduled check must run within its interval"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        matches!(check.state(), ChainCheckState::Intact { .. }),
        "{:?}",
        check.state()
    );

    let off = AuditConfig {
        enabled: true,
        store: StoreConfig {
            verify_interval_seconds: 0,
            ..StoreConfig::default()
        },
        ..AuditConfig::default()
    };
    let (off_sender, off_handle) = sender::start(off, None, Some(pool.clone()))
        .await
        .expect("start the audit subsystem");
    assert!(
        off_handle.chain_check().is_none(),
        "0 turns the schedule off"
    );

    drop(sender);
    drop(off_sender);
    handle.shutdown(std::time::Duration::from_secs(5)).await;
    off_handle.shutdown(std::time::Duration::from_secs(5)).await;
}

/// The columns no chain digest hashes: the digest itself and the two forwarding
/// stamps, the one permitted change to a stored record.
const UNHASHED_COLUMNS: [&str; 3] = ["row_hash", "delivered_syslog_at", "delivered_fhir_feed_at"];

/// Every column of `audit.audit_event` with its `information_schema` data type,
/// in table order.
async fn audit_event_columns(pool: &PgPool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT column_name::text, data_type::text FROM information_schema.columns \
         WHERE table_schema = 'audit' AND table_name = 'audit_event' \
         ORDER BY ordinal_position",
    )
    .fetch_all(pool)
    .await
    .expect("read the audit_event columns")
}

/// Run `statements` with the user triggers off inside a transaction, read the
/// verifier's findings as `(chain_seq, finding)`, and roll everything back.
async fn findings_after(pool: &PgPool, statements: &[String]) -> Vec<(Option<i64>, String)> {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("ALTER TABLE audit.audit_event DISABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("disable the append-only triggers");
    for statement in statements {
        sqlx::query(AssertSqlSafe(statement.clone()))
            .execute(&mut *tx)
            .await
            .expect("the tampering statement itself must succeed");
    }
    let findings: Vec<(Option<i64>, String)> =
        sqlx::query_as("SELECT chain_seq, finding FROM audit.verify_audit_chain()")
            .fetch_all(&mut *tx)
            .await
            .expect("verify");
    tx.rollback().await.expect("rollback");
    findings
}

/// Returns an `UPDATE` right-hand side that changes a value of `data_type`,
/// NULL included, or `None` for a type this helper does not know yet.
fn changed_value(column: &str, data_type: &str) -> Option<String> {
    let value = match data_type {
        "text" => format!("coalesce(\"{column}\", '') || 'x'"),
        "jsonb" => format!("jsonb_build_array(\"{column}\")"),
        "smallint" | "integer" | "bigint" => format!("coalesce(\"{column}\", 0) + 1000"),
        "uuid" => "uuidv7()".to_owned(),
        "timestamp with time zone" => {
            format!("coalesce(\"{column}\", now()) + interval '1 second'")
        }
        "boolean" => format!("NOT coalesce(\"{column}\", false)"),
        "date" => format!("coalesce(\"{column}\", current_date) + 1"),
        "bytea" => format!("coalesce(\"{column}\", '\\x'::bytea) || '\\x00'::bytea"),
        "ARRAY" => format!("array_append(\"{column}\", 0)"),
        _ => return None,
    };
    Some(value)
}

/// The digest lists every column of the record but the unhashed three, so a
/// migration that adds a column fails here until it lists the column.
#[tokio::test]
async fn the_digest_covers_every_column() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();

    let mut covered: Vec<String> =
        sqlx::query_scalar("SELECT unnest(audit.audit_event_digest_columns())")
            .fetch_all(&pool)
            .await
            .expect("the current digest's columns");
    covered.sort();

    let mut expected: Vec<String> = audit_event_columns(&pool)
        .await
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| !UNHASHED_COLUMNS.contains(&name.as_str()))
        .collect();
    expected.sort();

    assert_eq!(
        covered, expected,
        "every column of audit.audit_event outside {UNHASHED_COLUMNS:?} must be in the \
         chain digest: list a new column in audit.audit_event_digest_columns"
    );
}

/// Changing any hashed column of a stored record, with the append-only triggers
/// and the CHECK constraints out of the way, is reported by the verifier.
#[tokio::test]
async fn every_hashed_column_is_tamper_evident() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    seed(&store, 3).await;

    assert!(
        crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .is_empty(),
        "a freshly written trail must verify clean"
    );

    // The owner can drop a CHECK or the render policy's foreign key as easily
    // as disable a trigger, and a value the constraints refuse is still a
    // change the chain must see.
    let checks: Vec<String> = sqlx::query_scalar(
        "SELECT conname::text FROM pg_constraint \
         WHERE conrelid = 'audit.audit_event'::regclass AND contype IN ('c', 'f')",
    )
    .fetch_all(&pool)
    .await
    .expect("the CHECK constraints");

    for (column, data_type) in audit_event_columns(&pool).await {
        if UNHASHED_COLUMNS.contains(&column.as_str()) {
            continue;
        }
        let mut statements: Vec<String> = checks
            .iter()
            .map(|name| format!("ALTER TABLE audit.audit_event DROP CONSTRAINT \"{name}\""))
            .collect();
        let value = changed_value(&column, &data_type);
        assert!(
            value.is_some(),
            "no tampering expression for column `{column}` of type `{data_type}`: extend \
             changed_value so the new column is exercised"
        );
        let value = value.expect("checked above");
        statements.push(format!(
            "UPDATE audit.audit_event SET \"{column}\" = {value} WHERE chain_seq = 2"
        ));
        let findings = findings_after(&pool, &statements).await;
        assert!(
            !findings.is_empty(),
            "changing `{column}` of a stored record must be reported by verify_audit_chain()"
        );
    }
}

/// A render policy changed after records name it changes what they render,
/// so the verifier reports it, and a record whose policy is gone is reported
/// on its own position.
#[tokio::test]
async fn a_changed_or_missing_render_policy_is_reported() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    seed(&store, 2).await;
    assert!(
        crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .is_empty()
    );

    let changed = policy_findings_after(
        &pool,
        &[
            "ALTER TABLE audit.audit_render_policy DROP CONSTRAINT ck_audit_render_policy_digest"
                .to_owned(),
            "UPDATE audit.audit_render_policy SET policy = \
             jsonb_set(policy, '{source_id}', '\"someone-else\"')"
                .to_owned(),
        ],
    )
    .await;
    assert!(
        changed
            .iter()
            .any(|(seq, finding)| seq.is_none() && finding.contains("render policy")),
        "a changed render policy must be reported: {changed:?}"
    );

    let missing = policy_findings_after(
        &pool,
        &[
            "ALTER TABLE audit.audit_event DROP CONSTRAINT fk_audit_event_render_policy".to_owned(),
            "DELETE FROM audit.audit_render_policy".to_owned(),
        ],
    )
    .await;
    assert!(
        missing
            .iter()
            .any(|(seq, finding)| *seq == Some(1) && finding.contains("missing")),
        "a record naming a deleted render policy must be reported: {missing:?}"
    );
}

/// A search token or set changed after records name it changes which records
/// a search finds, so the verifier reports it.
#[tokio::test]
async fn a_changed_search_token_or_set_is_reported() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    seed(&store, 2).await;
    assert!(
        crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .is_empty()
    );

    let token = findings_after(
        &pool,
        &[
            "ALTER TABLE audit.audit_search_token DISABLE TRIGGER USER".to_owned(),
            "ALTER TABLE audit.audit_search_token DROP CONSTRAINT ck_audit_search_token_digest"
                .to_owned(),
            "UPDATE audit.audit_search_token SET code = 'someone-else' \
             WHERE parameter = 'agent.identifier'"
                .to_owned(),
        ],
    )
    .await;
    assert!(
        token
            .iter()
            .any(|(seq, finding)| seq.is_none() && finding.contains("search token")),
        "a changed search token must be reported: {token:?}"
    );

    let set = findings_after(
        &pool,
        &[
            "ALTER TABLE audit.audit_search_set DISABLE TRIGGER USER".to_owned(),
            "ALTER TABLE audit.audit_search_set DROP CONSTRAINT ck_audit_search_set_digest"
                .to_owned(),
            "UPDATE audit.audit_search_set SET tokens = tokens[2:]".to_owned(),
        ],
    )
    .await;
    assert!(
        set.iter()
            .any(|(seq, finding)| seq.is_none() && finding.contains("search set")),
        "a changed search set must be reported: {set:?}"
    );
}

/// [`findings_after`] for statements over the render policy table, whose
/// append-only triggers are switched off with the record table's.
async fn policy_findings_after(pool: &PgPool, statements: &[String]) -> Vec<(Option<i64>, String)> {
    let mut all = vec!["ALTER TABLE audit.audit_render_policy DISABLE TRIGGER USER".to_owned()];
    all.extend_from_slice(statements);
    findings_after(pool, &all).await
}

/// The position the verification checkpoint has reached, `None` before any
/// intact run.
async fn checkpoint(pool: &PgPool) -> Option<i64> {
    sqlx::query_scalar("SELECT through_seq FROM audit.audit_chain_checkpoint")
        .fetch_optional(pool)
        .await
        .expect("the checkpoint")
}

/// An incremental run verifies the records written after the last intact
/// link and moves the checkpoint past them; a change to a record behind the
/// checkpoint is the full pass's to find, and a change after it holds the
/// checkpoint where it is until it is resolved.
#[tokio::test]
async fn an_incremental_run_verifies_from_the_checkpoint() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    seed(&store, 3).await;

    assert_eq!(checkpoint(&pool).await, None);
    let first = store
        .verify_chain_advancing(VerifyScope::SinceCheckpoint)
        .await
        .expect("verify");
    assert!(first.is_empty(), "{first:?}");
    assert_eq!(checkpoint(&pool).await, Some(3));

    with_triggers_disabled(
        &pool,
        "UPDATE audit.audit_event SET principal = 'nobody' WHERE chain_seq = 2",
    )
    .await;
    assert!(
        store
            .verify_chain_advancing(VerifyScope::SinceCheckpoint)
            .await
            .expect("verify")
            .is_empty(),
        "a record behind the checkpoint is outside an incremental run"
    );
    let full = store
        .verify_chain_advancing(VerifyScope::Full)
        .await
        .expect("verify");
    assert!(
        full.iter()
            .any(|f| f.chain_seq == Some(2) && f.finding.contains("modified")),
        "the full pass finds it: {full:?}"
    );
    assert_eq!(
        crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .len(),
        full.len()
    );

    seed(&store, 2).await;
    with_triggers_disabled(
        &pool,
        "UPDATE audit.audit_event SET principal = 'nobody' WHERE chain_seq = 5",
    )
    .await;
    let found = store
        .verify_chain_advancing(VerifyScope::SinceCheckpoint)
        .await
        .expect("verify");
    assert!(
        found
            .iter()
            .any(|f| f.chain_seq == Some(5) && f.finding.contains("modified")),
        "a record after the checkpoint is verified: {found:?}"
    );
    assert_eq!(
        checkpoint(&pool).await,
        Some(3),
        "a run that finds damage leaves the checkpoint"
    );
}

/// An incremental run checks the link from the checkpoint and the chain
/// head: a record deleted right after the checkpoint, and one deleted from
/// the end, are both reported.
#[tokio::test]
async fn an_incremental_run_detects_deletions_after_the_checkpoint() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());
    seed(&store, 2).await;
    assert!(
        store
            .verify_chain_advancing(VerifyScope::SinceCheckpoint)
            .await
            .expect("verify")
            .is_empty()
    );
    seed(&store, 3).await;

    with_triggers_disabled(&pool, "DELETE FROM audit.audit_event WHERE chain_seq = 3").await;
    let middle = store
        .verify_chain_advancing(VerifyScope::SinceCheckpoint)
        .await
        .expect("verify");
    assert!(
        middle
            .iter()
            .any(|f| f.chain_seq == Some(4) && f.finding.contains("deleted between")),
        "{middle:?}"
    );

    with_triggers_disabled(&pool, "DELETE FROM audit.audit_event WHERE chain_seq = 5").await;
    let end = store
        .verify_chain_advancing(VerifyScope::SinceCheckpoint)
        .await
        .expect("verify");
    assert!(
        end.iter()
            .any(|f| f.finding.contains("deleted from the end of the chain")),
        "{end:?}"
    );
    assert_eq!(checkpoint(&pool).await, Some(2));
}

/// Retention reaping past the checkpoint record leaves a trail an
/// incremental run still verifies: the tombstone reaping merged across the
/// checkpoint accounts for the gap.
#[tokio::test]
async fn an_incremental_run_verifies_across_a_reaped_checkpoint() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AuditStore::new(pool.clone());

    let now = Timestamp::now();
    let insert = async |at: Timestamp| {
        let event = read_event(at, "8fa1::ferroehr::1");
        store.insert(&event, None).await.expect("insert");
    };
    insert(now - jiff::SignedDuration::from_hours(60 * 24)).await;
    insert(now - jiff::SignedDuration::from_hours(50 * 24)).await;
    assert!(
        store
            .verify_chain_advancing(VerifyScope::SinceCheckpoint)
            .await
            .expect("verify")
            .is_empty()
    );
    assert_eq!(checkpoint(&pool).await, Some(2));
    insert(now - jiff::SignedDuration::from_hours(40 * 24)).await;
    insert(now).await;

    assert_eq!(store.reap(30).await.expect("reap"), 3);
    let after = store
        .verify_chain_advancing(VerifyScope::SinceCheckpoint)
        .await
        .expect("verify");
    assert!(after.is_empty(), "{after:?}");
    assert_eq!(checkpoint(&pool).await, Some(4));
    assert!(
        crate::fixtures::verify_chain(&pool)
            .await
            .expect("verify")
            .is_empty()
    );
}
