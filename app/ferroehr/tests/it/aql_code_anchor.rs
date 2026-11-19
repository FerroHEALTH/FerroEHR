// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An equality on the code of an ELEMENT value enters through an index and
//! answers the rows it answered before (#6033).
//!
//! `ext.node_code` reads the code of a coded text, or of the symbol of an
//! ordinal or a scale, from the place the row holds it. The AQL engine adds,
//! to a top-level equality of such a leaf, the versions some ELEMENT row of
//! which carries the code (an anchor). The comparison itself is unchanged, so
//! the anchor may only remove work. The tests hold the three halves: which
//! comparisons get the anchor, that the anchored statement answers what the
//! unanchored one does over the typed columns and the fragment, and that the
//! plan enters through the index. AQL QUERY master03 §Comparison operators
//! governs the comparison; no openEHR spec governs access paths: our own
//! design.

use std::collections::BTreeMap;

use ferroehr::aql::ir::{ParamValue, Params};
use ferroehr::service::FerroEhrService;
use serde_json::{Value, json};
use sqlx::Executor as _;

use crate::aql_range_anchor::{
    FROM_OBSERVATION, date_time, event_holding, lowered, rows, rows_fetched, timed_composition,
};
use crate::fixtures::{create_ehr, uv};
use crate::storage_plans::{
    WIDE_ARCHETYPE, aql_sql_with, assert_lookup_through, explain_aql, wide_observation_composition,
};

/// The path from the OBSERVATION `o` to the one ELEMENT of an event.
const ELEMENT: &str = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]";

/// The two leaves `ext.node_code` reads: the code of a coded text and the
/// code of a symbol.
const LEAVES: [&str; 2] = [
    "value/defining_code/code_string",
    "value/symbol/defining_code/code_string",
];

/// How many code anchors `sql` carries: the comparison reads the leaf through
/// the typed columns, so only an anchor calls the function.
fn code_anchors(sql: &str) -> usize {
    sql.matches("node_code(").count()
}

/// A top-level equality of either code leaf is anchored, against a literal
/// and against a parameter, from the OBSERVATION and from an ELEMENT source.
#[test]
fn an_equality_of_a_code_is_anchored() {
    let params = Params::default().with("code", ParamValue::Str("at0008".to_owned()));
    for leaf in LEAVES {
        for filter in [
            format!("{ELEMENT}/{leaf} = 'at0008'"),
            format!("{ELEMENT}/{leaf} = $code"),
        ] {
            let sql = lowered(
                &format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {filter}"),
                &params,
            );
            assert_eq!(code_anchors(&sql), 1, "{filter}: one anchor\n{sql}");
            assert!(
                sql.contains(r#"_n"."rm_type" = ('ELEMENT')"#)
                    && sql.contains(r#""archetype" = $"#),
                "{filter}: the anchor reads the ELEMENT rows of the leaf step's at-code\n{sql}"
            );
        }
        let element = lowered(
            &format!(
                "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS ELEMENT \
                 el[at0004] WHERE el/{leaf} = 'at0008'"
            ),
            &params,
        );
        assert_eq!(code_anchors(&element), 1, "{element}");
    }
}

/// An inequality, a conjunct under `OR` or `NOT`, a comparison with a number
/// or with another path, and the terminology of the code imply no anchor.
#[test]
fn only_a_top_level_equality_of_a_code_is_anchored() {
    let code = format!("{ELEMENT}/value/defining_code/code_string");
    for filter in [
        format!("{code} != 'at0008'"),
        format!("{code} = 'at0008' OR {code} = 'at0009'"),
        format!("NOT {code} = 'at0008'"),
        format!("{code} = 8"),
        format!("{code} = {ELEMENT}/value/value"),
        format!("{ELEMENT}/value/defining_code/terminology_id/value = 'local'"),
    ] {
        let sql = lowered(
            &format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {filter}"),
            &Params::default(),
        );
        assert_eq!(code_anchors(&sql), 0, "{filter}: no anchor\n{sql}");
    }
    let both = lowered(
        &format!(
            "SELECT c/uid/value {FROM_OBSERVATION} WHERE {code} = 'at0008' AND {code} = 'at0009'"
        ),
        &Params::default(),
    );
    assert_eq!(
        code_anchors(&both),
        2,
        "each conjunct has its anchor\n{both}"
    );
}

/// A `CODE_PHRASE` of the terminology `local`.
fn code_phrase(code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
        "code_string": code
    })
}

/// A `DV_CODED_TEXT` of `code`.
fn coded_text(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": format!("rubric {code}"),
        "defining_code": code_phrase(code) })
}

/// The values the equality store commits: coded texts and ordinals the typed
/// columns hold, the same with a member that keeps them in the fragment, a
/// scale, and values that carry no code.
fn coded_values() -> Vec<Value> {
    let mut values = Vec::new();
    for code in ["at0007", "at0008", "at0009", "8", "AT0008", "at0008 "] {
        values.push(coded_text(code));
        let mut kept = coded_text(code);
        kept["formatting"] = json!("plain");
        values.push(kept);
        values.push(json!({ "_type": "DV_ORDINAL", "value": 8, "symbol": coded_text(code) }));
        values.push(json!({
            "_type": "DV_ORDINAL", "value": 8, "symbol": coded_text(code),
            "normal_status": code_phrase("N")
        }));
        values.push(json!({ "_type": "DV_SCALE", "value": 8.5, "symbol": coded_text(code) }));
    }
    values.push(json!({ "_type": "DV_TEXT", "value": "at0008" }));
    values.push(json!({ "_type": "DV_QUANTITY", "magnitude": 8.0, "units": "mm[Hg]" }));
    values.push(json!({ "_type": "DV_IDENTIFIER", "id": "at0008" }));
    values
}

/// Every equality of a code answers the same rows with its anchor and
/// without it, over the typed columns and the fragment alike.
#[tokio::test]
async fn an_anchored_code_equality_answers_the_rows_the_unanchored_one_does() {
    let (db, svc) = crate::fixtures::service().await;
    let values = coded_values();
    for (index, value) in values.iter().enumerate() {
        let ehr = create_ehr(&svc).await;
        let other = &values[(index + 7) % values.len()];
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
           AND ext.node_code(value_type, value_code, data) IS NOT NULL",
    )
    .fetch_one(&db.pool())
    .await
    .expect("count the stored codes");
    assert!(
        typed >= 20 && kept >= 30,
        "the store holds codes in the typed columns ({typed}) and in the fragment ({kept})"
    );

    let sources = [
        (FROM_OBSERVATION.to_owned(), ELEMENT.to_owned()),
        (
            "FROM EHR e CONTAINS COMPOSITION c CONTAINS ELEMENT s[at0004]".to_owned(),
            "s".to_owned(),
        ),
        (
            "FROM EHR e CONTAINS COMPOSITION c CONTAINS ELEMENT s".to_owned(),
            "s".to_owned(),
        ),
    ];
    let mut partial = 0_usize;
    for (from, element) in &sources {
        for leaf in LEAVES {
            let leaf = format!("{element}/{leaf}");
            let all = rows(
                &svc,
                &format!("SELECT c/uid/value, {leaf} {from}"),
                BTreeMap::new(),
            )
            .await
            .len();
            for code in ["at0007", "at0008", "8", "AT0008", "at0008 ", "absent", ""] {
                for (comparison, params) in [
                    (format!("{leaf} = '{code}'"), BTreeMap::new()),
                    (
                        format!("{leaf} = $code"),
                        BTreeMap::from([("code".to_owned(), json!(code))]),
                    ),
                ] {
                    let anchored = format!("SELECT c/uid/value, {leaf} {from} WHERE {comparison}");
                    let plain = format!(
                        "SELECT c/uid/value, {leaf} {from} WHERE {comparison} OR {comparison}"
                    );
                    let bound = Params::default().with("code", ParamValue::Str(code.to_owned()));
                    assert_eq!(code_anchors(&lowered(&anchored, &bound)), 1, "{anchored}");
                    assert_eq!(code_anchors(&lowered(&plain, &bound)), 0, "{plain}");
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
        partial >= 40,
        "the equalities split the store: {partial} partial answers"
    );
}

/// The at-code of the one coded ELEMENT of the plan store's compositions.
const CODED_ITEM: &str = "at0005";

/// A lookup of a composition by the code of an ELEMENT value reads the one
/// composition, through `idx_node_hot_value_code`, on the flat shape and on
/// the streaming one, and reads the store without its anchor.
#[tokio::test]
async fn a_lookup_by_an_element_code_reads_the_one_composition() {
    const COMPOSITIONS: u32 = 400;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    for index in 0..COMPOSITIONS {
        let ehr = create_ehr(&svc).await;
        let mut body = wide_observation_composition(&format!("finding {index}"), 0.0);
        *body
            .pointer_mut("/content/0/data/events/0/data/items")
            .expect("the observation fixture holds its items") = json!([{
            "_type": "ELEMENT",
            "name": { "_type": "DV_TEXT", "value": "finding" },
            "archetype_node_id": CODED_ITEM,
            "value": coded_text(&format!("code-{index}"))
        }]);
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("commit a finding");
    }
    pool.execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded corpus");

    let code = format!(
        "o/data[at0001]/events[at0002]/data[at0003]/items[{CODED_ITEM}]/value/defining_code/code_string"
    );
    let aql = format!(
        "SELECT c/uid/value, {code} FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[{WIDE_ARCHETYPE}] WHERE {code} = $code"
    );
    let params = Params::default().with(
        "code",
        ParamValue::Str(format!("code-{}", COMPOSITIONS.div_euclid(2))),
    );
    for (name, limit) in [("the lookup", None), ("the lookup with a limit", Some(10))] {
        let plan = explain_aql(&pool, aql_sql_with(&aql, &params, Vec::new(), limit)).await;
        assert_lookup_through(name, &plan, COMPOSITIONS, "idx_node_hot_value_code");
    }
    let unanchored = aql.replace(
        &format!("WHERE {code} = $code"),
        &format!("WHERE {code} = $code OR {code} = $code"),
    );
    let plan = explain_aql(&pool, aql_sql_with(&unanchored, &params, Vec::new(), None)).await;
    let buffers = plan
        .pointer("/0/Plan/Shared Hit Blocks")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its buffers");
    assert!(
        buffers > f64::from(COMPOSITIONS),
        "the same lookup without its anchor reads the store: {buffers} buffers"
    );
}
