// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/templates/{template_id}` screen — template detail: OPT/WT/example tabs +
//! path catalog.
//!
//! Three tabs over one operational template: **WT** (the Web Template path
//! catalog as an expandable tree + a node inspector), **OPT** (the raw
//! canonical-XML operational template), and **Example** (the CDR-generated
//! example composition, switchable by representation, detail level and example
//! form). No openEHR spec governs the viewer — our own design / product
//! extension; the `WebTemplate` shape it renders
//! is `openehr_sdt::flat`'s (built from the CDR's OPT), per the ITS-REST
//! Simplified Formats spec (`master04`).
//!
//! The WT catalog, the OPT source pane and the identity card are three views
//! of ONE document, so the screen reads the operational template exactly once
//! per page: one [`fetch_template_detail`], shared by every pane. The
//! Example tab reads its own on that tab alone: it is a different CDR
//! resource whose fetch runs the CDR's example generator.

use crate::builder::catalog::CatalogNode;
use crate::error::ViewerError;
use crate::example_options::{ExampleDetail, ExampleType};
use crate::format::ReprFormat;
use crate::locale::{Msg, tr_with};

/// Template identity + language metadata for the detail header card,
/// combined from the typed OPT (uid, concept, original language) and its
/// Web Template (version, default + available languages).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TemplateMeta {
    /// The operational-template id.
    pub template_id: String,
    /// The template concept / display name.
    pub concept: String,
    /// The OPT `uid` (empty when the template carries none).
    pub uid: String,
    /// The OPT's original language code.
    pub language: String,
    /// The Web Template `version`.
    pub version: String,
    /// Every language the template carries terms for.
    pub languages: Vec<String>,
}

/// Everything the template-detail screen shows about one operational template,
/// distilled from a SINGLE fetch of its OPT source.
///
/// The three panes of the screen are three views of the same document — the raw
/// source, its identity metadata, and its Web Template path catalog — so they
/// travel together and the screen reads the CDR once.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TemplateDetail {
    /// The raw OPT 1.4 canonical XML the CDR served, verbatim.
    pub source: String,
    /// The identity/metadata card fields.
    pub meta: TemplateMeta,
    /// The Web Template path catalog (the same [`CatalogNode`] tree the Query
    /// Builder navigates).
    pub catalog: CatalogNode,
}

/// Distil the fetched OPT source into the screen's three panes: one parse, one
/// Web Template build, both views derived from them.
///
/// The XML is parsed with [`openehr_its::opt14::from_xml`] — the OPT 1.4
/// canonical-XML parse entry (root `<template>` = `OPERATIONAL_TEMPLATE`) —
/// then [`openehr_sdt::flat::webtemplate::builder::build_web_template`]
/// produces the Web Template, and
/// [`crate::builder::catalog::from_web_template`] the slim serializable tree.
///
/// # Errors
/// [`ViewerError::Internal`] when the OPT fails to parse or the Web Template
/// fails to build (the diagnostic named, never a panic).
pub fn template_detail_from_opt(source: String) -> Result<TemplateDetail, ViewerError> {
    let opt = openehr_its::opt14::from_xml(&source).map_err(|e| {
        ViewerError::Internal(tr_with(Msg::Opt14Parse, &[("e", (e).to_string().into())]))
    })?;
    let web_template =
        openehr_sdt::flat::webtemplate::builder::build_web_template(&opt).map_err(|e| {
            ViewerError::Internal(tr_with(
                Msg::WebtemplateBuild2,
                &[("value0", (crate::error::error_chain(&e)).clone().into())],
            ))
        })?;
    Ok(TemplateDetail {
        meta: TemplateMeta {
            template_id: opt.template_id.value.clone(),
            concept: opt.concept.clone(),
            uid: opt
                .uid
                .as_ref()
                .map(|u| u.value().to_owned())
                .unwrap_or_default(),
            language: opt.language.code_string.clone(),
            version: web_template.version.clone(),
            languages: web_template.languages.clone(),
        },
        catalog: crate::builder::catalog::from_web_template(&web_template),
        source,
    })
}

/// Fetch the operational template ONCE and distil everything the detail screen
/// shows from that one document.
///
/// GET `definition/template/adl1.4/{template_id}` with
/// `Accept: application/xml`; the `template_id` path segment is percent-encoded
/// server-side. The OPT is parsed exactly once, by
/// [`template_detail_from_opt`].
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Cdr`] (e.g. `404` for an unknown template) /
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR;
/// [`ViewerError::Internal`] when the OPT fails to parse or the Web Template
/// fails to build.
pub async fn fetch_template_detail(
    // The template whose OPT to read.
    template_id: String,
) -> Result<TemplateDetail, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1(&format!(
        "definition/template/adl1.4/{}",
        urlencoding::encode(&template_id)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/xml")
        .await?;
    template_detail_from_opt(crate::cdr::CdrClient::expect_success(response)?.body)
}

/// Fetch a template's Web Template path catalog alone — the Query Builder's
/// reader, which navigates paths and has no use for the OPT source or the
/// identity card.
///
/// It rides [`fetch_template_detail`]'s single fetch + parse pipeline and
/// returns only the catalog, so the builder's wire payload stays the tree
/// rather than the whole operational template.
///
/// # Errors
/// As [`fetch_template_detail`].
pub async fn fetch_template_catalog(
    // The template to build the path catalog from.
    template_id: String,
) -> Result<CatalogNode, ViewerError> {
    Ok(fetch_template_detail(template_id).await?.catalog)
}

/// The ITS-REST path of the ADL 1.4 example resource for `template_id`, at
/// `detail` and `kind`.
///
/// The two example options ride the query string
/// ([`crate::example_options::example_query`]); the id is percent-encoded with
/// the `urlencoding` crate, because an operational-template id is CDR-supplied
/// text.
#[must_use]
pub fn example_path(template_id: &str, detail: ExampleDetail, kind: ExampleType) -> String {
    format!(
        "definition/template/adl1.4/{}/example{}",
        urlencoding::encode(template_id),
        crate::example_options::example_query(detail, kind)
    )
}

/// Fetch the CDR-generated example composition for the template, in `format`.
///
/// GET `definition/template/adl1.4/{template_id}/example` with `Accept` set to
/// the selected representation's media type, and the operator's `detail_level`
/// and `type` on the query string
/// (`docs/specs/openehr/ITS-REST/specifications/operations/definition_template_adl1.4_example_get.yaml`).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
pub async fn fetch_example(
    // The template to generate an example composition for.
    template_id: String,
    // Which representation to negotiate for the example.
    format: ReprFormat,
    // How much of the template the example fills in (`detail_level`).
    detail: ExampleDetail,
    // Which form the example is shaped for (`type`).
    kind: ExampleType,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1(&example_path(&template_id, detail, kind));
    let response = state
        .cdr
        .get(&session.credential, &url, format.media_type())
        .await?;
    Ok(crate::cdr::CdrClient::expect_success(response)?.body)
}

#[cfg(test)]
mod tests {
    use crate::bff::template_detail::example_path;
    use crate::example_options::{ExampleDetail, ExampleType};

    /// The operational template the viewer's own e2e stack is seeded with
    /// (`scripts/ui-e2e.sh`), so the derivation is pinned against the same
    /// document the browser journeys inspect.
    const OPT_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/minimal_evaluation.opt"
    ));

    #[test]
    fn one_parse_yields_the_source_the_meta_and_the_catalog() {
        // The screen's three panes come out of ONE parse: the source travels
        // verbatim, the identity card reads the typed OPT, and the path
        // catalog the Web Template built from it.
        let detail = crate::bff::template_detail::template_detail_from_opt(OPT_FIXTURE.to_owned())
            .expect("the seeded OPT parses");
        assert_eq!(detail.source, OPT_FIXTURE, "the source pane is verbatim");
        assert_eq!(detail.meta.template_id, "minimal_evaluation.en.v1");
        assert_eq!(detail.meta.concept, "Minimal evaluation");
        assert_eq!(detail.meta.uid, "711d7d49-b3c6-4a6a-a6b4-a4bd02fc353d");
        assert_eq!(detail.meta.language, "en");
        assert!(
            detail.meta.languages.contains(&"en".to_owned()),
            "the Web Template's language list carries the original language, got {:?}",
            detail.meta.languages
        );
        assert!(
            !detail.catalog.children.is_empty(),
            "the path catalog carries the template's own nodes"
        );
    }

    #[test]
    fn a_defective_opt_names_the_failing_stage() {
        // A refusal is a named diagnostic, never a panic — the screen renders
        // it inline where the panes would be.
        let error = crate::bff::template_detail::template_detail_from_opt(
            "<template>not an OPT</template>".to_owned(),
        )
        .expect_err("a defective OPT is refused");
        assert!(
            error.to_string().contains("OPT 1.4 parse"),
            "the diagnostic names the stage that failed, got {error}"
        );
    }

    #[test]
    fn the_example_path_carries_the_selected_options() {
        // Both parameters ride every request, defaults included, so the CDR
        // generates exactly what the pane's controls show.
        assert_eq!(
            example_path(
                "minimal_evaluation.en.v1",
                ExampleDetail::default(),
                ExampleType::default()
            ),
            "definition/template/adl1.4/minimal_evaluation.en.v1/example?detail_level=required&type=input"
        );
        assert_eq!(
            example_path(
                "minimal_evaluation.en.v1",
                ExampleDetail::Complete,
                ExampleType::Output
            ),
            "definition/template/adl1.4/minimal_evaluation.en.v1/example?detail_level=complete&type=output"
        );
    }

    #[test]
    fn the_example_path_re_encodes_the_decoded_route_param() {
        // The id arrived percent-decoded from the route, so a reserved
        // character must be escaped again or the request lands elsewhere.
        assert_eq!(
            example_path("a/b", ExampleDetail::Medium, ExampleType::Input),
            "definition/template/adl1.4/a%2Fb/example?detail_level=medium&type=input"
        );
        assert_eq!(
            example_path(
                "temperatur-°C",
                ExampleDetail::default(),
                ExampleType::default()
            ),
            "definition/template/adl1.4/temperatur-%C2%B0C/example?detail_level=required&type=input"
        );
    }
}
