// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/templates/adl2/{template_id}` screen — ADL2 template detail.
//!
//! Four panes over one stored ADL2 operational template. Three of them are
//! what the ITS-REST Definition API serves for the resource: **Source** (the
//! stored artefact verbatim, `Accept: text/plain`), **AOM2 JSON** (the
//! `OperationalTemplateV2` canonical-JSON projection,
//! `Accept: application/json`), and **Example** (the CDR-generated example
//! composition, negotiated across the four `Accept_LOCATABLE` forms). A
//! version bar drives the versioned get
//! (`definition/template/adl2/{template_id}/{version}`) as `?version=` URL
//! state, so a pinned view is shareable.
//!
//! The fourth, **Path catalog**, is built viewer-side: the ADL2 resource
//! serves no Web Template representation, so the BFF reads the same
//! `application/json` body, parses the AOM2 operational template, builds its
//! Web Template and projects the [`crate::builder::catalog::CatalogNode`] tree
//! the ADL 1.4 detail screen renders — same tree component, same node
//! inspector, one code path for both families.
//!
//! No openEHR spec governs the viewer — our own design / product extension;
//! the wire it reads is the ITS-REST Definition API.
//!
//! Absence (`404`) is `Ok(None)`, a first-class state rather than an error;
//! tab and version state live in the URL.


use crate::builder::catalog::CatalogNode;
use crate::error::ViewerError;
use crate::example_options::{ExampleDetail, ExampleType};
use crate::format::ReprFormat;
use crate::locale::definitions::DefinitionsMsg;
use crate::locale::tr_with;

/// Fetch the stored ADL2 operational-template SOURCE.
///
/// GET `definition/template/adl2/{template_id}` (or
/// `…/{template_id}/{version}` when `version` is given) with
/// `Accept: text/plain` — the artefact interchange form the resource serves
/// verbatim. `version` is an exact SEMVER or a `{major}` / `{major}.{minor}`
/// prefix resolving to the highest match; an unknown template or version is a
/// `404`, returned as `Ok(None)` because absence is a state this screen
/// renders, not a failure.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
pub async fn fetch_adl2_source(
    // The ADL2 template to read (a full HRID, or a partial that resolves to
    // the latest matching version).
    template_id: String,
    // The release version to pin the read to, or `None` for the artefact
    // `template_id` itself resolves to.
    version: Option<String>,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1(&crate::adl2::template_path(
        &template_id,
        version.as_deref(),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "text/plain")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(crate::cdr::CdrClient::expect_success(response)?.body))
}

/// Fetch the `OperationalTemplateV2` canonical JSON of a stored ADL2 template.
///
/// The same resource as [`fetch_adl2_source`] under
/// `Accept: application/json`, which the CDR answers with the AOM2
/// canonical-JSON projection of the operational template. `404` → `Ok(None)`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
pub async fn fetch_adl2_json(
    // The ADL2 template to read.
    template_id: String,
    // The release version to pin the read to, or `None`.
    version: Option<String>,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1(&crate::adl2::template_path(
        &template_id,
        version.as_deref(),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(crate::cdr::CdrClient::expect_success(response)?.body))
}

/// Build the Web-Template path catalog of one `OperationalTemplateV2`
/// canonical-JSON body.
///
/// The body is read with [`openehr_its::json::from_canonical_json`] into the
/// AOM2
/// [`OperationalTemplate`](openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate)
/// — the exact type the CDR serialized — then
/// [`openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4`]
/// produces the Web Template and [`crate::builder::catalog::from_web_template`]
/// the slim tree the views render. Both stages are named in the error.
/// # Errors
/// [`ViewerError::Internal`] when the AOM2 JSON fails to parse or the Web
/// Template fails to build (the diagnostic named, never a panic).
pub fn adl2_catalog_from_json(json: &str) -> Result<CatalogNode, ViewerError> {
    let opt = openehr_its::json::from_canonical_json::<
        openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate,
    >(json)
    .map_err(|e| {
        ViewerError::Internal(tr_with(
            DefinitionsMsg::Operationaltemplatev2Parse,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let web_template = openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4(&opt)
        .map_err(|e| {
            ViewerError::Internal(tr_with(
                DefinitionsMsg::WebtemplateBuild,
                &[("value0", (crate::error::error_chain(&e)).clone().into())],
            ))
        })?;
    Ok(crate::builder::catalog::from_web_template(&web_template))
}

/// Fetch the Web-Template path catalog of a stored ADL2 template.
///
/// The same resource and `Accept` as [`fetch_adl2_json`]: the ADL2 template
/// GET's `application/json` 200 body IS the `OperationalTemplateV2` canonical
/// form (`docs/specs/openehr/ITS-REST/specifications/responses/
/// 200_Template_adl2_retrieved.yaml`). The catalog is built viewer-side by
/// [`adl2_catalog_from_json`], because the resource serves no `wt+json`
/// representation. `404` → `Ok(None)`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR;
/// [`ViewerError::Internal`] when the served JSON fails to parse or its Web
/// Template fails to build.
pub async fn fetch_adl2_catalog(
    // The ADL2 template to build the path catalog from.
    template_id: String,
    // The release version to pin the read to, or `None`.
    version: Option<String>,
) -> Result<Option<CatalogNode>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1(&crate::adl2::template_path(
        &template_id,
        version.as_deref(),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    adl2_catalog_from_json(&body).map(Some)
}

/// Fetch the CDR-generated example composition for a stored ADL2 template, in
/// `format`.
///
/// GET `definition/template/adl2/{template_id}/example` with `Accept` set to
/// the selected representation's media type, and the operator's `detail_level`
/// and `type` on the query string
/// (`docs/specs/openehr/ITS-REST/specifications/operations/definition_template_adl2_example_get.yaml`).
/// The Definition API declares no VERSIONED example resource, so the example is
/// generated from the artefact `template_id` itself resolves to.
/// `404` → `Ok(None)`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR (a template that cannot be
/// compiled into a Web Template answers `422`).
pub async fn fetch_adl2_example(
    // The ADL2 template to generate an example composition for.
    template_id: String,
    // Which representation to negotiate for the example.
    format: ReprFormat,
    // How much of the template the example fills in (`detail_level`).
    detail: ExampleDetail,
    // Which form the example is shaped for (`type`).
    kind: ExampleType,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state
        .cdr
        .rest_v1(&crate::adl2::example_path(&template_id, detail, kind));
    let response = state
        .cdr
        .get(&session.credential, &url, format.media_type())
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(crate::cdr::CdrClient::expect_success(response)?.body))
}
