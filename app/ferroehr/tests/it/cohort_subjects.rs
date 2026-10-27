// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Cohort queries that name their subjects by identifier, under each EHR
//! index provider: the local map, and an IHE patient identity source asked
//! over PIXm (ITI-83) through the stub of the `ehr_index_ihe` suite.
//!
//! Under the identity source the server holds only the pseudonym `EHR_STATUS`
//! carries in its own clinical domain (RM ehr master04 §EHR Status,
//! `EHR_STATUS.subject`), so a named subject is translated forward into that
//! domain and a demographic predicate, which would need the party-to-EHR map,
//! is refused. No openEHR spec governs the cohort surface: our own design.

#![expect(
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;

use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr_index::IndexError;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr::service::ehr_index::types::SubjectRef;
use ferroehr::service::linkage::cohort::config::{CohortConfig, PredicateBinding, PredicateKind};
use ferroehr::service::linkage::cohort::{CohortError, CohortPredicate, CohortQueryRequest};
use ferroehr::service::status::{CallStatusType, SmError};

use crate::ehr_index_ihe::{
    CLINICAL, OPERATION, RED_ID, answer, red_subject, red_to_blue, service,
};
use crate::fixtures::{composition, uv};

/// A synthetic source domain for the local-map cases.
const LOCAL_DOMAIN: &str = "urn:oid:2.999.1.4114";

/// The AQL every case runs: one row per composition in the scoped EHRs.
const COHORT_AQL: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";

/// Subject cohorts on, and one predicate bound so the refusal case has a
/// predicate the deployment did bind.
fn cohort_config() -> CohortConfig {
    CohortConfig {
        small_cell_threshold: 0,
        subjects: true,
        predicates: BTreeMap::from([(
            "city".to_owned(),
            PredicateBinding {
                archetype: "openEHR-DEMOGRAPHIC-ADDRESS.address.v1".to_owned(),
                node: "at0012".to_owned(),
                kind: PredicateKind::Text,
            },
        )]),
        ..CohortConfig::default()
    }
}

/// A request naming `subjects`, with the standard AQL and a declared purpose.
fn request(subjects: Vec<SubjectRef>) -> CohortQueryRequest {
    CohortQueryRequest {
        aql: COHORT_AQL.to_owned(),
        predicates: Vec::new(),
        subjects,
        purpose: Some("secondary-use".to_owned()),
        query: ferroehr::service::query::request::AqlQueryRequest::default(),
    }
}

/// One composition committed into `ehr`, by its version uid.
async fn commit(svc: &FerroEhrService, ehr: EhrId, name: &str) -> String {
    svc.create_composition(ehr, uv(&composition(name), "249", None))
        .await
        .unwrap_or_else(|e| panic!("create_composition ({name}): {e:?}"))
        .version_uid()
}

/// The served EHRs of an outcome, sorted.
fn served(outcome: &ferroehr::service::linkage::cohort::CohortOutcome) -> Vec<String> {
    let mut ehrs: Vec<String> = outcome
        .query
        .served_ehrs
        .iter()
        .map(|(ehr, _rows)| ehr.clone())
        .collect();
    ehrs.sort();
    ehrs
}

/// Under the local map the named subjects resolve through the map, and an
/// EHR whose subject the cohort does not name is never served.
#[tokio::test]
async fn named_subjects_resolve_through_the_local_map() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_ehr_index(EhrIndexProvider::Local)
        .with_cohort(cohort_config());
    let mut named = Vec::new();
    let mut expected = Vec::new();
    for value in ["4114-a", "4114-b", "4114-c"] {
        let subject = SubjectRef::person(value, LOCAL_DOMAIN);
        let ehr = svc
            .create_ehr_for_subject(subject.clone(), None)
            .await
            .expect("an EHR for the subject");
        commit(&svc, ehr, value).await;
        if value != "4114-c" {
            named.push(subject);
            expected.push(ehr.to_string());
        }
    }
    expected.sort();

    let outcome = svc
        .execute_cohort_query(request(named))
        .await
        .expect("the cohort query runs");

    assert_eq!(outcome.cohort_size, 2);
    assert_eq!(
        served(&outcome),
        expected,
        "exactly the named subjects' EHRs"
    );
    assert!(
        !outcome.definition_hash.contains("4114-a"),
        "the digest names the cohort without its identifiers"
    );

    let unknown = svc
        .execute_cohort_query(request(vec![SubjectRef::person("absent", LOCAL_DOMAIN)]))
        .await
        .expect("an unknown subject adds nothing");
    let single = svc
        .execute_cohort_query(request(vec![SubjectRef::person("4114-a", LOCAL_DOMAIN)]))
        .await
        .expect("one named subject");
    assert_eq!(single.cohort_size, 1);
    assert_eq!(
        single.definition_hash, unknown.definition_hash,
        "a subject enters the digest by its namespace alone"
    );
    assert_eq!(
        unknown.cohort_size, 0,
        "a subject no EHR belongs to adds nothing"
    );
}

/// Under an IHE identity source a subject named in another domain is
/// translated forward into the clinical domain, and its EHR alone is served.
#[tokio::test]
async fn a_named_subject_is_translated_forward_under_an_identity_source() {
    let pix = MockServer::start().await;
    let pseudonym = Uuid::now_v7().to_string();
    Mock::given(method("POST"))
        .and(path(OPERATION))
        .respond_with(answer(200, &red_to_blue(&pseudonym)))
        .mount(&pix)
        .await;
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &pix.uri()).with_cohort(cohort_config());
    let ehr = svc
        .create_ehr_for_subject(red_subject(), None)
        .await
        .expect("an EHR under the clinical pseudonym");
    commit(&svc, ehr, "named").await;
    // An EHR of another clinical pseudonym, which the cohort does not name.
    let local = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_ehr_index(EhrIndexProvider::Local);
    let other = local
        .create_ehr_for_subject(
            SubjectRef::person(Uuid::now_v7().to_string(), CLINICAL),
            None,
        )
        .await
        .expect("an EHR of another subject");
    commit(&svc, other, "unnamed").await;

    let outcome = svc
        .execute_cohort_query(request(vec![red_subject()]))
        .await
        .expect("the cohort query runs");

    assert_eq!(outcome.cohort_size, 1);
    assert_eq!(served(&outcome), vec![ehr.to_string()]);
    let document = outcome.query.result_set.to_string();
    assert!(
        !document.contains(RED_ID) && !document.contains(&pseudonym),
        "neither identifier reaches the served document"
    );
}

/// A demographic predicate under an identity source is refused before any
/// domain or the PIX Manager is asked: the server holds no party-to-EHR map
/// and never translates a pseudonym back.
#[tokio::test]
async fn a_predicate_is_refused_under_an_identity_source() {
    let pix = MockServer::start().await;
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &pix.uri()).with_cohort(cohort_config());
    let mut predicated = request(vec![red_subject()]);
    predicated.predicates = vec![CohortPredicate {
        name: "city".to_owned(),
        value: "Groningen".to_owned(),
    }];

    let refused = svc
        .execute_cohort_query(predicated)
        .await
        .expect_err("a predicate needs the map");
    assert!(
        matches!(refused, CohortError::IdentityNotHeld),
        "{refused:?}"
    );
    assert_eq!(
        SmError::from(refused).status,
        CallStatusType::PreconditionViolation
    );
    assert!(
        pix.received_requests().await.expect("recorded").is_empty(),
        "a refused cohort asks the source nothing"
    );
}

/// A failing identity source is the cohort's failure, never an empty cohort,
/// and a service with no provider refuses a named subject.
#[tokio::test]
async fn a_failing_resolution_fails_the_cohort() {
    let pix = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(OPERATION))
        .respond_with(ResponseTemplate::new(503))
        .mount(&pix)
        .await;
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &pix.uri()).with_cohort(cohort_config());

    let refused = svc
        .execute_cohort_query(request(vec![red_subject()]))
        .await
        .expect_err("the source failed");
    assert!(
        matches!(refused, CohortError::Subject(IndexError::IdentitySource(_))),
        "{refused:?}"
    );
    assert_eq!(SmError::from(refused).status, CallStatusType::Exception);

    let bare = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_cohort(cohort_config());
    let refused = bare
        .execute_cohort_query(request(vec![red_subject()]))
        .await
        .expect_err("no provider installed");
    assert!(
        matches!(refused, CohortError::Subject(IndexError::Unconfigured)),
        "{refused:?}"
    );
}

/// `[cohort] subjects = true` turns the surface on without a bound predicate.
#[test]
fn subject_cohorts_turn_the_surface_on() {
    assert!(!CohortConfig::default().is_enabled());
    assert!(
        CohortConfig {
            subjects: true,
            ..CohortConfig::default()
        }
        .is_enabled()
    );
}
