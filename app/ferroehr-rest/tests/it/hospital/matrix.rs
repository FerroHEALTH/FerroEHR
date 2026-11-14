// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every caller of the hospital scenario against every kind of resource of
//! Eva's record: a composition read, a version read, an AQL query's rows,
//! its `COUNT` and its paging, the EHR extract export, the directory, an
//! item tag and a contribution, each compared with the expected outcome
//! table, refusal code included, and each access's entry in the access log.
//! A query's rows, its count and every page agree with what the caller may
//! read, and an export carries nothing the caller may not read. The table is
//! the scenario's; the decision procedure is our own design (#5487).

use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{BASE, EXT};
use crate::hospital::abac::ids;
use crate::hospital::build::world;
use crate::hospital::ledger::{Expect, Ledger};
use crate::hospital::world::{Answer, Ask, Doc, World, ask};

/// One kind of resource of Eva's record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resource {
    Composition,
    Version,
    Directory,
    ItemTag,
    Contribution,
    Export,
}

/// What a caller gets on the path resources.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    /// `200`, on the grounds named.
    Read(&'static [&'static str]),
    /// `200`, marked an emergency.
    Emergency,
    /// `403` with the refusal code, decided at the step.
    Refused(&'static str, u8),
    /// `404`, with the code and step the log names.
    Absent(&'static str, u8),
    /// `403` before the evaluator, naming no refusal code: the coarse grant.
    Ungranted,
}

/// One row of the table: the caller, the purpose it declares, what it gets
/// on the path resources and on the export, and the compositions its query
/// returns.
struct Row {
    caller: String,
    purpose: Option<&'static str>,
    paths: Outcome,
    export: Outcome,
    visible: Vec<String>,
}

const TREATED: Outcome = Outcome::Read(&["treatment relationship"]);

/// The expected-outcome table.
fn table(world: &World) -> Vec<Row> {
    let (enc, vit, men, hiv) = (&world.encounter, &world.vitals, &world.mental, &world.hiv);
    let all = ids(&[enc, vit, men, hiv]);
    let row = |caller: &str, purpose, paths, export, visible: Vec<String>| Row {
        caller: caller.to_owned(),
        purpose,
        paths,
        export,
        visible,
    };
    let no_care = Outcome::Refused("refused-no-care-relationship", 5);
    let no_consent = Outcome::Refused("refused-consent-required", 7);
    let restricted = Outcome::Absent("refused-restricted", 2);
    let consent = Outcome::Read(&["consent"]);
    let subject = Outcome::Read(&["subject of care"]);
    vec![
        row(
            "jansen",
            Some("TREAT"),
            TREATED,
            TREATED,
            ids(&[enc, vit, hiv]),
        ),
        row("devries", Some("TREAT"), TREATED, TREATED, ids(&[enc, vit])),
        row("smit", Some("TREAT"), TREATED, TREATED, all.clone()),
        row("bakker", Some("TREAT"), no_care, no_care, Vec::new()),
        row(
            "bakker",
            Some("ETREAT"),
            Outcome::Emergency,
            Outcome::Emergency,
            all.clone(),
        ),
        row("gina", Some("TREAT"), restricted, restricted, Vec::new()),
        row(
            "hans",
            Some("TREAT"),
            Outcome::Read(&[]),
            Outcome::Read(&[]),
            ids(&[enc, vit]),
        ),
        row(
            &format!("eva:{}", world.ehr),
            None,
            subject,
            subject,
            all.clone(),
        ),
        row(
            &format!("frank:{}", world.ehr),
            None,
            Outcome::Read(&[]),
            Outcome::Read(&[]),
            all,
        ),
        row(
            "client:hie-member",
            Some("TREAT"),
            consent,
            Outcome::Ungranted,
            ids(&[enc, vit]),
        ),
        row(
            "client:hie-outside",
            Some("TREAT"),
            no_consent,
            no_consent,
            Vec::new(),
        ),
        row(
            "client:access-service",
            Some("TREAT"),
            consent,
            Outcome::Ungranted,
            ids(&[enc, vit]),
        ),
        row(
            "client:lab",
            Some("TREAT"),
            no_consent,
            no_consent,
            Vec::new(),
        ),
    ]
}

/// The URI of `resource` in the world.
fn uri(world: &World, resource: Resource) -> String {
    let (ehr, vitals): (&str, &Doc) = (&world.ehr, &world.vitals);
    match resource {
        Resource::Composition => format!("{BASE}/ehr/{ehr}/composition/{}", vitals.vo),
        Resource::Version => format!(
            "{BASE}/ehr/{ehr}/versioned_composition/{}/version/{}",
            vitals.vo, vitals.version
        ),
        Resource::Directory => format!("{BASE}/ehr/{ehr}/directory"),
        Resource::ItemTag => format!("{BASE}/ehr/{ehr}/composition/{}/tags", vitals.vo),
        Resource::Contribution => format!("{BASE}/ehr/{ehr}/contribution/{}", world.contribution),
        Resource::Export => format!("{EXT}/message/export/{ehr}"),
    }
}

/// Asserts `answer` is `outcome`, and what its access-log entry must be.
#[track_caller]
fn judged(label: &str, answer: &Answer, outcome: Outcome) -> Expect {
    let (status, code, expect) = match outcome {
        Outcome::Read(grounds) => (StatusCode::OK, None, Expect::granted(grounds)),
        Outcome::Emergency => (
            StatusCode::OK,
            None,
            Expect::granted(&["emergency"]).emergency(),
        ),
        Outcome::Refused(code, step) => (
            StatusCode::FORBIDDEN,
            Some(code),
            Expect::refused(code, step),
        ),
        Outcome::Absent(code, step) => (StatusCode::NOT_FOUND, None, Expect::refused(code, step)),
        Outcome::Ungranted => (StatusCode::FORBIDDEN, None, Expect::granted(&[])),
    };
    assert_eq!(answer.status, status, "{label}: {}", answer.body);
    let named = code.map_or(Value::Null, |code| json!(code));
    assert_eq!(answer.refusal(), named, "{label}: {}", answer.body);
    expect
}

/// `aql` over Eva's EHR as `row`'s caller, with `paging` members; the first
/// column of each row.
async fn query(world: &World, row: &Row, aql: &str, paging: &Value) -> (Vec<Value>, Answer) {
    let mut body = json!({ "q": aql });
    if let (Some(body), Some(paging)) = (body.as_object_mut(), paging.as_object()) {
        body.extend(paging.clone());
    }
    let answer = ask(
        &world.app,
        &row.caller,
        ("POST", &format!("{BASE}/query/aql")),
        Ask {
            purpose: row.purpose,
            body: Some(&body),
            scope: Some(&world.ehr),
        },
    )
    .await;
    let status = if matches!(row.paths, Outcome::Absent(..)) {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::OK
    };
    assert_eq!(
        answer.status, status,
        "{}'s {aql}: {}",
        row.caller, answer.body
    );
    let column = answer.json()["rows"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|cells| cells[0].clone())
        .collect();
    (column, answer)
}

/// The versioned objects of a column of composition uids, sorted.
fn objects(column: &[Value]) -> Vec<String> {
    let mut ids: Vec<String> = column
        .iter()
        .filter_map(Value::as_str)
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect();
    ids.sort();
    ids
}

/// The query's rows and its pages of one row each agree with what `row`'s
/// caller may read, every query an access of its own.
async fn queries_agree(world: &World, row: &Row, ledger: &mut Ledger, expect: Expect) {
    let (column, answer) = query(world, row, SELECT, &json!({})).await;
    assert_eq!(objects(&column), row.visible, "{}'s rows", row.caller);
    ledger.note(world, format!("{} rows", row.caller), &answer, expect);
    let mut paged = Vec::new();
    for offset in 0..row.visible.len() {
        let (page, answer) =
            query(world, row, SELECT, &json!({ "offset": offset, "fetch": 1 })).await;
        assert_eq!(page.len(), 1, "{}'s page {offset}: {page:?}", row.caller);
        paged.extend(page);
        ledger.note(
            world,
            format!("{} page {offset}", row.caller),
            &answer,
            expect,
        );
    }
    assert_eq!(objects(&paged), row.visible, "{}'s pages", row.caller);
}

/// The query of every composition of an EHR.
const SELECT: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";

/// The access-log entry a row's query must write.
fn query_expect(row: &Row) -> Expect {
    match row.paths {
        Outcome::Absent(code, step) | Outcome::Refused(code, step) => Expect::refused(code, step),
        Outcome::Emergency => Expect::granted(&["emergency"]).emergency(),
        Outcome::Read(grounds) => Expect::granted(grounds),
        Outcome::Ungranted => Expect::granted(&[]),
    }
}

/// An export carries every composition the caller reads and none it does
/// not.
#[track_caller]
fn exports_only_what_it_reads(world: &World, row: &Row, answer: &Answer) {
    for doc in [&world.encounter, &world.vitals, &world.mental, &world.hiv] {
        assert_eq!(
            answer.body.contains(&doc.vo),
            row.visible.contains(&doc.vo),
            "{}'s export and {}",
            row.caller,
            doc.vo
        );
    }
}

/// Each caller against each resource kind, as the table says, with each
/// access logged once with its grounds, its code and its step.
#[tokio::test]
async fn every_caller_against_every_resource_matches_the_table() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    for row in table(&world) {
        for resource in [
            Resource::Composition,
            Resource::Version,
            Resource::Directory,
            Resource::ItemTag,
            Resource::Contribution,
        ] {
            let label = format!("{} {resource:?}", row.caller);
            let answer = ask(
                &world.app,
                &row.caller,
                ("GET", &uri(&world, resource)),
                Ask {
                    purpose: row.purpose,
                    ..Ask::default()
                },
            )
            .await;
            let expect = judged(&label, &answer, row.paths);
            ledger.note(&world, label, &answer, expect);
        }
        if !row.visible.is_empty() {
            queries_agree(&world, &row, &mut ledger, query_expect(&row)).await;
        }
    }
    ledger.verify(&world).await;
}

/// Each caller exports Eva's record as the table says: what it reads and
/// nothing else, refused as its path reads are, each export logged once
/// with its grounds.
#[tokio::test]
async fn every_caller_exports_as_the_table_says() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    for row in table(&world) {
        let label = format!("{} export", row.caller);
        let answer = ask(
            &world.app,
            &row.caller,
            ("GET", &uri(&world, Resource::Export)),
            Ask {
                purpose: row.purpose,
                ..Ask::default()
            },
        )
        .await;
        let expect = judged(&label, &answer, row.export);
        if answer.status == StatusCode::OK {
            exports_only_what_it_reads(&world, &row, &answer);
        }
        if !matches!(row.export, Outcome::Ungranted) {
            ledger.note(&world, label, &answer, expect);
        }
    }
    ledger.verify(&world).await;
}

/// Each caller's `COUNT` over Eva's record, its page past the last row, and
/// the query of a caller who reads nothing of it agree with what it may
/// read, and each is an access to her record, logged against her EHR with
/// its grounds or its refusal (#5639).
#[tokio::test]
async fn every_callers_count_and_last_page_agree_and_are_logged() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    for row in table(&world) {
        let expect = query_expect(&row);
        let (count, answer) = query(
            &world,
            &row,
            "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c",
            &json!({}),
        )
        .await;
        if !matches!(row.paths, Outcome::Absent(..)) {
            assert_eq!(
                count,
                vec![json!(row.visible.len())],
                "{}'s COUNT",
                row.caller
            );
        }
        ledger.note(&world, format!("{} COUNT", row.caller), &answer, expect);
        if row.visible.is_empty() {
            let (column, answer) = query(&world, &row, SELECT, &json!({})).await;
            assert!(column.is_empty(), "{}'s rows: {column:?}", row.caller);
            ledger.note(&world, format!("{} rows", row.caller), &answer, expect);
        }
        let past = json!({ "offset": row.visible.len(), "fetch": 1 });
        let (page, answer) = query(&world, &row, SELECT, &past).await;
        assert!(
            page.is_empty(),
            "{}'s page past the end: {page:?}",
            row.caller
        );
        ledger.note(
            &world,
            format!("{} past the end", row.caller),
            &answer,
            expect,
        );
    }
    ledger.verify(&world).await;
}
