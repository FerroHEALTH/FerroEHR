// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How an access-log entry is stored: the part that describes the caller
//! once, the part that describes one access per row (#5703).
//!
//! Every entry of one request repeats who accessed, how they authenticated,
//! the roles and authorisations they acted under, their purposes and the
//! retention their access calls for, and the same caller's next request
//! repeats it again. That shared part is stored once in
//! `audit.access_log_shared`, keyed by the SHA-256 of its canonical form, and
//! each row of `audit.access_log_record` keeps the rest: its uid, time, EHR,
//! items, outcome, request and query, and the access details that differ per
//! EHR. The entry stays one logical record with its own link:
//! `content_digest` is the SHA-256 of the whole canonical form, computed when
//! the entry is written, and a reader joins the two parts back into exactly
//! that form, so the chain covers every byte. A tombstone replaces one row
//! and keeps no shared part. The draft fixes the entry and its canonical
//! form; how they are stored is our own design, and no openEHR spec governs
//! it.

#![expect(
    clippy::disallowed_types,
    reason = "the stored parts are computed over the entry's canonical ITS-JSON value (#1694 \
              boundary class: canonical form)"
)]

use serde_json::{Map, Value};

/// The members of an entry the shared part holds.
const SHARED: [&str; 8] = [
    "accessor",
    "action",
    "authorisations",
    "is_emergency",
    "operation",
    "purposes",
    "retention",
    "security_labels",
];

/// The at-codes of the access details the shared part holds
/// (`crate::access_log::details`): the caller, the credential, the
/// classification map and the kind of access, the same for every EHR a
/// request reached.
const SHARED_DETAILS: [&str; 18] = [
    "at0001", // event action
    "at0002", // resource class
    "at0004", // principal
    "at0005", // issuer
    "at0006", // client id
    "at0007", // token id
    "at0009", // role
    "at0010", // domain
    "at0015", // purpose
    "at0016", // legal basis
    "at0020", // category map digest
    "at0022", // acting mode
    "at0023", // assurance level
    "at0024", // assurance value
    "at0025", // professional id
    "at0026", // token attributes
    "at0027", // binding
    "at0029", // event type
];

/// The members split below their top level (#5714): the shared part keeps
/// what repeats, the row the attributes named here, which count one access.
const SPLIT: [(&str, &[&str]); 2] = [("outcome", &["delivered"]), ("query", &["rows"])];

/// The member holding the access details.
const DETAILS: &str = "other_details";
/// The member of the details tree holding its elements.
const ITEMS: &str = "items";
/// The member of an element naming its detail.
const NODE: &str = "archetype_node_id";

/// An entry as it is stored: its row part, and its shared part when it has
/// one.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredForm {
    /// The members kept on the row.
    pub row: Value,
    /// The members kept once for every entry that shares them; `None` for a
    /// record that keeps none, such as a tombstone.
    pub shared: Option<Value>,
}

/// Whether the element `item` is a shared access detail.
fn shared_detail(item: &Value) -> bool {
    item.get(NODE)
        .and_then(Value::as_str)
        .is_some_and(|code| SHARED_DETAILS.contains(&code))
}

/// The stored form of the entry whose canonical form, without its link, is
/// the object `members`.
#[must_use]
pub fn split(members: &Map<String, Value>) -> StoredForm {
    let mut row = Map::new();
    let mut shared = Map::new();
    for (name, member) in members {
        if SHARED.contains(&name.as_str()) {
            shared.insert(name.clone(), member.clone());
        } else if let (Some((_, own_keys)), Value::Object(object)) =
            (SPLIT.iter().find(|(split, _)| split == name), member)
        {
            let (own, common): (Map<String, Value>, Map<String, Value>) = object
                .clone()
                .into_iter()
                .partition(|(key, _)| own_keys.contains(&key.as_str()));
            shared.insert(name.clone(), Value::Object(common));
            if !own.is_empty() {
                row.insert(name.clone(), Value::Object(own));
            }
        } else if name == DETAILS
            && let Value::Object(tree) = member
        {
            let mut envelope = Map::new();
            let mut own = Vec::new();
            let mut common = Vec::new();
            for (field, value) in tree {
                if field == ITEMS
                    && let Value::Array(items) = value
                {
                    for item in items {
                        if shared_detail(item) {
                            common.push(item.clone());
                        } else {
                            own.push(item.clone());
                        }
                    }
                } else {
                    envelope.insert(field.clone(), value.clone());
                }
            }
            envelope.insert(ITEMS.to_owned(), Value::Array(common));
            shared.insert(DETAILS.to_owned(), Value::Object(envelope));
            let mut kept = Map::new();
            kept.insert(ITEMS.to_owned(), Value::Array(own));
            row.insert(DETAILS.to_owned(), Value::Object(kept));
        } else {
            row.insert(name.clone(), member.clone());
        }
    }
    StoredForm {
        row: Value::Object(row),
        shared: (!shared.is_empty()).then_some(Value::Object(shared)),
    }
}

/// The record `row` and `shared` store, joined back into its canonical form's
/// object.
///
/// The shared members go beside the row's, and the access details of both
/// merge in the order of their codes, which is the order the entry was
/// written in ([`crate::access_log::details::DetailsWriter::finish`]).
#[must_use]
pub fn join(row: Value, shared: Option<Value>) -> Value {
    let (Value::Object(mut members), Some(Value::Object(common))) = (row.clone(), shared) else {
        return row;
    };
    let own_details = members.remove(DETAILS);
    for (name, member) in common {
        if name == DETAILS {
            members.insert(name, joined_details(member, own_details.clone()));
        } else if let (Some(Value::Object(own)), Value::Object(mut object)) =
            (members.remove(&name), member.clone())
        {
            object.extend(own);
            members.insert(name, Value::Object(object));
        } else {
            members.insert(name, member);
        }
    }
    Value::Object(members)
}

/// The details tree `common` holds with the elements of `own` merged in, in
/// the order of their codes; within one code each side keeps its order.
fn joined_details(common: Value, own: Option<Value>) -> Value {
    let Value::Object(mut tree) = common else {
        return common;
    };
    let shared_items = match tree.remove(ITEMS) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    let own_items = match own.and_then(|own| own.get(ITEMS).cloned()) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    let code = |item: &Value| item.get(NODE).and_then(Value::as_str).map(str::to_owned);
    let mut merged = Vec::with_capacity(shared_items.len() + own_items.len());
    let mut left = shared_items.into_iter().peekable();
    let mut right = own_items.into_iter().peekable();
    loop {
        let take_left = match (left.peek(), right.peek()) {
            (Some(l), Some(r)) => code(l) <= code(r),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        let next = if take_left { left.next() } else { right.next() };
        merged.extend(next);
    }
    tree.insert(ITEMS.to_owned(), Value::Array(merged));
    Value::Object(tree)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{join, split};

    /// An entry splits into its row and shared parts and joins back into the
    /// same object, its access details merged in the order of their codes.
    #[test]
    fn an_entry_joins_back_into_its_canonical_object() {
        let entry = json!({
            "_type": "ACCESS_LOG_ENTRY",
            "uid": { "value": "u" },
            "accessor": { "kind": "person" },
            "action": { "value": "read" },
            "retention": { "grounds": [] },
            "outcome": { "result": "granted", "delivered": 1 },
            "query": { "language": "AQL", "text_digest": "d" },
            "other_details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "x",
                "items": [
                    { "archetype_node_id": "at0002", "v": 1 },
                    { "archetype_node_id": "at0004", "v": 2 },
                    { "archetype_node_id": "at0009", "v": 3 },
                    { "archetype_node_id": "at0009", "v": 4 },
                    { "archetype_node_id": "at0012", "v": 5 },
                    { "archetype_node_id": "at0018", "v": 6 },
                    { "archetype_node_id": "at0018", "v": 7 },
                    { "archetype_node_id": "at0027", "v": 8 }
                ]
            }
        });
        let Some(members) = entry.as_object() else {
            panic!("an object");
        };
        let stored = split(members);
        assert!(stored.row.get("accessor").is_none());
        assert_eq!(stored.row["outcome"], json!({ "delivered": 1 }));
        assert!(
            stored.row.get("query").is_none(),
            "a query that counts no rows is shared whole"
        );
        assert_eq!(
            stored.row["other_details"],
            json!({ "items": [
                { "archetype_node_id": "at0012", "v": 5 },
                { "archetype_node_id": "at0018", "v": 6 },
                { "archetype_node_id": "at0018", "v": 7 }
            ] })
        );
        assert_eq!(join(stored.row, stored.shared), entry);
    }

    /// The shared at-codes are the codes of the caller's details.
    #[cfg(feature = "fhir")]
    #[test]
    fn the_shared_codes_name_the_callers_details() {
        use crate::access_log::details::Detail;
        let shared = [
            Detail::EventAction,
            Detail::ResourceClass,
            Detail::Principal,
            Detail::Issuer,
            Detail::ClientId,
            Detail::TokenId,
            Detail::Role,
            Detail::Domain,
            Detail::Purpose,
            Detail::LegalBasis,
            Detail::CategoryMapDigest,
            Detail::ActingMode,
            Detail::AssuranceLevel,
            Detail::AssuranceValue,
            Detail::ProfessionalId,
            Detail::TokenAttributes,
            Detail::Binding,
            Detail::EventType,
        ];
        assert_eq!(shared.map(Detail::code), super::SHARED_DETAILS);
    }

    /// A record with no shared member, such as a tombstone, is its row.
    #[test]
    fn a_tombstone_is_its_row() {
        let tombstone = json!({ "_type": "ACCESS_LOG_TOMBSTONE", "uid": { "value": "u" } });
        let Some(members) = tombstone.as_object() else {
            panic!("an object");
        };
        let stored = split(members);
        assert_eq!(stored.shared, None);
        assert_eq!(join(stored.row, None), tombstone);
    }
}
