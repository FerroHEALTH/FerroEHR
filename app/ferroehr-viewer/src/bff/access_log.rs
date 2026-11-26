// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDR's openEHR access log: an EHR's entries and one entry.
//!
//! FerroEHR serves the log under its extensions root (`GET
//! {ext}/ehr/{ehr_id}/access_log`, `/{uid}`), its access-log proposal to
//! openEHR, which no ITS-REST release defines. The CDR's authorization
//! chooses the view: the subject view for the person and their
//! representative, the full record for a reviewer; the viewer asks for
//! neither and draws what it is answered. An entry is read as the JSON wire
//! the CDR serves and kept whole beside the facts a list draws, so a member
//! the subject view leaves out is simply absent. Every read writes an
//! access-log entry of its own. No openEHR spec governs the viewer: our own
//! design.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use serde_json::Value;

use crate::error::ViewerError;
use crate::locale::access::AccessMsg;
use crate::locale::tr;

/// The filters an access-log read takes, each as the CDR names it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilters {
    /// The first day selected, `YYYY-MM-DD`.
    pub from: String,
    /// The last day selected, `YYYY-MM-DD`.
    pub to: String,
    /// The action, the token `terminology_id|code`.
    pub action: String,
    /// The outcome, the token `terminology_id|code`.
    pub outcome: String,
    /// The accessor kind, the token `terminology_id|code`.
    pub accessor_kind: String,
    /// `justification` or `emergency`.
    pub justification: String,
    /// Whether the subject's own reads of their log are included.
    pub include_self: bool,
}

impl LogFilters {
    /// The query pairs of the filters a read sends, each day widened to the
    /// instants that bound it.
    #[must_use]
    pub fn query_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = Vec::new();
        if !self.from.is_empty() {
            pairs.push(("from", day_bound(&self.from, false)));
        }
        if !self.to.is_empty() {
            pairs.push(("to", day_bound(&self.to, true)));
        }
        for (name, value) in [
            ("action", &self.action),
            ("outcome", &self.outcome),
            ("accessor_kind", &self.accessor_kind),
            ("justification", &self.justification),
        ] {
            if !value.is_empty() {
                pairs.push((name, value.clone()));
            }
        }
        if self.include_self {
            pairs.push(("include_self", "true".to_owned()));
        }
        pairs
    }
}

/// The first instant of `day` in UTC, or its last when `end`; a text that is
/// no date goes as typed, for the CDR to refuse in its own words.
pub(crate) fn day_bound(day: &str, end: bool) -> String {
    let Ok(date) = day.parse::<jiff::civil::Date>() else {
        return day.to_owned();
    };
    let time = if end {
        jiff::civil::Time::MAX
    } else {
        jiff::civil::Time::MIN
    };
    date.to_datetime(time)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .map_or_else(|_| day.to_owned(), |zoned| zoned.timestamp().to_string())
}

/// A coded value as an entry carries it: its words and its token.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coded {
    /// The rubric.
    pub value: String,
    /// The terminology id.
    pub terminology: String,
    /// The code.
    pub code: String,
}

impl Coded {
    /// The `DV_CODED_TEXT` at `node`, `None` when it holds none.
    pub(crate) fn of(node: Option<&Value>) -> Option<Self> {
        let node = node?;
        let text = |pointer: &str| {
            node.pointer(pointer)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let coded = Self {
            value: text("/value"),
            terminology: text("/defining_code/terminology_id/value"),
            code: text("/defining_code/code_string"),
        };
        (!coded.value.is_empty() || !coded.code.is_empty()).then_some(coded)
    }

    /// The words to draw: the rubric, else the code.
    #[must_use]
    pub fn words(&self) -> &str {
        if self.value.is_empty() {
            &self.code
        } else {
            &self.value
        }
    }
}

/// One authorisation an access rested on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Authorisation {
    /// Its kind: a role, a care relationship, a justification, an emergency.
    pub kind: Option<Coded>,
    /// The name of the grant or rule.
    pub name: String,
    /// The version of the policy it names.
    pub version: String,
    /// The reason the accessor stated, for a justification or an emergency.
    pub reason: String,
}

impl Authorisation {
    /// Whether this is an emergency access.
    #[must_use]
    pub fn is_emergency(&self) -> bool {
        self.kind
            .as_ref()
            .is_some_and(|kind| kind.code == "emergency")
    }

    /// Whether this is an access on a justification.
    #[must_use]
    pub fn is_justification(&self) -> bool {
        self.kind
            .as_ref()
            .is_some_and(|kind| kind.code == "justification")
    }
}

/// One object an access reached.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    /// The RM type read.
    pub rm_type: String,
    /// The id of the object, its version id when it names one.
    pub id: String,
    /// The categories the audit category map placed it in.
    pub categories: Vec<String>,
}

/// One entry of the access log, as the CDR served it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    /// The entry's uid.
    pub uid: String,
    /// When it was recorded.
    pub recorded: String,
    /// The action.
    pub action: Option<Coded>,
    /// The operation of the API.
    pub operation: Option<Coded>,
    /// The outcome.
    pub outcome: Option<Coded>,
    /// The reason of a refused or failed access.
    pub outcome_reason: Option<Coded>,
    /// The kind of accessor.
    pub accessor_kind: Option<Coded>,
    /// The accessing party: its name, else its reference.
    pub accessor: String,
    /// The type and id of the demographic party the accessor is bound to.
    pub accessor_party: Option<(String, String)>,
    /// The provider the accessor acted for.
    pub provider: String,
    /// The accessor's roles.
    pub roles: Vec<String>,
    /// The objects reached.
    pub items: Vec<Item>,
    /// The purposes of use.
    pub purposes: Vec<Coded>,
    /// What the access rested on.
    pub authorisations: Vec<Authorisation>,
    /// The entry as the CDR served it, pretty-printed.
    pub document: String,
}

impl Entry {
    /// The authorisation of a justified or an emergency access, when the
    /// access rested on one.
    #[must_use]
    pub fn justified(&self) -> Option<&Authorisation> {
        self.authorisations
            .iter()
            .find(|authorisation| authorisation.is_emergency() || authorisation.is_justification())
    }

    /// Whether the access was refused or failed.
    #[must_use]
    pub fn denied(&self) -> bool {
        self.outcome
            .as_ref()
            .is_some_and(|outcome| outcome.code != "granted")
    }
}

/// The text at `pointer` under `node`, empty when it holds none.
pub(crate) fn text_at(node: &Value, pointer: &str) -> String {
    node.pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The words of a party: its name, else the id its reference names, else
/// its first identifier.
pub(crate) fn party_words(party: Option<&Value>) -> String {
    let Some(party) = party else {
        return String::new();
    };
    if party.get("_type").and_then(Value::as_str) == Some("PARTY_SELF") {
        return tr(AccessMsg::PartySelf);
    }
    [
        text_at(party, "/name"),
        text_at(party, "/external_ref/id/value"),
        text_at(party, "/identifiers/0/id"),
    ]
    .into_iter()
    .find(|words| !words.is_empty())
    .unwrap_or_default()
}

/// The elements of the array at `key` of `node`.
pub(crate) fn each<'a>(node: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    node.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// The entry `node` as the facts a page draws, `None` when it carries no uid.
#[must_use]
pub fn entry_of(node: &Value) -> Option<Entry> {
    let uid = text_at(node, "/uid/value");
    if uid.is_empty() {
        return None;
    }
    let accessor = node.get("accessor");
    Some(Entry {
        uid,
        recorded: text_at(node, "/time_recorded/value"),
        action: Coded::of(node.get("action")),
        operation: Coded::of(node.get("operation")),
        outcome: Coded::of(node.pointer("/outcome/result")),
        outcome_reason: Coded::of(node.pointer("/outcome/reason")),
        accessor_kind: Coded::of(accessor.and_then(|accessor| accessor.get("kind"))),
        accessor: party_words(accessor.and_then(|accessor| accessor.get("party"))),
        accessor_party: accessor
            .and_then(|accessor| accessor.pointer("/party/external_ref"))
            .filter(|reference| text_at(reference, "/namespace") == "demographic")
            .map(|reference| (text_at(reference, "/type"), text_at(reference, "/id/value")))
            .filter(|(_, id)| !id.is_empty()),
        provider: party_words(accessor.and_then(|accessor| accessor.get("provider"))),
        roles: accessor
            .into_iter()
            .flat_map(|accessor| each(accessor, "roles"))
            .filter_map(|role| Coded::of(Some(role)))
            .map(|role| role.words().to_owned())
            .collect(),
        items: each(node, "items")
            .map(|item| Item {
                rm_type: text_at(item, "/rm_type"),
                id: [
                    text_at(item, "/version_id/value"),
                    text_at(item, "/object/id/value"),
                ]
                .into_iter()
                .find(|id| !id.is_empty())
                .unwrap_or_default(),
                categories: each(item, "categories")
                    .filter_map(|category| Coded::of(Some(category)))
                    .map(|category| category.words().to_owned())
                    .collect(),
            })
            .collect(),
        purposes: each(node, "purposes")
            .filter_map(|purpose| Coded::of(Some(purpose)))
            .collect(),
        authorisations: each(node, "authorisations")
            .map(|authorisation| Authorisation {
                kind: Coded::of(authorisation.get("kind")),
                name: text_at(authorisation, "/name"),
                version: text_at(authorisation, "/version"),
                reason: text_at(authorisation, "/reason"),
            })
            .collect(),
        document: serde_json::to_string_pretty(node).unwrap_or_default(),
    })
}

/// The path of the access log of the EHR `ehr_id`, or of its entry `uid`.
fn log_path(ehr_id: &str, uid: Option<&str>) -> String {
    let base = format!("ehr/{}/access_log", urlencoding::encode(ehr_id));
    match uid {
        Some(uid) => format!("{base}/{}", urlencoding::encode(uid)),
        None => base,
    }
}

/// The entries of the EHR `ehr_id`'s access log that `filters` select,
/// newest first, one page at `offset`.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)): the `403` of a caller
/// who is neither the person nor a reviewer, the `404` of an EHR the caller
/// may not see or a CDR that keeps no access log, the `409` of a CDR whose
/// audit store is off; [`ViewerError::Failed`] when the body is not JSON.
pub async fn ehr_access_log(
    // The EHR whose log is read.
    ehr_id: &str,
    // The filters the read sends.
    filters: &LogFilters,
    // First entry of the page.
    offset: u32,
    // How many entries the page holds.
    fetch: u32,
) -> Result<Vec<Entry>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let mut pairs = filters.query_pairs();
    pairs.push(("offset", offset.to_string()));
    pairs.push(("fetch", fetch.to_string()));
    let query: Vec<String> = pairs
        .iter()
        .map(|(name, value)| format!("{name}={}", urlencoding::encode(value)))
        .collect();
    let url = format!(
        "{}?{}",
        cdr.extensions_v1(&log_path(ehr_id, None)),
        query.join("&")
    );
    let body = cdr.get(&url).ok().await?.body;
    let doc: Value = serde_json::from_str(&body)
        .map_err(|error| ViewerError::failed(tr(AccessMsg::LogJson), error))?;
    Ok(each(&doc, "entries").filter_map(entry_of).collect())
}

/// The entry `uid` of the EHR `ehr_id`'s access log.
///
/// # Errors
/// As [`ehr_access_log`], the `404` of an unknown entry among them.
pub async fn ehr_access_log_entry(
    // The EHR whose log is read.
    ehr_id: &str,
    // The entry's uid.
    uid: &str,
) -> Result<Entry, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&log_path(ehr_id, Some(uid)));
    let body = cdr.get(&url).ok().await?.body;
    let doc: Value = serde_json::from_str(&body)
        .map_err(|error| ViewerError::failed(tr(AccessMsg::LogJson), error))?;
    entry_of(&doc).ok_or_else(|| ViewerError::Internal(tr(AccessMsg::LogJson)))
}

#[cfg(test)]
mod tests {
    use super::{LogFilters, entry_of};

    #[test]
    fn the_filters_widen_each_day_to_its_instants() {
        let filters = LogFilters {
            from: "2026-11-01".to_owned(),
            to: "2026-11-02".to_owned(),
            outcome: "openehr_access|refused".to_owned(),
            include_self: true,
            ..LogFilters::default()
        };
        assert_eq!(
            filters.query_pairs(),
            vec![
                ("from", "2026-11-01T00:00:00Z".to_owned()),
                ("to", "2026-11-02T23:59:59.999999999Z".to_owned()),
                ("outcome", "openehr_access|refused".to_owned()),
                ("include_self", "true".to_owned()),
            ]
        );
        assert_eq!(LogFilters::default().query_pairs(), Vec::new());
    }

    #[test]
    fn an_entry_names_its_accessor_items_and_justification() {
        let node = serde_json::json!({
            "_type": "ACCESS_LOG_ENTRY",
            "uid": {"value": "01a21788-af74-70a3-8d1b-0283473df148"},
            "time_recorded": {"value": "2026-11-26T11:06:03.504176Z"},
            "action": {"value": "read", "defining_code": {
                "terminology_id": {"value": "HL7_AuditEventAction"}, "code_string": "R"}},
            "outcome": {"result": {"value": "granted", "defining_code": {
                "terminology_id": {"value": "openehr_access"}, "code_string": "granted"}}},
            "accessor": {
                "kind": {"value": "person", "defining_code": {
                    "terminology_id": {"value": "openehr_access"}, "code_string": "person"}},
                "party": {"_type": "PARTY_IDENTIFIED", "name": "Dr Example"},
                "roles": [{"value": "USER", "defining_code": {
                    "terminology_id": {"value": "local"}, "code_string": "USER"}}]
            },
            "items": [{"rm_type": "COMPOSITION",
                "version_id": {"value": "8a1e::local::1"},
                "categories": [{"value": "Laboratory reports", "defining_code": {
                    "terminology_id": {"value": "x"}, "code_string": "Laboratory-Reports"}}]}],
            "authorisations": [{"kind": {"value": "emergency", "defining_code": {
                "terminology_id": {"value": "openehr_access"}, "code_string": "emergency"}},
                "name": "break-glass", "version": "1", "reason": "Unresponsive on arrival"}]
        });
        let entry = entry_of(&node).expect("the entry has a uid");
        assert_eq!(entry.accessor, "Dr Example");
        assert_eq!(entry.roles, vec!["USER".to_owned()]);
        assert_eq!(entry.items[0].id, "8a1e::local::1");
        assert_eq!(
            entry.items[0].categories,
            vec!["Laboratory reports".to_owned()]
        );
        let justified = entry.justified().expect("an emergency access");
        assert!(justified.is_emergency());
        assert_eq!(justified.reason, "Unresponsive on arrival");
        assert!(!entry.denied());
    }

    #[test]
    fn a_subject_view_without_a_party_still_reads() {
        let node = serde_json::json!({
            "uid": {"value": "u"},
            "accessor": {"kind": {"value": "person"}},
            "outcome": {"result": {"value": "refused", "defining_code": {"code_string": "refused"}}}
        });
        let entry = entry_of(&node).expect("the entry has a uid");
        assert_eq!(entry.accessor, "");
        assert!(entry.denied());
        assert!(entry_of(&serde_json::json!({})).is_none());
    }
}
