// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The copy of read COMPOSITION versions (#5778), against a real
//! `PostgreSQL` 18 (shared testkit harness): a repeated read by version id
//! reads the version's body from memory and makes one statement, the
//! indexed confirmation of its object's state; and a restriction, its lifting
//! and an erasure committed on one replica refuse or serve the next read on
//! another replica that hears no notification at all, since the decision
//! rests on that confirmation (owner decision on #5891). A version never
//! changes once committed (RM common master06 §Version Identification); the
//! copy and its confirmation are our own design.

use std::str::FromStr;
use std::sync::Arc;

use std::time::Duration;

use openehr_base::prelude::ObjectVersionId;
use openehr_rm::draft::access_control::terminology::{AccessDefault, CallerKind, OperationClass};
use sqlx::postgres::PgConnectOptions;
use tracing::Instrument as _;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

use crate::fixtures::{composition, create_ehr, uv};
use ferroehr::access_eval::condition::{
    Asking, InForce, RequestAccess, current, with_request_access,
};
use ferroehr::access_eval::procedure::{Caller, Emergency};
use ferroehr::access_eval::scope::CategoryIndex;
use ferroehr::db::domain::DomainPools;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::policy_register::listen::{Listened, Listeners};
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::ehr::composition_read::JudgedRead;

/// A replica over `db`, every policy copy listening on `listen`: the store
/// itself, or a database of its own, where the replica hears of no change.
async fn replica(db: &testkit::TestDb, listen: &testkit::TestDb) -> FerroEhrService {
    let svc = FerroEhrService::new(&DomainPools::from_shared(&db.pool()));
    let listened: Vec<Arc<Listened>> = svc
        .policy_listeners()
        .into_iter()
        .map(|(_, listened)| listened)
        .collect();
    Listeners::on(
        PgConnectOptions::from_str(listen.url()).expect("the test DSN parses"),
        &listened,
    )
    .spawn();
    for _ in 0..250 {
        if listened.iter().all(|listened| listened.is_listening()) {
            return svc;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the replica's listeners connect");
}

/// The request access of a professional under an open server default.
fn professional(svc: &FerroEhrService) -> Arc<RequestAccess> {
    let now = jiff::Timestamp::now();
    let party = VoId(uuid::Uuid::now_v7());
    Arc::new(RequestAccess::new(
        Caller {
            kind: CallerKind::Professional,
            party: Some(party.to_string()),
            own_ehrs: Vec::new(),
            patient_grants_only: false,
            acts_for: Vec::new(),
            facts: svc.grants().facts_of(party.0, now),
            administrator: false,
            settings: SettingsCaller::new(false, false, Some(party.to_string())),
            lacks_consent_reference: false,
        },
        Asking {
            operation: OperationClass::Read,
            purpose: Some("TREAT".to_owned()),
            emergency: Emergency::default(),
        },
        InForce {
            policy: None,
            categories: CategoryIndex::default(),
            server_default: AccessDefault::Open,
            now,
            grants: Some(Arc::clone(svc.grants())),
        },
    ))
}

/// Whether one request on `svc` reads the COMPOSITION `vo` of `ehr_id` at
/// `version`, and the body it served.
async fn read(
    svc: &FerroEhrService,
    ehr_id: EhrId,
    vo: VoId,
    version: &ObjectVersionId,
) -> Option<String> {
    let run = async {
        let access = current().expect("the request access");
        match Box::pin(svc.composition_judged(&access, ehr_id, vo, Some(version))).await {
            JudgedRead::Admitted(Ok(read)) => Some(format!("{:?}", read.body)),
            JudgedRead::Admitted(Err(_))
            | JudgedRead::Absent
            | JudgedRead::Refused { .. }
            | JudgedRead::Unheld(_) => None,
            JudgedRead::Unjudged(error) => panic!("the read is judged: {error}"),
        }
    };
    Box::pin(with_request_access(professional(svc), run)).await
}

/// The span a counted read is made in.
const READ_SPAN: &str = "version_copy_read";

/// The statements sqlx sends inside a [`READ_SPAN`] span.
#[derive(Clone, Default)]
struct Statements(Arc<std::sync::Mutex<Vec<String>>>);

/// Reads the statement text of a `sqlx::query` event.
#[derive(Default)]
struct Text(String);

impl tracing::field::Visit for Text {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "db.statement" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "db.statement" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for Statements
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        if event.metadata().target() != "sqlx::query" {
            return;
        }
        let in_read = ctx
            .event_scope(event)
            .is_some_and(|mut scope| scope.any(|span| span.name() == READ_SPAN));
        if in_read {
            let mut text = Text::default();
            event.record(&mut text);
            self.0.lock().expect("the statements").push(text.0);
        }
    }
}

/// A COMPOSITION committed on `svc` in a new EHR: the EHR, the object and
/// its first version.
async fn committed(svc: &FerroEhrService) -> (EhrId, VoId, ObjectVersionId) {
    let ehr_id = create_ehr(svc).await;
    let committed = svc
        .create_composition(ehr_id, uv(&composition("lab result"), "249", None))
        .await
        .expect("the composition commits");
    let version = ObjectVersionId::new(committed.version_uid()).expect("the version id parses");
    (ehr_id, committed.vo_id, version)
}

#[tokio::test]
async fn a_repeated_read_by_version_id_reads_only_its_confirmation() {
    let statements = Statements::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(statements.clone()));
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db, &db).await;
    let (ehr_id, vo, version) = committed(&svc).await;

    let span = || tracing::info_span!(READ_SPAN);
    let first = read(&svc, ehr_id, vo, &version).instrument(span()).await;
    assert!(first.is_some(), "the version is read");
    let sent = std::mem::take(&mut *statements.0.lock().expect("the statements"));
    assert!(
        sent.iter()
            .any(|sql| sql.contains("held_digest") && sql.contains("LATERAL")),
        "the first read reads the version: {sent:?}"
    );
    let again = read(&svc, ehr_id, vo, &version).instrument(span()).await;
    assert_eq!(again, first, "the copy serves the version as the store did");
    let sent = std::mem::take(&mut *statements.0.lock().expect("the statements"));
    assert_eq!(sent.len(), 1, "one statement: {sent:?}");
    assert!(
        sent.iter()
            .all(|sql| sql.contains("h.restricted_at") && !sql.contains("LATERAL")),
        "the one statement confirms the object's state and reads no version: {sent:?}"
    );
}

#[tokio::test]
async fn a_restriction_its_lifting_and_an_erasure_reach_a_replica_that_hears_no_notification() {
    let db = testkit::db().await.expect("testkit database");
    let elsewhere = testkit::db().await.expect("a second database");
    let one = replica(&db, &db).await;
    let two = replica(&db, &elsewhere).await;
    let (ehr_id, vo, version) = committed(&one).await;
    assert!(read(&two, ehr_id, vo, &version).await.is_some());
    assert!(
        read(&two, ehr_id, vo, &version).await.is_some(),
        "the second replica holds the version"
    );

    one.restrict_processing(ehr_id, Some(vo), "gdpr-18-1-a", None)
        .await
        .expect("the restriction commits");
    assert_eq!(
        read(&two, ehr_id, vo, &version).await,
        None,
        "the restricted object is not served from the copy"
    );

    one.lift_restriction(ehr_id, Some(vo))
        .await
        .expect("the lifting commits");
    assert!(
        read(&two, ehr_id, vo, &version).await.is_some(),
        "the lifted object is served again"
    );

    one.admin_ehr_delete(ehr_id)
        .await
        .expect("the erasure commits");
    assert_eq!(
        read(&two, ehr_id, vo, &version).await,
        None,
        "the erased object is not served from the copy"
    );
}
