// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a policy party's body holds that the grant rows read: a system
//! `AGENT`'s client identifiers and a policy `ROLE`'s capabilities, derived
//! from the body once, in Rust, with the rest of [`crate::party_binding::derive`].
//!
//! The node ids read are the contract of the policy archetypes: the client
//! identifier at [`AGENT_CLIENT_PATH`], each capability's coded `name`, its
//! `time_validity` and its `credentials`, and in those the `Not before` and
//! `Not after` elements of each `ROLE` kind, a node of a specialised archetype
//! (`id15.1`) standing for the node it redefines. No openEHR spec gives a
//! capability an authorization meaning: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored canonical party body \
              the grant rows are derived from"
)]

use std::sync::LazyLock;

use serde_json::Value;

use crate::party_binding::IdentifierSelector;
use crate::party_binding::grants::kind::stored_name;
use crate::party_binding::policy::{AGENT_CLIENT_PATH, AGENT_CLIENT_TYPE, PolicyKind, PolicyRole};
use crate::versioning::Kind;

/// The selector of a system `AGENT`'s client identifiers.
#[expect(
    clippy::expect_used,
    reason = "AGENT_CLIENT_PATH and AGENT_CLIENT_TYPE are constants the unit test compiles"
)]
static CLIENT: LazyLock<IdentifierSelector> = LazyLock::new(|| {
    IdentifierSelector::new(AGENT_CLIENT_PATH, AGENT_CLIENT_TYPE)
        .expect("the AGENT client path should compile")
});

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

/// One `CAPABILITY` of a policy `ROLE`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CapabilityFacts {
    /// The kind it is stored under ([`stored_name`]).
    pub(crate) kind: String,
    /// Its `time_validity`.
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
    /// The policy kind of the body's root archetype.
    pub(crate) policy: Option<PolicyKind>,
    /// A system `AGENT`'s client identifiers, as `(issuer, client_id)`,
    /// distinct, in document order.
    pub(crate) clients: Vec<(String, String)>,
    /// A policy `ROLE`'s capabilities, in document order.
    pub(crate) capabilities: Vec<CapabilityFacts>,
}

impl GrantFacts {
    /// Derives what `body`, a party of `kind`, holds.
    pub(crate) fn of(kind: Kind, body: &Value) -> Self {
        let policy = body
            .get("archetype_node_id")
            .and_then(Value::as_str)
            .and_then(PolicyKind::of);
        let mut facts = Self {
            policy,
            ..Self::default()
        };
        match (kind, policy) {
            (Kind::Agent, Some(PolicyKind::Agent)) => {
                for identifier in CLIENT.identifiers(body) {
                    let client = (identifier.issuer, identifier.id);
                    if !facts.clients.contains(&client) {
                        facts.clients.push(client);
                    }
                }
            }
            (Kind::Role, Some(PolicyKind::Role(role))) => {
                facts.capabilities = body
                    .get("capabilities")
                    .and_then(Value::as_array)
                    .map(|capabilities| {
                        capabilities
                            .iter()
                            .map(|capability| CapabilityFacts::of(role, capability))
                            .collect()
                    })
                    .unwrap_or_default();
            }
            _ => {}
        }
        facts
    }
}

impl CapabilityFacts {
    /// What `capability`, of a `role` kind `ROLE`, holds.
    fn of(role: PolicyRole, capability: &Value) -> Self {
        let code = capability
            .get("name")
            .and_then(|name| name.get("defining_code"))
            .and_then(|code| code.get("code_string"))
            .and_then(Value::as_str);
        let credentials = capability.get("credentials").cloned();
        let (before, after) = bound_nodes(role);
        Self {
            kind: stored_name(role, code),
            validity: capability.get("time_validity").cloned(),
            not_before: bound(credentials.as_ref(), before),
            not_after: bound(credentials.as_ref(), after),
            credentials,
        }
    }
}

/// The node ids of the `Not before` and `Not after` elements of a `role`
/// kind `ROLE`'s credentials, as its default definition defines them.
const fn bound_nodes(role: PolicyRole) -> (&'static str, &'static str) {
    match role {
        PolicyRole::SystemClient | PolicyRole::DataMigration => ("id15", "id16"),
        PolicyRole::FederationPartner => ("id30", "id31"),
        PolicyRole::Privileged => ("id9", "id10"),
    }
}

/// Whether the node id `held` is `node` or a specialisation of it.
pub(crate) fn is_node(held: &str, node: &str) -> bool {
    held == node
        || held
            .strip_prefix(node)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// The bound the element `node` of `credentials` names.
fn bound(credentials: Option<&Value>, node: &str) -> Bound {
    let Some(element) = credentials
        .and_then(|tree| tree.get("items"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item.get("archetype_node_id")
                    .and_then(Value::as_str)
                    .is_some_and(|held| is_node(held, node))
            })
        })
    else {
        return Bound::Open;
    };
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

    fn capability(code: &str, items: &Value) -> Value {
        json!({
            "_type": "CAPABILITY", "archetype_node_id": "id6",
            "name": {"_type": "DV_CODED_TEXT", "value": "x",
                     "defining_code": {"code_string": code}},
            "credentials": {"_type": "ITEM_TREE", "archetype_node_id": "id8", "items": items}
        })
    }

    fn at(node: &str, value: &str) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": node,
               "value": {"_type": "DV_DATE_TIME", "value": value}})
    }

    #[test]
    fn the_client_selector_compiles() {
        assert_eq!(CLIENT.path(), AGENT_CLIENT_PATH);
    }

    #[test]
    fn an_agent_holds_its_client_identifiers_once_each() {
        let identifier = |issuer: &str, id: &str, kind: &str| {
            json!({"_type": "ELEMENT", "archetype_node_id": "id3",
                   "value": {"_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": kind}})
        };
        let body = json!({
            "_type": "AGENT", "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.system.v1",
            "details": {"archetype_node_id": "id2", "items": [
                identifier("https://idp", "lab", "client_id"),
                identifier("https://idp", "lab", "client_id"),
                identifier("https://idp", "LAB-1", "serial"),
                identifier("https://other", "lab", "client_id")
            ]}
        });
        let facts = GrantFacts::of(Kind::Agent, &body);
        assert_eq!(facts.policy, Some(PolicyKind::Agent));
        assert_eq!(
            facts.clients,
            vec![
                ("https://idp".to_owned(), "lab".to_owned()),
                ("https://other".to_owned(), "lab".to_owned())
            ]
        );
        assert_eq!(
            GrantFacts::of(Kind::Person, &body).clients,
            Vec::<(String, String)>::new(),
            "only an AGENT holds a client"
        );
    }

    #[test]
    fn a_role_holds_each_capability_with_its_bounds() {
        let body = json!({
            "_type": "ROLE", "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1",
            "capabilities": [
                capability("at10", &json!([at("id9", "2026-12-05T22:00:00+01:00"),
                                          at("id10.1", "2026-12-06T06:00:00")])),
                capability("at12", &json!([at("id9", "not a time")])),
                capability("at77", &json!([]))
            ]
        });
        let facts = GrantFacts::of(Kind::Role, &body);
        let kinds: Vec<&str> = facts.capabilities.iter().map(|c| c.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "privileged.subject_audit",
                "privileged.identifier_reader",
                "privileged:at77"
            ]
        );
        let first = &facts.capabilities[0];
        assert_eq!(
            first.not_before,
            Bound::At("2026-12-05T21:00:00Z".parse().expect("an instant"))
        );
        assert_eq!(
            first.not_after,
            Bound::At("2026-12-06T06:00:00Z".parse().expect("an instant"))
        );
        assert_eq!(facts.capabilities[1].not_before, Bound::Unreadable);
        assert_eq!(facts.capabilities[1].not_after, Bound::Open);
    }

    #[test]
    fn a_date_alone_names_no_instant() {
        assert_eq!(instant("2026-12-05"), None);
        assert!(instant("2026-12-05T22:00").is_some());
        assert!(instant("2026-12-05T22:00:00Z").is_some());
    }
}
