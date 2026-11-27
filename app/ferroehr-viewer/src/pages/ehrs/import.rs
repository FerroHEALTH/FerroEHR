// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Import EHR Extract: an extract imported as a new EHR.
//!
//! The extract is chosen, dropped or pasted
//! ([`crate::bff::extract::import_extract`]). A success opens the imported
//! EHR; a refusal stays on the page in the CDR's words, the source as typed. No openEHR spec governs the viewer's pages: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::form::Field;

use crate::bff::extract::import_extract;
use crate::flash::{Done, redirect_with};
use crate::locale::operations::OperationsMsg;
use crate::locale::tr;
use crate::markup::{h, opt, t};
use crate::pages::common::{card_panel, plain_section, refused, shortened};
use crate::pages::ehr::ehr_href;
use crate::routes::Drawn;

/// The route of the page.
pub const IMPORT_ROUTE: &str = "/ehrs/import";

/// The files an extract upload takes.
const EXTRACT_SOURCES: &[&str] = &[".json", ".xml"];

/// Import EHR Extract.
pub async fn page() -> Drawn {
    let posted = crate::posted::posted();
    let text = |key: &str| {
        posted
            .as_ref()
            .and_then(|posted| posted.get(key))
            .unwrap_or_default()
    };
    let (source, ehr_id) = (text("source-text"), text("ehr_id"));
    let refusal = match posted
        .as_ref()
        .map(crate::posted::Posted::intent)
        .as_deref()
    {
        Some("import") => match import_extract(&source, &ehr_id).await {
            Ok(uid) => {
                redirect_with(
                    &ehr_href(&uid),
                    &Done::ok(tr(OperationsMsg::ExtractImported)).line(shortened(&uid)),
                );
                return Drawn::default();
            }
            Err(error) => Some(refused(
                tr(OperationsMsg::ExtractImportRefused),
                &error,
                None,
            )),
        },
        _ => None,
    };
    let title = tr(OperationsMsg::ImportExtract);
    let heading = super::under_ehrs(title.clone(), tr(OperationsMsg::ImportExtractSubtitle));
    let upload = crate::components::source_upload(
        &source,
        tr(OperationsMsg::ExtractSource),
        tr(OperationsMsg::ExtractChooseFile),
        tr(OperationsMsg::ExtractPlaceholder),
        EXTRACT_SOURCES,
    );
    let target = Field::new("x-ehr", tr(OperationsMsg::ExtractTarget))
        .holds_id()
        .view(h(format!(
            r#"<input id="x-ehr" name="ehr_id" type="text" class="fui-input fui-mono" value="{}" autocomplete="off">"#,
            t(&ehr_id)
        )));
    let form = h(format!(
        r#"<form method="post" action="{IMPORT_ROUTE}" class="fui-stack">{upload}{target}<div class="fui-cluster"><button type="submit" name="intent" value="import" class="fui-button fui-button-primary">{glyph}{import}</button></div></form>"#,
        glyph = ferrohealth_ui::icon::icon(Glyph::UPLOAD, "16"),
        import = t(tr(OperationsMsg::ExtractImport)),
    ));
    Drawn::page(
        title,
        heading.then(h(format!(
            r#"<div class="fui-stack">{}{}</div>"#,
            opt(refusal),
            card_panel(plain_section(form))
        ))),
    )
}
