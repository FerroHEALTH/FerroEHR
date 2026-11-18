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
//! and a population AQL query, an ITI-81 page and a party read, all over the
//! IPS composition (128 KB, 402 node rows).
//!
//! The conversions are the `trace` events under
//! [`openehr_its::conversion::TARGET`]: the codec entry points record each
//! one, and the server records the changes it makes itself
//! (`ferroehr::telemetry::conversion`). The allocations are counted on the
//! thread that polls the request, so the audit runtime's threads do not
//! count; the peak is the most the request held at once.
//!
//! Each figure is held to the ceiling [`BUDGETS`] names. A figure with no
//! ceiling yet is reported and not judged: the core rewrite (#6001) sets each
//! one to its measured end state plus a small margin, and from then on a
//! ceiling only ever comes down, on an owner decision. No openEHR spec
//! governs the cost of a request: our own design.

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

use super::{CHAINER_ROUND, CHAINER_TAKE, SYSTEM_ROUND, calls, counted, drained};
use crate::access_evaluator_http::{app, credential, ehr_with_composition};
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
    Iti81,
    PartyRead,
}

impl Path {
    const ALL: [Self; 11] = [
        Self::CommitJson,
        Self::CommitXml,
        Self::CommitFlat,
        Self::ReadJson,
        Self::ReadXml,
        Self::ReadFlat,
        Self::EhrRead,
        Self::AqlSingle,
        Self::AqlPopulation,
        Self::Iti81,
        Self::PartyRead,
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
            Self::Iti81 => "ITI-81 page",
            Self::PartyRead => "party read",
        }
    }
}

/// The ceilings of one path, per request; `None` is reported and not judged.
struct Budget {
    path: Path,
    conversions: Option<u64>,
    allocations: Option<u64>,
    peak_bytes: Option<u64>,
    round_trips: Option<u64>,
    nested: Option<u64>,
    wal: Option<u64>,
}

/// A path whose every figure is reported and none judged yet.
const fn reported(path: Path) -> Budget {
    Budget {
        path,
        conversions: None,
        allocations: None,
        peak_bytes: None,
        round_trips: None,
        nested: None,
        wal: None,
    }
}

/// The ceilings by path. Before the core rewrite (feat/core-rewrite at
/// 13cda4c7f, debug build) the paths measured, per request, conversions /
/// allocations / round trips / nested statements / WAL bytes: recorded on
/// #6007.
const BUDGETS: [Budget; 11] = [
    reported(Path::CommitJson),
    reported(Path::CommitXml),
    reported(Path::CommitFlat),
    reported(Path::ReadJson),
    reported(Path::ReadXml),
    reported(Path::ReadFlat),
    reported(Path::EhrRead),
    reported(Path::AqlSingle),
    reported(Path::AqlPopulation),
    reported(Path::Iti81),
    reported(Path::PartyRead),
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
                body: Some((JSON, person)),
            },
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "the person: {body}");
        Self {
            ehrs,
            xml,
            flat,
            template_id,
            party: object_id(&headers),
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
        }
    }
}

/// What one path measured over [`REQUESTS`] requests.
#[derive(Default)]
struct Measure {
    conversions: BTreeMap<String, u64>,
    allocated: Allocated,
    peak: i64,
    round_trips: u64,
    nested: u64,
    wal: u64,
}

impl Measure {
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

/// The write-ahead log of the test database's top-level statements since the
/// last reset, its own reads left out.
async fn wal_bytes(pool: &PgPool) -> u64 {
    let bytes: Option<i64> = sqlx::query_scalar(
        "SELECT /* statement_budget_own */ sum(wal_bytes)::bigint FROM pg_stat_statements \
         WHERE toplevel AND dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         AND query NOT LIKE '%pg_stat_statements%' AND query NOT LIKE '%statement_budget_own%'",
    )
    .fetch_one(pool)
    .await
    .expect("pg_stat_statements reads");
    bytes.map_or(0, |b| u64::try_from(b).expect("a byte count"))
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
        let (status, _, body) = plain(app, &world.call(path, usize::try_from(n).unwrap())).await;
        assert!(status.is_success(), "{}: {status} {body}", path.name());
    }
    drained(pool).await;
    sqlx::query(
        "SELECT pg_stat_statements_reset(0, \
         (SELECT oid FROM pg_database WHERE datname = current_database()), 0)",
    )
    .execute(pool)
    .await
    .expect("the statement counts reset");
    *counter.0.lock().unwrap() = Counts::default();
    let mut out = Measure::default();
    for n in 0..REQUESTS {
        let call = world.call(path, usize::try_from(WARM + n).unwrap());
        let mut allocated = Allocated::default();
        let (status, _, body) = Measured {
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
        out.allocated.count += allocated.count;
        out.allocated.bytes += allocated.bytes;
        out.peak = out.peak.max(allocated.peak);
    }
    drained(pool).await;
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
        + calls(&statements, "SELECT audit.audit_event_chain_move") * SYSTEM_ROUND;
    let counts = std::mem::take(&mut *counter.0.lock().unwrap());
    out.conversions = counts.conversions;
    out.round_trips = counts.round_trips.div_euclid(REQUESTS);
    out.nested = nested.saturating_sub(chainer).div_euclid(REQUESTS);
    out.wal = wal_bytes(pool).await.div_euclid(REQUESTS);
    out
}

/// Whether `measured` passes `ceiling`, and the cell that reports it.
fn judged(measured: u64, ceiling: Option<u64>) -> (bool, String) {
    match ceiling {
        Some(c) => (measured <= c, format!("{measured} (<= {c})")),
        None => (true, format!("{measured}")),
    }
}

/// Every hot path stays within its budget, and the report names what each
/// one measured.
#[tokio::test]
async fn every_hot_path_stays_within_its_budget() {
    let counter = Counter::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(counter.clone()));
    let (_db, pool, _svc, app) = Box::pin(app()).await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    let world = World::build(&app).await;
    let mut report = String::from(
        "path | conversions | audit renders | allocations | alloc KiB | peak KiB | round trips | nested | WAL B\n",
    );
    let mut ledger = String::new();
    let mut over = Vec::new();
    for budget in &BUDGETS {
        let path = budget.path;
        let m = Box::pin(measure(&app, &pool, &world, &counter, path)).await;
        let allocations = m.allocated.count.div_euclid(REQUESTS);
        let kib = m.allocated.bytes.div_euclid(REQUESTS * 1024);
        let peak = u64::try_from(m.peak.max(0)).unwrap();
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
    }
    eprintln!("{report}\nconversions per request by op:\n{ledger}");
    assert!(
        over.is_empty(),
        "a hot path exceeds its budget: {over:?}\n{report}\n{ledger}"
    );
    assert_eq!(BUDGETS.len(), Path::ALL.len(), "every path has its budget");
}
