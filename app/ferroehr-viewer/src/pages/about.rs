// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! About: the viewer, the CDR it reads and the manufacturer, and the
//! conformance manifest the CDR serves at `OPTIONS` on its base path, drawn
//! with FerroUI's About template (`view::template::about`).
//!
//! The manifest is the System API's `Options` document
//! (`docs/specs/openehr/ITS-REST/specifications/schemas/system/Options.yaml`),
//! shown as the CDR sent it; its Download reads the same document from
//! [`DOWNLOAD_ROUTE`]. No openEHR spec governs the viewer: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::view::surface::Header;
use ferrohealth_ui::view::template::about::{
    AboutPage as Page, Link, Manufacturer, SelfDescription, about_page,
};
use leptos::prelude::*;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{Refused, drawn, page_model};

/// The route the manifest's Download reads the document from.
pub const DOWNLOAD_ROUTE: &str = "/about/manifest.json";

/// The FerroEHR book, the viewer's documentation.
const BOOK: &str = "https://ferroehr.eu/docs/latest/";

/// About's model: the versions it names, the release, the manufacturer's
/// address and the manifest as the CDR sent it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Model {
    /// The CDR's version.
    cdr_version: String,
    /// The REST API release the manifest names.
    rest: String,
    /// The viewer's product and version.
    release: String,
    /// The manufacturer's support address.
    support: String,
    /// The manifest pretty-printed, or the CDR's refusal of it.
    manifest: Result<String, Refused>,
}

/// Loads About: the status the shell read and the conformance manifest.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load(
    /// The path and query of the page.
    href: String,
) -> Result<Model, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_page().await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load`] reads, on the server.
async fn read_page() -> Model {
    let status = crate::shell::status().await;
    let document = match crate::request::state() {
        Some(state) => crate::system_api::fetch_conformance_document(&state).await,
        None => Err(ViewerError::Internal(tr(Msg::AboutNoState))),
    };
    // NOTE: no openEHR spec governs this: our own design; a manifest that does
    // not parse leaves its versions unknown, and the card shows it as sent.
    let manifest = document
        .as_deref()
        .ok()
        .and_then(|text| serde_json::from_str::<crate::system_api::ConformanceManifest>(text).ok());
    let cdr_version = status
        .ok()
        .and_then(|body| crate::deployment::StatusDocument::parse(&body))
        .and_then(|document| document.server_version)
        .unwrap_or_else(|| tr(Msg::UnknownVersion));
    let rest = manifest
        .as_ref()
        .map(|manifest| manifest.restapi_specs_version.clone())
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| tr(Msg::UnknownVersion));
    Model {
        cdr_version,
        rest,
        release: crate::shell::footer().release(),
        support: crate::manufacturer::MANUFACTURER.email.to_owned(),
        manifest: document
            .map(|text| crate::bff::pretty_body(&text, crate::format::ReprFormat::CanonicalJson))
            .map_err(|error| Refused::new(tr(Msg::AboutManifestRefused), error)),
    }
}

/// About, drawn from its model.
fn about(model: Model) -> AnyView {
    let Model {
        cdr_version,
        rest,
        release,
        support,
        manifest,
    } = model;
    let versions = vec![
        (
            tr(Msg::AboutCdr),
            tr_with(Msg::AboutCdrVersion, &[("version", cdr_version.into())]).into_any(),
        ),
        (
            tr(Msg::AboutOpenEhrRest),
            tr_with(Msg::AboutRestVersion, &[("version", rest.into())]).into_any(),
        ),
        (
            tr(Msg::AboutFerroUi),
            tr_with(
                Msg::AboutFerroUiVersion,
                &[("version", ferrohealth_ui_version().into())],
            )
            .into_any(),
        ),
    ];
    let (document, cards) = match manifest {
        Ok(text) => (Some(text), Vec::new()),
        Err(refusal) => (None, vec![refusal.view()]),
    };
    let self_description = document.map(|text| SelfDescription {
        title: tr(Msg::AboutManifest),
        line: tr(Msg::AboutManifestLine),
        language: Language::Json,
        text,
        copy_id: "manifest-copy".to_owned(),
        download: DOWNLOAD_ROUTE.to_owned(),
    });
    about_page(Page {
        header: Header {
            title: tr(Msg::NavAbout).into(),
            subtitle: Some(tr(Msg::AboutSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        release,
        versions,
        manufacturer: Manufacturer {
            licence: tr(Msg::AboutLicence),
            support: Link {
                label: support.clone(),
                href: format!("mailto:{support}"),
            },
            documentation: Some(Link {
                label: tr(Msg::AboutBook),
                href: BOOK.to_owned(),
            }),
        },
        cards,
        self_description,
        replica: None,
    })
}

/// About.
#[component]
pub fn AboutPage() -> impl IntoView {
    let model = page_model!(load());
    let body = drawn(model, about);
    view! {
        <Title text=tr(Msg::NavAbout) />
        {body}
    }
}

/// The FerroUI release the viewer draws with: the exact pin of the
/// workspace manifest, held to it by a test.
const FERROHEALTH_UI: &str = "0.5.1";

/// The FerroUI release the viewer draws with.
fn ferrohealth_ui_version() -> String {
    FERROHEALTH_UI.to_owned()
}

#[cfg(test)]
mod tests {
    use super::FERROHEALTH_UI;

    #[test]
    fn about_names_the_ferroui_release_the_workspace_pins() {
        let manifest = include_str!("../../../../Cargo.toml");
        let line = manifest
            .lines()
            .find(|line| line.starts_with("ferrohealth-ui = "))
            .expect("the workspace pins ferrohealth-ui");
        let pin = format!("\"={FERROHEALTH_UI}\"");
        assert!(line.contains(&pin), "the workspace pins {pin}: {line}");
    }
}
