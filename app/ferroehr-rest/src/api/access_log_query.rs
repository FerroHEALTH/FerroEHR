// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The query parameters, the page and the record of a read of the access
//! log's ITS-REST resources (#5130), which the EHR's log and the
//! administrator's review share.
//!
//! The resources follow the ITS-REST 1.1.0 conventions the draft's ITS-REST
//! page names: `offset` and `fetch` as the Query API's
//! (`docs/specs/openehr/ITS-REST/specifications/parameters/query/offset.yaml`,
//! `fetch.yaml`) and ISO 8601 date-times. A parameter a resource does not
//! offer, or a value outside its form, is a `400`, so no parameter widens a
//! view. A list answers no total count, since a count over a large log costs a
//! scan. No openEHR spec governs the access log: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "a page of the access log is a JSON document of ITS-JSON records, the subject \
              view among them, which no generated type carries"
)]

use axum::response::{IntoResponse, Response};
use http::{HeaderMap, StatusCode};
use openehr_its::rest::runtime::ApiError;
use serde_json::{Value, json};

use ferroehr::access_log::search::{LOG_QUERY, LogQuery};
use ferroehr::access_log::view::{LogReader, LogStanding, view_of};
use ferroehr::ids::EhrId;
use ferroehr::system_log::event::{EntryScope, QueryNaming};
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

use crate::extensions::access::authn::{Principal, current_principal};
use crate::extensions::access::authz::AuthzHandle;
use crate::extensions::access::authz::classify::OperationClass;
use crate::extensions::access::authz::roles::RbacDecision;
use crate::extensions::access::delay;
use crate::overview::error::RestError;
use crate::overview::params::query_pairs;
use crate::state::AppState;
use crate::system_log::middleware::AuditObject;

/// The parameters of an EHR's log (the draft's ITS-REST page).
pub(crate) const EHR_PARAMETERS: &[&str] = &[
    "from",
    "to",
    "action",
    "outcome",
    "accessor_kind",
    "emergency",
    "include_self",
    "offset",
    "fetch",
];

/// The parameters of the review of the whole log (the draft's ITS-REST page).
pub(crate) const REVIEW_PARAMETERS: &[&str] = &[
    "ehr_id",
    "party_namespace",
    "party_type",
    "party_id",
    "issuer",
    "subject",
    "request_id",
    "from",
    "to",
    "outcome",
    "emergency",
    "offset",
    "fetch",
];

/// What the current caller may read of the access log, whichever resource
/// names it: the subject of care and a representative bound through the party
/// binding read the subject view of the EHR they are confined to (the EHR of
/// the patient token's launch context, or of the subject the representation
/// names, #5446, #5532), any other caller the admin class admits reads the
/// full view, and any other caller is refused.
pub(crate) fn standing(state: &AppState) -> LogStanding {
    // NOTE: our own design, a server with authentication off is open, as the admin review is.
    let Some(principal) = current_principal() else {
        return LogStanding::Reviewer;
    };
    if delay::acts_for_the_person(&principal) {
        return confined(&principal)
            .and_then(|ehr| ehr.parse::<uuid::Uuid>().ok())
            .map_or(LogStanding::Refused, |ehr| LogStanding::Person(EhrId(ehr)));
    }
    let authz = state.authz();
    let admitted = authz
        .as_deref()
        .and_then(AuthzHandle::rbac)
        .is_none_or(|rbac| {
            rbac.decide(OperationClass::Admin, &principal.roles) == RbacDecision::Allow
        });
    if admitted {
        LogStanding::Reviewer
    } else {
        LogStanding::Refused
    }
}

/// The EHR the person's caller is confined to: the represented subject's for
/// a representative, else the launch context's of the patient's token.
fn confined(principal: &Principal) -> Option<&str> {
    match &principal.represented {
        Some(represented) => represented.ehr_id.as_deref(),
        None => principal.access.ehr_id(),
    }
}

/// The largest page served: our own design.
const MAX_FETCH: i64 = 1000;

/// The filter the query string `query` names, each parameter one of
/// `offered`.
///
/// # Errors
/// A `400` naming the first parameter `offered` does not hold, repeated, or
/// outside its form.
pub(crate) fn log_query(query: Option<&str>, offered: &[&str]) -> Result<LogQuery, RestError> {
    let mut filter = LogQuery {
        without_own_log_reads: offered.contains(&"include_self"),
        ..LogQuery::default()
    };
    let mut seen: Vec<String> = Vec::new();
    for (name, value) in query.map(query_pairs).unwrap_or_default() {
        if !offered.contains(&name.as_str()) {
            return Err(malformed(format!(
                "the access log takes no `{name}` parameter here"
            )));
        }
        if seen.contains(&name) {
            return Err(malformed(format!("`{name}` is given more than once")));
        }
        set(&mut filter, &name, value)?;
        seen.push(name);
    }
    Ok(filter)
}

/// Set the member `name` of `filter` to `value`.
fn set(filter: &mut LogQuery, name: &str, value: String) -> Result<(), RestError> {
    match name {
        "from" => filter.from = Some(instant(name, &value)?),
        "to" => filter.to = Some(instant(name, &value)?),
        "emergency" => filter.emergency = Some(boolean(name, &value)?),
        "include_self" => filter.without_own_log_reads = !boolean(name, &value)?,
        "offset" => filter.offset = count(name, &value, 0, i64::MAX)?,
        "fetch" => filter.fetch = count(name, &value, 1, MAX_FETCH)?,
        "ehr_id" => {
            crate::overview::version_id::parse_ehr_id(&value)?;
            filter.ehr_id = Some(value);
        }
        "action" => filter.action = Some(value),
        "outcome" => filter.outcome = Some(value),
        "accessor_kind" => filter.accessor_kind = Some(value),
        "party_namespace" => filter.party.namespace = Some(value),
        "party_type" => filter.party.class = Some(value),
        "party_id" => filter.party.id = Some(value),
        "issuer" => filter.issuer = Some(value),
        "subject" => filter.subject = Some(value),
        "request_id" => filter.request_id = Some(value),
        other => {
            return Err(malformed(format!(
                "the access log takes no `{other}` parameter"
            )));
        }
    }
    Ok(())
}

/// `value`, an ISO 8601 date-time with its offset, as an instant.
fn instant(name: &str, value: &str) -> Result<jiff::Timestamp, RestError> {
    value.parse().map_err(|_unparsed: jiff::Error| {
        malformed(format!(
            "`{name}` is an ISO 8601 date-time with an offset, such as 2026-11-13T09:00:00Z"
        ))
    })
}

/// `value`, `true` or `false`.
fn boolean(name: &str, value: &str) -> Result<bool, RestError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(malformed(format!("`{name}` is true or false"))),
    }
}

/// `value`, an integer from `least` to `most`.
fn count(name: &str, value: &str, least: i64, most: i64) -> Result<i64, RestError> {
    value
        .parse::<i64>()
        .ok()
        .filter(|number| (least..=most).contains(number))
        .ok_or_else(|| malformed(format!("`{name}` is an integer from {least} to {most}")))
}

/// The `400` naming a malformed filter.
fn malformed(message: String) -> RestError {
    RestError::from(ApiError::BadRequest(message))
}

/// The page of `records` for `reader`, answered under `filter`'s paging.
///
/// # Errors
/// A `500` when a record has no JSON form.
pub(crate) fn page(
    records: &[AccessLogRecord],
    filter: &LogQuery,
    reader: LogReader,
) -> Result<Value, RestError> {
    let entries = records
        .iter()
        .filter_map(|record| record_view(record, reader).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "offset": filter.offset, "fetch": filter.fetch, "entries": entries }))
}

/// `record` as `reader` reads it ([`view_of`]), its rendering failure a `500`.
///
/// # Errors
/// A `500` when the record has no JSON form.
pub(crate) fn record_view(
    record: &AccessLogRecord,
    reader: LogReader,
) -> Result<Option<Value>, RestError> {
    view_of(record, reader).map_err(|error| {
        RestError::from(crate::overview::error::internal_fault_caused(
            "render the access-log record",
            &error,
        ))
    })
}

/// The answer to a read of the log, `answered` with the records it served,
/// carrying the record of the read whatever it answers.
///
/// That record writes its own access-log entry (the draft's recording rule 4;
/// DICOM PS3.15 A.5.3.2 "Audit Log Used"), naming the EHR whose log was read,
/// the entry read, the records served and the filter by its digest.
pub(crate) fn recorded(
    headers: &HeaderMap,
    answered: Result<(Value, Option<usize>), RestError>,
    ehr_id: Option<&str>,
    uid: Option<&str>,
    query: Option<&str>,
) -> Response {
    let (mut resp, served) = match answered {
        Ok((body, served)) => (
            crate::negotiate::body::respond(headers, StatusCode::OK, &body),
            served,
        ),
        Err(error) => (error.into_response(), None),
    };
    resp.extensions_mut().insert(AuditObject {
        ehr_id: ehr_id.map(str::to_owned),
        uid: uid.map(str::to_owned),
        result_count: served.and_then(|served| u64::try_from(served).ok()),
        query: query.map(|text| QueryNaming::of(LOG_QUERY, text, None)),
        entry: EntryScope::LogRead,
        ..AuditObject::default()
    });
    resp
}
