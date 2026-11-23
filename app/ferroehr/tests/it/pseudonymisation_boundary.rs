// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pseudonymisation boundary, exercised the way production runs it: the
//! cutover, the runtime roles, the service's routing, and the boot gate.
//!
//! NOTE: no openEHR spec governs storage layout or database roles — our own
//! design/extension (GDPR Art. 4(5) and Art. 32(1)(a); the migrations carry
//! the derivation).
//!
//! The cutover tests come first. The testkit clones a template that is already
//! fully migrated, so every other DB test meets the demographic schema empty
//! and the data move runs against nothing. That is precisely the half
//! production does not have: an installation upgrading into this release
//! carries parties in `ehr`, and the move is the only thing that carries them
//! across. Those tests therefore reconstruct the pre-move state with raw SQL
//! inside the migrated clone and re-run the cutover statements against it, so
//! the move is proven on data rather than on an empty schema.
//!
//! The rest prove the boundary holds afterwards: that no runtime role can
//! read a single relation in a domain it does not own (connecting as each one,
//! over relations enumerated from `information_schema` rather than a
//! hand-written list), that a party committed through the service seam lands in
//! `demographic` and nowhere else, that the boot self-check refuses a
//! database whose grants cross the boundary and passes once they do not.

use ferroehr::service::version_update::WriteRequest;
use sqlx::{Connection, PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::typed_body::typed;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;

/// Write the audit and contribution rows every version row needs, leaving the
/// boundary constraints exactly as the migrated clone carries them.
///
/// Returns `(contribution_id, commit_audit_id)`.
async fn change_control(pool: &PgPool, schema: &str, ehr_id: Option<Uuid>) -> (Uuid, Uuid) {
    let (commit_audit_id, contribution_id) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "WITH s AS (INSERT INTO {schema}.commit_audit_shared (digest, system_id) \
                    VALUES (sha256(gen_random_uuid()::text::bytea), 'test.system') RETURNING id) \
         INSERT INTO {schema}.commit_audit_facts (id, shared, change_type, committer) \
         SELECT $1, s.id, '249', \
                '{{\"_type\":\"PARTY_IDENTIFIED\",\"name\":\"tester\"}}'::jsonb FROM s"
    )))
    .bind(commit_audit_id)
    .execute(pool)
    .await
    .expect("seed audit");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {schema}.contribution (id, ehr_id, commit_audit_id) VALUES ($1, $2, $3)"
    )))
    .bind(contribution_id)
    .bind(ehr_id)
    .bind(commit_audit_id)
    .execute(pool)
    .await
    .expect("seed contribution");
    (contribution_id, commit_audit_id)
}

async fn count(pool: &PgPool, sql: &'static str, vo_id: Uuid) -> i64 {
    sqlx::query(sql)
        .bind(vo_id)
        .fetch_one(pool)
        .await
        .expect("count")
        .try_get::<i64, _>(0)
        .expect("count column")
}
#[tokio::test]
async fn the_boundary_refuses_a_party_written_back_to_the_clinical_schema() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    // Change control first, then the write a code path that missed the split
    // would attempt. The boundary constraint is left in place: this asserts the
    // clone the testkit hands every other test already carries it.
    // The contribution needs a real EHR: its own half of the boundary already
    // refuses an EHR-less one, which is the constraint the sibling test covers.
    let ehr_id = Uuid::now_v7();
    sqlx::query("INSERT INTO clinical.ehr (id, system_id) VALUES ($1, 'test.system')")
        .bind(ehr_id)
        .execute(&pool)
        .await
        .expect("seed an EHR");
    let (contribution_id, commit_audit_id) = change_control(&pool, "clinical", Some(ehr_id)).await;
    let refused = sqlx::query(
        "INSERT INTO clinical.version \
           (vo_id, kind, ehr_id, sys_version, trunk_version, committed_at, \
            creating_system_id, contribution_id, commit_audit_id, body) \
         VALUES ($1, 'PERSON', NULL, 1, 1, now(), 'test.system', $2, $3, '{}')",
    )
    .bind(Uuid::now_v7())
    .bind(contribution_id)
    .bind(commit_audit_id)
    .execute(&pool)
    .await;

    let error = refused.expect_err("an EHR-less row must not enter the clinical schema");
    assert!(
        error.to_string().contains("ck_version_kind"),
        "the refusal names the boundary constraint rather than failing obscurely: {error}"
    );
}

#[tokio::test]
async fn the_boundary_refuses_an_ehr_scoped_object_in_the_demographic_schema() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let (contribution_id, commit_audit_id) = change_control(&pool, "party", None).await;
    let refused = sqlx::query(
        "INSERT INTO party.version \
           (vo_id, kind, ehr_id, sys_version, trunk_version, committed_at, \
            creating_system_id, contribution_id, commit_audit_id, body) \
         VALUES ($1, 'COMPOSITION', $2, 1, 1, now(), 'test.system', \
                 $3, $4, '{}')",
    )
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .bind(contribution_id)
    .bind(commit_audit_id)
    .execute(&pool)
    .await;

    let error = refused.expect_err("a clinical object must not enter the demographic schema");
    assert!(
        error.to_string().contains("ck_version_kind"),
        "the refusal names the boundary constraint: {error}"
    );
}

// ── the runtime roles ────────────────────────────────────────────────────────

/// Each runtime role, with a short per-test login suffix and the schemas the
/// pseudonymisation boundary bars it from.
///
/// Two domains, mutually barred: the clinical roles are barred from the
/// demographic domain, and the demographic roles from the clinical one. No
/// openEHR spec governs database roles — our own design/extension.
const BARRIERS: &[(&str, &str, &[&str])] = &[
    ("ew", "ferroehr_clinical", &["party"]),
    ("er", "ferroehr_clinical_reader", &["party"]),
    ("dw", "ferroehr_party", &["clinical"]),
    ("dr", "ferroehr_party_reader", &["clinical"]),
];

/// `SQLSTATE` 42501 `insufficient_privilege` — what `PostgreSQL` reports for a
/// refused read, whether the missing grant is on the relation or on its schema
/// (`PostgreSQL` docs § Appendix A "`PostgreSQL` Error Codes", class 42).
const SQLSTATE_INSUFFICIENT_PRIVILEGE: &str = "42501";

/// A connection as a fresh non-superuser login role that is a member of
/// `domain_role`, which is how a production deployment runs (never as
/// superuser — a superuser bypasses both RLS and, being a superuser, every
/// privilege check this test is about).
///
/// Roles are cluster-global on the shared testkit server, so the login role is
/// named off the clone's database name and the testkit sweep reaps it.
async fn role_conn(db: &testkit::TestDb, suffix: &str, domain_role: &str) -> PgConnection {
    let login = format!("{}_{suffix}", db.name());
    let password = crate::fixtures::throwaway_password();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD '{password}' IN ROLE {domain_role}"
    )))
    .execute(&db.pool())
    .await
    .expect("create the login role");
    PgConnection::connect(&crate::fixtures::with_role(db.url(), &login, &password))
        .await
        .expect("connect as the runtime role")
}

/// Every table, view and sequence in `schemas`, read from `information_schema`
/// rather than listed by hand — so a relation added to either domain later is
/// covered by these tests without anybody remembering to extend a list.
///
/// Returns `(qualified name, probe statement)` pairs.
async fn readable_objects(pool: &PgPool, schemas: &[&str]) -> Vec<(String, String)> {
    let names: Vec<String> = schemas.iter().map(|s| (*s).to_owned()).collect();
    let tables: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_schema, table_name FROM information_schema.tables \
         WHERE table_schema = ANY($1) ORDER BY table_schema, table_name",
    )
    .bind(&names)
    .fetch_all(pool)
    .await
    .expect("enumerate tables and views");
    let sequences: Vec<(String, String)> = sqlx::query_as(
        "SELECT sequence_schema, sequence_name FROM information_schema.sequences \
         WHERE sequence_schema = ANY($1) ORDER BY sequence_schema, sequence_name",
    )
    .bind(&names)
    .fetch_all(pool)
    .await
    .expect("enumerate sequences");
    let mut probes: Vec<(String, String)> = tables
        .into_iter()
        .map(|(schema, name)| {
            (
                format!("{schema}.{name}"),
                format!("SELECT 1 FROM {schema}.{name} LIMIT 1"),
            )
        })
        .collect();
    probes.extend(sequences.into_iter().map(|(schema, name)| {
        (
            format!("{schema}.{name}"),
            format!("SELECT last_value FROM {schema}.{name}"),
        )
    }));
    probes
}

#[tokio::test]
async fn each_runtime_role_is_refused_every_relation_in_the_other_domain() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    for (suffix, domain_role, forbidden) in BARRIERS {
        let objects = readable_objects(&pool, forbidden).await;
        assert!(
            objects.len() > 5,
            "the enumeration must actually find the other domain's relations, \
             else this test passes vacuously: {domain_role} saw {objects:?}"
        );
        let mut conn = role_conn(&db, suffix, domain_role).await;
        for (name, probe) in &objects {
            let refused = sqlx::query(sqlx::AssertSqlSafe(probe.clone()))
                .execute(&mut conn)
                .await;
            let error =
                refused.expect_err(&format!("{domain_role} must not be able to read {name}"));
            let code = error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .map(std::borrow::Cow::into_owned);
            assert_eq!(
                code.as_deref(),
                Some(SQLSTATE_INSUFFICIENT_PRIVILEGE),
                "{domain_role} reading {name} must be refused for want of privilege, \
                 not fail some other way: {error}"
            );
        }
        drop(conn.close().await);
    }
}

#[tokio::test]
async fn a_party_committed_through_the_service_lands_only_in_the_demographic_domain() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let created = service
        .party_create(PartyKind::Person, WriteRequest::new(typed(&a_person())))
        .await
        .expect("create a person through the service seam");
    let vo_id: Uuid = created.body["uid"]["value"]
        .as_str()
        .expect("uid.value")
        .split("::")
        .next()
        .expect("the versioned-object uuid")
        .parse()
        .expect("a uuid");

    // The version, its decomposed content, its change control and the event it
    // announced are all in the demographic domain.
    for (what, sql) in [
        (
            "the version",
            "SELECT count(*) FROM party.version WHERE vo_id = $1",
        ),
        (
            "its contribution",
            "SELECT count(*) FROM party.contribution c \
             JOIN party.version v ON v.contribution_id = c.id WHERE v.vo_id = $1",
        ),
        (
            "its audit",
            "SELECT count(*) FROM party.commit_audit a \
             JOIN party.version v ON v.commit_audit_id = a.id WHERE v.vo_id = $1",
        ),
        (
            "its outbox event",
            "SELECT count(*) FROM party.event_outbox o \
             JOIN party.version v ON v.contribution_id = o.contribution_id \
             WHERE v.vo_id = $1",
        ),
    ] {
        assert_eq!(
            count(&pool, sql, vo_id).await,
            1,
            "{what} is in `demographic`"
        );
    }
    assert!(
        count(
            &pool,
            "SELECT count(*) FROM party.node WHERE vo_id = $1",
            vo_id
        )
        .await
            > 0,
        "and so are its content nodes"
    );

    // Nothing of it reached the clinical schema. `clinical.version` now refuses an
    // EHR-less row outright, so a routing miss would have failed the create —
    // this asserts the whole domain, not only the row the CHECK covers.
    for (what, sql) in [
        (
            "the version",
            "SELECT count(*) FROM clinical.version WHERE vo_id = $1",
        ),
        (
            "a node",
            "SELECT count(*) FROM clinical.node WHERE vo_id = $1",
        ),
        (
            "an archive row",
            "SELECT count(*) FROM clinical.vo_head WHERE vo_id = $1 AND archived_at IS NOT NULL",
        ),
    ] {
        assert_eq!(
            count(&pool, sql, vo_id).await,
            0,
            "the clinical schema must not hold {what} of the party"
        );
    }
    let clinical_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM clinical.event_outbox WHERE ehr_id IS NULL")
            .fetch_one(&pool)
            .await
            .expect("count the clinical outbox");
    assert_eq!(
        clinical_events, 0,
        "and the event it announced went to the demographic outbox, not the clinical one"
    );
}

/// Two domains a deployment placed on DIFFERENT DSNs that authenticate as the
/// SAME database role are refused: the separation exists in the configuration
/// and not in the database, which reads as one that holds.
///
/// The fixture is the shape an operator actually reaches this state through —
/// two DSNs that differ in text and name one credential — so the check cannot
/// pass by comparing strings.
#[tokio::test]
async fn the_boot_gate_refuses_two_domains_that_share_one_role() {
    let db = testkit::db().await.expect("testkit database");
    let settings = ferroehr::db::DbConfig::new(db.url());
    let storage = ferroehr::db::DbConfig {
        party: ferroehr::db::domain::DomainDsn {
            url: Some(ferrohealth_secret::secret_url::SecretUrl::new(format!(
                "{}?application_name=party",
                db.url()
            ))),
            ..Default::default()
        },
        ..settings.clone()
    };
    let layout = storage.layout();
    assert!(
        !layout
            .placement(ferroehr::db::domain::Domain::Clinical)
            .shares_dsn_with(layout.placement(ferroehr::db::domain::Domain::Party)),
        "the fixture must configure two DIFFERENT DSNs, else it measures nothing"
    );

    let pools = ferroehr::db::connect_domains(&storage)
        .await
        .expect("both pools connect");
    let refused = ferroehr::db::verify_domain_isolation(
        &pools,
        &layout,
        ferroehr::config::deployment::DeploymentProfile::Sandbox,
    )
    .await
    .expect_err("two domains on one role must refuse the boot");
    let rendered = refused.to_string();
    assert!(
        matches!(refused, ferroehr::db::DbError::DomainRoleShared { .. }),
        "the refusal must be the typed one: {rendered}"
    );
    assert!(
        rendered.contains("clinical") && rendered.contains("party"),
        "and it must name both domains: {rendered}"
    );

    // And the same database with both domains on ONE configured DSN is the
    // co-located posture, which boots: one DSN is one credential by
    // construction, and that is what the deployment profile reports.
    ferroehr::db::verify_domain_isolation(
        &crate::fixtures::shared_pools(&db.pool()),
        &crate::fixtures::shared_layout(),
        ferroehr::config::deployment::DeploymentProfile::Sandbox,
    )
    .await
    .expect("one DSN for every domain is the co-located posture, not a breach");
}

/// The refusal for an absent domain role names the role, the domain and the
/// remedy.
///
/// The role-presence branch itself needs a cluster with a domain role missing,
/// which no test may produce: roles are cluster-global and the harness's
/// template databases hold their grants, so dropping one would break every
/// other test in the run. What is pinned here is the message an operator acts
/// on.
#[test]
fn the_missing_role_refusal_names_the_role_and_the_remedy() {
    let refusal = ferroehr::db::DbError::DomainRoleMissing {
        role: "ferroehr_party".to_owned(),
        domain: ferroehr::db::domain::Domain::Party,
    }
    .to_string();
    assert!(refusal.contains("ferroehr_party"), "{refusal}");
    assert!(refusal.contains("party"), "{refusal}");
    assert!(refusal.contains("CREATE ROLE"), "{refusal}");
    assert!(refusal.contains("production"), "{refusal}");
}

#[tokio::test]
async fn the_boot_self_check_refuses_a_cross_domain_grant() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    ferroehr::db::verify_domain_isolation(
        &crate::fixtures::shared_pools(&pool),
        &crate::fixtures::shared_layout(),
        ferroehr::config::deployment::DeploymentProfile::Sandbox,
    )
    .await
    .expect("a correctly migrated database passes the boot gate");

    // One object of each kind the gate claims to cover, granted and revoked in
    // turn: the gate must fail while the grant stands and pass once it is gone,
    // so neither verdict can be the one it always returns.
    let sequence: String =
        sqlx::query_scalar("SELECT pg_get_serial_sequence('party.event_outbox', 'seq')")
            .fetch_one(&pool)
            .await
            .expect("the demographic outbox identity sequence");
    for (role, object, kind, grant, revoke) in [
        (
            "ferroehr_clinical",
            "party.version",
            "table",
            "GRANT SELECT ON",
            "REVOKE SELECT ON",
        ),
        (
            "ferroehr_clinical",
            sequence.as_str(),
            "sequence",
            "GRANT SELECT ON SEQUENCE",
            "REVOKE SELECT ON SEQUENCE",
        ),
        // The party role reaching the clinical domain: the other direction.
        (
            "ferroehr_party",
            "clinical.version",
            "table",
            "GRANT SELECT ON",
            "REVOKE SELECT ON",
        ),
        // A function is the fourth breach shape: a trigger function runs as its
        // owner, so a role holding EXECUTE across the boundary reaches the
        // other domain without a single grant on its tables.
        (
            "ferroehr_party",
            "clinical.subject_pseudonym_guard()",
            "function",
            "GRANT EXECUTE ON FUNCTION",
            "REVOKE EXECUTE ON FUNCTION",
        ),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!("{grant} {object} TO {role}")))
            .execute(&pool)
            .await
            .expect("grant across the boundary");

        let refused = ferroehr::db::verify_domain_isolation(
            &crate::fixtures::shared_pools(&pool),
            &crate::fixtures::shared_layout(),
            ferroehr::config::deployment::DeploymentProfile::Sandbox,
        )
        .await;
        let error = refused.expect_err("a role reaching another domain must refuse the boot");
        let text = error.to_string();
        // The object is named without its argument list, which only the GRANT
        // statement needs. Kind and object are matched as one phrase: the
        // message's remedy mentions functions whatever the kind, so the word
        // alone would prove nothing.
        let named = object.split('(').next().unwrap_or(object);
        assert!(
            text.contains(role) && text.contains(&format!("the {kind} `{named}`")),
            "the refusal names the role, the kind and the object it can reach: {text}"
        );

        sqlx::query(sqlx::AssertSqlSafe(format!(
            "{revoke} {object} FROM {role}"
        )))
        .execute(&pool)
        .await
        .expect("revoke across the boundary");
        ferroehr::db::verify_domain_isolation(
            &crate::fixtures::shared_pools(&pool),
            &crate::fixtures::shared_layout(),
            ferroehr::config::deployment::DeploymentProfile::Sandbox,
        )
        .await
        .unwrap_or_else(|e| panic!("the gate passes again once {object} is revoked: {e}"));
    }
}

/// A minimal valid PERSON body, authored as canonical JSON exactly as a client
/// would post it (`.claude/rules/testing.md` §Test-fixture construction,
/// class 2).
pub(crate) fn a_person() -> serde_json::Value {
    serde_json::json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0"
        },
        "name": { "_type": "DV_TEXT", "value": "Ada Lovelace" },
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
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": "Lovelace" }
                }]
            }
        }]
    })
}

/// The split clinical role writes and reads the audit trail (#3267).
///
/// The store writes through the clinical pool. Before the grant migration the
/// Audit Record Repository was held by the single-domain pair only, so a
/// deployment whose clinical credential is a member of `ferroehr_clinical` alone
/// wrote no access record at all, and every auditable operation was refused.
/// Runs as a fresh login role IN ROLE `ferroehr_clinical`, never as the superuser
/// the testkit pool is.
#[tokio::test]
async fn the_split_clinical_role_writes_and_reads_the_audit_trail() {
    use ferroehr::db::DbConfig;
    use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
    use ferroehr::system_log::store::AuditStore;

    let db = testkit::db().await.expect("testkit database");
    let clinical = ferroehr::db::connect_domain(
        &DbConfig {
            url: ferrohealth_secret::secret_url::SecretUrl::new(
                crate::fixtures::dsn_as(&db, "audclin", "ferroehr_clinical").await,
            ),
            ..DbConfig::default()
        },
        ferroehr::db::domain::Domain::Clinical,
    )
    .await
    .expect("the clinical pool connects on its own credential");

    let store = AuditStore::new(clinical.clone());
    // A party read is kept as an `audit_event` record, a composition read as
    // its access-log entry with its search index row: the role writes both.
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Demographic,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut event.user_id);
    let id = store
        .insert(&event, None)
        .await
        .expect("the clinical role inserts an access record");
    let mut access = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut access.user_id);
    store
        .insert(&access, None)
        .await
        .expect("the clinical role inserts an access-log entry");
    let seq: i64 = sqlx::query_scalar("SELECT seq FROM audit.audit_event WHERE id = $1")
        .bind(id)
        .fetch_one(&clinical)
        .await
        .expect("the record's position");
    store
        .mark_delivered(
            ferroehr::system_log::store::delivery::Sink::FhirFeed,
            ferroehr::system_log::store::delivery::Due {
                chain: ferroehr::system_log::store::chains::AuditChain::System,
                id: seq,
                retry: false,
            },
        )
        .await
        .expect("a delivery moves the cursor and changes no record");
    crate::fixtures::verify_chain(&clinical)
        .await
        .expect("the clinical role verifies the chain");

    let reader = ferroehr::db::connect_domain(
        &DbConfig {
            url: ferrohealth_secret::secret_url::SecretUrl::new(
                crate::fixtures::dsn_as(&db, "audread", "ferroehr_clinical_reader").await,
            ),
            ..DbConfig::default()
        },
        ferroehr::db::domain::Domain::Clinical,
    )
    .await
    .expect("the clinical reader connects");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&reader)
        .await
        .expect("the clinical reader reads the trail");
    assert_eq!(count, 1);
    let entries: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_record")
        .fetch_one(&reader)
        .await
        .expect("the clinical reader reads every record");
    assert_eq!(entries, 2);
    let rewrite = sqlx::query("UPDATE audit.audit_event SET principal = 'bob' WHERE id = $1")
        .bind(id)
        .execute(&clinical)
        .await;
    assert!(
        rewrite.is_err(),
        "the clinical role holds no grant to rewrite a record: {rewrite:?}"
    );
}

/// The one door the clinical roles may pass into the party domain (#6320):
/// the clinical role may run `party_door.party_revisions()` and reads no
/// party table, and the boot gate refuses anything else in the door's schema,
/// and the function itself once it stops running as its owner with a fixed
/// `search_path`.
#[tokio::test]
async fn the_party_door_holds_its_one_function_and_nothing_else() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let gate = || async {
        ferroehr::db::verify_domain_isolation(
            &crate::fixtures::shared_pools(&pool),
            &crate::fixtures::shared_layout(),
            ferroehr::config::deployment::DeploymentProfile::Sandbox,
        )
        .await
    };
    gate()
        .await
        .expect("a correctly migrated database passes the boot gate");
    let (door, tables): (bool, bool) = sqlx::query_as(
        "SELECT has_function_privilege('ferroehr_clinical', 'party_door.party_revisions()', \
         'EXECUTE'), has_table_privilege('ferroehr_clinical', 'party.binding_grant_revision', \
         'SELECT')",
    )
    .fetch_one(&pool)
    .await
    .expect("the clinical role's privileges");
    assert!(door, "the clinical role may run the door");
    assert!(!tables, "and reads no table behind it");

    for (breach, mend, problem) in [
        (
            "CREATE TABLE party_door.smuggled (id int)",
            "DROP TABLE party_door.smuggled",
            "is a relation",
        ),
        (
            "CREATE TYPE party_door.smuggled AS ENUM ('kept')",
            "DROP TYPE party_door.smuggled",
            "is a type",
        ),
        (
            "CREATE FUNCTION party_door.other() RETURNS int LANGUAGE sql AS 'SELECT 1'",
            "DROP FUNCTION party_door.other()",
            "is not the door's function",
        ),
        (
            "ALTER FUNCTION party_door.party_revisions() SECURITY INVOKER",
            "ALTER FUNCTION party_door.party_revisions() SECURITY DEFINER",
            "does not run as its owner",
        ),
        (
            "ALTER FUNCTION party_door.party_revisions() RESET search_path",
            "ALTER FUNCTION party_door.party_revisions() SET search_path = party, pg_catalog",
            "runs on the caller's search_path",
        ),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(breach))
            .execute(&pool)
            .await
            .expect("open the door wider");
        let error = gate().await.expect_err("a wider door must refuse the boot");
        assert!(
            matches!(
                &error,
                ferroehr::db::DbError::PartyDoorBreached { problem: found, .. } if *found == problem
            ),
            "{breach}: {error}"
        );
        sqlx::query(sqlx::AssertSqlSafe(mend))
            .execute(&pool)
            .await
            .expect("close it again");
        gate().await.expect("the mended door passes the boot gate");
    }
}
