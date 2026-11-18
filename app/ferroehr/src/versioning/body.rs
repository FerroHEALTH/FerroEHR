// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The content a commit carries: a typed COMPOSITION, or the canonical value
//! of a kind whose commit has not moved onto typed RM yet (#6003).
//!
//! The typed COMPOSITION is what the commit stamps, writes as `version.body`
//! and signs. Its `view` is the one canonical value the passes that still read
//! JSON take (archetype conformance, constraint bindings, privacy, multimedia
//! offload, node decomposition); it is produced once, at the edge of the
//! commit, and kept equal to the typed value. No openEHR spec governs the
//! commit's internal representation: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the canonical view the JSON-reading commit \
              passes take until they read typed RM (#6003)"
)]

use openehr_base::prelude::{ObjectVersionId, UidBasedId};
use openehr_rm::prelude::Composition;
use serde_json::Value;

use crate::error::ServiceError;

/// The content of a create or a modify. It has no `Debug`: it is clinical
/// content (`reliability.md`).
#[derive(Clone)]
pub(crate) enum Body {
    /// A COMPOSITION as typed RM, with its canonical view.
    Composition {
        /// The typed COMPOSITION the commit writes and signs.
        rm: Box<Composition>,
        /// Its canonical value, for the passes that still read JSON.
        view: Value,
    },
    /// The canonical value of any other kind.
    Canonical(Value),
}

impl From<Value> for Body {
    fn from(canonical: Value) -> Self {
        Body::Canonical(canonical)
    }
}

impl Body {
    /// A typed COMPOSITION body, with its canonical view rendered once.
    pub(crate) fn composition(rm: Composition) -> Self {
        let view = openehr_its::json::to_canonical_value(&rm);
        Body::Composition {
            rm: Box::new(rm),
            view,
        }
    }

    /// The canonical value of the body.
    pub(crate) fn view(&self) -> &Value {
        match self {
            Body::Composition { view, .. } | Body::Canonical(view) => view,
        }
    }

    /// The canonical value of a body whose typed form, if any, it does not
    /// change: the caller only reads it or rewrites a kind with no typed form.
    pub(crate) fn into_view(self) -> Value {
        match self {
            Body::Composition { view, .. } | Body::Canonical(view) => view,
        }
    }

    /// Re-reads the typed COMPOSITION from its view after a pass rewrote the
    /// view (the multimedia offload); a canonical body has nothing to re-read.
    ///
    /// # Errors
    /// [`ServiceError`] when the rewritten view no longer reads as a
    /// COMPOSITION, which the offload never produces.
    pub(crate) fn retype_from_view(&mut self) -> Result<(), ServiceError> {
        if let Body::Composition { rm, view } = self {
            **rm = openehr_its::json::from_canonical_value(view).map_err(|e| {
                ServiceError::internal("re-read the COMPOSITION after its offload", e)
            })?;
        }
        Ok(())
    }

    /// Stamps the version's own `OBJECT_VERSION_ID` into the root `uid` of
    /// the typed value and of its view alike.
    pub(crate) fn stamp_uid(&mut self, uid: &ObjectVersionId) {
        match self {
            Body::Composition { rm, view } => {
                rm.uid = Some(UidBasedId::ObjectVersionId(uid.clone()));
                super::change::stamp_uid_value(view, uid);
            }
            Body::Canonical(view) => super::change::stamp_uid_value(view, uid),
        }
    }
}

#[cfg(test)]
mod tests {
    use openehr_base::prelude::ObjectVersionId;
    use openehr_rm::prelude::Composition;
    use serde_json::{Value, json};

    use super::Body;
    use crate::versioning::change::resolve::body_text_and_digest;
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
            assert_eq!(
                body_text_and_digest(&body).unwrap(),
                canonical_path(&view),
                "{path}"
            );
            let Body::Composition { rm, .. } = &body else {
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
