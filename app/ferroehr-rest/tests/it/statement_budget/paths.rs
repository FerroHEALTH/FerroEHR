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
//! and the commit of a PERSON, plain and with ordinal and scale values.
//!
//! The conversions are the `trace` events under
//! [`openehr_its::conversion::TARGET`]: the codec entry points record each
//! one, and the server records the changes it makes itself
//! (`ferroehr::telemetry::conversion`). The allocations are counted on the
//! thread that polls the request, so the audit runtime's threads do not
//! count; the peak is the most the request held at once. Both are judged on
//! the median request of the measured ones ([`Measure::median`]).
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

use super::{CHAINER_RELEASE, CHAINER_ROUND, CHAINER_TAKE, SYSTEM_ROUND, calls, counted, drained};
use crate::access_evaluator_http::{credential, ehr_with_composition};
use crate::common::{self, BASE, FHIR};

/// The measured requests of each path.
const REQUESTS: u64 = 10;

/// The requests of each path sent before the measured ones.
const WARM: u64 = 3;

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
enum Path {
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
}

impl Path {
    const ALL: [Self; 20] = [
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
    ];

    fn name(self) -> &'static str {
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
    wal: u64,
}

/// The ceilings by path. Before the core rewrite (feat/core-rewrite at
/// 13cda4c7f, debug build) the paths measured, per request, conversions /
/// allocations / round trips / nested statements / WAL bytes: recorded on
/// #6007. The end state (#6001) is the figure measured over four runs at
/// feat/core-rewrite dfed03d9d plus a margin: none on conversions and round
/// trips, 2% on allocations, 10% on the peak, 3% on WAL bytes, and one nested
/// statement on a commit. The nested statements leave out the chainer's
/// rounds, the release of the records the commits held among them. The
/// commits have since taken the `event_time` column and the magnitude index
/// (#6022): 246,564, 247,837, 248,829 and 248,242 WAL bytes measured against
/// 242,177, 243,204, 244,956 and 244,045, 1.8% more, under the same ceilings.
/// The conversions and the allocations of the commits and the raw reads are
/// the figures measured over four runs once a read left the stored audit
/// undecoded and a commit signed its version from typed members (#6037), with
/// the same margins.
/// The commits of a body the data-minimisation pass reads have since lost
/// their second render and the pass's own allocations (#6038): 18,841
/// allocations on the JSON commit against 33,307, measured over three runs.
/// A CONTRIBUTION commit has since read its envelope once from the text and
/// each member from its own text, and judged its committers as typed RM
/// (#6038): 6 conversions and 19,010 allocations against 9 and 51,858.
/// The WAL figure has since been the row records a request writes, read with
/// `pg_walinspect` as `statement_budget::wal` reads them, with the chain
/// heads and page maintenance left out and the lower-quartile request judged
/// (#6035): the sum `pg_stat_statements` gave moved with the chainer's
/// passes, and a read measured 2,248 to 2,325 bytes between runs. Twelve of
/// the sixteen paths now measure the same figure in every one of seven runs,
/// one of them beside 175 other tests, and the other four within 64 bytes,
/// so the ceilings are the largest of the seven plus 1%.
/// The four commits of the IPS composition have since taken the index on the
/// code of an ELEMENT value (#6033): 36 entries of 72 bytes, 2,592 bytes a
/// commit and 1.08%, inside the 3% that issue allows the index. Their WAL
/// ceilings rose by that, to the largest of three runs plus 1%: the one rise
/// of a ceiling, and the trade #6033 states.
/// The harness has since started the `LISTEN` sessions a server runs with,
/// and a request confirms the copies its decision rests on with two indexed
/// reads, one per store (#6048). The round trips are the figures measured
/// then: a read sends 3 (its own, the party store's revisions, the policy
/// heads) where the harness without listeners sent 7 and a server before
/// #6048 sent 1. A party commit moves the binding revision in its
/// transaction, one nested statement and 88 bytes of WAL, the second rise of
/// a ceiling and the price #6048 states for a decision that waits on no
/// notification.
/// The data-minimisation pass has since stopped reading string leaves for
/// the shape of a national identifier (#6081): 1,069 fewer allocations on
/// each of the three composition commits and 1,057 on a CONTRIBUTION commit,
/// measured beside the merge base, and their ceilings fell by as much.
/// The population `matches` over a value list (#6084) is measured last, so
/// its access records stay off the ITI-81 page; its ceilings are the figures
/// of three runs (5,637 to 5,639 allocations, a median peak of 174,788
/// bytes, 13,180 WAL bytes) with the margins above.
/// The demographic population query (#6087) runs as a caller with access
/// groups after the `matches`, for the same reason; its ceilings are the
/// figures of three runs (2,630 to 2,644 allocations, a median peak of
/// 98,022 bytes, 2,062 WAL bytes) with the same margins. It has since judged
/// no `EHR_ACCESS` condition (#6110): 2,619 to 2,621 allocations and a median
/// peak of 97,454 bytes over three runs, and its ceilings fell by as much.
/// The commit of a PERSON holding two ordinals and two scales holds the WAL
/// the index on their numbers costs (#6090): 4 entries of 72 bytes, 288 of
/// the 10,489 bytes it wrote in each of three runs. Its ceilings are the
/// figures of those runs (2,486 allocations, a median peak of at most 120,133
/// bytes, 20 nested statements) with the margins above; no other path moved.
/// It is measured last, so the PERSONs it commits stay out of the population
/// the demographic query reads, which they grew by some 100 allocations.
/// A Basic user's held roles and scopes have since been read once with the
/// grant rows (#6013): 34 to 54 fewer allocations on every path, each of
/// which authenticates a Basic user, measured beside the merge base, and
/// each ceiling fell by its path's figure.
/// The version-tree uniqueness has since been declared per partition without
/// the tier and served the read by version id, and the separate tree index is
/// gone (#6207): every commit that writes a version writes 96 bytes of WAL
/// fewer, measured beside the merge base, and each WAL ceiling fell by it.
/// The read by version id served from the version copy (#5778) is measured
/// last, on EHRs of its own whose settings name an object, so each read is
/// judged on its own row; its median request is a copy hit. The copy has
/// since held the body as shared text (#6205): a hit allocated 564 KiB with a
/// peak of 272,547 bytes before and 431 to 439 KiB with a peak of at most
/// 147,965 bytes after, over three runs, and a fill one copy of the body
/// less. Its ceilings are those figures with the margins above.
const BUDGETS: [Budget; 20] = [
    Budget {
        path: Path::CommitJson,
        conversions: 5,
        allocations: 18_109,
        peak_bytes: 1_530_778,
        round_trips: 9,
        nested: 13,
        wal: 245_120,
    },
    Budget {
        path: Path::CommitXml,
        conversions: 5,
        allocations: 39_537,
        peak_bytes: 1_533_031,
        round_trips: 9,
        nested: 14,
        wal: 245_193,
    },
    Budget {
        path: Path::CommitFlat,
        conversions: 5,
        allocations: 32_697,
        peak_bytes: 1_528_525,
        round_trips: 9,
        nested: 13,
        wal: 245_167,
    },
    Budget {
        path: Path::ReadJson,
        conversions: 0,
        allocations: 1335,
        peak_bytes: 617_268,
        round_trips: 3,
        nested: 2,
        wal: 2773,
    },
    Budget {
        path: Path::ReadXml,
        conversions: 2,
        allocations: 10_812,
        peak_bytes: 1_017_140,
        round_trips: 3,
        nested: 2,
        wal: 2773,
    },
    Budget {
        path: Path::ReadFlat,
        conversions: 2,
        allocations: 15_529,
        peak_bytes: 1_634_407,
        round_trips: 3,
        nested: 2,
        wal: 2773,
    },
    Budget {
        path: Path::EhrRead,
        conversions: 1,
        allocations: 1121,
        peak_bytes: 100_250,
        round_trips: 3,
        nested: 2,
        wal: 1989,
    },
    Budget {
        path: Path::AqlSingle,
        conversions: 2,
        allocations: 2451,
        peak_bytes: 118_273,
        round_trips: 4,
        nested: 2,
        wal: 4439,
    },
    Budget {
        path: Path::AqlPopulation,
        conversions: 2,
        allocations: 3303,
        peak_bytes: 158_823,
        round_trips: 3,
        nested: 2,
        wal: 11_380,
    },
    Budget {
        path: Path::Iti81,
        conversions: 0,
        allocations: 8985,
        peak_bytes: 624_026,
        round_trips: 3,
        nested: 2,
        wal: 1852,
    },
    Budget {
        path: Path::PartyRead,
        conversions: 0,
        allocations: 1066,
        peak_bytes: 78_848,
        round_trips: 8,
        nested: 6,
        wal: 2070,
    },
    Budget {
        path: Path::CommitContribution,
        conversions: 6,
        allocations: 18_298,
        peak_bytes: 1_649_934,
        round_trips: 13,
        nested: 14,
        wal: 245_688,
    },
    Budget {
        path: Path::ReadAtTime,
        conversions: 0,
        allocations: 1346,
        peak_bytes: 701_748,
        round_trips: 3,
        nested: 2,
        wal: 2773,
    },
    Budget {
        path: Path::CommitStatus,
        conversions: 4,
        allocations: 1761,
        peak_bytes: 144_180,
        round_trips: 15,
        nested: 11,
        wal: 6117,
    },
    Budget {
        path: Path::ReadStatus,
        conversions: 0,
        allocations: 1190,
        peak_bytes: 102_503,
        round_trips: 3,
        nested: 2,
        wal: 2102,
    },
    Budget {
        path: Path::CommitParty,
        conversions: 8,
        allocations: 2006,
        peak_bytes: 117_146,
        round_trips: 31,
        nested: 20,
        wal: 7857,
    },
    Budget {
        path: Path::AqlMatches,
        conversions: 2,
        allocations: 5711,
        peak_bytes: 192_267,
        round_trips: 4,
        nested: 2,
        wal: 13_312,
    },
    Budget {
        path: Path::AqlParty,
        conversions: 2,
        allocations: 2634,
        peak_bytes: 107_199,
        round_trips: 8,
        nested: 6,
        wal: 2083,
    },
    Budget {
        path: Path::CommitPartyScored,
        conversions: 8,
        allocations: 2495,
        peak_bytes: 132_147,
        round_trips: 31,
        nested: 21,
        wal: 10_498,
    },
    Budget {
        path: Path::ReadVersion,
        conversions: 0,
        allocations: 1341,
        peak_bytes: 162_800,
        round_trips: 3,
        nested: 2,
        wal: 2757,
    },
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
}

/// Reads the `op` field of a conversion event.
#[derive(Default)]
struct Op(String);

impl Visit for Op {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "op" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "op" && self.0.is_empty() {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for Counter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
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
struct Call {
    method: &'static str,
    uri: String,
    user: &'static str,
    accept: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Option<(&'static str, String)>,
}

impl Call {
    fn request(&self) -> Request<Body> {
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
async fn exchange(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
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
struct World {
    ehrs: Vec<(String, String)>,
    xml: String,
    flat: String,
    template_id: String,
    party: String,
    person: String,
    /// The PERSON with two ordinals and two scales in its details.
    scored: String,
    /// An instant after every fixture commit.
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
    async fn build(app: &Router) -> Self {
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
        let person = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../corpus/fixtures/demographic/person.v1.json"),
        )
        .expect("the person fixture");
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
            ehrs,
            xml,
            flat,
            template_id,
            party: object_id(&headers),
            scored: scored(&person),
            person,
            at: jiff::Timestamp::now().to_string(),
            status: status_body,
            status_uids: Mutex::new(status_uids),
            judged: Mutex::new(Vec::new()),
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
        self.status_uids.lock().unwrap()[0]
            .split("::")
            .nth(1)
            .expect("a system id")
            .to_owned()
    }

    /// Records what the answer to the `n`th request of `path` changed.
    fn observe(&self, path: Path, n: usize, headers: &HeaderMap) {
        if path == Path::CommitStatus {
            self.status_uids.lock().unwrap()[n % self.ehrs.len()] = etag(headers);
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
    fn call(&self, path: Path, n: usize) -> Call {
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
            Path::Iti81 => get(
                format!("{FHIR}/AuditEvent?date=ge2026-01-01&_count=20"),
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
            Path::CommitParty => Self::party_commit(self.person.clone()),
            Path::CommitPartyScored => Self::party_commit(self.scored.clone()),
            Path::ReadVersion => {
                let (ehr, version) = self.judged.lock().unwrap()[n % EHRS].clone();
                get(
                    format!("{BASE}/ehr/{ehr}/composition/{version}"),
                    "nurse",
                    JSON,
                )
            }
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
    /// The row records of the lower-quartile request.
    wal: u64,
    /// The row records of each request and of the window by relation, with
    /// the chain heads' the figure leaves out.
    wal_ledger: String,
    /// The statements `PostgreSQL` ran nested, and the chainer's rounds the
    /// count leaves out.
    statements: String,
}

impl Measure {
    /// The median request's `figure`.
    ///
    /// One request in a window sometimes opens a pool connection, or is
    /// handed one whose buffers have not grown yet, when the chainer holds
    /// the pool's other connection, so the mean and the largest moved with
    /// the host. A change in what a request itself allocates moves every
    /// request, and so the median.
    fn median(&self, figure: impl Fn(&Allocated) -> u64) -> u64 {
        // NOTE: our own design (#6007), a connection the pool opens inside a request is the
        // pool's work, as its session setup is for the round trips (#5899).
        let mut figures: Vec<u64> = self.each.iter().map(figure).collect();
        figures.sort_unstable();
        figures
            .get(figures.len().div_euclid(2))
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

/// Whether `relation` is a chain head: the access log's and the system
/// audit trail's, which a chainer pass writes once whatever it links, so how
/// often depends on how the passes fall among the requests.
fn head_relation(relation: &str) -> bool {
    super::wal::pass_relation(relation) || relation == "audit_chain_state"
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
            AND c.relname NOT IN ('access_log_chain_state', 'audit_chain_state') \
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
    world: &World,
    counter: &Counter,
    path: Path,
) -> Measure {
    for n in 0..WARM {
        let n = usize::try_from(n).unwrap();
        let (status, headers, body) = plain(app, &world.call(path, n)).await;
        assert!(status.is_success(), "{}: {status} {body}", path.name());
        world.observe(path, n, &headers);
    }
    drained(pool).await;
    let evicted = testkit::statements::statement_window(pool)
        .await
        .expect("the statement counts reset");
    *counter.0.lock().unwrap() = Counts::default();
    let mut marks = vec![super::wal::insert_lsn(pool).await];
    let mut out = Measure::default();
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
    }
    let statements = counted(pool).await;
    let nested: u64 = statements
        .iter()
        .filter(|s| !s.toplevel)
        .map(|s| s.calls)
        .sum();
    let chainer = calls(
        &statements,
        "SELECT head_seq, head_digest FROM audit.access_log_chain_take",
    ) * CHAINER_TAKE
        + calls(&statements, "SELECT audit.access_log_chain_move") * CHAINER_ROUND
        + calls(&statements, "SELECT audit.audit_event_chain_move") * SYSTEM_ROUND
        + calls(&statements, "SELECT audit.audit_intake_release") * CHAINER_RELEASE;
    let counts = std::mem::take(&mut *counter.0.lock().unwrap());
    out.conversions = counts.conversions;
    out.round_trips = counts.round_trips.div_euclid(REQUESTS);
    out.nested = nested.saturating_sub(chainer).div_euclid(REQUESTS);
    out.statements = nested_ledger(&statements, nested, chainer);
    (out.wal, out.wal_ledger) = row_records(pool, &marks).await;
    testkit::statements::assert_statements_kept(pool, evicted)
        .await
        .expect("no statement evicted while the test counted them");
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

/// Whether `measured` passes `ceiling`, and the cell that reports it.
fn judged(measured: u64, ceiling: u64) -> (bool, String) {
    (measured <= ceiling, format!("{measured} (<= {ceiling})"))
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
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_walinspect")
        .execute(&pool)
        .await
        .expect("pg_walinspect: a contrib module the server ships");
    super::wal::kept(&pool).await;
    let world = World::build(&app).await;
    let mut report = String::from(
        "path | conversions | audit renders | allocations | alloc KiB | peak KiB | round trips | nested | WAL B\n",
    );
    let mut ledger = String::new();
    let mut statements = String::new();
    let mut over = Vec::new();
    for budget in &BUDGETS {
        let path = budget.path;
        if path == Path::ReadVersion {
            world.judge_on_objects(&app, &svc).await;
        }
        let m = Box::pin(measure(&app, &pool, &world, &counter, path)).await;
        let allocations = m.median(|a| a.count);
        let kib = m.median(|a| a.bytes).div_euclid(1024);
        let peak = m.median(|a| u64::try_from(a.peak.max(0)).unwrap());
        let cells = [
            ("conversions", judged(m.conversions(), budget.conversions)),
            ("allocations", judged(allocations, budget.allocations)),
            ("peak bytes", judged(peak, budget.peak_bytes)),
            ("round trips", judged(m.round_trips, budget.round_trips)),
            ("nested statements", judged(m.nested, budget.nested)),
            ("WAL bytes", judged(m.wal, budget.wal)),
        ];
        for (what, (ok, cell)) in &cells {
            if !ok {
                over.push(format!("{}: {what} {cell}", path.name()));
            }
        }
        writeln!(
            report,
            "{} | {} | {} | {} | {kib} | {} | {} | {} | {}",
            path.name(),
            cells[0].1.1,
            m.audit(),
            cells[1].1.1,
            peak.div_euclid(1024),
            cells[3].1.1,
            cells[4].1.1,
            cells[5].1.1,
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
