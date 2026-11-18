// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one rendering of a commit body and what is read off it (#6003).
//!
//! The `version.body` text, its content digest, the node rows, the feeder
//! origins and the external blob references, with no canonical value built.
//!
//! The text is `serde_json`'s compact output, so it carries no insignificant
//! whitespace, and a scan that knows strings and nesting finds a member's
//! bytes exactly. No openEHR spec governs storage: our own design.

pub mod index;
pub mod nodes;

use std::collections::BTreeSet;
use std::ops::Range;

use self::index::{Kind, Rendered};

/// SHA-256 over the rendered `body` with its root `uid` member left out: the
/// digest a version's content is compared with its predecessor's on (#4415).
#[must_use]
pub fn content_digest(body: &Rendered) -> [u8; 32] {
    use sha2::Digest as _;
    let text = body.text();
    let mut digest = sha2::Sha256::new();
    let uid = body
        .members(body.root())
        .iter()
        .rev()
        .find(|m| body.key(m) == "uid");
    match uid {
        Some(member) => {
            let (head, tail) = without_member(text, Rendered::member_span(member));
            digest.update(head);
            digest.update(tail);
        }
        None => digest.update(text),
    }
    digest.finalize().into()
}

/// Every `FEEDER_AUDIT.originating_system_audit.system_id` in `body`, trimmed,
/// leaving out a blank one ([`crate::versioning::origins::fed`]).
#[must_use]
pub fn fed_origins(body: &Rendered) -> BTreeSet<String> {
    body.all()
        .filter(|n| body.kind(*n) == Some(Kind::Object))
        .filter(|n| body.rm_type(*n).as_deref() == Some("FEEDER_AUDIT"))
        .filter_map(|n| {
            let details = body.get(n, "originating_system_audit")?;
            let id = body.get_str(details, "system_id")?;
            let id = id.trim();
            (!id.is_empty()).then(|| id.to_owned())
        })
        .collect()
}

/// The external blob URIs `body` references, each once and sorted
/// ([`crate::storage::blob_ref::uris_of`]).
#[must_use]
pub fn blob_uris(body: &Rendered) -> Vec<String> {
    let found: BTreeSet<String> = body
        .all()
        .filter(|n| body.kind(*n) == Some(Kind::Object))
        .filter_map(|n| {
            let uri = body.get(n, "uri")?;
            let value = body.get_str(uri, "value")?;
            value
                .starts_with(crate::storage::blob_ref::EXTERNAL_BLOB_SCHEME)
                .then(|| value.into_owned())
        })
        .collect();
    found.into_iter().collect()
}

/// The byte range of the member `name` of the root object of the compact JSON
/// `text`.
///
/// The range runs from the opening quote of its name to the end of its value;
/// `None` when the root is no object, has no such member, or the text is not
/// compact JSON.
#[must_use]
pub fn root_member(text: &[u8], name: &str) -> Option<Range<usize>> {
    if text.first() != Some(&b'{') {
        return None;
    }
    let mut at = 1;
    while text.get(at) != Some(&b'}') {
        let start = at;
        let key_end = string_end(text, at)?;
        let key = text.get(start + 1..key_end - 1)?;
        if text.get(key_end) != Some(&b':') {
            return None;
        }
        let end = value_end(text, key_end + 1)?;
        if key == name.as_bytes() {
            return Some(start..end);
        }
        at = end;
        match text.get(at) {
            Some(b',') => at += 1,
            Some(b'}') => {}
            _ => return None,
        }
    }
    None
}

/// The two slices of `text` around the member at `span` ([`root_member`]),
/// with the comma that joined it to a neighbour taken out as well, so their
/// concatenation is the object without that member.
#[must_use]
pub fn without_member(text: &[u8], span: Range<usize>) -> (&[u8], &[u8]) {
    let (start, end) = if span.start > 1 && text.get(span.start - 1) == Some(&b',') {
        (span.start - 1, span.end)
    } else if text.get(span.end) == Some(&b',') {
        (span.start, span.end + 1)
    } else {
        (span.start, span.end)
    };
    (
        text.get(..start).unwrap_or_default(),
        text.get(end..).unwrap_or_default(),
    )
}

/// The index one past the closing quote of the string opening at `at`.
fn string_end(text: &[u8], at: usize) -> Option<usize> {
    if text.get(at) != Some(&b'"') {
        return None;
    }
    let mut i = at + 1;
    loop {
        match text.get(i)? {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
}

/// The index one past the end of the value starting at `at`.
fn value_end(text: &[u8], at: usize) -> Option<usize> {
    match text.get(at)? {
        b'"' => string_end(text, at),
        b'{' | b'[' => {
            let mut depth = 0_usize;
            let mut i = at;
            loop {
                match text.get(i)? {
                    b'"' => {
                        i = string_end(text, i)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth = depth.checked_sub(1)?;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        _ => {
            let mut i = at;
            while !matches!(text.get(i), None | Some(b',' | b'}' | b']')) {
                i += 1;
            }
            Some(i)
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "the tests compare the scan with the canonical value it reads"
)]
mod tests {
    use serde_json::json;

    use super::{root_member, without_member};

    fn removed(value: &serde_json::Value, name: &str) -> Option<String> {
        let text = serde_json::to_vec(value).unwrap();
        let span = root_member(&text, name)?;
        let (head, tail) = without_member(&text, span);
        Some(String::from_utf8([head, tail].concat()).unwrap())
    }

    #[test]
    fn a_root_member_comes_out_with_its_comma() {
        let uid = json!({ "_type": "OBJECT_VERSION_ID", "value": "a::b::1" });
        for doc in [
            json!({ "_type": "COMPOSITION", "uid": uid, "name": { "value": "\"uid\":ä" } }),
            json!({ "uid": uid, "_type": "COMPOSITION", "language": 1.5 }),
            json!({ "_type": "COMPOSITION", "name": "n", "uid": uid }),
            json!({ "uid": uid }),
            json!({ "a": [1, {"uid": 2}, "}"], "uid": [], "b": null, "c": true }),
        ] {
            let mut without = doc.clone();
            without.as_object_mut().unwrap().shift_remove("uid");
            assert_eq!(
                removed(&doc, "uid").as_deref(),
                Some(serde_json::to_string(&without).unwrap().as_str()),
                "{doc}"
            );
        }
    }

    #[test]
    fn a_nested_member_is_no_root_member() {
        let doc = json!({ "_type": "COMPOSITION", "context": { "uid": 1 } });
        assert_eq!(removed(&doc, "uid"), None);
        assert_eq!(removed(&json!(null), "uid"), None);
        assert_eq!(removed(&json!({}), "uid"), None);
    }
}
