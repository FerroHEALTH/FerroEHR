// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the PMIR Patient Identity Consumer refuses rather than guesses, and
//! how one replica's lease on a message is taken over (#5693): every refused
//! message is stored, keeps the splits it made at receipt and changes no
//! `PERSON`. The rules are our own design (PMIR 1.6.0 §2:3.93.4.1.3).

use fhir_types::r4::identifier::Identifier as FhirIdentifier;
use fhir_types::r4::patient::{Patient, PatientLink};
use fhir_types::r4::reference::Reference;
use ihe_iti::pmir::server::notification::{self, Addressing, Notice};
use url::Url;
use uuid::Uuid;

use crate::ehr_index_ihe::CLINICAL;
use crate::fixtures::create_ehr_subject;
use crate::patient_index::{NATIONAL, identifier, patient, replica, sorted};
use crate::pmir_consumer::MasterIdentifier;
use crate::policy_fixtures::{ISSUER, capability, create, granted};
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::identity_feed::apply::Applied;
use ferroehr::service::demographic::identity_feed::config::IdentifierDomain;
use ferroehr::service::demographic::identity_feed::{FeedSender, Refusal};
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr::service::ehr_index::types::SubjectRef;

const FEED: &str = "https://cdr.example/pmir/feed";
const CLIENT: &str = "ferropix";
/// A domain no patient `PERSON` carries an identifier of.
const OTHER: &str = "urn:oid:2.999.9.9";

fn sender() -> FeedSender {
    FeedSender {
        issuer: ISSUER.to_owned(),
        client_id: CLIENT.to_owned(),
    }
}

fn fhir_patient(id: &str, identifiers: &[(&str, &str)]) -> Patient {
    Patient {
        id: Some(id.to_owned()),
        identifier: identifiers
            .iter()
            .map(|(system, value)| FhirIdentifier {
                system: Some((*system).into()),
                value: Some((*value).into()),
                ..FhirIdentifier::default()
            })
            .collect(),
        active: Some(true.into()),
        ..Patient::default()
    }
    .with_master_identifier(id)
}

fn national(id: &str, values: &[&str]) -> Patient {
    let identifiers: Vec<(&str, &str)> = values.iter().map(|v| (NATIONAL, *v)).collect();
    fhir_patient(id, &identifiers)
}

fn merged(id: &str, survivor: &str) -> Patient {
    Patient {
        active: Some(false.into()),
        link: vec![PatientLink {
            other: Reference {
                reference: Some(format!("Patient/{survivor}").as_str().into()),
                ..Reference::default()
            },
            r#type: "replaced-by".into(),
            ..PatientLink::default()
        }],
        ..fhir_patient(id, &[])
    }
}

fn message(id: &str, notices: &[Notice]) -> Vec<u8> {
    let registry = Url::parse("https://pix.example/fhir").expect("a URL");
    let destination = Url::parse(FEED).expect("a URL");
    let history = Uuid::now_v7().to_string();
    serde_json::to_vec(&notification::message(
        notices,
        &Addressing {
            message_id: id,
            history_id: &history,
            registry: &registry,
            destination: &destination,
        },
    ))
    .expect("the message writes")
}

async fn receive(svc: &FerroEhrService, id: &str, notices: &[Notice]) {
    svc.pmir_receive(
        &sender(),
        Some("application/fhir+json"),
        &message(id, notices),
        FEED,
    )
    .await
    .expect("the message is stored");
}

async fn apply(svc: &FerroEhrService) -> Option<Applied> {
    Box::pin(svc.pmir_apply_next(Uuid::now_v7()))
        .await
        .expect("the apply runs")
}

/// A patient `PERSON` carrying the national identifiers `values` and
/// declaring a pseudonym of its own, with an EHR on it.
async fn patient_with_ehr(svc: &FerroEhrService, name: &str, values: &[&str]) -> EhrId {
    let pseudonym = Uuid::now_v7().to_string();
    let identifiers: Vec<_> = values
        .iter()
        .map(|value| identifier(NATIONAL, value, "NI"))
        .collect();
    create(
        svc,
        PartyKind::Person,
        &patient(
            name,
            &identifiers,
            &[identifier(CLINICAL, &pseudonym, "PI")],
        ),
    )
    .await
    .expect("the patient commits");
    create_ehr_subject(svc, &SubjectRef::person(&pseudonym, CLINICAL)).await
}

async fn trc(svc: &FerroEhrService, system: &str, id: &str) -> Vec<EhrId> {
    svc.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    sorted(
        svc.patient_ehrs(&SubjectRef::person(id, system))
            .await
            .expect("the TRC resolves"),
    )
}

/// One replica over `db`, the Registry's client granted the care
/// relationship writes when `grant`.
async fn consumer(db: &testkit::TestDb, grant: bool) -> FerroEhrService {
    let svc = replica(db, EhrIndexProvider::Local).await;
    if grant {
        granted(&svc, CLIENT, &[capability("write_care_relationship", &[])]).await;
    }
    svc
}

fn refused(id: &str, refusal: Refusal) -> Applied {
    Applied::Refused(id.to_owned(), refusal)
}

#[tokio::test]
async fn a_link_of_a_domain_with_no_type_is_refused_until_the_domain_is_named() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, true).await;
    let ehr = patient_with_ehr(&svc, "B", &["NAT-B"]).await;
    let pix = Uuid::now_v7().to_string();
    receive(
        &svc,
        "bind",
        &[Notice::Updated(Box::new(national(&pix, &["NAT-B"])))],
    )
    .await;
    assert!(
        matches!(apply(&svc).await, Some(Applied::Applied(_))),
        "the identity binds"
    );
    let link = |id: &str| {
        message(
            id,
            &[Notice::Updated(Box::new(fhir_patient(
                &pix,
                &[(NATIONAL, "NAT-B"), (OTHER, "OTHER-1")],
            )))],
        )
    };
    svc.pmir_receive(
        &sender(),
        Some("application/fhir+json"),
        &link("link-1"),
        FEED,
    )
    .await
    .expect("the link is stored");
    assert_eq!(
        apply(&svc).await,
        Some(refused("link-1", Refusal::UnknownDomainType))
    );
    assert_eq!(trc(&svc, OTHER, "OTHER-1").await, Vec::<EhrId>::new());
    assert_eq!(
        trc(&svc, NATIONAL, "NAT-B").await,
        vec![ehr],
        "the refusal changed nothing"
    );

    let named = consumer(&db, false)
        .await
        .with_pmir_identifier_domains(&[IdentifierDomain {
            system: OTHER.to_owned(),
            identifier_type: "MR".to_owned(),
        }]);
    named
        .pmir_receive(
            &sender(),
            Some("application/fhir+json"),
            &link("link-2"),
            FEED,
        )
        .await
        .expect("the link is stored");
    assert!(
        matches!(apply(&named).await, Some(Applied::Applied(_))),
        "a named domain links"
    );
    assert_eq!(trc(&named, OTHER, "OTHER-1").await, vec![ehr]);
}

#[tokio::test]
async fn an_identity_two_persons_carry_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, true).await;
    let first = patient_with_ehr(&svc, "A", &["NAT-A"]).await;
    let second = patient_with_ehr(&svc, "A2", &["NAT-A2"]).await;
    let pix = Uuid::now_v7().to_string();
    receive(
        &svc,
        "ambiguous",
        &[Notice::Updated(Box::new(national(
            &pix,
            &["NAT-A", "NAT-A2"],
        )))],
    )
    .await;
    assert_eq!(
        apply(&svc).await,
        Some(refused("ambiguous", Refusal::Ambiguous))
    );
    assert_eq!(trc(&svc, NATIONAL, "NAT-A").await, vec![first]);
    assert_eq!(trc(&svc, NATIONAL, "NAT-A2").await, vec![second]);
}

#[tokio::test]
async fn an_identifier_another_identity_still_holds_is_contested() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, true).await;
    let a = patient_with_ehr(&svc, "A", &["NAT-A"]).await;
    patient_with_ehr(&svc, "B", &["NAT-B"]).await;
    let (pix_a, pix_b) = (Uuid::now_v7().to_string(), Uuid::now_v7().to_string());
    receive(
        &svc,
        "bind",
        &[
            Notice::Updated(Box::new(national(&pix_a, &["NAT-A"]))),
            Notice::Updated(Box::new(national(&pix_b, &["NAT-B"]))),
        ],
    )
    .await;
    assert!(
        matches!(apply(&svc).await, Some(Applied::Applied(_))),
        "both identities bind"
    );
    receive(
        &svc,
        "contested",
        &[Notice::Updated(Box::new(national(
            &pix_b,
            &["NAT-B", "NAT-A"],
        )))],
    )
    .await;
    assert_eq!(
        apply(&svc).await,
        Some(refused("contested", Refusal::Contested))
    );
    assert_eq!(
        trc(&svc, NATIONAL, "NAT-A").await,
        vec![a],
        "A keeps its identifier"
    );
}

#[tokio::test]
async fn a_merge_into_a_merged_identity_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, true).await;
    patient_with_ehr(&svc, "A", &["NAT-A"]).await;
    patient_with_ehr(&svc, "B", &["NAT-B"]).await;
    let (pix_a, pix_b) = (Uuid::now_v7().to_string(), Uuid::now_v7().to_string());
    receive(
        &svc,
        "bind",
        &[
            Notice::Updated(Box::new(national(&pix_a, &["NAT-A"]))),
            Notice::Updated(Box::new(national(&pix_b, &["NAT-B"]))),
        ],
    )
    .await;
    apply(&svc).await;
    receive(
        &svc,
        "merge",
        &[
            Notice::Merged(Box::new(merged(&pix_a, &pix_b))),
            Notice::Updated(Box::new(national(&pix_b, &["NAT-A", "NAT-B"]))),
        ],
    )
    .await;
    assert!(
        matches!(apply(&svc).await, Some(Applied::Applied(_))),
        "the merge applies"
    );
    receive(
        &svc,
        "into-merged",
        &[Notice::Merged(Box::new(merged(&pix_b, &pix_a)))],
    )
    .await;
    assert_eq!(
        apply(&svc).await,
        Some(refused("into-merged", Refusal::Survivor))
    );
}

#[tokio::test]
async fn a_merge_the_client_may_not_write_is_refused_by_the_commit_checks() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, false).await;
    patient_with_ehr(&svc, "A", &["NAT-A"]).await;
    let b = patient_with_ehr(&svc, "B", &["NAT-B"]).await;
    let (pix_a, pix_b) = (Uuid::now_v7().to_string(), Uuid::now_v7().to_string());
    receive(
        &svc,
        "bind",
        &[
            Notice::Updated(Box::new(national(&pix_a, &["NAT-A"]))),
            Notice::Updated(Box::new(national(&pix_b, &["NAT-B"]))),
        ],
    )
    .await;
    apply(&svc).await;
    receive(
        &svc,
        "ungranted",
        &[
            Notice::Merged(Box::new(merged(&pix_a, &pix_b))),
            Notice::Updated(Box::new(national(&pix_b, &["NAT-A", "NAT-B"]))),
        ],
    )
    .await;
    assert_eq!(
        apply(&svc).await,
        Some(refused("ungranted", Refusal::Commit))
    );
    assert_eq!(
        trc(&svc, NATIONAL, "NAT-B").await,
        vec![b],
        "a refused contribution moves no record subject"
    );
}

#[tokio::test]
async fn a_held_lease_is_waited_for_and_a_stale_one_is_taken_over() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = (consumer(&db, true).await, consumer(&db, false).await);
    patient_with_ehr(&one, "B", &["NAT-B"]).await;
    let pix = Uuid::now_v7().to_string();
    receive(
        &one,
        "leased",
        &[Notice::Updated(Box::new(national(&pix, &["NAT-B"])))],
    )
    .await;
    // NOTE: a replica that claimed the message and then stopped, its lease still running.
    sqlx::query(
        "UPDATE party.pmir_message SET leased_by = $1, leased_until = now() + interval '1 minute' \
         WHERE message_id = 'leased'",
    )
    .bind(Uuid::now_v7())
    .execute(&db.pool())
    .await
    .expect("the lease is set");
    assert_eq!(
        apply(&two).await,
        None,
        "a lease another replica holds is waited for"
    );
    sqlx::query(
        "UPDATE party.pmir_message SET leased_until = now() - interval '1 second' \
         WHERE message_id = 'leased'",
    )
    .execute(&db.pool())
    .await
    .expect("the lease runs out");
    assert_eq!(
        apply(&two).await,
        Some(Applied::Applied("leased".to_owned())),
        "a lease that ran out is taken over"
    );
    assert_eq!(apply(&one).await, None, "the message applied once");
}

#[tokio::test]
async fn a_redelivery_of_a_refused_message_is_answered_and_stays_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, true).await;
    patient_with_ehr(&svc, "A", &["NAT-A"]).await;
    patient_with_ehr(&svc, "A2", &["NAT-A2"]).await;
    let pix = Uuid::now_v7().to_string();
    let bytes = message(
        "twice",
        &[Notice::Updated(Box::new(national(
            &pix,
            &["NAT-A", "NAT-A2"],
        )))],
    );
    svc.pmir_receive(&sender(), Some("application/fhir+json"), &bytes, FEED)
        .await
        .expect("the message is stored");
    assert_eq!(
        apply(&svc).await,
        Some(refused("twice", Refusal::Ambiguous))
    );
    let again = svc
        .pmir_receive(&sender(), Some("application/fhir+json"), &bytes, FEED)
        .await
        .expect("the redelivery is answered");
    assert!(!again.stored, "a redelivery is stored once");
    assert_eq!(
        apply(&svc).await,
        None,
        "a refused message is not applied again"
    );
}
