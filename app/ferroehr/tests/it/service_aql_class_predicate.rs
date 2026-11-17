// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A comparison in a class-expression or node predicate answers as the same
//! comparison in `WHERE` (#5910), executed against a real PostgreSQL 18
//! (shared testkit harness).
//!
//! QUERY master03 §Standard predicate gives a predicate the operators of
//! §Comparison operators, so `ELEMENT el[value/magnitude > 1000]` compares
//! numbers, a date/time `value` compares instants, and a whole data value
//! compares with a literal of its own kind, never as text.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;

use crate::fixtures::{create_ehr, observation_composition, uv};

const OBS: &str = "openEHR-EHR-OBSERVATION.minimal.v1";

/// The minimal observation composition named `name` holding `value` in its
/// one ELEMENT (`at0004`).
fn holding(name: &str, value: Value) -> Value {
    let mut c = observation_composition(name, 0.0);
    c["content"][0]["data"]["events"][0]["data"]["items"][0]["value"] = value;
    c
}

fn quantity(magnitude: f64) -> Value {
    json!({ "_type": "DV_QUANTITY", "magnitude": magnitude, "units": "mm[Hg]" })
}

fn date_time(value: &str) -> Value {
    json!({ "_type": "DV_DATE_TIME", "value": value })
}

async fn seed(svc: &FerroEhrService, ehr: &EhrId, compositions: Vec<Value>) {
    for c in compositions {
        svc.create_composition(*ehr, uv(&c, "249", None))
            .await
            .unwrap_or_else(|e| panic!("create_composition: {e:?}"));
    }
}

fn request(ehr: &EhrId) -> AqlQueryRequest {
    AqlQueryRequest {
        ehr_ids: vec![*ehr],
        parameters: BTreeMap::new(),
        ..AqlQueryRequest::default()
    }
}

/// The composition names `aql` answers in its first column.
async fn names(svc: &FerroEhrService, ehr: &EhrId, aql: &str) -> BTreeSet<String> {
    let result = svc
        .execute_aql(aql, None, &request(ehr))
        .await
        .unwrap_or_else(|e| panic!("{aql}: {e:?}"))
        .result_set;
    result["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(|row| row[0].as_str().expect("a name").to_owned())
        .collect()
}

/// The names `el[<predicate>]` selects and the names `WHERE el/<where>`
/// selects, for the same comparison.
async fn both(
    svc: &FerroEhrService,
    ehr: &EhrId,
    predicate: &str,
    condition: &str,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let from = format!("FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS}]");
    let in_class = format!("SELECT c/name/value {from} CONTAINS ELEMENT el[{predicate}]");
    let in_where = format!("SELECT c/name/value {from} CONTAINS ELEMENT el WHERE el/{condition}");
    (
        names(svc, ehr, &in_class).await,
        names(svc, ehr, &in_where).await,
    )
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

/// A numeric leaf compares as a number in a class expression, as in `WHERE`
/// (the backlog report: `> 1000` kept 120, 80 and 5, `< 100` kept none).
#[tokio::test]
async fn a_magnitude_predicate_compares_numbers() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    seed(
        &svc,
        &ehr,
        vec![
            holding("q120", quantity(120.0)),
            holding("q80", quantity(80.0)),
            holding("q5", quantity(5.0)),
        ],
    )
    .await;
    let cases: [(&str, &[&str]); 6] = [
        ("value/magnitude > 1000", &[]),
        ("value/magnitude > 100", &["q120"]),
        ("value/magnitude < 100", &["q80", "q5"]),
        ("value/magnitude = 80", &["q80"]),
        ("value/magnitude >= 80", &["q120", "q80"]),
        ("value/magnitude != 5", &["q120", "q80"]),
    ];
    for (cond, want) in cases {
        let (in_class, in_where) = both(&svc, &ehr, cond, cond).await;
        assert_eq!(in_class, set(want), "el[{cond}]");
        assert_eq!(in_class, in_where, "el[{cond}] answers as WHERE el/{cond}");
    }
}

/// A whole date/time value compares as the instant it names, so an offset
/// that sorts after the bound as text is still before it in time.
#[tokio::test]
async fn a_date_time_predicate_compares_instants() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    seed(
        &svc,
        &ehr,
        vec![
            // 05:00Z, after the bound as text and before it in time
            holding("east", date_time("2020-06-01T10:00:00+05:00")),
            holding("utc", date_time("2020-06-01T07:00:00Z")),
        ],
    )
    .await;
    let cond = "value > '2020-06-01T06:00:00Z'";
    let (in_class, in_where) = both(&svc, &ehr, cond, cond).await;
    assert_eq!(in_class, set(&["utc"]), "el[{cond}]");
    assert_eq!(in_class, in_where, "el[{cond}] answers as WHERE el/{cond}");
}

/// A text leaf still compares as text, and a whole data value compares with a
/// literal of its own kind (#5751).
#[tokio::test]
async fn text_and_whole_value_predicates_answer_as_where() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    seed(
        &svc,
        &ehr,
        vec![
            holding("q120", quantity(120.0)),
            holding("q80", quantity(80.0)),
            holding("text", json!({ "_type": "DV_TEXT", "value": "Annotation" })),
        ],
    )
    .await;
    let cases: [(&str, &[&str]); 4] = [
        ("value/value = 'Annotation'", &["text"]),
        ("value/units = 'mm[Hg]'", &["q120", "q80"]),
        ("value > 100", &["q120"]),
        ("value = 'Annotation'", &[]),
    ];
    for (cond, want) in cases {
        let (in_class, in_where) = both(&svc, &ehr, cond, cond).await;
        assert_eq!(in_class, set(want), "el[{cond}]");
        assert_eq!(in_class, in_where, "el[{cond}] answers as WHERE el/{cond}");
    }
}

/// A predicate value the leaf's type cannot compare with is the caller's
/// `400`, as in `WHERE`, and so is a predicate path the RM does not define.
#[tokio::test]
async fn an_uncomparable_predicate_is_a_bad_request() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    seed(&svc, &ehr, vec![holding("q120", quantity(120.0))]).await;
    for predicate in ["value/magnitude = true", "value/no_such_attribute = 1"] {
        let aql = format!(
            "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c CONTAINS \
             OBSERVATION o[{OBS}] CONTAINS ELEMENT el[{predicate}]"
        );
        let err = svc
            .execute_aql(&aql, None, &request(&ehr))
            .await
            .expect_err("refused");
        assert!(
            matches!(&err, ServiceError::BadRequest(_)),
            "{predicate}: a typed 400, got {err:?}"
        );
    }
}
