// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The members of an entry's row part its index columns give back (#6005).
//!
//! The `request_id` column holds the request the entry's channel names, and
//! the `resource_id` column the version its only item names. The writer
//! leaves each out when the column writes its exact text, and names it
//! `null` where a reader would give back a member the entry does not have;
//! `audit.access_log_row_part` reads a row part as `restored` does. How a
//! record is stored is our own design; no openEHR spec governs it.

use std::borrow::Cow;
use std::cmp::Ordering;

use super::{
    ITEMS, Member, Members, NO_OBJECT, Object, array, column_member, is_name, last_named, member,
    members, name_order, object_text, value_end,
};

/// The index columns of an entry that give back a member of its row part:
/// the request its channel names and the version its one item names.
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexColumns<'c> {
    /// The `request_id` column, the ITI-81 index's request.
    pub request_id: Option<&'c str>,
    /// The `resource_id` column, the ITI-81 index's object.
    pub resource_id: Option<&'c str>,
}

/// The row part `row` of an entry without the members its columns give back.
///
/// `_type` always goes, and `uid`, `ehr_id` and `time_recorded` when the
/// column gives back their exact text ([`super::column_uuid`],
/// [`super::column_instant`]); `audit.access_log_document` puts them back for
/// a row part without `_type` (#5784). Of the members the index columns
/// `index` give back, `channel.request_id` and the `version_id` of an only
/// item without an `object` go when the column writes their exact text; where
/// a reader would give back a member the entry does not have, the row part
/// names it `null` instead (#6005).
///
/// # Errors
/// A [`serde_json::Error`] when `row` is no canonical object.
pub fn without_columns(row: &str, index: &IndexColumns<'_>) -> Result<String, serde_json::Error> {
    let mut members = Members::of(row).ok_or_else(|| serde::ser::Error::custom(NO_OBJECT))?;
    let mut kept = Object::with_capacity(row.len());
    for (name, value) in members.by_ref() {
        if column_member(name, value) {
            continue;
        }
        if is_name(name, CHANNEL) && Members::of(value).is_some() {
            kept.open(name);
            elided(
                &mut kept.out,
                value,
                REQUEST_ID,
                index.request_id,
                string_is,
            );
        } else if is_name(name, ITEMS)
            && let Some(item) = only_item(value)
        {
            let given = index
                .resource_id
                .filter(|_| member(item, "object").is_none());
            kept.open(name);
            kept.out.push('[');
            elided(&mut kept.out, item, VERSION_ID, given, version_is);
            kept.out.push(']');
        } else {
            kept.push(name, value);
        }
    }
    if !members.complete {
        return Err(serde::ser::Error::custom(NO_OBJECT));
    }
    Ok(kept.finish())
}

/// The member of an entry naming the request it arrived in.
const CHANNEL: &str = "channel";
/// The member of the channel the `request_id` column gives back.
const REQUEST_ID: &str = "request_id";
/// The member of an item the `resource_id` column gives back.
const VERSION_ID: &str = "version_id";
/// The member text a row part writes for one its reader would otherwise give
/// back.
const ABSENT: &str = "null";

/// Writes the canonical object `object` to `out` with its member `plain` left
/// out when `matches` says its value is the one a reader gives back from
/// `given`; with `plain` named `null` when it has no such member and a reader
/// would give one back.
fn elided(
    out: &mut String,
    object: &str,
    plain: &str,
    given: Option<&str>,
    matches: impl Fn(&str, &str) -> bool,
) {
    let mut placed = given.is_none() || member(object, plain).is_some();
    let mut first = true;
    let mut write = |out: &mut String, name: &str, value: &str| {
        out.push(if first { '{' } else { ',' });
        first = false;
        out.push_str(name);
        out.push(':');
        out.push_str(value);
    };
    let quoted = |out: &mut String| {
        out.push('"');
        out.push_str(plain);
        out.push('"');
    };
    for (name, value) in Members::of(object).into_iter().flatten() {
        if !placed && name_after(name, plain) {
            let mut name_text = String::new();
            quoted(&mut name_text);
            write(out, &name_text, ABSENT);
            placed = true;
        }
        if is_name(name, plain) && given.is_some_and(|id| matches(value, id)) {
            continue;
        }
        write(out, name, value);
    }
    if !placed {
        let mut name_text = String::new();
        quoted(&mut name_text);
        write(out, &name_text, ABSENT);
    }
    if first {
        out.push('{');
    }
    out.push('}');
}

/// Whether the canonical name `name` sorts after `plain`, a name in ASCII
/// with nothing to escape (RFC 8785 §3.2.3).
fn name_after(name: &str, plain: &str) -> bool {
    let quoted = name.strip_prefix('"').and_then(|n| n.strip_suffix('"'));
    match quoted {
        Some(inner) if inner.is_ascii() && !inner.contains('\\') => inner > plain,
        _ => name_order(name, plain) == Some(Ordering::Greater),
    }
}

/// Whether the canonical value `text` is the string `id`, as the reader
/// writes it.
fn string_is(text: &str, id: &str) -> bool {
    match text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        Some(inner) if !needs_escape(id) => inner == id,
        _ => serde_json::to_string(id).is_ok_and(|written| written == text),
    }
}

/// Whether the canonical value `text` is the `OBJECT_VERSION_ID` of `id`, as
/// the reader writes it.
fn version_is(text: &str, id: &str) -> bool {
    text.strip_prefix("{\"_type\":\"OBJECT_VERSION_ID\",\"value\":")
        .and_then(|rest| rest.strip_suffix('}'))
        .is_some_and(|value| string_is(value, id))
}

/// Whether the string `text` writes an escape in its canonical form.
fn needs_escape(text: &str) -> bool {
    text.bytes()
        .any(|byte| byte < 0x20 || byte == b'"' || byte == b'\\')
}

/// The one element of the canonical array `items` when that element is an
/// object; `None` for any other value.
fn only_item(items: &str) -> Option<&str> {
    let bytes = items.as_bytes();
    if bytes.first() != Some(&b'[') || bytes.get(1) != Some(&b'{') {
        return None;
    }
    let end = value_end(bytes, 1)?;
    (bytes.get(end) == Some(&b']') && end + 1 == bytes.len())
        .then(|| items.get(1..end))
        .flatten()
        .filter(|item| Members::of(item).is_some())
}

/// The canonical text of the `OBJECT_VERSION_ID` whose value is `id`.
///
/// # Errors
/// A [`serde_json::Error`], which a string never raises.
fn version_text(id: &str) -> Result<String, serde_json::Error> {
    Ok(format!(
        "{{\"_type\":\"OBJECT_VERSION_ID\",\"value\":{}}}",
        serde_json::to_string(id)?
    ))
}

/// The value of the row part's member `name`, `value`, with the member its
/// index columns give back restored ([`without_columns`]).
///
/// # Errors
/// A [`serde_json::Error`], which a string never raises.
pub(super) fn restored<'t>(
    name: &str,
    value: &'t str,
    index: &IndexColumns<'_>,
) -> Result<Cow<'t, str>, serde_json::Error> {
    if is_name(name, CHANNEL)
        && let Some(channel) = members(value)
    {
        let given = index.request_id.map(serde_json::to_string).transpose()?;
        return Ok(given_back(channel, REQUEST_ID, given).into());
    }
    if is_name(name, ITEMS)
        && let Some(item) = only_item(value).and_then(members)
    {
        let version = match index.resource_id {
            Some(id) if last_named(&item, "object").is_none() => Some(version_text(id)?),
            _ => None,
        };
        let item = given_back(item, VERSION_ID, version);
        return Ok(array(&[item.as_str()]).into());
    }
    Ok(Cow::Borrowed(value))
}

/// The canonical object `object` with its member `plain` given back as
/// `given` when it has none, and left out where it is named `null`.
fn given_back(object: Vec<(&str, &str)>, plain: &str, given: Option<String>) -> String {
    let mut out: Vec<Member<'_>> = Vec::with_capacity(object.len() + 1);
    let mut named = false;
    for (name, value) in object {
        if is_name(name, plain) {
            named = true;
            if value == ABSENT {
                continue;
            }
        }
        out.push((name.into(), value.into()));
    }
    if let (false, Some(given)) = (named, given) {
        out.push((format!("\"{plain}\"").into(), given.into()));
    }
    object_text(out)
}
