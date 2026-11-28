// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What one ADL 1.4 template's page reads from the CDR, tab by tab.

use crate::adl2::TemplateFamily;
use crate::bff::archetypes::{Conversion, convert_template};
use crate::bff::template_detail::{TemplateDetail, fetch_example, fetch_template_detail};
use crate::bff::templates::list_templates;
use crate::error::ViewerError;
use crate::pages::template_releases::{CompareAsk, Releases, read_releases};

use super::{ExampleAsk, TemplateTab, delete_posted};

/// What one ADL 1.4 template's page read.
#[derive(Debug, Clone)]
pub(super) struct Template {
    /// The CDR's refusal of the delete posted.
    pub(super) acted: Option<ViewerError>,
    /// The template, carrying only the parts its tab draws.
    pub(super) detail: Result<TemplateDetail, ViewerError>,
    /// The template's download address.
    pub(super) download: String,
    /// Its releases, with what the tab draws of them.
    pub(super) releases: Releases,
    /// The example the CDR built, on the Example tab of a root it builds
    /// examples for.
    pub(super) example: Option<Result<Option<String>, ViewerError>>,
    /// The template's ADL 2 conversion, on its ADL 2 tab.
    pub(super) converted: Option<Result<Conversion, ViewerError>>,
}

/// Answers a delete posted to the ADL 1.4 template `template_id` and reads
/// its releases and what its tab `tab` draws, the example as `ask` names it.
pub(super) async fn read_template(
    // The template's id.
    template_id: String,
    // The tab shown.
    tab: TemplateTab,
    // The example options the address names.
    ask: ExampleAsk,
) -> Template {
    let acted = delete_posted("/templates").await;
    let download = crate::download::template_href(TemplateFamily::Adl14, &template_id);
    let mut detail = fetch_template_detail(template_id.clone()).await;
    let releases = read_releases(
        TemplateFamily::Adl14,
        list_templates().await,
        &template_id,
        tab,
        CompareAsk::asked(),
    )
    .await;
    let mut example = None;
    let mut converted = None;
    match &mut detail {
        Err(error) => {
            if crate::pages::common::is_status(error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
        }
        Ok(detail) => {
            // NOTE: no openEHR spec governs this: our own design; the model keeps
            // only what the tab draws, so the page carries no unshown source.
            if tab != TemplateTab::Source {
                detail.source = String::new();
            }
            if tab != TemplateTab::Paths {
                detail.catalog.children = Vec::new();
            }
            match tab {
                TemplateTab::Example => {
                    let formats = crate::example_options::example_formats(&detail.catalog.rm_type);
                    if !formats.is_empty() {
                        let format = ask.format(formats);
                        example = Some(
                            fetch_example(template_id.clone(), format, ask.detail, ask.kind)
                                .await
                                .map(|text| Some(crate::bff::pretty_body(&text, format))),
                        );
                    }
                }
                TemplateTab::Converted => {
                    converted = Some(convert_template(&template_id).await);
                }
                TemplateTab::Overview
                | TemplateTab::Paths
                | TemplateTab::Source
                | TemplateTab::Json
                | TemplateTab::Compare => {}
            }
        }
    }
    Template {
        acted,
        detail,
        download,
        releases,
        example,
        converted,
    }
}
