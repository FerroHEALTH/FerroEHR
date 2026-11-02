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

use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::filter::{Find, find_form};
use ferrohealth_ui::table::model::{OpenWindow, StateSource};
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_open_pager};
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::template::list::{ActiveFilters, FilterChip, active_filters, list_empty};

use crate::bff::ehrs::cell_text;
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{Refused, ago, shortened};
use crate::pages::ehr::EhrModel;
use crate::pages::ehr::{EhrTab, ehr_href, ehr_id_param, ehr_view};
use crate::paging::Listing;
use crate::routes::Drawn;

/// The filters the address names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

/// The filter toolbar: the composer as the find field, the template and the
/// start-time range beside it.
fn toolbar(ehr_id: &str, filters: &Filters) -> Html {
    let action = ehr_href(ehr_id);
    let tools = Html::join([
        h(format!(
            r#"<div class="fui-list-find"><label for="f-template" class="fui-sr-only">{label}</label><input id="f-template" name="template" type="text" class="fui-input" value="{value}" placeholder="{label}" autocomplete="off"></div>"#,
            label = t(tr(Msg::FilterTemplate)),
            value = t(&filters.template),
        )),
        crate::pages::common::date_range(
            "f-start",
            tr(RecordsMsg::FilterStartTime),
            &filters.from,
            &filters.to,
        ),
        h(format!(
            r#"<button type="submit" class="fui-button fui-button-secondary">{}{}</button>"#,
            ferrohealth_ui::icon::icon(ferrohealth_ui::icon::Glyph::FUNNEL, "16"),
            t(tr(Msg::FilterApply))
        )),
    ]);
    let label = tr(RecordsMsg::FilterComposer);
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
fn chips(ehr_id: &str, filters: &Filters) -> Option<Html> {
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
        &current(),
        ActiveFilters {
            chips,
            clear: ehr_href(ehr_id),
            shown: None,
        },
        None,
    ))
}

/// One composition of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct Panel {
    /// The filters.
    filters: Filters,
    /// The page, or the CDR's refusal of it.
    rows: Result<Vec<Row>, Refused>,
}

/// What the page reads.
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
            .map_err(|error| Refused::new(tr(RecordsMsg::CompositionsRefused), error));
        Panel { filters, rows }
    })
    .await
}

/// The tab's panel.
fn panel(ehr_id: &str, panel: Panel) -> Html {
    let Panel { filters, rows } = panel;
    let rows = match rows {
        Ok(rows) => rows,
        Err(refusal) => {
            return h(format!(
                r#"<div class="fui-panel"><div class="fui-panel-body fui-stack">{}</div>{}</div>"#,
                toolbar(ehr_id, &filters),
                refusal.view()
            ));
        }
    };
    let paging = crate::paging::current(Listing::Compositions);
    let fields = crate::paging::fields(Listing::Compositions);
    let held = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    let window = OpenWindow::of(paging, held);
    let columns = [
        Column::text("composition", tr(RecordsMsg::ColumnComposition)).role(Role::Title),
        Column::text("start", tr(RecordsMsg::ColumnStartTime)).role(Role::Sub),
        Column::text("composer", tr(RecordsMsg::ColumnComposer)).role(Role::Meta),
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
    ];
    let empty = rows.is_empty();
    let body = rows
        .into_iter()
        .map(|row| {
            let version = row.uid.rsplit("::").next().unwrap_or_default().to_owned();
            let href = crate::pages::composition::composition_href(
                ehr_id,
                &crate::uid::container_uid_of(&row.uid),
            );
            let sub = h(format!(
                r#"<span class="fui-row-sub">{}</span>"#,
                t(&row.template)
            ));
            let title = title_cell(Some(&href), t(&row.name), Some(sub));
            let start = cell(&columns[1], ago(&row.start));
            let composer = cell(&columns[2], t(&row.composer));
            let version = cell(&columns[3], t(version));
            h(format!("<tr>{title}{start}{composer}{version}</tr>"))
        })
        .collect();
    let table = if empty && window.page == 1 {
        let sentence = if filters.any() {
            tr(RecordsMsg::CompositionsNoMatch)
        } else {
            tr(RecordsMsg::CompositionsEmpty)
        };
        list_empty(sentence, None)
    } else {
        let caption = tr(RecordsMsg::EhrTabCompositions);
        data_table(DataTable {
            caption: &caption,
            columns: &columns,
            sorting: Sorting::default(),
            body: Body::Rows(body),
        })
    };
    let pager = (window.page > 1 || window.has_next()).then(|| {
        let pager = post_open_pager(
            &current(),
            PagerListing::new(tr(RecordsMsg::CompositionsPages)),
            window,
            PostTo {
                id: "compositions-pager",
                action: crate::paging::PAGING_ROUTE,
                fields: &fields,
            },
        );
        h(format!(r#"<div class="fui-list-foot">{pager}</div>"#))
    });
    h(format!(
        r#"<div class="fui-panel"><div class="fui-panel-body fui-stack">{}{}</div>{table}{}</div>"#,
        toolbar(ehr_id, &filters),
        opt(chips(ehr_id, &filters)),
        opt(pager)
    ))
}

/// An EHR's Compositions tab.
pub async fn page() -> Drawn {
    let ehr_id = ehr_id_param();
    let title = tr_with(RecordsMsg::EhrTitle, &[("id", shortened(&ehr_id).into())]);
    let body = {
        let model = read_page(ehr_id.clone()).await;
        ehr_view(&ehr_id, EhrTab::Compositions, model, |found| {
            panel(&ehr_id, found)
        })
    };
    Drawn::page(title, body)
}
