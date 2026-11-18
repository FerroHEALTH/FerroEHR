// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The edits an applied PMIR message makes to a patient `PERSON` body: the
//! record subject elements it moves and the patient identifier elements it
//! removes.
//!
//! A record subject element is moved verbatim, so the survivor carries the
//! element the registering system wrote. An identifier is removed by its
//! issuer and the id the stored body carries. Identifiers are added through
//! the template-aware mapped write, never here. No openEHR spec governs these
//! edits: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored canonical party body an \
              applied identity change edits"
)]

use serde_json::Value;

use crate::party_binding::policy_mark::coded;
use crate::service::demographic::mapped_write::{held, step_parts};
use openehr_rm::draft::access_control::terminology::{RecordSubject, TERMINOLOGY_ID_ACCESS, Term};

/// A record subject reference: its namespace and its id.
pub(super) type SubjectRef = (String, String);

/// Whether `node` is an `ELEMENT` named by the record subject code.
fn is_record_subject(node: &Value) -> bool {
    node.get("name")
        .and_then(coded)
        .is_some_and(|code| RecordSubject::read(TERMINOLOGY_ID_ACCESS, &code).is_ok())
}

/// The reference the record subject element `node` names.
fn reference(node: &Value) -> Option<SubjectRef> {
    let value = node.get("value")?;
    Some((
        value.get("issuer")?.as_str()?.to_owned(),
        value.get("id")?.as_str()?.to_owned(),
    ))
}

/// The record subjects `body` declares in `details.items`, in order.
pub(super) fn record_subjects(body: &Value) -> Vec<SubjectRef> {
    body.pointer("/details/items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| is_record_subject(item))
                .filter_map(reference)
                .collect()
        })
        .unwrap_or_default()
}

/// Removes from `body` the record subject elements naming one of `subjects`
/// (the id compared without case, as the projection does) and returns them.
pub(super) fn take_record_subjects(body: &mut Value, subjects: &[SubjectRef]) -> Vec<Value> {
    let Some(Value::Array(items)) = body.pointer_mut("/details/items") else {
        return Vec::new();
    };
    let named = |item: &Value| {
        is_record_subject(item)
            && reference(item).is_some_and(|(namespace, id)| {
                subjects
                    .iter()
                    .any(|(n, i)| *n == namespace && i.eq_ignore_ascii_case(&id))
            })
    };
    let (taken, kept): (Vec<Value>, Vec<Value>) = items.drain(..).partition(|item| named(item));
    *items = kept;
    taken
}

/// Appends the record subject `elements` to the `details.items` of `body`,
/// skipping one whose reference it already declares.
pub(super) fn put_record_subjects(body: &mut Value, elements: Vec<Value>) {
    let held = record_subjects(body);
    // NOTE: the stored typed rendering leaves an empty `items` out, so the
    // list is opened again when the first element returns (#6003).
    let Some(Value::Array(items)) = body
        .pointer_mut("/details")
        .and_then(Value::as_object_mut)
        .map(|details| {
            details
                .entry("items")
                .or_insert_with(|| Value::Array(Vec::new()))
        })
    else {
        return;
    };
    for element in elements {
        let duplicate = reference(&element).is_some_and(|(namespace, id)| {
            held.iter()
                .any(|(n, i)| *n == namespace && i.eq_ignore_ascii_case(&id))
        });
        if !duplicate {
            items.push(element);
        }
    }
}

/// Removes from `body` each `ELEMENT` at the patient identifier `path` whose
/// value is a `DV_IDENTIFIER` of `issuer` carrying `id` as stored; whether
/// one was removed.
pub(super) fn remove_identifier(body: &mut Value, path: &str, issuer: &str, id: &str) -> bool {
    at_path(body, &steps(path), true, &mut |items, node_id| {
        let before = items.len();
        items.retain(|item| !(held(item, node_id) && carries(item.get("value"), issuer, id)));
        items.len() != before
    })
}

/// Appends to the first array the patient identifier `path` reaches in
/// `body` an `ELEMENT` shaped as the first one there, holding `value`;
/// whether one was appended. With no `ELEMENT` at the path to shape it,
/// nothing is appended and the caller writes it through the template.
pub(super) fn push_identifier(body: &mut Value, path: &str, value: &Value) -> bool {
    at_path(body, &steps(path), false, &mut |items, node_id| {
        let Some(mut element) = items.iter().find(|item| held(item, node_id)).cloned() else {
            return false;
        };
        let Some(slot) = element.get_mut("value") else {
            return false;
        };
        *slot = value.clone();
        items.push(element);
        true
    })
}

/// The steps of an `ELEMENT` path, without a trailing `/value`.
fn steps(path: &str) -> Vec<&str> {
    path.trim_end_matches("/value")
        .trim_start_matches('/')
        .split('/')
        .collect()
}

/// Calls `visit` with each array the last of `steps` names under `node` and
/// that step's node id, in every node the steps before it reach: every one
/// when `every`, else until one answers `true`; whether one did.
fn at_path(
    node: &mut Value,
    steps: &[&str],
    every: bool,
    visit: &mut dyn FnMut(&mut Vec<Value>, Option<&str>) -> bool,
) -> bool {
    let Some((step, rest)) = steps.split_first() else {
        return false;
    };
    let (attribute, node_id) = step_parts(step);
    match node.get_mut(attribute) {
        Some(Value::Array(items)) if rest.is_empty() => visit(items, node_id),
        Some(Value::Array(items)) => {
            let mut any = false;
            for item in items.iter_mut().filter(|item| held(item, node_id)) {
                any |= at_path(item, rest, every, visit);
                if any && !every {
                    break;
                }
            }
            any
        }
        Some(single) if !rest.is_empty() && held(single, node_id) => {
            at_path(single, rest, every, visit)
        }
        _ => false,
    }
}

/// Whether `value` is a `DV_IDENTIFIER` of `issuer` carrying `id`.
fn carries(value: Option<&Value>, issuer: &str, id: &str) -> bool {
    value.is_some_and(|value| {
        value.get("_type").and_then(Value::as_str) == Some("DV_IDENTIFIER")
            && value.get("issuer").and_then(Value::as_str) == Some(issuer)
            && value.get("id").and_then(Value::as_str) == Some(id)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn subject(namespace: &str, id: &str) -> Value {
        json!({ "_type": "ELEMENT", "archetype_node_id": "id36",
            "name": { "_type": "DV_CODED_TEXT", "value": "record_subject",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY_ID_ACCESS },
                    "code_string": "record_subject" } },
            "value": { "_type": "DV_IDENTIFIER", "issuer": namespace, "id": id, "type": "PI" } })
    }

    fn identifier(issuer: &str, id: &str) -> Value {
        json!({ "_type": "ELEMENT", "archetype_node_id": "id3",
            "name": { "_type": "DV_TEXT", "value": "Patient identifier" },
            "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": "NI" } })
    }

    fn person(items: &[Value]) -> Value {
        json!({ "_type": "PERSON", "details": { "_type": "ITEM_TREE", "items": items } })
    }

    #[test]
    fn record_subjects_move_verbatim_and_once() {
        let mut a = person(&[
            identifier("urn:oid:2.999.4", "A-1"),
            subject("urn:ns", "P-A"),
        ]);
        let mut b = person(&[subject("urn:ns", "P-B")]);
        let taken = take_record_subjects(&mut a, &[("urn:ns".to_owned(), "p-a".to_owned())]);
        assert_eq!(taken, vec![subject("urn:ns", "P-A")]);
        assert_eq!(record_subjects(&a), Vec::<SubjectRef>::new());
        put_record_subjects(&mut b, taken.clone());
        put_record_subjects(&mut b, taken);
        assert_eq!(
            record_subjects(&b),
            vec![
                ("urn:ns".to_owned(), "P-B".to_owned()),
                ("urn:ns".to_owned(), "P-A".to_owned())
            ]
        );
    }

    #[test]
    fn an_identifier_is_appended_in_the_shape_of_its_siblings() {
        let mut body = person(&[identifier("urn:oid:2.999.4", "A-1")]);
        body["details"]["archetype_node_id"] = json!("id2");
        let path = "/details[id2]/items[id3]";
        let value = json!({ "_type": "DV_IDENTIFIER", "issuer": "urn:oid:2.999.4", "id": "A-2",
            "type": "NI" });
        assert!(push_identifier(&mut body, path, &value));
        assert_eq!(
            body.pointer("/details/items/1"),
            Some(&identifier("urn:oid:2.999.4", "A-2"))
        );
        let mut bare = person(&[]);
        bare["details"]["archetype_node_id"] = json!("id2");
        assert!(
            !push_identifier(&mut bare, path, &value),
            "no sibling shapes it"
        );
    }

    #[test]
    fn an_identifier_is_removed_by_issuer_and_stored_id_at_its_path() {
        let mut body = person(&[
            identifier("urn:oid:2.999.4", "A-1"),
            identifier("urn:oid:2.999.4", "A-2"),
            subject("urn:oid:2.999.4", "A-1"),
        ]);
        let path = "/details[id2]/items[id3]";
        assert!(!remove_identifier(
            &mut body,
            path,
            "urn:oid:2.999.4",
            "A-1"
        ));
        body["details"]["archetype_node_id"] = json!("id2");
        assert!(remove_identifier(&mut body, path, "urn:oid:2.999.4", "A-1"));
        assert!(!remove_identifier(
            &mut body,
            path,
            "urn:oid:2.999.4",
            "A-1"
        ));
        let items = body
            .pointer("/details/items")
            .and_then(Value::as_array)
            .cloned();
        assert_eq!(
            items,
            Some(vec![
                identifier("urn:oid:2.999.4", "A-2"),
                subject("urn:oid:2.999.4", "A-1")
            ]),
            "the record subject element under another node id stays"
        );
    }
}
