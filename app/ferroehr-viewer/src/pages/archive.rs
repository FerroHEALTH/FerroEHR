// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Archive and Restore on an EHR's and a party's page: the two More-menu
//! items, their confirmations, and the post they answer
//! ([`crate::admin::archive`]).
//!
//! An archived record stays readable by id and leaves AQL results, so the
//! confirmation says so. The items are drawn only while the admin group is
//! mounted; the CDR decides every call. No openEHR spec governs these
//! routes: the CDR's own extension.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::action_menu::{Act, Action, Item};
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, Stakes, confirm_dialog};

use crate::admin::Archived;
use crate::flash::{Done, redirect_with};
use crate::locale::operations::OperationsMsg;
use crate::locale::{current, tr};
use crate::markup::{h, t};
use crate::pages::common::Refused;

/// The id of Archive's confirmation.
const ARCHIVE_DIALOG: &str = "archive-record";

/// The id of Restore's confirmation.
const RESTORE_DIALOG: &str = "restore-record";

/// The More-menu items Archive… and Restore….
#[must_use]
pub fn items() -> Vec<Item> {
    vec![
        Item::new(Action::new(
            Some(Glyph::FOLDER_X),
            tr(OperationsMsg::ArchiveItem),
            Act::Dialog(ARCHIVE_DIALOG.to_owned()),
        )),
        Item::new(Action::new(
            Some(Glyph::FOLDER_OPEN),
            tr(OperationsMsg::RestoreItem),
            Act::Dialog(RESTORE_DIALOG.to_owned()),
        )),
    ]
}

/// The two confirmations of the record of `kind` whose page is `action`.
#[must_use]
pub fn dialogs(kind: Archived, action: &str) -> Html {
    let (archive_message, restore_message) = match kind {
        Archived::Ehrs => (
            OperationsMsg::ArchiveEhrMessage,
            OperationsMsg::RestoreEhrMessage,
        ),
        Archived::Parties => (
            OperationsMsg::ArchivePartyMessage,
            OperationsMsg::RestorePartyMessage,
        ),
    };
    let dialog = |id: &str, intent: &str, title, message, confirm, glyph| {
        confirm_dialog(
            &current(),
            Confirmation {
                id: id.to_owned().into(),
                title: tr(title).into(),
                message: tr(message).into(),
            },
            Stakes::Plain,
            Some(ConfirmForm {
                action: action.to_owned(),
                fields: h(format!(
                    r#"<input type="hidden" name="intent" value="{intent}">"#
                )),
            }),
            h(format!(
                r#"<button type="submit" class="fui-button fui-button-primary">{}{}</button>"#,
                ferrohealth_ui::icon::icon(glyph, "16"),
                t(tr(confirm))
            )),
        )
    };
    dialog(
        ARCHIVE_DIALOG,
        "archive",
        OperationsMsg::ArchiveTitle,
        archive_message,
        OperationsMsg::ArchiveConfirm,
        Glyph::FOLDER_X,
    )
    .then(dialog(
        RESTORE_DIALOG,
        "restore",
        OperationsMsg::RestoreTitle,
        restore_message,
        OperationsMsg::RestoreConfirm,
        Glyph::FOLDER_OPEN,
    ))
}

/// Answers an Archive or a Restore of the record `id` of `kind`.
///
/// `None` when `intent` is neither; a success redirects to `back` with its
/// toast, and a refusal is returned in the CDR's words.
pub async fn act(kind: Archived, id: &str, intent: &str, back: &str) -> Option<Option<Refused>> {
    let restore = match intent {
        "archive" => false,
        "restore" => true,
        _ => return None,
    };
    let (done, refused) = if restore {
        (OperationsMsg::Restored, OperationsMsg::RestoreRefused)
    } else {
        (OperationsMsg::Archived, OperationsMsg::ArchiveRefused)
    };
    Some(
        match crate::admin::archive(kind, id.to_owned(), restore).await {
            Ok(()) => {
                redirect_with(back, &Done::ok(tr(done)));
                None
            }
            Err(error) => Some(Refused::new(tr(refused), error)),
        },
    )
}
