// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Directory tab's confirmations: delete the directory and restore an
//! older version. No openEHR spec governs the viewer's pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, Stakes, confirm_dialog};

use crate::locale::records::RecordsMsg;
use crate::locale::{current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::ehr::{EhrTab, tab_href};
use crate::uid::version_number;

use super::{DELETE_DIALOG, RESTORE_DIALOG};

/// The confirmations of the tab: delete the directory whose latest version
/// is `latest_uid`, and restore the version `restoring` names, whose
/// document it holds.
pub(super) fn dialogs(ehr_id: &str, latest_uid: &str, restoring: Option<(&str, &str)>) -> Html {
    let action = tab_href(ehr_id, EhrTab::Directory);
    let delete = confirm_dialog(
        &current(),
        Confirmation {
            id: DELETE_DIALOG.into(),
            title: tr(RecordsMsg::DirectoryDeleteTitle).into(),
            message: tr(RecordsMsg::DirectoryDeleteMessage).into(),
        },
        Stakes::Danger,
        Some(ConfirmForm {
            action: action.clone(),
            fields: h(format!(
                r#"<input type="hidden" name="intent" value="delete-directory"><input type="hidden" name="version" value="{}">"#,
                t(latest_uid)
            )),
        }),
        h(format!(
            r#"<button type="submit" class="fui-button fui-button-danger">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::TRASH, "16"),
            t(tr(RecordsMsg::DirectoryDeleteConfirm))
        )),
    );
    let restore = restoring.map(|(number, body)| {
        let next = version_number(latest_uid)
            .parse::<u32>()
            .map_or_else(|_| String::new(), |latest| (latest + 1).to_string());
        let form = h(format!(
            r#"<form method="post" action="{action}"><input type="hidden" name="intent" value="restore"><input type="hidden" name="version" value="{version}"><input type="hidden" name="body" value="{body}"><button type="submit" class="fui-button fui-button-primary">{glyph}{words}</button></form>"#,
            action = t(&action),
            version = t(latest_uid),
            body = t(body),
            glyph = ferrohealth_ui::icon::icon(Glyph::ROTATE_CCW, "16"),
            words = t(tr_with(
                RecordsMsg::DirectoryRestoreAs,
                &[("next", next.clone().into())]
            )),
        ));
        confirm_dialog(
            &current(),
            Confirmation {
                id: RESTORE_DIALOG.into(),
                title: tr_with(
                    RecordsMsg::DirectoryRestoreTitle,
                    &[("version", number.to_owned().into())],
                )
                .into(),
                message: tr_with(
                    RecordsMsg::DirectoryRestoreMessage,
                    &[
                        ("version", number.to_owned().into()),
                        ("next", next.into()),
                    ],
                )
                .into(),
            },
            Stakes::Plain,
            None,
            form,
        )
    });
    delete.then(opt(restore))
}
