// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The **Validity Checking** component of the platform crate: the SM
//! `I_VALIDITY_CHECKER` interface realized on [`FerroEhrService`]'s existing
//! validation choke points.
//!
//! Spec: `docs/specs/openehr/SM/docs/openehr_platform/
//! master03-common_package.adoc` §Class Definitions and
//! `UML/classes/i_validity_checker.adoc` (the two calls `definitions_valid` and
//! `content_valid`, both over a `LOCATABLE`). The SM keeps `I_VALIDITY_CHECKER`
//! in its `common` package rather than among the platform services, so its impl
//! sits as the peer file `service/validity.rs`.
//!
//! Validation itself is owned by the validation register (`src/validation/`);
//! this file is only the SM interface adapter over the shared choke points
//! `FerroEhrService::web_template_for` and
//! `FerroEhrService::validate_for_commit`.
//!
//! NOTE: `i_validity_checker.adoc` §`definitions_valid` covers "archetype and
//! template identifiers", so the check collects every `ARCHETYPED` declaration
//! in the content and resolves archetype ids against both stored repositories
//! (ADL 1.4 + ADL 2) and template ids against both template stores.
//! `content_valid` runs the same per-`Kind` structural validation every commit
//! runs, and an unrecognized root `_type` is `false`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use serde_json::Value;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::status::SmError;
use crate::versioning::Kind;

impl FerroEhrService {
    /// `definitions_valid` (`i_validity_checker.adoc`): "Return `True` if the
    /// definition identifiers (i.e. archetype and template identifiers) are
    /// known in the local `definitions` service."
    ///
    /// Every `archetype_details` (RM `ARCHETYPED`) declaration in the content
    /// contributes its `archetype_id` and, where present, its `template_id`.
    /// A template id is known when it resolves through the same store lookup
    /// every commit uses (OPT 1.4 with the ADL2/OPT2 fallback). An archetype
    /// id is known when a declared template's own node tree carries it — an
    /// operational template inlines its constituent archetypes (AM OPT2
    /// `master03-opt_raw.adoc` §Semantics), so template-scoped content
    /// resolves through the template, not the source repositories — or when
    /// either stored repository holds it (the ADL 1.4 store or the ADL 2
    /// store; the clause names no dialect). Identity is compared
    /// case-insensitively (BASE `master05-identification_package.adoc`
    /// §Composite Identifiers and Case).
    /// Content declaring no definition identifiers at all resolves `true`:
    /// nothing is used, so nothing is unknown.
    ///
    /// # Errors
    ///
    /// An unknown identifier answers `Ok(false)`, never an error; a database
    /// or template-store fault propagates as its [`SmError`] mapping.
    pub async fn definitions_valid(&self, a_content: &Value) -> Result<bool, SmError> {
        let mut archetype_ids = std::collections::BTreeSet::new();
        let mut template_ids = std::collections::BTreeSet::new();
        collect_definition_ids(a_content, &mut archetype_ids, &mut template_ids);

        // Resolve the declared templates first: each one that resolves also
        // vouches for the archetype ids its node tree inlines.
        let mut template_covered = std::collections::BTreeSet::new();
        for id in template_ids {
            match self.web_template_for(&id).await {
                Ok(wt) => collect_template_node_ids(&wt.tree, &mut template_covered),
                Err(ServiceError::Unprocessable { .. } | ServiceError::NotFound(_)) => {
                    return Ok(false);
                }
                Err(other) => return Err(other.into()),
            }
        }
        for id in archetype_ids {
            if template_covered.contains(&id.to_lowercase()) {
                continue;
            }
            if !self.has_archetype(id.clone()).await? && !self.has_artefact(id).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `content_valid` (`i_validity_checker.adoc`): "Return `True` if the
    /// content structure is a valid instance of the relevant RM classes."
    /// Runs the same per-`Kind` structural validation every commit runs; an
    /// unrecognized root `_type` is `false`. A bare validity check has no
    /// lifecycle context → full strictness.
    ///
    /// # Errors
    ///
    /// A validation verdict is never an error (`ValidationFailed` /
    /// `Unprocessable` answer `Ok(false)`); any other service failure from
    /// the validation path (e.g. a template-store/database fault) propagates
    /// as its [`SmError`] mapping.
    pub async fn content_valid(&self, a_content: &Value) -> Result<bool, SmError> {
        let rm_type = a_content.get("_type").and_then(Value::as_str).unwrap_or("");
        let Some(kind) = Kind::from_type(rm_type) else {
            return Ok(false);
        };
        Ok(self.commit_rejection(kind, a_content).await?.is_none())
    }

    /// The diagnostics-bearing sibling of [`Self::content_valid`]: the same
    /// commit-path validation, answering `None` for valid content and
    /// `Some(rejection)` with the refusal text VERBATIM — the seam a dry-run
    /// caller (the FHIR `$validate` door) previews the commit's own verdict
    /// through. Full strictness, exactly like a bare validity check.
    ///
    /// # Errors
    /// A validation verdict is never an error; any other service failure from
    /// the validation path (e.g. a template-store/database fault) propagates
    /// as its [`SmError`] mapping.
    pub(crate) async fn commit_rejection(
        &self,
        a_kind: Kind,
        a_content: &Value,
    ) -> Result<Option<String>, SmError> {
        match self.validate_for_commit(a_kind, a_content, false).await {
            Ok(()) => Ok(None),
            // Rendered through the SM bridge — where the per-path violation
            // list joins into one message — so the text matches what the
            // committing routes serve, verbatim.
            Err(
                rejection
                @ (ServiceError::ValidationFailed(_) | ServiceError::Unprocessable { .. }),
            ) => Ok(Some(SmError::from(rejection).message)),
            Err(other) => Err(other.into()),
        }
    }
}

/// Collects, lowercased, every node id a resolved template's tree carries, so
/// archetype ids the operational template inlines resolve through it.
fn collect_template_node_ids(
    node: &openehr_sdt::flat::webtemplate::model::WebTemplateNode,
    covered: &mut std::collections::BTreeSet<String>,
) {
    if let Some(node_id) = &node.node_id {
        covered.insert(node_id.to_lowercase());
    }
    for child in &node.children {
        collect_template_node_ids(child, covered);
    }
}

/// Collects every definition identifier an `ARCHETYPED` declaration in the
/// content carries: `archetype_details/archetype_id/value` and
/// `archetype_details/template_id/value`, at any depth (RM common
/// `master03-archetyped_package.adoc` §ARCHETYPED Class — every archetype
/// root node carries one).
fn collect_definition_ids(
    value: &Value,
    archetype_ids: &mut std::collections::BTreeSet<String>,
    template_ids: &mut std::collections::BTreeSet<String>,
) {
    match value {
        Value::Object(map) => {
            if let Some(details) = map.get("archetype_details") {
                if let Some(id) = details
                    .pointer("/archetype_id/value")
                    .and_then(Value::as_str)
                {
                    archetype_ids.insert(id.to_owned());
                }
                if let Some(id) = details
                    .pointer("/template_id/value")
                    .and_then(Value::as_str)
                {
                    template_ids.insert(id.to_owned());
                }
            }
            for child in map.values() {
                collect_definition_ids(child, archetype_ids, template_ids);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_definition_ids(child, archetype_ids, template_ids);
            }
        }
        _ => {}
    }
}
