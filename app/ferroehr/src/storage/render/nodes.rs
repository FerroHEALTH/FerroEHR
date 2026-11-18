// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The node rows of a body, read off its one rendering ([`super::index`]).
//!
//! The decomposition into structure nodes, and each row's slimmed fragment,
//! typed value, header and markers, with no canonical value built (#6003).
//! The rules are the ones the stored form is read back with
//! ([`crate::storage::slim::rebuild`], `ext.node_fragment`): a row leaves out
//! the keys its columns hold, an `ELEMENT` value of one of the typed shapes
//! moves to the value columns, a root's archetype header, language and
//! encoding move to the header columns, and the four markers stand for the
//! keys they replace. No openEHR spec governs storage: our own design.

use std::borrow::Cow;

use crate::storage::compact::{Compact, RootIdentity};
use crate::storage::error::StorageError;
use crate::storage::header::{ENCODING_TERMINOLOGY, Header, LANGUAGE_TERMINOLOGY, recased};
use crate::storage::slim::{STRING_VALUED, TypedValue};
use crate::storage::structure::{archetype_parts, is_structure_type, is_versioned_root_type};

use super::index::{Kind, Member, NodeId, Rendered};

/// One `node` row of a version, every column as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredNode {
    /// Pre-order number within the versioned object (root = 0).
    pub num: i32,
    /// Max `num` in this row's subtree.
    pub num_cap: i32,
    /// `num` of the parent structure node (the root points at 0).
    pub parent_num: i32,
    /// The RM `_type`.
    pub rm_type: String,
    /// `archetype_node_id`, lowercased.
    pub archetype: Option<String>,
    /// `qualified_rm_entity` of a full archetype HRID, lowercased.
    pub arch_entity: Option<String>,
    /// `domain_concept` of a full archetype HRID, lowercased.
    pub arch_concept: Option<String>,
    /// The major version of a full archetype HRID.
    pub arch_major: Option<i32>,
    /// `name/value`.
    pub name: Option<String>,
    /// The code string of a coded name.
    pub name_code: Option<String>,
    /// The terminology id of a coded name.
    pub name_terminology: Option<String>,
    /// The materialized path from the root (`content0.data.events1.`).
    pub path: String,
    /// The slimmed fragment text, `None` when nothing is left of it.
    pub data: Option<String>,
    /// The `ELEMENT.value` the typed columns hold.
    pub value: Option<TypedValue>,
    /// The header columns.
    pub header: Header,
    /// The marker columns.
    pub compact: Compact,
    /// The promoted-leaf text, aligned to
    /// [`crate::storage::promoted::PROMOTED_LEAVES`]; empty below the root.
    pub promoted: Vec<Option<String>>,
}

/// The node rows `body` decomposes into, slimmed against the version identity
/// `root`.
///
/// # Errors
/// [`StorageError::NotAStructureRoot`] when the root has no versioned-root
/// `_type`, and [`StorageError::MixedArray`] for an array that mixes
/// structure and non-structure members.
pub fn stored_nodes(
    body: &Rendered,
    root: &RootIdentity<'_>,
) -> Result<Vec<StoredNode>, StorageError> {
    let top = body.root();
    let root_type = body.rm_type(top);
    if !root_type.as_deref().is_some_and(is_versioned_root_type) {
        return Err(StorageError::NotAStructureRoot(
            root_type.map(Cow::into_owned),
        ));
    }
    let mut rows = Vec::new();
    walk(body, top, String::new(), -1, root, &mut rows)?;
    let mut caps: Vec<i32> = rows.iter().map(|r| r.num).collect();
    for i in (1..rows.len()).rev() {
        let (Some(cap), Some(parent)) = (
            caps.get(i).copied(),
            rows.get(i)
                .and_then(|r| usize::try_from(r.parent_num).ok())
                .filter(|parent| *parent < i),
        ) else {
            return Err(StorageError::InvalidRows(format!(
                "node row {i} does not reference an earlier parent row"
            )));
        };
        if let Some(parent_cap) = caps.get_mut(parent) {
            *parent_cap = (*parent_cap).max(cap);
        }
        if let Some(row) = rows.get_mut(i) {
            row.num_cap = cap;
        }
    }
    if let (Some(row), Some(cap)) = (rows.first_mut(), caps.first()) {
        row.num_cap = *cap;
    }
    Ok(rows)
}

/// Pushes the row of `node` and then, in written order, the rows of its
/// structure children.
fn walk(
    body: &Rendered,
    node: NodeId,
    path: String,
    parent: i32,
    root: &RootIdentity<'_>,
    rows: &mut Vec<StoredNode>,
) -> Result<(), StorageError> {
    let index = rows.len();
    let num = i32::try_from(index).unwrap_or(i32::MAX);
    let rm_type = body.rm_type(node).map(Cow::into_owned).unwrap_or_default();
    let archetype = body
        .get_str(node, "archetype_node_id")
        .map(|id| id.to_ascii_lowercase());
    let (arch_entity, arch_concept, arch_major) = archetype
        .as_deref()
        .and_then(archetype_parts)
        .map_or((None, None, None), |(e, c, m)| (Some(e), Some(c), Some(m)));
    let label = body.get(node, "name");
    let name = label
        .and_then(|n| body.get_str(n, "value"))
        .map(Cow::into_owned);
    let defining_code = label.and_then(|n| body.get(n, "defining_code"));
    let name_code = defining_code
        .and_then(|c| body.get_str(c, "code_string"))
        .map(Cow::into_owned);
    let name_terminology = defining_code
        .and_then(|c| body.get(c, "terminology_id"))
        .and_then(|t| body.get_str(t, "value"))
        .map(Cow::into_owned);
    let promoted = promoted(body, num, &rm_type, node);

    let mut kept: Vec<&Member> = Vec::new();
    let mut structure: Vec<&Member> = Vec::new();
    for member in body.members(node) {
        if carries_structure(body, member)? {
            structure.push(member);
        } else {
            kept.push(member);
        }
    }
    let slimmed = slim(body, &rm_type, (num == 0).then_some(root), kept);
    rows.push(StoredNode {
        num,
        num_cap: num,
        parent_num: parent.max(0),
        rm_type,
        archetype,
        arch_entity,
        arch_concept,
        arch_major,
        name,
        name_code,
        name_terminology,
        path: String::new(),
        data: slimmed.data,
        value: slimmed.value,
        header: slimmed.header,
        compact: slimmed.compact,
        promoted,
    });
    for member in structure {
        let attribute = body.key(member);
        let child = Rendered::value(member);
        if body.kind(child) == Some(Kind::Object) {
            walk(body, child, format!("{path}{attribute}."), num, root, rows)?;
        } else {
            for (i, item) in body.items(child).iter().enumerate() {
                walk(
                    body,
                    *item,
                    format!("{path}{attribute}{i}."),
                    num,
                    root,
                    rows,
                )?;
            }
        }
    }
    if let Some(row) = rows.get_mut(index) {
        row.path = path;
    }
    Ok(())
}

/// Whether one member's value is a structure node, or an array of them.
fn carries_structure(body: &Rendered, member: &Member) -> Result<bool, StorageError> {
    let value = Rendered::value(member);
    if body.kind(value) != Some(Kind::Array) {
        return Ok(is_structure(body, value));
    }
    let items = body.items(value);
    let count = items.iter().filter(|i| is_structure(body, **i)).count();
    if count == 0 {
        return Ok(false);
    }
    if count == items.len() {
        return Ok(true);
    }
    Err(StorageError::MixedArray {
        attribute: body.key(member).to_owned(),
    })
}

fn is_structure(body: &Rendered, node: NodeId) -> bool {
    body.rm_type(node).as_deref().is_some_and(is_structure_type)
}

/// The promoted leaves of a root row ([`crate::storage::promoted::extract`]).
fn promoted(body: &Rendered, num: i32, rm_type: &str, node: NodeId) -> Vec<Option<String>> {
    if num != 0 {
        return Vec::new();
    }
    crate::storage::promoted::PROMOTED_LEAVES
        .iter()
        .map(|leaf| {
            if rm_type != leaf.rm_type {
                return None;
            }
            let mut at = node;
            for step in leaf.path {
                at = body.get(at, step)?;
            }
            match body.kind(at)? {
                Kind::String | Kind::EscapedString => body.str(at).map(Cow::into_owned),
                Kind::Scalar if body.raw(at) != b"null" => {
                    std::str::from_utf8(body.raw(at)).ok().map(str::to_owned)
                }
                Kind::Scalar | Kind::Object | Kind::Array => None,
            }
        })
        .collect()
}

/// What a row stores of its fragment.
struct Slimmed {
    data: Option<String>,
    value: Option<TypedValue>,
    header: Header,
    compact: Compact,
}

/// A kept member: its bytes as written, or a replacement for its value.
struct Kept<'a> {
    member: &'a Member,
    /// The raw JSON of the value that replaces the member's own.
    replaced: Option<&'a [u8]>,
}

/// Slims the members `kept` of one fragment ([`crate::storage::slim::slim`]).
fn slim<'a>(
    body: &'a Rendered,
    rm_type: &str,
    root: Option<&RootIdentity<'_>>,
    kept: Vec<&'a Member>,
) -> Slimmed {
    let mut members: Vec<Kept<'a>> = kept
        .into_iter()
        .map(|member| Kept {
            member,
            replaced: None,
        })
        .collect();
    let find =
        |members: &[Kept<'a>], key: &str| members.iter().rposition(|k| body.key(k.member) == key);
    let value_of = |members: &[Kept<'a>], key: &str| {
        find(members, key)
            .and_then(|i| members.get(i))
            .filter(|k| k.replaced.is_none())
            .map(|k| Rendered::value(k.member))
    };
    let remove = |members: &mut Vec<Kept<'a>>, key: &str| {
        members.retain(|k| body.key(k.member) != key);
    };

    if value_of(&members, "_type")
        .and_then(|t| body.str(t))
        .is_some_and(|t| t == rm_type)
    {
        remove(&mut members, "_type");
    }

    let mut header = Header::default();
    if let Some(rm_version) = derived_archetype_details(
        body,
        value_of(&members, "archetype_node_id"),
        value_of(&members, "archetype_details"),
        root,
    ) {
        remove(&mut members, "archetype_details");
        remove(&mut members, "archetype_node_id");
        header.rm_version = Some(rm_version);
    }
    if let Some(code) =
        value_of(&members, "language").and_then(|p| code_phrase(body, p, LANGUAGE_TERMINOLOGY))
    {
        remove(&mut members, "language");
        header.language = Some(code);
    }
    if let Some(code) =
        value_of(&members, "encoding").and_then(|p| code_phrase(body, p, ENCODING_TERMINOLOGY))
    {
        remove(&mut members, "encoding");
        header.encoding = Some(code);
    }

    let mut compact = Compact {
        origin: date_time_as_string(body, &mut members, "origin"),
        time: date_time_as_string(body, &mut members, "time"),
        ..Compact::default()
    };
    if value_of(&members, "subject").is_some_and(|s| {
        body.has_keys(s, &["_type"]) && body.rm_type(s).as_deref() == Some("PARTY_SELF")
    }) {
        remove(&mut members, "subject");
        compact.party_self = true;
    }
    if let Some(root) = root
        && value_of(&members, "uid").is_some_and(|uid| {
            body.is_typed_string(uid, "OBJECT_VERSION_ID", Some(root.version_uid))
        })
    {
        remove(&mut members, "uid");
        compact.uid_derived = true;
    }

    if value_of(&members, "archetype_node_id")
        .and_then(|id| body.str(id))
        .is_some_and(|id| id == id.to_ascii_lowercase())
    {
        remove(&mut members, "archetype_node_id");
    }
    if value_of(&members, "name").is_some_and(|n| rebuildable_name(body, n)) {
        remove(&mut members, "name");
    }
    let mut value = None;
    if rm_type == "ELEMENT"
        && let Some(typed) = value_of(&members, "value").and_then(|v| typed_value(body, v))
    {
        remove(&mut members, "value");
        value = Some(typed);
    }
    Slimmed {
        data: fragment(body, &members),
        value,
        header,
        compact,
    }
}

/// Stores the member `key` as the value string of its `DV_DATE_TIME` when it
/// is one ([`crate::storage::compact`]), and says whether it did.
fn date_time_as_string<'a>(body: &'a Rendered, members: &mut [Kept<'a>], key: &str) -> bool {
    let Some(kept) = members
        .iter_mut()
        .rev()
        .find(|k| body.key(k.member) == key)
        .filter(|k| k.replaced.is_none())
    else {
        return false;
    };
    let value = Rendered::value(kept.member);
    if !body.is_typed_string(value, "DV_DATE_TIME", None) {
        return false;
    }
    let Some(text) = body.get(value, "value") else {
        return false;
    };
    kept.replaced = Some(body.raw(text));
    true
}

/// The fragment text of the members left, `None` when none are.
fn fragment(body: &Rendered, members: &[Kept<'_>]) -> Option<String> {
    // NOTE: a name written twice keeps its last value, as the canonical value
    // the fragment was taken from did.
    let last: Vec<&Kept<'_>> = members
        .iter()
        .enumerate()
        .filter(|(i, k)| {
            !members
                .get(i + 1..)
                .unwrap_or_default()
                .iter()
                .any(|later| body.key(later.member) == body.key(k.member))
        })
        .map(|(_, k)| k)
        .collect();
    if last.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(
        last.iter()
            .map(|k| body.member_raw(k.member).len() + 1)
            .sum::<usize>()
            + 1,
    );
    out.push(b'{');
    for (i, kept) in last.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        match kept.replaced {
            None => out.extend_from_slice(body.member_raw(kept.member)),
            Some(value) => {
                out.push(b'"');
                out.extend_from_slice(body.key(kept.member).as_bytes());
                out.extend_from_slice(b"\":");
                out.extend_from_slice(value);
            }
        }
    }
    out.push(b'}');
    String::from_utf8(out).ok()
}

/// The `rm_version` of an `archetype_details` the header columns rebuild
/// from the archetype id and the version's template
/// ([`crate::storage::header`]).
fn derived_archetype_details(
    body: &Rendered,
    node_id: Option<NodeId>,
    details: Option<NodeId>,
    root: Option<&RootIdentity<'_>>,
) -> Option<String> {
    let node_id = body.str(node_id?)?;
    if recased(&node_id.to_ascii_lowercase()).as_deref() != Some(&*node_id) {
        return None;
    }
    let details = details?;
    let template = root.and_then(|root| root.template_id);
    let keys: &[&str] = if template.is_some() {
        &["_type", "archetype_id", "rm_version", "template_id"]
    } else {
        &["_type", "archetype_id", "rm_version"]
    };
    if !body.has_keys(details, keys) || body.rm_type(details).as_deref() != Some("ARCHETYPED") {
        return None;
    }
    if let Some(template) = template
        && !body
            .get(details, "template_id")
            .is_some_and(|t| body.is_typed_string(t, "TEMPLATE_ID", Some(template)))
    {
        return None;
    }
    let id = body.get(details, "archetype_id")?;
    if !body.is_typed_string(id, "ARCHETYPE_ID", Some(&node_id)) {
        return None;
    }
    body.get_str(details, "rm_version").map(Cow::into_owned)
}

/// The code string of a `CODE_PHRASE` in `terminology` that carries nothing
/// else.
fn code_phrase(body: &Rendered, phrase: NodeId, terminology: &str) -> Option<String> {
    if !body.has_keys(phrase, &["_type", "code_string", "terminology_id"])
        || body.rm_type(phrase).as_deref() != Some("CODE_PHRASE")
    {
        return None;
    }
    let id = body.get(phrase, "terminology_id")?;
    if !body.is_typed_string(id, "TERMINOLOGY_ID", Some(terminology)) {
        return None;
    }
    body.get_str(phrase, "code_string").map(Cow::into_owned)
}

/// Whether `name` is a `DV_TEXT` or a simply coded `DV_CODED_TEXT` the name
/// columns rebuild exactly.
fn rebuildable_name(body: &Rendered, name: NodeId) -> bool {
    match body.rm_type(name).as_deref() {
        Some("DV_TEXT") => body.is_typed_string(name, "DV_TEXT", None),
        Some("DV_CODED_TEXT") => coded(body, name).is_some(),
        _ => false,
    }
}

/// The value and the `(code_string, terminology_id)` of a `DV_CODED_TEXT`
/// that carries nothing but those.
fn coded(body: &Rendered, fields: NodeId) -> Option<(String, (String, String))> {
    if !body.has_keys(fields, &["_type", "defining_code", "value"]) {
        return None;
    }
    let text = string(body, fields, "value")?;
    let code = body.get(fields, "defining_code")?;
    if !body.has_keys(code, &["_type", "code_string", "terminology_id"])
        || body.rm_type(code).as_deref() != Some("CODE_PHRASE")
    {
        return None;
    }
    let code_string = string(body, code, "code_string")?;
    let terminology = body.get(code, "terminology_id")?;
    if !body.has_keys(terminology, &["_type", "value"])
        || body.rm_type(terminology).as_deref() != Some("TERMINOLOGY_ID")
    {
        return None;
    }
    Some((text, (code_string, string(body, terminology, "value")?)))
}

/// The string member `key` of `fields`.
fn string(body: &Rendered, fields: NodeId, key: &str) -> Option<String> {
    body.get_str(fields, key).map(Cow::into_owned)
}

/// The number member `key` of `fields`, in the lexical form it was written in.
fn number(body: &Rendered, fields: NodeId, key: &str) -> Option<String> {
    let value = body.get(fields, key)?;
    body.is_number(value)
        .then(|| std::str::from_utf8(body.raw(value)).ok().map(str::to_owned))
        .flatten()
}

/// The typed form of an `ELEMENT.value` ([`crate::storage::slim`]).
fn typed_value(body: &Rendered, value: NodeId) -> Option<TypedValue> {
    let value_type = body.rm_type(value)?;
    let typed = |num: Option<String>, text: Option<String>, coding: Option<(String, String)>| {
        let (code, terminology) = coding.unzip();
        TypedValue {
            value_type: value_type.clone().into_owned(),
            num,
            text,
            code,
            terminology,
        }
    };
    let is_string = |key: &str| body.get(value, key).is_some_and(|v| body.is_string(v));
    match &*value_type {
        t if STRING_VALUED.contains(&t) => (body.has_keys(value, &["_type", "value"])
            && is_string("value"))
        .then(|| typed(None, string(body, value, "value"), None)),
        "DV_BOOLEAN" => {
            let flag = body.get(value, "value").and_then(|v| body.bool(v))?;
            body.has_keys(value, &["_type", "value"])
                .then(|| typed(None, Some(flag.to_string()), None))
        }
        "DV_IDENTIFIER" => (body.has_keys(value, &["_type", "id"]) && is_string("id"))
            .then(|| typed(None, string(body, value, "id"), None)),
        "DV_COUNT" => body
            .has_keys(value, &["_type", "magnitude"])
            .then(|| number(body, value, "magnitude"))
            .flatten()
            .map(|n| typed(Some(n), None, None)),
        "DV_QUANTITY" => (body.has_keys(value, &["_type", "magnitude", "units"])
            && is_string("units"))
        .then(|| number(body, value, "magnitude"))
        .flatten()
        .map(|n| typed(Some(n), string(body, value, "units"), None)),
        "DV_CODED_TEXT" => {
            coded(body, value).map(|(text, coding)| typed(None, Some(text), Some(coding)))
        }
        "DV_ORDINAL" => {
            if !body.has_keys(value, &["_type", "symbol", "value"]) {
                return None;
            }
            let ordinal = number(body, value, "value")?;
            let symbol = body.get(value, "symbol")?;
            if body.rm_type(symbol).as_deref() != Some("DV_CODED_TEXT") {
                return None;
            }
            let (text, coding) = coded(body, symbol)?;
            Some(typed(Some(ordinal), Some(text), Some(coding)))
        }
        _ => None,
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "the test holds the rows read off the rendering equal to the rows the canonical \
              value decomposed and slimmed into"
)]
mod tests {
    use serde_json::Value;

    use super::{StoredNode, stored_nodes};
    use crate::storage::compact::RootIdentity;
    use crate::storage::render::index::render;
    use crate::storage::structure::is_versioned_root_type;

    /// Every versioned object in the repository's corpora, by path.
    fn corpus() -> Vec<(String, Value)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut out = Vec::new();
        let mut stack: Vec<std::path::PathBuf> = ["corpus", "crates", "docs/specs/openehr/CNF"]
            .iter()
            .map(|d| root.join(d))
            .collect();
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for path in entries.flatten().map(|e| e.path()) {
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n != "target") {
                        stack.push(path);
                    }
                    continue;
                }
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if value
                    .get("_type")
                    .and_then(Value::as_str)
                    .is_some_and(is_versioned_root_type)
                {
                    out.push((path.display().to_string(), value));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The rows the canonical-value decomposition and slimming store.
    fn canonical_rows(value: &Value, root: &RootIdentity<'_>) -> Vec<StoredNode> {
        let rows = crate::storage::codec::decompose(value.clone()).unwrap();
        rows.into_iter()
            .map(|row| {
                let slim = crate::storage::slim::slim(
                    &row.rm_type,
                    (row.num == 0).then_some(root),
                    row.data.clone(),
                );
                StoredNode {
                    num: row.num,
                    num_cap: row.num_cap,
                    parent_num: row.parent_num,
                    rm_type: row.rm_type,
                    archetype: row.archetype,
                    arch_entity: row.arch_entity,
                    arch_concept: row.arch_concept,
                    arch_major: row.arch_major,
                    name: row.name,
                    name_code: row.name_code,
                    name_terminology: row.name_terminology,
                    path: row.path,
                    data: slim.data.map(|d| serde_json::to_string(&d).unwrap()),
                    value: slim.value,
                    header: slim.header,
                    compact: slim.compact,
                    promoted: row.promoted,
                }
            })
            .collect()
    }

    /// A row with its fragment read back, so two fragments compare as the
    /// jsonb values they are stored as.
    fn comparable(mut row: StoredNode) -> (StoredNode, Option<Value>) {
        let data = row
            .data
            .take()
            .map(|d| serde_json::from_str::<Value>(&d).unwrap());
        (row, data)
    }

    /// The rows read off the one rendering are the rows the canonical value
    /// decomposed and slimmed into, every column, on every versioned object of
    /// the corpora, with and without the root identity matching (#6003).
    #[test]
    fn the_rendering_stores_the_rows_the_canonical_value_did() {
        let documents = corpus();
        assert!(documents.len() >= 200, "{} documents", documents.len());
        let mut compared = 0_usize;
        for (path, value) in documents {
            let uid = value
                .pointer("/uid/value")
                .and_then(Value::as_str)
                .unwrap_or("8849182c-82ad-4088-a07f-48ead4180515::cdr.example::1")
                .to_owned();
            let template = value
                .pointer("/archetype_details/template_id/value")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let expected_rows = crate::storage::codec::decompose(value.clone()).map(|_| ());
            let rendered = render(&value).unwrap();
            for root in [
                RootIdentity {
                    version_uid: &uid,
                    template_id: template.as_deref(),
                },
                RootIdentity {
                    version_uid: "other::cdr.example::2",
                    template_id: None,
                },
            ] {
                match expected_rows {
                    Ok(()) => {
                        let expected: Vec<_> = canonical_rows(&value, &root)
                            .into_iter()
                            .map(comparable)
                            .collect();
                        let actual: Vec<_> = stored_nodes(&rendered, &root)
                            .unwrap()
                            .into_iter()
                            .map(comparable)
                            .collect();
                        assert_eq!(actual, expected, "{path}");
                        compared += 1;
                    }
                    Err(ref refusal) => assert_eq!(
                        stored_nodes(&rendered, &root).unwrap_err().to_string(),
                        refusal.to_string(),
                        "{path}"
                    ),
                }
            }
        }
        assert!(compared >= 400, "{compared} comparisons");
    }
}
