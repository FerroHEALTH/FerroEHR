// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The confirmation of Delete composition. No openEHR spec governs the
//! viewer's pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, Stakes, confirm_dialog};

use crate::locale::records::RecordsMsg;
use crate::locale::{current, tr, tr_with};
use crate::markup::{h, t};
use crate::uid::version_number;

use super::{DELETE_DIALOG, composition_href};

/// The confirmation of Delete composition.
pub(super) fn delete_dialog(
    ehr_id: &str,
    uid: &str,
    name: &str,
    latest: &str,
    count: usize,
) -> Html {
    let next = version_number(latest).parse::<u32>().map_or_else(
        |_| String::new(),
        |number| number.saturating_add(1).to_string(),
    );
    confirm_dialog(
        &current(),
        Confirmation {
            id: DELETE_DIALOG.into(),
            title: tr_with(
                RecordsMsg::CompositionDeleteTitle,
                &[("name", name.to_owned().into())],
            )
            .into(),
            message: tr_with(
                RecordsMsg::CompositionDeleteMessage,
                &[
                    ("next", next.into()),
                    ("count", crate::pages::common::count(count)),
                ],
            )
            .into(),
        },
        Stakes::Danger,
        Some(ConfirmForm {
            action: composition_href(ehr_id, uid),
            fields: h(format!(
                r#"<input type="hidden" name="intent" value="delete-composition"><input type="hidden" name="version" value="{}">"#,
                t(latest)
            )),
        }),
        h(format!(
            r#"<button type="submit" class="fui-button fui-button-danger">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::TRASH, "16"),
            t(tr(RecordsMsg::CompositionDeleteConfirm))
        )),
    )
}
