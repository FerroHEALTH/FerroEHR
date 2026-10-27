// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Tags tab of one object, a composition or a party: its `ITEM_TAG`s
//! with Remove on each, and the row that sets one (FerroUI
//! `design/consoles/ferroehr/`, the composition and party Tags tabs).
//!
//! Setting and removing are plain posts to the object's page. A tag stays on
//! the object across its versions and commits no version (RM
//! `org.openehr.rm.common.item_tag.adoc`); the set replaces the collection
//! with the tag merged in ([`crate::bff::item_tags::merged_tag_body`]). No
//! openEHR spec governs the viewer's pages: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, menu_cell,
};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::template::list::list_empty;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bff::item_tags::ItemTagRow;
use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{count, refused, section};

/// The prefix of a Remove's intent; the key follows it.
const REMOVE: &str = "remove-tag:";

/// What a post to a Tags tab asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagAct {
    /// Set the tag `key`, with its optional value and target path.
    Set {
        /// The tag's key.
        key: String,
        /// Its value, empty for none.
        value: String,
        /// Its target path, empty for the whole object.
        path: String,
    },
    /// Remove the tag `key`.
    Remove(String),
}

#[cfg(not(target_arch = "wasm32"))]
/// What the post asks the Tags tab, when it asks anything.
#[must_use]
pub fn tag_act() -> Option<TagAct> {
    let posted = crate::posted::posted()?;
    let intent = posted.intent();
    if intent == "set-tag" {
        return Some(TagAct::Set {
            key: posted.text("tag_key"),
            value: posted.text("tag_value"),
            path: posted.text("tag_path"),
        });
    }
    intent
        .strip_prefix(REMOVE)
        .map(|key| TagAct::Remove(key.to_owned()))
}

#[cfg(not(target_arch = "wasm32"))]
/// The toast that says what a tag act did to `object`.
#[must_use]
pub fn tag_done(act: &TagAct, object: &str) -> crate::flash::Done {
    match act {
        TagAct::Set { key, .. } => crate::flash::Done::ok(tr(Msg::TagSet)).line(tr_with(
            Msg::TagSetLine,
            &[
                ("key", key.clone().into()),
                ("object", object.to_owned().into()),
            ],
        )),
        TagAct::Remove(key) => crate::flash::Done::ok(tr(Msg::TagRemoved)).line(tr_with(
            Msg::TagRemovedLine,
            &[
                ("key", key.clone().into()),
                ("object", object.to_owned().into()),
            ],
        )),
    }
}

/// What a Tags tab draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagPanel {
    /// What the object is, in the reader's words: "composition", "party".
    pub collection: String,
    /// Its tags, or the refusal that kept them out of reach.
    pub tags: Result<Vec<ItemTagRow>, ViewerError>,
    /// The page the forms post to.
    pub action: String,
    /// The refusal of the post that drew the page, with the draft it kept.
    pub refused: Option<(ViewerError, TagAct)>,
}

/// The tags of one object and the row that sets one, as two sections.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
pub fn tag_panel(panel: TagPanel) -> AnyView {
    let TagPanel {
        collection,
        tags,
        action,
        refused: refusal,
    } = panel;
    let columns = [
        Column::text("key", tr(Msg::TagsKey)).role(Role::Title),
        Column::text("value", tr(Msg::TagsValue)).role(Role::Sub),
        Column::text("path", tr(Msg::TagsTargetPath)).role(Role::Meta),
        Column::text("remove", tr(Msg::TagRemoveColumn)).role(Role::Menu),
    ];
    let (list, shown) = match tags {
        Ok(tags) if tags.is_empty() => (
            list_empty(
                tr_with(Msg::TagNone, &[("object", collection.clone().into())]),
                None,
            ),
            Some(0),
        ),
        Ok(tags) => {
            let total = tags.len();
            let rows = tags
                .into_iter()
                .map(|tag| {
                    let path = if tag.target_path.is_empty() {
                        view! {
                            <span class="fui-muted">
                                {tr_with(Msg::TagWhole, &[("object", collection.clone().into())])}
                            </span>
                        }
                        .into_any()
                    } else {
                        view! { <code>{tag.target_path.clone()}</code> }.into_any()
                    };
                    let remove = view! {
                        <button
                            type="submit"
                            form="tag-remove"
                            name="intent"
                            value=format!("{REMOVE}{}", tag.key)
                            class="fui-icon-button"
                            aria-label=tr_with(Msg::TagRemove, &[("key", tag.key.clone().into())])
                        >
                            {ferrohealth_ui::icon::icon(Glyph::X, "16")}
                        </button>
                    };
                    view! {
                        <tr>
                            <th scope="row" data-role="title">
                                <code>{tag.key}</code>
                            </th>
                            {cell(&columns[1], tag.value)}
                            {cell(&columns[2], path)}
                            {menu_cell(remove)}
                        </tr>
                    }
                    .into_any()
                })
                .collect_view()
                .into_any();
            let caption = tr(Msg::EhrTabTags);
            let table = data_table(DataTable {
                caption: &caption,
                columns: &columns,
                sorting: Sorting::default(),
                body: Body::Rows(rows),
            })
            .into_any();
            (table, Some(total))
        }
        Err(error) => (refused(tr(Msg::TagsRefused), &error, None), None),
    };
    let (top, draft) = match refusal {
        Some((error, act)) => (Some(refused(tr(Msg::TagRefused), &error, None)), Some(act)),
        None => (None, None),
    };
    let (key, value, path) = match draft {
        Some(TagAct::Set { key, value, path }) => (key, value, path),
        _ => (String::new(), String::new(), String::new()),
    };
    let set = view! {
        <form method="post" action=action.clone() class="fui-clause-filter">
            {Field::new("tk", tr(Msg::TagsKey))
                .view(
                    view! {
                        <input
                            id="tk"
                            name="tag_key"
                            type="text"
                            class="fui-input fui-mono"
                            value=key
                            required
                        />
                    },
                )}
            {Field::new("tv", tr(Msg::TagValueOptional))
                .view(
                    view! {
                        <input id="tv" name="tag_value" type="text" class="fui-input" value=value />
                    },
                )}
            {Field::new("tp", tr(Msg::TagPathOptional))
                .view(
                    view! {
                        <input
                            id="tp"
                            name="tag_path"
                            type="text"
                            class="fui-input fui-mono"
                            value=path
                        />
                    },
                )}
            <button
                type="submit"
                name="intent"
                value="set-tag"
                class="fui-button fui-button-secondary"
            >
                {ferrohealth_ui::icon::icon(Glyph::TAGS, "16")}
                {tr(Msg::TagSetAction)}
            </button>
        </form>
    };
    view! {
        <form id="tag-remove" method="post" action=action hidden></form>
        {section(
            Some(tr_with(Msg::TagsOf, &[("object", collection.clone().into())])),
            shown.map(|shown| u64::try_from(shown).unwrap_or(u64::MAX)),
            None,
            view! {
                {ferrohealth_ui::view::surface::section_intro(
                    tr_with(Msg::TagIntro, &[("object", collection.clone().into())]),
                )}
                {top}
                {list}
            },
        )}
        {section(Some(tr(Msg::TagSetTitle)), None, None, set)}
    }
    .into_any()
}

/// The count words of a Tags tab.
#[must_use]
pub fn tag_count_words(total: usize) -> String {
    tr_with(Msg::TagCount, &[("count", count(total))])
}
