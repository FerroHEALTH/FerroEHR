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

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              fixture helpers; a failing fixture must panic at the fixture (the \
              Rust Book ch11)"
)]

use sqlx::{Connection, PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::typed_body::typed;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::error::ServiceError;
use ferroehr::service::refusal::{Clash, Invalid, Refusal};

/// Write the audit and contribution rows every version row needs, leaving the
/// boundary constraints exactly as the migrated clone carries them.
///
/// Returns `(contribution_id, commit_audit_id)`.
async fn change_control(pool: &PgPool, schema: &str, ehr_id: Option<Uuid>) -> (Uuid, Uuid) {
    let (commit_audit_id, contribution_id) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {schema}.commit_audit (id, system_id, change_type, committer) \
         VALUES ($1, 'test.system', '249', \
                 '{{\"_type\":\"PARTY_IDENTIFIED\",\"name\":\"tester\"}}'::jsonb)"
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
        .party_create(PartyKind::Person, typed(&a_person()), None)
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
    let storage = ferroehr::db::domain::StorageConfig {
        party: ferroehr::db::domain::DomainDsn {
            url: Some(ferrohealth_secret::secret_url::SecretUrl::new(format!(
                "{}?application_name=party",
                db.url()
            ))),
            url_file: None,
        },
        ..ferroehr::db::domain::StorageConfig::default()
    };
    let layout = storage.layout(&settings);
    assert!(
        !layout
            .placement(ferroehr::db::domain::Domain::Clinical)
            .shares_dsn_with(layout.placement(ferroehr::db::domain::Domain::Party)),
        "the fixture must configure two DIFFERENT DSNs, else it measures nothing"
    );

    let pools = ferroehr::db::connect_domains(&settings, &storage)
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

// ── protected national identifiers (#3155) ───────────────────────────────────

/// A synthetic BSN: constructed by running the elfproef forward, issued to
/// nobody.
const SYNTHETIC_BSN: &str = "111222333"; // privacy-allow: synthetic

/// A test root key. Sixty-four hex characters, and a literal here is not a
/// credential: it protects one ephemeral clone for the length of one test.
const TEST_ROOT_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn test_keys() -> ferroehr::service::demographic::identifier::crypto::DomainKeys {
    use ferroehr::service::demographic::identifier::crypto::{DomainKeys, KeyDomain, RootKey};
    let root = RootKey::from_hex(&secrecy::SecretString::from(TEST_ROOT_KEY.to_owned()))
        .expect("a 32-byte root key");
    DomainKeys::derive(&root, KeyDomain::Demographic)
}

/// The party whose live row stores the lookup digest of `value` in `scheme`.
///
/// Reads `party.national_identifier` directly: what these tests check is the
/// stored digest, computed here under the test key exactly as a seal stores it.
async fn live_holder(party_pool: &PgPool, scheme: &str, value: &str) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT party_id FROM party.national_identifier \
         WHERE scheme = $1 AND lookup_digest = $2 AND retired_at IS NULL",
    )
    .bind(scheme)
    .bind(test_keys().lookup_digest(scheme, value))
    .fetch_optional(party_pool)
    .await
    .expect("read the live identifier row")
}

/// A sealed identifier round-trips, and its stored digest finds its party
/// without decrypting anything.
///
/// The two halves are the point of the design: the ciphertext answers "what is
/// this party's identifier" only to a holder of the key, and the keyed digest
/// answers "which party holds this identifier" to a caller that already knows
/// the value. Neither answers the other's question.
#[tokio::test]
async fn a_sealed_identifier_round_trips_and_resolves_to_its_party() {
    use ferroehr::service::demographic::identifier::store::IdentifierStore;

    let db = testkit::db().await.expect("testkit database");
    let party_pool =
        ferroehr::db::domain_pool_from(&db.pool(), ferroehr::db::domain::Domain::Party);
    let store = IdentifierStore::new(party_pool.clone());
    let keys = test_keys();
    let party = Uuid::now_v7();

    let mut conn = party_pool
        .acquire()
        .await
        .expect("a party-domain connection");
    let row = IdentifierStore::seal(&mut conn, &keys, party, "nl-bsn", SYNTHETIC_BSN)
        .await
        .expect("seal the identifier");
    drop(conn);

    assert_eq!(
        store.open(&keys, row).await.expect("open"),
        Some(SYNTHETIC_BSN.to_owned()),
        "the key holder reads the value back"
    );
    assert_eq!(
        live_holder(&party_pool, "nl-bsn", SYNTHETIC_BSN).await,
        Some(party),
        "the digest resolves to the party without decryption"
    );
    assert_eq!(
        live_holder(&party_pool, "nl-bsn", "987654321").await,
        None,
        "an identifier nobody holds resolves to nothing, not to an arbitrary party"
    );

    // The stored bytes are not the value, in either column.
    let stored: (Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT ciphertext, lookup_digest FROM party.national_identifier WHERE id = $1",
    )
    .bind(row)
    .fetch_one(&db.pool())
    .await
    .expect("the stored row");
    for column in [stored.0, stored.1] {
        assert!(
            !column
                .windows(SYNTHETIC_BSN.len())
                .any(|w| w == SYNTHETIC_BSN.as_bytes()),
            "no stored column may carry the value"
        );
    }
}

/// An unregistered scheme is refused rather than stored unprotected.
#[tokio::test]
async fn an_unregistered_scheme_is_refused() {
    use ferroehr::service::demographic::identifier::store::{IdentifierStore, StoreError};

    let db = testkit::db().await.expect("testkit database");
    let mut conn = ferroehr::db::domain_pool_from(&db.pool(), ferroehr::db::domain::Domain::Party)
        .acquire()
        .await
        .expect("a party-domain connection");
    let refused = IdentifierStore::seal(
        &mut conn,
        &test_keys(),
        Uuid::now_v7(),
        "zz-invented",
        SYNTHETIC_BSN,
    )
    .await;
    assert!(
        matches!(refused, Err(StoreError::UnknownScheme { ref scheme }) if scheme == "zz-invented"),
        "an identifier kind nobody registered is refused, naming the scheme: {refused:?}"
    );
}

/// The clinical roles cannot read the protected identifiers at all, and the
/// demographic READER cannot read the two sensitive columns.
///
/// The column-level grant is the part a relation-level test would miss: the
/// reporting role legitimately sees that a party holds a protected identifier,
/// and must never see the sealed value or the digest that matches it.
#[tokio::test]
async fn only_the_demographic_writer_reaches_the_sealed_value() {
    let db = testkit::db().await.expect("testkit database");

    for (suffix, role) in [
        ("nie", "ferroehr_clinical"),
        ("nir", "ferroehr_clinical_reader"),
    ] {
        let mut conn = role_conn(&db, suffix, role).await;
        let refused = sqlx::query("SELECT ciphertext FROM party.national_identifier")
            .fetch_all(&mut conn)
            .await;
        let code = refused
            .err()
            .and_then(|e| {
                e.as_database_error()
                    .and_then(sqlx::error::DatabaseError::code)
                    .map(|c| c.to_string())
            })
            .unwrap_or_default();
        assert_eq!(
            code, SQLSTATE_INSUFFICIENT_PRIVILEGE,
            "{role} must not reach the protected identifiers at all"
        );
    }

    let mut reader = role_conn(&db, "nidr", "ferroehr_party_reader").await;
    for column in ["ciphertext", "lookup_digest"] {
        let refused = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {column} FROM party.national_identifier"
        )))
        .fetch_all(&mut reader)
        .await;
        let code = refused
            .err()
            .and_then(|e| {
                e.as_database_error()
                    .and_then(sqlx::error::DatabaseError::code)
                    .map(|c| c.to_string())
            })
            .unwrap_or_default();
        assert_eq!(
            code, SQLSTATE_INSUFFICIENT_PRIVILEGE,
            "the demographic reader must not read {column}"
        );
    }
    // …but it does see that the identifier exists and whose it is, which its
    // reporting role needs.
    sqlx::query("SELECT id, party_id, scheme FROM party.national_identifier")
        .fetch_all(&mut reader)
        .await
        .expect("the reader sees the non-sensitive columns");
}

/// A party committed with protection ON stores a reference, never the value —
/// and the version's own body, its decomposed nodes and the served read all
/// carry the same form.
///
/// The end-to-end property #3155 exists for. The sealing runs before the body
/// is decomposed and signed, so stored, signed and served are one form; a test
/// that only checked `version.body` would miss the node rows, which are a
/// second copy of the same content.
#[tokio::test]
async fn a_protected_identifier_never_reaches_the_versioned_body() {
    use ferroehr::service::demographic::identifier::engine::IdentifierProtection;

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let engine = IdentifierProtection::from_config(
        &ferroehr::service::demographic::identifier::config::IdentifierProtectionConfig {
            enabled: true,
            schemes: vec!["nl-bsn".to_owned()],
            key: Some(ferrohealth_secret::secret::Secret::new(TEST_ROOT_KEY)),
            key_file: None,
        },
        Some(&ferrohealth_secret::secret::Secret::new(TEST_ROOT_KEY)),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    )
    .expect("the engine builds")
    .expect("protection is enabled");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_identifier_protection(std::sync::Arc::new(engine));

    let mut person = a_person();
    person["identities"][0]["details"]["items"]
        .as_array_mut()
        .expect("the identity items")
        .push(serde_json::json!({
            "_type": "ELEMENT",
            "archetype_node_id": "at0004",
            "name": { "_type": "DV_TEXT", "value": "bsn" },
            "value": { "_type": "DV_IDENTIFIER", "type": "nl-bsn",
                       "id": SYNTHETIC_BSN, "issuer": "RvIG", "assigner": "RvIG" }
        }));

    let created = service
        .party_create(PartyKind::Person, typed(&person), None)
        .await
        .expect("commit a person carrying a protected identifier");
    let vo_id: Uuid = created.body["uid"]["value"]
        .as_str()
        .expect("uid.value")
        .split("::")
        .next()
        .expect("the versioned-object uuid")
        .parse()
        .expect("a uuid");

    // Neither copy of the content carries the value: the version body…
    let body: String = sqlx::query_scalar(
        "SELECT v.body::text FROM party.version v JOIN party.vo_head h ON h.vo_id = v.vo_id \
         AND h.trunk_head_sys_version = v.sys_version WHERE v.vo_id = $1",
    )
    .bind(vo_id)
    .fetch_one(&pool)
    .await
    .expect("the stored body");
    assert!(
        !body.contains(SYNTHETIC_BSN),
        "the versioned body must carry a reference, not the identifier"
    );
    assert!(
        body.contains("urn:ferroehr:protected-identifier:"),
        "…and the reference must be there in its place: {body}"
    );

    // …nor the decomposed node rows, which are the same content a second time.
    let nodes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM party.node WHERE vo_id = $1 \
         AND ext.node_fragment(data, rm_type, archetype, name, name_code, name_terminology, \
             value_type, value_num, value_text, value_code, value_terminology, rm_version, \
             language, encoding)::text LIKE $2",
    )
    .bind(vo_id)
    .bind(format!("%{SYNTHETIC_BSN}%"))
    .fetch_one(&pool)
    .await
    .expect("scan the node rows");
    assert_eq!(nodes, 0, "no decomposed node may carry the identifier");

    // The sealed row exists, and its digest names this party.
    let party_pool = ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party);
    assert_eq!(
        live_holder(&party_pool, "nl-bsn", SYNTHETIC_BSN).await,
        Some(vo_id),
        "the identifier resolves to the party that holds it"
    );
}

/// The protection engine over `party_pool`, sealing `nl-bsn` under the test key.
fn protection_engine(
    party_pool: PgPool,
) -> ferroehr::service::demographic::identifier::engine::IdentifierProtection {
    use ferroehr::service::demographic::identifier::config::IdentifierProtectionConfig;
    use ferroehr::service::demographic::identifier::engine::IdentifierProtection;

    IdentifierProtection::from_config(
        &IdentifierProtectionConfig {
            enabled: true,
            schemes: vec!["nl-bsn".to_owned()],
            key: Some(ferrohealth_secret::secret::Secret::new(TEST_ROOT_KEY)),
            key_file: None,
        },
        Some(&ferrohealth_secret::secret::Secret::new(TEST_ROOT_KEY)),
        party_pool,
    )
    .expect("the engine builds")
    .expect("protection is enabled")
}

/// A PERSON with `family` as its family name, carrying one `DV_IDENTIFIER` per
/// `(type, id)` pair.
fn person_carrying(family: &str, identifiers: &[(&str, &str)]) -> serde_json::Value {
    let mut person = a_person();
    *person
        .pointer_mut("/identities/0/details/items/0/value/value")
        .expect("the family name") = serde_json::Value::from(family);
    let items = person
        .pointer_mut("/identities/0/details/items")
        .and_then(serde_json::Value::as_array_mut)
        .expect("the identity items");
    for (scheme, id) in identifiers {
        items.push(serde_json::json!({
            "_type": "ELEMENT",
            "archetype_node_id": "at0004",
            "name": { "_type": "DV_TEXT", "value": "identifier" },
            "value": { "_type": "DV_IDENTIFIER", "type": scheme,
                       "id": id, "issuer": "RvIG", "assigner": "RvIG" }
        }));
    }
    person
}

/// A demographic CONTRIBUTION creating one PERSON per family name, each
/// carrying `bsn` as a protected `nl-bsn` identifier.
fn persons_with_bsn(families: &[&str], bsn: &str) -> serde_json::Value {
    let coded = |code: &str, value: &str| {
        serde_json::json!({
            "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": code
            }
        })
    };
    let committer = serde_json::json!({ "_type": "PARTY_IDENTIFIED", "name": "tester" });
    let versions: Vec<serde_json::Value> = families
        .iter()
        .map(|family| {
            let person = person_carrying(family, &[("nl-bsn", bsn)]);
            serde_json::json!({
                "_type": "ORIGINAL_VERSION",
                "lifecycle_state": crate::fixtures::lifecycle("532"),
                "commit_audit": {
                    "change_type": coded("249", "creation"),
                    "committer": committer
                },
                "data": person
            })
        })
        .collect();
    serde_json::json!({
        "_type": "CONTRIBUTION",
        "versions": versions,
        "audit": { "change_type": coded("249", "creation"), "committer": committer }
    })
}

/// The sealed rows and the party versions in the clone, read on the harness
/// credential.
async fn sealed_and_versions(pool: &PgPool) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM party.national_identifier), \
                (SELECT count(*) FROM party.version)",
    )
    .fetch_one(pool)
    .await
    .expect("count the sealed rows and the party versions")
}

/// Drives one layout: a commit that fails after its first member was sealed
/// leaves no sealed row, and a commit that succeeds keeps its row.
///
/// The failure is a second PERSON carrying the first one's identifier: its seal
/// hits the one-party-per-value index after the first member's seal already
/// ran, so the commit fails with a sealed row in hand.
async fn a_failed_commit_leaves_no_sealed_identifier(service: &FerroEhrService, harness: &PgPool) {
    let refused = service
        .demographic_contribution_create(persons_with_bsn(&["Lovelace", "Byron"], SYNTHETIC_BSN))
        .await
        .expect_err("two parties cannot hold one national identifier");
    assert!(
        matches!(
            &refused,
            ServiceError::Conflict(Refusal {
                kind: Clash::State,
                ..
            })
        ),
        "a value another party holds is the 409 conflict, never a 500: {refused:?}"
    );
    assert_eq!(
        sealed_and_versions(harness).await,
        (0, 0),
        "a commit that failed after sealing leaves neither a sealed row nor a version"
    );

    service
        .demographic_contribution_create(persons_with_bsn(&["Lovelace"], SYNTHETIC_BSN))
        .await
        .expect("a party carrying a protected identifier commits");
    assert_eq!(
        sealed_and_versions(harness).await,
        (1, 1),
        "a committed party keeps its sealed row"
    );
    assert_eq!(
        live_holder(harness, "nl-bsn", SYNTHETIC_BSN).await,
        sqlx::query_scalar::<_, Uuid>("SELECT vo_id FROM party.version")
            .fetch_optional(harness)
            .await
            .expect("the committed party"),
        "the kept row belongs to the party that committed"
    );
}

/// Sealing and the commit succeed or fail together on the shared credential:
/// the seal runs on the party commit's own transaction.
#[tokio::test]
async fn a_failed_party_commit_leaves_no_sealed_identifier() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let party_pool = ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party);
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_identifier_protection(std::sync::Arc::new(protection_engine(party_pool.clone())));

    a_failed_commit_leaves_no_sealed_identifier(&service, &party_pool).await;
}

/// The same with each domain on its own runtime credential: the party
/// commit's transaction runs as `ferroehr_party`, which writes the sealed rows,
/// so separated credentials need no second connection and no cleanup.
#[tokio::test]
async fn a_failed_party_commit_leaves_no_sealed_identifier_on_separated_credentials() {
    use ferroehr::db::DbConfig;
    use ferrohealth_secret::secret_url::SecretUrl;

    let db = testkit::db().await.expect("testkit database");
    let settings = DbConfig {
        url: SecretUrl::new(crate::fixtures::dsn_as(&db, "rbclin", "ferroehr_clinical").await),
        ..DbConfig::default()
    };
    let storage = ferroehr::db::domain::StorageConfig {
        party: ferroehr::db::domain::DomainDsn {
            url: Some(SecretUrl::new(
                crate::fixtures::dsn_as(&db, "rbdemo", "ferroehr_party").await,
            )),
            url_file: None,
        },
        ..ferroehr::db::domain::StorageConfig::default()
    };
    assert!(
        storage.is_separated(ferroehr::db::domain::Domain::Party),
        "the fixture must actually separate the credentials"
    );
    let pools = ferroehr::db::connect_domains(&settings, &storage)
        .await
        .expect("each pool connects on its own credential");
    let service = FerroEhrService::new(&pools)
        .with_identifier_protection(std::sync::Arc::new(protection_engine(pools.party.clone())));

    let harness = ferroehr::db::domain_pool_from(&db.pool(), ferroehr::db::domain::Domain::Party);
    a_failed_commit_leaves_no_sealed_identifier(&service, &harness).await;
    pools.close().await;
}

/// A second synthetic BSN, for a party whose identifier changes: constructed by
/// running the elfproef forward, issued to nobody.
const SECOND_SYNTHETIC_BSN: &str = "123456782"; // privacy-allow: synthetic

/// A service sealing `nl-bsn` on the shared credential, its engine, and the
/// party-domain pool the assertions read through.
fn protected_service(
    db: &testkit::TestDb,
) -> (
    FerroEhrService,
    std::sync::Arc<ferroehr::service::demographic::identifier::engine::IdentifierProtection>,
    PgPool,
) {
    let pool = db.pool();
    let party_pool = ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party);
    let engine = std::sync::Arc::new(protection_engine(party_pool.clone()));
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_identifier_protection(std::sync::Arc::clone(&engine));
    (service, engine, party_pool)
}

/// The versioned-object uuid and the version uid of a committed party.
fn committed_ids(body: &serde_json::Value) -> (String, String) {
    let version = body
        .pointer("/uid/value")
        .and_then(serde_json::Value::as_str)
        .expect("uid.value")
        .to_owned();
    let vo = version
        .split("::")
        .next()
        .expect("the versioned-object uuid")
        .to_owned();
    (vo, version)
}

/// Every stored version body of `vo`, oldest first, each expanded back to the
/// values its own identifier rows hold.
async fn expanded_versions(
    engine: &ferroehr::service::demographic::identifier::engine::IdentifierProtection,
    pool: &PgPool,
    vo: &str,
) -> Vec<serde_json::Value> {
    let texts: Vec<String> = sqlx::query_scalar(
        "SELECT body::text FROM party.version WHERE vo_id = $1::uuid ORDER BY sys_version",
    )
    .bind(vo)
    .fetch_all(pool)
    .await
    .expect("the stored version bodies");
    let mut bodies = Vec::with_capacity(texts.len());
    for text in texts {
        let mut body: serde_json::Value = serde_json::from_str(&text).expect("a stored body");
        engine.expand(&mut body).await.expect("expand the body");
        bodies.push(body);
    }
    bodies
}

/// The `DV_IDENTIFIER.id` the identity item of [`person_carrying`] holds.
fn identifier_id(body: &serde_json::Value) -> &str {
    body.pointer("/identities/0/details/items/1/value/id")
        .and_then(serde_json::Value::as_str)
        .expect("the identifier item")
}

/// Each party version keeps the identifier it committed: a changed BSN gives
/// version 2 a row of its own and leaves version 1's row untouched, while
/// resolution answers the latest version only.
///
/// RM common `master06-change_control_package.adoc` §Contributions: the
/// repository "is by definition indelible", and a modification is "achieved
/// by physically committing new Versions".
#[tokio::test]
async fn each_party_version_keeps_the_identifier_it_committed() {
    let db = testkit::db().await.expect("testkit database");
    let (service, engine, party_pool) = protected_service(&db);

    let created = service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("create the person");
    let (vo, v1) = committed_ids(&created.body);
    service
        .party_update(
            PartyKind::Person,
            vo.clone(),
            v1,
            typed(&person_carrying(
                "Lovelace",
                &[("nl-bsn", SECOND_SYNTHETIC_BSN)],
            )),
            None,
        )
        .await
        .expect("change the person's identifier");

    let versions = expanded_versions(&engine, &party_pool, &vo).await;
    assert_eq!(versions.len(), 2, "two versions committed");
    assert_eq!(
        identifier_id(&versions[0]),
        SYNTHETIC_BSN,
        "version 1 expands to the value it committed"
    );
    assert_eq!(
        identifier_id(&versions[1]),
        SECOND_SYNTHETIC_BSN,
        "version 2 expands to its own value"
    );

    let party: Uuid = vo.parse().expect("a uuid");
    assert_eq!(
        live_holder(&party_pool, "nl-bsn", SECOND_SYNTHETIC_BSN).await,
        Some(party),
        "resolution answers the latest version's value"
    );
    assert_eq!(
        live_holder(&party_pool, "nl-bsn", SYNTHETIC_BSN).await,
        None,
        "a value only an earlier version held resolves to nobody"
    );

    let refused = service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying(
                "Byron",
                &[("nl-bsn", SECOND_SYNTHETIC_BSN)],
            )),
            None,
        )
        .await
        .expect_err("another party may not take a live value");
    assert!(
        matches!(
            &refused,
            ServiceError::Conflict(Refusal {
                kind: Clash::State,
                ..
            })
        ),
        "{refused:?}"
    );
    service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Byron", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("a value no latest version holds is free for another party");
}

/// A version that sends the served reference back unchanged gets a row of its
/// own holding the same value, so the identifier keeps resolving and neither
/// version shares a row with the other.
#[tokio::test]
async fn a_carried_reference_is_sealed_again_for_the_new_version() {
    let db = testkit::db().await.expect("testkit database");
    let (service, engine, party_pool) = protected_service(&db);

    let created = service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("create the person");
    let (vo, v1) = committed_ids(&created.body);
    let served = service
        .party_get(PartyKind::Person, vo.clone(), None)
        .await
        .expect("read the person back");
    let carried = identifier_id(&served.body).to_owned();
    assert!(
        carried.starts_with("urn:ferroehr:protected-identifier:"),
        "the served form carries the reference: {carried}"
    );
    let mut resent = served.body.clone();
    resent
        .as_object_mut()
        .expect("a PERSON object")
        .remove("uid");
    service
        .party_update(PartyKind::Person, vo.clone(), v1, typed(&resent), None)
        .await
        .expect("commit the served body unchanged");

    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT body::text FROM party.version WHERE vo_id = $1::uuid ORDER BY sys_version",
    )
    .bind(&vo)
    .fetch_all(&party_pool)
    .await
    .expect("the stored bodies");
    let references: Vec<String> = stored
        .iter()
        .map(|text| {
            let body: serde_json::Value = serde_json::from_str(text).expect("a stored body");
            identifier_id(&body).to_owned()
        })
        .collect();
    assert_eq!(references.len(), 2, "two versions committed");
    assert_ne!(
        references[0], references[1],
        "each version references a row of its own"
    );
    for body in expanded_versions(&engine, &party_pool, &vo).await {
        assert_eq!(identifier_id(&body), SYNTHETIC_BSN);
    }
    assert_eq!(
        live_holder(&party_pool, "nl-bsn", SYNTHETIC_BSN).await,
        Some(vo.parse().expect("a uuid")),
        "the identifier still resolves to the party"
    );
}

/// A body referencing another party's sealed row is refused as `422`, so no
/// party can expand to a value it does not hold.
#[tokio::test]
async fn a_reference_to_another_partys_identifier_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let (service, _engine, party_pool) = protected_service(&db);

    let created = service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("create the person");
    let (vo, _) = committed_ids(&created.body);
    let stored = service
        .party_get(PartyKind::Person, vo, None)
        .await
        .expect("read the person back");
    let foreign = identifier_id(&stored.body).to_owned();
    assert!(
        foreign.starts_with("urn:ferroehr:protected-identifier:"),
        "the stored form carries the reference: {foreign}"
    );

    let refused = service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Byron", &[("nl-bsn", &foreign)])),
            None,
        )
        .await
        .expect_err("another party's row cannot be carried");
    assert!(
        matches!(
            &refused,
            ServiceError::Unprocessable {
                kind: Invalid::Content,
                ..
            }
        ),
        "{refused:?}"
    );
    assert_eq!(
        sealed_and_versions(&party_pool).await,
        (1, 1),
        "the refusal wrote nothing"
    );
}

/// Two different values in one scheme are refused as `422`: a party holds one
/// value per scheme.
#[tokio::test]
async fn two_values_in_one_scheme_are_refused() {
    let db = testkit::db().await.expect("testkit database");
    let (service, _engine, party_pool) = protected_service(&db);

    let refused = service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying(
                "Lovelace",
                &[("nl-bsn", SYNTHETIC_BSN), ("nl-bsn", SECOND_SYNTHETIC_BSN)],
            )),
            None,
        )
        .await
        .expect_err("one party, two values in one scheme");
    assert!(
        matches!(
            &refused,
            ServiceError::Unprocessable {
                kind: Invalid::Content,
                ..
            }
        ),
        "{refused:?}"
    );
    assert_eq!(sealed_and_versions(&party_pool).await, (0, 0));
}

/// A configured scheme the registry does not hold is a configuration fault:
/// the boot check names it, and a commit reaching it answers `500` and writes
/// nothing, rather than storing the identifier unprotected.
#[tokio::test]
async fn an_unregistered_configured_scheme_is_a_configuration_fault() {
    use ferroehr::service::demographic::identifier::config::IdentifierProtectionConfig;
    use ferroehr::service::demographic::identifier::engine::IdentifierProtection;
    use ferroehr::service::demographic::identifier::store::StoreError;

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let party_pool = ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party);
    protection_engine(party_pool.clone())
        .verify_registered()
        .await
        .expect("the shipped registry holds nl-bsn");

    let engine = IdentifierProtection::from_config(
        &IdentifierProtectionConfig {
            enabled: true,
            schemes: vec!["zz-invented".to_owned()],
            key: Some(ferrohealth_secret::secret::Secret::new(TEST_ROOT_KEY)),
            key_file: None,
        },
        Some(&ferrohealth_secret::secret::Secret::new(TEST_ROOT_KEY)),
        party_pool.clone(),
    )
    .expect("the engine builds")
    .expect("protection is enabled");
    let checked = engine.verify_registered().await;
    assert!(
        matches!(checked, Err(StoreError::UnknownScheme { ref scheme }) if scheme == "zz-invented"),
        "the boot check names the unregistered scheme: {checked:?}"
    );

    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_identifier_protection(std::sync::Arc::new(engine));
    let refused = service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("zz-invented", "AB-12")])),
            None,
        )
        .await
        .expect_err("an unregistered scheme cannot be sealed");
    assert!(
        matches!(&refused, ServiceError::Internal(_)),
        "the client cannot fix the deployment's configuration: {refused:?}"
    );
    assert_eq!(
        sealed_and_versions(&party_pool).await,
        (0, 0),
        "nothing is stored, least of all the identifier unprotected"
    );
}

/// A write response is the stored form: the create and update bodies carry the
/// reference, never the submitted value, and equal what a later read serves.
#[tokio::test]
async fn a_party_write_response_serves_the_stored_form() {
    let db = testkit::db().await.expect("testkit database");
    let (service, _engine, _party_pool) = protected_service(&db);

    let created = service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("create the person");
    let (vo, v1) = committed_ids(&created.body);
    assert!(
        !created.body.to_string().contains(SYNTHETIC_BSN),
        "the create response must not carry the protected value"
    );
    assert!(
        identifier_id(&created.body).starts_with("urn:ferroehr:protected-identifier:"),
        "the create response carries the reference the store holds"
    );
    let read = service
        .party_get(PartyKind::Person, vo.clone(), None)
        .await
        .expect("read the person back");
    assert_eq!(
        created.body, read.body,
        "the create response is the stored form a later read serves"
    );

    let updated = service
        .party_update(
            PartyKind::Person,
            vo.clone(),
            v1,
            typed(&person_carrying(
                "Lovelace",
                &[("nl-bsn", SECOND_SYNTHETIC_BSN)],
            )),
            None,
        )
        .await
        .expect("update the person");
    assert!(
        !updated.body.to_string().contains(SECOND_SYNTHETIC_BSN),
        "the update response must not carry the protected value"
    );
    let reread = service
        .party_get(PartyKind::Person, vo, None)
        .await
        .expect("read the update back");
    assert_eq!(
        updated.body, reread.body,
        "the update response is the stored form a later read serves"
    );
}

/// An EHR extract whose party carries a protected value in plaintext is
/// refused by a deployment that protects the scheme: a copy is stored verbatim
/// (RM common `master06-change_control_package.adoc` §Copying), so it cannot
/// be sealed, and nothing of it is stored.
#[tokio::test]
async fn an_imported_party_with_a_plaintext_identifier_is_refused() {
    use openehr_rm::v1_2::ehr_extract::common::extract::Extract;

    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let (target, _engine, target_party) = protected_service(&target_db);

    let created = source
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("an unprotected source stores the person as written");
    let (party, _) = committed_ids(&created.body);

    let ehr = Box::pin(source.create_ehr_meta(None, None))
        .await
        .map(|(id, _)| id)
        .expect("ehr");
    let mut status = source
        .ehr_status_at_time_response(ehr, None)
        .await
        .map(|r| r.body)
        .expect("status get");
    let status_ovid = status
        .pointer("/uid/value")
        .and_then(serde_json::Value::as_str)
        .expect("uid")
        .to_owned();
    status.as_object_mut().expect("status obj").remove("uid");
    *status.pointer_mut("/subject").expect("the subject") = serde_json::json!({
        "_type": "PARTY_SELF",
        "external_ref": {
            "_type": "PARTY_REF",
            "namespace": "demographic",
            "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": party }
        }
    });
    Box::pin(
        source
            .replace_ehr_status_meta(ehr, crate::fixtures::uv(&status, "251", Some(&status_ovid))),
    )
    .await
    .map(|m| m.uid)
    .expect("status update");
    let exported = source.extract_ehrs(ehr).await.expect("export_ehrs");
    let extract: Extract =
        openehr_its::json::from_canonical_value(exported.first().expect("one EXTRACT"))
            .expect("EXTRACT deserializes into the typed RM model");

    let refused = target
        .import_ehr(None, extract)
        .await
        .expect_err("a plaintext protected value cannot be stored verbatim");
    assert!(
        matches!(
            &refused,
            ServiceError::Unprocessable {
                kind: Invalid::Content,
                ..
            }
        ),
        "{refused:?}"
    );
    assert_eq!(
        sealed_and_versions(&target_party).await,
        (0, 0),
        "nothing of the refused party is stored"
    );
}

/// A party sealed by another deployment carries a reference whose row does not
/// travel in an extract, so the import is refused with the remedy, and nothing
/// of the party is stored (#3777).
#[tokio::test]
async fn an_imported_party_with_another_deployments_sealed_identifier_is_refused() {
    use openehr_rm::v1_2::ehr_extract::common::extract::Extract;

    let source_db = testkit::db().await.expect("testkit database");
    let (source, _source_engine, _source_party) = protected_service(&source_db);
    let target_db = testkit::db().await.expect("testkit database");
    let (target, _engine, target_party) = protected_service(&target_db);

    let created = source
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("the source seals the identifier");
    let (party, _) = committed_ids(&created.body);

    let ehr = Box::pin(source.create_ehr_meta(None, None))
        .await
        .map(|(id, _)| id)
        .expect("ehr");
    let mut status = source
        .ehr_status_at_time_response(ehr, None)
        .await
        .map(|r| r.body)
        .expect("status get");
    let status_ovid = status
        .pointer("/uid/value")
        .and_then(serde_json::Value::as_str)
        .expect("uid")
        .to_owned();
    status.as_object_mut().expect("status obj").remove("uid");
    *status.pointer_mut("/subject").expect("the subject") = serde_json::json!({
        "_type": "PARTY_SELF",
        "external_ref": {
            "_type": "PARTY_REF",
            "namespace": "demographic",
            "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": party }
        }
    });
    Box::pin(
        source
            .replace_ehr_status_meta(ehr, crate::fixtures::uv(&status, "251", Some(&status_ovid))),
    )
    .await
    .map(|m| m.uid)
    .expect("status update");
    let exported = source.extract_ehrs(ehr).await.expect("export_ehrs");
    let text = serde_json::to_string(&exported).expect("the extract serializes");
    assert!(
        text.contains("urn:ferroehr:protected-identifier:"),
        "the extract carries the party with its sealed reference"
    );
    assert!(
        !text.contains(SYNTHETIC_BSN),
        "the extract never carries the value"
    );
    let extract: Extract =
        openehr_its::json::from_canonical_value(exported.first().expect("one EXTRACT"))
            .expect("EXTRACT deserializes into the typed RM model");

    let refused = target
        .import_ehr(None, extract)
        .await
        .expect_err("a reference to a row this deployment does not hold is refused");
    assert!(
        matches!(
            &refused,
            ServiceError::Unprocessable {
                kind: Invalid::Content,
                ..
            }
        ),
        "{refused:?}"
    );
    assert!(
        format!("{refused:?}").contains("does not travel in an EHR extract"),
        "the refusal names the remedy: {refused:?}"
    );
    assert_eq!(
        sealed_and_versions(&target_party).await,
        (0, 0),
        "nothing of the refused party is stored"
    );
}

/// The dump carries a party's sealed rows as ciphertext and never the value,
/// and a load restores them, so the restored versions expand to what they
/// committed and the identifier resolves again.
#[tokio::test]
async fn a_dump_and_load_carries_the_sealed_identifiers() {
    let source_db = testkit::db().await.expect("testkit database");
    let (source, _source_engine, _) = protected_service(&source_db);
    let target_db = testkit::db().await.expect("testkit database");
    let (target, target_engine, target_party) = protected_service(&target_db);

    let created = source
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("create the person");
    let (vo, v1) = committed_ids(&created.body);
    source
        .party_update(
            PartyKind::Person,
            vo.clone(),
            v1,
            typed(&person_carrying(
                "Lovelace",
                &[("nl-bsn", SECOND_SYNTHETIC_BSN)],
            )),
            None,
        )
        .await
        .expect("change the identifier");

    let dir = crate::admin_fixture::archive_dir();
    let exported = source
        .export_ehrs(
            dir.clone(),
            ferroehr::service::admin::types::ExportSpec::canonical_json(64),
        )
        .await
        .expect("export");
    assert!(exported.is_empty(), "a clean export: {exported:?}");
    for entry in std::fs::read_dir(&dir).expect("the archive directory") {
        let text = std::fs::read_to_string(entry.expect("an entry").path()).unwrap_or_default();
        assert!(
            !text.contains(SYNTHETIC_BSN) && !text.contains(SECOND_SYNTHETIC_BSN),
            "no archive entry may carry a protected value"
        );
    }

    let loaded = target.load_ehrs(dir.clone()).await.expect("load");
    assert!(loaded.is_empty(), "a clean load: {loaded:?}");
    let versions = expanded_versions(&target_engine, &target_party, &vo).await;
    assert_eq!(versions.len(), 2, "both versions restored");
    assert_eq!(identifier_id(&versions[0]), SYNTHETIC_BSN);
    assert_eq!(identifier_id(&versions[1]), SECOND_SYNTHETIC_BSN);
    assert_eq!(
        live_holder(&target_party, "nl-bsn", SECOND_SYNTHETIC_BSN).await,
        Some(vo.parse().expect("a uuid")),
        "the restored live row resolves"
    );
    assert_eq!(
        live_holder(&target_party, "nl-bsn", SYNTHETIC_BSN).await,
        None,
        "the restored retired row stays retired"
    );
    std::fs::remove_dir_all(&dir).expect("remove the archive");
}

/// Loading a party whose version carries a protected value in plaintext into
/// a deployment that protects the scheme reports that container and stores
/// nothing of it.
#[tokio::test]
async fn a_load_refuses_a_plaintext_identifier_per_container() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &source_db.pool(),
    ));
    let target_db = testkit::db().await.expect("testkit database");
    let (target, _engine, target_party) = protected_service(&target_db);

    let created = source
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("an unprotected source stores the person as written");
    let (vo, _) = committed_ids(&created.body);

    let dir = crate::admin_fixture::archive_dir();
    source
        .export_ehrs(
            dir.clone(),
            ferroehr::service::admin::types::ExportSpec::canonical_json(64),
        )
        .await
        .expect("export");
    let loaded = target.load_ehrs(dir.clone()).await.expect("load");
    assert!(
        loaded.iter().any(|r| r.entity_id == vo && !r.dump_status),
        "the plaintext container is reported: {loaded:?}"
    );
    assert_eq!(
        sealed_and_versions(&target_party).await,
        (0, 0),
        "nothing of the refused container is stored"
    );
    std::fs::remove_dir_all(&dir).expect("remove the archive");
}

/// A physical party erasure removes every sealed row of the party, live and
/// retired, while a logical deletion keeps them and only takes them out of
/// resolution.
#[tokio::test]
async fn erasing_a_party_removes_its_sealed_identifiers() {
    let db = testkit::db().await.expect("testkit database");
    let (service, _engine, party_pool) = protected_service(&db);

    let created = service
        .party_create(
            PartyKind::Person,
            typed(&person_carrying("Lovelace", &[("nl-bsn", SYNTHETIC_BSN)])),
            None,
        )
        .await
        .expect("create the person");
    let (vo, v1) = committed_ids(&created.body);
    let changed = service
        .party_update(
            PartyKind::Person,
            vo.clone(),
            v1,
            typed(&person_carrying(
                "Lovelace",
                &[("nl-bsn", SECOND_SYNTHETIC_BSN)],
            )),
            None,
        )
        .await
        .expect("change the identifier");

    let party = ferroehr::ids::VoId(vo.parse().expect("a uuid"));
    let latest = changed.meta.expect("the update answers its version").uid;
    service
        .party_delete(PartyKind::Person, latest, None, None)
        .await
        .expect("logically delete the person");
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM party.national_identifier WHERE party_id = $1")
            .bind(party.0)
            .fetch_one(&party_pool)
            .await
            .expect("count the sealed rows");
    assert_eq!(kept, 2, "a logical deletion keeps every version's row");
    assert_eq!(
        live_holder(&party_pool, "nl-bsn", SECOND_SYNTHETIC_BSN).await,
        None,
        "a deleted party holds no live identifier"
    );

    service
        .physical_party_delete(vo)
        .await
        .expect("physically erase the person");
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM party.national_identifier WHERE party_id = $1")
            .bind(party.0)
            .fetch_one(&party_pool)
            .await
            .expect("count the sealed rows");
    assert_eq!(left, 0, "the erasure removes every sealed row of the party");
}

/// A sealed identifier seals and resolves on the SEPARATED demographic
/// credential.
///
/// The party pool authenticates as its own login role here, so the seal, the
/// open and the read of the stored digest all run on a credential that does
/// not own the table.
///
/// The digest half is the one that would fail silently: a digest that matches
/// no row reads exactly like an identifier nobody holds.
#[tokio::test]
async fn a_sealed_identifier_resolves_on_the_separated_demographic_credential() {
    use ferroehr::db::DbConfig;
    use ferroehr::service::demographic::identifier::store::IdentifierStore;

    let db = testkit::db().await.expect("testkit database");
    let settings = DbConfig {
        url: ferrohealth_secret::secret_url::SecretUrl::new(
            crate::fixtures::dsn_as(&db, "sealclin", "ferroehr_clinical").await,
        ),
        ..DbConfig::default()
    };
    let storage = ferroehr::db::domain::StorageConfig {
        party: ferroehr::db::domain::DomainDsn {
            url: Some(ferrohealth_secret::secret_url::SecretUrl::new(
                crate::fixtures::dsn_as(&db, "sealdemo", "ferroehr_party").await,
            )),
            url_file: None,
        },
        ..ferroehr::db::domain::StorageConfig::default()
    };
    assert!(
        storage.is_separated(ferroehr::db::domain::Domain::Party),
        "the fixture must actually separate the credentials, or this measures \
         the shared-credential path again"
    );

    let demographic =
        ferroehr::db::connect_domain(&settings, &storage, ferroehr::db::domain::Domain::Party)
            .await
            .expect("the party pool connects on its own credential");
    let mut conn = demographic.acquire().await.expect("a party connection");
    let store = IdentifierStore::new(demographic.clone());
    let keys = test_keys();
    let party = Uuid::now_v7();

    let row = IdentifierStore::seal(&mut conn, &keys, party, "nl-bsn", SYNTHETIC_BSN)
        .await
        .expect("the separated credential seals an identifier");
    drop(conn);
    assert_eq!(
        store.open(&keys, row).await.expect("open"),
        Some(SYNTHETIC_BSN.to_owned()),
        "the key holder reads the value back through the separated credential"
    );
    assert_eq!(
        live_holder(&demographic, "nl-bsn", SYNTHETIC_BSN).await,
        Some(party),
        "the digest the separated credential stored names the party"
    );
    assert_eq!(
        live_holder(&demographic, "nl-bsn", "987654321").await,
        None,
        "and an identifier nobody holds matches no row, so the assertion above \
         cannot be met by a read that answers everything"
    );
}

/// The split clinical role writes and reads the audit trail (#3267).
///
/// The store writes through the clinical pool. Before the grant migration the
/// Audit Record Repository was held by the single-domain pair only, so a
/// deployment whose clinical credential is a member of `ferroehr_clinical` alone
/// wrote no access record at all: dropped and metered under
/// `fail_mode = "open"`, every auditable operation refused under `"closed"`.
/// Runs as a fresh login role IN ROLE `ferroehr_clinical`, never as the superuser
/// the testkit pool is.
#[tokio::test]
async fn the_split_clinical_role_writes_and_reads_the_audit_trail() {
    use ferroehr::db::DbConfig;
    use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
    use ferroehr::system_log::store::AuditStore;

    let db = testkit::db().await.expect("testkit database");
    let clinical = ferroehr::db::connect(&DbConfig {
        url: ferrohealth_secret::secret_url::SecretUrl::new(
            crate::fixtures::dsn_as(&db, "audclin", "ferroehr_clinical").await,
        ),
        ..DbConfig::default()
    })
    .await
    .expect("the clinical pool connects on its own credential");

    let store = AuditStore::new(clinical.clone());
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "alice".clone_into(&mut event.user_id);
    let id = store
        .insert(&event, None)
        .await
        .expect("the clinical role inserts an access record");
    let seq: i64 = sqlx::query_scalar("SELECT chain_seq FROM audit.audit_event WHERE id = $1")
        .bind(id)
        .fetch_one(&clinical)
        .await
        .expect("the record's chain position");
    store
        .mark_fhir_feed_delivered(seq)
        .await
        .expect("a delivery stamp is the one sanctioned mutation");
    crate::fixtures::verify_chain(&clinical)
        .await
        .expect("the clinical role verifies the chain");

    let reader = ferroehr::db::connect(&DbConfig {
        url: ferrohealth_secret::secret_url::SecretUrl::new(
            crate::fixtures::dsn_as(&db, "audread", "ferroehr_clinical_reader").await,
        ),
        ..DbConfig::default()
    })
    .await
    .expect("the clinical reader connects");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&reader)
        .await
        .expect("the clinical reader reads the trail");
    assert_eq!(count, 1);
    let rewrite = sqlx::query("UPDATE audit.audit_event SET principal = 'bob' WHERE id = $1")
        .bind(id)
        .execute(&clinical)
        .await;
    assert!(
        rewrite.is_err(),
        "the clinical role holds no grant to rewrite a record: {rewrite:?}"
    );
}
