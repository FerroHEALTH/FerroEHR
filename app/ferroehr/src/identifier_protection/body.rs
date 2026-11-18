// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Finding protected identifiers in a party body, and replacing them with a
//! reference to the row that holds the sealed value.
//!
//! **No openEHR spec governs this — our own design/extension.** A party's
//! identifiers live in `PARTY_IDENTITY.details` as `DV_IDENTIFIER` items (RM
//! demographic `UML/classes/org.openehr.rm.demographic.party_identity.adoc`;
//! RM `data_types` `UML/classes/org.openehr.rm.data_types.dv_identifier.adoc`),
//! and the RM types the value as a plain `String` with no protected form. The
//! substitution here is a storage decision layered over that.
//!
//! The scan is by `DV_IDENTIFIER.type`, the attribute the RM gives for "the
//! identifier type, such as prescription, or Social Security Number", matched
//! against the deployment's configured scheme list. An identifier of an
//! unconfigured type is left exactly as written: this module protects what the
//! deployment declared and never guesses.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): service-surface signatures over stored canonical \
              fragments and dynamic shapes"
)]

use openehr_rm::v1_2::access::{AccessError, RmObject, RmValue};
use serde_json::Value;

use crate::versioning::tree::{Shape, Tree};

/// The `urn` prefix a substituted identifier carries in the stored body.
///
/// A reference rather than the value, and self-describing rather than an
/// opaque uuid, so a person reading a stored body sees immediately that the
/// value is held elsewhere rather than missing.
pub const REFERENCE_PREFIX: &str = "urn:ferroehr:protected-identifier:";

/// One identifier found in a body: its scheme, its value, and where it sat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundIdentifier {
    /// The `DV_IDENTIFIER.type`, matched against the configured schemes.
    pub scheme: String,
    /// The identifier value as written.
    pub value: String,
    /// The RFC 6901 JSON pointer of the `DV_IDENTIFIER` object holding it, so
    /// the substitution and any later expansion address the same node.
    pub pointer: String,
}

/// Every protected-scheme identifier in `body`, in document order.
///
/// Walks the whole body rather than only `PARTY_IDENTITY.details`: the RM
/// allows a `DV_IDENTIFIER` anywhere an `ITEM` can appear, and an identifier
/// that reached an unexpected slot is exactly the one a protection rule must
/// not miss.
#[must_use]
pub(crate) fn find<'a, T: Tree<'a>>(body: T, schemes: &[String]) -> Vec<FoundIdentifier> {
    let mut found = Vec::new();
    walk(body, &mut String::new(), schemes, &mut found);
    found
}

/// Replace each found identifier's `id` with a reference to its stored row.
///
/// `references` pairs each [`FoundIdentifier::pointer`] with the row id that
/// now holds the sealed value. A pointer with no pairing is left untouched,
/// which cannot happen on the write path and would be a silent value drop if
/// it did.
pub(crate) fn substitute(body: &mut Value, references: &[(String, uuid::Uuid)]) {
    for (pointer, row) in references {
        if let Some(Value::String(id)) = body.pointer_mut(&format!("{pointer}/id")) {
            *id = format!("{REFERENCE_PREFIX}{row}");
        }
    }
}

/// [`substitute`] over a typed RM body: each `DV_IDENTIFIER` a pointer names
/// gets the reference as its `id`.
///
/// # Errors
/// [`AccessError`] when a pointer names no `DV_IDENTIFIER` of the body,
/// unreachable for a pointer [`find`] read off the body's own rendering.
pub(crate) fn substitute_typed(
    body: &mut dyn RmObject,
    references: &[(String, uuid::Uuid)],
) -> Result<(), AccessError> {
    for (pointer, row) in references {
        let target = at_pointer(body, pointer).ok_or_else(|| AccessError::UndeclaredAttribute {
            rm_type: "DV_IDENTIFIER",
            attribute: pointer.clone(),
        })?;
        target.set("id", RmValue::String(format!("{REFERENCE_PREFIX}{row}")))?;
    }
    Ok(())
}

/// The RM object an RFC 6901 pointer of attribute names and container
/// indices names in `root`.
fn at_pointer<'b>(root: &'b mut dyn RmObject, pointer: &str) -> Option<&'b mut dyn RmObject> {
    let mut steps = pointer.split('/').skip(1).peekable();
    let mut at = root;
    while let Some(step) = steps.next() {
        let attribute = step.replace("~1", "/").replace("~0", "~");
        let index = steps.peek().and_then(|next| next.parse::<usize>().ok());
        at = match index {
            Some(index) if at.items(&attribute).is_some() => {
                steps.next();
                at.items_mut(&attribute)?.nth(index)?
            }
            _ => at.object_mut(&attribute)?,
        };
    }
    Some(at)
}

/// Put the plaintext values back, addressed by the same pointers.
///
/// The inverse of `substitute`, for the audited expansion path: the served
/// default is the stored reference form, and a caller entitled to the values
/// asks for them explicitly.
pub fn expand(body: &mut Value, values: &[(String, String)]) {
    for (pointer, value) in values {
        if let Some(Value::String(id)) = body.pointer_mut(&format!("{pointer}/id")) {
            id.clone_from(value);
        }
    }
}

/// The row id a reference names, when this `id` string is one.
#[must_use]
pub fn referenced_row(id: &str) -> Option<uuid::Uuid> {
    id.strip_prefix(REFERENCE_PREFIX)
        .and_then(|rest| rest.parse().ok())
}

/// Recursive walk collecting `DV_IDENTIFIER` nodes of a configured scheme.
fn walk<'a, T: Tree<'a>>(
    node: T,
    pointer: &mut String,
    schemes: &[String],
    found: &mut Vec<FoundIdentifier>,
) {
    each_object(node, pointer, &mut |node, pointer| {
        if node.get_text("_type").as_deref() == Some("DV_IDENTIFIER")
            && let Some(scheme) = node.get_text("type")
            && schemes.iter().any(|configured| *configured == scheme)
            && let Some(value) = node.get_text("id")
            // A body being re-committed unchanged already carries the
            // reference; substituting a reference for a reference would
            // orphan the first row.
            && referenced_row(&value).is_none()
        {
            found.push(FoundIdentifier {
                scheme: scheme.into_owned(),
                value: value.into_owned(),
                pointer: pointer.to_owned(),
            });
        }
    });
}

/// Visits every object of `node`, itself first, in document order, with the
/// RFC 6901 pointer that names it below the one `pointer` holds.
pub(crate) fn each_object<'a, T: Tree<'a>>(
    node: T,
    pointer: &mut String,
    visit: &mut impl FnMut(T, &str),
) {
    match node.shape() {
        Shape::Object => {
            visit(node, pointer);
            for (key, child) in node.members() {
                let mark = pointer.len();
                pointer.push('/');
                pointer.push_str(&escape(key));
                each_object(child, pointer, visit);
                pointer.truncate(mark);
            }
        }
        Shape::Array => {
            for (index, child) in node.items().enumerate() {
                let mark = pointer.len();
                pointer.push('/');
                pointer.push_str(&index.to_string());
                each_object(child, pointer, visit);
                pointer.truncate(mark);
            }
        }
        Shape::String | Shape::Other => {}
    }
}

/// RFC 6901 token escaping: `~` becomes `~0` and `/` becomes `~1`.
fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    //! Every identifier here is synthetic.

    use super::{FoundIdentifier, REFERENCE_PREFIX, expand, find, referenced_row, substitute};
    use serde_json::{Value, json};

    fn schemes() -> Vec<String> {
        vec!["nl-bsn".to_owned()]
    }

    fn party() -> Value {
        json!({
            "_type": "PERSON",
            "name": { "_type": "DV_TEXT", "value": "a party" },
            "identities": [{
                "_type": "PARTY_IDENTITY",
                "name": { "_type": "DV_TEXT", "value": "identifiers" },
                "details": { "_type": "ITEM_LIST", "items": [
                    { "_type": "ELEMENT", "value": {
                        "_type": "DV_IDENTIFIER", "type": "nl-bsn",
                                                "id": "111222333", "issuer": "RvIG", "assigner": "RvIG" } }, // privacy-allow: synthetic
                    { "_type": "ELEMENT", "value": {
                        "_type": "DV_IDENTIFIER", "type": "local-mrn",
                        "id": "MRN-004221", "issuer": "the hospital", "assigner": "the hospital" } }
                ]}
            }]
        })
    }

    #[test]
    fn a_configured_scheme_is_found_and_an_unconfigured_one_is_not() {
        let found = find(&party(), &schemes());
        assert_eq!(
            found,
            vec![FoundIdentifier {
                scheme: "nl-bsn".to_owned(),
                value: "111222333".to_owned(), // privacy-allow: synthetic
                pointer: "/identities/0/details/items/0/value".to_owned(),
            }],
            "only the configured scheme is protected; the local MRN is left alone"
        );
    }

    #[test]
    fn substitution_replaces_the_value_and_leaves_everything_else() {
        let mut body = party();
        let found = find(&body, &schemes());
        let row = uuid::Uuid::from_u128(42);
        substitute(&mut body, &[(found[0].pointer.clone(), row)]);

        let stored = body
            .pointer("/identities/0/details/items/0/value/id")
            .and_then(Value::as_str)
            .expect("the identifier slot survives");
        assert_eq!(stored, format!("{REFERENCE_PREFIX}{row}"));
        assert_eq!(referenced_row(stored), Some(row));
        assert!(
            !body.to_string().contains("111222333"), // privacy-allow: synthetic, asserted GONE
            "the value must not survive anywhere in the stored body"
        );
        // The sibling identifier and the surrounding structure are untouched.
        assert_eq!(
            body.pointer("/identities/0/details/items/1/value/id")
                .and_then(Value::as_str),
            Some("MRN-004221")
        );
        assert_eq!(
            body.pointer("/identities/0/details/items/0/value/issuer")
                .and_then(Value::as_str),
            Some("RvIG"),
            "only the id is replaced"
        );
    }

    #[test]
    fn expansion_is_the_exact_inverse_of_substitution() {
        // The property the whole design rests on: what a reader expands is
        // byte-for-byte what the writer submitted.
        let original = party();
        let mut body = original.clone();
        let found = find(&body, &schemes());
        let row = uuid::Uuid::from_u128(42);
        substitute(&mut body, &[(found[0].pointer.clone(), row)]);
        assert_ne!(body, original, "substitution changed the body");

        expand(
            &mut body,
            &[(found[0].pointer.clone(), found[0].value.clone())],
        );
        assert_eq!(
            body, original,
            "expansion must restore the submitted body exactly"
        );
    }

    #[test]
    fn a_body_that_already_carries_a_reference_is_not_substituted_again() {
        // Re-committing an unchanged party must not orphan the first row by
        // sealing the reference text as though it were a value.
        let mut body = party();
        let row = uuid::Uuid::from_u128(42);
        let pointer = find(&body, &schemes())[0].pointer.clone();
        substitute(&mut body, &[(pointer, row)]);
        assert!(
            find(&body, &schemes()).is_empty(),
            "a stored reference is not a value to protect"
        );
    }

    #[test]
    fn an_identifier_outside_party_identity_is_found_too() {
        // The RM admits a DV_IDENTIFIER wherever an ITEM can appear, and one
        // that reached an unexpected slot is exactly the one to catch.
        let body = json!({
            "_type": "PERSON",
            "contacts": [{ "addresses": [{ "details": { "items": [
                { "_type": "ELEMENT", "value": {
                    "_type": "DV_IDENTIFIER", "type": "nl-bsn",
                                        "id": "111222333" } } // privacy-allow: synthetic
            ]}}]}]
        });
        let found = find(&body, &schemes());
        assert_eq!(found.len(), 1, "the walk covers the whole body");
        assert_eq!(
            found[0].pointer,
            "/contacts/0/addresses/0/details/items/0/value"
        );
    }

    #[test]
    fn a_key_with_a_pointer_metacharacter_still_addresses_its_own_node() {
        // RFC 6901 escaping: an object key containing `/` or `~` must not
        // shift which node the pointer names.
        let body = json!({
            "a/b": { "c~d": {
                                "_type": "DV_IDENTIFIER", "type": "nl-bsn", "id": "111222333" // privacy-allow: synthetic
            }}
        });
        let found = find(&body, &schemes());
        assert_eq!(found[0].pointer, "/a~1b/c~0d");
        let mut substituted = body.clone();
        substitute(
            &mut substituted,
            &[(found[0].pointer.clone(), uuid::Uuid::nil())],
        );
        assert_ne!(substituted, body, "the escaped pointer addressed the node");
    }
}
