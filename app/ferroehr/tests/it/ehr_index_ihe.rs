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
//! §`PARTY_SELF`): our own design.

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
use ferroehr::service::status::CallStatusType;
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

/// A subject named in another domain is created under the identifier the
/// clinical domain holds for it, and found again through the same question.
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
    let svc = service(&db, &pix.uri());

    assert!(
        !svc.has_ehr_for_subject(red_subject()).await.expect("has"),
        "no EHR carries the pseudonym yet"
    );
    let ehr = svc
        .create_ehr_for_subject(red_subject(), None)
        .await
        .expect("create for the red subject");
    assert!(svc.has_ehr_for_subject(red_subject()).await.expect("has"));

    let stored = svc
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("the status reads");
    assert_eq!(
        stored.pointer("/subject/external_ref/namespace"),
        Some(&Value::from(CLINICAL)),
        "EHR_STATUS names the clinical domain"
    );
    assert_eq!(
        stored.pointer("/subject/external_ref/id/value"),
        Some(&Value::from(pseudonym.as_str())),
        "EHR_STATUS carries the pseudonym the source answered, never the red id"
    );
    let found = svc
        .get_ehrs_for_subject(red_subject())
        .await
        .expect("get for the red subject");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].ehr_id, ehr.to_string());

    let requests = pix.received_requests().await.expect("recorded");
    assert!(
        requests
            .iter()
            .all(|request| !request.url.as_str().contains(RED_ID)),
        "the POST form keeps the identifier out of every URL"
    );
}

/// The IG's not-found answer is an answer: no EHR, and no EHR is created.
#[tokio::test]
async fn an_unknown_subject_has_no_ehr_and_gets_none() {
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
    let svc = service(&db, &pix.uri());

    assert!(!svc.has_ehr_for_subject(red_subject()).await.expect("has"));
    assert!(
        svc.get_ehrs_for_subject(red_subject())
            .await
            .expect("get")
            .is_empty()
    );
    let refused = svc
        .create_ehr_for_subject(red_subject(), None)
        .await
        .expect_err("no clinical identifier, no EHR");
    assert_eq!(refused.status, CallStatusType::SubjectIdDoesNotExist);
}

/// The IG's example identifier is no pseudonym, so the source answering it
/// for the clinical domain is refused rather than written onto `EHR_STATUS`.
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
    let svc = service(&db, &pix.uri());

    let refused = svc
        .has_ehr_for_subject(red_subject())
        .await
        .expect_err("IHEBLUE-994 is no pseudonym");
    assert_eq!(refused.status, CallStatusType::Exception);
    assert!(refused.message.contains("no pseudonym"), "{refused}");
}

/// An identity source that fails is an error, never an answer from the local
/// `EHR_STATUS` columns, even when an EHR carries the subject verbatim.
#[tokio::test]
async fn a_failing_source_never_falls_back_to_the_local_map() {
    let pix = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(OPERATION))
        .respond_with(ResponseTemplate::new(503))
        .mount(&pix)
        .await;
    let db = testkit::db().await.expect("testkit database");
    // The verbatim subject is stored through the local map, so a fallback
    // would find it.
    let local = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_ehr_index(EhrIndexProvider::Local);
    local
        .create_ehr_for_subject(red_subject(), None)
        .await
        .expect("a locally stored subject");

    let svc = service(&db, &pix.uri());
    let refused = svc
        .has_ehr_for_subject(red_subject())
        .await
        .expect_err("the source failed");
    assert_eq!(refused.status, CallStatusType::Exception);
    let refused = svc
        .subject_ehrs(red_subject())
        .await
        .expect_err("the source failed");
    assert_eq!(refused.status, CallStatusType::Exception);
}

/// The identity source holds the cross-reference, so the `I_EHR_INDEX` writes
/// and the duplicate scan of the local map are refused, and a service with no
/// provider refuses every subject-scoped call, naming both choices.
#[tokio::test]
async fn the_local_map_is_not_written_under_an_identity_source() {
    let pix = MockServer::start().await;
    let db = testkit::db().await.expect("testkit database");
    let svc = service(&db, &pix.uri());
    let ehr = Uuid::now_v7().to_string();

    let refused = svc
        .add_ehr_subject(ehr.clone(), red_subject(), None, None)
        .await
        .expect_err("held by the source");
    assert_eq!(refused.status, CallStatusType::NotImplemented);
    let refused = svc
        .remove_subject(red_subject())
        .await
        .expect_err("held by the source");
    assert_eq!(refused.status, CallStatusType::NotImplemented);
    assert!(svc.index_conflicts().await.is_err());
    assert!(
        pix.received_requests().await.expect("recorded").is_empty(),
        "a refused write asks the source nothing"
    );

    let bare = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let refused = bare
        .has_ehr_for_subject(red_subject())
        .await
        .expect_err("no provider installed");
    assert!(
        refused.message.contains("provider = \"local\""),
        "{refused}"
    );
    assert!(refused.message.contains("pixm_url"), "{refused}");
}

/// The subjects of an EHR are the clinical pseudonym its `EHR_STATUS`
/// carries, and the source is never asked to resolve that pseudonym back to
/// another domain's identifier (GDPR Art. 4(5)).
#[tokio::test]
async fn the_subjects_of_an_ehr_are_its_pseudonym_alone() {
    let pix = MockServer::start().await;
    let pseudonym = Uuid::now_v7().to_string();
    Mock::given(method("POST"))
        .and(path(OPERATION))
        .respond_with(answer(
            200,
            &ig("example/Parameters-pixm-response-mohralice-red-all.json"),
        ))
        .expect(0)
        .mount(&pix)
        .await;
    let db = testkit::db().await.expect("testkit database");
    let local = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_ehr_index(EhrIndexProvider::Local);
    let ehr = local
        .create_ehr_for_subject(SubjectRef::person(pseudonym.clone(), CLINICAL), None)
        .await
        .expect("an EHR under the clinical pseudonym");

    let svc = service(&db, &pix.uri());
    let subjects = svc
        .ehr_subjects(ehr.to_string())
        .await
        .expect("the subjects");
    let named: Vec<(String, String)> = subjects
        .into_iter()
        .map(|entry| (entry.subject.namespace, entry.subject.id))
        .collect();
    assert_eq!(named, vec![(CLINICAL.to_owned(), pseudonym)]);
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
