// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use openehr_base::prelude::ObjectVersionId;
use serde_json::{Value, json};

use super::{Body, decode_member};
use crate::ids::VoId;
use crate::privacy::PrivacyPolicy;
use crate::privacy::config::PrivacyConfig;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::change::stamp_uid_value;
use crate::versioning::integrity::{Signed, signed_form};
use crate::versioning::object_version_id::TreeId;
use crate::versioning::tree::{Indexed, Tree as _, Typed};
use crate::versioning::wire::{OriginalVersionParts, build_original_version, contribution_ref};

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

/// The parts of the version `uid` names, with a predecessor, the merge
/// inputs `others` and the `contribution` and `commit_audit` given.
fn parts<'a, C, A>(
    uid: &'a ObjectVersionId,
    contribution: C,
    commit_audit: A,
    others: &'a [String],
    signature: Option<&'a str>,
) -> OriginalVersionParts<'a, C, A> {
    OriginalVersionParts {
        creating_system_id: "cdr.example",
        vo_id: VoId(uid.object_id().value().parse().unwrap()),
        tree: TreeId::trunk(2),
        preceding_version_uid: Some("8849182c-82ad-4088-a07f-48ead4180515::other.example::1"),
        other_input_version_uids: others,
        contribution,
        commit_audit,
        lifecycle_state: "532",
        data: &Value::Null,
        attestations: &[],
        signature,
    }
}

/// A commit audit with a coded description and an identified committer.
fn audit() -> AuditInput {
    AuditInput {
        system_id: "cdr.example".to_owned(),
        change_type: "249".to_owned(),
        description: Some(
            crate::versioning::audit::decode_description(&json!({
                "_type": "DV_CODED_TEXT",
                "value": "Größe",
                "defining_code": { "terminology_id": { "value": "local" }, "code_string": "at0001" }
            }))
            .unwrap(),
        ),
        committer: crate::versioning::audit::party_proxy(&json!({
            "_type": "PARTY_IDENTIFIED",
            "name": "Dr. Ærø",
            "identifiers": [{ "id": "1", "issuer": "i", "assigner": "a", "type": "t" }]
        }))
        .unwrap(),
        attestation: None,
    }
}

/// The typed commit writes the `version.body` bytes, the content digest and
/// the RFC 8785 signed form the canonical-value commit wrote from the
/// canonical value of the same typed body, on every document of every
/// versioned kind in the corpora (#6003).
#[test]
fn the_typed_commit_writes_and_signs_the_bytes_the_canonical_one_did() {
    let documents = corpus();
    let uid = ObjectVersionId::new("8849182c-82ad-4088-a07f-48ead4180515::cdr.example::2").unwrap();
    let audit = audit();
    let at: jiff::Timestamp = "2026-11-18T10:11:12.123456Z".parse().unwrap();
    let contribution = uuid::Uuid::from_u128(7);
    let others = ["8849182c-82ad-4088-a07f-48ead4180515::third.example::4".to_owned()];
    // The envelope a read rebuilds to verify the signature, from canonical values.
    let envelope = build_original_version(parts(
        &uid,
        openehr_its::json::to_canonical_value(&contribution_ref(contribution)),
        audit.canonical(&at),
        &others,
        Some("left out"),
    ))
    .unwrap();
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
            signed_form(
                parts(&uid, contribution_ref(contribution), audit.typed(&at), &others, None),
                Signed::Typed(&body)
            )
            .unwrap()
            .unwrap(),
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

/// A nine-digit value that passes the BSN eleven-test and names no person.
const SYNTHETIC_BSN: &str = "123456782"; // privacy-allow: synthetic, built to satisfy the published arithmetic

/// `composition` with the record's subject as its composer, by name and by a
/// formal identifier: what the data-minimisation pass refuses.
fn identifying(composition: &Value) -> Value {
    let mut planted = composition.clone();
    planted["composer"] = json!({
        "_type": "PARTY_RELATED",
        "name": "A. Subject",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "id": SYNTHETIC_BSN }],
        "relationship": {
            "_type": "DV_CODED_TEXT",
            "value": "self",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "0"
            }
        }
    });
    planted
}

/// The typed reading of a body is its canonical JSON, value for value and in
/// order, and the data-minimisation pass finds the same on it as on the
/// rendering, on every document of every versioned kind in the corpora and on
/// every COMPOSITION of them made to identify its subject (#6038).
#[test]
fn the_typed_reading_of_a_body_is_its_canonical_json() {
    let policy = PrivacyPolicy::from(&PrivacyConfig::default());
    let mut found = 0;
    let mut read = 0;
    for (path, kind, body, value) in corpus() {
        let planted = (kind == Kind::Composition)
            .then(|| typed(kind, &identifying(&value)))
            .flatten();
        for body in std::iter::once(body).chain(planted) {
            let rm = body.as_rm().expect("a typed body");
            let typed = Typed::Object(rm);
            assert_eq!(
                typed.to_value(),
                openehr_its::json::to_canonical_value(&body),
                "{path}"
            );
            let rendered = body.render().unwrap();
            let want = policy.findings(kind.as_str(), Indexed::root(&rendered));
            assert_eq!(policy.findings(kind.as_str(), typed), want, "{path}");
            found += want.len();
            read += 1;
        }
    }
    assert!(
        read >= 360 && found >= 180,
        "{read} bodies, {found} findings"
    );
}
