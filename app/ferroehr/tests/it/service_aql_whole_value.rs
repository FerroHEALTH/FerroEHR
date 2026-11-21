// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL comparisons and ORDER BY keys that read a value's type, executed
//! against a real PostgreSQL 19 (shared testkit harness).
//!
//! - A whole data value compares with a literal of its own kind and with no
//!   other, never by its serialized JSON (#5751): a number with the magnitude
//!   or value of a whole numeric `DV_ORDERED`, an ISO 8601 date/time with a
//!   whole `DV_DATE`, `DV_TIME` or `DV_DATE_TIME` (QUERY master03 §Dates and
//!   Times), and it orders by the same keys (§ORDER BY).
//! - A `uid/value` predicate in a class expression reads the version's
//!   identifier, as the same path does in `WHERE` (#5749, §Standard predicate).
//! - A Boolean compared with a number is the caller's `400` (#5750).
//! - A Real of 17 significant digits compares as the number written (#5753,
//!   §Real data).
//! - A `DV_DURATION`'s `value` is a String and orders as text (#5752,
//!   §ORDER BY, §Dates and Times).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;

use crate::fixtures::{create_ehr, observation_composition, uv};

const OBS: &str = "openEHR-EHR-OBSERVATION.minimal.v1";
/// The whole `DATA_VALUE` of the one ELEMENT, which may be any of its subtypes.
const ITEM: &str = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value";

/// The minimal observation composition named `name` holding `value` at
/// [`ITEM`].
fn holding(name: &str, value: Value) -> Value {
    let mut c = observation_composition(name, 0.0);
    c["content"][0]["data"]["events"][0]["data"]["items"][0]["value"] = value;
    c
}

/// Commits each composition into `ehr`, returning the version uids in order.
async fn seed(svc: &FerroEhrService, ehr: &EhrId, compositions: Vec<Value>) -> Vec<String> {
    let mut uids = Vec::with_capacity(compositions.len());
    for c in compositions {
        let uid = svc
            .create_composition(*ehr, uv(&c, "249", None))
            .await
            .unwrap_or_else(|e| panic!("create_composition: {e:?}"))
            .version_uid();
        uids.push(uid);
    }
    uids
}

/// The request scoped to `ehr`, with `params`.
fn request(ehr: &EhrId, params: BTreeMap<String, Value>) -> AqlQueryRequest {
    AqlQueryRequest {
        ehr_ids: vec![*ehr],
        parameters: params,
        ..AqlQueryRequest::default()
    }
}

/// The first column of each row `aql` answers, in order.
async fn column(
    svc: &FerroEhrService,
    ehr: &EhrId,
    aql: &str,
    params: BTreeMap<String, Value>,
) -> Vec<Value> {
    let result = svc
        .execute_aql(aql, None, &request(ehr, params))
        .await
        .unwrap_or_else(|e| panic!("the query failed: {e:?}"))
        .result_set
        .to_value()
        .unwrap();
    result["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(|row| row[0].clone())
        .collect()
}

/// The names of the compositions whose [`ITEM`] satisfies `cond`.
async fn names_where(
    svc: &FerroEhrService,
    ehr: &EhrId,
    cond: &str,
    params: BTreeMap<String, Value>,
) -> BTreeSet<String> {
    let aql = format!(
        "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS}] \
         WHERE {cond}"
    );
    column(svc, ehr, &aql, params)
        .await
        .into_iter()
        .map(|v| v.as_str().expect("a name").to_owned())
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

/// One composition per `DV_ORDERED` subtype, plus a `DV_TEXT` that reads like
/// a date.
fn whole_values() -> Vec<Value> {
    let coded = |code: &str, value: &str| {
        json!({
            "_type": "DV_CODED_TEXT",
            "value": value,
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": code,
            },
        })
    };
    vec![
        holding(
            "quantity",
            json!({ "_type": "DV_QUANTITY", "magnitude": 82.0, "units": "kg" }),
        ),
        holding("count42", json!({ "_type": "DV_COUNT", "magnitude": 42 })),
        holding("count400", json!({ "_type": "DV_COUNT", "magnitude": 400 })),
        holding(
            "ordinal",
            json!({ "_type": "DV_ORDINAL", "value": 1, "symbol": coded("at0010", "one") }),
        ),
        holding(
            "proportion",
            json!({ "_type": "DV_PROPORTION", "numerator": 28.0, "denominator": 2.0, "type": 0 }),
        ),
        holding("date", json!({ "_type": "DV_DATE", "value": "2022-03-01" })),
        holding("time", json!({ "_type": "DV_TIME", "value": "04:05:06" })),
        holding(
            "date_time",
            json!({ "_type": "DV_DATE_TIME", "value": "2022-03-03T04:05:06Z" }),
        ),
        holding("text", json!({ "_type": "DV_TEXT", "value": "2022-03-01" })),
    ]
}

/// A whole data value compares with a literal or a parameter of its own kind,
/// by its magnitude, value or instant, and with nothing else (#5751).
#[tokio::test]
async fn a_whole_data_value_compares_with_a_literal_of_its_kind() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    seed(&svc, &ehr, whole_values()).await;
    let none = BTreeMap::new;
    let cases: [(&str, &[&str]); 16] = [
        // a date literal: the whole DV_DATE and DV_DATE_TIME by their instant
        (" > '9999-12-31'", &[]),
        (" = '2022-03-01'", &["date"]),
        (" > '2022-03-02'", &["date_time"]),
        (" >= '2022-02-28'", &["date", "date_time"]),
        // a time literal: the whole DV_TIME
        (" = '04:05:06'", &["time"]),
        (" > '00:00'", &["time"]),
        // a number: the magnitude or value of a whole numeric DV_ORDERED
        (" = 82.0", &["quantity"]),
        (" = 14", &["proportion"]),
        (" = 1", &["ordinal"]),
        (" > 50", &["quantity", "count400"]),
        (" < 100", &["quantity", "count42", "ordinal", "proportion"]),
        (
            " != 82.0",
            &["count42", "count400", "ordinal", "proportion"],
        ),
        // a value of another kind never compares, and no JSON text is read
        (" = 'original value'", &[]),
        (" < 'zzz'", &[]),
        (" = false", &[]),
        (" < true", &[]),
    ];
    for (cond, want) in cases {
        let got = names_where(&svc, &ehr, &format!("{ITEM}{cond}"), none()).await;
        assert_eq!(got, set(want), "{ITEM}{cond}");
    }
    // NOT keeps the three-valued answer: a value of another kind stays unknown.
    let got = names_where(&svc, &ehr, &format!("NOT {ITEM} = 82.0"), none()).await;
    assert_eq!(
        got,
        set(&["count42", "count400", "ordinal", "proportion"]),
        "NOT {ITEM} = 82.0"
    );
    for (param, want) in [
        (json!("2022-03-03T04:05:06Z"), "date_time"),
        (json!("04:05:06"), "time"),
        (json!(82.0), "quantity"),
        (json!(400), "count400"),
    ] {
        let params = BTreeMap::from([("p".to_owned(), param.clone())]);
        let got = names_where(&svc, &ehr, &format!("{ITEM} = $p"), params).await;
        assert_eq!(got, set(&[want]), "{ITEM} = {param}");
    }
}

/// ORDER BY a whole data value sorts each kind by its own order: instants,
/// then numbers, never JSON text (#5751).
#[tokio::test]
async fn order_by_a_whole_data_value_sorts_by_its_ordering_value() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    seed(&svc, &ehr, whole_values()).await;
    let aql = |direction: &str, filter: &str| {
        format!(
            "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c CONTAINS \
             OBSERVATION o[{OBS}] WHERE c/name/value matches {{{filter}}} \
             ORDER BY {ITEM} {direction}"
        )
    };
    let numbers = "'count42', 'count400', 'quantity', 'ordinal', 'proportion'";
    let got = column(&svc, &ehr, &aql("ASC", numbers), BTreeMap::new()).await;
    assert_eq!(
        got,
        [
            json!("ordinal"),
            json!("proportion"),
            json!("count42"),
            json!("quantity"),
            json!("count400")
        ],
        "ascending by magnitude, 42 before 400"
    );
    let got = column(&svc, &ehr, &aql("DESC", numbers), BTreeMap::new()).await;
    assert_eq!(
        got,
        [
            json!("count400"),
            json!("quantity"),
            json!("count42"),
            json!("proportion"),
            json!("ordinal")
        ],
        "descending by magnitude"
    );
    let instants = "'time', 'date', 'date_time'";
    let got = column(&svc, &ehr, &aql("DESC", instants), BTreeMap::new()).await;
    assert_eq!(
        got,
        [json!("date_time"), json!("date"), json!("time")],
        "descending by instant"
    );
}

/// The magnitude path a 17-significant-digit literal is compared with.
const MAGNITUDE: &str = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/magnitude";

/// A Real of 17 significant digits names the stored double exactly: `=`
/// keeps it and `<`, `>` do not, as a literal or a parameter (#5753).
#[tokio::test]
async fn a_seventeen_digit_real_compares_as_the_number_written() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    seed(
        &svc,
        &ehr,
        vec![observation_composition("precise", 148.012_101_650_238_04)],
    )
    .await;
    let none = BTreeMap::new;
    for (cond, want) in [
        (" = 148.01210165023804", &["precise"][..]),
        (" < 148.01210165023804", &[]),
        (" > 148.01210165023804", &[]),
        (" > 148.012101650238", &["precise"]),
        (" != 148.012101650238", &["precise"]),
    ] {
        let got = names_where(&svc, &ehr, &format!("{MAGNITUDE}{cond}"), none()).await;
        assert_eq!(got, set(want), "{MAGNITUDE}{cond}");
    }
    let params = BTreeMap::from([("m".to_owned(), json!(148.012_101_650_238_04))]);
    let got = names_where(&svc, &ehr, &format!("{MAGNITUDE} = $m"), params).await;
    assert_eq!(got, set(&["precise"]), "{MAGNITUDE} = $m");
}

/// A Boolean compared with a number, or a number with a Boolean, is the
/// caller's `400`, never a database error (#5750).
#[tokio::test]
async fn a_boolean_against_a_number_is_a_bad_request() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    seed(&svc, &ehr, vec![observation_composition("q", 82.0)]).await;
    let refused = [
        (
            format!(
                "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c CONTAINS \
                 OBSERVATION o[{OBS}] WHERE {MAGNITUDE} = false"
            ),
            BTreeMap::new(),
        ),
        (
            format!(
                "SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c CONTAINS \
                 OBSERVATION o[{OBS}] WHERE {MAGNITUDE} < $b"
            ),
            BTreeMap::from([("b".to_owned(), json!(true))]),
        ),
        (
            "SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_status/is_queryable = 1".to_owned(),
            BTreeMap::new(),
        ),
        (
            "SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_status/is_queryable = $n".to_owned(),
            BTreeMap::from([("n".to_owned(), json!(1.5))]),
        ),
    ];
    for (aql, params) in refused {
        let err = svc
            .execute_aql(&aql, None, &request(&ehr, params))
            .await
            .expect_err("a Boolean against a number is refused");
        assert!(
            matches!(&err, ServiceError::BadRequest(r) if r.message.contains("cannot be compared")),
            "{aql:?}: the caller's defect classifies 400, got {err:?}"
        );
    }
    // The same paths against a value of their own kind still answer.
    let got = column(
        &svc,
        &ehr,
        "SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_status/is_queryable = true",
        BTreeMap::new(),
    )
    .await;
    assert_eq!(got.len(), 1, "is_queryable = true keeps the EHR");
}

/// A `uid/value` predicate in a class expression selects the composition whose
/// version it names, as the same comparison in `WHERE` does (#5749).
#[tokio::test]
async fn a_uid_predicate_in_a_class_expression_selects_its_version() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    let uids = seed(
        &svc,
        &ehr,
        vec![
            observation_composition("one", 1.0),
            observation_composition("two", 2.0),
        ],
    )
    .await;
    let (one, two) = (json!(uids[0]), json!(uids[1]));
    let uid = || BTreeMap::from([("uid".to_owned(), one.clone())]);
    let cases = [
        (
            "a parameter in the class expression",
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c[uid/value=$uid]".to_owned(),
            uid(),
            vec![one.clone()],
        ),
        (
            "the same comparison in WHERE",
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE c/uid/value = $uid"
                .to_owned(),
            uid(),
            vec![one.clone()],
        ),
        (
            "a literal in the class expression",
            format!(
                "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c[uid/value='{}']",
                uids[0]
            ),
            BTreeMap::new(),
            vec![one.clone()],
        ),
        (
            "a `!=` literal in the class expression",
            format!(
                "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c[uid/value!='{}']",
                uids[0]
            ),
            BTreeMap::new(),
            vec![two.clone()],
        ),
        (
            "the class expression under CONTAINS",
            format!(
                "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c[uid/value=$uid] \
                 CONTAINS OBSERVATION o[{OBS}]"
            ),
            uid(),
            vec![one.clone()],
        ),
    ];
    // The uids stay out of the failure message: it names the case only.
    for (case, aql, params, want) in cases {
        let got = column(&svc, &ehr, &aql, params).await;
        assert!(got == want, "{case}: the version it names, and no other");
    }
    // A population query with a LIMIT takes the streaming shape.
    let got = svc
        .execute_aql(
            "SELECT c/uid/value FROM COMPOSITION c[uid/value=$uid] LIMIT 5",
            None,
            &AqlQueryRequest {
                parameters: uid(),
                ..AqlQueryRequest::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("population query: {e:?}"))
        .result_set
        .to_value()
        .unwrap();
    assert!(got["rows"] == json!([[one]]), "the streaming root");
}

/// A `uid` predicate on a composition no version is bound beside is refused,
/// never answered as matching nothing (#5749).
#[tokio::test]
async fn a_uid_predicate_under_not_contains_is_refused() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    let err = svc
        .execute_aql(
            "SELECT e/ehr_id/value FROM EHR e NOT CONTAINS COMPOSITION c[uid/value='x']",
            None,
            &request(&ehr, BTreeMap::new()),
        )
        .await
        .expect_err("refused");
    assert!(
        matches!(&err, ServiceError::BadRequest(_)),
        "a typed 400, got {err:?}"
    );
}

/// A composition whose one event is an `INTERVAL_EVENT` of `width`.
fn interval(name: &str, width: &str) -> Value {
    let mut c = observation_composition(name, 1.0);
    let event = &mut c["content"][0]["data"]["events"][0];
    event["_type"] = json!("INTERVAL_EVENT");
    event["width"] = json!({ "_type": "DV_DURATION", "value": width });
    event["math_function"] = json!({
        "_type": "DV_CODED_TEXT",
        "value": "mean",
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": "146",
        },
    });
    c
}

/// A `DV_DURATION`'s `value` is a String and orders as text; the whole
/// `DV_DURATION` orders by the duration it names (#5752).
#[tokio::test]
async fn a_duration_value_orders_as_text() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    seed(
        &svc,
        &ehr,
        vec![interval("days", "P40D"), interval("months", "P30M")],
    )
    .await;
    let by = |key: &str, direction: &str| {
        format!(
            "SELECT i/width/value FROM EHR e CONTAINS COMPOSITION c CONTAINS INTERVAL_EVENT i \
             ORDER BY {key} {direction}"
        )
    };
    for (key, direction, want) in [
        ("i/width/value", "DESC", ["P40D", "P30M"]),
        ("i/width/value", "ASC", ["P30M", "P40D"]),
        ("i/width", "ASC", ["P40D", "P30M"]),
        ("i/width", "DESC", ["P30M", "P40D"]),
    ] {
        let got = column(&svc, &ehr, &by(key, direction), BTreeMap::new()).await;
        assert_eq!(got, want.map(|w| json!(w)), "ORDER BY {key} {direction}");
    }
}
