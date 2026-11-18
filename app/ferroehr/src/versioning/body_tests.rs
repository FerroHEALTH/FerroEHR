// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use openehr_base::prelude::ObjectVersionId;
use serde_json::{Value, json};

use super::{Body, decode_member};
use crate::versioning::Kind;
use crate::versioning::change::stamp_uid_value;

/// Every document in the repository's corpora whose root is a versioned
/// kind and that reads as strict typed RM, with its kind, its typed body and
/// the canonical value it was read from.
fn corpus() -> Vec<(String, Kind, Body, Value)> {
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
            let Some(kind) = value
                .get("_type")
                .and_then(Value::as_str)
                .and_then(Kind::from_type)
            else {
                continue;
            };
            // NOTE: a document the strict reader refuses never reaches a
            // commit, so it has no typed path to compare.
            if let Some(body) = typed(kind, &value) {
                out.push((path.display().to_string(), kind, body, value));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The typed body of `value`, read as a commit reads a complete member of
/// `kind`; `None` when the strict reader refuses it.
fn typed(kind: Kind, value: &Value) -> Option<Body> {
    use openehr_its::json::from_canonical_value as read;
    use openehr_rm::v1_2::demographic::party::Party;
    use openehr_rm::v1_2::demographic::party_relationship::PartyRelationship;
    match kind {
        Kind::Agent | Kind::Group | Kind::Organisation | Kind::Person | Kind::Role => {
            read::<Party>(value).ok().map(Body::from)
        }
        Kind::PartyRelationship => read::<PartyRelationship>(value).ok().map(Body::from),
        _ => decode_member(kind, value, false).ok()?,
    }
}

/// The text and digest the commit wrote from a canonical value, before the
/// typed path: the compact text, and SHA-256 of it without the root `uid`.
fn canonical_path(view: &Value) -> (String, [u8; 32]) {
    use sha2::Digest as _;
    let mut without = view.clone();
    without.as_object_mut().unwrap().shift_remove("uid");
    (
        serde_json::to_string(view).unwrap(),
        sha2::Sha256::digest(serde_json::to_vec(&without).unwrap()).into(),
    )
}

/// The envelope a signed version joins its data to.
fn envelope(uid: &ObjectVersionId) -> Value {
    json!({
        "_type": "ORIGINAL_VERSION",
        "uid": { "_type": "OBJECT_VERSION_ID", "value": uid.value() },
        "contribution": { "id": { "_type": "HIER_OBJECT_ID", "value": "c" }, "namespace": "local", "type": "CONTRIBUTION" },
        "lifecycle_state": { "value": "complete", "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "532" } },
        "signature": "left out",
    })
}

/// The typed commit writes the `version.body` bytes, the content digest and
/// the RFC 8785 signed form the canonical-value commit wrote from the
/// canonical value of the same typed body, on every document of every
/// versioned kind in the corpora (#6003).
#[test]
fn the_typed_commit_writes_and_signs_the_bytes_the_canonical_one_did() {
    let documents = corpus();
    let uid = ObjectVersionId::new("8849182c-82ad-4088-a07f-48ead4180515::cdr.example::1").unwrap();
    let envelope = envelope(&uid);
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for (path, kind, mut body, _) in documents {
        *counts.entry(kind.as_str()).or_default() += 1;
        let mut view = openehr_its::json::to_canonical_value(&body);
        stamp_uid_value(&mut view, &uid);
        body.stamp_uid(&uid);
        let rendered = body.render().unwrap();
        assert_eq!(
            (
                String::from_utf8(rendered.text().to_vec()).unwrap(),
                crate::storage::render::content_digest(&rendered)
            ),
            canonical_path(&view),
            "{path}"
        );
        assert_eq!(
            crate::versioning::integrity::typed_canonical_form(&envelope, &body).unwrap(),
            openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json_with_data(
                &envelope, &view
            )
            .unwrap(),
            "{path}"
        );
    }
    assert!(
        counts.get("COMPOSITION").copied().unwrap_or_default() >= 180,
        "{counts:?}"
    );
    for kind in ["EHR_STATUS", "FOLDER", "PERSON"] {
        assert!(counts.contains_key(kind), "no {kind} in {counts:?}");
    }
}
