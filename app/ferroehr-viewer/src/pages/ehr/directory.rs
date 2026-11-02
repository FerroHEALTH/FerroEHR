// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Directory tab and Edit the directory (FerroUI
//! `design/consoles/ferroehr/`, pages 12.9 to 12.11 and 13.2 to 13.3).
//!
//! The tab draws the directory's `FOLDER` tree as FerroUI's folder tree (E4),
//! the folders with their counts and the references as names in a list, and
//! its versions with At a time and Show one folder by path (ITS-REST
//! `directory_get`, `directory_get_at_time`, `directory_get_by_path`). An
//! older version opens read-only and can be restored, written back as a new
//! version. Delete the directory is a logical delete: the history stays.
//!
//! The editor keeps the working tree in the form, so every action is a plain
//! post the server answers with the tree changed: add a folder, add a
//! reference to one of the EHR's compositions, rename, remove. Save writes the
//! tree as one new version with the version it was opened at as `If-Match`
//! (ITS-REST `directory_update`), so a directory another client changed is
//! refused (`412`), never overwritten unless the reader chooses Overwrite.
//! The working tree carries a viewer-local `_key` on each node, stripped
//! before anything reaches the CDR ([`crate::bff::ehr_detail::directory::edit`]).
//! No openEHR spec governs the viewer's pages: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer works on the CDR's FOLDER JSON wire, the working copy of the directory editor"
)]

use std::collections::BTreeSet;

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action, Item, Menu, row_menu};
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, confirm_dialog, danger_form_dialog};
use ferrohealth_ui::view::surface::{Crumb, Header, Mode, editor_heading};
use ferrohealth_ui::view::tree::{Edit, Mark, Tree, TreeKind, TreeNode, TreeWords, tree_view};
use ferrohealth_ui::view::versions::{
    Standing, Version, VersionHistory, older_version, version_history,
};
use serde_json::Value;

use crate::bff::ehr_detail::directory::edit::{
    add_item, add_subfolder, delete_folder, find_item_by_key, find_path_by_key, next_counter,
    object_ref, remove_item, rename_folder, stamp_keys, strip_keys,
};
use crate::bff::ehr_detail::directory::{
    DirectoryAtTime, DirectoryState, DirectorySubtree, delete_directory, fetch_directory,
    fetch_directory_at_time, fetch_directory_subtree, list_directory_versions, update_directory,
};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, message, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::is_status;
use crate::pages::common::{
    EditorDock, Refused, at_time_row, card_panel, dock_editor, info_note, panel, plain_section,
    quiet, section, shortened, version_number,
};
use crate::pages::ehr::EhrModel;
use crate::pages::ehr::{EhrTab, crumbs, ehr_id_param, ehr_view, tab_href};
use crate::pages::form::Form;
use crate::routes::Drawn;

/// The id of Delete the directory's confirmation dialog.
const DELETE_DIALOG: &str = "delete-directory";

/// The id of a restore's confirmation dialog.
const RESTORE_DIALOG: &str = "restore-directory";

/// How many versions the history reads at once, and how many more Older
/// versions reads.
const HISTORY_WINDOW: u32 = 10;

/// The most versions one page of the history reads.
const HISTORY_MAX: u32 = 500;

/// The display name of a `FOLDER` or `OBJECT_REF` node.
fn folder_name(node: &Value) -> String {
    node.pointer("/name/value")
        .and_then(Value::as_str)
        .map_or_else(|| tr(RecordsMsg::DirectoryUnnamed), str::to_owned)
}

/// The words counting what a folder holds.
fn count_words(node: &Value) -> String {
    let folders = node
        .get("folders")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let items = node
        .get("items")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    match (folders, items) {
        (0, 0) => tr(RecordsMsg::DirectoryEmpty),
        (0, items) => tr_with(
            RecordsMsg::DirectoryItems,
            &[("count", crate::pages::common::count(items))],
        ),
        (folders, 0) => tr_with(
            RecordsMsg::DirectoryFolders,
            &[("count", crate::pages::common::count(folders))],
        ),
        (folders, items) => tr_with(
            RecordsMsg::DirectoryFoldersItems,
            &[
                ("folders", crate::pages::common::count(folders)),
                ("items", crate::pages::common::count(items)),
            ],
        ),
    }
}

/// The page of the object an `OBJECT_REF` names, when it names a composition
/// of the EHR.
fn reference_href(ehr_id: &str, item: &Value) -> Option<String> {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
    let id = item.pointer("/id/value").and_then(Value::as_str)?;
    matches!(kind, "VERSIONED_COMPOSITION" | "COMPOSITION").then(|| {
        crate::pages::composition::composition_href(ehr_id, &crate::uid::container_uid_of(id))
    })
}

/// One node of the folder tree, keyed by `key` (its `_key` in an edit, its
/// place in a reading), its folders before its references.
fn node_of(
    ehr_id: &str,
    folder: &Value,
    key: &str,
    editing: Option<&Edits>,
    open: &mut BTreeSet<String>,
) -> TreeNode {
    let mut children = Vec::new();
    if let Some(folders) = folder.get("folders").and_then(Value::as_array) {
        for (index, child) in folders.iter().enumerate() {
            let child_key = node_key(child, key, "f", index, editing.is_some());
            children.push(node_of(ehr_id, child, &child_key, editing, open));
        }
    }
    if let Some(items) = folder.get("items").and_then(Value::as_array) {
        for (index, item) in items.iter().enumerate() {
            let item_key = node_key(item, key, "i", index, editing.is_some());
            let id = item
                .pointer("/id/value")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut node = TreeNode::new(item_key, shortened(id))
                .mark(Mark::Reference)
                .tag(
                    item.get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                );
            if editing.is_some() {
                node = node.edits(vec![Edit::Remove]);
            } else if let Some(href) = reference_href(ehr_id, item) {
                node = node.href(href);
            }
            children.push(node);
        }
    }
    open.insert(key.to_owned());
    let mut node = TreeNode::new(key, folder_name(folder))
        .mark(Mark::Folder)
        .count(count_words(folder));
    if let Some(edits) = editing {
        let root = key == edits.root_key;
        node = node.edits(if root {
            vec![Edit::AddFolder, Edit::AddReference]
        } else {
            vec![
                Edit::AddFolder,
                Edit::AddReference,
                Edit::Rename,
                Edit::Remove,
            ]
        });
        if edits.renaming.as_deref() == Some(key) {
            node = node.renaming(folder_name(folder));
        }
    }
    if children.is_empty() {
        node
    } else {
        node.children(children)
    }
}

/// The key of a child node: its `_key` in an edit, its place in a reading.
fn node_key(node: &Value, parent: &str, kind: &str, index: usize, editing: bool) -> String {
    if editing && let Some(key) = node.get("_key").and_then(Value::as_str) {
        return key.to_owned();
    }
    format!("{parent}-{kind}{index}")
}

/// What an edit of the tree draws: the root's key and the folder in a rename.
#[derive(Debug, Clone)]
struct Edits {
    /// The root folder's `_key`.
    root_key: String,
    /// The folder whose name is being changed.
    renaming: Option<String>,
}

/// The folder tree (E4) of `root`, in an edit when `editing`.
fn tree(ehr_id: &str, root: &Value, editing: Option<&Edits>) -> Html {
    let mut open = BTreeSet::new();
    let root_key = editing.map_or_else(|| "root".to_owned(), |edits| edits.root_key.clone());
    let node = node_of(ehr_id, root, &root_key, editing, &mut open);
    tree_view(&Tree {
        id: "directory-tree".to_owned(),
        label: tr(RecordsMsg::DirectoryTreeLabel),
        kind: TreeKind::Folder,
        editing: editing.is_some(),
        nodes: vec![node],
        open,
        selected: None,
        open_field: None,
        words: TreeWords::of(&current()),
    })
}

/// The version history of the directory: each version with Restore… on an
/// older one, the version in view marked.
fn history(ehr_id: &str, versions: &[DirVersion], reading: Option<&str>) -> Html {
    let rows = versions
        .iter()
        .map(|version| {
            let number = version.number.to_string();
            let href = if version.is_latest {
                tab_href(ehr_id, EhrTab::Directory)
            } else {
                format!("{}?version={number}", tab_href(ehr_id, EhrTab::Directory))
            };
            Version {
                href: Some(href),
                standing: if version.is_latest {
                    Standing::Current
                } else {
                    Standing::Older
                },
                read: reading == Some(number.as_str()),
                change: tr_with(
                    RecordsMsg::DirectoryVersionHolds,
                    &[
                        ("folders", version.folder_count.into()),
                        ("items", version.item_count.into()),
                    ],
                ),
                committer: String::new(),
                when: Html::empty(),
                contribution: if version.is_latest {
                    Html::empty()
                } else {
                    restore_button(&number)
                },
                number,
            }
        })
        .collect();
    version_history(
        &current(),
        VersionHistory {
            caption: tr(RecordsMsg::DirectoryVersions),
            versions: rows,
            at_time: Some(at_time_row(
                tab_href(ehr_id, EhrTab::Directory),
                &crate::request::query("at").unwrap_or_default(),
                tr(RecordsMsg::DirectoryAtTimeOpens),
            )),
        },
    )
}

/// The Restore… of an older version: the page with that version in view,
/// whose header offers the restore.
fn restore_button(number: &str) -> Html {
    // NOTE: R9: the row's own link covers the row; a table action sits above it.
    h(format!(
        r#"<span class="fui-table-actions"><a class="fui-button fui-button-ghost fui-button-small" href="{href}" aria-label="{label}">{glyph}{words}</a></span>"#,
        href = t(format!("?version={number}&restore=1")),
        label = t(tr_with(
            RecordsMsg::DirectoryRestoreVersion,
            &[("version", number.to_owned().into())],
        )),
        glyph = ferrohealth_ui::icon::icon(Glyph::ROTATE_CCW, "14"),
        words = t(tr(RecordsMsg::DirectoryRestore)),
    ))
}

/// The directory's own menu: Copy the directory id when the version names
/// it, then Delete the directory…, last.
fn directory_menu(version_uid: &str) -> Html {
    let copy = version_uid
        .split("::")
        .next()
        .filter(|id| !id.is_empty())
        .map(|id| {
            Item::new(Action::new(
                Some(Glyph::COPY),
                tr(RecordsMsg::DirectoryCopyId),
                Act::Copy(id.to_owned()),
            ))
        });
    row_menu(
        &current(),
        &Menu::new(
            "directory-more",
            tr(RecordsMsg::DirectoryMore),
            copy.into_iter().collect(),
        )
        .destructive(vec![Item::new(Action::new(
            Some(Glyph::TRASH),
            tr(RecordsMsg::DirectoryDelete),
            Act::Dialog(DELETE_DIALOG.to_owned()),
        ))]),
    )
}

/// The confirmations of the tab: delete the directory whose latest version
/// is `latest_uid`, and restore the version `restoring` names, whose
/// document it holds.
fn dialogs(ehr_id: &str, latest_uid: &str, restoring: Option<(&str, &str)>) -> Html {
    let action = tab_href(ehr_id, EhrTab::Directory);
    let delete = danger_form_dialog(
        &current(),
        Confirmation {
            id: DELETE_DIALOG.into(),
            title: tr(RecordsMsg::DirectoryDeleteTitle).into(),
            message: tr(RecordsMsg::DirectoryDeleteMessage).into(),
        },
        ConfirmForm {
            action: action.clone(),
            fields: h(format!(
                r#"<input type="hidden" name="intent" value="delete-directory"><input type="hidden" name="version" value="{}">"#,
                t(latest_uid)
            )),
        },
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
            form,
        )
    });
    delete.then(opt(restore))
}

/// A write the CDR refused.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Unsaved {
    /// The refusal.
    refusal: Refused,
    /// Whether another client changed the directory meanwhile (`412`).
    conflict: bool,
}

impl Unsaved {
    /// The refusal `error` of what `summary` says.
    fn new(summary: String, error: ViewerError) -> Self {
        Self {
            conflict: is_status(&error, http::StatusCode::PRECONDITION_FAILED),
            refusal: Refused::new(summary, error),
        }
    }
}

/// What a post to the tab asks, done: delete the directory, restore a
/// version. `Some` holds the refusal to draw at the top.
async fn act(ehr_id: &str) -> Option<Unsaved> {
    let posted = crate::posted::posted()?;
    let version = posted.text("version");
    match posted.intent().as_str() {
        "delete-directory" => match delete_directory(ehr_id.to_owned(), version).await {
            Ok(()) => {
                redirect_with(
                    &tab_href(ehr_id, EhrTab::Compositions),
                    &Done::ok(tr(RecordsMsg::DirectoryDeleted)),
                );
                None
            }
            Err(error) => Some(Unsaved {
                refusal: Refused::new(tr(RecordsMsg::DirectoryDeleteRefused), error),
                conflict: false,
            }),
        },
        "restore" => {
            let body = posted.get("body").unwrap_or_default();
            match update_directory(ehr_id.to_owned(), version, body).await {
                Ok(uid) => {
                    redirect_with(
                        &tab_href(ehr_id, EhrTab::Directory),
                        &Done::ok(tr(RecordsMsg::DirectoryRestored)).line(tr_with(
                            RecordsMsg::DirectorySavedVersion,
                            &[("version", version_number(&uid).into())],
                        )),
                    );
                    None
                }
                Err(error) => Some(Unsaved::new(tr(RecordsMsg::DirectoryRestoreRefused), error)),
            }
        }
        _ => None,
    }
}

/// The refusal a post to the tab met, with Reload when the directory changed
/// meanwhile.
fn acted_view(ehr_id: &str, unsaved: &Unsaved) -> Html {
    let reload = unsaved.conflict.then(|| {
        h(format!(
            r#"<div class="fui-cluster">{}</div>"#,
            ferrohealth_ui::view::button::link_button(
                tab_href(ehr_id, EhrTab::Directory),
                ferrohealth_ui::view::button::Variant::Secondary,
                Some(Glyph::REFRESH_CW),
                tr(RecordsMsg::DirectoryReload),
            )
        ))
    });
    unsaved.refusal.view().then(opt(reload))
}

/// One version of the directory's history.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DirVersion {
    /// Its number.
    number: i32,
    /// How many folders it holds.
    folder_count: i32,
    /// How many references it holds.
    item_count: i32,
    /// Whether it is the latest.
    is_latest: bool,
}

/// The folders the tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Folders {
    /// The whole tree of the version shown.
    Whole,
    /// The folder at the path asked for.
    Path(Value),
    /// No folder at the path asked for, so the whole tree.
    NoPath,
    /// The CDR's refusal of the path, or a folder that is not JSON.
    Refused(Refused),
}

/// The directory version the tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    /// What a post to the tab did not do.
    acted: Option<Unsaved>,
    /// The id of the latest version.
    latest_uid: String,
    /// The window of versions the history read.
    window: u32,
    /// The versions, or the CDR's refusal of them.
    versions: Result<Vec<DirVersion>, Refused>,
    /// The number of the version asked for, when one was.
    older: Option<String>,
    /// The id of the version shown.
    version_uid: String,
    /// Its document as the CDR sent it.
    body: String,
    /// Its tree.
    root: Value,
    /// The path asked for, when one was.
    path: Option<String>,
    /// The folders shown.
    folders: Folders,
    /// Whether the address asks to restore the version shown.
    restoring: bool,
}

/// The Directory tab's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DirectoryPanel {
    /// The EHR has no directory.
    None,
    /// It had none at the time asked for.
    NoneAtTime,
    /// The version shown.
    Shown(Box<Shown>),
    /// The CDR's refusal, or a document that is not JSON.
    Refused(Refused),
}

/// The refusal of a directory document the CDR sent that is not JSON.
fn unreadable(error: &serde_json::Error) -> Refused {
    Refused::new(
        tr(RecordsMsg::DirectoryRefused),
        ViewerError::Internal(error.to_string()),
    )
}

/// What the page reads.
#[expect(
    clippy::too_many_lines,
    reason = "one loader: the post, the version asked for, its versions and the folder at a path"
)]
async fn read_tab(ehr_id: String) -> EhrModel<DirectoryPanel> {
    let acted = act(&ehr_id).await;
    crate::pages::ehr::load_ehr(&ehr_id, async |_facts| {
        let asked = crate::request::query("version");
        let at = crate::request::query("at");
        let folder_path = crate::request::query("path");
        let current = match fetch_directory(ehr_id.clone()).await {
            Ok(Some(current)) => current,
            Ok(None) => return DirectoryPanel::None,
            Err(error) => {
                return DirectoryPanel::Refused(Refused::new(
                    tr(RecordsMsg::DirectoryRefused),
                    error,
                ));
            }
        };
        let latest = version_number(&current.version_uid);
        let latest_number = latest.parse::<u32>().unwrap_or(1);
        // NOTE: ITS-REST 1.1.0 has no FOLDER revision history to page (AMB-24); the window
        // widens instead, far enough to hold the version asked for.
        let reach = asked
            .as_deref()
            .and_then(|number| number.parse::<u32>().ok())
            .map_or(0, |number| {
                latest_number.saturating_sub(number).saturating_add(1)
            });
        let window = crate::request::query("versions")
            .and_then(|window| window.parse::<u32>().ok())
            .unwrap_or(HISTORY_WINDOW)
            .max(reach)
            .clamp(HISTORY_WINDOW, HISTORY_MAX);
        let versions = list_directory_versions(ehr_id.clone(), window).await;
        let shown: Result<Option<(DirectoryState, Option<String>)>, ViewerError> =
            match (&at, &asked) {
                (Some(at), _) => fetch_directory_at_time(ehr_id.clone(), at.clone())
                    .await
                    .map(|found| match found {
                        DirectoryAtTime::Present(state) => {
                            let number = version_number(&state.version_uid);
                            Some((state, Some(number)))
                        }
                        DirectoryAtTime::DeletedAtTime | DirectoryAtTime::NoneAtTime => None,
                    }),
                (None, Some(number)) if *number != latest => Ok(versions
                    .as_ref()
                    .ok()
                    .and_then(|history| {
                        history
                            .versions
                            .iter()
                            .find(|version| version.number.to_string() == *number)
                    })
                    .map(|version| {
                        (
                            DirectoryState {
                                body: version.body.clone(),
                                version_uid: version.version_uid.clone(),
                            },
                            Some(number.clone()),
                        )
                    })),
                _ => Ok(Some((current.clone(), None))),
            };
        let (shown, older) = match shown {
            Ok(Some(shown)) => shown,
            Ok(None) => return DirectoryPanel::NoneAtTime,
            Err(error) => {
                return DirectoryPanel::Refused(Refused::new(
                    tr(RecordsMsg::DirectoryRefused),
                    error,
                ));
            }
        };
        let root: Value = match serde_json::from_str(&shown.body) {
            Ok(root) => root,
            Err(error) => return DirectoryPanel::Refused(unreadable(&error)),
        };
        let folders = match &folder_path {
            Some(path) => match fetch_directory_subtree(ehr_id.clone(), path.clone()).await {
                Ok(DirectorySubtree::Found(body)) => match serde_json::from_str::<Value>(&body) {
                    Ok(sub) => Folders::Path(sub),
                    Err(error) => Folders::Refused(unreadable(&error)),
                },
                Ok(DirectorySubtree::Missing) => Folders::NoPath,
                Err(error) => {
                    Folders::Refused(Refused::new(tr(RecordsMsg::DirectoryPathRefused), error))
                }
            },
            None => Folders::Whole,
        };
        let versions = versions
            .map(|history| {
                history
                    .versions
                    .into_iter()
                    .map(|version| DirVersion {
                        number: version.number,
                        folder_count: version.folder_count,
                        item_count: version.item_count,
                        is_latest: version.is_latest,
                    })
                    .collect()
            })
            .map_err(|error| Refused::new(tr(RecordsMsg::DirectoryHistoryRefused), error));
        DirectoryPanel::Shown(Box::new(Shown {
            acted,
            latest_uid: current.version_uid,
            window,
            versions,
            older,
            version_uid: shown.version_uid,
            body: shown.body,
            root,
            path: folder_path,
            folders,
            restoring: crate::request::query("restore").is_some(),
        }))
    })
    .await
}

/// The Directory tab's panel.
#[expect(
    clippy::too_many_lines,
    reason = "one tab: its notice, its folders with the path form, its versions and its dialogs"
)]
fn directory_panel(ehr_id: &str, model: DirectoryPanel) -> Html {
    let shown = match model {
        DirectoryPanel::None => return panel(plain_section(quiet(tr(RecordsMsg::DirectoryNone)))),
        DirectoryPanel::NoneAtTime => {
            return panel(plain_section(info_note(t(tr(
                RecordsMsg::DirectoryNoneAtTime,
            )))));
        }
        DirectoryPanel::Refused(refusal) => return panel(plain_section(refusal.view())),
        DirectoryPanel::Shown(shown) => *shown,
    };
    let Shown {
        acted,
        latest_uid,
        window,
        versions,
        older,
        version_uid,
        body,
        root,
        path,
        folders,
        restoring,
    } = shown;
    let latest = version_number(&latest_uid);
    let latest_number = latest.parse::<u32>().unwrap_or(1);
    let folder_path = path.unwrap_or_default();
    let shown_tree = match folders {
        Folders::Path(sub) => info_note(t(tr_with(
            RecordsMsg::DirectoryShowingPath,
            &[("path", folder_path.clone().into())],
        )))
        .then(tree(ehr_id, &sub, None)),
        Folders::NoPath => info_note(t(tr_with(
            RecordsMsg::DirectoryNoPath,
            &[("path", folder_path.clone().into())],
        )))
        .then(tree(ehr_id, &root, None)),
        Folders::Refused(refusal) => refusal.view(),
        Folders::Whole => tree(ehr_id, &root, None),
    };
    let path_form = h(format!(
        r#"<form class="fui-at-time" method="get" action="{action}"><div class="fui-field"><label for="d-path" class="fui-label">{label}</label><input id="d-path" name="path" type="text" class="fui-input fui-mono" placeholder="{placeholder}" value="{value}"></div><button type="submit" class="fui-button fui-button-secondary">{glyph}{show}</button></form>"#,
        action = t(tab_href(ehr_id, EhrTab::Directory)),
        label = t(tr(RecordsMsg::DirectoryShowPath)),
        placeholder = t(tr(RecordsMsg::DirectoryPathPlaceholder)),
        value = t(&folder_path),
        glyph = ferrohealth_ui::icon::icon(Glyph::FOLDER_SEARCH, "16"),
        show = t(tr(RecordsMsg::DirectoryShow)),
    ));
    let reading = older.as_deref().filter(|number| *number != latest.as_str());
    let tools = ferrohealth_ui::view::button::link_button(
        format!("{}/edit", tab_href(ehr_id, EhrTab::Directory)),
        ferrohealth_ui::view::button::Variant::Secondary,
        Some(Glyph::PENCIL),
        tr(RecordsMsg::DirectoryEdit),
    )
    .then(directory_menu(&version_uid));
    let restore = reading.filter(|_| restoring).map(|number| {
        h(format!(
            r#"<button type="button" class="fui-button fui-button-primary" command="show-modal" commandfor="{RESTORE_DIALOG}" aria-haspopup="dialog">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::ROTATE_CCW, "16"),
            t(tr_with(
                RecordsMsg::DirectoryRestoreVersion,
                &[("version", number.to_owned().into())],
            ))
        ))
    });
    let count = versions.as_ref().map_or(0, |versions| {
        u32::try_from(versions.len()).unwrap_or(u32::MAX)
    });
    let notice = reading.map(|number| {
        plain_section(
            older_version(
                &current(),
                number,
                count,
                tab_href(ehr_id, EhrTab::Directory),
            )
            .then(h(format!(
                r#"<div class="fui-cluster">{}</div>"#,
                opt(restore)
            ))),
        )
    });
    let history_section = match &versions {
        Ok(history_data) => section(
            Some(tr(RecordsMsg::DirectoryVersions)),
            Some(u64::from(count)),
            None,
            Html::join([
                ferrohealth_ui::view::surface::section_intro(tr(
                    RecordsMsg::DirectoryVersionsIntro,
                )),
                history(ehr_id, history_data, reading),
                opt((count < latest_number && window < HISTORY_MAX).then(|| {
                    h(format!(
                        r#"<div class="fui-cluster">{}</div>"#,
                        ferrohealth_ui::view::button::link_button(
                            format!(
                                "{}?versions={}",
                                tab_href(ehr_id, EhrTab::Directory),
                                window.saturating_add(HISTORY_WINDOW),
                            ),
                            ferrohealth_ui::view::button::Variant::Ghost,
                            Some(Glyph::CHEVRON_DOWN),
                            tr(RecordsMsg::DirectoryOlderVersions),
                        )
                    ))
                })),
            ]),
        ),
        Err(refusal) => section(
            Some(tr(RecordsMsg::DirectoryVersions)),
            None,
            None,
            refusal.view(),
        ),
    };
    let restoring_state = reading.map(|number| (number, body.as_str()));
    panel(Html::join([
        opt(acted.map(|acted| acted_view(ehr_id, &acted))),
        opt(notice),
        section(
            Some(tr(RecordsMsg::DirectoryFoldersTitle)),
            None,
            Some(tools),
            shown_tree.then(path_form),
        ),
        history_section,
        dialogs(ehr_id, &latest_uid, restoring_state),
    ]))
}

/// An EHR's Directory tab.
pub async fn page() -> Drawn {
    let ehr_id = ehr_id_param();
    let title = tr_with(
        RecordsMsg::DirectoryTitle,
        &[("id", shortened(&ehr_id).into())],
    );
    let body = {
        let model = read_tab(ehr_id.clone()).await;
        ehr_view(&ehr_id, EhrTab::Directory, model, |shown| {
            directory_panel(&ehr_id, shown)
        })
    };
    Drawn::page(title, body)
}

/// The id of Edit the directory's form.
const EDIT_FORM: &str = "directory-edit";

/// The working tree of an edit: the tree as posted, or `base`, the
/// directory as the CDR holds it, stamped with the viewer-local keys.
fn working_tree(posted: Option<&crate::posted::Posted>, base: &Value) -> Value {
    // NOTE: the tree is the page's own hidden field; one that does not parse starts again from the CDR's.
    if let Some(tree) = posted
        .and_then(|posted| posted.get("tree"))
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
    {
        return tree;
    }
    let mut tree = base.clone();
    let mut counter = 0_u64;
    stamp_keys(&mut tree, &mut counter);
    tree
}

/// Applies the edit a post asks to `tree`; returns the folder now in a
/// rename, if any, and how many edits the tree carries.
fn apply(tree: &mut Value, posted: &crate::posted::Posted) -> Option<String> {
    let mut counter = next_counter(tree);
    if let Some(key) = posted.get("renaming").filter(|key| !key.is_empty()) {
        let name = posted.text("new_name");
        if let Some(path) = find_path_by_key(tree, &key)
            && !name.is_empty()
        {
            rename_folder(tree, &path, &name);
        }
    }
    if let Some(key) = posted.get(Edit::AddFolder.name())
        && let Some(path) = find_path_by_key(tree, &key)
    {
        add_subfolder(tree, &path, &tr(RecordsMsg::DirectoryNewFolder));
        stamp_keys(tree, &mut counter);
    }
    if let Some(key) = posted.get(Edit::Rename.name()) {
        return Some(key);
    }
    if let Some(key) = posted.get(Edit::Remove.name()) {
        if let Some(path) = find_path_by_key(tree, &key) {
            delete_folder(tree, &path);
        } else if let Some((path, index)) = find_item_by_key(tree, &key) {
            remove_item(tree, &path, index);
        }
    }
    if let (Some(key), Some(uid)) = (posted.get("reference_to"), posted.get("reference"))
        && let Some(path) = find_path_by_key(tree, &key)
        && !uid.trim().is_empty()
    {
        add_item(
            tree,
            &path,
            object_ref(
                "local",
                "VERSIONED_COMPOSITION",
                "HIER_OBJECT_ID",
                uid.trim(),
            ),
        );
        stamp_keys(tree, &mut counter);
    }
    None
}

/// The tree as the CDR takes it: the viewer-local keys stripped.
fn stripped(tree: &Value) -> String {
    let mut tree = tree.clone();
    strip_keys(&mut tree);
    tree.to_string()
}

/// A composition a reference can name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    /// Its id.
    uid: String,
    /// Its name.
    name: String,
}

/// The picker of a reference to add to a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Picker {
    /// The folder's `_key`.
    folder: String,
    /// The EHR's compositions, or the CDR's refusal of the list.
    choices: Result<Vec<Choice>, Refused>,
}

/// What Edit the directory's form holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Editor {
    /// The id of the version the edit is based on.
    opened: String,
    /// The id of the latest version.
    current_uid: String,
    /// The working tree.
    working: Value,
    /// The folder whose name is being changed.
    renaming: Option<String>,
    /// Whether the working tree differs from the version opened.
    changed: bool,
    /// The save the CDR refused, when the post was one.
    unsaved: Option<Unsaved>,
    /// The picker of a reference to add, when one is asked for.
    picker: Option<Picker>,
}

/// What the page reads.
async fn read_edit(ehr_id: String) -> Form<Editor, ()> {
    let posted = crate::posted::posted();
    if crate::pages::common::discarded(posted.as_ref()) {
        crate::request::redirect(&tab_href(&ehr_id, EhrTab::Directory));
        return Form::Left;
    }
    let current = match fetch_directory(ehr_id.clone()).await {
        Ok(Some(current)) => current,
        Ok(None) => return Form::Not(()),
        Err(error) => {
            return Form::Refused(Refused::new(tr(RecordsMsg::DirectoryRefused), error));
        }
    };
    let opened = posted
        .as_ref()
        .map(|posted| posted.text("version"))
        .filter(|uid| !uid.is_empty())
        .unwrap_or_else(|| current.version_uid.clone());
    let base: Value = match serde_json::from_str(&current.body) {
        Ok(base) => base,
        Err(error) => return Form::Refused(unreadable(&error)),
    };
    let mut working = working_tree(posted.as_ref(), &base);
    let renaming = posted
        .as_ref()
        .and_then(|posted| apply(&mut working, posted));
    let adding = posted
        .as_ref()
        .and_then(|posted| posted.get(Edit::AddReference.name()));
    let original = working_tree(None, &base);
    let changed = stripped(&working) != stripped(&original);
    let intent = posted
        .as_ref()
        .map(crate::posted::Posted::intent)
        .unwrap_or_default();
    let mut unsaved = None;
    if intent == "save" || intent == "overwrite" {
        let version = if intent == "overwrite" {
            current.version_uid.clone()
        } else {
            opened.clone()
        };
        match update_directory(ehr_id.clone(), version, stripped(&working)).await {
            Ok(uid) => {
                redirect_with(
                    &tab_href(&ehr_id, EhrTab::Directory),
                    &Done::ok(tr(RecordsMsg::DirectorySaved)).line(tr_with(
                        RecordsMsg::DirectorySavedVersion,
                        &[("version", version_number(&uid).into())],
                    )),
                );
                return Form::Left;
            }
            Err(error) => unsaved = Some(Unsaved::new(tr(RecordsMsg::DirectorySaveRefused), error)),
        }
    }
    let picker = match adding {
        Some(folder) => {
            let compositions =
                crate::bff::ehr_detail::compositions::all_compositions(ehr_id.clone(), 0).await;
            let choices = compositions
                .map(|page| {
                    page.rows
                        .iter()
                        .map(|row| Choice {
                            uid: crate::uid::container_uid_of(
                                &row.first()
                                    .map(crate::bff::ehrs::cell_text)
                                    .unwrap_or_default(),
                            ),
                            name: row
                                .get(1)
                                .map(crate::bff::ehrs::cell_text)
                                .unwrap_or_default(),
                        })
                        .collect()
                })
                .map_err(|error| Refused::new(tr(RecordsMsg::DirectoryCompositionsRefused), error));
            Some(Picker { folder, choices })
        }
        None => None,
    };
    Form::Shown(Box::new(Editor {
        opened,
        current_uid: current.version_uid,
        working,
        renaming,
        changed,
        unsaved,
        picker,
    }))
}

/// The picker of a reference to add: the EHR's compositions to choose from,
/// or the CDR's refusal of the list where the choice would be.
fn picker_view(Picker { folder, choices }: Picker) -> Html {
    let choices = match choices {
        Ok(choices) => choices,
        Err(refusal) => return refusal.view(),
    };
    let options: Html = choices
        .into_iter()
        .map(|choice| {
            let words = format!("{} · {}", choice.name, shortened(&choice.uid));
            h(format!(
                r#"<option value="{}">{}</option>"#,
                t(&choice.uid),
                t(words)
            ))
        })
        .collect();
    h(format!(
        r#"<div class="fui-transform-options"><input type="hidden" name="reference_to" value="{folder}"><div class="fui-field is-id"><label for="d-reference" class="fui-label">{label}</label><select id="d-reference" name="reference" class="fui-select">{options}</select></div><div class="fui-transform-run"><button type="submit" name="intent" value="edit" class="fui-button fui-button-secondary">{glyph}{add}</button></div></div>"#,
        folder = t(&folder),
        label = t(tr(RecordsMsg::DirectoryReferenceLabel)),
        glyph = ferrohealth_ui::icon::icon(Glyph::FILE_PLUS, "16"),
        add = t(tr(RecordsMsg::DirectoryAddReference)),
    ))
}

/// Edit the directory's form, its decision bar docked.
fn edit_form(ehr_id: &str, editor: Editor) -> Html {
    let Editor {
        opened,
        current_uid,
        working,
        renaming,
        changed,
        unsaved,
        picker,
    } = editor;
    let conflict = unsaved.as_ref().is_some_and(|unsaved| unsaved.conflict);
    let line = if unsaved.is_some() {
        (Glyph::CIRCLE_X, tr(RecordsMsg::EditorNotSaved))
    } else if changed {
        (Glyph::PENCIL, tr(RecordsMsg::DirectoryChanged))
    } else {
        (Glyph::PENCIL, tr(RecordsMsg::EditorOpened))
    };
    let top = unsaved.map(|unsaved| {
        let ways = unsaved.conflict.then(|| {
            h(format!(
                r#"<div class="fui-cluster">{reload}<button type="submit" form="{EDIT_FORM}" name="intent" value="overwrite" class="fui-button fui-button-danger">{glyph}{overwrite}</button><span class="fui-small fui-muted">{note}</span></div>"#,
                reload = ferrohealth_ui::view::button::link_button(
                    format!("{}/edit", tab_href(ehr_id, EhrTab::Directory)),
                    ferrohealth_ui::view::button::Variant::Secondary,
                    Some(Glyph::REFRESH_CW),
                    tr_with(
                        RecordsMsg::EditorReloadVersion,
                        &[("version", version_number(&current_uid).into())],
                    ),
                ),
                glyph = ferrohealth_ui::icon::icon(Glyph::SAVE, "16"),
                overwrite = t(tr(RecordsMsg::DirectoryOverwrite)),
                note = t(tr(RecordsMsg::DirectoryOverwriteNote)),
            ))
        });
        unsaved.refusal.view().then(opt(ways))
    });
    let next = version_number(&opened).parse::<u32>().map_or_else(
        |_| tr(Msg::EditorSave),
        |number| {
            tr_with(
                RecordsMsg::EditorSaveAsVersion,
                &[("version", (number + 1).into())],
            )
        },
    );
    dock_editor(EditorDock {
        form: EDIT_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save: next,
        blocked: conflict.then(|| tr(RecordsMsg::DirectoryReloadOrOverwrite)),
    });
    let root_key = working
        .get("_key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let edits = Edits {
        root_key,
        renaming: renaming.clone(),
    };
    let picker = picker.map(picker_view);
    let rename_keep = renaming.map(|key| {
        h(format!(
            r#"<input type="hidden" name="renaming" value="{}"><button type="submit" name="intent" value="edit" class="fui-button fui-button-secondary fui-button-small">{}{}</button>"#,
            t(key),
            ferrohealth_ui::icon::icon(Glyph::CHECK, "14"),
            t(tr(RecordsMsg::DirectoryKeepName))
        ))
    });
    let body = section(
        Some(tr(RecordsMsg::DirectoryFoldersTitle)),
        None,
        None,
        Html::join([
            ferrohealth_ui::view::surface::section_intro(tr(RecordsMsg::DirectoryEditIntro)),
            tree(ehr_id, &working, Some(&edits)),
            opt(rename_keep),
            opt(picker),
        ]),
    );
    opt(top).then(h(format!(
        r#"<form id="{EDIT_FORM}" method="post" action="{action}"><input type="hidden" name="version" value="{version}"><input type="hidden" name="tree" value="{tree}">{panel}</form>"#,
        action = t(format!("{}/edit", tab_href(ehr_id, EhrTab::Directory))),
        version = t(&opened),
        tree = t(working.to_string()),
        panel = card_panel(body),
    )))
}

/// Edit the directory.
pub async fn edit_page() -> Drawn {
    let ehr_id = ehr_id_param();
    let body = match read_edit(ehr_id.clone()).await {
        Form::Shown(editor) => edit_form(&ehr_id, *editor),
        Form::Not(()) => quiet(tr(RecordsMsg::DirectoryNone)),
        Form::Refused(refusal) => refusal.view(),
        Form::Left => Html::empty(),
    };
    let mut trail = crumbs(&ehr_id);
    trail.push(Crumb::new(
        tr(RecordsMsg::EhrTabDirectory),
        tab_href(&ehr_id, EhrTab::Directory),
    ));
    let heading = editor_heading(
        &current(),
        message(RecordsMsg::EditorTitle),
        &Mode::Editing(tr(RecordsMsg::EhrTabDirectory)),
        Header {
            title: String::new().into(),
            subtitle: Some(tr(RecordsMsg::DirectoryEditSubtitle).into()),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    Drawn::page(
        tr(RecordsMsg::DirectoryEditTitle),
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

#[cfg(test)]
mod tests {
    use super::{Choice, Picker, picker_view};
    use crate::error::ViewerError;
    use crate::locale::records::RecordsMsg;
    use crate::locale::tr;
    use crate::pages::common::Refused;

    fn drawn(picker: Picker) -> String {
        String::from(picker_view(picker))
    }

    #[test]
    fn a_refused_composition_list_draws_its_refusal_in_place_of_the_picker() {
        let html = drawn(Picker {
            folder: "f1".to_owned(),
            choices: Err(Refused::new(
                tr(RecordsMsg::DirectoryCompositionsRefused),
                ViewerError::Cdr {
                    status: 403,
                    message: "the query is not allowed".to_owned(),
                },
            )),
        });
        assert!(
            html.contains(
                ferrohealth_ui::html::Html::text(tr(RecordsMsg::DirectoryCompositionsRefused))
                    .as_str()
            ),
            "the refusal's summary is drawn: {html}"
        );
        assert!(
            html.contains("the query is not allowed"),
            "the CDR's own words are drawn: {html}"
        );
        assert!(
            !html.contains("<select"),
            "no empty picker is drawn as if the EHR had no compositions: {html}"
        );
    }

    #[test]
    fn a_listed_composition_is_offered_by_the_picker() {
        let html = drawn(Picker {
            folder: "f1".to_owned(),
            choices: Ok(vec![Choice {
                uid: "0199c2a4-7e1b-4c3e-9a51-4f2d8b6e1a01".to_owned(),
                name: "Vital signs".to_owned(),
            }]),
        });
        assert!(html.contains("<select"), "the picker is drawn: {html}");
        assert!(
            html.contains("value=\"0199c2a4-7e1b-4c3e-9a51-4f2d8b6e1a01\""),
            "the composition is offered: {html}"
        );
        assert!(!html.contains(
            ferrohealth_ui::html::Html::text(tr(RecordsMsg::DirectoryCompositionsRefused)).as_str()
        ));
    }
}
