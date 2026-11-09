// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/audit` screen — the ATNA audit-log browser.
//!
//! Browses the CDR's local Audit Record Repository through the RESTful-ATNA
//! **ITI-81** retrieval (`GET fhir/r4/AuditEvent`): a filter form (event-time
//! window, patient, principal, outcome, action), a paged table of the stored
//! FHIR R4 `AuditEvent` documents (IHE BALP shape), and a per-row raw-record
//! view. No openEHR spec governs the viewer — our own design / product
//! extension; the wire it reads is IHE's (the `RESTful` ATNA supplement's
//! ITI-81 FHIR search), served by the CDR.
//!
//! The CDR answering `404` (the local audit store disabled) is a first-class
//! rendered state, not an error.

use fhir_types::codec::Json;
use fhir_types::r4::audit_event::AuditEvent;
use fhir_types::r4::bundle::Bundle;
use fhir_types::r4::reference::Reference;
use fhir_types::r4::resource::Resource;

use crate::bff::PAGE_SIZE;
use crate::error::ViewerError;
use crate::locale::console::ConsoleMsg;
use crate::locale::tr;

/// One distilled audit record: the promoted facts the table shows plus the
/// full stored `AuditEvent` JSON for the raw view. Client-safe fields only
/// (fixed-size ints, no `usize`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    /// The event instant (`AuditEvent.recorded`), as stored.
    pub recorded: String,
    /// The event family, humanized (REST / Authentication / Export / Import).
    pub family: String,
    /// The concrete operation (the ITS-REST operation id, or the login kind).
    pub operation: String,
    /// The action code (`C`/`R`/`U`/`D`/`E`).
    pub action: String,
    /// The outcome, humanized (success / minor failure / …).
    pub outcome: String,
    /// Whether the outcome is a success (drives the outcome tint).
    pub success: bool,
    /// The requesting principal (the requestor agent's identifier).
    pub principal: String,
    /// The recorded patient (EHR subject) id, when patient-centric.
    pub patient: String,
    /// The touched resource id, when one was recorded.
    pub resource: String,
    /// The full stored `AuditEvent` document, pretty-printed.
    pub raw: String,
}

/// One retrieved page plus the availability verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPage {
    /// `false` when the CDR answered `404` — the local Audit Record
    /// Repository is disabled (`[audit.store]`); a first-class state.
    pub available: bool,
    /// Total records matching the filter (the Bundle `total`).
    pub total: u32,
    /// The current page offset.
    pub offset: u32,
    /// The page rows, newest first.
    pub rows: Vec<AuditRow>,
}

/// Retrieve one page of audit records via the ITI-81 FHIR search (`GET
/// fhir/r4/AuditEvent`), with the supported filter subset.
///
/// Empty filter strings are omitted from the query; instants are sent as
/// `date=ge…`/`date=le…` bounds; values are percent-encoded.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::CdrUnauthorized`] when the CDR no longer accepts this
/// session, [`ViewerError::Forbidden`] when the caller lacks the CDR's admin
/// role (the audit trail is an operator surface); [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`] on
/// an unparseable Bundle.
pub async fn search_audit(
    // Inclusive lower bound on the record instant; empty means unbounded.
    from: String,
    // Inclusive upper bound on the record instant; empty means unbounded.
    to: String,
    // Patient reference to match; empty means any patient.
    patient: String,
    // Agent (acting user) to match; empty means any agent.
    agent: String,
    // Outcome code to match; empty means any outcome.
    outcome: String,
    // Action code to match; empty means any action.
    action: String,
    // First record of the page to return.
    offset: u32,
) -> Result<AuditPage, ViewerError> {
    let cdr = crate::bff::cdr()?;

    let mut params: Vec<String> = Vec::new();
    let mut push = |key: &str, value: &str| {
        if !value.trim().is_empty() {
            params.push(format!("{key}={}", urlencoding::encode(value.trim())));
        }
    };
    // NOTE: RESTful ATNA §3.81.4.1.2.1, a retrieval names at least one date, so an unset
    // window is the last 30 days (no specification governs the length: our own design).
    let from = if from.trim().is_empty() && to.trim().is_empty() {
        window_start()
    } else {
        from
    };
    push("date", &prefixed("ge", &from));
    push("date", &prefixed("le", &to));
    push("patient", &patient);
    push("agent", &agent);
    push("outcome", &outcome);
    push("action", &action);
    params.push(format!("_count={PAGE_SIZE}"));
    params.push(format!("_offset={offset}"));

    let url = format!("{}?{}", cdr.fhir_r4("AuditEvent"), params.join("&"));
    let response = cdr.get(&url).accept("application/fhir+json").send().await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        // The local Audit Record Repository is disabled ([audit.store]) —
        // a rendered state, never an error.
        return Ok(AuditPage {
            available: false,
            total: 0,
            offset,
            rows: Vec::new(),
        });
    }
    let body = response.ok()?.body;
    parse_bundle(&body, offset)
}

/// `ge`/`le`-prefix a non-empty instant for the ITI-81 `date` parameter.
fn prefixed(prefix: &str, value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        String::new()
    } else {
        format!("{prefix}{value}")
    }
}

/// The first day of the window a search with no date asks for: 30 days
/// back, as a FHIR date.
fn window_start() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .checked_sub(jiff::Span::new().days(30))
        .map(|day| day.to_string())
        .unwrap_or_default()
}

/// An ITI-81 `searchset` entry that is no `AuditEvent` and no search outcome.
#[derive(Debug, thiserror::Error)]
#[error("searchset entry {index} is not an AuditEvent")]
struct NotAnAuditEvent {
    /// The entry's position in the Bundle.
    index: usize,
}

/// Distil the ITI-81 `searchset` Bundle into an [`AuditPage`], read as a FHIR
/// R4 `Bundle` of `AuditEvent` resources (`RESTful` ATNA ITI-81).
///
/// An entry with `search.mode` `outcome` is the server's note on the search
/// (FHIR R4 `bundle.html` §Bundle.entry.search.mode) and makes no row; any
/// other entry that is no `AuditEvent` refuses the page.
fn parse_bundle(body: &str, offset: u32) -> Result<AuditPage, ViewerError> {
    let bundle = serde_json::from_str::<Bundle>(body)
        .map_err(|e| ViewerError::failed(tr(ConsoleMsg::AuditeventBundleJson), e))?;
    let total = bundle
        .total
        .and_then(|total| total.value)
        .unwrap_or_default();
    let mut rows = Vec::with_capacity(bundle.entry.len());
    for (index, entry) in bundle.entry.iter().enumerate() {
        let outcome = entry
            .search
            .as_ref()
            .and_then(|search| search.mode.as_ref())
            .and_then(|mode| mode.value.as_deref())
            == Some("outcome");
        match &entry.resource {
            Some(Resource::AuditEvent(event)) => rows.push(audit_row(event)?),
            _ if outcome => {}
            _ => {
                return Err(ViewerError::failed(
                    tr(ConsoleMsg::AuditeventBundleJson),
                    NotAnAuditEvent { index },
                ));
            }
        }
    }
    Ok(AuditPage {
        available: true,
        total,
        offset,
        rows,
    })
}

/// The text of a primitive, empty when it carries none.
fn text_of(value: Option<&Option<String>>) -> String {
    value
        .and_then(Option::as_deref)
        .unwrap_or_default()
        .to_owned()
}

/// The `identifier.value` of a reference, empty when it carries none.
fn identifier_of(reference: Option<&Reference>) -> String {
    text_of(
        reference
            .and_then(|r| r.identifier.as_deref())
            .and_then(|identifier| identifier.value.as_ref())
            .map(|value| &value.value),
    )
}

/// Distil one stored `AuditEvent` (IHE BALP shape) into an [`AuditRow`].
///
/// An optional element the event leaves out renders empty; the event itself
/// has passed the FHIR R4 read.
fn audit_row(event: &AuditEvent) -> Result<AuditRow, ViewerError> {
    let type_code = text_of(event.r#type.code.as_ref().map(|code| &code.value));
    let family = match type_code.as_str() {
        "rest" => "REST".to_owned(),
        "110114" => tr(ConsoleMsg::Authentication),
        "110106" => tr(ConsoleMsg::Export),
        "110107" => tr(ConsoleMsg::Import),
        other => other.to_owned(),
    };
    // The concrete operation: the last subtype coding (the ITS-REST
    // operation id, or the DCM login kind on authentication records).
    let operation = event
        .subtype
        .last()
        .map(|coding| {
            let code = text_of(coding.code.as_ref().map(|code| &code.value));
            match code.as_str() {
                "110122" => "login".to_owned(),
                "110123" => "logout".to_owned(),
                _ => code,
            }
        })
        .unwrap_or_default();
    let outcome_code = text_of(event.outcome.as_ref().map(|code| &code.value));
    let (outcome, success) = match outcome_code.as_str() {
        "0" => ("success".to_owned(), true),
        "4" => (tr(ConsoleMsg::MinorFailure), false),
        "8" => (tr(ConsoleMsg::SeriousFailure), false),
        "12" => (tr(ConsoleMsg::MajorFailure), false),
        other => (other.to_owned(), false),
    };
    // The requesting principal: the requestor agent's who-identifier
    // (skipping the token agent, which carries no who).
    let principal = event
        .agent
        .iter()
        .find(|agent| {
            agent.requestor.value == Some(true) && !identifier_of(agent.who.as_ref()).is_empty()
        })
        .map(|agent| identifier_of(agent.who.as_ref()))
        .unwrap_or_default();
    // Entities: the patient (audit-entity-type 1) and the data object (2).
    let entity_value = |code: &str| {
        event
            .entity
            .iter()
            .find(|entity| {
                entity
                    .r#type
                    .as_ref()
                    .and_then(|coding| coding.code.as_ref())
                    .and_then(|c| c.value.as_deref())
                    == Some(code)
            })
            .map(|entity| identifier_of(entity.what.as_ref()))
            .unwrap_or_default()
    };
    let refused = |e| ViewerError::failed(tr(ConsoleMsg::AuditeventBundleJson), e);
    let raw = serde_json::to_string_pretty(&event.to_json().map_err(refused)?)
        .map_err(|e| ViewerError::failed(tr(ConsoleMsg::AuditeventBundleJson), e))?;
    Ok(AuditRow {
        recorded: text_of(Some(&event.recorded.value)),
        family,
        operation,
        action: text_of(event.action.as_ref().map(|code| &code.value)),
        outcome,
        success,
        principal,
        patient: entity_value("1"),
        resource: entity_value("2"),
        raw,
    })
}

/// The outcome choices of the filter, as the value the query carries and
/// the words shown.
pub(crate) const OUTCOMES: [(&str, ConsoleMsg); 5] = [
    ("", ConsoleMsg::ChoiceAny),
    ("0", ConsoleMsg::ChoiceSuccess),
    ("4", ConsoleMsg::MinorFailure),
    ("8", ConsoleMsg::SeriousFailure),
    ("12", ConsoleMsg::MajorFailure),
];

/// The action choices of the filter, as the value the query carries and the
/// words shown.
pub(crate) const ACTIONS: [(&str, ConsoleMsg); 6] = [
    ("", ConsoleMsg::ChoiceAny),
    ("C", ConsoleMsg::ActionCreate),
    ("R", ConsoleMsg::ActionRead),
    ("U", ConsoleMsg::ActionUpdate),
    ("D", ConsoleMsg::ActionDelete),
    ("E", ConsoleMsg::ActionExecute),
];

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "test fixtures are raw JSON by the testing rule (.claude/rules/testing.md)"
)]
mod tests {
    use fhir_types::codec::{Json, Object, Path};
    use fhir_types::r4::audit_event::AuditEvent;

    use super::{audit_row, parse_bundle};

    /// The source every stored record names (`AuditEvent.source`, 1..1).
    fn source() -> serde_json::Value {
        serde_json::json!({ "observer": { "display": "ferroehr" } })
    }

    /// The one agent of a record that names no one (`AuditEvent.agent`, 1..*).
    fn agent() -> serde_json::Value {
        serde_json::json!([{ "requestor": true }])
    }

    fn bundle(entries: &serde_json::Value) -> String {
        serde_json::json!({
            "resourceType": "Bundle",
            "type": "searchset",
            "total": 2,
            "entry": entries,
        })
        .to_string()
    }

    /// `resource` read as a FHIR R4 `AuditEvent`.
    fn event(resource: &serde_json::Value) -> AuditEvent {
        let object: Object = serde_json::from_value(resource.clone()).expect("an object");
        AuditEvent::from_json(&object, &mut Path::root("AuditEvent")).expect("an AuditEvent")
    }

    #[test]
    fn parses_a_searchset_bundle() {
        let body = bundle(&serde_json::json!([
            { "resource": { "resourceType": "AuditEvent", "recorded": "2026-07-10T08:00:00Z",
                "type": { "code": "rest" }, "action": "R", "outcome": "0",
                "agent": agent(), "source": source() } },
            { "resource": { "resourceType": "AuditEvent", "recorded": "2026-07-09T08:00:00Z",
                "type": { "code": "110114" }, "action": "E", "outcome": "4",
                "agent": agent(), "source": source() } },
        ]));
        let page = parse_bundle(&body, 0).expect("page");
        assert!(page.available);
        assert_eq!(page.total, 2);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].family, "REST");
        assert!(page.rows[0].success);
        assert_eq!(page.rows[1].family, "Authentication");
        assert_eq!(page.rows[1].outcome, "minor failure");
    }

    /// A searchset that is no FHIR R4 `Bundle` of `AuditEvent`s is refused, never
    /// read as a page with an empty row.
    #[test]
    fn a_body_that_is_no_audit_searchset_is_refused() {
        // An AuditEvent without its required `source`.
        let body = bundle(&serde_json::json!([
            { "resource": { "resourceType": "AuditEvent", "recorded": "2026-07-10T08:00:00Z",
                "type": { "code": "rest" }, "agent": agent() } },
        ]));
        assert!(parse_bundle(&body, 0).is_err());
        // Another resource in the searchset.
        let body = bundle(&serde_json::json!([
            { "resource": { "resourceType": "Basic", "code": { "text": "x" } } },
        ]));
        assert!(parse_bundle(&body, 0).is_err());
        assert!(parse_bundle("not json", 0).is_err());
        // The server's note on the search makes no row.
        let body = bundle(&serde_json::json!([
            { "resource": { "resourceType": "OperationOutcome",
                "issue": [{ "severity": "information", "code": "informational" }] },
              "search": { "mode": "outcome" } },
        ]));
        assert_eq!(parse_bundle(&body, 0).expect("page").rows, Vec::new());
    }

    #[test]
    fn distils_the_balp_shape() {
        let resource = serde_json::json!({
            "resourceType": "AuditEvent",
            "recorded": "2026-07-10T08:00:00Z",
            "type": { "code": "rest" },
            "subtype": [
                { "code": "read" },
                { "code": "composition_get" }
            ],
            "action": "R",
            "outcome": "0",
            "agent": [
                { "requestor": true, "who": { "identifier": { "value": "alice" } } },
                { "requestor": false, "who": { "identifier": { "value": "ferroehr" } } },
                { "requestor": true, "policy": ["jti-1"] }
            ],
            "source": source(),
            "entity": [
                { "type": { "code": "1" }, "what": { "identifier": { "value": "patient-42" } } },
                { "type": { "code": "2" }, "what": { "identifier": { "value": "8fa1::ferroehr::1" } } }
            ]
        });
        let row = audit_row(&event(&resource)).expect("a row");
        assert_eq!(row.operation, "composition_get");
        assert_eq!(row.principal, "alice");
        assert_eq!(row.patient, "patient-42");
        assert_eq!(row.resource, "8fa1::ferroehr::1");
        assert!(row.raw.contains("composition_get"));
    }

    #[test]
    fn login_records_humanize() {
        let resource = serde_json::json!({
            "resourceType": "AuditEvent",
            "recorded": "2026-07-10T08:00:00Z",
            "type": { "code": "110114" },
            "subtype": [{ "code": "110122" }],
            "action": "E",
            "outcome": "4",
            "agent": agent(),
            "source": source(),
        });
        let row = audit_row(&event(&resource)).expect("a row");
        assert_eq!(row.family, "Authentication");
        assert_eq!(row.operation, "login");
        assert!(!row.success);
    }
}
