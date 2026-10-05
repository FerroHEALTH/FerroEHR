// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end cohort queries against a real PostgreSQL 18 (shared testkit
//! harness): seed parties in the demographic domain, link each to its own EHR
//! through the linkage domain, then select a population by a demographic
//! predicate and run AQL over exactly the EHRs it resolves to.
//!
//! **No openEHR spec governs the cohort surface — our own design/extension**
//! (`ferroehr::service::linkage::cohort`). What the suite pins is the property
//! the design exists for: the two statement families never cross the boundary,
//! the crossing runs on three separated credentials, a small cell is withheld
//! rather than served, and every execution — suppressed ones included — leaves
//! one access record naming the cohort by digest and never by value.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;

use serde_json::{Value, json};
use uuid::Uuid;

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::linkage::cohort::config::{CohortConfig, PredicateBinding, PredicateKind};
use ferroehr::service::linkage::cohort::{CohortError, CohortPredicate, CohortQueryRequest};

use crate::fixtures::{composition, uv};

/// The archetype of the `ADDRESS` under the party's `contacts`, which the
/// `city` predicate binds to. A demographic container is decomposed into its
/// own row like composition content (`ferroehr::storage::structure`), so an
/// archetyped ADDRESS is the nearest archetyped ancestor of the ELEMENTs in its
/// `details`.
const CONTACT_ADDRESS_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-ADDRESS.address.v1";
/// The address CLUSTER archetype the postcode predicate binds to: a CLUSTER
/// under the party's `details`, so its ELEMENT children's nearest archetyped
/// ancestor is the CLUSTER rather than the party root.
const ADDRESS_CLUSTER_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-CLUSTER.address.v1";
/// The person archetype the sex and age-band predicates bind to: those leaves
/// sit under a plain at-coded `details` `ITEM_TREE`, so their nearest archetyped
/// ancestor is the PERSON root itself. Exercising both shapes is the point —
/// the join is on the ancestor ROW, not on where the leaf happens to sit.
const PERSON_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-PERSON.person.v1";

/// The full five-key allow-list, as a deployment would bind it.
fn cohort_config(small_cell_threshold: u32) -> CohortConfig {
    let text = |archetype: &str, node: &str, kind: PredicateKind| PredicateBinding {
        archetype: archetype.to_owned(),
        node: node.to_owned(),
        kind,
    };
    CohortConfig {
        small_cell_threshold,
        max_cohort_size: 100_000,
        predicates: BTreeMap::from([
            (
                "city".to_owned(),
                text(CONTACT_ADDRESS_ARCHETYPE, "at0012", PredicateKind::Text),
            ),
            (
                "postcode_area".to_owned(),
                text(
                    ADDRESS_CLUSTER_ARCHETYPE,
                    "at0014",
                    PredicateKind::TextPrefix,
                ),
            ),
            (
                "sex".to_owned(),
                text(PERSON_ARCHETYPE, "at0017", PredicateKind::Coded),
            ),
            (
                "age_band".to_owned(),
                text(PERSON_ARCHETYPE, "at0010", PredicateKind::BirthDate),
            ),
        ]),
    }
}

/// A PERSON carrying the four selectable leaves.
///
/// Three ancestor shapes on purpose. The party's `details` `ITEM_TREE` carries
/// a plain at-code, so the `sex` and birth-date ELEMENTs' nearest archetyped
/// ancestor row is the PERSON root; the postcode sits under a CLUSTER carrying
/// a full archetype HRID, so its ancestor is that CLUSTER; the city sits in the
/// `details` of an archetyped `ADDRESS` under `contacts`, so its ancestor is
/// the ADDRESS row. All three are what the predicate statement joins on
/// (the nested-set interval of the ancestor row carrying the archetype id,
/// `ferroehr::storage::codec`), and the third only exists because the
/// demographic containers are decomposed into rows of their own.
///
/// RM shapes: `PERSON` (`identities` 1..*, `contacts`, `details`), `CONTACT`
/// (`addresses` 1..*), `ADDRESS` (`details`), `ITEM_TREE` (`items`), `CLUSTER`
/// (`items`), `ELEMENT` (`value`) — `openehr_rm::v1_2`.
fn person(name: &str, city: &str, postcode: &str, sex: &str, born: &str) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": PERSON_ARCHETYPE,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": PERSON_ARCHETYPE },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0002",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0003",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0004",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }],
        "contacts": [{
            "_type": "CONTACT",
            "archetype_node_id": "at0005",
            "name": { "_type": "DV_TEXT", "value": "home" },
            "addresses": [{
                "_type": "ADDRESS",
                "archetype_node_id": CONTACT_ADDRESS_ARCHETYPE,
                "archetype_details": { "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": CONTACT_ADDRESS_ARCHETYPE },
                    "rm_version": "1.1.0" },
                "name": { "_type": "DV_TEXT", "value": "postal" },
                "details": {
                    "_type": "ITEM_TREE",
                    "archetype_node_id": "at0006",
                    "name": { "_type": "DV_TEXT", "value": "address" },
                    "items": [{
                        "_type": "ELEMENT",
                        "archetype_node_id": "at0012",
                        "name": { "_type": "DV_TEXT", "value": "city" },
                        "value": { "_type": "DV_TEXT", "value": city }
                    }]
                }
            }]
        }],
        "details": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "structure" },
            "items": [
                {
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0017",
                    "name": { "_type": "DV_TEXT", "value": "sex" },
                    "value": {
                        "_type": "DV_CODED_TEXT", "value": sex,
                        "defining_code": {
                            "_type": "CODE_PHRASE",
                            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                            "code_string": sex
                        }
                    }
                },
                {
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0010",
                    "name": { "_type": "DV_TEXT", "value": "date of birth" },
                    "value": { "_type": "DV_DATE", "value": born }
                },
                {
                    "_type": "CLUSTER",
                    "archetype_node_id": ADDRESS_CLUSTER_ARCHETYPE,
                    "archetype_details": { "_type": "ARCHETYPED",
                        "archetype_id": { "_type": "ARCHETYPE_ID",
                            "value": ADDRESS_CLUSTER_ARCHETYPE },
                        "rm_version": "1.1.0" },
                    "name": { "_type": "DV_TEXT", "value": "address" },
                    "items": [
                        {
                            "_type": "ELEMENT",
                            "archetype_node_id": "at0014",
                            "name": { "_type": "DV_TEXT", "value": "postcode" },
                            "value": { "_type": "DV_TEXT", "value": postcode }
                        }
                    ]
                }
            ]
        }
    })
}

/// The access-log columns the recording case reads back.
#[derive(Debug, sqlx::FromRow)]
struct AccessRow {
    domain: String,
    resource_id: Option<String>,
    purpose: Option<String>,
    result_count: Option<i64>,
}

/// One seeded subject: the party, the EHR it is the subject of, and the
/// composition committed into that EHR.
struct Subject {
    party: VoId,
    ehr: EhrId,
    composition_uid: String,
}

/// Seed a party, its EHR, the mapping between them, and one composition.
async fn seed(
    svc: &FerroEhrService,
    name: &str,
    city: &str,
    postcode: &str,
    sex: &str,
    born: &str,
) -> Subject {
    let party =
        Box::pin(svc.create_party(uv(&person(name, city, postcode, sex, born), "249", None)))
            .await
            .unwrap_or_else(|e| panic!("create_party ({name}): {e:?}"));
    let ehr = svc.create_ehr(None).await.expect("create_ehr");
    svc.link(party, ehr).await.expect("link party to ehr");
    let composition_uid = svc
        .create_composition(ehr, uv(&composition(name), "249", None))
        .await
        .unwrap_or_else(|e| panic!("create_composition ({name}): {e:?}"))
        .version_uid();
    Subject {
        party,
        ehr,
        composition_uid,
    }
}

/// The six-person corpus: three in Groningen, three in Assen, split by sex and
/// birth year so every predicate kind has something to select on.
async fn seed_corpus(svc: &FerroEhrService) -> Vec<Subject> {
    let people = [
        ("Ada", "Groningen", "9711AB", "F", "1980-03-01"),
        ("Bram", "Groningen", "9711CD", "M", "1955-07-14"),
        ("Cato", "Groningen", "9722EF", "F", "1990-11-30"),
        ("Daan", "Assen", "9401GH", "M", "1980-01-20"),
        ("Eva", "Assen", "9401IJ", "F", "1972-05-05"),
        ("Fons", "Assen", "9412KL", "M", "1965-09-09"),
    ];
    let mut seeded = Vec::with_capacity(people.len());
    for (name, city, postcode, sex, born) in people {
        seeded.push(seed(svc, name, city, postcode, sex, born).await);
    }
    seeded
}

/// The AQL every case runs: one row per composition in the scoped EHRs.
const COHORT_AQL: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";

/// A request over `predicates`, with the standard AQL and a declared purpose.
fn request(predicates: &[(&str, &str)]) -> CohortQueryRequest {
    CohortQueryRequest {
        aql: COHORT_AQL.to_owned(),
        predicates: predicates
            .iter()
            .map(|(name, value)| CohortPredicate {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
            })
            .collect(),
        purpose: Some("secondary-use".to_owned()),
        query: ferroehr::service::query::request::AqlQueryRequest::default(),
    }
}

/// The `uid/value` cells of a served `RESULT_SET`, sorted.
fn served_uids(result_set: &Value) -> Vec<String> {
    let mut uids: Vec<String> = result_set["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row[0].as_str().expect("uid cell").to_owned())
        .collect();
    uids.sort();
    uids
}

/// A city predicate selects exactly the linked EHRs of the parties it matches
/// — never a party outside the cohort, and never a party the map does not link.
#[tokio::test]
async fn a_city_predicate_selects_only_the_linked_ehrs() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_cohort(cohort_config(0));
    let corpus = seed_corpus(&svc).await;

    let outcome = svc
        .execute_cohort_query(request(&[("city", "Groningen")]))
        .await
        .expect("the cohort query runs");

    assert_eq!(outcome.cohort_size, 3, "three parties live in Groningen");
    assert_eq!(outcome.served_ehrs, 3, "each has exactly one EHR");
    assert!(!outcome.suppressed, "suppression is off at threshold 0");

    let mut expected: Vec<String> = corpus[..3]
        .iter()
        .map(|s| s.composition_uid.clone())
        .collect();
    expected.sort();
    assert_eq!(
        served_uids(&outcome.query.result_set),
        expected,
        "exactly the Groningen compositions are served"
    );

    let mut served: Vec<String> = outcome
        .query
        .served_ehrs
        .iter()
        .map(|(ehr, _rows)| ehr.clone())
        .collect();
    served.sort();
    let mut linked: Vec<String> = corpus[..3].iter().map(|s| s.ehr.to_string()).collect();
    linked.sort();
    assert_eq!(
        served, linked,
        "the served EHRs are exactly the ones the map linked those parties to"
    );

    let cohort = &outcome.query.result_set["meta"]["cohort"];
    assert_eq!(cohort["size"], json!(3));
    assert_eq!(cohort["served_ehrs"], json!(3));
    assert_eq!(cohort["suppressed"], json!(false));
    assert_eq!(
        cohort["definition"].as_str().map(str::len),
        Some(64),
        "the cohort is named by a hex SHA-256 digest: {cohort}"
    );
    assert!(
        !outcome.query.result_set.to_string().contains("Groningen"),
        "no predicate value reaches the served document"
    );
}

/// A cohort narrower than the configured floor has its rows withheld, and the
/// response says so rather than looking like an empty cohort.
#[tokio::test]
async fn a_small_cell_is_suppressed() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_cohort(cohort_config(5));
    seed_corpus(&svc).await;

    let outcome = svc
        .execute_cohort_query(request(&[("city", "Groningen")]))
        .await
        .expect("the cohort query runs");

    assert!(
        outcome.suppressed,
        "three served EHRs is below the floor of 5"
    );
    assert_eq!(
        outcome.cohort_size, 3,
        "the cohort itself is still reported"
    );
    assert_eq!(outcome.served_ehrs, 0, "nothing was disclosed");
    assert!(
        served_uids(&outcome.query.result_set).is_empty(),
        "the rows are withheld"
    );
    assert_eq!(outcome.query.served_rows, 0);
    let cohort = &outcome.query.result_set["meta"]["cohort"];
    assert_eq!(cohort["suppressed"], json!(true));
    assert_eq!(cohort["served_ehrs"], json!(0));

    // The same corpus above the floor is served, so the assertion above is not
    // met by a suppressor that withholds everything.
    let wide = svc
        .execute_cohort_query(request(&[("postcode_area", "9")]))
        .await
        .expect("the wide cohort runs");
    assert!(!wide.suppressed, "six served EHRs clears the floor of 5");
    assert_eq!(wide.served_ehrs, 6);
}

/// Several predicates INTERSECT, and each kind selects on its own leaf.
#[tokio::test]
async fn predicates_intersect() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_cohort(cohort_config(0));
    let corpus = seed_corpus(&svc).await;

    // city ∧ sex: Ada and Cato are the Groningen women.
    let intersected = svc
        .execute_cohort_query(request(&[("city", "Groningen"), ("sex", "F")]))
        .await
        .expect("the intersected cohort runs");
    assert_eq!(intersected.cohort_size, 2, "Ada and Cato");
    let mut expected = vec![
        corpus[0].composition_uid.clone(),
        corpus[2].composition_uid.clone(),
    ];
    expected.sort();
    assert_eq!(served_uids(&intersected.query.result_set), expected);

    // The prefix predicate: `9711` covers Ada and Bram, not Cato's `9722`.
    let prefix = svc
        .execute_cohort_query(request(&[("postcode_area", "9711")]))
        .await
        .expect("the prefix cohort runs");
    assert_eq!(prefix.cohort_size, 2, "Ada and Bram share the 9711 area");

    // A `%` in a prefix value is a literal, not a wildcard — the escape holds.
    let escaped = svc
        .execute_cohort_query(request(&[("postcode_area", "97%")]))
        .await
        .expect("the escaped cohort runs");
    assert_eq!(escaped.cohort_size, 0, "no postcode literally starts `97%`");

    // The age band: born 1980, so between 40 and 49 in 2020 and older since.
    // Anchored on the database's own clock rather than a fixed year, because
    // the statement compares against `current_date`.
    let this_year: i32 = sqlx::query_scalar("SELECT date_part('year', current_date)::int")
        .fetch_one(&db.pool())
        .await
        .expect("the database year");
    let band = format!("{}-{}", this_year - 1981, this_year - 1980);
    let aged = svc
        .execute_cohort_query(request(&[("age_band", band.as_str())]))
        .await
        .unwrap_or_else(|e| panic!("the age-band cohort runs ({band}): {e:?}"));
    assert_eq!(
        aged.cohort_size, 2,
        "Ada (1980-03-01) and Daan (1980-01-20) are the 1980 cohort, band {band}"
    );
}

/// A predicate the deployment did not bind, and a deployment that bound none,
/// are refused with their own typed errors rather than an empty cohort.
#[tokio::test]
async fn an_unknown_predicate_and_an_unbound_deployment_are_refused() {
    let db = testkit::db().await.expect("testkit database");

    let unbound = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    assert!(!unbound.cohort_enabled(), "no predicate is bound");
    assert!(
        matches!(
            unbound
                .execute_cohort_query(request(&[("city", "Assen")]))
                .await,
            Err(CohortError::NotConfigured)
        ),
        "an unbound deployment refuses NotConfigured"
    );

    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_cohort(cohort_config(0));
    assert!(svc.cohort_enabled());
    match svc
        .execute_cohort_query(request(&[("national_identifier", "x")]))
        .await
    {
        Err(CohortError::UnknownPredicate(name)) => assert_eq!(name, "national_identifier"),
        other => panic!("an unbound predicate name must be refused, got {other:?}"),
    }
    assert!(
        matches!(
            svc.execute_cohort_query(request(&[])).await,
            Err(CohortError::NoPredicates)
        ),
        "a cohort query names at least one predicate"
    );
    match svc
        .execute_cohort_query(request(&[("age_band", "forty to fifty")]))
        .await
    {
        Err(CohortError::InvalidValue { name, .. }) => assert_eq!(name, "age_band"),
        other => panic!("a malformed age band must be refused, got {other:?}"),
    }
    let mut prescoped = request(&[("city", "Assen")]);
    prescoped.query.ehr_ids = vec![Uuid::now_v7().to_string()];
    assert!(
        matches!(
            svc.execute_cohort_query(prescoped).await,
            Err(CohortError::ScopeNotAllowed)
        ),
        "the cohort is the scope; a request may not supply its own"
    );
}

/// Every execution — the suppressed one included — leaves one linkage-domain
/// access record naming the cohort by digest and carrying no predicate value.
#[tokio::test]
async fn every_execution_is_recorded_as_a_linkage_access() {
    use ferroehr::system_log::config::{AuditConfig, StoreConfig};
    use ferroehr::system_log::sender::{AuditHandle, start};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let audit_config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            sgb_v_309_controller: false,
        },
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit_config, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    // The floor is above the corpus, so this execution is the suppressed one —
    // the case a record could most plausibly be skipped on.
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_cohort(cohort_config(5))
        .with_audit(sender);
    seed_corpus(&svc).await;

    let outcome = svc
        .execute_cohort_query(request(&[("city", "Groningen")]))
        .await
        .expect("the cohort query runs");
    assert!(outcome.suppressed);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let record = loop {
        let found: Option<AccessRow> = sqlx::query_as(
            "SELECT domain, resource_id, purpose, result_count FROM audit.audit_event \
             WHERE domain = 'linkage' AND resource_id LIKE 'cohort:%' \
             ORDER BY recorded_at DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("read the access log");
        if let Some(row) = found {
            break row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the cohort execution was not recorded within the drain window"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };

    assert_eq!(
        record.domain, "linkage",
        "a cohort query is a linkage access"
    );
    assert_eq!(
        record.resource_id.as_deref(),
        Some(format!("cohort:{}", outcome.definition_hash).as_str()),
        "the record names the cohort by its definition digest"
    );
    assert_eq!(
        record.purpose.as_deref(),
        Some("secondary-use"),
        "and the purpose the caller declared"
    );
    assert_eq!(
        record.result_count,
        Some(0),
        "a suppressed execution disclosed nothing, and says so"
    );

    let events: Vec<String> = sqlx::query_scalar("SELECT fhir::text FROM audit.audit_event")
        .fetch_all(&pool)
        .await
        .expect("every recorded event");
    for event in events {
        assert!(
            !event.contains("Groningen"),
            "no access record may carry the value the cohort was selected on: {event}"
        );
    }
}

/// Neither statement family names the other domain, and the clinical one binds
/// its EHR scope as ONE uuid array rather than one parameter per id.
#[test]
fn the_two_statements_never_cross_the_boundary() {
    use std::sync::Arc;

    use ferroehr::aql::ir::Params;
    use ferroehr::aql::lineage::ArchetypeLineage;
    use ferroehr::aql::sql::SqlCtx;

    // The demographic half: four constant statements, none of which can reach
    // a clinical archetype or another domain's schema.
    for kind in [
        PredicateKind::Text,
        PredicateKind::TextPrefix,
        PredicateKind::Coded,
        PredicateKind::BirthDate,
    ] {
        let sql = ferroehr::service::linkage::cohort::predicate::predicate_sql(kind);
        for forbidden in [
            "openehr-ehr-",
            "/",
            "clinical.",
            "party.",
            "demographic.",
            "linkage.",
            "subject_ehr",
        ] {
            assert!(
                !sql.contains(forbidden),
                "the {} predicate must not name `{forbidden}`: {sql}",
                kind.as_str()
            );
        }
    }

    // The clinical half: the cohort's EHR ids arrive as one array bind.
    let ehr_ids: Vec<EhrId> = (0..3).map(|_| EhrId(Uuid::now_v7())).collect();
    let ctx = SqlCtx {
        system_id: "sys.example.com".to_owned(),
        ehr_ids: ehr_ids.clone(),
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    let ast = openehr_query::parser::parse_str(COHORT_AQL).expect("the AQL parses");
    let params = Params::new();
    let ir = ferroehr::aql::plan(
        &ast,
        &params,
        ferroehr::config::profile::SpecProfile::default(),
    )
    .expect("the AQL plans");
    let prepared = ferroehr::aql::sql::build(&ir, &params, &ctx).expect("the AQL lowers");

    assert!(
        prepared.sql.contains("= ANY("),
        "the EHR scope binds as an array: {}",
        prepared.sql
    );
    for forbidden in ["city", "postcode", "party", "subject_ehr", "ELEMENT"] {
        assert!(
            !prepared.sql.contains(forbidden),
            "the clinical statement must not name `{forbidden}`: {}",
            prepared.sql
        );
    }
    let uuid_binds = prepared
        .values
        .0
        .0
        .iter()
        .filter(|value| matches!(value, sea_query::Value::Uuid(_)))
        .count();
    assert_eq!(
        uuid_binds, 0,
        "three EHR ids must ride ONE array bind, not one parameter each: {:?}",
        prepared.values.0.0
    );
}

/// The whole crossing runs with each domain on its OWN login role, none of
/// which can reach another's schema.
///
/// The separation is what makes the cohort surface safe to have at all: if one
/// credential could run all three statements, the boundary would be a naming
/// convention rather than a database fact.
#[tokio::test]
async fn the_crossing_runs_on_three_separated_credentials() {
    use ferroehr::config::secret::SecretUrl;
    use ferroehr::db::DbConfig;

    let db = testkit::db().await.expect("testkit database");
    let settings = DbConfig {
        url: SecretUrl::new(crate::fixtures::dsn_as(&db, "cohclin", "ferroehr_clinical").await),
        ..DbConfig::default()
    };
    let storage = ferroehr::db::domain::StorageConfig {
        party: ferroehr::db::domain::DomainDsn {
            url: Some(SecretUrl::new(
                crate::fixtures::dsn_as(&db, "cohdemo", "ferroehr_party").await,
            )),
            url_file: None,
        },
        linkage: ferroehr::db::domain::DomainDsn {
            url: Some(SecretUrl::new(
                crate::fixtures::dsn_as(&db, "cohlink", "ferroehr_linkage").await,
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
    let linkage = pools.linkage.clone();

    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
        &pools.clinical,
    ))
    .with_demographic_pool(pools.party.clone())
    .with_linkage_pool(linkage.clone())
    .with_cohort(cohort_config(0));
    let corpus = seed_corpus(&svc).await;

    let outcome = svc
        .execute_cohort_query(request(&[("city", "Groningen")]))
        .await
        .expect("the crossing runs on three credentials");
    assert_eq!(outcome.cohort_size, 3);
    let mut expected: Vec<String> = corpus[..3]
        .iter()
        .map(|s| s.composition_uid.clone())
        .collect();
    expected.sort();
    assert_eq!(served_uids(&outcome.query.result_set), expected);

    // The credential that performed the crossing cannot perform either end of
    // it, which is the property the separation buys.
    assert!(
        sqlx::query("SELECT count(*) FROM party.node")
            .fetch_one(&linkage)
            .await
            .is_err(),
        "the linkage credential must not be able to read the demographic domain"
    );
    assert!(
        sqlx::query("SELECT count(*) FROM clinical.node")
            .fetch_one(&linkage)
            .await
            .is_err(),
        "nor the clinical one"
    );
}

/// The linked party is never disclosed: the served document carries the AQL's
/// own projection and the cohort digest, and nothing that identifies a person.
#[tokio::test]
async fn the_result_carries_no_party_identifier() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_cohort(cohort_config(0));
    let corpus = seed_corpus(&svc).await;

    let outcome = svc
        .execute_cohort_query(request(&[("city", "Groningen")]))
        .await
        .expect("the cohort query runs");
    let rendered = outcome.query.result_set.to_string();
    for subject in &corpus {
        assert!(
            !rendered.contains(&subject.party.to_string()),
            "a party id must never reach the caller: {rendered}"
        );
    }
}
