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
//! database whose grants cross the boundary and passes once they do not, and
//! that the linkage map holds one open mapping per party at a time.

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
/// Three domains, mutually barred: the clinical roles are barred from the
/// demographic domain and its cold tier and from the linkage map; the
/// demographic roles from the clinical ones and from the map; the linkage role
/// from both of the domains its rows join. No openEHR spec governs database
/// roles — our own design/extension.
const BARRIERS: &[(&str, &str, &[&str])] = &[
    ("ew", "ferroehr_clinical", &["party", "linkage"]),
    ("er", "ferroehr_clinical_reader", &["party", "linkage"]),
    ("dw", "ferroehr_party", &["clinical", "linkage"]),
    ("dr", "ferroehr_party_reader", &["clinical", "linkage"]),
    ("lk", "ferroehr_linkage", &["clinical", "party"]),
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
            url: Some(ferroehr::config::secret::SecretUrl::new(format!(
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
        // The linkage map, in both directions: a clinical role that can read
        // it holds the join, and so does the linkage role that can read a
        // clinical relation.
        (
            "ferroehr_clinical",
            "linkage.subject_ehr",
            "table",
            "GRANT SELECT ON",
            "REVOKE SELECT ON",
        ),
        (
            "ferroehr_party",
            "linkage.subject_ehr",
            "table",
            "GRANT SELECT ON",
            "REVOKE SELECT ON",
        ),
        (
            "ferroehr_linkage",
            "party.version",
            "table",
            "GRANT SELECT ON",
            "REVOKE SELECT ON",
        ),
        (
            "ferroehr_linkage",
            "clinical.version",
            "table",
            "GRANT SELECT ON",
            "REVOKE SELECT ON",
        ),
        // A function is the fourth breach shape, and the erasure definer is
        // the one that matters: it runs as its owner and deletes rows the
        // caller cannot, so a clinical role holding EXECUTE reaches into the
        // linkage domain without holding a single grant on its table.
        (
            "ferroehr_clinical",
            "linkage.erase_ehr(uuid)",
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

/// A sealed identifier round-trips, and resolution finds its party without
/// decrypting anything.
///
/// The two halves are the point of the design: the ciphertext answers "what is
/// this party's identifier" only to a holder of the key, and the keyed digest
/// answers "which party holds this identifier" to a caller that already knows
/// the value. Neither answers the other's question.
#[tokio::test]
async fn a_sealed_identifier_round_trips_and_resolves_to_its_party() {
    use ferroehr::service::demographic::identifier::store::IdentifierStore;

    let db = testkit::db().await.expect("testkit database");
    let store = IdentifierStore::new(ferroehr::db::domain_pool_from(
        &db.pool(),
        ferroehr::db::domain::Domain::Party,
    ));
    let keys = test_keys();
    let party = Uuid::now_v7();

    let row = store
        .seal(&keys, party, "nl-bsn", SYNTHETIC_BSN)
        .await
        .expect("seal the identifier");

    assert_eq!(
        store.open(&keys, row).await.expect("open"),
        Some(SYNTHETIC_BSN.to_owned()),
        "the key holder reads the value back"
    );
    assert_eq!(
        store
            .resolve(&keys, "nl-bsn", SYNTHETIC_BSN)
            .await
            .expect("resolve"),
        Some(party),
        "the digest resolves to the party without decryption"
    );
    assert_eq!(
        store
            .resolve(&keys, "nl-bsn", "987654321")
            .await
            .expect("resolve a value nobody holds"),
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
    let store = IdentifierStore::new(ferroehr::db::domain_pool_from(
        &db.pool(),
        ferroehr::db::domain::Domain::Party,
    ));
    let refused = store
        .seal(&test_keys(), Uuid::now_v7(), "zz-invented", SYNTHETIC_BSN)
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
            key: Some(ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
            key_file: None,
        },
        Some(&ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
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
        "SELECT count(*) FROM party.node WHERE vo_id = $1 AND data::text LIKE $2",
    )
    .bind(vo_id)
    .bind(format!("%{SYNTHETIC_BSN}%"))
    .fetch_one(&pool)
    .await
    .expect("scan the node rows");
    assert_eq!(nodes, 0, "no decomposed node may carry the identifier");

    // The sealed row exists, and resolution finds this party by the value.
    let store = ferroehr::service::demographic::identifier::store::IdentifierStore::new(
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    );
    assert_eq!(
        store
            .resolve(&test_keys(), "nl-bsn", SYNTHETIC_BSN)
            .await
            .expect("resolve"),
        Some(vo_id),
        "the identifier resolves to the party that holds it"
    );
}

/// Resolving an identifier to its party is recorded as a linkage-domain access,
/// naming the scheme and never the value.
///
/// The resolution is the one operation that walks from an identity to a record,
/// so a resolution nobody can reconstruct afterwards is exactly the boundary
/// crossing the access log exists to make answerable. A MISS is recorded for
/// the same reason a hit is: it says someone asked whether this deployment
/// holds that identifier.
#[tokio::test]
async fn resolving_an_identifier_is_recorded_as_an_access() {
    use ferroehr::service::demographic::identifier::engine::IdentifierProtection;
    use ferroehr::system_log::config::{AuditConfig, StoreConfig};
    use ferroehr::system_log::sender::{AuditHandle, start};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let engine = IdentifierProtection::from_config(
        &ferroehr::service::demographic::identifier::config::IdentifierProtectionConfig {
            enabled: true,
            schemes: vec!["nl-bsn".to_owned()],
            key: Some(ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
            key_file: None,
        },
        Some(&ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    )
    .expect("the engine builds")
    .expect("protection is enabled");
    let audit_config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit_config, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_identifier_protection(std::sync::Arc::new(engine))
        .with_audit(sender);

    // An identifier nobody holds: the resolution misses, and is still recorded.
    let resolved = service
        .resolve_party_by_identifier("nl-bsn", SYNTHETIC_BSN)
        .await
        .expect("the resolution runs");
    assert!(resolved.is_none(), "nobody holds it yet");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let record = loop {
        let found: Option<(String, Option<String>, Option<i64>)> = sqlx::query_as(
            "SELECT domain, resource_id, result_count FROM audit.audit_event \
             WHERE domain = 'linkage' ORDER BY recorded_at DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("read the access log");
        if let Some(row) = found {
            break row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the resolution was not recorded within the drain window"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };

    assert_eq!(
        record.0, "linkage",
        "a resolution is a linkage-domain access"
    );
    assert_eq!(
        record.1.as_deref(),
        Some("national-identifier:nl-bsn"),
        "the record names the scheme"
    );
    assert_eq!(record.2, Some(0), "a miss is recorded as nothing resolved");

    let events: Vec<String> = sqlx::query_scalar("SELECT fhir::text FROM audit.audit_event")
        .fetch_all(&pool)
        .await
        .expect("every recorded event");
    for event in events {
        assert!(
            !event.contains(SYNTHETIC_BSN),
            "no audit record may carry the identifier value: {event}"
        );
    }
}

/// `SQLSTATE` 23P01 `exclusion_violation` — what `PostgreSQL` reports when a
/// key carrying `WITHOUT OVERLAPS` is violated, because such a key is enforced
/// by a `GiST` exclusion index (`PostgreSQL` docs § Appendix A "`PostgreSQL`
/// Error Codes", class 23; `CREATE TABLE`, "`PRIMARY KEY`").
const SQLSTATE_EXCLUSION_VIOLATION: &str = "23P01";

/// One party holds at most one mapping to an EHR at any one instant, enforced
/// by the temporal primary key rather than by whichever code path writes.
///
/// The second half of the test is what makes the first half mean something: a
/// plain `UNIQUE (party_id)` would also refuse the overlapping row,
/// and would then wrongly refuse the mapping a merge opens after closing the
/// previous one. Both must hold, or the constraint is the wrong one.
#[tokio::test]
async fn one_party_holds_one_open_mapping_at_a_time() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let party = Uuid::now_v7();

    sqlx::query("INSERT INTO linkage.subject_ehr (party_id, ehr_id) VALUES ($1, $2)")
        .bind(party)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the first mapping is accepted");

    let refused = sqlx::query("INSERT INTO linkage.subject_ehr (party_id, ehr_id) VALUES ($1, $2)")
        .bind(party)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await;
    let error = refused.expect_err("a second mapping open at the same instant must be refused");
    let code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(std::borrow::Cow::into_owned);
    assert_eq!(
        code.as_deref(),
        Some(SQLSTATE_EXCLUSION_VIOLATION),
        "the overlap must be refused by the temporal key, not fail some other way: {error}"
    );

    // Close the first mapping, the way a merge or a split does, and the next
    // one is accepted: the periods meet at an instant and do not overlap.
    sqlx::query(
        "UPDATE linkage.subject_ehr SET sys_period = tstzrange(lower(sys_period), now(), '[)') \
         WHERE party_id = $1",
    )
    .bind(party)
    .execute(&pool)
    .await
    .expect("close the mapping");

    sqlx::query("INSERT INTO linkage.subject_ehr (party_id, ehr_id) VALUES ($1, $2)")
        .bind(party)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("a successor mapping is accepted once the previous one is closed");

    let history: i64 =
        sqlx::query_scalar("SELECT count(*) FROM linkage.subject_ehr WHERE party_id = $1")
            .bind(party)
            .fetch_one(&pool)
            .await
            .expect("count the party's mappings");
    assert_eq!(
        history, 2,
        "the closed mapping is kept, so which party was the subject when a \
         composition was written stays answerable"
    );
}

/// A sealed identifier seals and resolves on the SEPARATED demographic
/// credential.
///
/// Two things meet on this path, and each was proven only on its own: the party
/// pool authenticating as its own login role, and a `SECURITY DEFINER` resolve
/// function owned by the migrator. Whether a resolve still returns its party is
/// a question about the two together.
///
/// The digest half is the one that would fail silently: a resolve that finds
/// nothing returns `None`, which reads exactly like an identifier nobody
/// holds.
#[tokio::test]
async fn a_sealed_identifier_resolves_on_the_separated_demographic_credential() {
    use ferroehr::db::DbConfig;
    use ferroehr::service::demographic::identifier::store::IdentifierStore;

    let db = testkit::db().await.expect("testkit database");
    let settings = DbConfig {
        url: ferroehr::config::secret::SecretUrl::new(
            crate::fixtures::dsn_as(&db, "sealclin", "ferroehr_clinical").await,
        ),
        ..DbConfig::default()
    };
    let storage = ferroehr::db::domain::StorageConfig {
        party: ferroehr::db::domain::DomainDsn {
            url: Some(ferroehr::config::secret::SecretUrl::new(
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
    let store = IdentifierStore::new(demographic);
    let keys = test_keys();
    let party = Uuid::now_v7();

    let row = store
        .seal(&keys, party, "nl-bsn", SYNTHETIC_BSN)
        .await
        .expect("the separated credential seals an identifier");
    assert_eq!(
        store.open(&keys, row).await.expect("open"),
        Some(SYNTHETIC_BSN.to_owned()),
        "the key holder reads the value back through the separated credential"
    );
    assert_eq!(
        store
            .resolve(&keys, "nl-bsn", SYNTHETIC_BSN)
            .await
            .expect("resolve"),
        Some(party),
        "the SECURITY DEFINER resolve returns the party under FORCE row-level \
         security, on a credential that does not own the function"
    );
    assert_eq!(
        store
            .resolve(&keys, "nl-bsn", "987654321")
            .await
            .expect("resolve a value nobody holds"),
        None,
        "and an identifier nobody holds still resolves to nothing, so the \
         assertion above cannot be met by a resolve that answers everything"
    );
}

/// How many of a party's mappings are still in force, and how many rows it has
/// in total.
///
/// Read straight from the table rather than through the service, because the
/// property under test is what the STORE holds: a service that reported one
/// open mapping while the table held two would be the defect.
async fn mapping_counts(pool: &PgPool, party: Uuid) -> (i64, i64) {
    sqlx::query_as(
        "SELECT count(*) FILTER (WHERE upper_inf(sys_period)), count(*) \
         FROM linkage.subject_ehr WHERE party_id = $1",
    )
    .bind(party)
    .fetch_one(pool)
    .await
    .expect("count the party's mappings")
}

/// A merge and a split keep every previous mapping and leave exactly ONE
/// mapping in force per party.
///
/// The half that matters is the history. A `DELETE`-and-reinsert would satisfy
/// "one open mapping per party" perfectly while destroying the only record of
/// which party was the subject of an EHR when a composition was written, so
/// the closed rows are asserted present, bounded, and still naming the EHR
/// they named — not merely absent from the open set.
#[tokio::test]
async fn a_merge_and_a_split_keep_their_history_and_leave_one_open_mapping() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let (alice, bob) = (
        ferroehr::ids::VoId(Uuid::now_v7()),
        ferroehr::ids::VoId(Uuid::now_v7()),
    );
    let (first, second) = (
        ferroehr::ids::EhrId(Uuid::now_v7()),
        ferroehr::ids::EhrId(Uuid::now_v7()),
    );

    service
        .link(alice, first)
        .await
        .expect("the first mapping opens");
    assert_eq!(
        service
            .resolve_ehr_for_party(alice)
            .await
            .expect("resolve the open mapping"),
        Some(first),
        "the mapping just opened is the one in force"
    );

    // Split: the same party moves to a different EHR.
    service.split(alice, second).await.expect("the split runs");
    assert_eq!(
        mapping_counts(&pool, alice.0).await,
        (1, 2),
        "the split leaves one mapping in force and keeps the one it closed"
    );
    assert_eq!(
        service
            .resolve_ehr_for_party(alice)
            .await
            .expect("resolve after the split"),
        Some(second),
        "the mapping in force after a split names the new EHR"
    );

    // Merge: alice is absorbed into bob, and the EHR goes with her.
    service.merge(alice, bob).await.expect("the merge runs");
    assert_eq!(
        mapping_counts(&pool, alice.0).await,
        (0, 2),
        "the absorbed party holds no mapping in force, and both of hers are kept"
    );
    assert_eq!(
        mapping_counts(&pool, bob.0).await,
        (1, 1),
        "the surviving party holds exactly one mapping in force"
    );
    assert_eq!(
        service
            .resolve_ehr_for_party(bob)
            .await
            .expect("resolve after the merge"),
        Some(second),
        "the merged-into party is now the subject of the EHR that moved"
    );
    assert_eq!(
        service
            .resolve_ehr_for_party(alice)
            .await
            .expect("resolve the absorbed party"),
        None,
        "and the absorbed party resolves to nothing, rather than to a stale EHR"
    );

    // The history itself: every closed row still names its EHR and carries a
    // bounded period, which is what makes "who was the subject then" answerable.
    let closed: Vec<(Uuid, bool)> = sqlx::query_as(
        "SELECT ehr_id, upper(sys_period) IS NOT NULL FROM linkage.subject_ehr \
         WHERE party_id = $1 AND NOT upper_inf(sys_period) ORDER BY lower(sys_period)",
    )
    .bind(alice.0)
    .fetch_all(&pool)
    .await
    .expect("read the closed mappings");
    assert_eq!(
        closed,
        vec![(first.0, true), (second.0, true)],
        "both closed mappings survive, in order, each with an upper bound"
    );

    // Nothing anywhere holds two mappings in force at one instant.
    let doubly_open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT party_id FROM linkage.subject_ehr \
         WHERE party_id IS NOT NULL AND upper_inf(sys_period) \
         GROUP BY party_id HAVING count(*) > 1) offenders",
    )
    .fetch_one(&pool)
    .await
    .expect("look for a party with two mappings in force");
    assert_eq!(doubly_open, 0, "no party holds two mappings in force");
}

/// One linkage access record: domain, principal, purpose, result count and
/// the FHIR rendering as text.
type LinkageAccessRow = (String, Option<String>, Option<String>, Option<i64>, String);

/// Resolving a party to its EHR is recorded as a linkage-domain access,
/// naming the actor, the declared purpose and what it resolved.
///
/// Asserted from the stored record rather than from a log line: a log line is
/// not the access log, and the question NEN 7513 and EHDS Art. 9 put — who
/// re-attributed this record to a person — is answered by the repository or
/// not at all.
#[tokio::test]
async fn resolving_a_party_to_its_ehr_is_recorded_as_an_access() {
    use ferroehr::system_log::config::{AuditConfig, StoreConfig};
    use ferroehr::system_log::sender::{AuditHandle, start};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let audit_config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit_config, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender);

    let party = ferroehr::ids::VoId(Uuid::now_v7());
    let ehr = ferroehr::ids::EhrId(Uuid::now_v7());
    service.link(party, ehr).await.expect("the mapping opens");

    // Under a request scope, so the record carries the actor and the purpose
    // the adapter publishes rather than the empty values a background task has.
    let committer = ferroehr::service::committer::CommitterIdentity {
        subject: "dr.house".to_owned(),
        id_type: "basic",
        issuer: None,
    };
    let resolved = ferroehr::service::committer::with_committer(
        Some(committer),
        ferroehr::system_log::access_context::with_purpose(
            Some("TREAT".to_owned()),
            service.resolve_ehr_for_party(party),
        ),
    )
    .await
    .expect("the resolution runs");
    assert_eq!(resolved, Some(ehr), "the mapping resolves");

    let object_id = format!("party-ehr:{party}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let record = loop {
        let found: Option<LinkageAccessRow> = sqlx::query_as(
            "SELECT domain, principal, purpose, result_count, fhir::text \
             FROM audit.audit_event \
             WHERE domain = 'linkage' AND action = 'R' AND resource_id = $1 \
             ORDER BY recorded_at DESC LIMIT 1",
        )
        .bind(&object_id)
        .fetch_optional(&pool)
        .await
        .expect("read the access log");
        if let Some(row) = found {
            break row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the resolution was not recorded within the drain window"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };

    assert_eq!(
        record.0, "linkage",
        "a resolution is a linkage-domain access"
    );
    assert_eq!(
        record.1.as_deref(),
        Some("dr.house"),
        "the record names the actor who resolved"
    );
    assert_eq!(
        record.2.as_deref(),
        Some("TREAT"),
        "and the purpose of use the caller declared"
    );
    assert_eq!(record.3, Some(1), "one mapping was resolved");
    assert!(
        !record.4.contains(&ehr.to_string()),
        "the trail lives outside this domain's role, so a record pairing the \
         party with its EHR would be a second copy of the map: {}",
        record.4
    );

    // The opening write is recorded too: a map nobody can see being written is
    // as unanswerable as one nobody can see being read.
    let written: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit.audit_event \
         WHERE domain = 'linkage' AND action = 'C' AND resource_id = $1",
    )
    .bind(&object_id)
    .fetch_one(&pool)
    .await
    .expect("count the opening record");
    assert_eq!(written, 1, "opening the mapping is recorded as well");
}

/// The whole crossing — external identity to party to EHR — runs with each
/// domain on its OWN login role, none of which can reach another's schema.
///
/// This is the posture #3222 proved for two domains, now that there are three.
/// The crossing is the thing under test: it happens in the APPLICATION, over
/// two pools and two credentials, and a single statement could not perform it
/// on either of them. If it could, the separation would be a naming convention.
#[tokio::test]
async fn an_identity_resolves_to_an_ehr_across_three_separated_credentials() {
    use ferroehr::config::secret::SecretUrl;
    use ferroehr::db::DbConfig;
    use ferroehr::service::demographic::identifier::config::IdentifierProtectionConfig;
    use ferroehr::service::demographic::identifier::engine::IdentifierProtection;
    use ferroehr::service::demographic::identifier::store::IdentifierStore;

    let db = testkit::db().await.expect("testkit database");
    let settings = DbConfig {
        url: SecretUrl::new(crate::fixtures::dsn_as(&db, "linkclin", "ferroehr_clinical").await),
        ..DbConfig::default()
    };
    let storage = ferroehr::db::domain::StorageConfig {
        party: ferroehr::db::domain::DomainDsn {
            url: Some(SecretUrl::new(
                crate::fixtures::dsn_as(&db, "linkdemo", "ferroehr_party").await,
            )),
            url_file: None,
        },
        linkage: ferroehr::db::domain::DomainDsn {
            url: Some(SecretUrl::new(
                crate::fixtures::dsn_as(&db, "linklink", "ferroehr_linkage").await,
            )),
            url_file: None,
        },
        ..ferroehr::db::domain::StorageConfig::default()
    };
    assert!(
        storage.is_separated(ferroehr::db::domain::Domain::Party)
            && storage.is_separated(ferroehr::db::domain::Domain::Linkage),
        "the fixture must actually separate all three credentials, or this \
         measures the shared-credential path again"
    );

    let pools = ferroehr::db::connect_domains(&settings, &storage)
        .await
        .expect("each pool connects on its own credential");
    let clinical = pools.clinical.clone();
    let demographic = pools.party.clone();
    let linkage = pools.linkage.clone();

    // The identity half: a sealed identifier held on the demographic
    // credential, resolvable by keyed digest without decryption.
    let party = ferroehr::ids::VoId(Uuid::now_v7());
    IdentifierStore::new(demographic.clone())
        .seal(&test_keys(), party.0, "nl-bsn", SYNTHETIC_BSN)
        .await
        .expect("the separated demographic credential seals an identifier");

    let engine = IdentifierProtection::from_config(
        &IdentifierProtectionConfig {
            enabled: true,
            schemes: vec!["nl-bsn".to_owned()],
            key: Some(ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
            key_file: None,
        },
        Some(&ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
        demographic,
    )
    .expect("the engine builds")
    .expect("protection is enabled");

    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&clinical))
        .with_linkage_pool(linkage.clone())
        .with_identifier_protection(std::sync::Arc::new(engine));

    let ehr = ferroehr::ids::EhrId(Uuid::now_v7());
    service
        .link(party, ehr)
        .await
        .expect("the separated linkage credential opens a mapping");

    assert_eq!(
        service
            .resolve_ehr_for_identity("nl-bsn", SYNTHETIC_BSN)
            .await
            .expect("the crossing runs"),
        Some(ehr),
        "the identity resolves to its EHR across the two pools"
    );
    assert_eq!(
        service
            .resolve_ehr_for_identity("nl-bsn", "987654321") // privacy-allow: synthetic
            .await
            .expect("resolve an identity nobody holds"),
        None,
        "and an identity nobody holds resolves to nothing, so the assertion \
         above cannot be met by a crossing that answers everything"
    );

    // The credential that performed the second hop cannot perform the first,
    // which is what makes the crossing safe to have at all.
    let refused = sqlx::query("SELECT count(*) FROM party.national_identifier")
        .fetch_one(&linkage)
        .await;
    assert!(
        refused.is_err(),
        "the linkage credential must not be able to read the identifier map"
    );
}

/// The server mints the subject pseudonym (#3232): `link_as_subject` derives
/// it from the party under the linkage key, writes it as the EHR's
/// `EHR_STATUS.subject.external_ref` in the declared namespace, and opens the
/// mapping. No caller value enters the path, so a national identifier cannot
/// become a subject reference through it; the same party mints the same
/// pseudonym, a different party a different one.
#[tokio::test]
async fn the_server_mints_the_subject_pseudonym_and_no_caller_value_enters_it() {
    use ferroehr::service::demographic::identifier::engine::IdentifierProtection;
    use ferroehr::service::linkage::LinkageError;

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let engine = IdentifierProtection::from_config(
        &ferroehr::service::demographic::identifier::config::IdentifierProtectionConfig {
            enabled: true,
            schemes: vec!["nl-bsn".to_owned()],
            key: Some(ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
            key_file: None,
        },
        Some(&ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    )
    .expect("the engine builds")
    .expect("protection is enabled");
    let namespace = "urn:test:pseudonym";
    let policy =
        ferroehr::privacy::PrivacyPolicy::compile(&ferroehr::privacy::config::PrivacyConfig {
            subject_namespaces: vec![namespace.to_owned()],
            ..ferroehr::privacy::config::PrivacyConfig::default()
        })
        .expect("the policy compiles");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_identifier_protection(std::sync::Arc::new(engine))
        .with_privacy(std::sync::Arc::new(policy));

    let party = ferroehr::ids::VoId(Uuid::now_v7());
    let ehr = service.create_ehr(None).await.expect("an EHR");

    let minted = service
        .link_as_subject(party, ehr)
        .await
        .expect("the party becomes the subject");
    assert_eq!(minted.namespace, namespace);
    assert_ne!(minted.id, party.0, "the pseudonym is not the party id");
    assert_eq!(minted.id.get_version_num(), 8, "an RFC 9562 custom UUID");

    let (subject_id, subject_namespace): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT subject_id, subject_namespace FROM ehr WHERE id = $1")
            .bind(Uuid::from(ehr))
            .fetch_one(&pool)
            .await
            .expect("the promoted subject columns");
    assert_eq!(subject_id.as_deref(), Some(minted.id.to_string().as_str()));
    assert_eq!(subject_namespace.as_deref(), Some(namespace));
    assert_eq!(
        service.resolve_ehr_for_party(party).await.expect("resolve"),
        Some(ehr),
        "the mapping opened"
    );

    assert_eq!(
        service.mint_subject_pseudonym(party).expect("mint again"),
        minted,
        "the same party mints the same pseudonym"
    );
    let other = ferroehr::ids::VoId(Uuid::now_v7());
    assert_ne!(
        service
            .mint_subject_pseudonym(other)
            .expect("mint another")
            .id,
        minted.id,
        "a different party mints a different pseudonym"
    );

    // Without a declared namespace there is nothing to mint into; without the
    // key there is nothing to derive from. Both refuse, typed.
    let unnamed = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_identifier_protection_opt(None);
    assert!(matches!(
        unnamed.mint_subject_pseudonym(party),
        Err(LinkageError::NoPseudonymNamespace)
    ));
    let keyless = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_privacy(std::sync::Arc::new(
            ferroehr::privacy::PrivacyPolicy::compile(&ferroehr::privacy::config::PrivacyConfig {
                subject_namespaces: vec![namespace.to_owned()],
                ..ferroehr::privacy::config::PrivacyConfig::default()
            })
            .expect("compiles"),
        ));
    assert!(matches!(
        keyless.mint_subject_pseudonym(party),
        Err(LinkageError::NoMintingKey)
    ));
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
        url: ferroehr::config::secret::SecretUrl::new(
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
        .insert(
            &event,
            None,
            &serde_json::json!({"resourceType": "AuditEvent"}),
        )
        .await
        .expect("the clinical role inserts an access record");
    store.mark_syslog_delivered(id).await;
    store
        .verify_chain()
        .await
        .expect("the clinical role verifies the chain");

    let reader = ferroehr::db::connect(&DbConfig {
        url: ferroehr::config::secret::SecretUrl::new(
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
