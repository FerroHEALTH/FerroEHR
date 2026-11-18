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

use std::borrow::Cow;
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

    /// Whether the record has the top-level member `name`.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        last_named(&self.top, name).is_some()
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

/// The row part `row` of an entry without the members its columns give back.
///
/// `_type` always goes, and `uid`, `ehr_id` and `time_recorded` when the
/// column gives back their exact text ([`super::column_uuid`],
/// [`super::column_instant`]); `audit.access_log_document` puts them back for
/// a row part without `_type` (#5784).
///
/// # Errors
/// A [`serde_json::Error`] when `row` is no canonical object.
pub fn without_columns(row: &str) -> Result<String, serde_json::Error> {
    let members = members(row).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?;
    let mut kept = Object::default();
    for (name, value) in members {
        if !column_member(name, value) {
            kept.push(name, value);
        }
    }
    Ok(kept.finish())
}

/// Whether the top-level member `name` with the canonical value `value` is
/// one a column of the record gives back exactly.
fn column_member(name: &str, value: &str) -> bool {
    if is_name(name, "_type") {
        return true;
    }
    if is_name(name, "uid") || is_name(name, "ehr_id") {
        return typed_value(value, "HIER_OBJECT_ID")
            .is_some_and(|id| super::column_uuid(&id).is_some());
    }
    if is_name(name, "time_recorded") {
        return typed_value(value, "DV_DATE_TIME").is_some_and(|at| super::column_instant(&at));
    }
    false
}

/// The `value` of the canonical object `text` when it holds that string and
/// a `_type` of `kind` and nothing else.
fn typed_value(text: &str, kind: &str) -> Option<String> {
    let object: serde_json::Map<String, serde_json::Value> = serde_json::from_str(text).ok()?;
    if object.len() != 2 || object.get("_type")?.as_str()? != kind {
        return None;
    }
    object.get("value")?.as_str().map(str::to_owned)
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

/// The columns the log derives from a record's content, as the text at the
/// paths the record's ITS-JSON form holds them, which the database casts.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Derived {
    /// `_type`.
    pub record_type: Option<String>,
    /// `uid/value`.
    pub uid: Option<String>,
    /// `time_recorded/value`.
    pub time_recorded: Option<String>,
    /// `ehr_id/value`.
    pub ehr_id: Option<String>,
    /// `action/defining_code/terminology_id/value`.
    pub action_terminology: Option<String>,
    /// `action/defining_code/code_string`.
    pub action_code: Option<String>,
    /// `outcome/result/defining_code/terminology_id/value`.
    pub result_terminology: Option<String>,
    /// `outcome/result/defining_code/code_string`.
    pub result_code: Option<String>,
    /// `accessor/party/external_ref/id/value`.
    pub party_id: Option<String>,
    /// `retention/retain_until/value`.
    pub retain_until: Option<String>,
    /// `time_removed/value`, a tombstone's.
    pub time_removed: Option<String>,
}

impl Derived {
    /// The derived columns of the record whose canonical form `cut` read.
    ///
    /// # Errors
    /// A [`serde_json::Error`] when a string there does not read back.
    pub fn of(cut: &Cut<'_>) -> Result<Self, serde_json::Error> {
        Ok(Self {
            record_type: cut.text_at(&["_type"])?,
            uid: cut.text_at(&["uid", "value"])?,
            time_recorded: cut.text_at(&["time_recorded", "value"])?,
            ehr_id: cut.text_at(&["ehr_id", "value"])?,
            action_terminology: cut.text_at(&[
                "action",
                "defining_code",
                "terminology_id",
                "value",
            ])?,
            action_code: cut.text_at(&["action", "defining_code", "code_string"])?,
            result_terminology: cut.text_at(&[
                "outcome",
                "result",
                "defining_code",
                "terminology_id",
                "value",
            ])?,
            result_code: cut.text_at(&["outcome", "result", "defining_code", "code_string"])?,
            party_id: cut.text_at(&["accessor", "party", "external_ref", "id", "value"])?,
            retain_until: cut.text_at(&["retention", "retain_until", "value"])?,
            time_removed: cut.text_at(&["time_removed", "value"])?,
        })
    }
}

/// The columns of a stored row that give back the members its row part
/// leaves out ([`without_columns`]).
#[derive(Debug, Clone, Copy)]
pub struct RowColumns<'c> {
    /// The record's `_type`.
    pub record_type: &'c str,
    /// The record's uid.
    pub uid: uuid::Uuid,
    /// The record's `time_recorded`.
    pub time_recorded: jiff::Timestamp,
    /// The record's `ehr_id`, when the column keeps it.
    pub ehr_id: Option<uuid::Uuid>,
}

/// A member being assembled: its canonical name, quotes included, and its
/// canonical value.
type Member<'t> = (Cow<'t, str>, Cow<'t, str>);

/// The canonical form of the record whose stored row part is `row`, whose
/// shared part is `shared` and whose columns are `columns`: the exact text
/// the writer cut the parts from, the input of its content digest.
///
/// The members the columns hold go first, unless the row part keeps a
/// `_type`, and a member the row part keeps itself replaces one of theirs.
/// The shared part's members join the row's, `outcome` and `query` merge
/// their members, and the access details merge their envelope and their
/// elements in the order of their at-codes (byte order, an element without
/// one first), the shared elements before the row's under one code.
///
/// # Errors
/// A [`serde_json::Error`] when a part is no canonical object, which a part
/// changed after it was written may no longer be.
pub fn whole(
    row: &str,
    shared: Option<&str>,
    columns: &RowColumns<'_>,
) -> Result<String, serde_json::Error> {
    let own = members(row).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?;
    let mut top: Vec<Member<'_>> = Vec::with_capacity(own.len() + 16);
    if !own.iter().any(|(name, _)| is_name(name, "_type")) {
        top.push((
            "\"_type\"".into(),
            serde_json::to_string(columns.record_type)?.into(),
        ));
        top.push((
            "\"uid\"".into(),
            typed_text(HIER_OBJECT_ID, columns.uid).into(),
        ));
        if let Some(at) = super::instant_text(columns.time_recorded) {
            top.push((
                "\"time_recorded\"".into(),
                typed_text("DV_DATE_TIME", at).into(),
            ));
        }
        if let Some(ehr_id) = columns.ehr_id {
            top.push((
                "\"ehr_id\"".into(),
                typed_text(HIER_OBJECT_ID, ehr_id).into(),
            ));
        }
    }
    let Some(shared) = shared else {
        for &(name, value) in &own {
            put(&mut top, name.into(), value.into());
        }
        return Ok(object_text(top));
    };
    let common = members(shared).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?;
    let merged =
        |name: &str| is_name(name, DETAILS) || SPLIT.iter().any(|(split, _)| is_name(name, split));
    for &(name, value) in common.iter().chain(own.iter()) {
        if !merged(name) {
            put(&mut top, name.into(), value.into());
        }
    }
    if let Some(details) = merged_details(last_named(&common, DETAILS), last_named(&own, DETAILS))?
    {
        put(&mut top, format!("\"{DETAILS}\"").into(), details.into());
    }
    for (split, _) in SPLIT {
        let sides = [last_named(&common, split), last_named(&own, split)];
        if sides.iter().all(Option::is_none) {
            continue;
        }
        let mut inner = Vec::new();
        for side in sides.into_iter().flatten() {
            for (name, value) in
                members(side).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?
            {
                put(&mut inner, name.into(), value.into());
            }
        }
        put(
            &mut top,
            format!("\"{split}\"").into(),
            object_text(inner).into(),
        );
    }
    Ok(object_text(top))
}

/// The `_type` of an id the columns keep.
const HIER_OBJECT_ID: &str = "HIER_OBJECT_ID";

/// The canonical text of a `{"_type": kind, "value": value}` object, whose
/// value needs no escape.
fn typed_text(kind: &str, value: impl std::fmt::Display) -> String {
    format!("{{\"_type\":\"{kind}\",\"value\":\"{value}\"}}")
}

/// Sets the member `name` of `members` to `value`, replacing one of that
/// name, as `jsonb` concatenation does.
fn put<'t>(members: &mut Vec<Member<'t>>, name: Cow<'t, str>, value: Cow<'t, str>) {
    match members.iter_mut().find(|(have, _)| *have == name) {
        Some(member) => member.1 = value,
        None => members.push((name, value)),
    }
}

/// The canonical object of `members`, sorted by name (RFC 8785 §3.2.3).
fn object_text(mut members: Vec<Member<'_>>) -> String {
    members.sort_by(|(left, _), (right, _)| canonical_order(left, right));
    let mut out = Object::default();
    for (name, value) in &members {
        out.push(name, value);
    }
    out.finish()
}

/// The order of two canonical names by the UTF-16 code units of the names
/// they write (RFC 8785 §3.2.3); for names in ASCII without an escape that is
/// the order of their bytes.
fn canonical_order(left: &str, right: &str) -> Ordering {
    let plain = |name: &str| name.is_ascii() && !name.contains('\\');
    if plain(left) && plain(right) {
        return left.cmp(right);
    }
    // NOTE: RFC 8785 §3.2.3, a canonical name always decodes; one that does not sorts as empty.
    let decode = |name: &str| serde_json::from_str::<String>(name).unwrap_or_default();
    decode(left)
        .encode_utf16()
        .cmp(decode(right).encode_utf16())
}

/// The access details of the shared part, `common`, and of the row, `own`,
/// merged: the envelope's members, the row's replacing the shared ones, and
/// the elements of both in the order of their at-codes; `None` when neither
/// part has them.
fn merged_details(
    common: Option<&str>,
    own: Option<&str>,
) -> Result<Option<String>, serde_json::Error> {
    if common.is_none() && own.is_none() {
        return Ok(None);
    }
    let mut envelope = Vec::new();
    let mut items: Vec<(Option<Cow<'_, str>>, usize, usize, &str)> = Vec::new();
    for (side, tree) in [common, own].into_iter().enumerate() {
        let Some(tree) = tree else {
            continue;
        };
        for (name, value) in members(tree).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))? {
            if is_name(name, ITEMS) {
                let listed = elements(value).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?;
                for (position, element) in listed.into_iter().enumerate() {
                    items.push((element_code(element), side, position, element));
                }
            } else {
                put(&mut envelope, name.into(), value.into());
            }
        }
    }
    items.sort_by(|(left, ls, lp, _), (right, rs, rp, _)| {
        left.cmp(right).then(ls.cmp(rs)).then(lp.cmp(rp))
    });
    let listed: Vec<&str> = items.iter().map(|(_, _, _, element)| *element).collect();
    put(
        &mut envelope,
        format!("\"{ITEMS}\"").into(),
        array(&listed).into(),
    );
    Ok(Some(object_text(envelope)))
}

/// The at-code an element names as `->> 'archetype_node_id'` reads it: a
/// string decoded, any other value but `null` as its text; `None` for no
/// code, which sorts first.
fn element_code(element: &str) -> Option<Cow<'_, str>> {
    let members = members(element)?;
    let code = last_named(&members, NODE)?;
    match code.as_bytes().first() {
        Some(b'"') if !code.contains('\\') => code.get(1..code.len() - 1).map(Cow::Borrowed),
        Some(b'"') => serde_json::from_str::<String>(code).ok().map(Cow::Owned),
        _ if code == "null" => None,
        _ => Some(Cow::Borrowed(code)),
    }
}

#[cfg(test)]
pub(crate) mod tests;
