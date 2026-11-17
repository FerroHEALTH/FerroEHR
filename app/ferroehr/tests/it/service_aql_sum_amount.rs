// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `SUM` and `AVG` over a whole data value add numbers and amounts only
//! (#5917), executed against a real PostgreSQL 18 (shared testkit harness).
//!
//! QUERY master03 §SUM, §AVG: "Input values type should be either Integer or
//! Real". RM `data_types` §`DV_AMOUNT` defines `+` on the relative amounts
//! "unlike descendants of `DV_ABSOLUTE_QUANTITY`, such as the date/time
//! types", so a `DV_QUANTITY`, `DV_COUNT`, `DV_PROPORTION` or `DV_DURATION`
//! adds by its magnitude and a date, time, date-time or ordinal is refused, as
//! a typed temporal path already is.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;

use crate::fixtures::{create_ehr, observation_composition, uv};

const OBS: &str = "openEHR-EHR-OBSERVATION.minimal.v1";
const ITEM: &str = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value";

fn holding(name: &str, value: Value) -> Value {
    let mut c = observation_composition(name, 0.0);
    c["content"][0]["data"]["events"][0]["data"]["items"][0]["value"] = value;
    c
}

fn request(ehr: &EhrId) -> AqlQueryRequest {
    AqlQueryRequest {
        ehr_ids: vec![*ehr],
        parameters: BTreeMap::new(),
        ..AqlQueryRequest::default()
    }
}

/// `SUM` and `AVG` of [`ITEM`] over the compositions named in `names`.
fn sum_avg(names: &str) -> String {
    format!(
        "SELECT SUM({ITEM}), AVG({ITEM}) FROM EHR e CONTAINS COMPOSITION c CONTAINS \
         OBSERVATION o[{OBS}] WHERE c/name/value matches {{{names}}}"
    )
}

async fn seeded() -> (testkit::TestDb, FerroEhrService, EhrId) {
    let (db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    let coded = json!({
        "_type": "DV_CODED_TEXT",
        "value": "one",
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
            "code_string": "at0001",
        },
    });
    for c in [
        holding(
            "q",
            json!({ "_type": "DV_QUANTITY", "magnitude": 80.0, "units": "mm[Hg]" }),
        ),
        holding("n", json!({ "_type": "DV_COUNT", "magnitude": 20 })),
        holding("date", json!({ "_type": "DV_DATE", "value": "2022-03-01" })),
        holding(
            "date_time",
            json!({ "_type": "DV_DATE_TIME", "value": "2022-03-03T04:05:06Z" }),
        ),
        holding("time", json!({ "_type": "DV_TIME", "value": "04:05:06" })),
        holding(
            "ordinal",
            json!({ "_type": "DV_ORDINAL", "value": 1, "symbol": coded }),
        ),
    ] {
        svc.create_composition(ehr, uv(&c, "249", None))
            .await
            .unwrap_or_else(|e| panic!("create_composition: {e:?}"));
    }
    (db, svc, ehr)
}

/// Amounts add by their magnitude.
#[tokio::test]
async fn sum_and_avg_add_amounts() {
    let (_db, svc, ehr) = Box::pin(seeded()).await;
    let rows = svc
        .execute_aql(&sum_avg("'q', 'n'"), None, &request(&ehr))
        .await
        .unwrap_or_else(|e| panic!("the query failed: {e:?}"))
        .result_set["rows"]
        .clone();
    let row = &rows[0];
    assert_eq!(row[0].as_f64(), Some(100.0), "SUM of 80 and 20: {rows}");
    assert_eq!(row[1].as_f64(), Some(50.0), "AVG of 80 and 20: {rows}");
}

/// A date, a time, a date-time or an ordinal among the values is the caller's
/// `400`, never a number of seconds or days.
#[tokio::test]
async fn sum_and_avg_refuse_a_value_that_is_no_amount() {
    let (_db, svc, ehr) = Box::pin(seeded()).await;
    for name in ["date", "date_time", "time", "ordinal"] {
        let err = svc
            .execute_aql(&sum_avg(&format!("'q', '{name}'")), None, &request(&ehr))
            .await
            .expect_err("refused");
        assert!(
            matches!(&err, ServiceError::BadRequest(r) if r.message.contains("SUM and AVG add numbers and amounts")),
            "{name}: a typed 400, got {err:?}"
        );
    }
}
