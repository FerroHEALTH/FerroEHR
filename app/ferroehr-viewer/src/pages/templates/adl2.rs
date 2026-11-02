// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What one ADL 2 template's page reads from the CDR, tab by tab.

use crate::adl2::{TemplateFamily, family_versions, split_hrid};
use crate::bff::template_adl2::{
    Compiled, adl2_catalog_from_json, fetch_adl2_compiled, fetch_adl2_example, fetch_adl2_source,
};
use crate::bff::templates::list_adl2_templates;
use crate::builder::catalog::CatalogNode;
use crate::error::ViewerError;
use crate::format::ReprFormat;
use crate::pages::template_releases::{CompareAsk, Releases, read_releases};

use super::{ExampleAsk, TemplateTab, delete_posted};

/// What the tab of an ADL 2 template read.
#[derive(Debug, Clone)]
pub(super) enum Adl2Body {
    /// Its identity, from the list.
    Overview,
    /// Its path catalog.
    Paths(Result<Option<CatalogNode>, ViewerError>),
    /// Its source.
    Source(Result<Option<String>, ViewerError>),
    /// Its operational JSON, pretty-printed.
    Json(Result<Option<String>, ViewerError>),
    /// The example the CDR built, pretty-printed.
    Example(Result<Option<String>, ViewerError>),
    /// Nothing: the tab draws from the operational template, which the CDR
    /// cannot compile while a constituent archetype is missing.
    Incomplete,
}

/// What one ADL 2 template's page read.
#[derive(Debug, Clone)]
pub(super) struct Adl2Template {
    /// The CDR's refusal of the delete posted.
    pub(super) acted: Option<ViewerError>,
    /// The HRID without its release.
    pub(super) stem: String,
    /// The latest release.
    pub(super) latest: Option<String>,
    /// The release read.
    pub(super) release: Option<String>,
    /// The HRID read.
    pub(super) hrid: String,
    /// Its releases, with what the tab draws of them.
    pub(super) releases: Releases,
    /// The tab shown.
    pub(super) tab: TemplateTab,
    /// What the tab read.
    pub(super) body: Adl2Body,
    /// The constituent archetypes the CDR does not hold, by id.
    pub(super) missing: Vec<String>,
    /// The template's download address.
    pub(super) download: String,
}

/// The tabs an ADL 2 template offers: Overview alone for an older release,
/// and Compare when it has `several` releases.
pub(super) fn adl2_tabs(older: bool, several: bool) -> Vec<TemplateTab> {
    let mut tabs = vec![TemplateTab::Overview];
    if !older {
        tabs.extend([
            TemplateTab::Paths,
            TemplateTab::Source,
            TemplateTab::Json,
            TemplateTab::Example,
        ]);
    }
    if several {
        tabs.push(TemplateTab::Compare);
    }
    tabs
}

/// Answers a delete posted to the ADL 2 template `asked_id` and reads the
/// release `asked_release` names, the latest for none, with its releases,
/// the archetypes it lacks and what its tab `tab` draws.
pub(super) async fn read_adl2_template(
    // The template id the address names.
    asked_id: String,
    // The release the address names.
    asked_release: Option<String>,
    // The tab the address names.
    tab: TemplateTab,
    // The example options the address names.
    ask: ExampleAsk,
) -> Adl2Template {
    let acted = delete_posted("/templates?family=adl2").await;
    let (stem, named) = split_hrid(&asked_id);
    let stem = stem.to_owned();
    let listed = list_adl2_templates().await;
    let ids: Vec<String> = listed
        .as_ref()
        .map(|rows| rows.iter().map(|row| row.template_id.clone()).collect())
        .unwrap_or_default();
    let versions = family_versions(&ids, &stem);
    let (latest, several) = (versions.last().cloned(), versions.len() > 1);
    let release = asked_release
        .clone()
        .or_else(|| named.map(str::to_owned))
        .or_else(|| latest.clone());
    let hrid = release
        .as_ref()
        .map_or_else(|| stem.clone(), |release| format!("{stem}.v{release}"));
    let older = release.is_some() && release != latest;
    let tab = if adl2_tabs(older, several).contains(&tab) {
        tab
    } else {
        TemplateTab::Overview
    };
    let releases = read_releases(
        TemplateFamily::Adl2,
        listed,
        &hrid,
        tab,
        CompareAsk::asked(),
    )
    .await;
    let version = release.clone();
    // Every tab asks whether the template compiles, so each can name the
    // archetypes it lacks (our own design: no openEHR spec governs the viewer).
    let compiled = fetch_adl2_compiled(hrid.clone(), version.clone()).await;
    let missing = match &compiled {
        Ok(Compiled::Missing(ids)) => ids.clone(),
        Ok(Compiled::Json(_) | Compiled::Absent) | Err(_) => Vec::new(),
    };
    let body = match tab {
        TemplateTab::Paths | TemplateTab::Json | TemplateTab::Example if !missing.is_empty() => {
            Adl2Body::Incomplete
        }
        TemplateTab::Paths => Adl2Body::Paths(compiled.and_then(|compiled| {
            compiled
                .json()
                .map(|json| adl2_catalog_from_json(&json))
                .transpose()
        })),
        TemplateTab::Source => Adl2Body::Source(fetch_adl2_source(hrid.clone(), version).await),
        TemplateTab::Json => Adl2Body::Json(compiled.map(|compiled| {
            compiled
                .json()
                .map(|json| crate::bff::pretty_body(&json, ReprFormat::CanonicalJson))
        })),
        TemplateTab::Example => {
            let format = ask.format(crate::example_options::CANONICAL_EXAMPLE_FORMATS);
            Adl2Body::Example(
                fetch_adl2_example(hrid.clone(), format, ask.detail, ask.kind)
                    .await
                    .map(|text| text.map(|text| crate::bff::pretty_body(&text, format))),
            )
        }
        TemplateTab::Overview | TemplateTab::Compare => Adl2Body::Overview,
    };
    let download = crate::download::template_href(TemplateFamily::Adl2, &hrid);
    Adl2Template {
        acted,
        stem,
        latest,
        release,
        hrid,
        releases,
        tab,
        body,
        missing,
        download,
    }
}
