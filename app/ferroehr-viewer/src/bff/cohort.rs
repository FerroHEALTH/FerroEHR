// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Cohort queries: AQL run over the EHRs a set of demographic predicates
//! selects, without disclosing a party identifier (`{ext}/query/cohort`).
//!
//! The predicates are keys the deployment bound in
//! `[privacy.cohort.predicates]`; the route answers `404` while none is
//! bound. No openEHR spec governs this route: the CDR's own extension.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use serde::Deserialize;

use crate::bff::ehrs::ResultPage;
use crate::error::ViewerError;
use crate::locale::server::ServerMsg;
use crate::locale::tr_served;

/// The predicate keys a deployment can bind, in the order the form offers
/// them.
pub const KEYS: [&str; 5] = ["city", "postcode_area", "sex", "age_band", "organisation"];

/// One cohort predicate: a bound key and the value it matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    /// The key.
    pub name: String,
    /// The value.
    pub value: String,
}

/// The cohort predicates `predicates` as the JSON array the route takes,
/// each entry `{name, value}`.
#[must_use]
pub fn cohort_json(predicates: &[Predicate]) -> String {
    serde_json::Value::Array(
        predicates
            .iter()
            .map(|predicate| serde_json::json!({"name": predicate.name, "value": predicate.value}))
            .collect(),
    )
    .to_string()
}

/// What the CDR says of the cohort a run selected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CohortMeta {
    /// How many EHRs the cohort resolved to.
    #[serde(default)]
    pub size: u64,
    /// How many EHRs the rows came from, `0` when suppressed.
    #[serde(default)]
    pub served_ehrs: u64,
    /// Whether small-cell suppression withheld the rows.
    #[serde(default)]
    pub suppressed: bool,
    /// The SHA-256 digest of the predicate list.
    #[serde(default)]
    pub definition: String,
}

/// The `meta` block of a cohort result set.
#[derive(Debug, Deserialize)]
struct Meta {
    /// The cohort facts.
    #[serde(default)]
    cohort: CohortMeta,
}

/// A cohort result set's envelope, read for its meta block.
#[derive(Debug, Deserialize)]
struct Envelope {
    /// The meta block.
    meta: Meta,
}

/// The request body of a cohort run of `aql` with `parameters` over the
/// predicates `cohort` (a JSON array text), the first `fetch` rows or the
/// CDR's default window.
#[must_use]
pub fn request_body(
    aql: &str,
    parameters: &serde_json::Value,
    cohort: &str,
    fetch: Option<u32>,
) -> String {
    // NOTE: no openEHR spec governs this: our own design; a cohort text that is no
    // JSON is sent as no predicate, and the CDR refuses the request in its words.
    let cohort: serde_json::Value =
        serde_json::from_str(cohort).unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
    let mut body = serde_json::json!({
        "q": aql,
        "query_parameters": parameters,
        "cohort": cohort,
    });
    if let (Some(fetch), Some(fields)) = (fetch, body.as_object_mut()) {
        fields.insert("fetch".to_owned(), fetch.into());
    }
    body.to_string()
}

/// Runs `aql` with `parameters_json` over the cohort `predicates` select:
/// the rows and the cohort's facts; `None` when the deployment binds no
/// predicate (`404`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the CDR's words for
/// an unbound predicate, an unreadable value or AQL that does not parse
/// (`400`) and a cohort above the configured maximum (`422`);
/// [`ViewerError::Rejected`] for parameters that are not a JSON object;
/// [`ViewerError::Failed`] for an answer that does not read.
pub async fn run_cohort(
    aql: &str,
    parameters_json: &str,
    predicates: &[Predicate],
) -> Result<Option<(ResultPage, CohortMeta)>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let parameters: serde_json::Value = if parameters_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(parameters_json).map_err(|error| {
            ViewerError::rejected(tr_served(ServerMsg::ParametersMustJsonObject3), error)
        })?
    };
    let url = cdr.extensions_v1("query/cohort");
    let body = request_body(
        aql,
        &parameters,
        &cohort_json(predicates),
        Some(crate::bff::PAGE_SIZE),
    );
    let response = cdr.post(&url).body("application/json", body).send().await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let body = response.ok()?.body;
    let meta = serde_json::from_str::<Envelope>(&body)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::CohortResultJson), error))?
        .meta
        .cohort;
    Ok(Some((crate::bff::ehrs::parse_result_set(&body, 0)?, meta)))
}

/// The predicate keys the deployment binds, read from its redacted
/// configuration; `None` when the session may not read it.
///
/// # Errors
/// [`ViewerError::Failed`] for a configuration that does not read.
pub async fn bound_keys() -> Result<Option<Vec<String>>, ViewerError> {
    let Ok(body) = crate::bff::system::fetch_admin_config().await else {
        // NOTE: no openEHR spec governs this: our own design; a configuration the
        // session may not read leaves the bound keys unknown, and the CDR judges each.
        return Ok(None);
    };
    let config: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::CohortResultJson), error))?;
    Ok(config
        .pointer("/privacy/cohort/predicates")
        .and_then(serde_json::Value::as_object)
        .map(|bound| bound.keys().cloned().collect()))
}

#[cfg(test)]
mod tests {
    use super::{Envelope, Predicate, cohort_json};

    /// The predicates go as the documented `{name, value}` entries.
    #[test]
    fn the_cohort_names_each_predicate() {
        let json = cohort_json(&[Predicate {
            name: "city".to_owned(),
            value: "Groningen".to_owned(),
        }]);
        assert_eq!(json, r#"[{"name":"city","value":"Groningen"}]"#);
    }

    /// The documented meta block reads.
    #[test]
    fn the_cohort_meta_reads() {
        let envelope: Envelope = serde_json::from_str(
            r#"{"meta":{"_type":"RESULTSET","cohort":{"size":412,"served_ehrs":388,"suppressed":false,"definition":"9f2c"}},"rows":[]}"#,
        )
        .expect("the envelope reads");
        assert_eq!(envelope.meta.cohort.served_ehrs, 388);
    }
}
