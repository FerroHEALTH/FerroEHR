// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The list of the parties the CDR holds, read with AQL rooted at a
//! demographic class.
//!
//! The Demographic API publishes no collection read, so the list is the
//! CDR's AQL over its demographic store (`FROM PARTY`, `FROM PERSON`, …,
//! each under `VERSION` for its commit time), which applies the policy-party
//! rule inside its one statement: the viewer shows what the CDR answers and
//! filters nothing itself. The query text is built from fixed fragments
//! chosen by the root and by whether a find is typed; the typed text travels
//! as the `$text` parameter, never in the query. No openEHR spec governs
//! demographic AQL roots or the viewer: FerroEHR's own design.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use serde_json::Value;

use crate::bff::demographics::PartyKind;
use crate::bff::ehrs::{aql_request_body, cell_text, parse_result_set};
use crate::error::ViewerError;
use crate::uid::container_uid_of;

/// The class a party list is rooted at: every party, or one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyRoot {
    /// `PARTY`: every party of the five kinds.
    Every,
    /// One kind's class.
    Kind(PartyKind),
}

impl PartyRoot {
    /// Every root, in the order the list's tabs show them.
    pub const ALL: [Self; 6] = [
        Self::Every,
        Self::Kind(PartyKind::Person),
        Self::Kind(PartyKind::Organisation),
        Self::Kind(PartyKind::Group),
        Self::Kind(PartyKind::Agent),
        Self::Kind(PartyKind::Role),
    ];

    /// The root's key in the address: `all` or the kind's segment.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Every => "all",
            Self::Kind(kind) => kind.segment(),
        }
    }

    /// The root `key` names, every party for anything else.
    #[must_use]
    pub fn from_key(key: &str) -> Self {
        PartyKind::from_segment(key).map_or(Self::Every, Self::Kind)
    }

    /// The RM class the query is rooted at.
    fn class(self) -> &'static str {
        match self {
            Self::Every => "PARTY",
            Self::Kind(kind) => kind.rm_type(),
        }
    }
}

/// The AQL that lists the parties of `root`, newest commit first, narrowed
/// to the parties whose identities hold `$text` when `find` is set.
#[must_use]
pub fn list_aql(root: PartyRoot, find: bool) -> String {
    let class = root.class();
    if find {
        format!(
            "SELECT p, v/commit_audit/time_committed/value FROM VERSION v CONTAINS {class} p \
             WHERE p/identities/details/items/value/value LIKE $text \
             ORDER BY v/commit_audit/time_committed/value DESC"
        )
    } else {
        format!(
            "SELECT p, v/commit_audit/time_committed/value FROM VERSION v CONTAINS {class} p \
             ORDER BY v/commit_audit/time_committed/value DESC"
        )
    }
}

/// One party of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedParty {
    /// The party's versioned-object uid.
    pub uid: String,
    /// Its kind, `None` for a type outside the five.
    pub kind: Option<PartyKind>,
    /// The name it goes by, `None` when its identities hold no text.
    pub name: Option<String>,
    /// When its latest version was committed.
    pub committed: String,
}

/// The `DV_TEXT` values of the `ELEMENT`s under `node`, in order.
fn collect_text(node: &Value, words: &mut Vec<String>) {
    match node {
        Value::Object(object) => {
            if object.get("_type").and_then(Value::as_str) == Some("ELEMENT") {
                if let Some(text) = node.pointer("/value/value").and_then(Value::as_str) {
                    words.push(text.to_owned());
                }
                return;
            }
            for (key, child) in object {
                if key != "name" {
                    collect_text(child, words);
                }
            }
        }
        Value::Array(list) => list.iter().for_each(|child| collect_text(child, words)),
        _ => {}
    }
}

/// The name a party goes by: the text values of its first identity's
/// elements, joined, or `None` when it carries none.
#[must_use]
pub fn identity_name(identities: &Value) -> Option<String> {
    let mut words = Vec::new();
    if let Some(first) = identities.as_array().and_then(|list| list.first()) {
        collect_text(first, &mut words);
    }
    (!words.is_empty()).then(|| words.join(" "))
}

/// The party of one result row, `None` when the row carries no party uid.
fn listed(row: &[Value]) -> Option<ListedParty> {
    let party = row.first()?;
    let uid = party.pointer("/uid/value").and_then(Value::as_str)?;
    Some(ListedParty {
        uid: container_uid_of(uid),
        kind: party
            .get("_type")
            .and_then(Value::as_str)
            .and_then(PartyKind::from_rm_type),
        name: party.get("identities").and_then(identity_name),
        committed: row.get(1).map(cell_text).unwrap_or_default(),
    })
}

/// The parties of `root`, newest commit first, one page at `offset`, the
/// ones whose identities hold `find` when it is set.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the `400` of a CDR
/// that serves no demographic root among them; [`ViewerError::Failed`] when
/// the result set is not valid JSON.
pub async fn list_parties(
    // The class the list is rooted at.
    root: PartyRoot,
    // The text the party's identities hold, `None` for every party.
    find: Option<&str>,
    // First row of the page to return.
    offset: u32,
    // How many rows the page holds.
    fetch: u32,
) -> Result<Vec<ListedParty>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.rest_v1("query/aql");
    let parameters = match find {
        Some(text) => serde_json::json!({ "text": format!("*{text}*") }),
        None => serde_json::json!({}),
    };
    let body = aql_request_body(&list_aql(root, find.is_some()), &parameters, offset, fetch);
    let response = cdr.post(&url).body("application/json", body).ok().await?;
    let page = parse_result_set(&response.body, offset)?;
    Ok(page.rows.iter().filter_map(|row| listed(row)).collect())
}

#[cfg(test)]
mod tests {
    use super::{PartyRoot, identity_name, list_aql, listed};
    use crate::bff::demographics::PartyKind;

    #[test]
    fn every_list_query_parses() {
        for root in PartyRoot::ALL {
            for find in [false, true] {
                let aql = list_aql(root, find);
                openehr_query::parser::parse_str(&aql)
                    .unwrap_or_else(|error| panic!("{aql} should parse: {error:?}"));
            }
        }
    }

    #[test]
    fn the_find_is_a_parameter_never_the_text() {
        let aql = list_aql(PartyRoot::Kind(PartyKind::Person), true);
        assert!(aql.contains("FROM VERSION v CONTAINS PERSON p WHERE"));
        assert!(aql.contains("LIKE $text"));
        assert!(list_aql(PartyRoot::Every, false).contains("CONTAINS PARTY p ORDER BY"));
    }

    #[test]
    fn a_root_reads_back_from_its_key() {
        for root in PartyRoot::ALL {
            assert_eq!(root, PartyRoot::from_key(root.key()));
        }
        assert_eq!(PartyRoot::Every, PartyRoot::from_key("nonsense"));
    }

    #[test]
    fn a_row_names_its_party_by_its_first_identity() {
        let row = vec![
            serde_json::json!({
                "_type": "PERSON",
                "uid": {"value": "8a1e3c9b-1c0f-4c8e-9a39-2a8b1b9c3d10::local::2"},
                "identities": [{
                    "_type": "PARTY_IDENTITY",
                    "name": {"value": "legal name"},
                    "details": {"items": [
                        {"_type": "ELEMENT", "name": {"value": "Given"}, "value": {"value": "Demo"}},
                        {"_type": "ELEMENT", "name": {"value": "Family"}, "value": {"value": "Person"}}
                    ]}
                }]
            }),
            serde_json::json!("2026-11-26T11:04:43.420414+00:00"),
        ];
        let party = listed(&row).expect("the row names a party");
        assert_eq!(party.uid, "8a1e3c9b-1c0f-4c8e-9a39-2a8b1b9c3d10");
        assert_eq!(party.kind, Some(PartyKind::Person));
        assert_eq!(party.name.as_deref(), Some("Demo Person"));
        assert_eq!(party.committed, "2026-11-26T11:04:43.420414+00:00");
    }

    #[test]
    fn a_party_without_identity_text_has_no_name() {
        assert_eq!(identity_name(&serde_json::json!([])), None);
        assert_eq!(identity_name(&serde_json::json!(null)), None);
    }
}
