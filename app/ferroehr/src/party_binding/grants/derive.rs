// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a party's body holds that the grant rows read: the client
//! identifiers of an `ACTOR` and the operations each `CAPABILITY` of a `ROLE`
//! grants, derived from the body once, in Rust, with the rest of
//! [`crate::party_binding::derive`].
//!
//! Nothing is read by archetype or node id: a client identifier is any
//! `DV_IDENTIFIER` of type [`CLIENT_ID`] in the actor's `details` or
//! `identities`, an operation any `DV_CODED_TEXT` value of
//! [`crate::party_binding::policy_mark::TERMINOLOGY`] in a capability's `credentials`, and a bound the
//! `DV_DATE_TIME` of an element that terminology names
//! [`NOT_BEFORE`] or [`NOT_AFTER`]. No openEHR spec gives a capability an
//! authorization meaning: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored canonical party body \
              the grant rows are derived from"
)]

use serde_json::Value;

use crate::party_binding::derive::container;
use crate::party_binding::grants::operation::{NOT_AFTER, NOT_BEFORE, is_qualifier};
use crate::party_binding::policy_mark::{CLIENT_ID, acts_for, coded};
use crate::versioning::Kind;

/// A `Not before` or `Not after` bound of a capability's credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bound {
    /// The credentials carry none.
    Open,
    /// The instant the bound names.
    At(jiff::Timestamp),
    /// The credentials carry one that reads as no instant, so the grant never
    /// holds.
    Unreadable,
}

/// One operation a `CAPABILITY` grants.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CapabilityFacts {
    /// The capability's position in `ROLE.capabilities`.
    pub(crate) ordinal: i32,
    /// The code of [`crate::party_binding::policy_mark::TERMINOLOGY`] it holds, as written.
    pub(crate) code: String,
    /// The capability's `time_validity`.
    pub(crate) validity: Option<Value>,
    /// The `Not before` bound of its credentials.
    pub(crate) not_before: Bound,
    /// The `Not after` bound of its credentials.
    pub(crate) not_after: Bound,
    /// Its `credentials` as written.
    pub(crate) credentials: Option<Value>,
}

/// What a body holds that the grant rows read.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct GrantFacts {
    /// An actor's client identifiers, as `(issuer, client_id)`, distinct, in
    /// document order.
    pub(crate) clients: Vec<(String, String)>,
    /// A `ROLE`'s granted operations, one per capability and code, in
    /// document order.
    pub(crate) capabilities: Vec<CapabilityFacts>,
    /// The container an actor's first inline acts-for `PARTY_RELATIONSHIP`
    /// targets: the provider a client acts for.
    pub(crate) provider: Option<uuid::Uuid>,
    /// An `ORGANISATION`'s identifier as a provider, `(issuer, id)`: the
    /// first `DV_IDENTIFIER` in its `details` that names no client.
    pub(crate) identifier: Option<(String, String)>,
}

impl GrantFacts {
    /// Derives what `body`, a party of `kind`, holds.
    pub(crate) fn of(kind: Kind, body: &Value) -> Self {
        let mut facts = Self::default();
        match kind {
            Kind::Agent | Kind::Person | Kind::Organisation | Kind::Group => {
                let mut found = Vec::new();
                for root in [body.get("details"), body.get("identities")]
                    .into_iter()
                    .flatten()
                {
                    walk(root, &mut |value| {
                        if let Some(client) = client(value)
                            && !found.contains(&client)
                        {
                            found.push(client);
                        }
                    });
                }
                facts.clients = found;
                facts.provider = body
                    .get("relationships")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|relationship| acts_for(relationship))
                    .find_map(|relationship| {
                        container(relationship.pointer("/target/id/value")?.as_str()?)
                    });
                if kind == Kind::Organisation {
                    facts.identifier = body.get("details").and_then(provider_identifier);
                }
            }
            Kind::Role => {
                for (ordinal, capability) in body
                    .get("capabilities")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let ordinal = i32::try_from(ordinal).unwrap_or(i32::MAX);
                    facts
                        .capabilities
                        .extend(CapabilityFacts::of(ordinal, capability));
                }
            }
            _ => {}
        }
        facts
    }
}

impl CapabilityFacts {
    /// The operations `capability`, at `ordinal`, grants.
    fn of(ordinal: i32, capability: &Value) -> Vec<Self> {
        let Some(credentials) = capability.get("credentials") else {
            return Vec::new();
        };
        let mut codes: Vec<String> = Vec::new();
        let (mut before, mut after) = (Bound::Open, Bound::Open);
        walk(credentials, &mut |node| {
            if let Some(code) = node.get("value").and_then(coded)
                && !is_qualifier(code)
                && !codes.iter().any(|held| held == code)
            {
                codes.push(code.to_owned());
            }
            match node.get("name").and_then(coded) {
                Some(NOT_BEFORE) if before == Bound::Open => before = bound(node),
                Some(NOT_AFTER) if after == Bound::Open => after = bound(node),
                _ => {}
            }
        });
        codes
            .into_iter()
            .map(|code| Self {
                ordinal,
                code,
                validity: capability.get("time_validity").cloned(),
                not_before: before,
                not_after: after,
                credentials: Some(credentials.clone()),
            })
            .collect()
    }
}

/// Calls `visit` on `value` and every object below it.
pub(crate) fn walk<'a>(value: &'a Value, visit: &mut impl FnMut(&'a Value)) {
    match value {
        Value::Object(map) => {
            visit(value);
            for member in map.values() {
                walk(member, visit);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, visit);
            }
        }
        _ => {}
    }
}

/// The `(issuer, client_id)` of `value` when it is a `DV_IDENTIFIER` of type
/// [`CLIENT_ID`] with an issuer and an id.
fn client(value: &Value) -> Option<(String, String)> {
    let text = |key: &str| value.get(key).and_then(Value::as_str);
    (text("_type") == Some("DV_IDENTIFIER") && text("type") == Some(CLIENT_ID))
        .then(|| Some((text("issuer")?.to_owned(), text("id")?.to_owned())))
        .flatten()
}

/// The `(issuer, id)` of the first `DV_IDENTIFIER` below `details` that names
/// no client.
fn provider_identifier(details: &Value) -> Option<(String, String)> {
    let mut found = None;
    walk(details, &mut |value| {
        let text = |key: &str| value.get(key).and_then(Value::as_str);
        if found.is_none()
            && text("_type") == Some("DV_IDENTIFIER")
            && text("type") != Some(CLIENT_ID)
            && let (Some(issuer), Some(id)) = (text("issuer"), text("id"))
        {
            found = Some((issuer.to_owned(), id.to_owned()));
        }
    });
    found
}

/// The bound the `DV_DATE_TIME` value of the element `element` names.
fn bound(element: &Value) -> Bound {
    element
        .get("value")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
        .and_then(instant)
        .map_or(Bound::Unreadable, Bound::At)
}

/// The instant an ISO 8601 date-time names (RM `data_types` `DV_DATE_TIME`):
/// at its offset, or in UTC when it carries none, the zone the server reads
/// every `DV_DATE` in.
///
/// A `DV_DATE_TIME` reduced below the hour, or a text that is no date-time,
/// names none.
pub(crate) fn instant(text: &str) -> Option<jiff::Timestamp> {
    let text = text.trim();
    if let Ok(at) = text.parse::<jiff::Timestamp>() {
        return Some(at);
    }
    // NOTE: RM data_types DV_DATE_TIME, the offset is optional; a form without one
    // reads in UTC, our own design, as the date of a time_validity does.
    let civil = text.parse::<jiff::civil::DateTime>().ok()?;
    text.contains('T')
        .then(|| civil.to_zoned(jiff::tz::TimeZone::UTC).ok())
        .flatten()
        .map(|zoned| zoned.timestamp())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    fn coded_text(code: &str, terminology: &str) -> Value {
        json!({"_type": "DV_CODED_TEXT", "value": code,
               "defining_code": {"terminology_id": {"value": terminology}, "code_string": code}})
    }

    fn element(name: &Value, value: &Value) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "at0001", "name": name, "value": value})
    }

    #[test]
    fn any_actor_holds_its_client_identifiers_once_each() {
        let identifier = |issuer: &str, id: &str, kind: &str| {
            json!({"_type": "ELEMENT", "archetype_node_id": "at9",
                   "value": {"_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": kind}})
        };
        let body = json!({
            "_type": "ORGANISATION", "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.mine.v3",
            "details": {"items": [{"items": [
                identifier("https://idp", "lab", "client_id"),
                identifier("https://idp", "lab", "client_id"),
                identifier("https://idp", "LAB-1", "serial")
            ]}]},
            "identities": [{"details": {"items": [identifier("https://other", "lab", "client_id")]}}]
        });
        let facts = GrantFacts::of(Kind::Organisation, &body);
        assert_eq!(
            facts.clients,
            vec![
                ("https://idp".to_owned(), "lab".to_owned()),
                ("https://other".to_owned(), "lab".to_owned())
            ]
        );
        assert_eq!(GrantFacts::of(Kind::Role, &body).clients.len(), 0);
    }

    #[test]
    fn a_capability_grants_each_operation_code_anywhere_in_its_credentials() {
        let body = json!({
            "_type": "ROLE", "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.anything.v7",
            "capabilities": [
                {"_type": "CAPABILITY", "credentials": {"items": [
                    {"items": [element(&json!({"value": "op"}), &coded_text("import_ehr", TERMINOLOGY))]},
                    element(&json!({"value": "op"}), &coded_text("export_ehr_extract", TERMINOLOGY)),
                    element(&json!({"value": "op"}), &coded_text("import_ehr", TERMINOLOGY)),
                    element(&json!({"value": "other"}), &coded_text("import_ehr", "local")),
                    element(&coded_text(NOT_BEFORE, TERMINOLOGY),
                            &json!({"_type": "DV_DATE_TIME", "value": "2026-12-05T22:00:00+01:00"})),
                    element(&coded_text(NOT_AFTER, TERMINOLOGY),
                            &json!({"_type": "DV_DATE_TIME", "value": "not a time"})),
                    element(&coded_text("template", TERMINOLOGY), &json!({"_type": "DV_TEXT", "value": "T.v1"}))
                ]}},
                {"_type": "CAPABILITY", "credentials": {"items": []}}
            ]
        });
        let facts = GrantFacts::of(Kind::Role, &body);
        let codes: Vec<&str> = facts.capabilities.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(codes, vec!["import_ehr", "export_ehr_extract"]);
        let first = &facts.capabilities[0];
        assert_eq!(first.ordinal, 0);
        assert_eq!(
            first.not_before,
            Bound::At("2026-12-05T21:00:00Z".parse().expect("an instant"))
        );
        assert_eq!(first.not_after, Bound::Unreadable);
    }

    #[test]
    fn a_date_alone_names_no_instant() {
        assert_eq!(instant("2026-12-05"), None);
        assert!(instant("2026-12-05T22:00").is_some());
        assert!(instant("2026-12-05T22:00:00Z").is_some());
    }
}
