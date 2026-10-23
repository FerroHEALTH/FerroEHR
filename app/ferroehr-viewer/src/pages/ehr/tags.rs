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

use crate::bff::ehr_tags::{EhrTargetKind, list_ehr_tags, resolve_ehr_target};
use crate::bff::item_tags::{TagGroup, group_by_target};
use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{awaited, refused, shortened};
use crate::pages::ehr::{EhrTab, ehr_id_param, ehr_page, tab_href};

/// The filters the address names.
#[derive(Debug, Clone, Default)]
struct Filters {
    /// The exact key.
    key: String,
    /// The exact value.
    value: String,
}

impl Filters {
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
#[derive(Debug, Clone)]
struct Target {
    /// Its name.
    name: String,
    /// Its page, when the viewer has one.
    href: Option<String>,
    /// Which form the tags target.
    form: String,
}

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

/// The tags, grouped by their object.
async fn table(ehr_id: &str, groups: Vec<TagGroup>) -> Result<AnyView, ViewerError> {
    let columns = [
        Column::text("key", tr(Msg::TagsKey)).role(Role::Title),
        Column::text("value", tr(Msg::TagsValue)).role(Role::Sub),
        Column::text("path", tr(Msg::TagsTargetPath)).role(Role::Meta),
    ];
    let mut rows = Vec::new();
    for group in groups {
        let target = target(ehr_id, &group).await?;
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
    Ok(data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(rows.collect_view().into_any()),
    })
    .into_any())
}

/// An EHR's Tags tab.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn TagsTab() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let filters = Filters::read();
    let title = tr_with(Msg::TagsTitle, &[("id", shortened(&ehr_id).into())]);
    let body = awaited({
        let ehr_id = ehr_id.clone();
        async move {
            ehr_page(ehr_id.clone(), EhrTab::Tags, move |_facts| async move {
                let tags = list_ehr_tags(
                    ehr_id.clone(),
                    filters.key.clone(),
                    filters.value.clone(),
                    String::new(),
                )
                .await;
                let list = match tags {
                    Ok(tags) if tags.is_empty() => list_empty(
                        tr(if filters.key.is_empty() && filters.value.is_empty() {
                            Msg::TagsEmpty
                        } else {
                            Msg::TagsNoMatch
                        }),
                        None,
                    ),
                    Ok(tags) => match table(&ehr_id, group_by_target(tags)).await {
                        Ok(table) => table,
                        Err(error) => refused(tr(Msg::TagsRefused), &error, None),
                    },
                    Err(error) => refused(tr(Msg::TagsRefused), &error, None),
                };
                view! {
                    <div class="fui-panel">
                        <div class="fui-panel-body fui-stack">
                            <p class="fui-small fui-muted">{tr(Msg::TagsIntro)}</p>
                            {toolbar(&ehr_id, &filters)}
                            {chips(&ehr_id, &filters)}
                        </div>
                        {list}
                    </div>
                }
                .into_any()
            })
            .await
        }
    });
    view! {
        <Title text=title />
        {body}
    }
}
