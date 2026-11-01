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

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, menu_cell,
};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::template::list::list_empty;

use crate::bff::item_tags::ItemTagRow;
use crate::error::ViewerError;
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{refused, section};

/// The prefix of a Remove's intent; the key follows it.
const REMOVE: &str = "remove-tag:";

/// What a post to a Tags tab asks.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// The toast that says what a tag act did to `object`.
#[must_use]
pub fn tag_done(act: &TagAct, object: &str) -> crate::flash::Done {
    match act {
        TagAct::Set { key, .. } => crate::flash::Done::ok(tr(RecordsMsg::TagSet)).line(tr_with(
            RecordsMsg::TagSetLine,
            &[
                ("key", key.clone().into()),
                ("object", object.to_owned().into()),
            ],
        )),
        TagAct::Remove(key) => crate::flash::Done::ok(tr(RecordsMsg::TagRemoved)).line(tr_with(
            RecordsMsg::TagRemovedLine,
            &[
                ("key", key.clone().into()),
                ("object", object.to_owned().into()),
            ],
        )),
    }
}

/// What a Tags tab draws.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    reason = "one panel: the tags, the refusal it drew and the row that sets one"
)]
pub fn tag_panel(panel: TagPanel) -> Html {
    let TagPanel {
        collection,
        tags,
        action,
        refused: refusal,
    } = panel;
    let columns = [
        Column::text("key", tr(RecordsMsg::TagsKey)).role(Role::Title),
        Column::text("value", tr(Msg::TagsValue)).role(Role::Sub),
        Column::text("path", tr(RecordsMsg::TagsTargetPath)).role(Role::Meta),
        Column::text("remove", tr(RecordsMsg::TagRemoveColumn)).role(Role::Menu),
    ];
    let (list, shown) = match tags {
        Ok(tags) if tags.is_empty() => (
            list_empty(
                tr_with(
                    RecordsMsg::TagNone,
                    &[("object", collection.clone().into())],
                ),
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
                        h(format!(
                            r#"<span class="fui-muted">{}</span>"#,
                            t(tr_with(
                                RecordsMsg::TagWhole,
                                &[("object", collection.clone().into())],
                            ))
                        ))
                    } else {
                        h(format!("<code>{}</code>", t(&tag.target_path)))
                    };
                    let remove = h(format!(
                        r#"<button type="submit" form="tag-remove" name="intent" value="{value}" class="fui-icon-button" aria-label="{label}">{glyph}</button>"#,
                        value = t(format!("{REMOVE}{}", tag.key)),
                        label = t(tr_with(
                            RecordsMsg::TagRemove,
                            &[("key", tag.key.clone().into())],
                        )),
                        glyph = ferrohealth_ui::icon::icon(Glyph::X, "16"),
                    ));
                    h(format!(
                        r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}{}{}</tr>"#,
                        t(&tag.key),
                        cell(&columns[1], t(&tag.value)),
                        cell(&columns[2], path),
                        menu_cell(remove)
                    ))
                })
                .collect();
            let caption = tr(RecordsMsg::EhrTabTags);
            let table = data_table(DataTable {
                caption: &caption,
                columns: &columns,
                sorting: Sorting::default(),
                body: Body::Rows(rows),
            });
            (table, Some(total))
        }
        Err(error) => (refused(tr(RecordsMsg::TagsRefused), &error, None), None),
    };
    let (top, draft) = match refusal {
        Some((error, act)) => (
            Some(refused(tr(RecordsMsg::TagRefused), &error, None)),
            Some(act),
        ),
        None => (None, None),
    };
    let (key, value, path) = match draft {
        Some(TagAct::Set { key, value, path }) => (key, value, path),
        _ => (String::new(), String::new(), String::new()),
    };
    let key = Field::new("tk", tr(RecordsMsg::TagsKey)).view(h(format!(
        r#"<input id="tk" name="tag_key" type="text" class="fui-input fui-mono" value="{}" required>"#,
        t(key)
    )));
    let value = Field::new("tv", tr(RecordsMsg::TagValueOptional)).view(h(format!(
        r#"<input id="tv" name="tag_value" type="text" class="fui-input" value="{}">"#,
        t(value)
    )));
    let path = Field::new("tp", tr(RecordsMsg::TagPathOptional)).view(h(format!(
        r#"<input id="tp" name="tag_path" type="text" class="fui-input fui-mono" value="{}">"#,
        t(path)
    )));
    let set = h(format!(
        r#"<form method="post" action="{action}" class="fui-clause-filter">{key}{value}{path}<button type="submit" name="intent" value="set-tag" class="fui-button fui-button-secondary">{glyph}{words}</button></form>"#,
        action = t(&action),
        glyph = ferrohealth_ui::icon::icon(Glyph::TAGS, "16"),
        words = t(tr(RecordsMsg::TagSetAction)),
    ));
    let held = section(
        Some(tr_with(
            RecordsMsg::TagsOf,
            &[("object", collection.clone().into())],
        )),
        shown.map(|shown| u64::try_from(shown).unwrap_or(u64::MAX)),
        None,
        Html::join([
            ferrohealth_ui::view::surface::section_intro(tr_with(
                RecordsMsg::TagIntro,
                &[("object", collection.clone().into())],
            )),
            opt(top),
            list,
        ]),
    );
    Html::join([
        h(format!(
            r#"<form id="tag-remove" method="post" action="{}" hidden></form>"#,
            t(&action)
        )),
        held,
        section(Some(tr(RecordsMsg::TagSetTitle)), None, None, set),
    ])
}
