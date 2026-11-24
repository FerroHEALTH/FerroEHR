// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one rendering of a commit body and what is read off it (#6003).
//!
//! The text is `serde_json`'s compact output, so it carries no insignificant
//! whitespace, and a scan that knows strings and nesting finds a member's
//! bytes exactly. No openEHR spec governs storage: our own design.

pub mod index;
pub mod nodes;

use std::collections::BTreeSet;
use std::ops::Range;

use serde::Serialize;

use self::index::{Kind, Rendered};
use self::nodes::{StoredNode, stored_nodes};
use crate::storage::compact::RootIdentity;
use crate::storage::error::StorageError;

/// The node rows of one version and the external blob URIs its body
/// references.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decomposed {
    /// The rows, every column as it is stored.
    pub rows: Vec<StoredNode>,
    /// The external blob URIs, each once and sorted.
    pub blob_uris: Vec<String>,
}

/// The node rows and blob URIs of the rendered `body`, slimmed against the
/// version identity `root`.
///
/// # Errors
/// The decomposition's refusal of a body that is no single tree
/// ([`stored_nodes`]).
pub fn decomposed(body: &Rendered, root: &RootIdentity<'_>) -> Result<Decomposed, StorageError> {
    Ok(Decomposed {
        rows: stored_nodes(body, root)?,
        blob_uris: blob_uris(body),
    })
}

/// [`decomposed`] of `body`, rendered here: the writers that hold a body
/// rather than its rendering (the archive load, the import replay, the node
/// rebuild).
///
/// # Errors
/// [`StorageError::BodyEncode`] when `body` does not serialize, and the refusals of
/// [`decomposed`].
pub fn decompose<T: Serialize + ?Sized>(
    body: &T,
    root: &RootIdentity<'_>,
) -> Result<Decomposed, StorageError> {
    let rendered = index::render(body).map_err(StorageError::BodyEncode)?;
    decomposed(&rendered, root)
}

/// SHA-256 over the rendered `body` with its root `uid` member left out: the
/// digest a version's content is compared with its predecessor's on (#4415).
#[must_use]
pub fn content_digest(body: &Rendered) -> [u8; 32] {
    use sha2::Digest as _;
    let text = body.text().as_bytes();
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
    // The rendering writes every string as serde_json does, escaping no
    // ASCII letter, so a body naming no `"FEEDER_AUDIT"` holds no such object.
    if !body.text().contains("\"FEEDER_AUDIT\"") {
        return BTreeSet::new();
    }
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

/// The external blob URIs `body` references, each once and sorted.
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

/// The two slices of `text` around the member at `span`, with the comma that
/// joined it to a neighbour taken out as well, so their concatenation is the
/// object without that member.
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

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "the tests compare the digest with the canonical value it covers"
)]
mod tests {
    use serde_json::json;

    use super::content_digest;
    use super::index::render;

    /// The digest of the one rendering: SHA-256 of the compact text with the
    /// root `uid` taken out.
    #[test]
    fn the_digest_leaves_out_the_root_uid_and_nothing_else() {
        use sha2::Digest as _;
        let uid = json!({ "_type": "OBJECT_VERSION_ID", "value": "a::b::1" });
        for doc in [
            json!({ "_type": "COMPOSITION", "uid": uid, "name": { "value": "\"uid\":ä" } }),
            json!({ "uid": uid, "_type": "COMPOSITION", "language": 1.5 }),
            json!({ "_type": "COMPOSITION", "name": "n", "uid": uid }),
            json!({ "uid": uid }),
            json!({ "a": [1, {"uid": 2}, "}"], "uid": [], "b": null, "c": true }),
            json!({ "_type": "COMPOSITION", "context": { "uid": 1 } }),
            json!({}),
            json!(null),
        ] {
            let mut without = doc.clone();
            if let Some(map) = without.as_object_mut() {
                map.shift_remove("uid");
            }
            let expected: [u8; 32] =
                sha2::Sha256::digest(serde_json::to_vec(&without).unwrap()).into();
            assert_eq!(content_digest(&render(&doc).unwrap()), expected, "{doc}");
        }
    }
}
