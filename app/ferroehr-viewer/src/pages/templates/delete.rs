// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Deleting a template: the confirmation dialog, the confirmation drawn in
//! the page for a list row's Delete…, the post it answers and the refusal it
//! draws. No openEHR spec governs the viewer's pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, Stakes, confirm_dialog};

use crate::adl2::TemplateFamily;
use crate::bff::templates::delete_adl2_artefact;
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::definitions::DefinitionsMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, t};
use crate::pages::common::refused;

/// The confirmation of deleting the template `template_id` of `family`,
/// posting to `action`.
pub(super) fn delete_dialog(
    family: TemplateFamily,
    template_id: &str,
    id: &'static str,
    action: &str,
) -> Html {
    confirm_dialog(
        &current(),
        Confirmation {
            id: id.into(),
            title: tr_with(
                DefinitionsMsg::TemplateDeleteTitle,
                &[("name", template_id.to_owned().into())],
            )
            .into(),
            message: tr(DefinitionsMsg::TemplateDeleteMessage).into(),
        },
        Stakes::Danger,
        Some(ConfirmForm {
            action: action.to_owned(),
            fields: delete_fields(family, template_id),
        }),
        delete_submit(),
    )
}

/// The fields a template's delete posts: its intent, its family and its id.
fn delete_fields(family: TemplateFamily, template_id: &str) -> Html {
    h(format!(
        r#"<input type="hidden" name="intent" value="delete-template"><input type="hidden" name="family" value="{}"><input type="hidden" name="template" value="{}">"#,
        family.as_query(),
        t(template_id)
    ))
}

/// The submit of a template's delete.
fn delete_submit() -> Html {
    h(format!(
        r#"<button type="submit" class="fui-button fui-button-danger">{}{}</button>"#,
        ferrohealth_ui::icon::icon(Glyph::TRASH, "16"),
        t(tr(DefinitionsMsg::TemplateDeleteConfirm))
    ))
}

/// The delete confirmation drawn in the page, for a reader who came from a
/// list row's Delete… (the address's `confirm=delete`).
// NOTE: FerroUI `view::dialog::Confirmation::id` is `&'static str`, so a list draws no dialog per row.
pub(super) fn inline_confirm(
    family: TemplateFamily,
    template_id: &str,
    action: &str,
) -> Option<Html> {
    (crate::request::query("confirm").as_deref() == Some("delete")).then(|| {
        crate::pages::common::warn_note(h(format!(
            r#"<p><strong>{title}</strong> {message}</p><form method="post" action="{action}" class="fui-cluster">{fields}{submit}<a class="fui-button fui-button-secondary" href="{action}">{cancel}</a></form>"#,
            title = t(tr_with(
                DefinitionsMsg::TemplateDeleteTitle,
                &[("name", template_id.to_owned().into())],
            )),
            message = t(tr(DefinitionsMsg::TemplateDeleteMessage)),
            action = t(action),
            fields = delete_fields(family, template_id),
            submit = delete_submit(),
            cancel = t(tr(Msg::Cancel)),
        )))
    })
}

/// Deletes the template a post names and sends the browser to `back`; `Some`
/// holds the CDR's refusal.
pub(super) async fn delete_posted(back: &str) -> Option<ViewerError> {
    let posted = crate::posted::posted_for("delete-template")?;
    let template = posted.text("template");
    let done = match TemplateFamily::from_query(&posted.text("family")) {
        TemplateFamily::Adl14 => crate::admin::admin_delete_template(template.clone()).await,
        TemplateFamily::Adl2 => delete_adl2_artefact(template.clone()).await,
    };
    match done {
        Ok(()) => {
            redirect_with(
                back,
                &Done::ok(tr(DefinitionsMsg::TemplateDeleted)).line(template),
            );
            None
        }
        Err(error) => Some(error),
    }
}

/// The refusal of a delete the page answered.
pub(super) fn delete_refused(acted: Option<&ViewerError>) -> Option<Html> {
    acted.map(|error| refused(tr(DefinitionsMsg::TemplateDeleteRefused), error, None))
}
