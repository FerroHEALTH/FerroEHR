// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operator's path for a refused ITI-93 message (#5725): the list names
//! the refused messages by ids and counts, a retry applies a message again
//! once its cause is fixed and is refused when a later message superseded
//! it, a settlement discards a message keeping its splits or lifts them by a
//! recorded decision, and every act is an access record naming the operator
//! and the message. PMIR leaves the Consumer's handling to its capabilities
//! (PMIR 1.6.0 §2:3.93.4.1.3): the rules are our own design.

use std::time::Duration;

use ihe_iti::pmir::server::notification::Notice;
use uuid::Uuid;

use crate::patient_index::NATIONAL;
use crate::pmir_refusals::{
    OTHER, apply, consumer, fhir_patient, national, patient_with_ehr, receive, refused, trc,
};
use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::identity_feed::Refusal;
use ferroehr::service::demographic::identity_feed::apply::Applied;
use ferroehr::service::demographic::identity_feed::config::IdentifierDomain;
use ferroehr::service::demographic::identity_feed::operator::{Decision, RefusedMessage};
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The operator every act runs as.
const OPERATOR: &str = "steward";
/// The operating-system account the CLI ran the acts under.
const ACCOUNT: &str = "os:steward/1001";

fn operator() -> CommitterIdentity {
    CommitterIdentity {
        subject: OPERATOR.to_owned(),
        id_type: "operator",
        issuer: None,
        client: Some(ACCOUNT.to_owned()),
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        agent: None,
        external: None,
    }
}

async fn list(svc: &FerroEhrService) -> Vec<RefusedMessage> {
    with_committer(Some(operator()), svc.pmir_refused())
        .await
        .expect("the list reads")
        .messages
}

async fn retry(svc: &FerroEhrService, id: &str) -> Result<(), ServiceError> {
    with_committer(Some(operator()), svc.pmir_retry(id))
        .await
        .map(|retried| assert_eq!(retried.state, "received", "a retry queues the message"))
}

async fn settle(
    svc: &FerroEhrService,
    id: &str,
    decision: Decision,
    reason: &str,
) -> Result<u64, ServiceError> {
    with_committer(Some(operator()), svc.pmir_settle(id, decision, reason))
        .await
        .map(|settled| settled.lifted)
}

/// One `PERSON` bound to the identity `pix` through the national identifier
/// `NAT-B`, with an EHR, and the message `link` that links the identifier
/// `OTHER-1` of a domain with no type to it, refused.
async fn refused_link(svc: &FerroEhrService, pix: &str) -> EhrId {
    let ehr = patient_with_ehr(svc, "B", &["NAT-B"]).await;
    receive(
        svc,
        "bind",
        &[Notice::Updated(Box::new(national(pix, &["NAT-B"])))],
    )
    .await;
    assert!(
        matches!(apply(svc).await, Some(Applied::Applied(_))),
        "the identity binds"
    );
    receive(svc, "link", &[link(pix)]).await;
    assert_eq!(
        apply(svc).await,
        Some(refused("link", Refusal::UnknownDomainType)),
        "a link of a domain with no type is refused"
    );
    ehr
}

fn link(pix: &str) -> Notice {
    Notice::Updated(Box::new(fhir_patient(
        pix,
        &[(NATIONAL, "NAT-B"), (OTHER, "OTHER-1")],
    )))
}

/// Two bound identities, and the message `contested` about the second one
/// that drops its own identifier `NAT-B`, split at receipt, and takes `NAT-A`
/// from the first, refused as contested.
async fn refused_with_split(svc: &FerroEhrService) -> (EhrId, EhrId) {
    let a = patient_with_ehr(svc, "A", &["NAT-A"]).await;
    let b = patient_with_ehr(svc, "B", &["NAT-B"]).await;
    let (pix_a, pix_b) = (Uuid::now_v7().to_string(), Uuid::now_v7().to_string());
    receive(
        svc,
        "bind",
        &[
            Notice::Updated(Box::new(national(&pix_a, &["NAT-A"]))),
            Notice::Updated(Box::new(national(&pix_b, &["NAT-B"]))),
        ],
    )
    .await;
    assert!(
        matches!(apply(svc).await, Some(Applied::Applied(_))),
        "both identities bind"
    );
    receive(
        svc,
        "contested",
        &[Notice::Updated(Box::new(national(&pix_b, &["NAT-A"])))],
    )
    .await;
    assert_eq!(
        apply(svc).await,
        Some(refused("contested", Refusal::Contested)),
        "NAT-A is still held by the first identity"
    );
    (a, b)
}

#[tokio::test]
async fn a_refused_link_is_listed_and_applies_when_retried_after_its_domain_is_named() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, true).await;
    let pix = Uuid::now_v7().to_string();
    let ehr = refused_link(&svc, &pix).await;

    let listed = list(&svc).await;
    assert_eq!(listed.len(), 1, "{listed:?}");
    let message = &listed[0];
    assert_eq!(message.message_id, "link");
    assert_eq!(message.reason, Refusal::UnknownDomainType.code());
    assert_eq!(message.attempts, 1);
    assert!(message.refused_at.is_some());
    assert_eq!(message.identities, std::slice::from_ref(&pix));
    assert_eq!(message.splits, [], "a link splits nothing");
    let wire = serde_json::to_string(&listed).expect("the list writes");
    assert!(
        !wire.contains("OTHER-1") && !wire.contains("NAT-B"),
        "the list carries no identifier value: {wire}"
    );

    let named = consumer(&db, false)
        .await
        .with_pmir_identifier_domains(&[IdentifierDomain {
            system: OTHER.to_owned(),
            identifier_type: "MR".to_owned(),
        }]);
    retry(&named, "link").await.expect("the retry is taken");
    assert_eq!(
        apply(&named).await,
        Some(Applied::Applied("link".to_owned())),
        "the retried message applies under the domain now named"
    );
    assert_eq!(trc(&named, OTHER, "OTHER-1").await, vec![ehr]);
    assert_eq!(list(&named).await, [], "nothing is left to settle");
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM party.pmir_message WHERE message_id = 'link'")
            .fetch_one(&db.pool())
            .await
            .expect("the row reads");
    assert_eq!(attempts, 2);
}

#[tokio::test]
async fn a_retry_is_refused_once_a_later_message_about_the_identity_applied() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, true).await;
    let pix = Uuid::now_v7().to_string();
    refused_link(&svc, &pix).await;
    receive(
        &svc,
        "later",
        &[Notice::Updated(Box::new(national(&pix, &["NAT-B"])))],
    )
    .await;
    assert!(
        matches!(apply(&svc).await, Some(Applied::Applied(_))),
        "the later message applies"
    );

    let refusal = retry(&svc, "link").await.expect_err("a superseded retry");
    assert!(matches!(refusal, ServiceError::Conflict(_)), "{refusal:?}");
    assert_eq!(list(&svc).await.len(), 1, "the message stays refused");
}

#[tokio::test]
async fn a_retry_needs_a_refused_message_and_an_operator() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, true).await;
    let pix = Uuid::now_v7().to_string();
    refused_link(&svc, &pix).await;

    let unknown = retry(&svc, "no-such")
        .await
        .expect_err("an unknown message");
    assert!(matches!(unknown, ServiceError::NotFound(_)), "{unknown:?}");
    let applied = retry(&svc, "bind").await.expect_err("an applied message");
    assert!(matches!(applied, ServiceError::Conflict(_)), "{applied:?}");
    let anonymous = svc.pmir_retry("link").await.expect_err("no operator");
    assert!(
        matches!(anonymous, ServiceError::BadRequest(_)),
        "{anonymous:?}"
    );
    let unreasoned = settle(&svc, "link", Decision::Discard, "  ")
        .await
        .expect_err("no reason");
    assert!(
        matches!(unreasoned, ServiceError::BadRequest(_)),
        "{unreasoned:?}"
    );
}

#[tokio::test]
async fn a_discard_keeps_the_splits_until_a_decision_lifts_them() {
    let db = testkit::db().await.expect("testkit database");
    let svc = consumer(&db, true).await;
    let (a, b) = refused_with_split(&svc).await;
    assert_eq!(
        trc(&svc, NATIONAL, "NAT-B").await,
        Vec::<EhrId>::new(),
        "the dropped identifier is split from receipt"
    );
    let listed = list(&svc).await;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].splits.len(), 1, "{listed:?}");
    assert_eq!(listed[0].splits[0].identifiers, 1);

    assert_eq!(
        settle(
            &svc,
            "contested",
            Decision::Discard,
            "the Registry is corrected"
        )
        .await
        .expect("the discard commits"),
        0
    );
    assert_eq!(
        list(&svc).await,
        [],
        "a settled message is no longer listed"
    );
    assert_eq!(
        trc(&svc, NATIONAL, "NAT-B").await,
        Vec::<EhrId>::new(),
        "a discard keeps the split"
    );
    let row: (String, Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT state, decision, decided_by, events IS NULL FROM party.pmir_message \
         WHERE message_id = 'contested'",
    )
    .fetch_one(&db.pool())
    .await
    .expect("the row reads");
    assert_eq!(
        row,
        (
            "settled".to_owned(),
            Some("discard".to_owned()),
            Some(OPERATOR.to_owned()),
            true
        ),
        "the settlement is recorded and the identifiers are cleared"
    );
    let again = settle(&svc, "contested", Decision::Discard, "twice")
        .await
        .expect_err("a discarded message is not discarded again");
    assert!(matches!(again, ServiceError::Conflict(_)), "{again:?}");

    assert_eq!(
        settle(&svc, "contested", Decision::LiftSplits, "B keeps NAT-B")
            .await
            .expect("the lift commits"),
        1
    );
    assert_eq!(
        trc(&svc, NATIONAL, "NAT-B").await,
        vec![b],
        "a lift restores the resolution before the message"
    );
    assert_eq!(trc(&svc, NATIONAL, "NAT-A").await, vec![a]);
}

#[tokio::test]
async fn every_list_retry_and_settlement_is_an_access_record_naming_the_operator() {
    let db = testkit::db().await.expect("testkit database");
    let audit = ferroehr::system_log::config::AuditConfig {
        enabled: true,
        ..Default::default()
    };
    let (sender, handle) = ferroehr::system_log::sender::start(audit, None, db.pool())
        .await
        .expect("the audit sender");
    let svc = consumer(&db, true).await.with_audit(sender);
    let pix = Uuid::now_v7().to_string();
    refused_link(&svc, &pix).await;

    list(&svc).await;
    retry(&svc, "link").await.expect("the retry is taken");
    assert_eq!(
        apply(&svc).await,
        Some(refused("link", Refusal::UnknownDomainType))
    );
    settle(&svc, "link", Decision::Discard, "the domain is not ours")
        .await
        .expect("the settlement commits");

    drop(svc);
    handle.shutdown(Duration::from_secs(5)).await;
    let recorded: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT action, principal, client_id FROM audit.access_trail \
         WHERE resource_id = 'pmir-message:link' ORDER BY chain_seq",
    )
    .fetch_all(&db.pool())
    .await
    .expect("the trail reads");
    let (operator, account) = (Some(OPERATOR.to_owned()), Some(ACCOUNT.to_owned()));
    assert_eq!(
        recorded,
        [
            ("R".to_owned(), operator.clone(), account.clone()),
            ("U".to_owned(), operator.clone(), account.clone()),
            ("U".to_owned(), operator, account),
        ],
        "the list, the retry and the settlement, each naming the operator and the account"
    );
}
