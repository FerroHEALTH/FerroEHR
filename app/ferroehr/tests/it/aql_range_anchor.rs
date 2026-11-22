// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A filter on an event's time, a history's origin or the magnitude of an
//! ELEMENT value enters through an index and answers the rows it answered
//! before (#6022).
//!
//! The node insert stores `ext.openehr_timestamp` of an event row's `time`
//! and of a history row's `origin` in `event_time`, and
//! `ext.node_magnitude` reads the magnitude of an ELEMENT value from the
//! place the row holds it. The AQL engine adds, to a top-level comparison of
//! such a leaf, the versions some row of which carries a value the
//! comparison keeps (an anchor). The comparison itself is unchanged, so an
//! anchor may only remove work. The tests hold the three halves: which
//! comparisons get an anchor, that the anchored statement answers what the
//! unanchored one does over every stored form of a value, and that the plan
//! starts from the index. AQL QUERY master03 §Built-in Types and
//! §Comparison operators govern the comparison; no openEHR spec governs
//! access paths: our own design.

use std::collections::BTreeMap;

use ferroehr::aql::ir::{ParamValue, Params};
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::fixtures::{ckm_example, composition, create_ehr, uv};
use crate::storage_plans::{
    EXPLAIN, WIDE_ARCHETYPE, analyze_as_autovacuum, aql_sql, aql_sql_with, explain_aql, on, render,
    scans, wide_leaf, wide_observation_composition,
};

/// The archetype of the OBSERVATION the equality store commits.
const TIMED: &str = "openEHR-EHR-OBSERVATION.timed.v1";

/// The time of an event under the OBSERVATION `o`.
const EVENT_LEAF: &str = "o/data[at0001]/events[at0002]/time/value";

/// The origin of the history under the OBSERVATION `o`.
const ORIGIN_LEAF: &str = "o/data[at0001]/origin/value";

/// `FROM` an OBSERVATION of [`TIMED`].
pub(crate) const FROM_OBSERVATION: &str =
    "FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.timed.v1]";

/// The magnitude of the one ELEMENT of an event under the OBSERVATION `o`.
const MAGNITUDE_LEAF: &str =
    "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/magnitude";

/// How many magnitude anchors `sql` carries: the comparison reads the leaf
/// through the typed columns, so only an anchor calls the function on its
/// own row, `va<n>_n`, beside the call its earlier-row probe makes on
/// `va<n>_e`.
fn magnitude_anchors(sql: &str) -> usize {
    sql.match_indices(r#"node_magnitude(""#)
        .filter(|(at, call)| {
            sql[at + call.len()..]
                .split('"')
                .next()
                .is_some_and(|alias| alias.ends_with("_n"))
        })
        .count()
}

/// How many time anchors `sql` carries: each reads the column once.
fn time_anchors(sql: &str) -> usize {
    sql.matches(r#"_n"."event_time""#).count()
}

/// The SQL of `aql` over the population, with `params` bound.
pub(crate) fn lowered(aql: &str, params: &Params) -> String {
    aql_sql_with(aql, params, Vec::new(), None).sql.to_string()
}

/// [`lowered`] for an execution with a row limit, over the population or
/// over one named EHR.
fn lowered_limited(aql: &str, one_ehr: bool) -> String {
    let ehrs = if one_ehr {
        vec![ferroehr::ids::EhrId(uuid::Uuid::nil())]
    } else {
        Vec::new()
    };
    aql_sql_with(aql, &Params::default(), ehrs, Some(50))
        .sql
        .to_string()
}

// ── which comparisons are anchored ───────────────────────────────────────────

/// Each ordering comparison and the equality of an event's time is anchored,
/// against a literal and against a parameter, under the operator it states.
#[test]
fn a_comparison_of_an_events_time_is_anchored() {
    let params = Params::default().with("t", ParamValue::Str("2026-03-01T10:00:00Z".to_owned()));
    for op in ["=", "<", "<=", ">", ">="] {
        for filter in [
            format!("{EVENT_LEAF} {op} '2026-03-01T10:00:00Z'"),
            format!("{EVENT_LEAF} {op} $t"),
        ] {
            let sql = lowered(
                &format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {filter}"),
                &params,
            );
            assert_eq!(time_anchors(&sql), 1, "{filter}: one anchor\n{sql}");
            assert!(
                sql.contains(&format!(r#"_n"."event_time" {op} "#)),
                "{filter}: the anchor compares the column as the filter does\n{sql}"
            );
            assert!(
                sql.contains(r#""rm_type" IN ('INTERVAL_EVENT', 'POINT_EVENT')"#),
                "{filter}: the anchor names the event types as literals the planner \
                 estimates\n{sql}"
            );
        }
    }
}

/// A history's origin is anchored on the HISTORY rows, and an event or a
/// history that is itself the source is anchored too.
#[test]
fn an_origin_and_a_timed_source_are_anchored() {
    let params = Params::default();
    let origin = lowered(
        &format!(
            "SELECT c/uid/value {FROM_OBSERVATION} WHERE {ORIGIN_LEAF} >= '2026-03-01T10:00:00Z'"
        ),
        &params,
    );
    assert_eq!(time_anchors(&origin), 1, "{origin}");
    assert!(origin.contains(r#""rm_type" IN ('HISTORY')"#), "{origin}");
    for (source, leaf) in [
        ("POINT_EVENT s", "s/time/value"),
        ("INTERVAL_EVENT s", "s/time/value"),
        ("HISTORY s", "s/origin/value"),
    ] {
        let sql = lowered(
            &format!(
                "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS {source} \
                 WHERE {leaf} >= '2026-03-01T10:00:00Z'"
            ),
            &params,
        );
        assert_eq!(time_anchors(&sql), 1, "{source}\n{sql}");
    }
}

/// A conjunct under `OR` or `NOT`, an inequality, a time of a row that is no
/// event or history, and a leaf step with a predicate of its own imply no
/// anchor; each conjunct of a conjunction is anchored.
#[test]
fn only_a_top_level_comparison_of_an_event_time_is_anchored() {
    for (name, filter) in [
        (
            "an OR",
            format!("{EVENT_LEAF} >= '2026-03-01' OR {EVENT_LEAF} >= '2026-03-01'"),
        ),
        ("a NOT", format!("NOT {EVENT_LEAF} >= '2026-03-01'")),
        ("an inequality", format!("{EVENT_LEAF} != '2026-03-01'")),
        (
            "the start time of the context",
            "c/context/start_time/value >= '2026-03-01'".to_owned(),
        ),
    ] {
        let sql = lowered(
            &format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {filter}"),
            &Params::default(),
        );
        assert_eq!(time_anchors(&sql), 0, "{name} is not anchored\n{sql}");
    }
    let action = lowered(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS ACTION a \
         WHERE a/time/value >= '2026-03-01'",
        &Params::default(),
    );
    assert_eq!(
        time_anchors(&action),
        0,
        "the time of an ACTION is no column of its row\n{action}"
    );
    let both = lowered(
        &format!(
            "SELECT c/uid/value {FROM_OBSERVATION} \
             WHERE {EVENT_LEAF} >= '2026-03-01' AND {EVENT_LEAF} < '2026-04-01' \
             AND {ORIGIN_LEAF} >= '2026'"
        ),
        &Params::default(),
    );
    assert_eq!(time_anchors(&both), 3, "each conjunct is anchored\n{both}");
}

/// Each ordering comparison of a magnitude is anchored on the ELEMENT rows,
/// against a literal and against a parameter, under the operator it states.
#[test]
fn an_ordering_comparison_of_a_magnitude_is_anchored() {
    let params = Params::default().with("x", ParamValue::Real(7.5));
    for op in ["<", "<=", ">", ">="] {
        for filter in [
            format!("{MAGNITUDE_LEAF} {op} 7.5"),
            format!("{MAGNITUDE_LEAF} {op} 7"),
            format!("{MAGNITUDE_LEAF} {op} $x"),
        ] {
            let sql = lowered(
                &format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {filter}"),
                &params,
            );
            assert_eq!(magnitude_anchors(&sql), 1, "{filter}: one anchor\n{sql}");
            assert!(
                sql.contains(r#"_n"."rm_type" IN ('ELEMENT')"#)
                    && sql.contains(&format!(r#"_n"."data") {op} "#))
                    && sql.contains(r#""archetype" = $"#),
                "{filter}: the anchor reads the ELEMENT rows of the leaf step's at-code \
                 under the filter's operator\n{sql}"
            );
        }
    }
    let element = lowered(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS ELEMENT el[at0004] \
         WHERE el/value/magnitude > 7",
        &params,
    );
    assert_eq!(magnitude_anchors(&element), 1, "{element}");
}

/// An equality, an inequality, a conjunct under `OR` or `NOT`, a comparison
/// with another path and any other numeric leaf imply no magnitude anchor.
#[test]
fn only_a_top_level_ordering_of_a_magnitude_is_anchored() {
    let value = MAGNITUDE_LEAF.trim_end_matches("/magnitude");
    for (name, filter) in [
        ("an equality", format!("{MAGNITUDE_LEAF} = 7")),
        ("an inequality", format!("{MAGNITUDE_LEAF} != 7")),
        (
            "an OR",
            format!("{MAGNITUDE_LEAF} > 7 OR {MAGNITUDE_LEAF} > 7"),
        ),
        ("a NOT", format!("NOT {MAGNITUDE_LEAF} > 7")),
        (
            "another path",
            format!("{MAGNITUDE_LEAF} > {value}/precision"),
        ),
        ("an ordinal's value", format!("{value}/value > 7")),
        ("a precision", format!("{value}/precision > 7")),
        (
            "a bound of an interval",
            format!("{value}/lower/magnitude > 7"),
        ),
    ] {
        let sql = lowered(
            &format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {filter}"),
            &Params::default(),
        );
        assert_eq!(magnitude_anchors(&sql), 0, "{name} is not anchored\n{sql}");
    }
}

/// The comparisons of one leaf share one anchor, the rows inside the window,
/// and the comparisons of another leaf get their own.
#[test]
fn the_bounds_of_one_leaf_share_one_anchor() {
    let sql = lowered(
        &format!(
            "SELECT c/uid/value {FROM_OBSERVATION} \
             WHERE {EVENT_LEAF} >= '2026-03-01' AND {EVENT_LEAF} < '2026-04-01' \
             AND {MAGNITUDE_LEAF} > 7 AND {MAGNITUDE_LEAF} <= 9"
        ),
        &Params::default(),
    );
    assert_eq!(
        sql.matches(r#"_e"."num" < "#).count(),
        2,
        "two anchors, each the first row of a version\n{sql}"
    );
    assert!(!sql.contains("SELECT DISTINCT"), "{sql}");
    assert!(
        sql.contains(r#"_n"."event_time" >= "#) && sql.contains(r#"_n"."event_time" < "#),
        "the window is one anchor's two bounds\n{sql}"
    );
    assert_eq!(magnitude_anchors(&sql), 2, "{sql}");
}

/// A population statement that streams toward its LIMIT takes one range
/// anchor, as the first row of each version the range keeps and never the
/// distinct versions, which a scan toward the LIMIT would have to finish. A
/// statement that names its EHRs takes none: their versions are its entry.
#[test]
fn a_streaming_statement_takes_a_range_anchor_only_over_the_population() {
    let aql = format!(
        "SELECT c/uid/value {FROM_OBSERVATION} \
         WHERE {EVENT_LEAF} >= '2026-03-01' AND {MAGNITUDE_LEAF} > 7"
    );
    let population = lowered_limited(&aql, false);
    assert!(
        population.contains("JOIN LATERAL"),
        "it streams\n{population}"
    );
    assert_eq!(time_anchors(&population), 1, "{population}");
    assert_eq!(
        magnitude_anchors(&population),
        0,
        "one anchor leads\n{population}"
    );
    assert!(
        !population.contains("SELECT DISTINCT")
            && population.contains("NOT EXISTS")
            && population.contains(r#""va0_e"."event_time" >= "#)
            && population.contains(r#""va0_e"."num" < "va0_n"."num""#),
        "the anchor is the first row of each version\n{population}"
    );
    let named = lowered_limited(&aql, true);
    assert!(named.contains("JOIN LATERAL"), "it streams\n{named}");
    assert_eq!(
        time_anchors(&named) + magnitude_anchors(&named),
        0,
        "{named}"
    );
    let pinned = lowered_limited(
        &format!(
            "SELECT c/uid/value FROM EHR e[ehr_id/value = '{}'] CONTAINS COMPOSITION c \
             CONTAINS OBSERVATION o[{TIMED}] WHERE {EVENT_LEAF} >= '2026-03-01'",
            uuid::Uuid::nil()
        ),
        false,
    );
    assert_eq!(time_anchors(&pinned), 0, "{pinned}");
    // An equality is the entry of a streaming statement that has one.
    let text = lowered_limited(
        &format!(
            "SELECT c/uid/value {FROM_OBSERVATION} WHERE {EVENT_LEAF} >= '2026-03-01' \
             AND o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/value = 'x'"
        ),
        false,
    );
    assert_eq!(time_anchors(&text), 0, "{text}");
    assert!(text.contains("node_search_text("), "{text}");
}

// ── the anchored statement answers what the unanchored one does ──────────────

/// The stored forms of a time the equality store commits: full precision
/// with `Z`, with an offset and with no zone, a fraction, the basic format,
/// each reduced precision, and the edges the bounds below sit on.
const TIMES: [&str; 14] = [
    "2026-03-01T10:00:00Z",
    "2026-03-01T12:00:00+02:00",
    "2026-03-01T10:00:00",
    "2026-03-01T10:00:00.250Z",
    "20260301T100000Z",
    "2026-03-01T10",
    "2026-03-01",
    "2026-03",
    "2026",
    "2026-02-28T23:59:59Z",
    "2026-03-01T00:00:00Z",
    "2025-12-31T23:59:59-01:00",
    "2027-01-01T00:00:00+01:00",
    "1999-07-04T04:04:04Z",
];

/// The bounds each comparison is run with, in the same forms.
const BOUNDS: [&str; 9] = [
    "2026-03-01T10:00:00Z",
    "2026-03-01T10:00:00",
    "2026-03-01T10:00:00.250Z",
    "2026-03-01",
    "2026-03",
    "2026",
    "2027",
    "2026-03-01T12:00:00+02:00",
    "1999-07-04T04:04:04Z",
];

/// A `DV_DATE_TIME` reading `value`: the plain form the node row stores
/// compact, or, `wide`, one carrying a `magnitude_status`, which the row
/// keeps as an object (`ferroehr::storage::compact`).
pub(crate) fn date_time(value: &str, wide: bool) -> Value {
    if wide {
        json!({ "_type": "DV_DATE_TIME", "value": value, "magnitude_status": "~" })
    } else {
        json!({ "_type": "DV_DATE_TIME", "value": value })
    }
}

/// An event of the equality store at `time`, a `POINT_EVENT` or, `interval`,
/// an `INTERVAL_EVENT`.
fn event(time: &str, wide: bool, interval: bool) -> Value {
    event_holding(
        time,
        wide,
        interval,
        &json!({ "_type": "DV_TEXT", "value": time }),
    )
}

/// [`event`] whose one ELEMENT holds `value`.
pub(crate) fn event_holding(time: &str, wide: bool, interval: bool, value: &Value) -> Value {
    let mut event = json!({
        "_type": "POINT_EVENT",
        "name": { "_type": "DV_TEXT", "value": "Any event" },
        "archetype_node_id": "at0002",
        "time": date_time(time, wide),
        "data": {
            "_type": "ITEM_TREE",
            "name": { "_type": "DV_TEXT", "value": "Tree" },
            "archetype_node_id": "at0003",
            "items": [{
                "_type": "ELEMENT",
                "name": { "_type": "DV_TEXT", "value": "note" },
                "archetype_node_id": "at0004",
                "value": value
            }]
        }
    });
    if interval {
        let object = event.as_object_mut().expect("the event is a JSON object");
        object.insert("_type".to_owned(), json!("INTERVAL_EVENT"));
        object.insert(
            "width".to_owned(),
            json!({ "_type": "DV_DURATION", "value": "PT1H" }),
        );
        object.insert(
            "math_function".to_owned(),
            json!({
                "_type": "DV_CODED_TEXT",
                "value": "mean",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "146"
                }
            }),
        );
    }
    event
}

/// A COMPOSITION carrying one OBSERVATION of [`TIMED`] whose history starts
/// at `origin` and holds `events`.
pub(crate) fn timed_composition(name: &str, origin: &Value, events: &[Value]) -> Value {
    let mut body = composition(name);
    let content = json!([{
        "_type": "OBSERVATION",
        "name": { "_type": "DV_TEXT", "value": "Timed" },
        "archetype_node_id": TIMED,
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": TIMED },
            "rm_version": "1.1.0"
        },
        "language": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "en"
        },
        "encoding": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" },
            "code_string": "UTF-8"
        },
        "subject": { "_type": "PARTY_SELF" },
        "data": {
            "_type": "HISTORY",
            "name": { "_type": "DV_TEXT", "value": "History" },
            "archetype_node_id": "at0001",
            "origin": origin,
            "events": events
        }
    }]);
    body.as_object_mut()
        .expect("the composition fixture is a JSON object")
        .insert("content".to_owned(), content);
    body
}

/// Commits the equality store: every [`TIMES`] entry as the time of a
/// `POINT_EVENT` and of an `INTERVAL_EVENT`, compact and as an object, as an
/// origin, and beside a second event of another time in the same history.
async fn seed_times(svc: &FerroEhrService) -> usize {
    let mut committed = 0;
    for (index, time) in TIMES.iter().enumerate() {
        let next = TIMES[(index + 1) % TIMES.len()];
        let far = TIMES[(index + 5) % TIMES.len()];
        let ehr = create_ehr(svc).await;
        for (name, body) in [
            (
                "point",
                timed_composition(
                    &format!("point {index}"),
                    &date_time(next, false),
                    &[event(time, false, false)],
                ),
            ),
            (
                "interval",
                timed_composition(
                    &format!("interval {index}"),
                    &date_time(far, true),
                    &[event(time, false, true)],
                ),
            ),
            (
                "an object time and two events",
                timed_composition(
                    &format!("pair {index}"),
                    &date_time(time, false),
                    &[event(time, true, false), event(far, false, true)],
                ),
            ),
        ] {
            svc.create_composition(ehr, uv(&body, "249", None))
                .await
                .unwrap_or_else(|e| panic!("commit {name} at {time}: {e:?}"));
            committed += 1;
        }
    }
    // Two events on either side of every window below and none inside one.
    let ehr = create_ehr(svc).await;
    let split = timed_composition(
        "split",
        &date_time("1998-01-01T00:00:00Z", false),
        &[
            event("1999-01-01T00:00:00Z", false, false),
            event("2031-01-01T00:00:00Z", false, false),
        ],
    );
    svc.create_composition(ehr, uv(&split, "249", None))
        .await
        .expect("commit the split history");
    committed + 1
}

/// The statements that fill `event_time` from the stored times, as the v5
/// migration that introduced the column filled it for the rows stored before
/// it: the reading the insert path performs, run over rows the test rewrote.
fn migration_fill() -> Vec<String> {
    [
        "UPDATE clinical.node \
            SET event_time = ext.openehr_timestamp( \
                CASE WHEN time_compact THEN data ->> 'time' ELSE data #>> '{time,value}' END) \
          WHERE rm_type IN ('POINT_EVENT', 'INTERVAL_EVENT')",
        "UPDATE clinical.node \
            SET event_time = ext.openehr_timestamp( \
                CASE WHEN origin_compact THEN data ->> 'origin' ELSE data #>> '{origin,value}' END) \
          WHERE rm_type = 'HISTORY'",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Rewrites the time of every fifth event and the origin of every fifth
/// history, counted in the stored order, to a text no ISO 8601 reading has,
/// which the commit path refuses and only a row stored by an earlier release
/// can hold, and fills the column as the migration does.
async fn store_unreadable_times(pool: &PgPool) -> u64 {
    let mut rewritten = 0;
    for (types, key) in [
        ("'POINT_EVENT', 'INTERVAL_EVENT'", "time"),
        ("'HISTORY'", "origin"),
    ] {
        rewritten += sqlx::query(sqlx::AssertSqlSafe(format!(
            "WITH picked AS ( \
                 SELECT tier, vo_id, sys_version, num, \
                        row_number() OVER (ORDER BY vo_id, sys_version, num) AS n \
                 FROM clinical.node WHERE rm_type IN ({types})) \
             UPDATE clinical.node AS node SET data = jsonb_set(data, '{{{key}}}', \
                 CASE WHEN jsonb_typeof(data -> '{key}') = 'string' THEN '\"99 o''clock\"'::jsonb \
                 ELSE jsonb_set(data -> '{key}', '{{value}}', '\"99 o''clock\"') END) \
             FROM picked p \
             WHERE (node.tier, node.vo_id, node.sys_version, node.num) \
                   = (p.tier, p.vo_id, p.sys_version, p.num) AND p.n % 5 = 0"
        )))
        .execute(pool)
        .await
        .expect("rewrite a stored time")
        .rows_affected();
    }
    let fill = migration_fill();
    assert_eq!(fill.len(), 2, "the migration fills events and histories");
    for statement in fill {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(pool)
            .await
            .expect("fill the column as the migration does");
    }
    rewritten
}

/// The rows `aql` answers over the population, sorted.
pub(crate) async fn rows(
    svc: &FerroEhrService,
    aql: &str,
    params: BTreeMap<String, Value>,
) -> Vec<String> {
    rows_fetched(svc, aql, params, None).await
}

/// [`rows`] of an execution that fetches at most `fetch` rows, which streams.
pub(crate) async fn rows_fetched(
    svc: &FerroEhrService,
    aql: &str,
    params: BTreeMap<String, Value>,
    fetch: Option<i64>,
) -> Vec<String> {
    let request = AqlQueryRequest {
        parameters: params,
        fetch,
        ..AqlQueryRequest::default()
    };
    let result = svc
        .execute_aql(aql, None, &request)
        .await
        .unwrap_or_else(|e| panic!("{aql} failed: {e:?}"))
        .result_set
        .to_value()
        .expect("the result set renders");
    let mut rows: Vec<String> = result["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(Value::to_string)
        .collect();
    rows.sort();
    rows
}

/// The position and the `event_time` of every event and history row, in a
/// stable order.
async fn stored_times(pool: &PgPool) -> Vec<(i32, Option<String>)> {
    sqlx::query_as(
        "SELECT num, event_time::text FROM clinical.node \
         WHERE rm_type IN ('POINT_EVENT', 'INTERVAL_EVENT', 'HISTORY') \
         ORDER BY vo_id, sys_version, num",
    )
    .fetch_all(pool)
    .await
    .expect("read the stored times")
}

/// The session time zone every pool connection runs under.
async fn session_time_zone(pool: &PgPool) -> String {
    sqlx::query_scalar("SHOW TimeZone")
        .fetch_one(pool)
        .await
        .expect("read the session time zone")
}

/// Every comparison answers the same rows with its anchor and without it.
///
/// `X OR X` is the comparison `X` with no anchor: a conjunct under `OR`
/// implies nothing on its own. Both run over events and histories whose
/// times cover each stored form, so a time the column reads differently
/// from the comparison would drop or add a row here.
#[tokio::test]
async fn an_anchored_comparison_answers_the_rows_the_unanchored_one_does() {
    let (db, svc) = crate::fixtures::service().await;
    let committed = seed_times(&svc).await;
    // NOTE: PostgreSQL 18 "Date/Time Types" §Time Zones: a time without a zone is read in
    // the session's TimeZone, so the stored column equals the comparison's reading only
    // while every session runs under one zone, which sqlx sets to UTC at startup.
    assert_eq!(session_time_zone(&db.pool()).await, "UTC");
    // The migration's fill and the insert agree on every row the commits
    // stored: running the fill over them changes nothing.
    let stored: Vec<(i32, Option<String>)> = stored_times(&db.pool()).await;
    let unreadable = store_unreadable_times(&db.pool()).await;
    assert!(unreadable > 10, "{unreadable} times are unreadable");
    let filled = stored_times(&db.pool()).await;
    assert_eq!(stored.len(), filled.len());
    let changed = stored.iter().zip(&filled).filter(|(a, b)| a != b).count();
    assert_eq!(
        u64::try_from(changed).expect("a count fits"),
        unreadable,
        "the fill changes the rewritten rows alone, to NULL"
    );

    let sources = [
        (FROM_OBSERVATION.to_owned(), EVENT_LEAF),
        (FROM_OBSERVATION.to_owned(), ORIGIN_LEAF),
        (
            "FROM EHR e CONTAINS COMPOSITION c CONTAINS POINT_EVENT s".to_owned(),
            "s/time/value",
        ),
        (
            "FROM EHR e CONTAINS COMPOSITION c CONTAINS INTERVAL_EVENT s".to_owned(),
            "s/time/value",
        ),
        (
            "FROM EHR e CONTAINS COMPOSITION c CONTAINS HISTORY s".to_owned(),
            "s/origin/value",
        ),
    ];
    let mut answered = 0_usize;
    let mut partial = 0_usize;
    for (from, leaf) in &sources {
        let all = rows(
            &svc,
            &format!("SELECT c/uid/value, {leaf} {from}"),
            BTreeMap::new(),
        )
        .await
        .len();
        for op in ["=", "<", "<=", ">", ">="] {
            for bound in BOUNDS {
                for (comparison, params) in [
                    (format!("{leaf} {op} '{bound}'"), BTreeMap::new()),
                    (
                        format!("{leaf} {op} $t"),
                        BTreeMap::from([("t".to_owned(), json!(bound))]),
                    ),
                ] {
                    let anchored = format!("SELECT c/uid/value, {leaf} {from} WHERE {comparison}");
                    let plain = format!(
                        "SELECT c/uid/value, {leaf} {from} WHERE {comparison} OR {comparison}"
                    );
                    let bound_params =
                        Params::default().with("t", ParamValue::Str((*bound).to_owned()));
                    assert_eq!(time_anchors(&lowered(&anchored, &bound_params)), 1);
                    assert_eq!(time_anchors(&lowered(&plain, &bound_params)), 0);
                    let with = rows(&svc, &anchored, params.clone()).await;
                    let without = rows(&svc, &plain, params.clone()).await;
                    assert_eq!(
                        with, without,
                        "{anchored}: the anchor changes no row ({committed} compositions)"
                    );
                    // The streaming shape, whose anchor is the first row of a version.
                    let streamed = rows_fetched(&svc, &anchored, params, Some(10_000)).await;
                    assert_eq!(streamed, without, "{anchored}: nor does it when it streams");
                    answered += with.len();
                    partial += usize::from(!with.is_empty() && with.len() < all);
                }
            }
        }
    }
    assert!(
        answered > 0 && partial > 100,
        "the comparisons split the store: {answered} rows, {partial} partial answers"
    );
    // A window is one anchor of two bounds: it keeps the versions with a row
    // inside it, as the two comparisons of the one bound row do.
    for (from, leaf) in &sources {
        for (lower, upper) in [
            ("2026-03-01T10:00:00Z", "2026-03-01T10:00:00.250Z"),
            ("2026", "2027"),
            ("2000", "2030"),
            ("2026-03", "2026-03-02"),
        ] {
            let window = format!("{leaf} >= '{lower}' AND {leaf} < '{upper}'");
            let plain = format!(
                "({leaf} >= '{lower}' OR {leaf} >= '{lower}') \
                 AND ({leaf} < '{upper}' OR {leaf} < '{upper}')"
            );
            let anchored = format!("SELECT c/uid/value, {leaf} {from} WHERE {window}");
            let unanchored = format!("SELECT c/uid/value, {leaf} {from} WHERE {plain}");
            let without = rows(&svc, &unanchored, BTreeMap::new()).await;
            assert_eq!(
                rows(&svc, &anchored, BTreeMap::new()).await,
                without,
                "{anchored}: the window's anchor changes no row"
            );
            assert_eq!(
                rows_fetched(&svc, &anchored, BTreeMap::new(), Some(10_000)).await,
                without,
                "{anchored}: nor does it when it streams"
            );
        }
    }
}

/// The values the magnitude store commits in its one ELEMENT: a
/// `DV_QUANTITY` of a magnitude and units alone, which the typed columns
/// hold, one with a precision, which stays in the fragment, a `DV_COUNT`,
/// and the values with a number that is no magnitude.
fn magnitude_values() -> Vec<Value> {
    let mut values = Vec::new();
    for magnitude in [
        -40.5, -1.0, 0.0, 0.5, 7.0, 7.5, 7.500_001, 8.0, 120.0, 1.0e9,
    ] {
        values.push(json!({ "_type": "DV_QUANTITY", "magnitude": magnitude, "units": "mm[Hg]" }));
        values.push(json!({
            "_type": "DV_QUANTITY", "magnitude": magnitude, "units": "mm[Hg]", "precision": 6
        }));
    }
    for magnitude in [-3, 0, 7, 8, 9_000_000_000_i64] {
        values.push(json!({ "_type": "DV_COUNT", "magnitude": magnitude }));
    }
    values.push(json!({
        "_type": "DV_ORDINAL",
        "value": 8,
        "symbol": {
            "_type": "DV_CODED_TEXT",
            "value": "Severe",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": "at0008"
            }
        }
    }));
    values.push(json!({ "_type": "DV_TEXT", "value": "8" }));
    values
        .push(json!({ "_type": "DV_PROPORTION", "numerator": 8.0, "denominator": 1.0, "type": 1 }));
    values
}

/// Every ordering comparison of a magnitude answers the same rows with its
/// anchor and without it, over the typed columns and the fragment alike.
#[tokio::test]
async fn an_anchored_magnitude_comparison_answers_the_rows_the_unanchored_one_does() {
    let (db, svc) = crate::fixtures::service().await;
    let values = magnitude_values();
    for (index, value) in values.iter().enumerate() {
        let ehr = create_ehr(&svc).await;
        let other = &values[(index + 3) % values.len()];
        let body = timed_composition(
            &format!("value {index}"),
            &date_time("2026-03-01T10:00:00Z", false),
            &[
                event_holding("2026-03-01T10:00:00Z", false, false, value),
                event_holding("2026-03-01T11:00:00Z", false, true, other),
            ],
        );
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .unwrap_or_else(|e| panic!("commit {value}: {e:?}"));
    }
    let (typed, kept): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE value_type IS NOT NULL), \
                count(*) FILTER (WHERE value_type IS NULL) \
         FROM clinical.node WHERE rm_type = 'ELEMENT' \
           AND ext.node_magnitude(value_type, value_num, data) IS NOT NULL",
    )
    .fetch_one(&db.pool())
    .await
    .expect("count the stored magnitudes");
    assert!(
        typed >= 30 && kept >= 20,
        "the store holds magnitudes in the typed columns ({typed}) and in the fragment ({kept})"
    );

    let sources = [
        (FROM_OBSERVATION.to_owned(), MAGNITUDE_LEAF),
        (
            "FROM EHR e CONTAINS COMPOSITION c CONTAINS ELEMENT s[at0004]".to_owned(),
            "s/value/magnitude",
        ),
        (
            "FROM EHR e CONTAINS COMPOSITION c CONTAINS ELEMENT s".to_owned(),
            "s/value/magnitude",
        ),
    ];
    let mut partial = 0_usize;
    for (from, leaf) in &sources {
        let all = rows(
            &svc,
            &format!("SELECT c/uid/value, {leaf} {from}"),
            BTreeMap::new(),
        )
        .await
        .len();
        for op in ["<", "<=", ">", ">="] {
            for bound in [
                json!(-41),
                json!(0),
                json!(7),
                json!(7.5),
                json!(8),
                json!(1.0e9),
            ] {
                for (comparison, params) in [
                    (format!("{leaf} {op} {bound}"), BTreeMap::new()),
                    (
                        format!("{leaf} {op} $x"),
                        BTreeMap::from([("x".to_owned(), bound.clone())]),
                    ),
                ] {
                    let anchored = format!("SELECT c/uid/value, {leaf} {from} WHERE {comparison}");
                    let plain = format!(
                        "SELECT c/uid/value, {leaf} {from} WHERE {comparison} OR {comparison}"
                    );
                    let bound_params = Params::default().with(
                        "x",
                        ParamValue::Real(bound.as_f64().expect("a numeric bound")),
                    );
                    assert_eq!(magnitude_anchors(&lowered(&anchored, &bound_params)), 1);
                    assert_eq!(magnitude_anchors(&lowered(&plain, &bound_params)), 0);
                    let with = rows(&svc, &anchored, params.clone()).await;
                    let without = rows(&svc, &plain, params.clone()).await;
                    assert_eq!(with, without, "{anchored}: the anchor changes no row");
                    let streamed = rows_fetched(&svc, &anchored, params, Some(10_000)).await;
                    assert_eq!(streamed, without, "{anchored}: nor does it when it streams");
                    partial += usize::from(!with.is_empty() && with.len() < all);
                }
            }
        }
    }
    assert!(
        partial > 100,
        "the comparisons split the store: {partial} partial answers"
    );
}

// ── the plan starts from the index ───────────────────────────────────────────

/// The lab compositions of the plan store, and how many of them carry an
/// event after [`RECENT`].
const LAB_COMPOSITIONS: u32 = 600;
/// One composition in this many is recent.
const RECENT_EVERY: u32 = 50;
/// The bound of the plan test's filter.
const RECENT: &str = "2026-11-10T23:00:00Z";

/// A population query for the recent results of a lab template, the shape a
/// partner's worklist runs.
const LAB_AQL: &str = "SELECT o/data[at0001]/events[at0002]/time/value, \
     o/data[at0001]/events[at0002]/data[at0003]/items[at0005]/value/value, c/composer/name, \
     a/items[at0024]/value, a/items[at0001]/value, c/uid/value, e/ehr_id/value \
     FROM EHR e CONTAINS COMPOSITION c \
     CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.laboratory_test_result.v1] \
     CONTAINS CLUSTER a[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1] \
     WHERE c/archetype_details/template_id/value = 'Generic lab test result example simple' \
     AND o/data[at0001]/events[at0002]/time/value >= $since";

/// Asserts that `plan` answered `expected` rows, entered through `index`,
/// walked the `population` compositions of the store in no node scan and
/// read fewer buffers than the store holds compositions.
fn assert_starts_from(name: &str, plan: &Value, index: &str, expected: u32, population: u32) {
    let root = plan
        .get(0)
        .expect("EXPLAIN FORMAT JSON returns a one-element array");
    let returned = root
        .pointer("/Plan/Actual Rows")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its rows");
    assert!(
        (returned - f64::from(expected)).abs() < f64::EPSILON,
        "{name} answers the {expected} rows the filter keeps: {returned} rows"
    );
    let scans = scans(plan);
    assert!(
        scans
            .iter()
            .any(|scan| scan.index.as_deref() == Some(index)),
        "{name} starts from {index}:\n{}",
        render(&scans)
    );
    let population = f64::from(population);
    for scan in on(&scans, "node_hot") {
        assert!(
            scan.rows * scan.loops < population,
            "no node scan of {name} walks the {population} compositions of the store, \
             {} read {} rows over {} loops:\n{}",
            scan.node_type,
            scan.rows,
            scan.loops,
            render(&scans)
        );
    }
    let buffers = root
        .pointer("/Plan/Shared Hit Blocks")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its buffers");
    assert!(
        buffers < population,
        "{name} reads fewer buffers than the store holds compositions ({buffers}):\n{}",
        render(&scans)
    );
}

/// A population filter on an event's time reads the events the filter keeps.
///
/// Without an access path the statement read every OBSERVATION of the
/// archetype, walked to its event row and compared the time last, 17 buffers
/// a lab composition whatever the filter kept. It now starts from
/// `idx_node_hot_event_time`: no node scan walks the store's compositions,
/// and the statement reads fewer buffers than the store holds compositions.
#[tokio::test]
async fn a_population_filter_on_an_event_time_starts_from_the_time_index() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let lab = ckm_example(&svc, "generic-lab-test-result").await;
    let mut ehrs = Vec::new();
    for _ in 0..30 {
        ehrs.push(create_ehr(&svc).await);
    }
    let mut recent = 0_u32;
    for index in 0..LAB_COMPOSITIONS {
        let mut body = lab.clone();
        let time = if index % RECENT_EVERY == 0 {
            recent += 1;
            format!("2026-11-{:02}T08:00:00Z", 11 + index % 7)
        } else {
            format!(
                "202{}-0{}-1{}T08:00:00Z",
                index % 6,
                1 + index % 9,
                index % 9
            )
        };
        *body
            .pointer_mut("/content/0/data/events/0/time/value")
            .expect("the lab example holds one event") = Value::String(time);
        let ehr = ehrs[usize::try_from(index).expect("an index fits") % ehrs.len()];
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("seed a lab composition");
    }
    analyze_as_autovacuum(&pool).await;
    ferroehr::db::parent_statistics::refresh_pools(
        &ferroehr::db::domain::DomainPools::from_shared(&pool),
    )
    .await
    .expect("refresh the partitioned parents' statistics");

    let params = Params::default().with("since", ParamValue::Str(RECENT.to_owned()));
    for (name, limit) in [
        ("the worklist", None),
        ("the worklist with a limit", Some(501)),
    ] {
        let plan = explain_aql(&pool, aql_sql_with(LAB_AQL, &params, Vec::new(), limit)).await;
        assert_starts_from(
            name,
            &plan,
            "idx_node_hot_event_time",
            recent,
            LAB_COMPOSITIONS,
        );
    }
    // A bound that keeps every result: the statement still stops at its
    // LIMIT, reading the index row by row and no scan to its end.
    let every = Params::default().with("since", ParamValue::Str("2000-01-01".to_owned()));
    let plan = explain_aql(&pool, aql_sql_with(LAB_AQL, &every, Vec::new(), Some(50))).await;
    let returned = plan
        .pointer("/0/Plan/Actual Rows")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its rows");
    assert!((returned - 50.0).abs() < f64::EPSILON, "{returned} rows");
    let scans = scans(&plan);
    for scan in on(&scans, "node_hot") {
        assert!(
            scan.rows * scan.loops < f64::from(LAB_COMPOSITIONS),
            "a filter that keeps every result stops at the LIMIT, {} read {} rows over {} \
             loops:\n{}",
            scan.node_type,
            scan.rows,
            scan.loops,
            render(&scans)
        );
    }
    // The unfiltered statement still answers every lab composition.
    let every = explain_aql(
        &pool,
        aql_sql(
            &LAB_AQL.replace(
                " AND o/data[at0001]/events[at0002]/time/value >= $since",
                "",
            ),
            Vec::new(),
        ),
    )
    .await;
    let returned = every
        .pointer("/0/Plan/Actual Rows")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its rows");
    assert!(
        (returned - f64::from(LAB_COMPOSITIONS)).abs() < f64::EPSILON,
        "the store holds {LAB_COMPOSITIONS} lab results: {returned} rows"
    );
}

/// A population filter on a magnitude reads the ELEMENTs the filter keeps.
///
/// Without an access path the statement read every OBSERVATION of the
/// archetype, walked to its ELEMENT and compared the number last. It now
/// starts from `idx_node_hot_value_magnitude`.
#[tokio::test]
async fn a_population_filter_on_a_magnitude_starts_from_the_magnitude_index() {
    const COMPOSITIONS: u32 = 600;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let mut ehrs = Vec::new();
    for _ in 0..30 {
        ehrs.push(create_ehr(&svc).await);
    }
    let mut high = 0_u32;
    for index in 0..COMPOSITIONS {
        let magnitude = if index % RECENT_EVERY == 0 {
            high += 1;
            1000.0 + f64::from(index)
        } else {
            60.0 + f64::from(index % 90)
        };
        let body = wide_observation_composition(&format!("reading {index}"), magnitude);
        let ehr = ehrs[usize::try_from(index).expect("an index fits") % ehrs.len()];
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("seed a reading");
    }
    analyze_as_autovacuum(&pool).await;
    ferroehr::db::parent_statistics::refresh_pools(
        &ferroehr::db::domain::DomainPools::from_shared(&pool),
    )
    .await
    .expect("refresh the partitioned parents' statistics");

    let leaf = wide_leaf("at0004");
    let aql = format!(
        "SELECT c/uid/value, {leaf}, e/ehr_id/value FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[{WIDE_ARCHETYPE}] WHERE {leaf} > $over"
    );
    let params = Params::default().with("over", ParamValue::Int(900));
    for (name, limit) in [("the filter", None), ("the filter with a limit", Some(501))] {
        let plan = explain_aql(&pool, aql_sql_with(&aql, &params, Vec::new(), limit)).await;
        assert_starts_from(
            name,
            &plan,
            "idx_node_hot_value_magnitude",
            high,
            COMPOSITIONS,
        );
    }
    let unanchored = aql.replace(
        &format!("WHERE {leaf} > $over"),
        &format!("WHERE {leaf} > $over OR {leaf} > $over"),
    );
    let plan = explain_aql(&pool, aql_sql_with(&unanchored, &params, Vec::new(), None)).await;
    let buffers = plan
        .pointer("/0/Plan/Shared Hit Blocks")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its buffers");
    assert!(
        buffers > f64::from(COMPOSITIONS),
        "the same filter without its anchor reads the store: {buffers} buffers"
    );
}

// ── a store grown past its statistics ────────────────────────────────────────

/// The OBSERVATION archetype the stale-statistics store commits only after
/// its last `ANALYZE`, so the statistics hold none of its rows.
const LATE_ARCHETYPE: &str = "openEHR-EHR-OBSERVATION.late_readings.v1";

/// The relations autovacuum is held off while the store grows, so their
/// statistics stay those of the first seed.
const GROWN_RELATIONS: &str = "SELECT format('%I.%I', n.nspname, c.relname) \
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relkind = 'r' AND c.relname IN ('version_hot', 'node_hot', 'vo_head') \
     ORDER BY 1";

/// A composition of [`wide_observation_composition`]'s shape under
/// [`LATE_ARCHETYPE`].
fn late_composition(name: &str, magnitude: f64) -> Value {
    let mut body = wide_observation_composition(name, magnitude);
    let observation = body
        .pointer_mut("/content/0")
        .expect("the wide composition holds one OBSERVATION");
    observation["archetype_node_id"] = json!(LATE_ARCHETYPE);
    observation["archetype_details"]["archetype_id"]["value"] = json!(LATE_ARCHETYPE);
    body
}

/// `body` with the units of its first ELEMENT, `at0004`, in kilopascal, a
/// value the statistics of the first seed never saw.
fn in_kilopascal(mut body: Value) -> Value {
    *body
        .pointer_mut("/content/0/data/events/0/data/items/0/value/units")
        .expect("the wide composition's first ELEMENT is a quantity") = json!("kPa");
    body
}

/// `EXPLAIN` a lowered AQL statement under the server's planner settings,
/// sequential scans included.
async fn explain_as_served(pool: &PgPool, prepared: ferroehr::aql::sql::PreparedQuery) -> Value {
    sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("{EXPLAIN}{}", prepared.sql)),
        prepared.values,
    )
    .fetch_one(pool)
    .await
    .expect("explain the AQL statement")
}

/// The node rows a composition of the stale-statistics store holds at most:
/// the composition, the OBSERVATION, its history, event and tree, and the
/// eight ELEMENTs, with room for the EHR's own.
const NODES_PER_VERSION: f64 = 16.0;

/// Asserts that `plan` answered `expected` rows, read fewer buffers than the
/// store holds node rows, and that no scan of a node, head or version
/// partition ran more loops than twice the `population` of versions or read
/// more rows over its loops than the store holds node rows: a plan linear in
/// the store, where a probe rescanned per outer row is quadratic.
fn assert_linear(name: &str, plan: &Value, expected: u32, population: u32) {
    let returned = plan
        .pointer("/0/Plan/Actual Rows")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its rows");
    assert!(
        (returned - f64::from(expected)).abs() < f64::EPSILON,
        "{name} answers {expected} rows: {returned}"
    );
    let population = f64::from(population);
    let buffers = plan
        .pointer("/0/Plan/Shared Hit Blocks")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its buffers");
    let scans = scans(plan);
    assert!(
        buffers <= NODES_PER_VERSION * population,
        "{name} reads {buffers} buffers, more than the store holds node rows:\n{}",
        render(&scans)
    );
    for scan in scans
        .iter()
        .filter(|scan| ["node_hot", "vo_head", "version_hot"].contains(&scan.relation.as_str()))
    {
        assert!(
            scan.loops <= 2.0 * population
                && scan.rows * scan.loops <= NODES_PER_VERSION * population,
            "{name}: {} on {} via {} ran {} loops for {} rows each, past a plan linear in \
             the {population} versions of the store:\n{}",
            scan.node_type,
            scan.relation,
            scan.index.as_deref().unwrap_or("(no index)"),
            scan.loops,
            scan.rows,
            render(&scans)
        );
    }
}

/// A population filter on a value keeps a plan linear in the store when
/// the store grew past its statistics, ordered, streamed and only in WHERE
/// (#6243, #6263).
///
/// cdr-benchmark's `bp_data_filtered` ran for 23 s on a store filling from
/// empty: the statistics held no blood-pressure OBSERVATION, the planner
/// estimated them at one row, and it rescanned the anchor, a set of distinct
/// versions no join could probe, with its head and version probes, once per
/// OBSERVATION. The streamed form's earlier-row probe went the same way here:
/// estimated at one row, it entered through the magnitude index once per
/// candidate row, 2.3 M buffers for 500 rows. The flat anchor is now the
/// first row of each version, which a join probes per version, and the
/// earlier-row probe reads its version through `node_hot_pkey`.
///
/// An equality whose leaf is only in WHERE went the same way through the
/// containment: the OBSERVATION node, estimated at one row, was rescanned
/// with the clause's `EXISTS` per anchor version, 32.7 M buffers, because the
/// probe per version was costed from the object count of the first seed. An
/// anchor on a leaf no SELECT column binds now reads each contained node
/// inside its object, behind a fence (#6263).
///
/// The store is analysed holding only low readings in mm[Hg], then grows by
/// the late archetype's compositions and by high readings in kPa with
/// autovacuum held off.
/// Each statement's scans stay linear in the store, before and after an
/// `ANALYZE`, and it answers the rows its unanchored form answers.
#[tokio::test]
async fn a_value_filter_stays_linear_on_a_store_grown_past_its_statistics() {
    const SEEDED: u32 = 2000;
    const LATE: u32 = 1200;
    const FETCH: u32 = 500;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let pools = ferroehr::db::domain::DomainPools::from_shared(&pool);
    let svc = FerroEhrService::new(&pools);
    let mut ehrs = Vec::new();
    for _ in 0..30 {
        ehrs.push(create_ehr(&svc).await);
    }
    let relations: Vec<String> = sqlx::query_scalar(GROWN_RELATIONS)
        .fetch_all(&pool)
        .await
        .expect("name the relations the store grows");
    for relation in relations {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {relation} SET (autovacuum_enabled = off)"
        )))
        .execute(&pool)
        .await
        .expect("hold autovacuum off the relations the store grows");
    }
    ferroehr::db::parent_statistics::refresh_pools(&pools)
        .await
        .expect("analyze the partitioned parents of the empty store, as the server's start does");
    for index in 0..SEEDED {
        let body = wide_observation_composition(&format!("seed {index}"), f64::from(index % 100));
        let ehr = ehrs[usize::try_from(index).expect("an index fits") % ehrs.len()];
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("seed a low reading");
    }
    analyze_as_autovacuum(&pool).await;
    let mut kept = 0_u32;
    for index in 0..LATE {
        let magnitude = 60.0 + f64::from(index * 7 % 200);
        kept += u32::from(magnitude >= 140.0);
        let ehr = ehrs[usize::try_from(index).expect("an index fits") % ehrs.len()];
        let late = in_kilopascal(late_composition(&format!("late {index}"), magnitude));
        svc.create_composition(ehr, uv(&late, "249", None))
            .await
            .expect("commit a late reading");
        let high = in_kilopascal(wide_observation_composition(
            &format!("high {index}"),
            140.0 + f64::from(index),
        ));
        svc.create_composition(ehr, uv(&high, "249", None))
            .await
            .expect("commit a high reading");
    }
    let population = SEEDED + 2 * LATE + 30;
    let expected = kept.min(FETCH);

    let leaf = wide_leaf("at0004");
    let streamed = format!(
        "SELECT c/uid/value, {leaf} FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[{LATE_ARCHETYPE}] WHERE {leaf} >= 140"
    );
    let ordered = format!("{streamed} ORDER BY {leaf} DESC");
    let units = wide_leaf("at0004").replace("/magnitude", "/units");
    let from =
        format!("FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{LATE_ARCHETYPE}]");
    let range_in_where =
        format!("SELECT c/uid/value {from} WHERE {leaf} >= 140 ORDER BY c/uid/value");
    let equality_in_where =
        format!("SELECT c/uid/value {from} WHERE {units} = 'kPa' ORDER BY c/uid/value");
    let reads = [
        ("the ordered filter", &ordered, expected),
        ("the streamed filter", &streamed, expected),
        ("the range only in WHERE", &range_in_where, expected),
        (
            "the equality only in WHERE",
            &equality_in_where,
            LATE.min(FETCH),
        ),
    ];
    let fetched =
        |aql: &str| aql_sql_with(aql, &Params::default(), Vec::new(), Some(i64::from(FETCH)));
    for (name, aql, expected) in reads {
        let plan = explain_as_served(&pool, fetched(aql)).await;
        assert_linear(
            &format!("{name} on stale statistics"),
            &plan,
            expected,
            population,
        );
    }
    let answered = rows(&svc, &ordered, BTreeMap::new()).await;
    let unanchored = ordered.replace(
        &format!("WHERE {leaf} >= 140"),
        &format!("WHERE {leaf} >= 140 OR {leaf} >= 140"),
    );
    assert_eq!(
        answered,
        rows(&svc, &unanchored, BTreeMap::new()).await,
        "the anchor changes no row"
    );
    assert_eq!(
        usize::try_from(kept).expect("a small count"),
        answered.len(),
        "the filter keeps the late readings at or above 140"
    );

    analyze_as_autovacuum(&pool).await;
    ferroehr::db::parent_statistics::refresh_pools(&pools)
        .await
        .expect("refresh the partitioned parents' statistics");
    for (name, aql, expected) in reads {
        let plan = explain_as_served(&pool, fetched(aql)).await;
        assert_linear(
            &format!("{name} on fresh statistics"),
            &plan,
            expected,
            population,
        );
    }
    assert_eq!(
        rows(&svc, &ordered, BTreeMap::new()).await,
        answered,
        "fresh statistics change no row"
    );
}
