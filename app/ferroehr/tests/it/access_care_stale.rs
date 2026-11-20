// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A care relationship removed on one replica never admits a read on
//! another whose notification has not arrived (#5806), against a real
//! `PostgreSQL` 18 (shared testkit harness).
//!
//! The lagging replica listens on a database of its own, so it holds its
//! copy of the caller facts and the care relationships as current and hears
//! of no change: the longest lag a notification can have. A care
//! relationship that ended, a membership of the care team that ended, and an
//! erased `PERSON` each refuse its next direct read, its next query pinned to
//! the EHR and its next query over every EHR. The care requirement is the
//! controller's (BASE Architecture Overview `master07-security.adoc`
//! §Security Policy); the copies and their check are our own design.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use sqlx::postgres::PgConnectOptions;

use crate::access_eval_examples::update_as_adt as adt;
use crate::access_eval_examples::{
    MEMBER, access, care, care_of, party, person, policy, relate_as_adt, rerelate_as_adt,
};
use crate::fixtures::{composition, create_ehr_for_party, uv};
use crate::policy_fixtures::{
    CARE_PROVIDER, care_provider, care_writer, create, granted, granting, provider_team,
};
use ferroehr::access_eval::alongside;
use ferroehr::access_eval::condition::{RequestAccess, current, with_request_access};
use ferroehr::access_eval::procedure::Outcome;
use ferroehr::access_eval::scope::Item;
use ferroehr::db::acquire::request_scope;
use ferroehr::error::ServiceError;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::policy_register::listen::Listeners;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::ehr::composition_read::JudgedRead;
use ferroehr::service::query::request::AqlQueryRequest;

/// The ward's records, written on the first replica and read on both.
struct Ward {
    db: testkit::TestDb,
    _elsewhere: testkit::TestDb,
    one: FerroEhrService,
    two: Arc<FerroEhrService>,
    team: VoId,
    doctor: (VoId, String),
    patient: VoId,
    /// The patient's care relationship and its version.
    stay: (VoId, String),
    ehr_id: EhrId,
    vo: VoId,
}

/// Today in UTC, and the day `days` from it, as `YYYY-MM-DD`.
fn day(days: i64) -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .checked_add(jiff::Span::new().days(days))
        .expect("a date")
        .to_string()
}

/// A replica over `db`, as the server installs it.
async fn replica(db: &testkit::TestDb) -> FerroEhrService {
    granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
    ))
    .await
}

impl Ward {
    /// Dr D, a member of the ward, and Mr E under its care with a record of
    /// one composition, both held on the second replica, which hears no
    /// notification.
    async fn new() -> Self {
        let db = testkit::db().await.expect("testkit database");
        let elsewhere = testkit::db().await.expect("a second database");
        let one = replica(&db).await;
        let two = Arc::new(replica(&db).await);
        granted(&one, "adt", &[care_writer()]).await;
        let provider = care_provider(&one, CARE_PROVIDER).await;
        let ward = party("GROUP", "openEHR-DEMOGRAPHIC-GROUP.ward.v1", "Ward", &[]);
        let (ward, _) = provider_team(&one, provider, &ward).await;
        let (doctor, uid) = create(&one, PartyKind::Person, &person("Dr D", &[]))
            .await
            .expect("the professional commits");
        let roster = person("Dr D", &[care(MEMBER, doctor, ward, &day(-30), None)]);
        let doctor_uid = adt(&one, (doctor, &uid), &roster).await;
        let (patient, _) = create(&one, PartyKind::Person, &person("Mr E", &[]))
            .await
            .expect("the patient commits");
        let stay = relate_as_adt(&one, &care_of(patient, ward, &day(-3), Some(&day(30)))).await;
        let ehr_id = create_ehr_for_party(&one, patient).await;
        let vo = one
            .create_composition(ehr_id, uv(&composition("ward note"), "249", None))
            .await
            .expect("the composition commits")
            .vo_id;
        let listened = two.grants().listened();
        Listeners::on(
            PgConnectOptions::from_str(elsewhere.url()).expect("the test DSN parses"),
            &[Arc::clone(listened)],
        )
        .spawn();
        for _ in 0..250 {
            if listened.is_listening() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(listened.is_listening(), "the lagging listener connects");
        two.grants().refresh().await.expect("the copy reads");
        let ward = Self {
            db,
            _elsewhere: elsewhere,
            one,
            two,
            team: ward,
            doctor: (doctor, doctor_uid),
            patient,
            stay,
            ehr_id,
            vo,
        };
        assert_eq!(
            ward.read_on_two().await,
            Read::admitted(),
            "the lagging replica reads the record of a patient in care"
        );
        ward
    }

    /// The evaluator of Dr D's next request on the lagging replica, built
    /// from what that replica holds, as the server builds it.
    async fn request_on_two(&self) -> Arc<RequestAccess> {
        self.request_on_two_under(true).await
    }

    /// [`Self::request_on_two`] under a policy that requires a care
    /// relationship when `care` holds.
    async fn request_on_two_under(&self, care: bool) -> Arc<RequestAccess> {
        self.two
            .grants()
            .refresh_if_stale()
            .await
            .expect("the copy reads");
        let ward = policy(&json!({ "care_relationship_required": care }));
        access(&self.two, self.doctor.0, ward, jiff::Timestamp::now())
    }

    /// Dr D's next request on the lagging replica as the REST layer runs a
    /// COMPOSITION read (#5959): the read of the composition handed to the
    /// decision on its EHR, inside a request scope, so a second connection
    /// of one pool trips the nested-acquire check.
    async fn handed_on_two(&self, care: bool, uses: alongside::Use) -> Handed {
        let access = self.request_on_two_under(care).await;
        let svc = Arc::clone(&self.two);
        let (ehr_id, vo) = (self.ehr_id, self.vo);
        let run = async move {
            let current = current().expect("the request access");
            let (found, mut answer) = tokio::sync::oneshot::channel();
            let reader = Arc::clone(&svc);
            let left = alongside::register(uses, async move {
                let _sent = found.send(reader.composition_ahead(vo, None).await);
            });
            assert!(left.is_none(), "the request's slot takes the read");
            let verdict = svc.ehr_verdict(&current, ehr_id).await;
            let read_by_decision = answer.try_recv();
            let ran_in_decision = read_by_decision.is_ok();
            let admitted = match verdict {
                Ok(verdict) => verdict.outcome(&Item::Ehr) == Outcome::Admit,
                Err(error) => {
                    return Handed {
                        admitted: Err(error),
                        ran_in_decision,
                        served: None,
                    };
                }
            };
            if !admitted {
                return Handed {
                    admitted: Ok(false),
                    ran_in_decision,
                    served: None,
                };
            }
            let ahead = if let Ok(ahead) = read_by_decision {
                ahead
            } else {
                alongside::run().await;
                answer.await.expect("the read left for the request answers")
            };
            let served = svc
                .composition_judged_on(&current, ehr_id, vo, None, ahead)
                .await;
            Handed {
                admitted: Ok(true),
                ran_in_decision,
                served: Some(matches!(served, JudgedRead::Admitted(Ok(_)))),
            }
        };
        Box::pin(request_scope(alongside::scope(with_request_access(
            access, run,
        ))))
        .await
    }

    /// What Dr D's next request on the lagging replica reads.
    async fn read_on_two(&self) -> Read {
        let access = self.request_on_two().await;
        let svc = &self.two;
        let run =
            async {
                let current = current().expect("the request access");
                let direct =
                    match Box::pin(svc.composition_judged(&current, self.ehr_id, self.vo, None))
                        .await
                    {
                        JudgedRead::Admitted(Ok(_)) => Ok(true),
                        JudgedRead::Absent | JudgedRead::Refused { .. } => Ok(false),
                        JudgedRead::Admitted(Err(error))
                        | JudgedRead::Unheld(error)
                        | JudgedRead::Unjudged(error) => Err(error),
                    };
                let pinned = query(svc, &pinned_to(self.ehr_id)).await;
                let census = query(svc, COMPOSITIONS).await;
                Read {
                    direct,
                    pinned,
                    census,
                }
            };
        Box::pin(with_request_access(access, run)).await
    }
}

/// What a request that handed its read to the decision found.
#[derive(Debug)]
struct Handed {
    /// Whether the decision admitted the EHR, or its error.
    admitted: Result<bool, ServiceError>,
    /// Whether the decision ran the read beside its party read.
    ran_in_decision: bool,
    /// Whether the admitted request served the composition it read.
    served: Option<bool>,
}

/// Every composition of every EHR.
const COMPOSITIONS: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";

/// The compositions of `ehr_id` alone, which the service decides before the
/// statement (#5746).
fn pinned_to(ehr_id: EhrId) -> String {
    format!("{COMPOSITIONS} WHERE e/ehr_id/value = '{ehr_id}'")
}

/// The rows `aql` answers the running request, or its refusal.
async fn query(svc: &FerroEhrService, aql: &str) -> Result<usize, ServiceError> {
    let request = AqlQueryRequest {
        ehr_access: svc.request_access_condition().await?,
        ..AqlQueryRequest::default()
    };
    let outcome = Box::pin(svc.execute_aql(aql, None, &request)).await?;
    Ok(outcome.result_set.to_value().unwrap()["rows"]
        .as_array()
        .map_or(0, Vec::len))
}

/// What one request read: whether the composition by id was admitted, the
/// rows of the query pinned to its EHR, and of the query over every EHR.
#[derive(Debug)]
struct Read {
    direct: Result<bool, ServiceError>,
    pinned: Result<usize, ServiceError>,
    census: Result<usize, ServiceError>,
}

impl Read {
    /// Everything read.
    fn admitted() -> Self {
        Self {
            direct: Ok(true),
            pinned: Ok(1),
            census: Ok(1),
        }
    }

    /// Nothing read: the composition refused, each query answering no row,
    /// or every one refused while the replica's state is refreshed (`503`).
    fn admits_nothing(&self) -> bool {
        let refreshing = |e: &ServiceError| matches!(e, ServiceError::AccessRefreshing(_));
        self.direct
            .as_ref()
            .map_or_else(refreshing, |admitted| !admitted)
            && self
                .pinned
                .as_ref()
                .map_or_else(refreshing, |rows| *rows == 0)
            && self
                .census
                .as_ref()
                .map_or_else(refreshing, |rows| *rows == 0)
    }
}

impl PartialEq for Read {
    fn eq(&self, other: &Self) -> bool {
        self.direct.as_ref().ok() == other.direct.as_ref().ok()
            && self.pinned.as_ref().ok() == other.pinned.as_ref().ok()
            && self.census.as_ref().ok() == other.census.as_ref().ok()
    }
}

/// The patient's care ends on the first replica; the lagging replica's next
/// request reads nothing of the record.
#[tokio::test]
async fn an_ended_care_relationship_admits_nothing_on_a_lagging_replica() {
    let ward = Box::pin(Ward::new()).await;
    let (stay, uid) = &ward.stay;
    let ended = care_of(ward.patient, ward.team, &day(-30), Some(&day(-20)));
    rerelate_as_adt(&ward.one, (*stay, uid), &ended).await;
    let read = ward.read_on_two().await;
    assert!(
        read.admits_nothing(),
        "an ended care relationship: {read:?}"
    );
}

/// Dr D's membership of the ward ends on the first replica; the lagging
/// replica's next request reads nothing of the record.
#[tokio::test]
async fn an_ended_membership_admits_nothing_on_a_lagging_replica() {
    let ward = Box::pin(Ward::new()).await;
    let (doctor, uid) = &ward.doctor;
    let left = person(
        "Dr D",
        &[care(MEMBER, *doctor, ward.team, &day(-30), Some(&day(-20)))],
    );
    adt(&ward.one, (*doctor, uid), &left).await;
    let read = ward.read_on_two().await;
    assert!(read.admits_nothing(), "an ended membership: {read:?}");
}

/// The patient's `PERSON` is erased on the first replica; the lagging
/// replica's next request reads nothing of the record.
#[tokio::test]
async fn an_erased_person_admits_nothing_on_a_lagging_replica() {
    let ward = Box::pin(Ward::new()).await;
    ward.one
        .physical_party_delete(ward.patient)
        .await
        .expect("the patient is erased");
    let read = ward.read_on_two().await;
    assert!(read.admits_nothing(), "an erased PERSON: {read:?}");
}

/// A care relationship in force reads on the lagging replica on every
/// request, whatever the check of its copy costs.
#[tokio::test]
async fn a_lagging_replica_still_admits_a_care_relationship_in_force() {
    let ward = Box::pin(Ward::new()).await;
    for _ in 0..3 {
        assert_eq!(
            ward.read_on_two().await,
            Read::admitted(),
            "a care relationship in force reads on every request"
        );
    }
}

/// A care-decided read hands a read it uses whenever admitted to the
/// decision, which runs it beside its party read with no second connection
/// of one pool, and serves it once admitted (#5959).
#[tokio::test]
async fn the_decision_runs_the_read_beside_its_party_read() {
    let ward = Box::pin(Ward::new()).await;
    let handed = ward.handed_on_two(true, alongside::Use::Admitted).await;
    assert!(matches!(handed.admitted, Ok(true)), "{handed:?}");
    assert!(
        handed.ran_in_decision,
        "the read ran inside the decision: {handed:?}"
    );
    assert_eq!(handed.served, Some(true), "{handed:?}");
}

/// A care relationship that ended refuses the lagging replica's request even
/// though its read already ran beside the decision, and nothing of the read
/// is served (#5959).
#[tokio::test]
async fn a_refusal_wins_over_a_read_that_already_ran() {
    let ward = Box::pin(Ward::new()).await;
    let (stay, uid) = &ward.stay;
    let ended = care_of(ward.patient, ward.team, &day(-30), Some(&day(-20)));
    rerelate_as_adt(&ward.one, (*stay, uid), &ended).await;
    let handed = ward.handed_on_two(true, alongside::Use::Admitted).await;
    let refused = match &handed.admitted {
        Ok(admitted) => !admitted,
        Err(error) => matches!(error, ServiceError::AccessRefreshing(_)),
    };
    assert!(refused, "an ended care relationship: {handed:?}");
    assert!(handed.ran_in_decision, "the read had run: {handed:?}");
    assert_eq!(handed.served, None, "nothing of it is served: {handed:?}");
}

/// A decision care does not decide makes no party read, leaves the read to
/// the request, which runs it once admitted, as before (#5959).
#[tokio::test]
async fn a_read_no_party_read_runs_beside_is_run_by_the_request() {
    let ward = Box::pin(Ward::new()).await;
    let handed = ward.handed_on_two(false, alongside::Use::Admitted).await;
    assert!(matches!(handed.admitted, Ok(true)), "{handed:?}");
    assert!(
        !handed.ran_in_decision,
        "no party read to run it beside: {handed:?}"
    );
    assert_eq!(handed.served, Some(true), "{handed:?}");
}

/// A read the request uses only under a verdict that judges items is left
/// to the request when the care-decided verdict judges the EHR alone, since
/// such a read is then served by another path; once admitted, the request
/// runs it itself (#5959).
#[tokio::test]
async fn a_read_the_verdict_does_not_judge_is_left_to_the_request() {
    let ward = Box::pin(Ward::new()).await;
    let handed = ward.handed_on_two(true, alongside::Use::ItemJudged).await;
    assert!(matches!(handed.admitted, Ok(true)), "{handed:?}");
    assert!(
        !handed.ran_in_decision,
        "a verdict on the EHR alone uses no row's read: {handed:?}"
    );
    assert_eq!(handed.served, Some(true), "{handed:?}");
}

/// A query that reads care on a replica whose care copy is cold holds one
/// party connection at a time: the care census and the read ahead of the
/// store's revisions each read the party store, and run one after the other
/// on the connection the request keeps. Run side by side, the second asked
/// the pool for a connection while the first held one (#5967).
#[tokio::test]
async fn a_query_reading_care_on_a_cold_copy_holds_one_party_connection() {
    let ward = Box::pin(Ward::new()).await;
    let cold = replica(&ward.db).await;
    cold.grants().refresh().await.expect("the copy reads");
    let ward_policy = policy(&json!({ "care_relationship_required": true }));
    let doctor = access(&cold, ward.doctor.0, ward_policy, jiff::Timestamp::now());
    let svc = &cold;
    let read = async {
        // NOTE: (#5879) a nested acquire is refused in a debug build.
        let census = query(svc, COMPOSITIONS).await;
        let pinned = query(svc, &pinned_to(ward.ehr_id)).await;
        (census, pinned)
    };
    let (census, pinned) = Box::pin(request_scope(with_request_access(doctor, read))).await;
    assert_eq!(census.expect("the query over every EHR"), 1);
    assert_eq!(pinned.expect("the query pinned to the EHR"), 1);
}
