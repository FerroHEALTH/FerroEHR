// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Tags tab: every `ITEM_TAG` in the EHR, grouped by the object it
//! is set on (FerroUI `design/consoles/ferroehr/`, page 12.13).
//!
//! The list is the CDR's aggregate read (ITS-REST `ehr_tags_get`), which
//! spans the version containers and single versions of every taggable kind.
//! A tag is set on its object's own Tags tab; this list reads them all. The
//! key and value filters are a `GET` form sent to the CDR as its released
//! filters, so a filtered list keeps its address. No openEHR spec governs
//! the viewer's pages: our own design.

use ferrohealth_ui::table::filter::{Find, find_form};
use ferrohealth_ui::table::model::StateSource;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, group_row,
};
use ferrohealth_ui::view::template::list::{ActiveFilters, FilterChip, active_filters, list_empty};
use leptos::prelude::*;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::bff::ehr_tags::{EhrTargetKind, list_ehr_tags, resolve_ehr_target};
#[cfg(not(target_arch = "wasm32"))]
use crate::bff::item_tags::{TagGroup, group_by_target};
#[cfg(not(target_arch = "wasm32"))]
use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{Refused, drawn, page_model, shortened};
#[cfg(not(target_arch = "wasm32"))]
use crate::pages::ehr::EhrModel;
use crate::pages::ehr::{EhrTab, ehr_id_param, ehr_view, tab_href};

/// The filters the address names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Filters {
    /// The exact key.
    key: String,
    /// The exact value.
    value: String,
}

impl Filters {
    #[cfg(not(target_arch = "wasm32"))]
    /// The filters of the request.
    fn read() -> Self {
        Self {
            key: crate::request::query("key").unwrap_or_default(),
            value: crate::request::query("value").unwrap_or_default(),
        }
    }

    /// The address of the tab with the filter `without` dropped.
    fn href_without(&self, ehr_id: &str, without: &str) -> String {
        let pairs: Vec<String> = [("key", &self.key), ("value", &self.value)]
            .into_iter()
            .filter(|(name, value)| *name != without && !value.is_empty())
            .map(|(name, value)| format!("{name}={}", urlencoding::encode(value)))
            .collect();
        let base = tab_href(ehr_id, EhrTab::Tags);
        if pairs.is_empty() {
            base
        } else {
            format!("{base}?{}", pairs.join("&"))
        }
    }
}

/// The filter toolbar: the key as the find field, the value beside it.
fn toolbar(ehr_id: &str, filters: &Filters) -> AnyView {
    let action = tab_href(ehr_id, EhrTab::Tags);
    let tools = view! {
        <div class="fui-list-find">
            <label for="t-value" class="fui-sr-only">
                {tr(Msg::TagsValue)}
            </label>
            <input
                id="t-value"
                name="value"
                type="text"
                class="fui-input"
                value=filters.value.clone()
                placeholder=tr(Msg::TagsValue)
                autocomplete="off"
            />
        </div>
        <button type="submit" class="fui-button fui-button-secondary">
            {ferrohealth_ui::icon::icon(ferrohealth_ui::icon::Glyph::FUNNEL, "16")}
            {tr(Msg::FilterApply)}
        </button>
    };
    let label = tr(Msg::TagsKey);
    find_form(
        Find {
            id: "t-key",
            name: "key",
            label: &label,
            value: &filters.key,
            source: StateSource::Url,
            action: Some(&action),
            fields: &[],
        },
        tools,
    )
}

/// The active filters as chips.
fn chips(ehr_id: &str, filters: &Filters) -> Option<AnyView> {
    let chips: Vec<FilterChip> = [
        ("key", &filters.key, Msg::ChipKey),
        ("value", &filters.value, Msg::ChipValue),
    ]
    .into_iter()
    .filter(|(_, value, _)| !value.is_empty())
    .map(|(name, value, msg)| FilterChip {
        label: tr_with(msg, &[("value", value.clone().into())]),
        remove: filters.href_without(ehr_id, name),
    })
    .collect();
    (!chips.is_empty()).then(|| {
        active_filters(
            ActiveFilters {
                chips,
                clear: tab_href(ehr_id, EhrTab::Tags),
                shown: None,
            },
            None,
        )
    })
}

/// A tagged object as its group heading names it: what it is, where it opens,
/// and whether the tags sit on the whole object or one version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Target {
    /// Its name.
    name: String,
    /// Its page, when the viewer has one.
    href: Option<String>,
    /// Which form the tags target.
    form: String,
}

/// One tag of a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Tag {
    /// Its key.
    key: String,
    /// Its value, empty when it carries none.
    value: String,
    /// The path it targets, empty for the whole object.
    target_path: String,
}

/// The tags set on one object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Tagged {
    /// The object.
    target: Target,
    /// Its tags.
    tags: Vec<Tag>,
}

/// The tab's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum List {
    /// No tag matches.
    Empty,
    /// The tags, grouped by their object.
    Groups(Vec<Tagged>),
    /// The CDR's refusal.
    Refused(Refused),
}

/// The tab's model: the filters it read and the list they found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Panel {
    /// The filters.
    filters: Filters,
    /// The list.
    list: List,
}

#[cfg(not(target_arch = "wasm32"))]
/// Names the object a group of tags is set on, asking the CDR which kind it
/// is.
async fn target(ehr_id: &str, group: &TagGroup) -> Result<Target, ViewerError> {
    let container = crate::uid::container_uid_of(&group.target);
    let version =
        (container != group.target).then(|| crate::pages::common::version_number(&group.target));
    let kind = resolve_ehr_target(ehr_id.to_owned(), group.target.clone())
        .await?
        .and_then(|segment| EhrTargetKind::from_segment(&segment));
    let (name, versioned) = match kind {
        Some(EhrTargetKind::Composition) => (
            tr_with(
                Msg::TagsComposition,
                &[("id", shortened(&container).into())],
            ),
            tr(Msg::TagsVersionedComposition),
        ),
        Some(EhrTargetKind::EhrStatus) => (tr(Msg::TagsEhrStatus), tr(Msg::TagsVersionedStatus)),
        Some(EhrTargetKind::Directory) => {
            (tr(Msg::EhrTabDirectory), tr(Msg::TagsVersionedDirectory))
        }
        None => (shortened(&container), tr(Msg::TagsUnknownObject)),
    };
    let form = match version {
        Some(number) => tr_with(Msg::TagsOneVersion, &[("version", number.into())]),
        None => versioned,
    };
    Ok(Target {
        name,
        href: kind.map(|kind| kind.href(ehr_id, &container)),
        form,
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// The tags of `groups`, each group's object named.
async fn tagged(ehr_id: &str, groups: Vec<TagGroup>) -> Result<Vec<Tagged>, ViewerError> {
    let mut named = Vec::new();
    for group in groups {
        let target = target(ehr_id, &group).await?;
        let tags = group
            .tags
            .into_iter()
            .map(|tag| Tag {
                key: tag.key,
                value: tag.value,
                target_path: tag.target_path,
            })
            .collect();
        named.push(Tagged { target, tags });
    }
    Ok(named)
}

#[cfg(not(target_arch = "wasm32"))]
/// Loads the tab: the tags the request's filters find.
async fn load(ehr_id: String) -> EhrModel<Panel> {
    crate::pages::ehr::load_ehr(&ehr_id, async |_facts| {
        let filters = Filters::read();
        let tags = list_ehr_tags(
            ehr_id.clone(),
            filters.key.clone(),
            filters.value.clone(),
            String::new(),
        )
        .await;
        let list = match tags {
            Ok(tags) if tags.is_empty() => List::Empty,
            Ok(tags) => match tagged(&ehr_id, group_by_target(tags)).await {
                Ok(groups) => List::Groups(groups),
                Err(error) => List::Refused(Refused::new(tr(Msg::TagsRefused), error)),
            },
            Err(error) => List::Refused(Refused::new(tr(Msg::TagsRefused), error)),
        };
        Panel { filters, list }
    })
    .await
}

/// The tags, grouped by their object.
fn table(groups: Vec<Tagged>) -> AnyView {
    let columns = [
        Column::text("key", tr(Msg::TagsKey)).role(Role::Title),
        Column::text("value", tr(Msg::TagsValue)).role(Role::Sub),
        Column::text("path", tr(Msg::TagsTargetPath)).role(Role::Meta),
    ];
    let mut rows = Vec::new();
    for group in groups {
        let target = group.target;
        let name = match target.href {
            Some(href) => view! { <a href=href>{target.name}</a> }.into_any(),
            None => target.name.into_any(),
        };
        rows.push(group_row(
            &columns,
            view! {
                {name}
                <span class="fui-small fui-muted">{format!(" · {}", target.form)}</span>
            },
        ));
        for tag in group.tags {
            let path = if tag.target_path.is_empty() {
                view! { <span class="fui-muted">{tr(Msg::TagsWholeObject)}</span> }.into_any()
            } else {
                view! { <code>{tag.target_path}</code> }.into_any()
            };
            rows.push(
                view! {
                    <tr>
                        <th scope="row" data-role="title">
                            <code>{tag.key}</code>
                        </th>
                        {cell(&columns[1], tag.value)}
                        {cell(&columns[2], path)}
                    </tr>
                }
                .into_any(),
            );
        }
    }
    let caption = tr(Msg::TagsCaption);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(rows.collect_view().into_any()),
    })
    .into_any()
}

/// The tab's panel.
fn panel(ehr_id: &str, panel: Panel) -> AnyView {
    let Panel { filters, list } = panel;
    let list = match list {
        List::Empty => list_empty(
            tr(if filters.key.is_empty() && filters.value.is_empty() {
                Msg::TagsEmpty
            } else {
                Msg::TagsNoMatch
            }),
            None,
        ),
        List::Groups(groups) => table(groups),
        List::Refused(refusal) => refusal.view(),
    };
    view! {
        <div class="fui-panel">
            <div class="fui-panel-body fui-stack">
                <p class="fui-small fui-muted">{tr(Msg::TagsIntro)}</p>
                {toolbar(ehr_id, &filters)}
                {chips(ehr_id, &filters)}
            </div>
            {list}
        </div>
    }
    .into_any()
}

/// An EHR's Tags tab.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn TagsTab() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let title = tr_with(Msg::TagsTitle, &[("id", shortened(&ehr_id).into())]);
    let model = page_model!(load(ehr_id.clone()));
    let body = drawn(model, move |model| {
        ehr_view(&ehr_id, EhrTab::Tags, model, |found| panel(&ehr_id, found))
    });
    view! {
        <Title text=title />
        {body}
    }
}
