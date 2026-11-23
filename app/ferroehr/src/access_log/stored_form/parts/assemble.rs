// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The renderer of stored form [`super::facts::FORM`] (#5703, #6005, #6322).
//!
//! It joins a stored record's canonical form back from its row part, its
//! shared part and its row's columns. How a record is stored is our own
//! design; no openEHR spec governs it.

use std::borrow::Cow;
use std::cmp::Ordering;

use super::{
    ACCESSED, DETAILS, ITEMS, NO_OBJECT, NODE, Object, SPLIT, array, elements, index, is_name,
    last_named, members,
};

/// The columns of a stored row that give back the members its row part
/// leaves out ([`index::without_columns`]).
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
    /// The index columns that give back members of the row part.
    pub index: index::IndexColumns<'c>,
}

/// A member being assembled: its canonical name, quotes included, and its
/// canonical value.
pub(super) type Member<'t> = (Cow<'t, str>, Cow<'t, str>);

/// The canonical form of the record whose stored row part is `row`, whose
/// shared part is `shared` and whose columns are `columns`: the exact text
/// the writer cut the parts from, the input of its content digest.
///
/// The members the columns hold go first, unless the row part keeps a
/// `_type`, and a member the row part keeps itself replaces one of theirs.
/// The shared part's members join the row's, `outcome` and `query` merge
/// their members, and the access details merge their envelope and their
/// elements in the order of their id-codes (byte order, an element without
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
    let mut own = members(row).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?;
    let mut top: Vec<Member<'_>> = Vec::with_capacity(own.len() + 16);
    let mut restoring = Vec::new();
    if !own.iter().any(|(name, _)| is_name(name, "_type")) {
        for (name, value) in &own {
            if let Cow::Owned(text) = index::restored(name, value, &columns.index)? {
                restoring.push((*name, text));
            }
        }
        top.push((
            "\"_type\"".into(),
            serde_json::to_string(columns.record_type)?.into(),
        ));
        top.push((
            "\"uid\"".into(),
            typed_text(HIER_OBJECT_ID, columns.uid).into(),
        ));
        if let Some(at) = super::super::instant_text(columns.time_recorded) {
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
    for (name, text) in &restoring {
        if let Some(member) = own.iter_mut().find(|(have, _)| have == name) {
            member.1 = text.as_str();
        }
    }
    let Some(shared) = shared else {
        for &(name, value) in &own {
            put(&mut top, name.into(), value.into());
        }
        return Ok(object_text(top));
    };
    let common = members(shared).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?;
    let merged = |name: &str| {
        is_name(name, DETAILS)
            || is_name(name, ACCESSED)
            || SPLIT.iter().any(|(split, _)| is_name(name, split))
    };
    for &(name, value) in common.iter().chain(own.iter()) {
        if !merged(name) {
            put(&mut top, name.into(), value.into());
        }
    }
    if let Some(details) = merged_details(last_named(&common, DETAILS), last_named(&own, DETAILS))?
    {
        put(&mut top, format!("\"{DETAILS}\"").into(), details.into());
    }
    if let Some(items) = merged_items(last_named(&common, ACCESSED), last_named(&own, ACCESSED))? {
        put(&mut top, format!("\"{ACCESSED}\"").into(), items.into());
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
pub(super) fn object_text(mut members: Vec<Member<'_>>) -> String {
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
/// the elements of both in the order of their id-codes; `None` when neither
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

/// The id-code an element names as `->> 'archetype_node_id'` reads it: a
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

/// The accessed items of the row part `own` with the shared part's item
/// `common` merged into its only item; `None` when neither part holds any.
///
/// # Errors
/// A [`serde_json::Error`] when a side is not one item.
fn merged_items(
    common: Option<&str>,
    own: Option<&str>,
) -> Result<Option<String>, serde_json::Error> {
    let (common, own) = match (common, own) {
        (None, None) => return Ok(None),
        (None, Some(only)) | (Some(only), None) => return Ok(Some(only.to_owned())),
        (Some(common), Some(own)) => (common, own),
    };
    let only = |items| match elements(items).as_deref() {
        Some([item]) => members(item).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT)),
        _ => Err(serde::ser::Error::custom(
            "a shared item joins an only item",
        )),
    };
    let mut item: Vec<Member<'_>> = Vec::with_capacity(12);
    for (name, value) in only(common)?.into_iter().chain(only(own)?) {
        put(&mut item, name.into(), value.into());
    }
    Ok(Some(array(&[&object_text(item)])))
}
