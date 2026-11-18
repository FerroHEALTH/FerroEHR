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
use crate::party_binding::grants::caller::{self, CallerFact, FactKind};
use crate::party_binding::grants::operation::{NOT_AFTER, NOT_BEFORE, Operation, is_qualifier};
use crate::party_binding::policy_mark::{ACTS_FOR, CLIENT_ID, TERMINOLOGY, carries, coded};
use crate::versioning::Kind;
use crate::versioning::tree::{Shape, Tree};

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
    /// The caller facts: the role a `ROLE` names, the care relationships of
    /// a `PERSON` and the memberships of an `ORGANISATION`, in document
    /// order.
    pub(crate) facts: Vec<CallerFact>,
    /// The EHR subject references a `PERSON` declares in its `details`,
    /// distinct, in document order.
    pub(crate) subjects: Vec<caller::subject::DeclaredSubject>,
    /// Why a fact the body carries cannot be read, which its commit refuses.
    pub(crate) unread: Vec<String>,
}

impl GrantFacts {
    pub(crate) fn of<'a, T: Tree<'a>>(kind: Kind, body: T) -> Self {
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
                    .into_iter()
                    .flat_map(Tree::items)
                    .filter(|relationship| carries(*relationship, TERMINOLOGY, ACTS_FOR))
                    .find_map(|relationship| {
                        container(&relationship.pointer_text("/target/id/value")?)
                    });
                if kind == Kind::Organisation {
                    facts.identifier = body.get("details").and_then(provider_identifier);
                    caller::coded_name(body, FactKind::OrganisationType, &mut facts);
                }
                caller::relationships(kind, body, &mut facts);
            }
            Kind::Role => {
                caller::coded_name(body, FactKind::Role, &mut facts);
                for (ordinal, capability) in body
                    .get("capabilities")
                    .into_iter()
                    .flat_map(Tree::items)
                    .enumerate()
                {
                    let ordinal = i32::try_from(ordinal).unwrap_or(i32::MAX);
                    facts
                        .capabilities
                        .extend(CapabilityFacts::of(ordinal, capability));
                    unknown_names(ordinal, capability, &mut facts.unread);
                }
            }
            _ => caller::relationships(kind, body, &mut facts),
        }
        caller::subject::declared(kind, body, &mut facts);
        facts
    }
}

/// Pushes onto `unread` each element name of `capability`, at `ordinal`,
/// that is a code of [`crate::party_binding::policy_mark::TERMINOLOGY`] no
/// table defines, such as a removed one: neither a qualifier nor an
/// operation, which its commit refuses.
fn unknown_names<'a, T: Tree<'a>>(ordinal: i32, capability: T, unread: &mut Vec<String>) {
    let Some(credentials) = capability.get("credentials") else {
        return;
    };
    walk(credentials, &mut |node| {
        if let Some(code) = node.get("name").and_then(coded)
            && !is_qualifier(&code)
            && Operation::of(code.trim()).is_none()
        {
            unread.push(format!(
                "ROLE.capabilities[{ordinal}].credentials: an element is named {code}, which no \
                 table of the operation terminology defines"
            ));
        }
    });
}

impl CapabilityFacts {
    /// The facts of the capability at `ordinal`, one per operation it grants.
    fn of<'a, T: Tree<'a>>(ordinal: i32, capability: T) -> Vec<Self> {
        let Some(credentials) = capability.get("credentials") else {
            return Vec::new();
        };
        let mut codes: Vec<String> = Vec::new();
        walk(credentials, &mut |node| {
            let name = node.get("name").and_then(coded);
            // NOTE: the value of an element a qualifier names is that qualifier's
            // value, such as an agreement's access class, and never an operation.
            if let Some(code) = node.get("value").and_then(coded)
                && !name.as_deref().is_some_and(is_qualifier)
                && !is_qualifier(&code)
                && !codes.iter().any(|held| *held == code)
            {
                codes.push(code.into_owned());
            }
        });
        let (before, after) = bounds(credentials);
        let validity = capability.get("time_validity").map(Tree::to_value);
        let credentials = Some(credentials.to_value());
        codes
            .into_iter()
            .map(|code| Self {
                ordinal,
                code,
                validity: validity.clone(),
                not_before: before,
                not_after: after,
                credentials: credentials.clone(),
            })
            .collect()
    }
}

/// The `not_before` and `not_after` bounds `credentials` name, each
/// [`Bound::Open`] when no element names it.
pub(crate) fn bounds<'a, T: Tree<'a>>(credentials: T) -> (Bound, Bound) {
    let (mut before, mut after) = (Bound::Open, Bound::Open);
    walk(
        credentials,
        &mut |node| match node.get("name").and_then(coded).as_deref() {
            Some(NOT_BEFORE) if before == Bound::Open => before = bound(node),
            Some(NOT_AFTER) if after == Bound::Open => after = bound(node),
            _ => {}
        },
    );
    (before, after)
}

/// Visits every object in `value`, itself first, in document order.
pub(crate) fn walk<'a, T: Tree<'a>>(value: T, visit: &mut impl FnMut(T)) {
    match value.shape() {
        Shape::Object => {
            visit(value);
            for (_, member) in value.members() {
                walk(member, visit);
            }
        }
        Shape::Array => {
            for item in value.items() {
                walk(item, visit);
            }
        }
        Shape::String | Shape::Other => {}
    }
}

/// The `(issuer, id)` of `value` when it is a client identifier.
fn client<'a, T: Tree<'a>>(value: T) -> Option<(String, String)> {
    let text = |key: &str| value.get_text(key);
    (text("_type").as_deref() == Some("DV_IDENTIFIER")
        && text("type").as_deref() == Some(CLIENT_ID))
    .then(|| Some((text("issuer")?.into_owned(), text("id")?.into_owned())))
    .flatten()
}

/// The `(issuer, id)` of the first identifier in `details` that is no client
/// identifier.
fn provider_identifier<'a, T: Tree<'a>>(details: T) -> Option<(String, String)> {
    let mut found = None;
    walk(details, &mut |value| {
        let text = |key: &str| value.get_text(key);
        if found.is_none()
            && text("_type").as_deref() == Some("DV_IDENTIFIER")
            && text("type").as_deref() != Some(CLIENT_ID)
            && let (Some(issuer), Some(id)) = (text("issuer"), text("id"))
        {
            found = Some((issuer.into_owned(), id.into_owned()));
        }
    });
    found
}

/// The bound the `DV_DATE_TIME` value of `element` sets.
fn bound<'a, T: Tree<'a>>(element: T) -> Bound {
    element
        .pointer_text("/value/value")
        .and_then(|text| instant(&text))
        .map_or(Bound::Unreadable, Bound::At)
}

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
                    element(&coded_text("template", TERMINOLOGY), &json!({"_type": "DV_TEXT", "value": "T.v1"})),
                    element(&coded_text("access_class", TERMINOLOGY), &coded_text("automated", TERMINOLOGY))
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
