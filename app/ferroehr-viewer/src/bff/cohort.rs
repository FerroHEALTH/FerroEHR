// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Cohort queries: AQL run over the EHRs a set of demographic predicates or
//! named subjects selects, without disclosing a party identifier
//! (`{ext}/query/cohort`).
//!
//! The predicates are keys the deployment bound in
//! `[privacy.cohort.predicates]`, and subjects are named only where
//! `[privacy.cohort] subjects` is on; the route answers `404` while the
//! deployment allows neither. No openEHR spec governs this route: the CDR's
//! own extension.

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

/// One subject a cohort names by identifier: the `id` and `namespace` of
/// its `OBJECT_REF` (RM common master04 §`OBJECT_REF`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    /// The identifier domain.
    pub namespace: String,
    /// The identifier.
    pub id: String,
}

/// What selects a cohort: the predicates and the named subjects, which
/// intersect.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The filled predicates.
    pub predicates: Vec<Predicate>,
    /// The named subjects.
    pub subjects: Vec<Subject>,
}

impl Selection {
    /// Whether nothing selects a cohort.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.predicates.is_empty() && self.subjects.is_empty()
    }
}

/// What the deployment lets a cohort select on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    /// The predicate keys, in the order the form offers them.
    pub keys: Vec<String>,
    /// Whether a cohort may name its subjects.
    pub subjects: bool,
}

/// The form field of the subjects' identifier domain.
pub const SUBJECT_NAMESPACE_FIELD: &str = "cohort-subject-namespace";

/// The form field of the subjects' identifiers, one per line.
pub const SUBJECT_IDS_FIELD: &str = "cohort-subject-ids";

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

/// The request body of a cohort run of `aql` with `parameters` over what
/// `selection` selects, the first `fetch` rows or the CDR's default window.
#[must_use]
pub fn request_body(
    aql: &str,
    parameters: &serde_json::Value,
    selection: &Selection,
    fetch: Option<u32>,
) -> String {
    let cohort: Vec<serde_json::Value> = selection
        .predicates
        .iter()
        .map(|predicate| serde_json::json!({"name": predicate.name, "value": predicate.value}))
        .collect();
    let mut body = serde_json::json!({
        "q": aql,
        "query_parameters": parameters,
        "cohort": cohort,
    });
    if let Some(fields) = body.as_object_mut() {
        if !selection.subjects.is_empty() {
            let subjects: Vec<serde_json::Value> = selection
                .subjects
                .iter()
                .map(
                    |subject| serde_json::json!({"id": subject.id, "namespace": subject.namespace}),
                )
                .collect();
            fields.insert("subjects".to_owned(), subjects.into());
        }
        if let Some(fetch) = fetch {
            fields.insert("fetch".to_owned(), fetch.into());
        }
    }
    body.to_string()
}

/// What the cohort form `field` reads: each bindable key whose field
/// `cohort-<key>` is filled, and each identifier line under the subjects'
/// namespace.
pub fn chosen(field: impl Fn(&str) -> String) -> Selection {
    let predicates = KEYS
        .iter()
        .map(|key| Predicate {
            name: (*key).to_owned(),
            value: field(&format!("cohort-{key}")).trim().to_owned(),
        })
        .filter(|predicate| !predicate.value.is_empty())
        .collect();
    let namespace = field(SUBJECT_NAMESPACE_FIELD).trim().to_owned();
    let subjects = field(SUBJECT_IDS_FIELD)
        .lines()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| Subject {
            namespace: namespace.clone(),
            id: id.to_owned(),
        })
        .collect();
    Selection {
        predicates,
        subjects,
    }
}

/// Runs `aql` with `parameters_json` over the cohort `selection` selects:
/// the rows and the cohort's facts; `None` when the deployment allows no
/// cohort (`404`).
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
    selection: &Selection,
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
    let body = request_body(aql, &parameters, selection, Some(crate::bff::PAGE_SIZE));
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

/// What the deployment lets a cohort select on, read from its redacted
/// configuration; every key and the subjects when the session may not read
/// it, for the CDR to judge.
///
/// # Errors
/// [`ViewerError::Failed`] for a configuration that does not read.
pub async fn offered() -> Result<Offered, ViewerError> {
    let Ok(body) = crate::bff::system::fetch_admin_config().await else {
        // NOTE: no openEHR spec governs this: our own design; a configuration the
        // session may not read leaves the allowed selection unknown, and the CDR judges it.
        return Ok(Offered {
            keys: KEYS.iter().map(|key| (*key).to_owned()).collect(),
            subjects: true,
        });
    };
    let config: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::CohortResultJson), error))?;
    let bound = config
        .pointer("/privacy/cohort/predicates")
        .and_then(serde_json::Value::as_object);
    Ok(Offered {
        keys: KEYS
            .iter()
            .filter(|key| bound.is_some_and(|bound| bound.contains_key(**key)))
            .map(|key| (*key).to_owned())
            .collect(),
        subjects: config
            .pointer("/privacy/cohort/subjects")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Envelope, SUBJECT_IDS_FIELD, SUBJECT_NAMESPACE_FIELD, chosen, request_body};

    /// The filled fields are the predicates, sent as the documented
    /// `{name, value}` entries.
    #[test]
    fn the_cohort_names_each_filled_predicate() {
        let predicates = chosen(|field| {
            if field == "cohort-city" {
                " Groningen ".to_owned()
            } else {
                String::new()
            }
        });
        assert_eq!(
            request_body("q", &serde_json::json!({}), &predicates, None),
            r#"{"q":"q","query_parameters":{},"cohort":[{"name":"city","value":"Groningen"}]}"#
        );
    }

    /// Each identifier line is a subject under the namespace, sent as the
    /// documented `{id, namespace}` entries beside the predicates.
    #[test]
    fn the_cohort_names_each_subject_line() {
        let selection = chosen(|field| match field {
            SUBJECT_NAMESPACE_FIELD => " urn:oid:2.999.1.1 ".to_owned(),
            SUBJECT_IDS_FIELD => "p-01\n\n p-02 \n".to_owned(),
            _ => String::new(),
        });
        assert_eq!(
            request_body("q", &serde_json::json!({}), &selection, None),
            r#"{"q":"q","query_parameters":{},"cohort":[],"subjects":[{"id":"p-01","namespace":"urn:oid:2.999.1.1"},{"id":"p-02","namespace":"urn:oid:2.999.1.1"}]}"#
        );
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
