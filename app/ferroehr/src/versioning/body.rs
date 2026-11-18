// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The content a commit carries: a typed COMPOSITION, or the canonical value
//! of a kind whose commit has not moved onto typed RM yet (#6003).
//!
//! The typed COMPOSITION is what the commit validates, offloads, stamps,
//! renders once as `version.body` and its node rows, and signs, with no
//! canonical value built; only a `553|incomplete|` commit validates over one.
//! No openEHR spec governs the commit's internal representation: our own
//! design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the canonical value of the kinds whose commit \
              is not typed yet, and the view the JSON-reading passes take (#6003)"
)]

use openehr_base::prelude::{ObjectVersionId, UidBasedId};
use openehr_rm::prelude::Composition;
use serde_json::Value;

use crate::error::ServiceError;
use crate::storage::render::index::Rendered;

/// The content of a create or a modify. It has no `Debug`: it is clinical
/// content (`reliability.md`).
#[derive(Clone)]
pub(crate) enum Body {
    /// A COMPOSITION as typed RM.
    Composition(Box<Composition>),
    /// The canonical value of any other kind.
    Canonical(Value),
}

impl From<Value> for Body {
    fn from(canonical: Value) -> Self {
        Body::Canonical(canonical)
    }
}

impl Body {
    /// A typed COMPOSITION body.
    pub(crate) fn composition(rm: Composition) -> Self {
        Body::Composition(Box::new(rm))
    }

    /// The canonical value of a body of a kind that has no typed form.
    pub(crate) fn canonical(&self) -> Option<&Value> {
        match self {
            Body::Composition(_) => None,
            Body::Canonical(view) => Some(view),
        }
    }

    /// The root's `archetype_node_id`.
    pub(crate) fn archetype_node_id(&self) -> Option<&str> {
        match self {
            Body::Composition(rm) => Some(rm.archetype_node_id.as_str()),
            Body::Canonical(view) => view.get("archetype_node_id").and_then(Value::as_str),
        }
    }

    /// The template id the root's `archetype_details` names.
    pub(crate) fn template_id(&self) -> Option<&str> {
        match self {
            Body::Composition(rm) => rm
                .archetype_details
                .as_ref()
                .and_then(|details| details.template_id.as_ref())
                .map(|id| id.value.as_str()),
            Body::Canonical(view) => view
                .pointer("/archetype_details/template_id/value")
                .and_then(Value::as_str),
        }
    }

    /// The code string of a COMPOSITION's `category`.
    pub(crate) fn category_code(&self) -> Option<&str> {
        match self {
            Body::Composition(rm) => Some(rm.category.defining_code.code_string.as_str()),
            Body::Canonical(view) => view
                .pointer("/category/defining_code/code_string")
                .and_then(Value::as_str),
        }
    }

    /// What the write rules of a caller read of a COMPOSITION body
    /// ([`super::content_facts`]).
    pub(crate) fn content_facts(&self) -> super::content_facts::ContentFacts<'_> {
        match self {
            Body::Composition(rm) => super::content_facts::ContentFacts::of_typed(rm),
            Body::Canonical(view) => super::content_facts::ContentFacts::of_canonical(view),
        }
    }

    /// Stamps the version's own `OBJECT_VERSION_ID` into the root `uid`.
    pub(crate) fn stamp_uid(&mut self, uid: &ObjectVersionId) {
        match self {
            Body::Composition(rm) => rm.uid = Some(UidBasedId::ObjectVersionId(uid.clone())),
            Body::Canonical(view) => super::change::stamp_uid_value(view, uid),
        }
    }

    /// The one rendering of the body to compact canonical JSON, indexed
    /// ([`crate::storage::render::index`]).
    ///
    /// # Errors
    /// [`ServiceError`] when the body does not serialize (a non-finite
    /// number, unreachable for a codec-produced value, refused loudly anyway).
    pub(crate) fn render(&self) -> Result<Rendered, ServiceError> {
        let fault = |e| ServiceError::internal("serialize the canonical body", e);
        match self {
            Body::Composition(rm) => {
                crate::telemetry::conversion::record("rm_to_text");
                crate::storage::render::index::render(rm.as_ref()).map_err(fault)
            }
            Body::Canonical(view) => Self::render_canonical(view),
        }
    }

    /// [`Self::render`] of a canonical value.
    ///
    /// # Errors
    /// As [`Self::render`].
    pub(crate) fn render_canonical(canonical: &Value) -> Result<Rendered, ServiceError> {
        crate::telemetry::conversion::record("value_to_text");
        crate::storage::render::index::render(canonical)
            .map_err(|e| ServiceError::internal("serialize the canonical body", e))
    }
}

#[cfg(test)]
mod tests {
    use openehr_base::prelude::ObjectVersionId;
    use openehr_rm::prelude::Composition;
    use serde_json::{Value, json};

    use super::Body;
    use crate::versioning::change::stamp_uid_value;

    /// Every COMPOSITION in the repository's corpora that reads as strict typed RM.
    fn corpus() -> Vec<(String, Composition)> {
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
                if !text.contains("\"COMPOSITION\"") {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if value.get("_type").and_then(Value::as_str) != Some("COMPOSITION") {
                    continue;
                }
                // NOTE: a document the strict reader refuses never reaches a
                // commit, so it has no typed path to compare.
                if let Ok(typed) = openehr_its::json::from_canonical_value::<Composition>(&value) {
                    out.push((path.display().to_string(), typed));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The text and digest the commit wrote from the canonical value, before
    /// the typed path: the compact text, and SHA-256 of it without the root `uid`.
    fn canonical_path(view: &Value) -> (String, [u8; 32]) {
        use sha2::Digest as _;
        let mut without = view.clone();
        without.as_object_mut().unwrap().shift_remove("uid");
        (
            serde_json::to_string(view).unwrap(),
            sha2::Sha256::digest(serde_json::to_vec(&without).unwrap()).into(),
        )
    }

    /// The typed commit writes the `version.body` bytes, the content digest and
    /// the RFC 8785 signed form the canonical-value commit wrote, on every
    /// COMPOSITION of the corpora (#6003).
    #[test]
    fn the_typed_commit_writes_and_signs_the_bytes_the_canonical_one_did() {
        let compositions = corpus();
        assert!(
            compositions.len() >= 180,
            "{} compositions",
            compositions.len()
        );
        let uid =
            ObjectVersionId::new("8849182c-82ad-4088-a07f-48ead4180515::cdr.example::1").unwrap();
        let envelope = json!({
            "_type": "ORIGINAL_VERSION",
            "uid": { "_type": "OBJECT_VERSION_ID", "value": uid.value() },
            "contribution": { "id": { "_type": "HIER_OBJECT_ID", "value": "c" }, "namespace": "local", "type": "CONTRIBUTION" },
            "lifecycle_state": { "value": "complete", "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "532" } },
            "signature": "left out",
        });
        for (path, typed) in compositions {
            let mut view = openehr_its::json::to_canonical_value(&typed);
            stamp_uid_value(&mut view, &uid);
            let mut body = Body::composition(typed);
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
            let Body::Composition(rm) = &body else {
                panic!("{path} was built as a COMPOSITION");
            };
            assert_eq!(
                crate::versioning::integrity::typed_canonical_form(&envelope, rm).unwrap(),
                openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json_with_data(
                    &envelope, &view
                )
                .unwrap(),
                "{path}"
            );
        }
    }
}
