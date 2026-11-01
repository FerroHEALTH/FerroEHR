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
use ferrohealth_ui::html::Html;
use ferrohealth_ui::view::surface::Header;
use ferrohealth_ui::view::template::about::{
    AboutPage as Page, Link, Manufacturer, SelfDescription, about_page,
};

use crate::error::ViewerError;
use crate::locale::console::ConsoleMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::t;
use crate::pages::common::{Refused, drawn};
use crate::routes::Drawn;

/// The route the manifest's Download reads the document from.
pub const DOWNLOAD_ROUTE: &str = "/about/manifest.json";

/// The FerroEHR book, the viewer's documentation.
const BOOK: &str = "https://ferroehr.eu/docs/latest/";

/// About's model: the versions it names, the release, the manufacturer's
/// address and the manifest as the CDR sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)).
async fn load() -> Result<Model, ViewerError> {
    crate::session::require_session()?;
    Ok(read_page().await)
}

/// What [`load`] reads.
async fn read_page() -> Model {
    let status = crate::shell::status().await;
    let document = match crate::request::state() {
        Some(state) => crate::system_api::fetch_conformance_document(&state).await,
        None => Err(ViewerError::Internal(tr(ConsoleMsg::AboutNoState))),
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
            .map_err(|error| Refused::new(tr(ConsoleMsg::AboutManifestRefused), error)),
    }
}

/// About, drawn from its model.
fn about(model: Model) -> Html {
    let Model {
        cdr_version,
        rest,
        release,
        support,
        manifest,
    } = model;
    let versions = vec![
        (
            tr(ConsoleMsg::AboutCdr),
            t(tr_with(
                ConsoleMsg::AboutCdrVersion,
                &[("version", cdr_version.into())],
            )),
        ),
        (
            tr(ConsoleMsg::AboutOpenEhrRest),
            t(tr_with(
                ConsoleMsg::AboutRestVersion,
                &[("version", rest.into())],
            )),
        ),
        (
            tr(ConsoleMsg::AboutFerroUi),
            t(tr_with(
                ConsoleMsg::AboutFerroUiVersion,
                &[("version", ferrohealth_ui_version().into())],
            )),
        ),
    ];
    let (document, cards) = match manifest {
        Ok(text) => (Some(text), Vec::new()),
        Err(refusal) => (None, vec![refusal.view()]),
    };
    let self_description = document.map(|text| SelfDescription {
        title: tr(ConsoleMsg::AboutManifest),
        line: tr(ConsoleMsg::AboutManifestLine),
        language: Language::Json,
        text,
        copy_id: "manifest-copy".to_owned(),
        download: DOWNLOAD_ROUTE.to_owned(),
    });
    about_page(
        &current(),
        Page {
            header: Header {
                title: tr(Msg::NavAbout).into(),
                subtitle: Some(tr(ConsoleMsg::AboutSubtitle).into()),
                crumbs: Vec::new(),
                mono: false,
                badge: None,
                subtitle_view: None,
            },
            release,
            versions,
            manufacturer: Manufacturer {
                licence: tr(ConsoleMsg::AboutLicence),
                support: Link {
                    label: support.clone(),
                    href: format!("mailto:{support}"),
                },
                documentation: Some(Link {
                    label: tr(ConsoleMsg::AboutBook),
                    href: BOOK.to_owned(),
                }),
            },
            cards,
            self_description,
            replica: None,
        },
    )
}

/// About.
pub async fn page() -> Drawn {
    Drawn::page(tr(Msg::NavAbout), drawn(load().await, about))
}

/// The FerroUI release the viewer draws with: the exact pin of the
/// workspace manifest, held to it by a test.
const FERROHEALTH_UI: &str = "0.6.0";

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
