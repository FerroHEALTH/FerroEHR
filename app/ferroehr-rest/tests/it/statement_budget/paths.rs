// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The budget of every hot path (#6007): per request, the representation
//! changes it makes, the allocations of the thread that drives it, its round
//! trips, the statements `PostgreSQL` runs nested in them and the write-ahead
//! log they write, over the assembled router with the access evaluator, the
//! audit drain and the local access log.
//!
//! The paths are the commit of a COMPOSITION in canonical JSON, canonical XML
//! and FLAT, its read in the same three formats, the EHR read, a single-EHR
//! and a population AQL query, a population `matches` over a value list
//! (#6084), a demographic population query (#6087), an ITI-81 page and a
//! party read, all over the
//! IPS composition (128 KB, 402 node rows), and the commit of that composition
//! as a CONTRIBUTION, its read at an instant, the `EHR_STATUS` commit and read,
//! the commit of a PERSON, plain and with ordinal and scale values, the
//! update and the delete of a COMPOSITION, the directory writes, and the
//! update and the delete of a PERSON and the create of an ORGANISATION,
//! GROUP, ROLE and AGENT (#6257).
//!
//! The conversions are the `trace` events under
//! [`openehr_its::conversion::TARGET`]: the codec entry points record each
//! one, and the server records the changes it makes itself
//! (`ferroehr::telemetry::conversion`). The allocations are counted on the
//! thread that polls the request, so the audit runtime's threads do not
//! count; the peak is the most the request held at once. Both are judged on
//! the lower-quartile request of the measured ones ([`Measure::low`]).
//!
//! The bytes a request moves over the database wire are counted on their own
//! run, through a relay that splits the protocol messages by statement
//! ([`super::wire`], #6244), and held to [`WIRE_BUDGETS`].
//!
//! Each figure is held to the ceiling [`BUDGETS`] names: the measured end
//! state of the core rewrite (#6001) plus a small margin. A ceiling only ever
//! comes down, on an owner decision. No openEHR spec governs the cost of a
//! request: our own design.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};

use axum::Router;
use axum::body::Body;
use http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt as _;
use tracing::Instrument as _;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

use super::{CHAINER_RELEASE, calls, counted, drained};
use crate::access_evaluator_http::{credential, ehr_with_composition};
use crate::common::{self, BASE, FHIR};

/// The measured requests of each path.
pub(super) const REQUESTS: u64 = 10;

/// The requests of each path sent before the measured ones.
pub(super) const WARM: u64 = 3;

/// The EHRs the paths spread over, each with one IPS composition.
const EHRS: usize = 5;

/// The span each measured request runs in.
const PATH_SPAN: &str = "hot_path_budget_request";

/// The prefix of the conversions that render the access record, which the
/// audit lane owns and the path's own budget leaves out.
const AUDIT: &str = "audit_";

/// The canonical JSON media type.
const JSON: &str = "application/json";
/// The canonical XML media type.
const XML: &str = "application/xml";
/// The FLAT media type (ITS-REST `Resources.md` §Simplified Formats).
const FLAT: &str = "application/openehr.wt.flat+json";

/// A hot path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Path {
    CommitJson,
    CommitXml,
    CommitFlat,
    ReadJson,
    ReadXml,
    ReadFlat,
    EhrRead,
    AqlSingle,
    AqlPopulation,
    AqlMatches,
    AqlParty,
    Iti81,
    PartyRead,
    CommitContribution,
    ReadAtTime,
    CommitStatus,
    ReadStatus,
    CommitParty,
    CommitPartyScored,
    ReadVersion,
    CommitUpdate,
    CommitDelete,
    DirectoryCreate,
    DirectoryUpdate,
    DirectoryDelete,
    PartyUpdate,
    PartyDelete,
    CommitOrganisation,
    CommitGroup,
    CommitRole,
    CommitAgent,
}

impl Path {
    pub(super) const ALL: [Self; 31] = [
        Self::CommitJson,
        Self::CommitXml,
        Self::CommitFlat,
        Self::ReadJson,
        Self::ReadXml,
        Self::ReadFlat,
        Self::EhrRead,
        Self::AqlSingle,
        Self::AqlPopulation,
        Self::AqlMatches,
        Self::AqlParty,
        Self::Iti81,
        Self::PartyRead,
        Self::CommitContribution,
        Self::ReadAtTime,
        Self::CommitStatus,
        Self::ReadStatus,
        Self::CommitParty,
        Self::CommitPartyScored,
        Self::ReadVersion,
        Self::CommitUpdate,
        Self::CommitDelete,
        Self::DirectoryCreate,
        Self::DirectoryUpdate,
        Self::DirectoryDelete,
        Self::PartyUpdate,
        Self::PartyDelete,
        Self::CommitOrganisation,
        Self::CommitGroup,
        Self::CommitRole,
        Self::CommitAgent,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::CommitJson => "commit JSON",
            Self::CommitXml => "commit XML",
            Self::CommitFlat => "commit FLAT",
            Self::ReadJson => "read JSON",
            Self::ReadXml => "read XML",
            Self::ReadFlat => "read FLAT",
            Self::EhrRead => "EHR read",
            Self::AqlSingle => "single-EHR AQL",
            Self::AqlPopulation => "population AQL",
            Self::AqlMatches => "population AQL matches",
            Self::AqlParty => "demographic AQL",
            Self::Iti81 => "ITI-81 page",
            Self::PartyRead => "party read",
            Self::CommitContribution => "contribution commit",
            Self::ReadAtTime => "read at time",
            Self::CommitStatus => "EHR_STATUS commit",
            Self::ReadStatus => "EHR_STATUS read",
            Self::CommitParty => "party commit",
            Self::CommitPartyScored => "party commit with scores",
            Self::ReadVersion => "read JSON from the version copy",
            Self::CommitUpdate => "update JSON",
            Self::CommitDelete => "delete",
            Self::DirectoryCreate => "directory create",
            Self::DirectoryUpdate => "directory update",
            Self::DirectoryDelete => "directory delete",
            Self::PartyUpdate => "party update",
            Self::PartyDelete => "party delete",
            Self::CommitOrganisation => "ORGANISATION commit",
            Self::CommitGroup => "GROUP commit",
            Self::CommitRole => "ROLE commit",
            Self::CommitAgent => "AGENT commit",
        }
    }
}

/// The ceilings of one path, per request.
struct Budget {
    path: Path,
    conversions: u64,
    allocations: u64,
    peak_bytes: u64,
    round_trips: u64,
    nested: u64,
    /// The `pg_notify` calls, top-level or nested.
    notifies: u64,
    wal: u64,
}

/// The ceilings by path, per request (#6007); the nested statements leave out
/// the chainer's rounds.
const BUDGETS: [Budget; 31] = [
    Budget {
        path: Path::CommitJson,
        conversions: 5,
        allocations: 18_029,
        peak_bytes: 1_530_778,
        round_trips: 4,
        nested: 9,
        notifies: 0,
        wal: 187_300,
    },
    Budget {
        path: Path::CommitXml,
        conversions: 5,
        allocations: 39_451,
        peak_bytes: 1_533_031,
        round_trips: 4,
        nested: 10,
        notifies: 0,
        wal: 187_629,
    },
    Budget {
        path: Path::CommitFlat,
        conversions: 5,
        allocations: 32_612,
        peak_bytes: 1_528_525,
        round_trips: 4,
        nested: 9,
        notifies: 0,
        wal: 188_099,
    },
    Budget {
        path: Path::ReadJson,
        conversions: 0,
        allocations: 1318,
        peak_bytes: 617_268,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 858,
    },
    Budget {
        path: Path::ReadXml,
        conversions: 2,
        allocations: 10_795,
        peak_bytes: 1_017_140,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 858,
    },
    Budget {
        path: Path::ReadFlat,
        conversions: 2,
        allocations: 15_512,
        peak_bytes: 1_634_407,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 858,
    },
    Budget {
        path: Path::EhrRead,
        conversions: 1,
        allocations: 1113,
        peak_bytes: 100_250,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 642,
    },
    Budget {
        path: Path::AqlSingle,
        conversions: 2,
        allocations: 2443,
        peak_bytes: 118_273,
        round_trips: 3,
        nested: 0,
        notifies: 0,
        wal: 1881,
    },
    Budget {
        path: Path::AqlPopulation,
        conversions: 2,
        allocations: 3295,
        peak_bytes: 158_823,
        round_trips: 2,
        nested: 0,
        notifies: 0,
        wal: 5178,
    },
    Budget {
        path: Path::Iti81,
        conversions: 0,
        allocations: 8962,
        peak_bytes: 624_026,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 617,
    },
    Budget {
        path: Path::PartyRead,
        conversions: 0,
        allocations: 1058,
        peak_bytes: 78_848,
        round_trips: 7,
        nested: 1,
        notifies: 0,
        wal: 1055,
    },
    Budget {
        path: Path::CommitContribution,
        conversions: 6,
        allocations: 18_282,
        peak_bytes: 1_649_934,
        round_trips: 4,
        nested: 10,
        notifies: 0,
        wal: 188_738,
    },
    Budget {
        path: Path::ReadAtTime,
        conversions: 0,
        allocations: 1329,
        peak_bytes: 701_748,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 858,
    },
    Budget {
        path: Path::CommitStatus,
        conversions: 4,
        allocations: 1529,
        peak_bytes: 147_242,
        round_trips: 5,
        nested: 7,
        notifies: 1,
        wal: 4773,
    },
    Budget {
        path: Path::ReadStatus,
        conversions: 0,
        allocations: 1182,
        peak_bytes: 102_503,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 763,
    },
    Budget {
        path: Path::CommitParty,
        conversions: 8,
        allocations: 1713,
        peak_bytes: 125_972,
        round_trips: 14,
        nested: 10,
        notifies: 1,
        wal: 6622,
    },
    Budget {
        path: Path::AqlMatches,
        conversions: 2,
        allocations: 5703,
        peak_bytes: 192_267,
        round_trips: 3,
        nested: 0,
        notifies: 0,
        wal: 5638,
    },
    Budget {
        path: Path::AqlParty,
        conversions: 2,
        allocations: 2626,
        peak_bytes: 107_199,
        round_trips: 8,
        nested: 1,
        notifies: 0,
        wal: 1064,
    },
    Budget {
        path: Path::CommitPartyScored,
        conversions: 8,
        allocations: 2200,
        peak_bytes: 140_901,
        round_trips: 14,
        nested: 11,
        notifies: 1,
        wal: 9191,
    },
    Budget {
        path: Path::ReadVersion,
        conversions: 0,
        allocations: 1324,
        peak_bytes: 162_800,
        round_trips: 1,
        nested: 1,
        notifies: 0,
        wal: 842,
    },
    Budget {
        path: Path::CommitUpdate,
        conversions: 5,
        allocations: 17_758,
        peak_bytes: 1_525_458,
        round_trips: 4,
        nested: 0,
        notifies: 0,
        wal: 186_794,
    },
    Budget {
        path: Path::CommitDelete,
        conversions: 2,
        allocations: 1544,
        peak_bytes: 150_537,
        round_trips: 4,
        nested: 0,
        notifies: 0,
        wal: 4357,
    },
    Budget {
        path: Path::DirectoryCreate,
        conversions: 5,
        allocations: 1576,
        peak_bytes: 150_606,
        round_trips: 4,
        nested: 1,
        notifies: 0,
        wal: 5882,
    },
    Budget {
        path: Path::DirectoryUpdate,
        conversions: 4,
        allocations: 1551,
        peak_bytes: 152_696,
        round_trips: 4,
        nested: 1,
        notifies: 0,
        wal: 4788,
    },
    Budget {
        path: Path::DirectoryDelete,
        conversions: 2,
        allocations: 1418,
        peak_bytes: 144_129,
        round_trips: 4,
        nested: 1,
        notifies: 0,
        wal: 4201,
    },
    Budget {
        path: Path::PartyUpdate,
        conversions: 8,
        allocations: 1954,
        peak_bytes: 129_641,
        round_trips: 12,
        nested: 4,
        notifies: 1,
        wal: 6599,
    },
    Budget {
        path: Path::PartyDelete,
        conversions: 2,
        allocations: 1568,
        peak_bytes: 112_950,
        round_trips: 12,
        nested: 4,
        notifies: 1,
        wal: 4316,
    },
    Budget {
        path: Path::CommitOrganisation,
        conversions: 8,
        allocations: 1566,
        peak_bytes: 126_183,
        round_trips: 10,
        nested: 1,
        notifies: 0,
        wal: 6710,
    },
    Budget {
        path: Path::CommitGroup,
        conversions: 8,
        allocations: 1565,
        peak_bytes: 126_027,
        round_trips: 10,
        nested: 1,
        notifies: 0,
        wal: 6689,
    },
    Budget {
        path: Path::CommitRole,
        conversions: 9,
        allocations: 2139,
        peak_bytes: 126_256,
        round_trips: 19,
        nested: 6,
        notifies: 0,
        wal: 7012,
    },
    Budget {
        path: Path::CommitAgent,
        conversions: 8,
        allocations: 1550,
        peak_bytes: 126_026,
        round_trips: 10,
        nested: 1,
        notifies: 0,
        wal: 6582,
    },
];

/// The paths in the order [`BUDGETS`] measures them, which keeps the access
/// records and parties a later path writes out of what an earlier one reads.
pub(super) fn order() -> impl Iterator<Item = Path> {
    BUDGETS.iter().map(|budget| budget.path)
}

/// The bytes each path may move over the database wire per request, both
/// directions together (#6244).
const WIRE_BUDGETS: [(Path, u64); 31] = [
    (Path::CommitJson, 201_836),
    (Path::CommitXml, 201_979),
    (Path::CommitFlat, 201_974),
    (Path::ReadJson, 131_081),
    (Path::ReadXml, 131_081),
    (Path::ReadFlat, 131_081),
    (Path::EhrRead, 1802),
    (Path::AqlSingle, 4821),
    (Path::AqlPopulation, 6391),
    (Path::Iti81, 27_438),
    (Path::PartyRead, 5524),
    (Path::CommitContribution, 202_796),
    (Path::ReadAtTime, 131_111),
    (Path::CommitStatus, 5543),
    (Path::ReadStatus, 3113),
    (Path::CommitParty, 8513),
    (Path::AqlMatches, 15_992),
    (Path::AqlParty, 11_509),
    (Path::CommitPartyScored, 10_861),
    (Path::ReadVersion, 2748),
    (Path::CommitUpdate, 202_260),
    (Path::CommitDelete, 4563),
    (Path::DirectoryCreate, 4955),
    (Path::DirectoryUpdate, 4765),
    (Path::DirectoryDelete, 4122),
    (Path::PartyUpdate, 9406),
    (Path::PartyDelete, 7957),
    (Path::CommitOrganisation, 7778),
    (Path::CommitGroup, 7608),
    (Path::CommitRole, 11_481),
    (Path::CommitAgent, 7586),
];

/// Counts, inside a [`PATH_SPAN`] span, the `sqlx::query` events (the round
/// trips) and the conversion events by their `op`.
#[derive(Clone, Default)]
struct Counter(Arc<Mutex<Counts>>);

/// What a [`Counter`] saw.
#[derive(Default)]
struct Counts {
    round_trips: u64,
    conversions: BTreeMap<String, u64>,
    /// The round trips by the opening words of their statement.
    sent: BTreeMap<String, u64>,
}

/// Reads the `op` field of a conversion event, and the statement of a
/// `sqlx::query` event.
#[derive(Default)]
struct Op(String, String);

impl Visit for Op {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "op" => value.clone_into(&mut self.0),
            "db.statement" if !value.is_empty() => value.clone_into(&mut self.1),
            "summary" if self.1.is_empty() => value.clone_into(&mut self.1),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "op" && self.0.is_empty() {
            self.0 = format!("{value:?}");
        }
    }
}

/// Whether the report lists each path's round trips by statement: set it to
/// see them, and leave it off for the budget, since reading them allocates on
/// the measured thread.
const LIST_SENT: bool = false;

/// The opening words of `statement`, its whitespace collapsed.
fn opening(statement: &str) -> String {
    statement
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(100)
        .collect()
}

impl<S> tracing_subscriber::Layer<S> for Counter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    // NOTE: our own design (#6321), the stage spans are left disabled, so this
    // budget counts what the request allocates with no stage marked.
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _: Context<'_, S>) -> bool {
        metadata.target() != ferroehr::telemetry::stage::TARGET
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let target = event.metadata().target();
        let conversion = target == openehr_its::conversion::TARGET;
        if !conversion && target != "sqlx::query" {
            return;
        }
        let Some(scope) = ctx.event_scope(event) else {
            return;
        };
        let names: Vec<&str> = scope.map(|span| span.name()).collect();
        if !names.contains(&PATH_SPAN) {
            return;
        }
        let mut counts = self.0.lock().unwrap();
        if conversion {
            let mut op = Op::default();
            event.record(&mut op);
            *counts.conversions.entry(op.0).or_default() += 1;
        } else if !names.contains(&ferroehr::db::pool::SESSION_SETUP_SPAN) {
            counts.round_trips += 1;
            // The statements are read only on request: reading them allocates on
            // the measured thread.
            if LIST_SENT {
                let mut op = Op::default();
                event.record(&mut op);
                *counts.sent.entry(opening(&op.1)).or_default() += 1;
            }
        }
    }
}

/// The allocations made while a future was polled on whichever thread
/// polled it, and the most it held at once.
#[derive(Default)]
struct Allocated {
    count: u64,
    bytes: u64,
    held: i64,
    peak: i64,
}

/// A future whose every poll is measured into an [`Allocated`].
struct Measured<'a, F> {
    inner: Pin<Box<F>>,
    into: &'a mut Allocated,
}

impl<F: Future> Future for Measured<'_, F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<F::Output> {
        let this = &mut *self;
        let mut out = Poll::Pending;
        let info = allocation_counter::measure(|| out = this.inner.as_mut().poll(cx));
        this.into.count += info.count_total;
        this.into.bytes += info.bytes_total;
        this.into.peak = this
            .into
            .peak
            .max(this.into.held + i64::try_from(info.bytes_max).unwrap());
        this.into.held += info.bytes_current;
        out
    }
}

/// One request, owned so it can be built again for each send.
pub(super) struct Call {
    method: &'static str,
    uri: String,
    user: &'static str,
    accept: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Option<(&'static str, String)>,
}

impl Call {
    pub(super) fn request(&self) -> Request<Body> {
        let mut builder = Request::builder()
            .method(self.method)
            .uri(&self.uri)
            .header(header::AUTHORIZATION, credential(self.user))
            .header(header::ACCEPT, self.accept);
        for (name, value) in &self.headers {
            builder = builder.header(*name, value);
        }
        match &self.body {
            Some((content_type, body)) => builder
                .header(header::CONTENT_TYPE, *content_type)
                .body(Body::from(body.clone())),
            None => builder.body(Body::empty()),
        }
        .unwrap()
    }
}

/// Sends `request` and reads its whole answer.
pub(super) async fn exchange(
    app: &Router,
    request: Request<Body>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(request).await.expect("a response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body")
        .to_bytes();
    (status, headers, body.to_vec())
}

/// Sends `call` unmeasured; its status, headers and body.
async fn plain(app: &Router, call: &Call) -> (StatusCode, HeaderMap, String) {
    let (status, headers, body) = exchange(app, call.request()).await;
    (status, headers, String::from_utf8(body).unwrap())
}

/// The fixtures the paths read and write.
pub(super) struct World {
    ehrs: Vec<(String, String)>,
    xml: String,
    flat: String,
    template_id: String,
    party: String,
    person: String,
    /// The PERSON with two ordinals and two scales in its details.
    scored: String,
    /// An instant after every fixture commit, before which the ITI-81 page
    /// reads a fixed set of records.
    at: String,
    /// The `EHR_STATUS` a status commit writes, without its `uid`.
    status: String,
    /// The current `EHR_STATUS` version uid of each EHR, which the next status
    /// commit names in `If-Match`.
    status_uids: Mutex<Vec<String>>,
    /// The EHRs whose settings make a read depend on the object, so it is
    /// judged on its own row and served from the version copy, each with the
    /// version uid of its composition; built right before their path.
    judged: Mutex<Vec<(String, String)>>,
    /// The current version uid of each EHR's composition, which the next
    /// update names in `If-Match`.
    update_uids: Mutex<Vec<String>>,
    /// The compositions the deletes remove, each with its EHR and the version
    /// uid the delete names; built right before their path.
    deletable: Mutex<Vec<(String, String)>>,
    /// The EHRs the directory paths write, one per request, each with the
    /// object id of its composition, which its FOLDER references, and its
    /// directory's current version uid; built right before their paths.
    directories: Mutex<Vec<(String, String, String)>>,
    /// The PERSONs the party updates and deletes write, one per request, each
    /// with its current version uid; built right before their paths.
    parties: Mutex<Vec<String>>,
}

/// The `ETag` value of a response, unquoted.
fn etag(headers: &HeaderMap) -> String {
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("an ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

/// The versioned-object id an `ETag` names.
fn object_id(headers: &HeaderMap) -> String {
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("an ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .split("::")
        .next()
        .expect("an object id")
        .to_owned()
}

impl World {
    pub(super) async fn build(app: &Router) -> Self {
        let mut ehrs = Vec::new();
        for _ in 0..EHRS {
            ehrs.push(ehr_with_composition(app).await);
        }
        let (ehr, vo) = &ehrs[0];
        let read = |accept| Call {
            method: "GET",
            uri: format!("{BASE}/ehr/{ehr}/composition/{vo}"),
            user: "root",
            accept,
            headers: Vec::new(),
            body: None,
        };
        let (status, _, xml) = plain(app, &read(XML)).await;
        assert_eq!(status, StatusCode::OK, "the XML read: {xml}");
        let (status, _, flat) = plain(app, &read(FLAT)).await;
        assert_eq!(status, StatusCode::OK, "the FLAT read: {flat}");
        let template_id =
            common::ips_canonical_composition()["archetype_details"]["template_id"]["value"]
                .as_str()
                .expect("the IPS template id")
                .to_owned();
        let person = demographic_fixture("person");
        let (status, headers, body) = plain(
            app,
            &Call {
                method: "POST",
                uri: format!("{BASE}/demographic/person"),
                user: "root",
                accept: JSON,
                headers: Vec::new(),
                body: Some((JSON, person.clone())),
            },
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "the person: {body}");
        let mut status_uids = Vec::new();
        let mut status_body = String::new();
        for (ehr, _) in &ehrs {
            let (code, headers, body) = plain(
                app,
                &Call {
                    method: "GET",
                    uri: format!("{BASE}/ehr/{ehr}/ehr_status"),
                    user: "root",
                    accept: JSON,
                    headers: Vec::new(),
                    body: None,
                },
            )
            .await;
            assert_eq!(code, StatusCode::OK, "the EHR_STATUS: {body}");
            status_uids.push(etag(&headers));
            let mut value: Value = serde_json::from_str(&body).expect("the EHR_STATUS");
            value
                .as_object_mut()
                .expect("an object")
                .shift_remove("uid");
            status_body = value.to_string();
        }
        Self {
            xml,
            flat,
            template_id,
            party: object_id(&headers),
            scored: scored(&person),
            person,
            at: jiff::Timestamp::now().to_string(),
            update_uids: Mutex::new(
                ehrs.iter()
                    .map(|(_, vo)| format!("{vo}::{}::1", system_id(&status_uids)))
                    .collect(),
            ),
            ehrs,
            status: status_body,
            status_uids: Mutex::new(status_uids),
            judged: Mutex::new(Vec::new()),
            deletable: Mutex::new(Vec::new()),
            directories: Mutex::new(Vec::new()),
            parties: Mutex::new(Vec::new()),
        }
    }

    /// Commits the PERSONs [`Path::PartyUpdate`] updates and
    /// [`Path::PartyDelete`] then deletes, one per request.
    async fn parties(&self, app: &Router) {
        let mut parties = Vec::new();
        for _ in 0..WARM + REQUESTS {
            let (status, headers, body) =
                plain(app, &Self::party_commit(self.person.clone())).await;
            assert_eq!(status, StatusCode::CREATED, "a person to update: {body}");
            parties.push(etag(&headers));
        }
        *self.parties.lock().unwrap() = parties;
    }

    /// The `n`th party update or delete, of the `n`th PERSON
    /// [`Self::parties`] committed, an update naming its `n` in the name so
    /// each one stores new content.
    fn party_write(&self, path: Path, n: usize) -> Call {
        let tip = self.parties.lock().unwrap()[n].clone();
        if path == Path::PartyDelete {
            return Call {
                method: "DELETE",
                uri: format!("{BASE}/demographic/person/{tip}"),
                user: "root",
                accept: JSON,
                headers: Vec::new(),
                body: None,
            };
        }
        let vo = tip.split("::").next().expect("an object id");
        let mut body: Value = serde_json::from_str(&self.person).expect("the person fixture");
        body["name"]["value"] = Value::from(format!("person {n}"));
        Call {
            method: "PUT",
            uri: format!("{BASE}/demographic/person/{vo}"),
            user: "root",
            accept: JSON,
            headers: vec![
                ("Prefer", "return=minimal".to_owned()),
                ("If-Match", format!("\"{tip}\"")),
            ],
            body: Some((JSON, body.to_string())),
        }
    }

    /// The commit of `path`'s party kind from its fixture: an ORGANISATION,
    /// GROUP or AGENT as the corpus holds it, a ROLE performed by the world's
    /// PERSON.
    fn kind_commit(&self, path: Path) -> Call {
        let kind = match path {
            Path::CommitOrganisation => "organisation",
            Path::CommitGroup => "group",
            Path::CommitRole => "role",
            _ => "agent",
        };
        let mut body: Value = serde_json::from_str(&demographic_fixture(kind)).expect("a fixture");
        if kind == "role" {
            body["performer"]["id"]["value"] = Value::from(self.party.clone());
        }
        Call {
            method: "POST",
            uri: format!("{BASE}/demographic/{kind}"),
            user: "root",
            accept: JSON,
            headers: vec![("Prefer", "return=minimal".to_owned())],
            body: Some((JSON, body.to_string())),
        }
    }

    /// Builds what `path` reads or writes besides the shared fixtures, right
    /// before its requests.
    pub(super) async fn prepare(
        &self,
        app: &Router,
        svc: &ferroehr::service::FerroEhrService,
        path: Path,
    ) {
        match path {
            Path::ReadVersion => self.judge_on_objects(app, svc).await,
            Path::CommitDelete => self.deletable(app).await,
            Path::DirectoryCreate => self.directories(app).await,
            Path::PartyUpdate => self.parties(app).await,
            _ => {}
        }
    }

    /// Sends the [`WARM`] requests of `path` that go before its measured ones.
    pub(super) async fn warm(&self, app: &Router, path: Path) {
        for n in 0..WARM {
            let n = usize::try_from(n).unwrap();
            let (status, headers, body) = plain(app, &self.call(path, n)).await;
            assert!(status.is_success(), "{}: {status} {body}", path.name());
            self.observe(path, n, &headers);
        }
    }

    /// Builds the EHRs of the directory paths, one per request, each with a
    /// composition its FOLDER references.
    async fn directories(&self, app: &Router) {
        let mut directories = Vec::new();
        for _ in 0..WARM + REQUESTS {
            let (ehr, vo) = ehr_with_composition(app).await;
            directories.push((ehr, vo, String::new()));
        }
        *self.directories.lock().unwrap() = directories;
    }

    /// The `n`th request of a directory path: a create, an update or a delete
    /// of the directory of the `n`th EHR [`Self::directories`] built.
    fn directory(&self, path: Path, n: usize) -> Call {
        let (ehr, vo, tip) = self.directories.lock().unwrap()[n].clone();
        let folder = serde_json::json!({
            "_type": "FOLDER",
            "name": { "_type": "DV_TEXT", "value": format!("directory {n}") },
            "archetype_node_id": "openEHR-EHR-FOLDER.directory.v1",
            "archetype_details": { "_type": "ARCHETYPED",
                "archetype_id": { "_type": "ARCHETYPE_ID",
                    "value": "openEHR-EHR-FOLDER.directory.v1" },
                "rm_version": "1.1.0" },
            "items": [{ "_type": "OBJECT_REF", "namespace": "local", "type": "COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": vo } }],
        });
        let mut headers = vec![("Prefer", "return=minimal".to_owned())];
        if path != Path::DirectoryCreate {
            headers.push(("If-Match", format!("\"{tip}\"")));
        }
        let (method, body) = match path {
            Path::DirectoryCreate => ("POST", Some((JSON, folder.to_string()))),
            Path::DirectoryUpdate => ("PUT", Some((JSON, folder.to_string()))),
            _ => ("DELETE", None),
        };
        Call {
            method,
            uri: format!("{BASE}/ehr/{ehr}/directory"),
            user: "root",
            accept: JSON,
            headers,
            body,
        }
    }

    /// Builds the EHRs of [`Path::ReadVersion`]: each restricts an object it
    /// does not hold, so its settings name an object and a read of its
    /// composition by version id is judged on the object's own row (#5959)
    /// and read from the version copy (#5778).
    async fn judge_on_objects(&self, app: &Router, svc: &ferroehr::service::FerroEhrService) {
        let mut judged = Vec::new();
        for _ in 0..EHRS {
            let (ehr, vo) = ehr_with_composition(app).await;
            let elsewhere = uuid::Uuid::now_v7().to_string();
            let restriction = common::access_directive("restriction", &[elsewhere.as_str()]);
            common::commit_access_settings(
                svc,
                ehr.parse().expect("an EHR id"),
                common::directive_settings(&[restriction]),
            )
            .await;
            let version = format!("{vo}::{}::1", self.system_id());
            judged.push((ehr, version));
        }
        *self.judged.lock().unwrap() = judged;
    }

    /// The system id the server stamps into the version uids it writes.
    fn system_id(&self) -> String {
        system_id(&self.status_uids.lock().unwrap())
    }

    /// Commits the compositions [`Path::CommitDelete`] removes, one per
    /// request, spread over the EHRs.
    async fn deletable(&self, app: &Router) {
        let mut deletable = Vec::new();
        for n in 0..usize::try_from(WARM + REQUESTS).unwrap() {
            let (ehr, _) = &self.ehrs[n % self.ehrs.len()];
            let (status, headers, body) = plain(
                app,
                &Call {
                    method: "POST",
                    uri: format!("{BASE}/ehr/{ehr}/composition"),
                    user: "root",
                    accept: JSON,
                    headers: vec![("Prefer", "return=minimal".to_owned())],
                    body: Some((JSON, common::ips_canonical_composition().to_string())),
                },
            )
            .await;
            assert_eq!(
                status,
                StatusCode::CREATED,
                "a composition to delete: {body}"
            );
            deletable.push((ehr.clone(), etag(&headers)));
        }
        *self.deletable.lock().unwrap() = deletable;
    }

    /// Records what the answer to the `n`th request of `path` changed.
    pub(super) fn observe(&self, path: Path, n: usize, headers: &HeaderMap) {
        match path {
            Path::CommitStatus => {
                self.status_uids.lock().unwrap()[n % self.ehrs.len()] = etag(headers);
            }
            Path::CommitUpdate => {
                self.update_uids.lock().unwrap()[n % self.ehrs.len()] = etag(headers);
            }
            Path::DirectoryCreate | Path::DirectoryUpdate => {
                self.directories.lock().unwrap()[n].2 = etag(headers);
            }
            Path::PartyUpdate => {
                self.parties.lock().unwrap()[n] = etag(headers);
            }
            _ => {}
        }
    }

    /// The `n`th delete, of a composition [`Self::deletable`] committed.
    fn delete(&self, n: usize) -> Call {
        let (ehr, version) = self.deletable.lock().unwrap()[n].clone();
        Call {
            method: "DELETE",
            uri: format!("{BASE}/ehr/{ehr}/composition/{version}"),
            user: "root",
            accept: JSON,
            headers: Vec::new(),
            body: None,
        }
    }

    /// The `n`th update, of the composition of the EHR `ehr`, with a start
    /// time of its own so each one stores new content.
    fn update(&self, ehr: &str, n: usize) -> Call {
        let tip = self.update_uids.lock().unwrap()[n % self.ehrs.len()].clone();
        let vo = tip.split("::").next().expect("an object id");
        let mut body = common::ips_canonical_composition();
        body.as_object_mut().expect("an object").shift_remove("uid");
        body["context"]["start_time"]["value"] =
            Value::from(format!("2021-12-03T17:34:{:02}.849379+01:00", n % 60));
        Call {
            method: "PUT",
            uri: format!("{BASE}/ehr/{ehr}/composition/{vo}"),
            user: "root",
            accept: JSON,
            headers: vec![
                ("Prefer", "return=minimal".to_owned()),
                ("If-Match", format!("\"{tip}\"")),
            ],
            body: Some((JSON, body.to_string())),
        }
    }

    /// The `n`th status commit, to the EHR `ehr`.
    fn status_commit(&self, ehr: &str, n: usize) -> Call {
        let tip = self.status_uids.lock().unwrap()[n % self.ehrs.len()].clone();
        Call {
            method: "PUT",
            uri: format!("{BASE}/ehr/{ehr}/ehr_status"),
            user: "root",
            accept: JSON,
            headers: vec![
                ("Prefer", "return=minimal".to_owned()),
                ("If-Match", format!("\"{tip}\"")),
            ],
            body: Some((JSON, self.status.clone())),
        }
    }

    /// The `n`th request of `path`.
    pub(super) fn call(&self, path: Path, n: usize) -> Call {
        let (ehr, vo) = &self.ehrs[n % self.ehrs.len()];
        let minimal = vec![("Prefer", "return=minimal".to_owned())];
        let commit = |content_type, body: String, mut headers: Vec<(&'static str, String)>| {
            headers.extend(minimal.clone());
            Call {
                method: "POST",
                uri: format!("{BASE}/ehr/{ehr}/composition"),
                user: "root",
                accept: JSON,
                headers,
                body: Some((content_type, body)),
            }
        };
        let get = |uri: String, user, accept| Call {
            method: "GET",
            uri,
            user,
            accept,
            headers: Vec::new(),
            body: None,
        };
        let composition = format!("{BASE}/ehr/{ehr}/composition/{vo}");
        let aql = |body: Value| Call {
            method: "POST",
            uri: format!("{BASE}/query/aql"),
            user: "nurse",
            accept: JSON,
            headers: Vec::new(),
            body: Some((JSON, body.to_string())),
        };
        match path {
            Path::CommitJson => commit(
                JSON,
                common::ips_canonical_composition().to_string(),
                Vec::new(),
            ),
            Path::CommitXml => commit(XML, self.xml.clone(), Vec::new()),
            Path::CommitFlat => commit(
                FLAT,
                self.flat.clone(),
                vec![("openehr-template-id", self.template_id.clone())],
            ),
            Path::ReadJson => get(composition, "nurse", JSON),
            Path::ReadXml => get(composition, "nurse", XML),
            Path::ReadFlat => get(composition, "nurse", FLAT),
            Path::EhrRead => get(format!("{BASE}/ehr/{ehr}"), "nurse", JSON),
            Path::AqlSingle => aql(serde_json::json!({
                "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = $ehr",
                "query_parameters": { "ehr": ehr },
            })),
            Path::AqlPopulation => {
                aql(serde_json::json!({ "q": "SELECT e/ehr_id/value FROM EHR e LIMIT 20" }))
            }
            Path::AqlMatches => aql(serde_json::json!({
                "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
                      CONTAINS ELEMENT el[at0020] WHERE el/value/value \
                      matches {'Medication item 83', $item}",
                "query_parameters": { "item": "Medication item 84" },
            })),
            Path::AqlParty => aql(serde_json::json!({
                "q": "SELECT p/uid/value, p/name/value FROM PERSON p LIMIT 20",
            })),
            // The records the fixtures left, which every request and every run reads the same
            // (#6366): the page's own records and the paths before it land after `at`.
            Path::Iti81 => get(
                format!(
                    "{FHIR}/AuditEvent?date=ge2026-01-01&date=le{}&_count=20",
                    self.at
                ),
                "root",
                "application/fhir+json",
            ),
            Path::PartyRead => get(
                format!("{BASE}/demographic/person/{}", self.party),
                "root",
                JSON,
            ),
            Path::CommitContribution => Call {
                method: "POST",
                uri: format!("{BASE}/ehr/{ehr}/contribution"),
                user: "root",
                accept: JSON,
                headers: minimal.clone(),
                body: Some((JSON, contribution().to_string())),
            },
            Path::ReadAtTime => get(
                format!("{composition}?version_at_time={}", self.at),
                "nurse",
                JSON,
            ),
            Path::CommitStatus => self.status_commit(ehr, n),
            Path::ReadStatus => get(format!("{BASE}/ehr/{ehr}/ehr_status"), "nurse", JSON),
            Path::CommitUpdate => self.update(ehr, n),
            Path::CommitDelete => self.delete(n),
            Path::DirectoryCreate | Path::DirectoryUpdate | Path::DirectoryDelete => {
                self.directory(path, n)
            }
            Path::ReadVersion => self.read_version(n),
            _ => self.party_call(path, n),
        }
    }

    /// The `n`th request of a party path.
    fn party_call(&self, path: Path, n: usize) -> Call {
        match path {
            Path::CommitParty => Self::party_commit(self.person.clone()),
            Path::CommitPartyScored => Self::party_commit(self.scored.clone()),
            Path::PartyUpdate | Path::PartyDelete => self.party_write(path, n),
            _ => self.kind_commit(path),
        }
    }

    /// The `n`th read by version id, of a composition of the EHRs
    /// [`Self::judge_on_objects`] built.
    fn read_version(&self, n: usize) -> Call {
        let (ehr, version) = self.judged.lock().unwrap()[n % EHRS].clone();
        Call {
            method: "GET",
            uri: format!("{BASE}/ehr/{ehr}/composition/{version}"),
            user: "nurse",
            accept: JSON,
            headers: Vec::new(),
            body: None,
        }
    }

    /// A commit of the `PERSON` `body`.
    fn party_commit(body: String) -> Call {
        Call {
            method: "POST",
            uri: format!("{BASE}/demographic/person"),
            user: "root",
            accept: JSON,
            headers: vec![("Prefer", "return=minimal".to_owned())],
            body: Some((JSON, body)),
        }
    }
}

/// The `v1` demographic fixture of the party `kind` in the corpus.
fn demographic_fixture(kind: &str) -> String {
    std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../corpus/fixtures/demographic/{kind}.v1.json")),
    )
    .expect("the demographic fixture")
}

/// The system id in the version uids `uids`.
fn system_id(uids: &[String]) -> String {
    uids[0].split("::").nth(1).expect("a system id").to_owned()
}

/// `person` with details holding two `DV_ORDINAL` and two `DV_SCALE`
/// values, whose numbers `idx_node_hot_value_number` indexes (#6090).
fn scored(person: &str) -> String {
    let mut body: Value = serde_json::from_str(person).expect("the person fixture");
    let symbol = |code: &str| {
        serde_json::json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": code } })
    };
    let element = |at: &str, value: Value| {
        serde_json::json!({ "_type": "ELEMENT", "archetype_node_id": at,
            "name": { "_type": "DV_TEXT", "value": at }, "value": value })
    };
    body["details"] = serde_json::json!({
        "_type": "ITEM_TREE",
        "archetype_node_id": "at0100",
        "name": { "_type": "DV_TEXT", "value": "details" },
        "items": [
            element("at0101", serde_json::json!({ "_type": "DV_ORDINAL", "value": 1, "symbol": symbol("at0111") })),
            element("at0102", serde_json::json!({ "_type": "DV_ORDINAL", "value": 3, "symbol": symbol("at0113") })),
            element("at0103", serde_json::json!({ "_type": "DV_SCALE", "value": 2.5, "symbol": symbol("at0122") })),
            element("at0104", serde_json::json!({ "_type": "DV_SCALE", "value": 4.5, "symbol": symbol("at0124") })),
        ],
    });
    body.to_string()
}

/// A CONTRIBUTION that creates the IPS composition.
fn contribution() -> Value {
    let coded = |code: &str, value: &str| {
        serde_json::json!({ "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": code } })
    };
    let committer = serde_json::json!({ "_type": "PARTY_IDENTIFIED",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": ferroehr::service::DEFAULT_SYSTEM_ID, "id": "root" }] });
    serde_json::json!({
        "_type": "CONTRIBUTION",
        "versions": [ {
            "_type": "ORIGINAL_VERSION",
            "commit_audit": { "change_type": coded("249", "creation"), "committer": committer },
            "lifecycle_state": coded("532", "complete"),
            "data": common::ips_canonical_composition(),
        } ],
        "audit": { "change_type": coded("249", "creation"), "committer": committer },
    })
}

/// What one path measured over [`REQUESTS`] requests.
#[derive(Default)]
struct Measure {
    conversions: BTreeMap<String, u64>,
    /// The allocations, the bytes allocated and the peak of each request.
    each: Vec<Allocated>,
    round_trips: u64,
    nested: u64,
    notifies: u64,
    /// The row records of the lower-quartile request.
    wal: u64,
    /// The row records of each request and of the window by relation, with
    /// the chain heads' the figure leaves out.
    wal_ledger: String,
    /// The statements `PostgreSQL` ran nested, and the chainer's rounds the
    /// count leaves out.
    statements: String,
    /// The bytes each request moved over the database wire.
    wire_each: Vec<u64>,
    /// The bytes the window moved over the database wire, by statement.
    wire: super::wire::Ledger,
}

impl Measure {
    /// The lower-quartile request's `figure`, the third smallest of ten.
    ///
    /// A request handed a pool connection whose buffers have not grown to
    /// the path's messages yet, or that opens one, grows them inside its
    /// window, 8 KiB a step, with the allocations around it: on CI the party
    /// read made 974 allocations with a peak of 72,870 bytes on four requests
    /// of ten and up to 1,234 and 101,773 on the others, and the demographic
    /// query 2,543 against up to 2,682, so the median crossed a ceiling
    /// whenever half a window met such connections. Which requests do depends
    /// on what the pool hands out; what a request itself allocates more,
    /// every request allocates more, so the lower quartile still moves with
    /// it (#6238).
    fn low(&self, figure: impl Fn(&Allocated) -> u64) -> u64 {
        // NOTE: our own design (#6007), a connection the pool opens inside a request is the
        // pool's work, as its session setup is for the round trips (#5899).
        self.ranked(figure, 4)
    }

    /// The `figure` of the request at `1 / of` of the window, smallest first.
    fn ranked(&self, figure: impl Fn(&Allocated) -> u64, of: usize) -> u64 {
        let mut figures: Vec<u64> = self.each.iter().map(figure).collect();
        figures.sort_unstable();
        figures
            .get(figures.len().div_euclid(of))
            .copied()
            .unwrap_or(0)
    }

    /// The allocations and the peak of each request, for the report.
    fn requests(&self) -> Vec<(u64, i64, u64)> {
        self.each
            .iter()
            .map(|a| (a.count, a.peak, a.bytes))
            .collect()
    }

    /// The conversions of the request's own body and answer, per request.
    fn conversions(&self) -> u64 {
        self.of(|op| !op.starts_with(AUDIT))
    }

    /// The renders of the request's access record, per request.
    fn audit(&self) -> u64 {
        self.of(|op| op.starts_with(AUDIT))
    }

    fn of(&self, keep: impl Fn(&str) -> bool) -> u64 {
        self.conversions
            .iter()
            .filter(|(op, _)| keep(op))
            .map(|(_, n)| n)
            .sum::<u64>()
            .div_euclid(REQUESTS)
    }
}

/// Whether `relation` is a seal or owing state of either log, which a
/// chainer pass writes once whatever it seals, so how often depends on how
/// the passes fall among the requests.
fn head_relation(relation: &str) -> bool {
    super::wal::pass_relation(relation)
}

/// The row records of the lower-quartile request, and the ledger of the
/// window: `marks` are the WAL positions before the first request and after
/// each one, taken once the chainer had moved that request's entries.
///
/// The figure is what [`super::wal`] counts for an access-log entry: the
/// records a statement writes for each row of the test's own database, read
/// with `pg_walinspect`. Page maintenance and the records of the chain heads
/// are left out, since both fall on a request by timing (#6035).
///
/// A low request is judged, the third smallest of ten, because timing still
/// moves single requests. An update that finds no room on its row's page
/// writes the whole row, a lock record and an entry in each index, and
/// whether it does depends on what pruning could clear, so on the oldest
/// transaction open on the shared server: beside the other budget tests a
/// window of ten `EHR_STATUS` commits wrote 1,550 bytes more. An index insert
/// that splits its page is logged as the split and leaves the count. What a
/// request itself writes more, every request writes more.
async fn row_records(pool: &PgPool, marks: &[String]) -> (u64, String) {
    let (Some(from), Some(to)) = (marks.first(), marks.last()) else {
        return (0, String::new());
    };
    super::wal::flushed(pool, to).await;
    let mut each = Vec::new();
    for pair in marks.windows(2) {
        let [before, after] = pair else { continue };
        each.push(
            super::wal::by_relation(pool, before, after)
                .await
                .into_iter()
                .filter(|(relation, _, _)| !head_relation(relation))
                .map(|(_, bytes, _)| bytes)
                .sum::<u64>(),
        );
    }
    let mut ledger = format!("  each request: {each:?}\n");
    for (relation, bytes, records) in super::wal::by_relation(pool, from, to).await {
        let head = head_relation(&relation);
        writeln!(
            ledger,
            "  {bytes:>8} B {records:>5}  {relation}{}",
            if head {
                "  (a chain head, left out)"
            } else {
                ""
            }
        )
        .unwrap();
    }
    writeln!(
        ledger,
        "  {} rows updated in place",
        updates_in_place(pool, from, to).await
    )
    .unwrap();
    each.sort_unstable();
    let judged = each.get(each.len().div_euclid(4)).copied().unwrap_or(0);
    (judged, ledger)
}

/// The rows the window updated in place, outside the chain heads and the
/// catalogs.
///
/// An update is the one row record whose size is not the statement's alone:
/// it is short while the new version fits the old one's page, and where it
/// does not it carries the whole row, a lock record before it and an entry in
/// each index. Which it is depends on the room pruning has left on the page,
/// and so on how long the oldest transaction of the server has been open. The
/// report prints the count so a path's ceiling can be read against it.
async fn updates_in_place(pool: &PgPool, from: &str, to: &str) -> u64 {
    let updates: i64 = sqlx::query_scalar(
        "SELECT /* statement_budget_own */ count(DISTINCT b.start_lsn) \
           FROM pg_get_wal_block_info($1::pg_lsn, $2::pg_lsn) b \
           JOIN pg_class c ON pg_relation_filenode(c.oid) = b.relfilenode \
          WHERE b.reldatabase = (SELECT oid FROM pg_database WHERE datname = current_database()) \
            AND b.resource_manager = 'Heap' \
            AND split_part(b.record_type, '+', 1) IN ('UPDATE', 'HOT_UPDATE') \
            AND c.relkind = 'r' \
            AND c.relname NOT IN ('access_log_seal_state', 'audit_event_seal_state', \
                                  'delivery_frontier') \
            AND c.relnamespace NOT IN \
                ('pg_catalog'::regnamespace, 'information_schema'::regnamespace)",
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await
    .expect("pg_walinspect reads");
    u64::try_from(updates).expect("a count")
}

/// Measures [`REQUESTS`] requests of `path`.
async fn measure(
    app: &Router,
    pool: &PgPool,
    wire: Option<&super::wire::Wire>,
    world: &World,
    counter: &Counter,
    path: Path,
) -> Measure {
    world.warm(app, path).await;
    drained(pool).await;
    let evicted = testkit::statements::statement_window(pool)
        .await
        .expect("the statement counts reset");
    *counter.0.lock().unwrap() = Counts::default();
    let mut marks = vec![super::wal::insert_lsn(pool).await];
    let mut out = Measure::default();
    if let Some(wire) = wire {
        drop(wire.take());
    }
    for n in 0..REQUESTS {
        let n = usize::try_from(WARM + n).unwrap();
        let call = world.call(path, n);
        let mut allocated = Allocated::default();
        let (status, headers, body) = Measured {
            inner: Box::pin(
                exchange(app, call.request()).instrument(tracing::info_span!(PATH_SPAN)),
            ),
            into: &mut allocated,
        }
        .await;
        assert!(
            status.is_success(),
            "{}: {status} {}",
            path.name(),
            String::from_utf8_lossy(&body)
        );
        world.observe(path, n, &headers);
        out.each.push(allocated);
        drained(pool).await;
        marks.push(super::wal::insert_lsn(pool).await);
        if let Some(wire) = wire {
            let moved = wire.take();
            out.wire_each.push(moved.total().bytes());
            out.wire.merge(&moved);
        }
    }
    let statements = counted(pool).await;
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
    let counts = std::mem::take(&mut *counter.0.lock().unwrap());
    out.conversions = counts.conversions;
    out.round_trips = counts.round_trips.div_euclid(REQUESTS);
    out.nested = nested.saturating_sub(chainer).div_euclid(REQUESTS);
    out.notifies = notifies.div_euclid(REQUESTS);
    out.statements = sent_ledger(&counts.sent) + &nested_ledger(&statements, nested, chainer);
    (out.wal, out.wal_ledger) = row_records(pool, &marks).await;
    testkit::statements::assert_statements_kept(pool, evicted)
        .await
        .expect("no statement evicted while the test counted them");
    out
}

/// The round trips of a window by their statement, per request.
fn sent_ledger(sent: &BTreeMap<String, u64>) -> String {
    let mut out = format!("round trips by statement over {REQUESTS} requests:\n");
    for (statement, n) in sent {
        writeln!(out, "  {n:>5}  {statement}").unwrap();
    }
    out
}

/// The nested statements of a window, and what the chainer's rounds take
/// out of their count.
fn nested_ledger(statements: &[super::Counted], nested: u64, chainer: u64) -> String {
    let mut out = format!("{nested} nested, {chainer} of them the chainer's\n");
    for s in statements {
        let chain = s.toplevel && s.query.contains("_chain_");
        if !s.toplevel || chain {
            let level = if s.toplevel { "top" } else { "nested" };
            let text: String = s.query.chars().take(140).collect();
            writeln!(out, "  {level:>6} {:>4}  {text}", s.calls).unwrap();
        }
    }
    out
}

/// The median of `figures`.
fn median(figures: &[u64]) -> u64 {
    let mut figures = figures.to_vec();
    figures.sort_unstable();
    figures
        .get(figures.len().div_euclid(2))
        .copied()
        .unwrap_or(0)
}

/// The database wire of a window by statement, per request: calls, bytes
/// sent and bytes received.
fn wire_ledger(m: &Measure) -> String {
    let total = m.wire.total();
    let mut out = format!(
        "database wire per request (each request {:?}): {} sent, {} received; \
         by statement, the calls over {REQUESTS} requests and the bytes per request:\n",
        m.wire_each,
        total.sent.div_euclid(REQUESTS),
        total.received.div_euclid(REQUESTS),
    );
    let mut lines: Vec<_> = m
        .wire
        .own
        .iter()
        .map(|line| ("", line))
        .chain(m.wire.background.iter().map(|line| ("(background) ", line)))
        .collect();
    lines.sort_by_key(|(_, (_, tally))| std::cmp::Reverse(tally.bytes()));
    for (apart, (statement, tally)) in lines {
        let text: String = apart.chars().chain(statement.chars()).take(160).collect();
        writeln!(
            out,
            "  {:>5} calls {:>8} B sent {:>8} B received  {text}",
            tally.calls,
            tally.sent.div_euclid(REQUESTS),
            tally.received.div_euclid(REQUESTS),
        )
        .unwrap();
    }
    out
}

/// Whether `measured` passes `ceiling`, and the cell that reports it.
fn judged(measured: u64, ceiling: u64) -> (bool, String) {
    (measured <= ceiling, format!("{measured} (<= {ceiling})"))
}

/// Whether the allocation ceilings gate this build: they are the figures
/// Linux CI measures, and the same paths allocate a few more times elsewhere.
// NOTE: #6338 chose Linux-only allocation gates; other platforms print the figures.
const ALLOCATIONS_GATED: bool = cfg!(target_os = "linux");

/// [`judged`] for an allocation count, which passes off Linux and reports
/// the measured figure beside its ceiling there.
fn judged_allocations(measured: u64, ceiling: u64) -> (bool, String) {
    if ALLOCATIONS_GATED {
        judged(measured, ceiling)
    } else {
        (
            true,
            format!("{measured} (ceiling {ceiling}, gated on Linux)"),
        )
    }
}

/// Measures every path of [`Path::ALL`] in the order of [`BUDGETS`] over the
/// router `app`, counting the database wire through `wire` when one is given.
async fn measure_all(
    app: &Router,
    pool: &PgPool,
    svc: &ferroehr::service::FerroEhrService,
    wire: Option<&super::wire::Wire>,
    counter: &Counter,
) -> Vec<(&'static Budget, Measure)> {
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_walinspect")
        .execute(pool)
        .await
        .expect("pg_walinspect: a contrib module the server ships");
    super::wal::kept(pool).await;
    let world = World::build(app).await;
    let mut measured = Vec::with_capacity(BUDGETS.len());
    for budget in &BUDGETS {
        let path = budget.path;
        world.prepare(app, svc, path).await;
        let m = Box::pin(measure(app, pool, wire, &world, counter, path)).await;
        measured.push((budget, m));
    }
    measured
}

/// Every hot path stays within its budget, and the report names what each
/// one measured.
#[tokio::test]
async fn every_hot_path_stays_within_its_budget() {
    let counter = Counter::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(counter.clone()));
    // The budgets count what a release build does, without the debug reference cells.
    ferroehr::aql::sql::without_reference_cells();
    let (_db, pool, svc, app) = Box::pin(super::served()).await;
    let measured = Box::pin(measure_all(&app, &pool, &svc, None, &counter)).await;
    let mut report = String::from(
        "path | conversions | audit renders | allocations | alloc KiB | peak KiB | round trips | nested | notifies | WAL B\n",
    );
    let mut ledger = String::new();
    let mut statements = String::new();
    let mut over = Vec::new();
    for (budget, m) in &measured {
        let path = budget.path;
        let allocations = m.low(|a| a.count);
        let kib = m.low(|a| a.bytes).div_euclid(1024);
        let peak = m.low(|a| u64::try_from(a.peak.max(0)).unwrap());
        let cells = [
            ("conversions", judged(m.conversions(), budget.conversions)),
            (
                "allocations",
                judged_allocations(allocations, budget.allocations),
            ),
            ("peak bytes", judged(peak, budget.peak_bytes)),
            ("round trips", judged(m.round_trips, budget.round_trips)),
            ("nested statements", judged(m.nested, budget.nested)),
            ("notifies", judged(m.notifies, budget.notifies)),
            ("WAL bytes", judged(m.wal, budget.wal)),
        ];
        for (what, (ok, cell)) in &cells {
            if !ok {
                over.push(format!("{}: {what} {cell}", path.name()));
            }
        }
        writeln!(
            report,
            "{} | {} | {} | {} | {kib} | {} | {} | {} | {} | {}",
            path.name(),
            cells[0].1.1,
            m.audit(),
            cells[1].1.1,
            peak.div_euclid(1024),
            cells[3].1.1,
            cells[4].1.1,
            cells[5].1.1,
            cells[6].1.1,
        )
        .unwrap();
        let ops: Vec<String> = m
            .conversions
            .iter()
            .map(|(op, n)| format!("{op}={}", n.div_euclid(REQUESTS)))
            .collect();
        writeln!(ledger, "{}: {}", path.name(), ops.join(", ")).unwrap();
        writeln!(
            statements,
            "{}: allocations, peak and bytes of each request {:?}\n{}row records by relation:\n{}",
            path.name(),
            m.requests(),
            m.statements,
            m.wal_ledger
        )
        .unwrap();
    }
    eprintln!(
        "{report}\nconversions per request by op:\n{ledger}\nnested statements over {REQUESTS} \
         requests:\n{statements}"
    );
    assert!(
        over.is_empty(),
        "a hot path exceeds its budget: {over:?}\n{report}\n{ledger}\n{statements}"
    );
    assert_eq!(BUDGETS.len(), Path::ALL.len(), "every path has its budget");
}

/// Every hot path stays within the bytes its budget allows it over the
/// database wire, both directions together (#6244), and the report names
/// what each statement moved.
///
/// The server reaches the database through a [`super::wire::Wire`] relay,
/// which delays every exchange a little and so moves how a request's buffers
/// grow: its other figures are judged by
/// [`every_hot_path_stays_within_its_budget`], over a direct connection.
#[tokio::test]
async fn every_hot_path_stays_within_its_wire_budget() {
    let counter = Counter::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(counter.clone()));
    ferroehr::aql::sql::without_reference_cells();
    let (_db, pool, svc, app, wire) = Box::pin(super::wired()).await;
    let measured = Box::pin(measure_all(&app, &pool, &svc, Some(&wire), &counter)).await;
    let mut report = String::from("path | wire B | sent B | received B\n");
    let mut ledgers = String::new();
    let mut over = Vec::new();
    for (budget, m) in &measured {
        let path = budget.path;
        let ceiling = WIRE_BUDGETS
            .iter()
            .find(|(of, _)| *of == path)
            .map(|(_, ceiling)| *ceiling)
            .expect("every path has its wire budget");
        let (ok, cell) = judged(median(&m.wire_each), ceiling);
        if !ok {
            over.push(format!("{}: wire bytes {cell}", path.name()));
        }
        let total = m.wire.total();
        writeln!(
            report,
            "{} | {cell} | {} | {}",
            path.name(),
            total.sent.div_euclid(REQUESTS),
            total.received.div_euclid(REQUESTS),
        )
        .unwrap();
        writeln!(ledgers, "{}: {}", path.name(), wire_ledger(m)).unwrap();
    }
    eprintln!("{report}\n{ledgers}");
    assert!(
        over.is_empty(),
        "a hot path exceeds its wire budget: {over:?}\n{report}\n{ledgers}"
    );
    assert_eq!(
        WIRE_BUDGETS.len(),
        Path::ALL.len(),
        "every path has its wire budget"
    );
}
