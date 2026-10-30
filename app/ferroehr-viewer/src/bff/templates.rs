// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/templates` screen — the Template Manager list + template upload.
//!
//! One screen, two template FAMILIES, switched by `?family=` (the switch is
//! deep-linkable URL state, not a private signal): ADL 1.4 operational
//! templates (`definition/template/adl1.4`, OPT/XML upload) and ADL2 ones
//! (`definition/template/adl2`, `text/plain` ADL2 source upload). Each family
//! has its own list, its own upload affordance, and its own detail route; the
//! client-side text filter and the URL page window serve both. No openEHR spec
//! governs the viewer — our own design / product extension; the wire it
//! reads/writes is the ITS-REST Definition API.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use crate::adl2::TemplateFamily;
use crate::error::ViewerError;
use crate::locale::{Msg, tr_served, tr_served_with};

/// Which template a delete addresses — the id, and the family that decides
/// WHICH wire route removes it.
///
/// The family travels with the id rather than being read off the URL at
/// confirm time: the row that opened the dialog is the one that knows its own
/// family, so the confirmation copy and the dispatched route can never
/// disagree with the row the reader clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    /// The archetype-model family the template belongs to.
    pub family: TemplateFamily,
    /// The `template_id` (ADL 1.4) or artefact HRID (ADL2) to remove.
    pub template_id: String,
}

/// One row of the template list, distilled from the ITS-REST Definition list
/// shape (`template_id` / `concept` / `created_timestamp`).
///
/// Shared across both compilation targets, so it carries only fixed-size,
/// client-safe fields (no `usize` in serialized types).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TemplateRow {
    /// The operational-template id (the detail route segment).
    pub template_id: String,
    /// The template concept / display name.
    pub concept: String,
    /// The root archetype id the template constrains.
    pub archetype_id: String,
    /// The creation timestamp as the CDR reported it (raw string).
    pub created: String,
}

impl TemplateRow {
    /// Whether the template describes a COMPOSITION, the one class a
    /// composition editor commits and the query builder reaches.
    ///
    /// The root archetype id names its RM class in its `rm_entity` part
    /// (`openEHR-EHR-COMPOSITION.encounter.v1`; BASE `master05-identification_package.adoc`
    /// §Archetype identifier: `rm_originator-rm_name-rm_entity`, compared
    /// case-insensitively). A demographic template
    /// (`openEHR-DEMOGRAPHIC-ROLE.Role.v1`) describes a party instead. A row
    /// whose root the CDR did not report is kept, since nothing says what it
    /// describes.
    #[must_use]
    pub fn is_composition(&self) -> bool {
        root_is_composition(&self.archetype_id)
    }
}

/// Whether `archetype_id` names a COMPOSITION root (see
/// [`TemplateRow::is_composition`]); an empty id is not known not to.
fn root_is_composition(archetype_id: &str) -> bool {
    if archetype_id.trim().is_empty() {
        return true;
    }
    archetype_id
        .split('.')
        .next()
        .and_then(|qualified| qualified.splitn(3, '-').nth(2))
        .is_some_and(|entity| entity.eq_ignore_ascii_case("COMPOSITION"))
}

/// List the CDR's ADL 1.4 operational templates.
///
/// GET `definition/template/adl1.4?version=*` with
/// `Accept: application/json`; the body is an array whose elements are parsed
/// defensively (missing fields become empty strings) into [`TemplateRow`]s.
/// `version=*` pins the FULL inventory: an absent `version` collapses the
/// CDR's listing to the latest version of each template (the released OAS
/// `parameters/query/filter_version.yaml`), and a management client must
/// show every stored version, not just the latest.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the CDR body is not valid JSON.
pub async fn list_templates() -> Result<Vec<TemplateRow>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1("definition/template/adl1.4?version=*");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::TemplateListJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let rows = value
        .as_array()
        .map(|items| items.iter().map(template_row).collect())
        .unwrap_or_default();
    Ok(rows)
}

/// Distil one Definition-list element into a [`TemplateRow`], reading each
/// field defensively so a missing/renamed field yields an empty string rather
/// than dropping the row.
fn template_row(item: &serde_json::Value) -> TemplateRow {
    let text = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    TemplateRow {
        template_id: text("template_id"),
        concept: text("concept"),
        archetype_id: text("archetype_id"),
        created: text("created_timestamp"),
    }
}

/// Upload an operational template (OPT/XML) to the CDR.
///
/// POST `definition/template/adl1.4` with `Content-Type: application/xml`,
/// `Accept: application/json`, `Prefer: return=minimal`. The CDR's 400/409/422
/// validation diagnostics surface verbatim through
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success). On
/// success the (possibly empty) response body is returned; the UI ignores it
/// and refetches the list.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for empty content;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR (the diagnostic verbatim).
pub async fn upload_template(
    // The operational-template XML to upload, as read in the browser.
    opt_xml: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    if opt_xml.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::SelectedFileWasEmpty)));
    }
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1("definition/template/adl1.4");
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/xml",
            "application/json",
            &[("Prefer", "return=minimal")],
            opt_xml,
        )
        .await?;
    Ok(crate::cdr::CdrClient::expect_success(response)?.body)
}

/// List the CDR's ADL2 operational templates.
///
/// GET `definition/template/adl2?version=*` with
/// `Accept: application/json`. The row shape is the same four fields the ADL
/// 1.4 listing carries (`template_id` / `concept` / `archetype_id` /
/// `created_timestamp`), so both families share one [`TemplateRow`] and one
/// parser. `version=*` pins the FULL inventory: without it the CDR collapses
/// the listing to the latest version of each HRID family (the released OAS
/// `parameters/query/filter_version.yaml`), which would hide every superseded
/// ADL2 artefact from a management client.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the CDR body is not valid JSON.
pub async fn list_adl2_templates() -> Result<Vec<TemplateRow>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1("definition/template/adl2?version=*");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::Adl2TemplateListJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let rows = value
        .as_array()
        .map(|items| items.iter().map(template_row).collect())
        .unwrap_or_default();
    Ok(rows)
}

/// Upload an ADL2 operational template (the artefact SOURCE) to the CDR.
///
/// POST `definition/template/adl2` with `Content-Type: text/plain` — the
/// operation's single declared body type — `Accept: application/json` and
/// `Prefer: return=identifier`, so the `201` echoes
/// `{"template_id": "<resolved HRID>"}` and the success toast can name the
/// artefact the CDR actually stored. The `openehr-adl` engine's refusals
/// (`400` unparseable, `422` AOM2-invalid with the rule codes, `409` duplicate
/// HRID) surface verbatim through
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for empty content;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR (the diagnostic verbatim).
pub async fn upload_adl2_template(
    // The ADL2 operational-template source, pasted or read from a file.
    adl2_source: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    if adl2_source.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            Msg::ThereNoAdl2SourceUpload,
        )));
    }
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1("definition/template/adl2");
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "text/plain",
            "application/json",
            &[("Prefer", "return=identifier")],
            adl2_source,
        )
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    Ok(accepted_template_id(&body))
}

/// The `template_id` a `Prefer: return=identifier` upload answer echoes
/// (`{"template_id": …}`), or the trimmed body when the CDR echoed something
/// else — the upload SUCCEEDED either way, so an unexpected body shape is
/// reported as-is rather than turned into a failure.
fn accepted_template_id(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(id) = value.get("template_id").and_then(serde_json::Value::as_str)
    {
        return id.to_owned();
    }
    body.trim().to_owned()
}

/// Delete one stored ADL2 artefact
/// (`DELETE definition/artefact/adl2/{artefact_id}`).
///
/// The ADL2 store's templates are removed through the ARTEFACT resource, not
/// through the Admin API's `admin/template/{template_id}` (which addresses the
/// ADL 1.4 store alone): the route is the CDR's own extension realizing SM
/// `I_DEFINITION_ADL2.delete_artefact`
/// (`docs/specs/openehr/SM/docs/UML/classes/i_definition_adl2.adoc`), whose
/// `artefact_does_not_exist` error is the `404` — which is also the answer for
/// a malformed HRID, since the store key is opaque. A template-kind artefact
/// that committed versions still reference is refused `409` with the
/// referencing count in the diagnostic, and that text surfaces verbatim
/// through [`delete_failure_copy`](crate::admin::delete_failure_copy).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty id;
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
pub async fn delete_adl2_artefact(
    // The AOM2 artefact HRID to remove.
    artefact_id: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session()?;
    if artefact_id.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::NoAdl2ArtefactIdDelete)));
    }
    let state = crate::scope::state()?;
    let url = state
        .cdr
        .extensions_v1(&crate::adl2::artefact_path(&artefact_id));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_composition_root_is_a_composition_template() {
        use super::root_is_composition;
        assert!(root_is_composition("openEHR-EHR-COMPOSITION.encounter.v1"));
        assert!(root_is_composition("openEHR-EHR-composition.minimal.v1"));
        assert!(!root_is_composition("openEHR-DEMOGRAPHIC-ROLE.Role.v1"));
        assert!(!root_is_composition("openEHR-DEMOGRAPHIC-PERSON.person.v2"));
        assert!(!root_is_composition(
            "openEHR-EHR-OBSERVATION.blood_pressure.v2"
        ));
        assert!(root_is_composition(""), "an unreported root is kept");
    }

    /// Both families' listings are read through this one parser, so it is
    /// pinned against the four fields the Definition API returns.
    #[test]
    fn a_list_element_distils_into_a_row_and_missing_fields_stay_empty() {
        let item = serde_json::json!({
            "template_id": "openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0",
            "concept": "cnf_vitals",
            "archetype_id": "openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0",
            "created_timestamp": "2026-08-22T14:51:32.596215Z",
        });
        let row = crate::bff::templates::template_row(&item);
        assert_eq!(row.template_id, "openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0");
        assert_eq!(row.concept, "cnf_vitals");
        assert_eq!(row.created, "2026-08-22T14:51:32.596215Z");
        // A renamed or absent field empties that cell rather than dropping the
        // whole row from the listing.
        let sparse = serde_json::json!({ "template_id": "only-the-id" });
        let row = crate::bff::templates::template_row(&sparse);
        assert_eq!(row.template_id, "only-the-id");
        assert_eq!(row.concept, "");
        assert_eq!(row.archetype_id, "");
        assert_eq!(row.created, "");
    }

    #[test]
    fn the_upload_answer_yields_the_echoed_template_id() {
        let body = r#"{"template_id":"openEHR-EHR-COMPOSITION.cnf_minimal.v1.0.0"}"#;
        assert_eq!(
            crate::bff::templates::accepted_template_id(body),
            "openEHR-EHR-COMPOSITION.cnf_minimal.v1.0.0"
        );
        // A body that is not the identifier object is reported as-is: the
        // upload still succeeded.
        assert_eq!(
            crate::bff::templates::accepted_template_id("  not json  "),
            "not json"
        );
        assert_eq!(crate::bff::templates::accepted_template_id("{}"), "{}");
    }
}
