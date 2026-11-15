// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The PMIR Patient Identity Consumer on two replicas (#5693): a merge, an
//! unmerge, an unlink and a link the Registry announces with ITI-93
//! (PMIR 1.6.0 §2:3.93), applied to the patient `PERSON`s as demographic
//! content, with what a split stops resolving held from receipt.
//!
//! The merge arrives as the bytes a Supplier sends, written by the
//! Registry's own notification writer and read by the consumer's reader. The
//! revert arrives as the stored changes the reader yields once it states
//! `active` and the links of every `Patient` (`FerroPIX` #656), through the
//! store the reader feeds. PMIR leaves the Consumer's handling to its
//! capabilities (§2:3.93.4.1.3); what is asserted is our own design.

use fhir_types::r4::identifier::Identifier as FhirIdentifier;
use fhir_types::r4::patient::{Patient, PatientLink};
use fhir_types::r4::reference::Reference;
use ihe_iti::pmir::server::notification::{self, Addressing, Notice};
use url::Url;
use uuid::Uuid;

use crate::ehr_index_ihe::CLINICAL;
use crate::fixtures::create_ehr_subject;
use crate::patient_index::{NATIONAL, identifier, patient, replica, sorted};
use crate::policy_fixtures::{ISSUER, capability, create, granted};
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::identity_feed::apply::Applied;
use ferroehr::service::demographic::identity_feed::{
    ChangeKind, FeedEvent, FeedSender, Identifier, Refusal,
};
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr::service::ehr_index::types::SubjectRef;

/// The Registry's FHIR base.
const REGISTRY: &str = "https://pix.example/fhir";
/// The consumer's feed route.
const FEED: &str = "https://cdr.example/pmir/feed";
/// The Registry's client.
const CLIENT: &str = "ferropix";

fn sender() -> FeedSender {
    FeedSender {
        issuer: ISSUER.to_owned(),
        client_id: CLIENT.to_owned(),
    }
}

/// The FHIR `Patient` `id` carrying the national identifiers `national`.
fn fhir_patient(id: &str, national: &[&str]) -> Patient {
    Patient {
        id: Some(id.to_owned()),
        identifier: national
            .iter()
            .map(|value| FhirIdentifier {
                system: Some(NATIONAL.into()),
                value: Some((*value).into()),
                ..FhirIdentifier::default()
            })
            .collect(),
        active: Some(true.into()),
        ..Patient::default()
    }
}

/// `merged` deprecated into `survivor`, as the merged Patient profile has it.
fn merged_patient(merged: &str, survivor: &str) -> Patient {
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
        ..fhir_patient(merged, &[])
    }
}

/// The ITI-93 message `id` reporting `notices`, as FHIR JSON.
fn message(id: &str, notices: &[Notice]) -> Vec<u8> {
    let registry = Url::parse(REGISTRY).expect("a URL");
    let destination = Url::parse(FEED).expect("a URL");
    let history = Uuid::now_v7().to_string();
    let bundle = notification::message(
        notices,
        &Addressing {
            message_id: id,
            history_id: &history,
            registry: &registry,
            destination: &destination,
        },
    );
    serde_json::to_vec(&bundle).expect("the message writes")
}

fn national(values: &[&str]) -> Vec<Identifier> {
    values
        .iter()
        .map(|v| Identifier::new(NATIONAL, v))
        .collect()
}

/// Two patients A and B, each carrying its national identifier and
/// declaring its pseudonym, with an EHR on it; and the Registry's identities
/// of them.
struct Patients {
    pseudonyms: [String; 2],
    ehrs: [EhrId; 2],
    pix: [String; 2],
}

impl Patients {
    async fn on(svc: &FerroEhrService) -> Self {
        let pseudonyms = [Uuid::now_v7().to_string(), Uuid::now_v7().to_string()];
        for (name, nat, pseudonym) in [
            ("A", "NAT-A", &pseudonyms[0]),
            ("B", "NAT-B", &pseudonyms[1]),
        ] {
            create(
                svc,
                PartyKind::Person,
                &patient(
                    name,
                    &[identifier(NATIONAL, nat, "NI")],
                    &[identifier(CLINICAL, pseudonym, "PI")],
                ),
            )
            .await
            .expect("the patient commits");
        }
        let ehrs = [
            create_ehr_subject(svc, &SubjectRef::person(&pseudonyms[0], CLINICAL)).await,
            create_ehr_subject(svc, &SubjectRef::person(&pseudonyms[1], CLINICAL)).await,
        ];
        Self {
            pseudonyms,
            ehrs,
            pix: [Uuid::now_v7().to_string(), Uuid::now_v7().to_string()],
        }
    }

    /// The Registry announcing both identities as they stand, which binds
    /// each to its `PERSON`, applied on `svc`.
    async fn announced(&self, svc: &FerroEhrService) {
        let announcement = message(
            &Uuid::now_v7().to_string(),
            &[
                Notice::Updated(Box::new(fhir_patient(&self.pix[0], &["NAT-A"]))),
                Notice::Updated(Box::new(fhir_patient(&self.pix[1], &["NAT-B"]))),
            ],
        );
        svc.pmir_receive(
            &sender(),
            Some("application/fhir+json"),
            &announcement,
            FEED,
        )
        .await
        .expect("the announcement is stored");
        assert!(
            matches!(
                svc.pmir_apply_next(Uuid::now_v7())
                    .await
                    .expect("the apply runs"),
                Some(Applied::Applied(_))
            ),
            "the announcement applies"
        );
    }

    fn subject(&self, at: usize) -> SubjectRef {
        SubjectRef::person(&self.pseudonyms[at], CLINICAL)
    }

    /// The merge of A into B as the Registry sends it.
    fn merge(&self) -> Vec<u8> {
        message(
            &Uuid::now_v7().to_string(),
            &[
                Notice::Merged(Box::new(merged_patient(&self.pix[0], &self.pix[1]))),
                Notice::Updated(Box::new(fhir_patient(&self.pix[1], &["NAT-A", "NAT-B"]))),
            ],
        )
    }

    /// The steward's revert of that merge, as the reader stores it once it
    /// states `active` and the links (`FerroPIX` #656).
    fn revert(&self) -> Vec<FeedEvent> {
        vec![
            FeedEvent::new(
                ChangeKind::Update,
                &self.pix[1],
                national(&["NAT-B"]),
                Some(true),
                None,
            ),
            FeedEvent::new(
                ChangeKind::Update,
                &self.pix[0],
                national(&["NAT-A"]),
                Some(true),
                None,
            ),
        ]
    }
}

/// Two replicas over one database, the Registry's client granted the care
/// relationship writes a merge needs.
async fn replicas(db: &testkit::TestDb) -> (FerroEhrService, FerroEhrService) {
    let one = replica(db, EhrIndexProvider::Local).await;
    let two = replica(db, EhrIndexProvider::Local).await;
    granted(&one, CLIENT, &[capability("write_care_relationship", &[])]).await;
    (one, two)
}

/// What `svc` answers for a TRC patient `id` of the national domain.
async fn trc(svc: &FerroEhrService, id: &str) -> Vec<EhrId> {
    svc.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    sorted(
        svc.patient_ehrs(&SubjectRef::person(id, NATIONAL))
            .await
            .expect("the TRC resolves"),
    )
}

/// What `svc`'s index answers for `subject`.
async fn index(svc: &FerroEhrService, subject: &SubjectRef) -> Vec<EhrId> {
    svc.grants()
        .refresh_if_stale()
        .await
        .expect("the copy reads");
    sorted(svc.subject_ehrs(subject).await.expect("the index answers"))
}

#[tokio::test]
async fn a_merge_and_its_revert_apply_on_both_replicas_and_the_split_holds_from_receipt() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = replicas(&db).await;
    let patients = Patients::on(&one).await;
    patients.announced(&two).await;
    let both = sorted(patients.ehrs.to_vec());
    assert_eq!(trc(&two, "NAT-A").await, vec![patients.ehrs[0]]);

    let received = one
        .pmir_receive(
            &sender(),
            Some("application/fhir+json"),
            &patients.merge(),
            FEED,
        )
        .await
        .expect("the merge is stored");
    assert!(received.stored);
    assert!(
        String::from_utf8_lossy(&received.acknowledgement).contains("\"ok\""),
        "the response reports ok"
    );
    assert_eq!(
        trc(&two, "NAT-A").await,
        vec![patients.ehrs[0]],
        "a merge widens nothing before it is applied"
    );
    let applied = two
        .pmir_apply_next(Uuid::now_v7())
        .await
        .expect("the apply runs");
    assert!(matches!(applied, Some(Applied::Applied(_))), "{applied:?}");
    assert_eq!(
        trc(&one, "NAT-A").await,
        both,
        "after the merge A's identifier reaches both"
    );
    assert_eq!(trc(&one, "NAT-B").await, both);
    assert_eq!(index(&one, &patients.subject(1)).await, both);

    assert!(
        two.pmir_store(&sender(), "revert-1", &patients.revert())
            .await
            .expect("the revert is stored")
    );
    assert_eq!(
        trc(&one, "NAT-A").await,
        Vec::<EhrId>::new(),
        "from receipt the split identifier resolves on no replica"
    );
    assert_eq!(
        index(&one, &patients.subject(1)).await,
        vec![patients.ehrs[1]],
        "from receipt the survivor no longer reaches the merged patient's EHR"
    );
    assert_eq!(trc(&two, "NAT-B").await, vec![patients.ehrs[1]]);

    let applied = one
        .pmir_apply_next(Uuid::now_v7())
        .await
        .expect("the apply runs");
    assert!(matches!(applied, Some(Applied::Applied(_))), "{applied:?}");
    assert_eq!(trc(&two, "NAT-A").await, vec![patients.ehrs[0]]);
    assert_eq!(trc(&two, "NAT-B").await, vec![patients.ehrs[1]]);
    assert_eq!(
        index(&two, &patients.subject(0)).await,
        vec![patients.ehrs[0]]
    );
    assert_eq!(
        index(&two, &patients.subject(1)).await,
        vec![patients.ehrs[1]]
    );
}

#[tokio::test]
async fn a_redelivered_or_late_message_changes_nothing_a_second_time() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = replicas(&db).await;
    let patients = Patients::on(&one).await;
    patients.announced(&two).await;
    let merge = patients.merge();
    for svc in [&one, &two] {
        svc.pmir_receive(&sender(), Some("application/fhir+json"), &merge, FEED)
            .await
            .expect("the merge is stored");
    }
    assert!(
        matches!(
            one.pmir_apply_next(Uuid::now_v7())
                .await
                .expect("the apply runs"),
            Some(Applied::Applied(_))
        ),
        "the message applies"
    );
    assert_eq!(
        two.pmir_apply_next(Uuid::now_v7())
            .await
            .expect("the apply runs"),
        None,
        "a redelivery was stored once and applies once"
    );
    two.pmir_store(&sender(), "revert-2", &patients.revert())
        .await
        .expect("the revert is stored");
    assert!(
        matches!(
            two.pmir_apply_next(Uuid::now_v7())
                .await
                .expect("the apply runs"),
            Some(Applied::Applied(_))
        ),
        "the message applies"
    );
    let late = one
        .pmir_receive(&sender(), Some("application/fhir+json"), &merge, FEED)
        .await
        .expect("the late merge is answered");
    assert!(
        !late.stored,
        "a message arriving again after a later one is not stored again"
    );
    assert_eq!(
        one.pmir_apply_next(Uuid::now_v7())
            .await
            .expect("the apply runs"),
        None
    );
    assert_eq!(trc(&two, "NAT-A").await, vec![patients.ehrs[0]]);
    assert_eq!(
        index(&two, &patients.subject(1)).await,
        vec![patients.ehrs[1]]
    );
}

#[tokio::test]
async fn an_unlink_stops_resolving_at_receipt_and_a_link_adds_the_identifier() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = replicas(&db).await;
    let patients = Patients::on(&one).await;
    patients.announced(&two).await;
    let link = message(
        "link-1",
        &[Notice::Updated(Box::new(fhir_patient(
            &patients.pix[1],
            &["NAT-B", "NAT-B2"],
        )))],
    );
    one.pmir_receive(&sender(), Some("application/fhir+json"), &link, FEED)
        .await
        .expect("the link is stored");
    one.pmir_apply_next(Uuid::now_v7())
        .await
        .expect("the apply runs");
    assert_eq!(
        trc(&two, "NAT-B2").await,
        vec![patients.ehrs[1]],
        "the linked identifier resolves"
    );

    let unlink = message(
        "unlink-1",
        &[Notice::Updated(Box::new(fhir_patient(
            &patients.pix[1],
            &["NAT-B"],
        )))],
    );
    two.pmir_receive(&sender(), Some("application/fhir+json"), &unlink, FEED)
        .await
        .expect("the unlink is stored");
    assert_eq!(
        trc(&one, "NAT-B2").await,
        Vec::<EhrId>::new(),
        "the unlinked identifier resolves on no replica from receipt"
    );
    assert_eq!(trc(&one, "NAT-B").await, vec![patients.ehrs[1]]);
    one.pmir_apply_next(Uuid::now_v7())
        .await
        .expect("the apply runs");
    assert_eq!(trc(&two, "NAT-B2").await, Vec::<EhrId>::new());
    assert_eq!(trc(&two, "NAT-B").await, vec![patients.ehrs[1]]);
}

#[tokio::test]
async fn a_message_about_a_merged_identity_that_states_no_state_is_refused_and_stays_split() {
    let db = testkit::db().await.expect("testkit database");
    let (one, two) = replicas(&db).await;
    let patients = Patients::on(&one).await;
    patients.announced(&two).await;
    one.pmir_receive(
        &sender(),
        Some("application/fhir+json"),
        &patients.merge(),
        FEED,
    )
    .await
    .expect("the merge is stored");
    one.pmir_apply_next(Uuid::now_v7())
        .await
        .expect("the apply runs");
    // NOTE: the reader of ihe-iti 0.9.0 keeps no `active` or link of an update, so this is read
    // as stating neither, the case the consumer refuses rather than guesses.
    let unstated = message(
        "unstated-1",
        &[
            Notice::Updated(Box::new(fhir_patient(&patients.pix[1], &["NAT-B"]))),
            Notice::Updated(Box::new(fhir_patient(&patients.pix[0], &["NAT-A"]))),
        ],
    );
    two.pmir_receive(&sender(), Some("application/fhir+json"), &unstated, FEED)
        .await
        .expect("the message is stored");
    let applied = two
        .pmir_apply_next(Uuid::now_v7())
        .await
        .expect("the apply runs");
    assert_eq!(
        applied,
        Some(Applied::Refused(
            "unstated-1".to_owned(),
            Refusal::MergedStateMissing
        ))
    );
    assert_eq!(
        index(&one, &patients.subject(1)).await,
        vec![patients.ehrs[1]],
        "the record subject a refused unmerge would split stays unresolved"
    );
    assert_eq!(trc(&one, "NAT-A").await, Vec::<EhrId>::new());
}

#[tokio::test]
async fn a_message_that_departs_from_the_profiles_is_refused_and_stores_nothing() {
    let db = testkit::db().await.expect("testkit database");
    let (one, _two) = replicas(&db).await;
    let refused = one
        .pmir_receive(
            &sender(),
            Some("application/fhir+json"),
            b"{\"resourceType\":\"Patient\"}",
            FEED,
        )
        .await;
    assert!(refused.is_err(), "a Patient is no message");
    assert_eq!(
        one.pmir_apply_next(Uuid::now_v7())
            .await
            .expect("the apply runs"),
        None
    );
}
