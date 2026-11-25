// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How an access-log entry is stored: the part that describes the caller
//! once, the part that describes one access per row (#5703).
//!
//! Every entry of one request repeats who accessed, how they authenticated,
//! the roles and authorisations they acted under, their purposes and the
//! retention their access calls for, and the same caller's next request
//! repeats it again, as their accesses to data of one template repeat what
//! the classification read. That shared part is stored once in
//! `audit.access_log_shared`, keyed by the SHA-256 of its canonical form, and
//! each row of `audit.access_log_entry` keeps the rest: its uid, time, EHR,
//! items, outcome, request and query, and the access details that differ per
//! EHR, and its only item's categories, which the category map classified
//! from the shared category content (#6322). The entry stays one logical
//! record: a reader joins the two parts and the row's columns back into its
//! canonical form by the renderer of the row's stored form (`form`), and
//! `content_digest` is the SHA-256 of those stored facts
//! ([`parts::facts::facts_digest`]), computed when the entry is written, so the
//! chain covers every stored byte and no rendering. A tombstone replaces one
//! row and keeps no shared part. The draft fixes the entry and its canonical
//! form; how they are stored is our own design, and no openEHR spec governs
//! it.

#![cfg_attr(
    test,
    expect(
        clippy::disallowed_types,
        reason = "the reference split is computed over the entry's canonical ITS-JSON value \
                  (#1694 boundary class: canonical form)"
    )
)]

#[cfg(test)]
use serde_json::Map;
#[cfg(test)]
use serde_json::Value;

pub mod parts;

/// The members of an entry the shared part holds.
pub(crate) const SHARED: [&str; 7] = [
    "accessor",
    "action",
    "authorisations",
    "operation",
    "purposes",
    "retention",
    "security_labels",
];

/// The id-codes of the access details the shared part holds
/// (`crate::access_log::details`): the caller, the credential, the
/// classification map and the kind of access, the same for every EHR a
/// request reached, and what the classification read, which repeats for
/// every access of a caller to data of one template or by one query shape
/// (#6005).
const SHARED_DETAILS: [&str; 19] = [
    "id11", // event action
    "id12", // resource class
    "id14", // principal
    "id15", // issuer
    "id16", // client id
    "id17", // token id
    "id19", // role
    "id20", // domain
    "id25", // purpose
    "id26", // legal basis
    "id30", // category map digest
    "id31", // category content
    "id32", // acting mode
    "id33", // assurance level
    "id34", // assurance value
    "id35", // professional id
    "id36", // token attributes
    "id37", // binding
    "id39", // event type
];

/// The members split below their top level (#5714): the shared part keeps
/// what repeats, the row the attributes named here, which count one access.
const SPLIT: [(&str, &[&str]); 2] = [("outcome", &["delivered"]), ("query", &["rows"])];

/// The member holding the accessed items.
const ACCESSED: &str = "items";

/// The member of an only accessed item the shared part keeps (#6322): its
/// categories, which the category map classified from what the shared
/// part's category content (`id31`) holds, so they repeat wherever it does.
const ITEM_SHARED: &str = "categories";

/// The member holding the access details.
const DETAILS: &str = "other_details";
/// The member of the details tree holding its elements.
const ITEMS: &str = "items";
/// The member of an element naming its detail.
const NODE: &str = "archetype_node_id";

/// An entry as it is stored, as JSON values: its row part, and its shared
/// part when it has one.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub struct StoredForm {
    /// The members kept on the row.
    pub row: Value,
    /// The members kept once for every entry that shares them; `None` for a
    /// record that keeps none, such as a tombstone.
    pub shared: Option<Value>,
}

/// Whether the element `item` is a shared access detail.
#[cfg(test)]
fn shared_detail(item: &Value) -> bool {
    item.get(NODE)
        .and_then(Value::as_str)
        .is_some_and(|code| SHARED_DETAILS.contains(&code))
}

/// Whether `items` is one item that holds [`ITEM_SHARED`].
#[cfg(test)]
fn one_item_shares(items: &[Value]) -> bool {
    matches!(items, [Value::Object(item)] if item.contains_key(ITEM_SHARED))
}

/// The stored form of the entry whose canonical form, without its link, is
/// the object `members`, its members moved into the part that keeps them.
#[cfg(test)]
#[must_use]
pub fn split(members: Map<String, Value>) -> StoredForm {
    let mut row = Map::new();
    let mut shared = Map::new();
    for (name, member) in members {
        if SHARED.contains(&name.as_str()) {
            shared.insert(name, member);
            continue;
        }
        let member = match (SPLIT.iter().find(|(split, _)| *split == name), member) {
            (Some((_, own_keys)), Value::Object(object)) => {
                let (own, common): (Map<String, Value>, Map<String, Value>) = object
                    .into_iter()
                    .partition(|(key, _)| own_keys.contains(&key.as_str()));
                if !own.is_empty() {
                    row.insert(name.clone(), Value::Object(own));
                }
                shared.insert(name, Value::Object(common));
                continue;
            }
            (_, member) => member,
        };
        let member = match member {
            Value::Array(mut items) if name == ACCESSED && one_item_shares(&items) => {
                let mut common = Map::new();
                if let Some(Value::Object(item)) = items.first_mut()
                    && let Some(categories) = item.remove(ITEM_SHARED)
                {
                    common.insert(ITEM_SHARED.to_owned(), categories);
                }
                shared.insert(name.clone(), Value::Array(vec![Value::Object(common)]));
                Value::Array(items)
            }
            member => member,
        };
        match member {
            Value::Object(tree) if name == DETAILS => {
                let (envelope, own) = split_details(tree);
                shared.insert(DETAILS.to_owned(), Value::Object(envelope));
                if !own.is_empty() {
                    let mut kept = Map::new();
                    kept.insert(ITEMS.to_owned(), Value::Array(own));
                    row.insert(DETAILS.to_owned(), Value::Object(kept));
                }
            }
            member => {
                row.insert(name, member);
            }
        }
    }
    StoredForm {
        row: Value::Object(row),
        shared: (!shared.is_empty()).then_some(Value::Object(shared)),
    }
}

/// The details tree `tree` split: its envelope with the shared elements, and
/// the elements of the row.
#[cfg(test)]
fn split_details(tree: Map<String, Value>) -> (Map<String, Value>, Vec<Value>) {
    let mut envelope = Map::new();
    let mut own = Vec::new();
    let mut common = Vec::new();
    for (field, value) in tree {
        match value {
            Value::Array(items) if field == ITEMS => {
                for item in items {
                    if shared_detail(&item) {
                        common.push(item);
                    } else {
                        own.push(item);
                    }
                }
            }
            value => {
                envelope.insert(field, value);
            }
        }
    }
    envelope.insert(ITEMS.to_owned(), Value::Array(common));
    (envelope, own)
}

/// The uuid column that keeps the id `text` of an entry: `Some` only for a
/// UUID in its lower-case hyphenated form, which the column gives back as it
/// was written (#5784).
#[must_use]
pub fn column_uuid(text: &str) -> Option<uuid::Uuid> {
    uuid::Uuid::parse_str(text).ok().filter(|id| {
        id.hyphenated()
            .encode_lower(&mut uuid::Uuid::encode_buffer())
            == text
    })
}

/// Whether the `timestamptz` column keeps the instant `text` of an entry so
/// that `audit.access_log_instant_text` gives the text back (#5784): UTC,
/// a four-digit year, the microseconds without trailing zeros.
#[must_use]
pub fn column_instant(text: &str) -> bool {
    text.parse::<jiff::Timestamp>()
        .ok()
        .and_then(instant_text)
        .is_some_and(|written| written == text)
}

/// The instant `at` as `audit.access_log_instant_text` writes it; `None`
/// when that text would not give `at` back, finer than a microsecond or past
/// a four-digit year.
pub(super) fn instant_text(at: jiff::Timestamp) -> Option<String> {
    let civil = at.to_zoned(jiff::tz::TimeZone::UTC).datetime();
    let nanos = civil.subsec_nanosecond();
    if nanos.rem_euclid(1000) != 0 || !(1000..=9999).contains(&civil.year()) {
        return None;
    }
    let micros = nanos.div_euclid(1000);
    let fraction = if micros == 0 {
        String::new()
    } else {
        format!(".{micros:06}").trim_end_matches('0').to_owned()
    };
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{fraction}Z",
        civil.year(),
        civil.month(),
        civil.day(),
        civil.hour(),
        civil.minute(),
        civil.second()
    ))
}

#[cfg(test)]
/// The record `row` and `shared` store, joined back into its canonical form's
/// object.
#[cfg(test)]
#[must_use]
pub fn join(row: Value, shared: Option<Value>) -> Value {
    let (row, common) = match (row, shared) {
        (Value::Object(row), Some(Value::Object(common))) => (row, common),
        (row, _) => return row,
    };
    let mut members = row;
    let mut own_details = members.remove(DETAILS);
    for (name, member) in common {
        if name == DETAILS {
            members.insert(name, joined_details(member, own_details.take()));
            continue;
        }
        if name == ACCESSED
            && let (Some(Value::Array(mut own)), Value::Array(common)) =
                (members.remove(&name), member.clone())
        {
            if let (Some(Value::Object(item)), Some(Value::Object(shared_item))) =
                (own.first_mut(), common.into_iter().next())
            {
                item.extend(shared_item);
            }
            members.insert(name, Value::Array(own));
            continue;
        }
        match (members.remove(&name), member) {
            (Some(Value::Object(own)), Value::Object(mut object)) => {
                object.extend(own);
                members.insert(name, Value::Object(object));
            }
            (_, member) => {
                members.insert(name, member);
            }
        }
    }
    Value::Object(members)
}

/// The id-code an element of the details tree names.
#[cfg(test)]
fn code(item: &Value) -> Option<&str> {
    item.get(NODE).and_then(Value::as_str)
}

/// The details tree `common` holds with the elements of `own` merged in, in
/// the order of their codes; within one code each side keeps its order.
#[cfg(test)]
fn joined_details(common: Value, own: Option<Value>) -> Value {
    let Value::Object(mut tree) = common else {
        return common;
    };
    let shared_items = match tree.remove(ITEMS) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    let own_items = match own {
        Some(Value::Object(mut own)) => match own.remove(ITEMS) {
            Some(Value::Array(items)) => items,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
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
                    { "archetype_node_id": "id12", "v": 1 },
                    { "archetype_node_id": "id14", "v": 2 },
                    { "archetype_node_id": "id19", "v": 3 },
                    { "archetype_node_id": "id19", "v": 4 },
                    { "archetype_node_id": "id22", "v": 5 },
                    { "archetype_node_id": "id28", "v": 6 },
                    { "archetype_node_id": "id28", "v": 7 },
                    { "archetype_node_id": "id37", "v": 8 }
                ]
            }
        });
        let Some(members) = entry.as_object() else {
            panic!("an object");
        };
        let stored = split(members.clone());
        assert!(stored.row.get("accessor").is_none());
        assert_eq!(stored.row["outcome"], json!({ "delivered": 1 }));
        assert!(
            stored.row.get("query").is_none(),
            "a query that counts no rows is shared whole"
        );
        assert_eq!(
            stored.row["other_details"],
            json!({ "items": [
                { "archetype_node_id": "id22", "v": 5 },
                { "archetype_node_id": "id28", "v": 6 },
                { "archetype_node_id": "id28", "v": 7 }
            ] })
        );
        assert_eq!(join(stored.row, stored.shared), entry);
    }

    /// The shared id-codes are the codes of the caller's details and of what
    /// the classification read.
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
            Detail::CategoryContent,
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
        let stored = split(members.clone());
        assert_eq!(stored.shared, None);
        assert_eq!(join(stored.row, None), tombstone);
    }
}
