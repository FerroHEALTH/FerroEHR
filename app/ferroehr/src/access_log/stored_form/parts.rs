// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A record's canonical form and its row and shared parts, with no JSON value
//! of the record built (#5849).
//!
//! The record is written once, by the canonical serializer of `openehr-rm`,
//! with the members `skip` names left out. Its parts are then cut from that
//! text: RFC 8785 sorts every object's members by name (§3.2.3), so the
//! members a part keeps, taken in the order of the whole form, are already in
//! the order the part's own canonical form puts them, and each part is the
//! whole form's member texts joined anew. The members are routed as the
//! split of the record's JSON value routes them, the reference the tests hold
//! the parts to. How a record is stored is our own design; no openEHR spec
//! governs it.

use std::cmp::Ordering;

use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::canonical_object_without;
use serde::Serialize;

use super::{DETAILS, ITEMS, NODE, SHARED, SHARED_DETAILS, SPLIT};

/// A record's canonical form and its stored parts, each in its canonical
/// form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parts {
    /// The whole record.
    pub whole: String,
    /// The row part.
    pub row: String,
    /// The shared part; `None` for a record that keeps none, such as a
    /// tombstone.
    pub shared: Option<String>,
}

/// Returns the canonical form of `record`, its top-level members named in
/// `skip` left out, and the stored parts cut from it.
///
/// # Errors
/// A [`serde_json::Error`] when `record` does not serialise to a canonical
/// form or is no object.
pub fn parts<T: Serialize + ?Sized>(record: &T, skip: &[&str]) -> Result<Parts, serde_json::Error> {
    let whole = canonical_object_without(record, skip)?;
    let (row, shared) = Cut::of(&whole)?.parts()?;
    Ok(Parts { whole, row, shared })
}

/// A record's canonical form read into its top-level members, which its
/// parts and its derived columns are cut from.
#[derive(Debug)]
pub struct Cut<'w> {
    /// Each top-level member's quoted name and value text, in order.
    top: Vec<(&'w str, &'w str)>,
}

impl<'w> Cut<'w> {
    /// Reads the canonical form `whole` of a record.
    ///
    /// # Errors
    /// A [`serde_json::Error`] when `whole` is no canonical object.
    pub fn of(whole: &'w str) -> Result<Self, serde_json::Error> {
        Ok(Self {
            top: members(whole).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?,
        })
    }

    /// The row part and the shared part, each in its canonical form.
    ///
    /// # Errors
    /// A [`serde_json::Error`] when the access details do not read back.
    pub fn parts(&self) -> Result<(String, Option<String>), serde_json::Error> {
        let mut row = Object::default();
        let mut shared = Object::default();
        for &(name, value) in &self.top {
            if SHARED.iter().any(|kept| is_name(name, kept)) {
                shared.push(name, value);
            } else if let Some((_, own_keys)) = SPLIT.iter().find(|(split, _)| is_name(name, split))
                && let Some(inner) = members(value)
            {
                let mut own = Object::default();
                let mut common = Object::default();
                for (key, member) in inner {
                    if own_keys.iter().any(|own_key| is_name(key, own_key)) {
                        own.push(key, member);
                    } else {
                        common.push(key, member);
                    }
                }
                if !own.is_empty() {
                    row.push(name, &own.finish());
                }
                shared.push(name, &common.finish());
            } else if is_name(name, DETAILS)
                && let Some(tree) = members(value)
            {
                let (envelope, kept) =
                    details(&tree).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?;
                shared.push(name, &envelope);
                row.push(name, &kept);
            } else {
                row.push(name, value);
            }
        }
        let shared = (!shared.is_empty()).then(|| shared.finish());
        Ok((row.finish(), shared))
    }

    /// Returns the string at the member path `path`, as a JSON pointer of
    /// member names reads the record's JSON value: `None` when a member is
    /// absent or the value there is no string.
    ///
    /// # Errors
    /// A [`serde_json::Error`] when the string there does not read back.
    pub fn text_at(&self, path: &[&str]) -> Result<Option<String>, serde_json::Error> {
        let Some((first, rest)) = path.split_first() else {
            return Ok(None);
        };
        let Some(mut value) = last_named(&self.top, first) else {
            return Ok(None);
        };
        for step in rest {
            // NOTE: a step into a value that is no object, or to an absent
            // member, finds no text, as a JSON pointer reads it.
            let Some(found) = members(value).and_then(|members| last_named(&members, step)) else {
                return Ok(None);
            };
            value = found;
        }
        if value.starts_with('"') {
            serde_json::from_str(value).map(Some)
        } else {
            Ok(None)
        }
    }
}

/// The value of the last member of `members` named `plain`.
fn last_named<'t>(members: &[(&'t str, &'t str)], plain: &str) -> Option<&'t str> {
    members
        .iter()
        .rev()
        .find(|(name, _)| is_name(name, plain))
        .map(|(_, value)| *value)
}

/// Why a record has no parts: it is no object, or its canonical form does
/// not read back.
const NO_OBJECT: &str = "a stored record's canonical form is no object";

/// The details tree `tree` cut in two: the envelope with the shared elements,
/// and the row's tree of the other elements, each in its canonical form.
fn details(tree: &[(&str, &str)]) -> Option<(String, String)> {
    let mut own = Vec::new();
    let mut common = Vec::new();
    let items = tree
        .iter()
        .find(|(name, _)| is_name(name, ITEMS))
        .and_then(|(_, value)| elements(value));
    for element in items.unwrap_or_default() {
        if shared_detail(element) {
            common.push(element);
        } else {
            own.push(element);
        }
    }
    let common = array(&common);
    let mut envelope = Object::default();
    let mut placed = false;
    for (name, value) in tree {
        if is_name(name, ITEMS) {
            continue;
        }
        if !placed && name_order(name, ITEMS)? == Ordering::Greater {
            envelope.push_named(ITEMS, &common);
            placed = true;
        }
        envelope.push(name, value);
    }
    if !placed {
        envelope.push_named(ITEMS, &common);
    }
    let mut kept = Object::default();
    kept.push_named(ITEMS, &array(&own));
    Some((envelope.finish(), kept.finish()))
}

/// Whether the canonical element `element` is a shared access detail: an
/// object whose at-code is one the shared part keeps.
fn shared_detail(element: &str) -> bool {
    members(element).is_some_and(|members| {
        members.iter().any(|(name, value)| {
            is_name(name, NODE)
                && SHARED_DETAILS.iter().any(|code| {
                    value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) == Some(*code)
                })
        })
    })
}

/// Whether the canonical name `name`, quotes included, is `plain`, a name
/// with nothing to escape.
fn is_name(name: &str, plain: &str) -> bool {
    name.strip_prefix('"')
        .and_then(|name| name.strip_suffix('"'))
        == Some(plain)
}

/// The order of the canonical name `name` against `plain` by the UTF-16 code
/// units of the names they write (RFC 8785 §3.2.3); `None` when `name` is no
/// JSON string.
fn name_order(name: &str, plain: &str) -> Option<Ordering> {
    let decoded: String = serde_json::from_str(name).ok()?;
    Some(decoded.encode_utf16().cmp(plain.encode_utf16()))
}

/// The canonical array of the canonical forms `elements`.
fn array(elements: &[&str]) -> String {
    let mut out = String::from("[");
    for (index, element) in elements.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(element);
    }
    out.push(']');
    out
}

/// A canonical object written member by member, in the order of a canonical
/// form it is cut from.
#[derive(Debug, Default)]
struct Object {
    out: String,
}

impl Object {
    /// Adds the member whose canonical name, quotes included, is `name`.
    fn push(&mut self, name: &str, value: &str) {
        self.out.push(if self.out.is_empty() { '{' } else { ',' });
        self.out.push_str(name);
        self.out.push(':');
        self.out.push_str(value);
    }

    /// Adds the member `plain`, a name with nothing to escape.
    fn push_named(&mut self, plain: &str, value: &str) {
        self.out.push(if self.out.is_empty() { '{' } else { ',' });
        self.out.push('"');
        self.out.push_str(plain);
        self.out.push_str("\":");
        self.out.push_str(value);
    }

    /// Whether no member was added.
    fn is_empty(&self) -> bool {
        self.out.is_empty()
    }

    /// The object's canonical form.
    fn finish(mut self) -> String {
        if self.out.is_empty() {
            self.out.push('{');
        }
        self.out.push('}');
        self.out
    }
}

/// The members of the canonical object `text`, each as its quoted name and
/// its value's text; `None` when `text` is no object.
fn members(text: &str) -> Option<Vec<(&str, &str)>> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut members = Vec::new();
    let mut at = 1;
    if bytes.get(at) == Some(&b'}') {
        return (at + 1 == bytes.len()).then_some(members);
    }
    loop {
        let name_end = string_end(bytes, at)?;
        let name = text.get(at..name_end)?;
        if bytes.get(name_end) != Some(&b':') {
            return None;
        }
        let value_start = name_end + 1;
        let value_end = value_end(bytes, value_start)?;
        members.push((name, text.get(value_start..value_end)?));
        match bytes.get(value_end) {
            Some(b',') => at = value_end + 1,
            Some(b'}') if value_end + 1 == bytes.len() => return Some(members),
            _ => return None,
        }
    }
}

/// The elements of the canonical array `text`; `None` when `text` is no
/// array.
fn elements(text: &str) -> Option<Vec<&str>> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }
    let mut elements = Vec::new();
    let mut at = 1;
    if bytes.get(at) == Some(&b']') {
        return (at + 1 == bytes.len()).then_some(elements);
    }
    loop {
        let end = value_end(bytes, at)?;
        elements.push(text.get(at..end)?);
        match bytes.get(end) {
            Some(b',') => at = end + 1,
            Some(b']') if end + 1 == bytes.len() => return Some(elements),
            _ => return None,
        }
    }
}

/// The index just past the string that starts at `at`.
fn string_end(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&b'"') {
        return None;
    }
    let mut index = at + 1;
    loop {
        match bytes.get(index)? {
            b'"' => return Some(index + 1),
            b'\\' => index += 2,
            _ => index += 1,
        }
    }
}

/// The index just past the value that starts at `at`.
fn value_end(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes.get(at)? {
        b'"' => string_end(bytes, at),
        b'{' | b'[' => {
            let mut depth = 0_usize;
            let mut index = at;
            loop {
                match bytes.get(index)? {
                    b'"' => {
                        index = string_end(bytes, index)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(index + 1);
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
        }
        _ => {
            let mut index = at;
            while let Some(byte) = bytes.get(index) {
                if matches!(byte, b',' | b'}' | b']') {
                    break;
                }
                index += 1;
            }
            Some(index)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
