// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Compositions tab: the compositions AQL finds in it, newest
//! first, with the template, start-time and composer filters as a toolbar.
//!
//! The filters are a `GET` form, so a filtered list keeps its address; each
//! value travels as an AQL binding, never as query text
//! ([`crate::bff::ehr_detail::composition_filter`]). The list states no total,
//! so it pages with Previous and Next, its position kept in the browser's
//! session ([`crate::paging`]). No openEHR spec governs the viewer's pages:
//! our own design.

use ferrohealth_ui::table::filter::{Find, find_form};
use ferrohealth_ui::table::model::{OpenWindow, StateSource};
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_open_pager};
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::template::list::{ActiveFilters, FilterChip, active_filters, list_empty};
use leptos::prelude::*;
use leptos_meta::Title;

use crate::bff::ehrs::cell_text;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{ago, awaited, refused, shortened};
use crate::pages::ehr::{EhrTab, ehr_href, ehr_id_param, ehr_page};
use crate::paging::Listing;

/// The filters the address names.
#[derive(Debug, Clone, Default)]
struct Filters {
    /// A part of the template id.
    template: String,
    /// The earliest start time.
    from: String,
    /// The latest start time.
    to: String,
    /// A part of the composer's name.
    composer: String,
}

impl Filters {
    /// The filters of the request.
    fn read() -> Self {
        let get = |key: &str| crate::request::query(key).unwrap_or_default();
        Self {
            template: get("template"),
            from: get("from"),
            to: get("to"),
            composer: get("composer"),
        }
    }

    /// Whether any filter is set.
    fn any(&self) -> bool {
        !(self.template.is_empty()
            && self.from.is_empty()
            && self.to.is_empty()
            && self.composer.is_empty())
    }

    /// The address of the tab with these filters but `without`.
    fn href_without(&self, ehr_id: &str, without: &str) -> String {
        let pairs: Vec<String> = [
            ("template", &self.template),
            ("from", &self.from),
            ("to", &self.to),
            ("composer", &self.composer),
        ]
        .into_iter()
        .filter(|(key, value)| *key != without && !value.is_empty())
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect();
        if pairs.is_empty() {
            ehr_href(ehr_id)
        } else {
            format!("{}?{}", ehr_href(ehr_id), pairs.join("&"))
        }
    }
}

/// The page of the composition `uid` of the EHR `ehr_id`.
#[must_use]
pub fn composition_href(ehr_id: &str, uid: &str) -> String {
    crate::pages::composition::composition_href(ehr_id, uid)
}

/// The filter toolbar: the composer as the find field, the template and the
/// start-time range beside it.
fn toolbar(ehr_id: &str, filters: &Filters) -> AnyView {
    let action = ehr_href(ehr_id);
    let tools = view! {
        <div class="fui-list-find">
            <label for="f-template" class="fui-sr-only">
                {tr(Msg::FilterTemplate)}
            </label>
            <input
                id="f-template"
                name="template"
                type="text"
                class="fui-input"
                value=filters.template.clone()
                placeholder=tr(Msg::FilterTemplate)
                autocomplete="off"
            />
        </div>
        {crate::pages::common::date_range(
            "f-start",
            tr(Msg::FilterStartTime),
            &filters.from,
            &filters.to,
        )}
        <button type="submit" class="fui-button fui-button-secondary">
            {ferrohealth_ui::icon::icon(ferrohealth_ui::icon::Glyph::FUNNEL, "16")}
            {tr(Msg::FilterApply)}
        </button>
    };
    let label = tr(Msg::FilterComposer);
    find_form(
        Find {
            id: "f-composer",
            name: "composer",
            label: &label,
            value: &filters.composer,
            source: StateSource::Url,
            action: Some(&action),
            fields: &[],
        },
        tools,
    )
}

/// The active filters as chips, each with its way to remove it.
fn chips(ehr_id: &str, filters: &Filters) -> Option<AnyView> {
    if !filters.any() {
        return None;
    }
    let mut chips = Vec::new();
    for (key, value, msg) in [
        ("template", &filters.template, Msg::ChipTemplate),
        ("from", &filters.from, Msg::ChipFrom),
        ("to", &filters.to, Msg::ChipTo),
        ("composer", &filters.composer, Msg::ChipComposer),
    ] {
        if !value.is_empty() {
            chips.push(FilterChip {
                label: tr_with(msg, &[("value", value.clone().into())]),
                remove: filters.href_without(ehr_id, key),
            });
        }
    }
    Some(active_filters(
        ActiveFilters {
            chips,
            clear: ehr_href(ehr_id),
            shown: None,
        },
        None,
    ))
}

/// An EHR's Compositions tab.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn CompositionsTab() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let filters = Filters::read();
    let paging = crate::paging::current(Listing::Compositions);
    let fields = crate::paging::fields(Listing::Compositions);
    let title = tr_with(Msg::EhrTitle, &[("id", shortened(&ehr_id).into())]);
    let body = awaited({
        let ehr_id = ehr_id.clone();
        async move {
            ehr_page(
                ehr_id.clone(),
                EhrTab::Compositions,
                move |_facts| async move {
                    let window = OpenWindow::of(paging, 0);
                    let page = crate::bff::ehr_detail::compositions::list_compositions(
                        ehr_id.clone(),
                        filters.template.clone(),
                        filters.from.clone(),
                        filters.to.clone(),
                        filters.composer.clone(),
                        window.start,
                        window.size,
                    )
                    .await;
                    let page = match page {
                        Ok(page) => page,
                        Err(error) => {
                            return view! {
                                <div class="fui-panel">
                                    <div class="fui-panel-body fui-stack">
                                        {toolbar(&ehr_id, &filters)}
                                    </div>
                                    {refused(tr(Msg::CompositionsRefused), &error, None)}
                                </div>
                            }
                            .into_any();
                        }
                    };
                    let held = u32::try_from(page.rows.len()).unwrap_or(u32::MAX);
                    let window = OpenWindow::of(paging, held);
                    let columns = [
                        Column::text("composition", tr(Msg::ColumnComposition)).role(Role::Title),
                        Column::text("start", tr(Msg::ColumnStartTime)).role(Role::Sub),
                        Column::text("composer", tr(Msg::ColumnComposer)).role(Role::Meta),
                        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
                    ];
                    let rows = page
                        .rows
                        .iter()
                        .map(|row| {
                            let uid = row.first().map(cell_text).unwrap_or_default();
                            let name = row.get(1).map(cell_text).unwrap_or_default();
                            let template = row.get(2).map(cell_text).unwrap_or_default();
                            let start = row.get(3).map(cell_text).unwrap_or_default();
                            let composer = row.get(4).map(cell_text).unwrap_or_default();
                            let version = uid.rsplit("::").next().unwrap_or_default().to_owned();
                            let href =
                                composition_href(&ehr_id, &crate::uid::container_uid_of(&uid));
                            let sub =
                                view! { <span class="fui-row-sub">{template}</span> }.into_any();
                            let title = title_cell(Some(&href), name, Some(sub));
                            let start = cell(&columns[1], ago(&start));
                            let composer = cell(&columns[2], composer);
                            let version = cell(&columns[3], version);
                            view! { <tr>{title} {start} {composer} {version}</tr> }.into_any()
                        })
                        .collect_view()
                        .into_any();
                    let table = if page.rows.is_empty() && window.page == 1 {
                        let sentence = if filters.any() {
                            tr(Msg::CompositionsNoMatch)
                        } else {
                            tr(Msg::CompositionsEmpty)
                        };
                        list_empty(sentence, None)
                    } else {
                        let caption = tr(Msg::EhrTabCompositions);
                        data_table(DataTable {
                            caption: &caption,
                            columns: &columns,
                            sorting: Sorting::default(),
                            body: Body::Rows(rows),
                        })
                        .into_any()
                    };
                    let pager = (window.page > 1 || window.has_next()).then(|| {
                        post_open_pager(
                            PagerListing::new(tr(Msg::CompositionsPages)),
                            window,
                            PostTo {
                                id: "compositions-pager",
                                action: crate::paging::PAGING_ROUTE,
                                fields: &fields,
                            },
                        )
                    });
                    view! {
                        <div class="fui-panel">
                            <div class="fui-panel-body fui-stack">
                                {toolbar(&ehr_id, &filters)} {chips(&ehr_id, &filters)}
                            </div>
                            {table}
                            {pager.map(|pager| view! { <div class="fui-list-foot">{pager}</div> })}
                        </div>
                    }
                    .into_any()
                },
            )
            .await
        }
    });
    view! {
        <Title text=title />
        {body}
    }
}
