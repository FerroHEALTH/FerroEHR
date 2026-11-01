// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR index's IHE patient identity source against a stub PIX Manager
//! that answers with the IHE PIXm 3.1.0 IG's own ITI-83 examples
//! (`corpus/ihe-pixm/`), and the provider seam around it.
//!
//! The stub is `wiremock`; the IG answers are read from the vendored package
//! and served verbatim, except where a test puts a UUID in the clinical
//! domain's `targetIdentifier`, because the clinical identifier is the
//! pseudonym `EHR_STATUS` carries (`[privacy] subject_namespaces`). No openEHR
//! spec governs where the cross-reference lives (RM common master04
//! §`PARTY_SELF`): our own design. A subject is resolved through the cohort
//! query that names it, the production path a subject resolution takes.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ferroehr::privacy::PrivacyPolicy;
use ferroehr::privacy::config::PrivacyConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr_index::config::EhrIndexConfig;
use ferroehr::service::ehr_index::ihe::{IdentitySourceError, IheIdentitySource};
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr::service::ehr_index::types::SubjectRef;
use ferroehr::service::error::ServiceError;
use ferroehr::service::linkage::cohort::CohortQueryRequest;
use ferroehr::service::linkage::cohort::config::CohortConfig;
use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_server::tls_client::TlsClientSection;

/// The IG's clinical ("blue") domain, the one the stub answers for.
pub(crate) const CLINICAL: &str = "urn:oid:1.3.6.1.4.1.21367.13.20.3000";
/// The IG's source ("red") domain the subjects are named in.
pub(crate) const RED: &str = "urn:oid:1.3.6.1.4.1.21367.13.20.1000";
/// The IG's red identifier, a synthetic value of the IHE example.
pub(crate) const RED_ID: &str = "IHERED-994";
/// The operation path under the stub's base (PIXm §2:3.83.4.1.2).
pub(crate) const OPERATION: &str = "/Patient/$ihe-pix";
/// The media type of every ITI-83 answer (PIXm §2:3.83.4.2.2).
const FHIR_JSON: &str = "application/fhir+json";

/// One file of the vendored PIXm package, parsed.
fn ig(file: &str) -> Value {
    let at = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/ihe-pixm/package")
        .join(file);
    serde_json::from_str(&std::fs::read_to_string(&at).expect("the vendored IG file reads"))
        .expect("the vendored IG file is JSON")
}

/// The IG's red-to-blue answer with `value` in the clinical identifier.
pub(crate) fn red_to_blue(value: &str) -> Value {
    let mut answer = ig("example/Parameters-pixm-response-mohralice-red-to-blue.json");
    let parameters = answer
        .get_mut("parameter")
        .and_then(Value::as_array_mut)
        .expect("parameters");
    for parameter in parameters {
        if parameter.get("name") == Some(&Value::from("targetIdentifier")) {
            let slot = parameter
                .pointer_mut("/valueIdentifier/value")
                .expect("an identifier value");
            *slot = Value::from(value);
        }
    }
    answer
}

pub(crate) fn answer(status: u16, body: &Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.to_string(), FHIR_JSON)
}

fn config(base: &str) -> EhrIndexConfig {
    EhrIndexConfig {
        pixm_url: SecretUrl::new(base),
        assigning_authority: CLINICAL.to_owned(),
        request_timeout_ms: 2_000,
        ..EhrIndexConfig::default()
    }
}

/// A service whose provider is the identity source at `base`, with the
/// clinical domain declared as the pseudonym namespace.
pub(crate) fn service(db: &testkit::TestDb, base: &str) -> FerroEhrService {
    let policy = PrivacyPolicy::compile(&PrivacyConfig {
        subject_namespaces: vec![CLINICAL.to_owned()],
        ..PrivacyConfig::default()
    })
    .expect("the deployment policy compiles");
    let source = IheIdentitySource::new(&config(base)).expect("the identity source builds");
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_privacy(Arc::new(policy))
        .with_ehr_index(EhrIndexProvider::Ihe(Arc::new(source)))
}

pub(crate) fn red_subject() -> SubjectRef {
    SubjectRef::person(RED_ID, RED)
}

/// The EHR ids a cohort naming `subject` resolves to, as its AQL projects
/// them, through the cohort query the `POST /query/cohort` route makes; a
/// refused resolution is the refusal.
async fn resolved(svc: &FerroEhrService, subject: SubjectRef) -> Result<Vec<String>, ServiceError> {
    let outcome = svc
        .execute_cohort_query(CohortQueryRequest {
            aql: "SELECT e/ehr_id/value FROM EHR e".to_owned(),
            subjects: vec![subject],
            ..CohortQueryRequest::default()
        })
        .await
        .map_err(ServiceError::from)?;
    Ok(outcome
        .query
        .result_set
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get(0).and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

/// Subject cohorts on, nothing suppressed.
fn subject_cohorts() -> CohortConfig {
    CohortConfig {
        small_cell_threshold: 0,
        subjects: true,
        ..CohortConfig::default()
    }
}

/// A subject named in another domain is found through the identifier the
/// clinical domain holds for it, on the EHR whose `EHR_STATUS` carries it.
#[tokio::test]
async fn a_subject_is_resolved_through_the_pix_manager() {
    let pix = MockServer::start().await;
    let pseudonym = Uuid::now_v7().to_string();
    Mock::given(method("POST"))
        .and(path(OPERATION))
        .respond_with(answer(200, &red_to_blue(&pseudonym)))
        .mount(&pix)
        .await;
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &pix.uri()).with_cohort(subject_cohorts());

    assert!(
        resolved(&svc, red_subject())
            .await
            .expect("the cohort runs")
            .is_empty(),
        "no EHR carries the pseudonym yet"
    );
    let ehr =
        crate::fixtures::create_ehr_subject(&svc, &SubjectRef::person(&pseudonym, CLINICAL)).await;

    let stored = svc
        .ehr_status_at_time_response(ehr, None)
        .await
        .map(|r| r.body)
        .expect("the status reads");
    assert_eq!(
        stored.pointer("/subject/external_ref/namespace"),
        Some(&Value::from(CLINICAL)),
        "EHR_STATUS names the clinical domain"
    );
    assert_eq!(
        stored.pointer("/subject/external_ref/id/value"),
        Some(&Value::from(pseudonym.as_str())),
        "EHR_STATUS carries the pseudonym, never the red id"
    );
    let found = resolved(&svc, red_subject())
        .await
        .expect("the cohort runs");
    assert_eq!(found, vec![ehr.to_string()]);

    let requests = pix.received_requests().await.expect("recorded");
    assert!(
        requests
            .iter()
            .all(|request| !request.url.as_str().contains(RED_ID)),
        "the POST form keeps the identifier out of every URL"
    );
}

/// The IG's not-found answer is an answer: no EHR.
#[tokio::test]
async fn an_unknown_subject_has_no_ehr() {
    let pix = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(OPERATION))
        .respond_with(answer(
            404,
            &ig("example/OperationOutcome-pixm-response-error-not-found.json"),
        ))
        .mount(&pix)
        .await;
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &pix.uri()).with_cohort(subject_cohorts());

    let found = resolved(&svc, red_subject())
        .await
        .expect("the cohort runs");
    assert_eq!(found, Vec::<String>::new(), "no EHR for an unknown subject");
}

/// The IG's example identifier is no pseudonym, so the source answering it
/// for the clinical domain is refused rather than matched.
#[tokio::test]
async fn a_clinical_identifier_that_is_no_pseudonym_is_refused() {
    let pix = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(OPERATION))
        .respond_with(answer(
            200,
            &ig("example/Parameters-pixm-response-mohralice-red-to-blue.json"),
        ))
        .mount(&pix)
        .await;
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &pix.uri()).with_cohort(subject_cohorts());

    let refused = resolved(&svc, red_subject())
        .await
        .expect_err("IHEBLUE-994 is no pseudonym");
    assert!(matches!(&refused, ServiceError::Fault(_)), "{refused:?}");
    assert!(refused.message().contains("no pseudonym"), "{refused}");
}

/// An identity source that fails is an error, never an answer from the
/// `EHR_STATUS` columns, even when an EHR carries the subject verbatim.
#[tokio::test]
async fn a_failing_source_never_falls_back_to_the_ehr_status_subject() {
    let pix = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(OPERATION))
        .respond_with(ResponseTemplate::new(503))
        .mount(&pix)
        .await;
    let db = testkit::db().await.expect("testkit database");
    // The verbatim subject is stored on an EHR, so a fallback would find it.
    let local = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_ehr_index(EhrIndexProvider::Local);
    crate::fixtures::create_ehr_subject(&local, &red_subject()).await;

    let svc = service(&db, &pix.uri()).with_cohort(subject_cohorts());
    let refused = resolved(&svc, red_subject())
        .await
        .expect_err("the source failed");
    assert!(matches!(&refused, ServiceError::Fault(_)), "{refused:?}");
}

/// A service with no provider refuses every subject-scoped call, naming both
/// choices.
#[tokio::test]
async fn a_service_without_a_provider_refuses_naming_both_choices() {
    let db = testkit::db().await.expect("testkit database");
    let bare = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_cohort(subject_cohorts());
    let refused = resolved(&bare, red_subject())
        .await
        .expect_err("no provider installed");
    assert!(
        refused.message().contains("provider = \"local\""),
        "{refused}"
    );
    assert!(refused.message().contains("pixm_url"), "{refused}");
}

/// The boot check reads the PIX Manager's `CapabilityStatement`; a refusal and
/// a body that is none are each the error they are.
#[tokio::test]
async fn the_probe_reads_the_capability_statement() {
    let pix = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/metadata"))
        .respond_with(answer(
            200,
            &ig("CapabilityStatement-IHE.PIXm.Manager.json"),
        ))
        .mount(&pix)
        .await;
    IheIdentitySource::new(&config(&pix.uri()))
        .expect("builds")
        .probe()
        .await
        .expect("a PIX Manager answers its capabilities");

    let refusing = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&refusing)
        .await;
    let error = IheIdentitySource::new(&config(&refusing.uri()))
        .expect("builds")
        .probe()
        .await
        .expect_err("a 500 is no capability statement");
    assert!(
        matches!(error, IdentitySourceError::Refused { .. }),
        "{error:?}"
    );

    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(answer(200, &red_to_blue("x")))
        .mount(&other)
        .await;
    let error = IheIdentitySource::new(&config(&other.uri()))
        .expect("builds")
        .probe()
        .await
        .expect_err("a Parameters body is no capability statement");
    assert!(
        matches!(error, IdentitySourceError::NotAFhirServer),
        "{error:?}"
    );
}

/// A PIX Manager behind a private CA is reached with `[ehr_index.tls]
/// trust_roots_file` naming that CA, beside the platform roots, and not
/// without it.
#[tokio::test]
async fn a_private_ca_is_trusted_through_the_tls_roots() {
    let body = serde_json::to_string(&ig("CapabilityStatement-IHE.PIXm.Manager.json"))
        .expect("serializes");
    let server = testkit::https::HttpsServer::answering(FHIR_JSON, body)
        .await
        .expect("the HTTPS stub starts");

    let trusting = EhrIndexConfig {
        tls: TlsClientSection {
            trust_roots_file: Some(server.ca_file.clone()),
            ..TlsClientSection::default()
        },
        ..config(&server.base_url)
    };
    IheIdentitySource::new(&trusting)
        .expect("builds with the test CA")
        .probe()
        .await
        .expect("the test CA is trusted");

    let error = IheIdentitySource::new(&config(&server.base_url))
        .expect("builds")
        .probe()
        .await
        .expect_err("the platform roots do not hold the test CA");
    assert!(
        matches!(error, IdentitySourceError::Unreachable(_)),
        "{error:?}"
    );

    let missing = EhrIndexConfig {
        tls: TlsClientSection {
            trust_roots_file: Some(server.ca_file.with_extension("absent")),
            ..TlsClientSection::default()
        },
        ..config(&server.base_url)
    };
    assert!(matches!(
        IheIdentitySource::new(&missing),
        Err(IdentitySourceError::Tls(_))
    ));
}
