// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The grants in force, read as policy parties through AQL.
//!
//! The queries are rooted at `ROLE`, `AGENT` and `PARTY_RELATIONSHIP`, so the
//! CDR's policy-party rule decides what a caller sees: an administrator reads
//! them, anyone else an empty answer.
//!
//! A `ROLE` holds a grant when its `CAPABILITY.credentials` hold an element
//! whose value is a `DV_CODED_TEXT` of the draft `openehr_access`
//! terminology; an element whose name is such a code qualifies the grant. An
//! actor binds an OAuth 2.0 client by an element named
//! `openehr_access::client_id`, and acts for a provider through an
//! `openehr_access::acts_for` relationship. No openEHR specification governs
//! grants: FerroEHR's own design over its draft access control package.

use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;
use crate::locale::tr;

/// The draft access control terminology.
const ACCESS: &str = "openehr_access";

/// The roles holding a grant, one row each.
const ROLES_AQL: &str = "SELECT DISTINCT r FROM ROLE r CONTAINS CAPABILITY c CONTAINS ELEMENT e \
                         WHERE e/value/defining_code/terminology_id/value = 'openehr_access'";

/// The actors bound to an OAuth 2.0 client.
const AGENTS_AQL: &str = "SELECT DISTINCT a FROM AGENT a CONTAINS ELEMENT e \
                          WHERE e/name/defining_code/terminology_id/value = 'openehr_access' \
                          AND e/name/defining_code/code_string = 'client_id'";

/// The acts-for relationships: source, target and the target's type.
const ACTS_FOR_AQL: &str = "SELECT r/source/id/value, r/target/id/value, r/target/type \
                            FROM PARTY_RELATIONSHIP r \
                            WHERE r/name/defining_code/terminology_id/value = 'openehr_access' \
                            AND r/name/defining_code/code_string = 'acts_for'";

/// The roles one performer holds grants by.
const PERFORMER_AQL: &str = "SELECT DISTINCT r FROM ROLE r CONTAINS CAPABILITY c \
                             CONTAINS ELEMENT e WHERE r/performer/id/value = $party \
                             AND e/value/defining_code/terminology_id/value = 'openehr_access'";

/// The most rows one listing reads.
pub const LIMIT: usize = 500;

/// A coded value of the access terminology: its rubric and code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    /// The rubric (`DV_CODED_TEXT.value`).
    pub rubric: String,
    /// The code.
    pub code: String,
}

/// A party a grant or a relationship names: its RM type and id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyRef {
    /// The RM type, such as `PERSON` or `AGENT`.
    pub rm_type: String,
    /// The versioned party's id.
    pub id: String,
}

/// A `DV_INTERVAL<DV_DATE>` as its bounds, either open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Validity {
    /// The lower bound, when there is one.
    pub lower: Option<String>,
    /// The upper bound, when there is one.
    pub upper: Option<String>,
}

impl Validity {
    /// Whether the interval holds the date `today` (`YYYY-MM-DD`): an open
    /// bound holds every date.
    #[must_use]
    pub fn holds(&self, today: &str) -> bool {
        let date = |bound: &str| bound.get(..10).unwrap_or(bound).to_owned();
        self.lower
            .as_deref()
            .is_none_or(|lower| date(lower).as_str() <= today)
            && self
                .upper
                .as_deref()
                .is_none_or(|upper| today <= date(upper).as_str())
    }
}

/// A role holding a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleGrant {
    /// The role's versioned id.
    pub role: String,
    /// The role's name.
    pub name: String,
    /// The operations its capabilities grant.
    pub operations: Vec<Code>,
    /// The elements that qualify them: each code and the text it names.
    pub qualifiers: Vec<(Code, String)>,
    /// The party it is held by.
    pub performer: Option<PartyRef>,
    /// The role's `time_validity`.
    pub role_validity: Validity,
    /// Its capabilities' `time_validity`, one per capability.
    pub capability_validity: Vec<Validity>,
}

impl RoleGrant {
    /// Whether the grant is in force at `now`: the role's and every
    /// capability's validity hold its date in UTC, and `now` is neither
    /// before a `not_before` nor after a `not_after`; one that does not read
    /// as an instant holds on no date, as the CDR reads it.
    #[must_use]
    pub fn in_force(&self, now: jiff::Timestamp) -> bool {
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date().to_string();
        let instant = |text: &str| {
            text.parse::<jiff::Timestamp>().ok().or_else(|| {
                text.parse::<jiff::civil::DateTime>()
                    .ok()
                    .and_then(|time| time.to_zoned(jiff::tz::TimeZone::UTC).ok())
                    .map(|zoned| zoned.timestamp())
            })
        };
        let bounds = self
            .qualifiers
            .iter()
            .all(|(code, text)| match code.code.as_str() {
                "not_before" => instant(text).is_some_and(|start| start <= now),
                "not_after" => instant(text).is_some_and(|end| now <= end),
                _ => true,
            });
        bounds
            && self.role_validity.holds(&today)
            && self
                .capability_validity
                .iter()
                .all(|validity| validity.holds(&today))
    }
}

/// An actor bound to an OAuth 2.0 client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemAgent {
    /// The actor's versioned id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The client id.
    pub client_id: String,
    /// The token issuer.
    pub issuer: String,
    /// The providers it acts for.
    pub acts_for: Vec<PartyRef>,
}

/// The grants in force, as Roles and grants lists them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Grants {
    /// The roles holding a grant.
    pub roles: Vec<RoleGrant>,
    /// The system agents.
    pub agents: Vec<SystemAgent>,
    /// Whether a listing reached [`LIMIT`].
    pub truncated: bool,
}

/// Runs `aql` with `parameters` and answers its rows.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's AQL result set and the canonical-JSON demographic \
              parties of the draft access control package"
)]
async fn rows(
    aql: &str,
    parameters: serde_json::Value,
) -> Result<Vec<Vec<serde_json::Value>>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.rest_v1("query/aql");
    let body =
        serde_json::json!({ "q": aql, "query_parameters": parameters, "fetch": LIMIT }).to_string();
    let response = cdr.post(&url).body("application/json", body).ok().await?;
    let value: serde_json::Value = serde_json::from_str(&response.body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::GrantsJson), error))?;
    Ok(value
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| row.as_array().cloned().unwrap_or_default())
                .collect()
        })
        .unwrap_or_default())
}

/// The roles holding a grant and the system agents, with the providers each
/// agent acts for.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the `403` of a caller
/// the CDR refuses included; [`ViewerError::Failed`] when an answer is not a
/// result set.
#[expect(
    clippy::disallowed_types,
    reason = "the AQL request's parameter object is JSON on the ITS-REST wire"
)]
pub async fn fetch_grants() -> Result<Grants, ViewerError> {
    let none = serde_json::Map::new;
    let roles = rows(ROLES_AQL, none().into()).await?;
    let agents = rows(AGENTS_AQL, none().into()).await?;
    let acts_for = rows(ACTS_FOR_AQL, none().into()).await?;
    Ok(read_grants(&roles, &agents, &acts_for))
}

/// The roles holding a grant that `party` performs.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not a result set.
#[expect(
    clippy::disallowed_types,
    reason = "the AQL request's parameter object is JSON on the ITS-REST wire"
)]
pub async fn fetch_grants_of(party: String) -> Result<Vec<RoleGrant>, ViewerError> {
    let rows = rows(PERFORMER_AQL, serde_json::json!({ "party": party })).await?;
    Ok(rows
        .iter()
        .filter_map(|row| row.first())
        .filter_map(role_grant)
        .collect())
}

/// The grants the three result sets read into.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's AQL result set and the canonical-JSON demographic \
              parties of the draft access control package"
)]
#[must_use]
pub fn read_grants(
    roles: &[Vec<serde_json::Value>],
    agents: &[Vec<serde_json::Value>],
    acts_for: &[Vec<serde_json::Value>],
) -> Grants {
    let text = |value: Option<&serde_json::Value>| {
        value
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let links: Vec<(String, PartyRef)> = acts_for
        .iter()
        .map(|row| {
            (
                text(row.first()),
                PartyRef {
                    id: text(row.get(1)),
                    rm_type: text(row.get(2)),
                },
            )
        })
        .collect();
    Grants {
        roles: roles
            .iter()
            .filter_map(|row| row.first())
            .filter_map(role_grant)
            .collect(),
        agents: agents
            .iter()
            .filter_map(|row| row.first())
            .filter_map(|agent| {
                let mut found = client_of(agent)?;
                found.acts_for = links
                    .iter()
                    .filter(|(source, _)| *source == found.id)
                    .map(|(_, target)| target.clone())
                    .collect();
                Some(found)
            })
            .collect(),
        truncated: roles.len() >= LIMIT || agents.len() >= LIMIT,
    }
}

/// The versioned id of a party document: its `uid` without the version.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON demographic parties"
)]
fn versioned_id(party: &serde_json::Value) -> String {
    crate::uid::container_uid_of(&crate::uid::uid_value_of_document(party))
}

/// The access code a value is, when it is a `DV_CODED_TEXT` of the access
/// terminology.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON demographic parties"
)]
fn access_code(value: &serde_json::Value) -> Option<Code> {
    let terminology = value
        .pointer("/defining_code/terminology_id/value")
        .and_then(serde_json::Value::as_str)?;
    (terminology == ACCESS).then(|| Code {
        rubric: value
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        code: value
            .pointer("/defining_code/code_string")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

/// Every `ELEMENT` under `value`, at any depth.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON demographic parties"
)]
fn elements<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a serde_json::Value>) {
    match value {
        serde_json::Value::Object(object) => {
            if object.get("_type").and_then(serde_json::Value::as_str) == Some("ELEMENT") {
                out.push(value);
            }
            for child in object.values() {
                elements(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                elements(item, out);
            }
        }
        _ => {}
    }
}

/// The text an element's value names: a text, a coded text's rubric, an
/// identifier's id or a date and time.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON demographic parties"
)]
fn element_text(element: &serde_json::Value) -> String {
    let value = element.get("value");
    value
        .and_then(|value| value.get("value").or_else(|| value.get("id")))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A `DV_INTERVAL` as its bounds.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON demographic parties"
)]
fn validity(interval: Option<&serde_json::Value>) -> Validity {
    let bound = |key: &str| {
        interval
            .and_then(|interval| interval.pointer(&format!("/{key}/value")))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    Validity {
        lower: bound("lower"),
        upper: bound("upper"),
    }
}

/// The grant a `ROLE` document holds, or `None` when its capabilities grant
/// no operation.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON demographic parties"
)]
#[must_use]
pub fn role_grant(role: &serde_json::Value) -> Option<RoleGrant> {
    if role.get("_type").and_then(serde_json::Value::as_str) != Some("ROLE") {
        return None;
    }
    let capabilities: &[serde_json::Value] = role
        .get("capabilities")
        .and_then(serde_json::Value::as_array)
        .map_or(&[], Vec::as_slice);
    let mut found = Vec::new();
    for capability in capabilities {
        if let Some(credentials) = capability.get("credentials") {
            elements(credentials, &mut found);
        }
    }
    let mut operations = Vec::new();
    let mut qualifiers = Vec::new();
    for element in found {
        let named = element.get("name").and_then(access_code);
        match named {
            Some(code) => qualifiers.push((code, element_text(element))),
            None => {
                if let Some(code) = element.get("value").and_then(access_code) {
                    operations.push(code);
                }
            }
        }
    }
    if operations.is_empty() {
        return None;
    }
    let text = |pointer: &str| {
        role.pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Some(RoleGrant {
        role: versioned_id(role),
        name: text("/name/value"),
        operations,
        qualifiers,
        performer: role.get("performer").map(|_| PartyRef {
            rm_type: text("/performer/type"),
            id: text("/performer/id/value"),
        }),
        role_validity: validity(role.get("time_validity")),
        capability_validity: capabilities
            .iter()
            .map(|capability| validity(capability.get("time_validity")))
            .collect(),
    })
}

/// The client an actor document binds, or `None` when it holds no
/// `openehr_access::client_id` element.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON demographic parties"
)]
#[must_use]
pub fn client_of(actor: &serde_json::Value) -> Option<SystemAgent> {
    let mut found = Vec::new();
    elements(actor, &mut found);
    let element = found.into_iter().find(|element| {
        element
            .get("name")
            .and_then(access_code)
            .is_some_and(|code| code.code == "client_id")
    })?;
    let identifier = |key: &str| {
        element
            .pointer(&format!("/value/{key}"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Some(SystemAgent {
        id: versioned_id(actor),
        name: actor
            .pointer("/name/value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        client_id: identifier("id"),
        issuer: identifier("issuer"),
        acts_for: Vec::new(),
    })
}

/// The grant a `ROLE` document's text holds, as [`role_grant`] reads it;
/// `None` for a document that is no JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON demographic parties"
)]
#[must_use]
pub fn role_grant_in(body: &str) -> Option<RoleGrant> {
    // NOTE: no specification governs this: our own design; the party read
    // parsed this body already, so one that is no JSON marks nothing.
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .as_ref()
        .and_then(role_grant)
}

/// The client an actor document's text binds, as [`client_of`] reads it;
/// `None` for a document that is no JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON demographic parties"
)]
#[must_use]
pub fn client_in(body: &str) -> Option<SystemAgent> {
    // NOTE: no specification governs this: our own design; the party read
    // parsed this body already, so one that is no JSON marks nothing.
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .as_ref()
        .and_then(client_of)
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "a party fixture is raw JSON by the testing rule (.claude/rules/testing.md)"
)]
mod tests {
    use serde_json::json;

    use super::{Validity, client_of, read_grants, role_grant};

    fn coded(code: &str) -> serde_json::Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
                "code_string": code } })
    }

    fn role() -> serde_json::Value {
        json!({ "_type": "ROLE", "name": { "_type": "DV_TEXT", "value": "scim_provision" },
            "uid": { "_type": "OBJECT_VERSION_ID", "value": "r1::localhost::1" },
            "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
                "id": { "_type": "HIER_OBJECT_ID", "value": "a1" } },
            "time_validity": { "_type": "DV_INTERVAL",
                "lower": { "_type": "DV_DATE", "value": "2026-01-01" } },
            "capabilities": [{ "_type": "CAPABILITY", "credentials": { "_type": "ITEM_TREE",
                "items": [
                    { "_type": "ELEMENT", "name": { "_type": "DV_TEXT", "value": "operation" },
                      "value": coded("scim_provision") },
                    { "_type": "ELEMENT", "name": coded("binding_issuer"),
                      "value": { "_type": "DV_TEXT", "value": "urn:example:basic-user" } }
                ] } }] })
    }

    #[test]
    fn a_role_reads_its_operation_its_qualifier_and_its_performer() {
        let grant = role_grant(&role()).expect("a grant");
        assert_eq!("r1", grant.role);
        assert_eq!(
            vec!["scim_provision"],
            grant
                .operations
                .iter()
                .map(|code| code.code.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(1, grant.qualifiers.len());
        assert_eq!(
            Some(("binding_issuer", "urn:example:basic-user")),
            grant
                .qualifiers
                .first()
                .map(|(code, text)| (code.code.as_str(), text.as_str()))
        );
        assert_eq!(
            Some("AGENT"),
            grant.performer.as_ref().map(|party| party.rm_type.as_str())
        );
        let at = |text: &str| text.parse::<jiff::Timestamp>().expect("an instant");
        assert!(grant.in_force(at("2026-11-26T10:00:00Z")));
        assert!(!grant.in_force(at("2025-12-31T10:00:00Z")));
    }

    #[test]
    fn a_role_of_another_terminology_grants_nothing() {
        let mut other = role();
        other["capabilities"][0]["credentials"]["items"][0]["value"]["defining_code"]["terminology_id"]
            ["value"] = json!("local");
        assert_eq!(None, role_grant(&other));
    }

    #[test]
    fn an_agent_reads_its_client_and_the_provider_it_acts_for() {
        let agent = json!({ "_type": "AGENT", "name": { "_type": "DV_TEXT", "value": "lab" },
            "uid": { "_type": "OBJECT_VERSION_ID", "value": "a1::localhost::1" },
            "details": { "_type": "ITEM_TREE", "items": [{ "_type": "ELEMENT",
                "name": coded("client_id"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": "https://login.example.org",
                    "id": "lab-system", "type": "client_id" } }] } });
        let client = client_of(&agent).expect("a client");
        assert_eq!("lab-system", client.client_id);
        let grants = read_grants(
            &[vec![role()]],
            &[vec![agent]],
            &[vec![json!("a1"), json!("o1"), json!("ORGANISATION")]],
        );
        let agent = grants.agents.first().expect("one agent");
        assert_eq!(
            "o1",
            agent
                .acts_for
                .first()
                .map(|party| party.id.as_str())
                .unwrap_or_default()
        );
        assert_eq!(1, grants.roles.len());
        assert!(!grants.truncated);
    }

    #[test]
    fn an_open_validity_holds_every_date() {
        assert!(Validity::default().holds("2026-11-26"));
        let ended = Validity {
            lower: None,
            upper: Some("2026-11-25".to_owned()),
        };
        assert!(!ended.holds("2026-11-26"));
    }
}
