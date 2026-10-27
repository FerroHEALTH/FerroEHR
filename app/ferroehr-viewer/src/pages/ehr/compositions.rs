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
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::bff::ehrs::cell_text;
use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{Refused, ago, drawn, page_model, shortened};
use crate::pages::ehr::EhrModel;
use crate::pages::ehr::{EhrTab, ehr_href, ehr_id_param, ehr_view};
use crate::paging::Listing;

/// The filters the address names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    #[cfg(not(target_arch = "wasm32"))]
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

/// One composition of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Row {
    /// Its version id.
    uid: String,
    /// Its name.
    name: String,
    /// Its template.
    template: String,
    /// Its start time.
    start: String,
    /// Its composer's name.
    composer: String,
}

/// The tab's model: the filters it read and the page of compositions they
/// found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Panel {
    /// The filters.
    filters: Filters,
    /// The page, or the CDR's refusal of it.
    rows: Result<Vec<Row>, Refused>,
}

/// Loads the tab: the page of compositions the request's filters find.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load(
    /// The path and query of the page.
    href: String,
    /// The EHR the route names.
    ehr_id: String,
) -> Result<EhrModel<Panel>, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_page(ehr_id).await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load`] reads, on the server.
async fn read_page(ehr_id: String) -> EhrModel<Panel> {
    crate::pages::ehr::load_ehr(&ehr_id, async |_facts| {
        let filters = Filters::read();
        let window = OpenWindow::of(crate::paging::current(Listing::Compositions), 0);
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
        let rows = page
            .map(|page| {
                page.rows
                    .iter()
                    .map(|row| {
                        let text = |at: usize| row.get(at).map(cell_text).unwrap_or_default();
                        Row {
                            uid: text(0),
                            name: text(1),
                            template: text(2),
                            start: text(3),
                            composer: text(4),
                        }
                    })
                    .collect()
            })
            .map_err(|error| Refused::new(tr(Msg::CompositionsRefused), error));
        Panel { filters, rows }
    })
    .await
}

/// The tab's panel.
fn panel(ehr_id: &str, panel: Panel) -> AnyView {
    let Panel { filters, rows } = panel;
    let rows = match rows {
        Ok(rows) => rows,
        Err(refusal) => {
            return view! {
                <div class="fui-panel">
                    <div class="fui-panel-body fui-stack">{toolbar(ehr_id, &filters)}</div>
                    {refusal.view()}
                </div>
            }
            .into_any();
        }
    };
    let paging = crate::paging::current(Listing::Compositions);
    let fields = crate::paging::fields(Listing::Compositions);
    let held = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    let window = OpenWindow::of(paging, held);
    let columns = [
        Column::text("composition", tr(Msg::ColumnComposition)).role(Role::Title),
        Column::text("start", tr(Msg::ColumnStartTime)).role(Role::Sub),
        Column::text("composer", tr(Msg::ColumnComposer)).role(Role::Meta),
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
    ];
    let empty = rows.is_empty();
    let body = rows
        .into_iter()
        .map(|row| {
            let version = row.uid.rsplit("::").next().unwrap_or_default().to_owned();
            let href = composition_href(ehr_id, &crate::uid::container_uid_of(&row.uid));
            let sub = view! { <span class="fui-row-sub">{row.template}</span> }.into_any();
            let title = title_cell(Some(&href), row.name, Some(sub));
            let start = cell(&columns[1], ago(&row.start));
            let composer = cell(&columns[2], row.composer);
            let version = cell(&columns[3], version);
            view! { <tr>{title} {start} {composer} {version}</tr> }.into_any()
        })
        .collect_view()
        .into_any();
    let table = if empty && window.page == 1 {
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
            body: Body::Rows(body),
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
                {toolbar(ehr_id, &filters)} {chips(ehr_id, &filters)}
            </div>
            {table}
            {pager.map(|pager| view! { <div class="fui-list-foot">{pager}</div> })}
        </div>
    }
    .into_any()
}

/// An EHR's Compositions tab.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn CompositionsTab() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let title = tr_with(Msg::EhrTitle, &[("id", shortened(&ehr_id).into())]);
    let model = page_model!(load(ehr_id.clone()));
    let body = drawn(model, move |model| {
        ehr_view(&ehr_id, EhrTab::Compositions, model, |found| {
            panel(&ehr_id, found)
        })
    });
    view! {
        <Title text=title />
        {body}
    }
}
