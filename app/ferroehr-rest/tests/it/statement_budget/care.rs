// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The budgets of the requests a care relationship decides or writes, in
//! the hospital scenario with a running server's `LISTEN` sessions connected
//! (#6074). No openEHR spec governs the statements of a request: our own
//! design.

use std::fmt::Write as _;
use std::sync::atomic::Ordering;

use http::StatusCode;
use serde_json::json;
use tracing::Instrument as _;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{CHAINER_RELEASE, REQUEST_SPAN, RoundTrips, calls, counted, ledger};
use crate::common::{self, BASE, EXT};
use crate::hospital::world::{PURPOSE, World, care, credential, day};

/// The requests sent before the measured ones.
const WARM: u64 = 3;

/// The measured requests.
const REQUESTS: u64 = 10;

/// What one request of a kind may cost: its round trips, the statements
/// nested in them (the chainer's rounds left out), the statements that
/// name `pg_notify`, top-level or nested, and the bytes of row records it
/// writes outside the audit and access-log relations and outside every TOAST
/// table, where the access-log entry's record goes.
#[derive(Clone, Copy, Debug)]
struct Ceiling {
    round_trips: u64,
    nested: u64,
    notifies: u64,
    wal: u64,
}

/// A care-gated read: the party store's revisions and the version read,
/// which carries the policy revision (#6251). Its nested statements fell
/// to 0 once the access-log entry was written by one plain insert (#6242).
const READ: Ceiling = Ceiling {
    round_trips: 2,
    nested: 0,
    notifies: 0,
    wal: 0,
};

/// A query narrowed to the caller's caseload (#6074, #6320).
const CASELOAD: Ceiling = Ceiling {
    round_trips: 4,
    nested: 0,
    notifies: 0,
    wal: 0,
};

/// A care-gated read of a datum in the episode the caller's one live
/// relationship is scoped to (#6414): the same statements as [`READ`].
const EPISODE_READ: Ceiling = READ;

/// A care-gated read of a session launched in an episode (#6417): the
/// launched episode is checked against the care copy the read holds.
const LAUNCHED_READ: Ceiling = READ;

/// A caseload query of a caller whose one live relationship is scoped to an
/// episode (#6415): the coverage rides in its one clinical statement.
const EPISODE_CASELOAD: Ceiling = CASELOAD;

/// A care relationship's commit (#6074).
const COMMIT: Ceiling = Ceiling {
    round_trips: 26,
    nested: 2,
    notifies: 0,
    wal: 7403,
};

/// A care relationship's commit, then the caseload query that reads the copy it kept (#6074).
const RECARE: Ceiling = Ceiling {
    round_trips: 34,
    nested: COMMIT.nested + CASELOAD.nested,
    notifies: COMMIT.notifies + CASELOAD.notifies,
    wal: COMMIT.wal + CASELOAD.wal,
};

/// A kind of request this module measures.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// Dr Jansen's read of the encounter, declaring the treatment purpose.
    Read,
    /// Dr Jansen's query of every composition in her caseload.
    Caseload,
    /// The ADT system's commit of a new care relationship of Eva with the
    /// cardiology team, from today.
    Commit,
    /// [`Self::Commit`], then [`Self::Caseload`].
    Recare,
    /// Dr Jansen's read of a note in the cardiology episode, once her one
    /// live relationship to Eva is scoped to it.
    EpisodeRead,
    /// Dr Jansen's caseload query, once her one live relationship to Eva is
    /// scoped to the cardiology episode.
    EpisodeCaseload,
    /// Dr Jansen's read of a note in the cardiology episode in a SMART on
    /// openEHR session launched in it.
    LaunchedRead,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Read => "care-gated read",
            Self::Caseload => "caseload query",
            Self::Commit => "care relationship commit",
            Self::Recare => "care relationship commit, then the caseload query",
            Self::EpisodeRead => "care-gated read in an episode",
            Self::EpisodeCaseload => "caseload query under an episode relationship",
            Self::LaunchedRead => "care-gated read in a launched episode",
        }
    }

    fn ceiling(self) -> Ceiling {
        match self {
            Self::Read => READ,
            Self::Caseload => CASELOAD,
            Self::Commit => COMMIT,
            Self::Recare => RECARE,
            Self::EpisodeRead => EPISODE_READ,
            Self::EpisodeCaseload => EPISODE_CASELOAD,
            Self::LaunchedRead => LAUNCHED_READ,
        }
    }

    /// The request, its expected status and a check of the body.
    fn request(self, world: &World) -> (http::Request<axum::body::Body>, StatusCode) {
        match self {
            Self::LaunchedRead => (
                common::req(
                    "GET",
                    format!(
                        "{BASE}/ehr/{}/composition/{}",
                        world.ehr, world.encounter.vo
                    ),
                )
                .auth(crate::hospital::episode_launch::jansen(Some(
                    "urn:sint-example:episodes|C-17",
                )))
                .accept("application/json")
                .header(PURPOSE, "TREAT")
                .build(),
                StatusCode::OK,
            ),
            Self::Read | Self::EpisodeRead => (
                common::req(
                    "GET",
                    format!(
                        "{BASE}/ehr/{}/composition/{}",
                        world.ehr, world.encounter.vo
                    ),
                )
                .auth(credential("jansen"))
                .accept("application/json")
                .header(PURPOSE, "TREAT")
                .build(),
                StatusCode::OK,
            ),
            Self::Caseload | Self::EpisodeCaseload => {
                let body = json!({
                    "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
                    "caseload": "all",
                });
                (
                    common::req("POST", format!("{BASE}/query/aql"))
                        .auth(credential("jansen"))
                        .accept("application/json")
                        .header(PURPOSE, "TREAT")
                        .json(body.to_string())
                        .build(),
                    StatusCode::OK,
                )
            }
            Self::Commit | Self::Recare => {
                let body = care(
                    "ATND",
                    &world.eva,
                    &world.teams["Cardiology"],
                    (&day(0), None),
                );
                (
                    common::req(
                        "POST",
                        format!("{EXT}/demographic/person/{}/care_relationship", world.eva),
                    )
                    .auth(credential("client:adt"))
                    .accept("application/json")
                    .json(body.to_string())
                    .build(),
                    StatusCode::CREATED,
                )
            }
        }
    }

    /// Sends one request on a task of its own, inside `span` when given, as
    /// `serve` polls each request at the root of its connection's task.
    async fn send(self, world: &World, span: Option<tracing::Span>) {
        if matches!(self, Self::Recare) {
            Box::pin(Self::Commit.send(world, span.clone())).await;
            return Box::pin(Self::Caseload.send(world, span)).await;
        }
        let (req, expected) = self.request(world);
        let app = world.app.clone();
        let answered = async move { common::send(&app, req).await };
        let (status, _, body) = match span {
            Some(span) => tokio::spawn(answered.instrument(span)).await,
            None => tokio::spawn(answered).await,
        }
        .expect("the request's task");
        assert_eq!(status, expected, "{}: {body}", self.name());
        if matches!(self, Self::Caseload | Self::EpisodeCaseload) {
            let rows =
                serde_json::from_str::<serde_json::Value>(&body).expect("a RESULT_SET")["rows"]
                    .as_array()
                    .map_or(0, Vec::len);
            assert!(rows > 0, "the caseload holds Eva's compositions: {body}");
        }
    }
}

/// Scopes Dr Jansen's care for Eva to the cardiology episode: a
/// relationship of Eva with Cardiology in it, the stay that names none
/// ended past its grace, and the encounter placed in it.
async fn scoped_to_an_episode(world: &mut World) {
    use crate::hospital::world::admit;
    crate::hospital::episodes::relate_in(world, "Cardiology", "C-17").await;
    world.encounter = crate::hospital::episodes::placed(world, "jansen", "C-17").await;
    let (ward, old_from, old_until) = (day(-2), day(-40), day(-20));
    admit(
        world,
        &[
            ("Cardiology", &old_from, Some(&old_until)),
            ("Ward 3", &ward, None),
            ("Psychiatry", &ward, None),
        ],
    )
    .await;
}

/// A care-gated read in an episode stays within its ceiling (#6414).
#[tokio::test]
async fn a_care_gated_read_in_an_episode_stays_within_its_statement_budget() {
    let (report, kept) = measured(Kind::EpisodeRead).await;
    eprintln!("{report}");
    assert!(
        kept,
        "a care-gated read in an episode exceeds its budget:\n{report}"
    );
}

/// A care-gated read in a launched episode stays within its ceiling (#6417).
#[tokio::test]
async fn a_care_gated_read_in_a_launched_episode_stays_within_its_statement_budget() {
    let (report, kept) = measured(Kind::LaunchedRead).await;
    eprintln!("{report}");
    assert!(
        kept,
        "a care-gated read in a launched episode exceeds its budget:\n{report}"
    );
}

/// A caseload query under an episode relationship stays within its ceiling
/// (#6415).
#[tokio::test]
async fn a_caseload_query_under_an_episode_relationship_stays_within_its_statement_budget() {
    let (report, kept) = measured(Kind::EpisodeCaseload).await;
    eprintln!("{report}");
    assert!(
        kept,
        "a caseload query under an episode relationship exceeds its budget:\n{report}"
    );
}

/// Whether `relation` is written by the request itself rather than by the
/// audit lane beside it.
fn owned(relation: &str) -> bool {
    super::wal::commit_relation(relation) && !relation.starts_with("pg_toast")
}

/// Measures `kind` over [`REQUESTS`] requests after [`WARM`] in a fresh
/// hospital world; its report and whether it kept its ceiling.
async fn measured(kind: Kind) -> (String, bool) {
    let round_trips = RoundTrips::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(round_trips.clone()));
    let mut world = Box::pin(crate::hospital::build::world()).await;
    if matches!(kind, Kind::EpisodeRead | Kind::EpisodeCaseload) {
        scoped_to_an_episode(&mut world).await;
    }
    let _launched = if matches!(kind, Kind::LaunchedRead) {
        crate::hospital::episodes::relate_in(&world, "Cardiology", "C-17").await;
        world.encounter = crate::hospital::episodes::placed(&world, "jansen", "C-17").await;
        let (svc, app, drain) = crate::hospital::episode_launch::launching(&world).await;
        super::listening(&world.db, &svc).await;
        world.app = app;
        Some((svc, drain))
    } else {
        None
    };
    super::listening(&world.db, &world.svc).await;
    for extension in ["pg_stat_statements", "pg_walinspect"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE EXTENSION IF NOT EXISTS {extension}"
        )))
        .execute(&world.pool)
        .await
        .expect("the server loads the extension (shared_preload_libraries)");
    }
    super::wal::kept(&world.pool).await;
    for _ in 0..WARM {
        kind.send(&world, None).await;
    }
    let sent_before = round_trips.0.load(Ordering::Relaxed);
    let (relations, evicted) = super::wal::commit_window(&world.pool, async {
        for _ in 0..REQUESTS {
            kind.send(&world, Some(tracing::info_span!(REQUEST_SPAN)))
                .await;
            super::drained(&world.pool).await;
        }
    })
    .await;
    let sent = round_trips.0.load(Ordering::Relaxed) - sent_before;
    let statements = counted(&world.pool).await;
    testkit::statements::assert_statements_kept(&world.pool, evicted)
        .await
        .expect("no statement evicted while the test counted them");
    let nested: u64 = statements
        .iter()
        .filter(|s| !s.toplevel)
        .map(|s| s.calls)
        .sum();
    let notifies: u64 = statements
        .iter()
        .filter(|s| s.query.contains("pg_notify"))
        .map(|s| s.calls)
        .sum();
    let chainer = calls(&statements, "SELECT audit.audit_intake_release") * CHAINER_RELEASE;
    let wal: u64 = relations
        .iter()
        .filter(|(name, ..)| owned(name))
        .map(|(_, bytes, _)| bytes)
        .sum();
    let ceiling = kind.ceiling();
    let cost = Ceiling {
        round_trips: sent.div_euclid(REQUESTS),
        nested: nested.saturating_sub(chainer).div_euclid(REQUESTS),
        notifies: notifies.div_euclid(REQUESTS),
        wal: wal.div_euclid(REQUESTS),
    };
    let mut report = format!(
        "{}: per request {cost:?} (ceiling {ceiling:?}), {chainer} nested statements of the \
         chainer\n",
        kind.name()
    );
    for (name, bytes, records) in relations.iter().filter(|(name, ..)| owned(name)).take(16) {
        writeln!(report, "  {bytes:>9} B {records:>6} records  {name}")
            .expect("a String takes the line");
    }
    report.push_str(&ledger(&statements));
    assert!(sent > 0, "the round trips are counted");
    let kept = cost.round_trips <= ceiling.round_trips
        && cost.nested <= ceiling.nested
        && cost.notifies <= ceiling.notifies
        && cost.wal <= ceiling.wal;
    (report, kept)
}

/// A care-gated composition read stays within its ceiling.
#[tokio::test]
async fn a_care_gated_read_stays_within_its_statement_budget() {
    let (report, kept) = measured(Kind::Read).await;
    eprintln!("{report}");
    assert!(kept, "a care-gated read exceeds its budget:\n{report}");
}

/// A query narrowed to the caller's caseload stays within its ceiling.
#[tokio::test]
async fn a_caseload_query_stays_within_its_statement_budget() {
    let (report, kept) = measured(Kind::Caseload).await;
    eprintln!("{report}");
    assert!(kept, "a caseload query exceeds its budget:\n{report}");
}

/// A care relationship's commit stays within its ceiling.
#[tokio::test]
async fn a_care_relationship_commit_stays_within_its_statement_budget() {
    let (report, kept) = measured(Kind::Commit).await;
    eprintln!("{report}");
    assert!(
        kept,
        "a care relationship commit exceeds its budget:\n{report}"
    );
}

/// A care relationship's commit to the caller's team followed by her
/// caseload query stays within its ceiling: the commit's delta keeps the
/// team's caseload copy, so the query reads no census of the team.
#[tokio::test]
async fn a_care_relationship_commit_keeps_the_teams_caseload_copy() {
    let (report, kept) = measured(Kind::Recare).await;
    eprintln!("{report}");
    assert!(
        kept,
        "a care relationship commit drops the team's caseload copy:\n{report}"
    );
}
