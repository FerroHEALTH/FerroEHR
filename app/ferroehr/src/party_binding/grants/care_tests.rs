// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care copy's bound weighs each census by its patients and the record
//! subjects they declare (#5760), each cap holds while an evicted entry
//! is read again with the store's answer (#5926), and a census a delta
//! brought up to date is the census read at the delta's revision (#6074).

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use uuid::Uuid;

use super::{CENSUS_BYTES, CareCopy, Census, SUBJECTS, read_census};
use crate::db::acquire::acquire;
use crate::db::snapshot::Snapshot;
use crate::party_binding::grants::caller::care_of::{CareOf, ProviderKey};
use crate::party_binding::grants::caller::store::{self, Changed};
use crate::party_binding::grants::store::Dates;
use openehr_rm::draft::access_control::terminology::ForeignCode;

/// An open care relationship to `carer` in the role `PART`, in the care of
/// one provider.
fn care(carer: Uuid) -> CareOf {
    let dates = Dates {
        lower: None,
        upper: None,
    };
    let role = ForeignCode {
        terminology: "HL7_ParticipationType".to_owned(),
        code: "PART".to_owned(),
    };
    CareOf::new(
        carer,
        dates,
        role,
        ProviderKey::new("urn:example:providers", "hospital"),
    )
}
use crate::storage::access_projection::EhrSubject;

/// A census of `patients` under `team`, each declaring `each` record
/// subjects of an identity source's namespace.
fn census(team: Uuid, patients: usize, each: usize) -> Census {
    let cared: Vec<_> = (0..patients)
        .map(|_| (Uuid::now_v7(), Arc::new(care(team))))
        .collect();
    let declared: BTreeMap<Uuid, Arc<[EhrSubject]>> = cared
        .iter()
        .filter(|_| each > 0)
        .map(|(patient, _)| {
            let subjects: Vec<EhrSubject> = (0..each)
                .map(|n| {
                    EhrSubject::new(
                        "urn:oid:2.16.840.1.113883.2.4.6.3",
                        format!("{patient}-{n}"),
                    )
                })
                .collect();
            let subjects: Arc<[EhrSubject]> = subjects.into();
            (*patient, subjects)
        })
        .collect();
    Census { cared, declared }
}

#[test]
fn a_census_weighs_the_subjects_its_patients_declare() {
    let team = Uuid::now_v7();
    let bare = census(team, 1000, 0);
    let declaring = census(team, 1000, 1);
    let subject_text = declaring
        .declared
        .values()
        .flat_map(|subjects| subjects.iter())
        .map(|subject| subject.namespace.len() + subject.id.len())
        .sum::<usize>();
    let floor = u64::try_from(1000 * size_of::<EhrSubject>() + subject_text).expect("fits");
    assert!(
        declaring.bytes() >= bare.bytes() + floor,
        "declared {} against bare {} and the subjects' {floor}",
        declaring.bytes(),
        bare.bytes(),
    );
}

#[tokio::test]
async fn the_bound_holds_the_declared_subjects_too() {
    let (one, two) = (Uuid::now_v7(), Uuid::now_v7());
    let bare = census(one, 1000, 0).bytes();
    let declaring = (census(one, 1000, 1), census(two, 1000, 1));
    let bound = declaring.0.bytes() + bare;
    assert!(
        2 * bare <= bound,
        "the patients of both teams alone fit the bound"
    );
    let copy = CareCopy::bounded(SUBJECTS, bound);
    copy.teams.insert(one, Arc::new(declaring.0)).await;
    copy.teams.insert(two, Arc::new(declaring.1)).await;
    copy.teams.run_pending_tasks().await;
    let held = copy.held();
    assert_eq!(
        held.teams.entries, 1,
        "the declared subjects of both teams do not fit"
    );
    assert!(held.census_bytes <= bound, "{held:?} within {bound}");
    assert_eq!(held.patients.entries, 1000);
    assert_eq!(held.declared.entries, 1000);
}

/// The entries the copies keep in these tests.
const CAP: u64 = 8;

/// More keys than [`CAP`].
const KEYS: u64 = 64;

/// The care relationships the store holds for `subject` in these tests.
fn relationships(subject: Uuid) -> Arc<[CareOf]> {
    vec![care(subject)].into()
}

#[tokio::test]
async fn the_subjects_stay_under_their_cap_and_an_evicted_one_reads_the_store() {
    let copy = CareCopy::bounded(CAP, CENSUS_BYTES);
    let reads = AtomicUsize::new(0);
    let subjects: Vec<Uuid> = (0..KEYS).map(|_| Uuid::now_v7()).collect();
    for round in 0..2 {
        for subject in &subjects {
            let read = async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(relationships(*subject))
            };
            let answer = copy
                .hold(&copy.subjects, *subject, read, |_| true)
                .await
                .expect("the read succeeds");
            assert_eq!(
                answer,
                relationships(*subject),
                "round {round}: the copy answers what the store holds"
            );
        }
        copy.subjects.run_pending_tasks().await;
        let held = copy.held().subjects;
        assert!(held.entries <= CAP, "{held:?} within {CAP}");
        assert!(held.bytes > 0, "{held:?} weighs its entries");
    }
    assert!(
        reads.load(Ordering::Relaxed) > usize::try_from(KEYS).expect("fits"),
        "the evicted subjects were read again"
    );
}

#[tokio::test]
async fn the_persons_stay_under_their_cap_and_an_evicted_one_reads_the_store() {
    let copy = CareCopy::bounded(CAP, CENSUS_BYTES);
    let reads = AtomicUsize::new(0);
    let declared: Vec<(EhrSubject, Option<Uuid>)> = (0..KEYS)
        .map(|n| {
            let person = (n % 3 != 0).then(Uuid::now_v7);
            (
                EhrSubject::new("urn:oid:2.16.840.1.113883.2.4.6.3", format!("{n}")),
                person,
            )
        })
        .collect();
    for round in 0..2 {
        for (subject, person) in &declared {
            let read = async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(*person)
            };
            let answer = copy
                .hold(&copy.persons, subject.clone(), read, |_| true)
                .await
                .expect("the read succeeds");
            assert_eq!(answer, *person, "round {round}: the store's PERSON");
        }
        copy.persons.run_pending_tasks().await;
        let held = copy.held().persons;
        assert!(held.entries <= CAP, "{held:?} within {CAP}");
    }
    assert!(
        reads.load(Ordering::Relaxed) > usize::try_from(KEYS).expect("fits"),
        "the evicted record subjects were read again"
    );
}

#[tokio::test]
async fn the_censuses_stay_under_their_bound_and_an_evicted_one_reads_the_store() {
    let teams: Vec<Uuid> = (0..16).map(|_| Uuid::now_v7()).collect();
    let one = census(Uuid::now_v7(), 100, 1).bytes();
    let bound = 4 * one;
    let copy = CareCopy::bounded(SUBJECTS, bound);
    let reads = AtomicUsize::new(0);
    let stored: BTreeMap<Uuid, Arc<Census>> = teams
        .iter()
        .map(|team| (*team, Arc::new(census(*team, 100, 1))))
        .collect();
    for _round in 0..2 {
        for team in &teams {
            let held = stored.get(team).map(Arc::clone).expect("a stored census");
            let read = async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(Arc::clone(&held))
            };
            let answer = copy
                .hold(&copy.teams, *team, read, |census| census.bytes() <= bound)
                .await
                .expect("the read succeeds");
            assert_eq!(answer.cared, held.cared, "the store's patients");
            assert_eq!(answer.declared, held.declared, "the store's subjects");
        }
        copy.teams.run_pending_tasks().await;
        let held = copy.held();
        assert!(held.census_bytes <= bound, "{held:?} within {bound}");
        assert!(held.teams.entries < 16, "{held:?} evicted some teams");
        assert_eq!(held.patients.entries, 100 * held.teams.entries);
    }
    assert!(
        reads.load(Ordering::Relaxed) > 16,
        "the evicted censuses were read again"
    );
}

#[tokio::test]
async fn a_census_past_the_bound_is_answered_and_never_held() {
    let team = Uuid::now_v7();
    let large = Arc::new(census(team, 100, 1));
    let bound = large.bytes() - 1;
    let copy = CareCopy::bounded(SUBJECTS, bound);
    for _ in 0..2 {
        let answer = copy
            .hold(
                &copy.teams,
                team,
                async { Ok::<_, Infallible>(Arc::clone(&large)) },
                |census| census.bytes() <= bound,
            )
            .await
            .expect("the read succeeds");
        assert_eq!(answer.cared, large.cared);
    }
    copy.teams.run_pending_tasks().await;
    assert_eq!(copy.held().teams.entries, 0);
}

/// Concurrent misses of one subject share one store read and are all
/// answered what the store holds (#6342); a read that holds nothing, here a
/// census past the bound, leaves each miss to read on its own.
#[tokio::test]
async fn concurrent_misses_of_one_key_make_one_store_read() {
    const MISSES: usize = 16;
    let copy = CareCopy::bounded(CAP, CENSUS_BYTES);
    let reads = AtomicUsize::new(0);
    let subject = Uuid::now_v7();
    let misses = (0..MISSES).map(|_| {
        copy.hold(
            &copy.subjects,
            subject,
            async {
                reads.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Ok::<_, Infallible>(relationships(subject))
            },
            |_| true,
        )
    });
    for answer in futures::future::join_all(misses).await {
        assert_eq!(answer.expect("the read succeeds"), relationships(subject));
    }
    assert_eq!(reads.load(Ordering::Relaxed), 1, "one store read");

    let team = Uuid::now_v7();
    let large = Arc::new(census(team, 100, 1));
    let bound = large.bytes() - 1;
    let copy = CareCopy::bounded(SUBJECTS, bound);
    let reads = AtomicUsize::new(0);
    let misses = (0..4).map(|_| {
        copy.hold(
            &copy.teams,
            team,
            async {
                reads.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                Ok::<_, Infallible>(Arc::clone(&large))
            },
            |census| census.bytes() <= bound,
        )
    });
    for answer in futures::future::join_all(misses).await {
        assert_eq!(answer.expect("the read succeeds").cared, large.cared);
    }
    assert_eq!(
        reads.load(Ordering::Relaxed),
        4,
        "a census never held is read by each"
    );
}

/// One statement of a commit, with the uuids and the texts it binds in turn.
type Statement<'a> = (&'a str, Vec<Uuid>, Vec<&'a str>);

/// One commit of the parity scenario: its name, its statements and the
/// parties it stamps.
type Step<'a> = (&'a str, Vec<Statement<'a>>, Vec<Uuid>);

/// The care teams, patients and holders of the parity scenario.
struct Ward {
    pool: sqlx::PgPool,
    teams: [Uuid; 3],
    patients: Vec<Uuid>,
    /// The `PARTY_RELATIONSHIP` holder of each patient's first relationship.
    holders: Vec<Uuid>,
}

impl Ward {
    fn patient(&self, n: usize) -> Uuid {
        self.patients.get(n).copied().expect("a patient")
    }

    fn holder(&self, n: usize) -> Uuid {
        self.holders.get(n).copied().expect("a holder")
    }

    /// Runs `statements`, each bound to its uuids and text, then stamps
    /// `parties` as a commit does ([`store::finish`]), in one transaction.
    async fn commit(&self, statements: &[Statement<'_>], parties: Vec<Uuid>) {
        let mut conn = acquire(&self.pool).await.expect("a connection");
        let mut tx = sqlx::Connection::begin(&mut *conn)
            .await
            .expect("a transaction");
        for (statement, ids, texts) in statements {
            let mut query = sqlx::query(sqlx::AssertSqlSafe(*statement));
            for id in ids {
                query = query.bind(*id);
            }
            for text in texts {
                query = query.bind(*text);
            }
            query.execute(&mut *tx).await.expect("the statement runs");
        }
        store::finish(&mut tx, parties).await.expect("the stamp");
        // NOTE: our own design (#5762), a test commit of party rows that no request makes.
        tx.commit().await.expect("the commit");
    }

    /// The facts stamped after `since`, read in one snapshot as a replica
    /// reads them.
    async fn changed(&self, since: &Snapshot) -> Changed {
        let mut conn = acquire(&self.pool).await.expect("a connection");
        let mut tx = sqlx::Connection::begin(&mut *conn)
            .await
            .expect("a transaction");
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .expect("the snapshot");
        let changed = store::load(&mut tx, Some(since)).await.expect("the delta");
        // NOTE: our own design (#5762), a read-only snapshot that writes nothing.
        tx.commit().await.expect("the read ends");
        changed
    }
}

/// A care relationship of `$2` to the team `$3` held by `$1` in the role `$4`
/// of `HL7_ParticipationType`, over `$5` in the holder state `$6`.
const RELATE: &str = "INSERT INTO binding_caller_fact (holder_id, ordinal, kind, party_id, \
     target_id, target_kind, terminology_id, code_string, provider_issuer, provider_id, \
     lifecycle_state, role_validity, validity, holder_version) \
     VALUES ($1, 0, 'under_care_of', $2, $3, 'GROUP', 'HL7_ParticipationType', $4, \
     'urn:example:providers', 'hospital', $6, '(,)', $5::daterange, $1::text || '::v1')";

/// [`RELATE`] scoped to the episode `$7`, `$8` (#6412).
const RELATE_IN: &str = "INSERT INTO binding_caller_fact (holder_id, ordinal, kind, party_id, \
     target_id, target_kind, terminology_id, code_string, provider_issuer, provider_id, \
     lifecycle_state, role_validity, validity, holder_version, episode_issuer, episode_id) \
     VALUES ($1, 0, 'under_care_of', $2, $3, 'GROUP', 'HL7_ParticipationType', $4, \
     'urn:example:providers', 'hospital', $6, '(,)', $5::daterange, $1::text || '::v1', $7, $8)";

/// The relationship the holder `$1` held, deleted as its next version does.
const UNRELATE: &str = "DELETE FROM binding_caller_fact WHERE holder_id = $1";

/// The record subject `$2` of the namespace `$3` the `PERSON` `$1` declares
/// at position 0 in the state `$4`.
const DECLARE: &str = "INSERT INTO binding_record_subject \
     (holder_id, ordinal, namespace, subject_id, lifecycle_state) VALUES ($1, 0, $2, $3, $4)";

/// A ward of twelve patients, each under the care of the first or the second
/// team, every one of the first four declaring a record subject.
async fn ward() -> (testkit::TestDb, Ward) {
    let db = testkit::db().await.expect("testkit database");
    let pool = crate::db::domain_pool_from(&db.pool(), crate::db::domain::Domain::Party);
    let teams = [Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()];
    let patients: Vec<Uuid> = (0..12).map(|_| Uuid::now_v7()).collect();
    let holders: Vec<Uuid> = (0..12).map(|_| Uuid::now_v7()).collect();
    let ward = Ward {
        pool,
        teams,
        patients,
        holders,
    };
    for n in 0..12 {
        let team = ward.teams.get(n % 2).copied().expect("a team");
        let mut statements = vec![(
            RELATE,
            vec![ward.holder(n), ward.patient(n), team],
            vec!["PART", "[2020-01-01,)", "532"],
        )];
        let id = format!("pseudonym-{n}");
        if n < 4 {
            statements.push((
                DECLARE,
                vec![ward.patient(n)],
                vec!["urn:example:pseudonyms", id.as_str(), "532"],
            ));
        }
        ward.commit(&statements, vec![ward.patient(n)]).await;
    }
    (db, ward)
}

/// Parity (#6074): after each relationship commit a delta brings every held
/// census to the census read fresh at the delta's revision: an admission to
/// a second team, a discharge, a role change, a move between teams, a
/// relationship deleted, record subjects declared and withdrawn, a holder
/// no longer live, an episode added, changed and removed (#6412), a stamp
/// that changed nothing and a party that is no patient.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario of commits, each checked against a fresh read"
)]
async fn a_census_after_each_delta_is_the_census_read_at_its_revision() {
    let (_db, ward) = ward().await;
    let copy = CareCopy::new();
    for team in ward.teams {
        copy.of_team(&ward.pool, team)
            .await
            .expect("the census reads");
    }
    let [first, second, third] = ward.teams;
    let (admitted, again) = (Uuid::now_v7(), Uuid::now_v7());
    let pseudonym = |n: usize| format!("pseudonym-{n}-b");
    let (late, withdrawn) = (pseudonym(5), pseudonym(2));
    let member = Uuid::now_v7();
    let steps: Vec<Step<'_>> = vec![
        (
            "an admission to a second team",
            vec![(
                RELATE,
                vec![admitted, ward.patient(0), third],
                vec!["PART", "[2026-01-01,)", "532"],
            )],
            vec![ward.patient(0)],
        ),
        (
            "a discharge",
            vec![
                (UNRELATE, vec![ward.holder(1)], vec![]),
                (
                    RELATE,
                    vec![ward.holder(1), ward.patient(1), second],
                    vec!["PART", "[2020-01-01,2024-01-01)", "532"],
                ),
            ],
            vec![ward.patient(1)],
        ),
        (
            "a role change",
            vec![
                (UNRELATE, vec![ward.holder(2)], vec![]),
                (
                    RELATE,
                    vec![ward.holder(2), ward.patient(2), first],
                    vec!["ATND", "[2020-01-01,)", "532"],
                ),
            ],
            vec![ward.patient(2)],
        ),
        (
            "a move between teams",
            vec![
                (UNRELATE, vec![ward.holder(3)], vec![]),
                (
                    RELATE,
                    vec![ward.holder(3), ward.patient(3), first],
                    vec!["PART", "[2020-01-01,)", "532"],
                ),
            ],
            vec![ward.patient(3)],
        ),
        (
            "a relationship deleted",
            vec![(UNRELATE, vec![ward.holder(4)], vec![])],
            vec![ward.patient(4)],
        ),
        (
            "a record subject declared and another withdrawn",
            vec![
                (
                    DECLARE,
                    vec![ward.patient(5)],
                    vec!["urn:example:pseudonyms", late.as_str(), "532"],
                ),
                (
                    "UPDATE binding_record_subject SET lifecycle_state = '800', subject_id = $2 \
                     WHERE holder_id = $1",
                    vec![ward.patient(2)],
                    vec![withdrawn.as_str()],
                ),
            ],
            vec![ward.patient(5), ward.patient(2)],
        ),
        (
            "a holder no longer live",
            vec![(
                "UPDATE binding_caller_fact SET lifecycle_state = '800' WHERE holder_id = $1",
                vec![ward.holder(6)],
                vec![],
            )],
            vec![ward.patient(6)],
        ),
        (
            "a second relationship to the same team",
            vec![(
                RELATE,
                vec![again, ward.patient(7), second],
                vec!["ATND", "[2025-01-01,2027-01-01)", "532"],
            )],
            vec![ward.patient(7)],
        ),
        (
            "an episode added",
            vec![
                (UNRELATE, vec![ward.holder(9)], vec![]),
                (
                    RELATE_IN,
                    vec![ward.holder(9), ward.patient(9), second],
                    vec!["PART", "[2020-01-01,)", "532", "H", "C-17"],
                ),
            ],
            vec![ward.patient(9)],
        ),
        (
            "an episode changed",
            vec![
                (UNRELATE, vec![ward.holder(9)], vec![]),
                (
                    RELATE_IN,
                    vec![ward.holder(9), ward.patient(9), second],
                    vec!["PART", "[2020-01-01,)", "532", "H", "D-4"],
                ),
            ],
            vec![ward.patient(9)],
        ),
        (
            "an episode removed",
            vec![
                (UNRELATE, vec![ward.holder(9)], vec![]),
                (
                    RELATE,
                    vec![ward.holder(9), ward.patient(9), second],
                    vec!["PART", "[2020-01-01,)", "532"],
                ),
            ],
            vec![ward.patient(9)],
        ),
        (
            "a stamp that changed nothing and a party that is no patient",
            vec![(
                "INSERT INTO binding_caller_fact (holder_id, ordinal, kind, party_id, target_id, \
                 lifecycle_state, role_validity, validity) \
                 VALUES ($1, 0, 'member_of', $1, $2, '532', '(,)', '(,)')",
                vec![member, first],
                vec![],
            )],
            vec![ward.patient(8), member],
        ),
    ];
    let mut since = store::load(&mut acquire(&ward.pool).await.expect("a connection"), None)
        .await
        .expect("the facts read")
        .snapshot;
    for (name, statements, parties) in steps {
        ward.commit(&statements, parties).await;
        let changed = ward.changed(&since).await;
        assert!(
            changed.snapshot.covers(&since) && !since.covers(&changed.snapshot),
            "{name}: the snapshot moves"
        );
        since = Arc::clone(&changed.snapshot);
        copy.apply(&changed).await;
        for (team, label) in [(first, "first"), (second, "second"), (third, "third")] {
            let held = copy.teams.get(&team).await.expect("the census stays held");
            let fresh = read_census(&ward.pool, team)
                .await
                .expect("the census reads");
            assert_eq!(*held, *fresh, "{name}: the {label} team's census");
        }
    }
    let first_census = copy.teams.get(&first).await.expect("held");
    assert!(
        first_census
            .cared
            .iter()
            .any(|(subject, care)| *subject == ward.patient(3) && care.carer() == first),
        "the moved patient is in the first team's census"
    );
}

/// A census a delta leaves is read again by no request: a relationship
/// commit to a held team keeps the team held, so the next use reads none.
#[tokio::test]
async fn a_relationship_commit_keeps_its_teams_census_held() {
    let (_db, ward) = ward().await;
    let copy = CareCopy::new();
    let [first, ..] = ward.teams;
    copy.of_team(&ward.pool, first)
        .await
        .expect("the census reads");
    let since = store::load(&mut acquire(&ward.pool).await.expect("a connection"), None)
        .await
        .expect("the facts read")
        .snapshot;
    let newcomer = Uuid::now_v7();
    ward.commit(
        &[(
            RELATE,
            vec![Uuid::now_v7(), newcomer, first],
            vec!["PART", "[2026-01-01,)", "532"],
        )],
        vec![newcomer],
    )
    .await;
    copy.apply(&ward.changed(&since).await).await;
    let reads = AtomicUsize::new(0);
    let held = copy
        .hold(
            &copy.teams,
            first,
            async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(Arc::new(Census::default()))
            },
            |_| true,
        )
        .await
        .expect("the census answers");
    assert_eq!(reads.load(Ordering::Relaxed), 0, "no read of the team");
    assert!(
        held.cared.iter().any(|(subject, _)| *subject == newcomer),
        "the newcomer is in the census"
    );
}
