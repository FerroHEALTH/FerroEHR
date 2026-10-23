// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The results of a query run.
//!
//! As Query and a stored query's Run tab draw them (FerroUI
//! `design/consoles/ferroehr/`, the results pane E7): the window line, the rows as a table or, when the result set has numbers to
//! draw, as a line chart, and the exports.
//!
//! A run is a form post, and its results exist only in the page that answers
//! it: the viewer keeps no store between requests. So Show as is a choice of
//! the page's own form, applied by the next run, and each export is a form
//! that posts the query again to `/export/aql` ([`crate::export`]). No
//! openEHR spec governs the viewer's pages: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Sorting, cell, data_table};
use ferrohealth_ui::view::chart::{LineChart, Series};
use ferrohealth_ui::view::segmented::choice_group;
use ferrohealth_ui::widget::chart::{ChartWords, line_chart};
use leptos::prelude::*;

use crate::bff::ehrs::{ResultPage, cell_text};
use crate::chart_model::AxisScale;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{count, quiet, section};

/// The name the Show as choice posts as.
pub const SHOWN: &str = "shown";

/// Whether the reader asked for the chart.
#[must_use]
pub fn chart_asked() -> bool {
    crate::posted::posted().is_some_and(|posted| posted.text(SHOWN) == "chart")
}

/// The Show as choice, a field of the page's form `form`.
#[must_use]
pub fn shown_choice(chart: bool) -> AnyView {
    choice_group(
        SHOWN,
        tr(Msg::ResultsShowAs),
        &[
            ("table", tr(Msg::ResultsTable)),
            ("chart", tr(Msg::ResultsChart)),
        ],
        if chart { "chart" } else { "table" },
    )
}

/// The rows of a result set as a table.
fn table(page: &ResultPage) -> AnyView {
    let columns: Vec<Column> = page
        .columns
        .iter()
        .map(|name| Column::owned(name.clone(), name.clone()))
        .collect();
    let rows = page
        .rows
        .iter()
        .map(|row| {
            let cells = columns
                .iter()
                .zip(row.iter())
                .enumerate()
                .map(|(index, (column, value))| {
                    if index == 0 {
                        view! {
                            <th scope="row" data-role="title">
                                <code>{cell_text(value)}</code>
                            </th>
                        }
                        .into_any()
                    } else {
                        cell(column, cell_text(value))
                    }
                })
                .collect_view();
            view! { <tr>{cells}</tr> }.into_any()
        })
        .collect_view()
        .into_any();
    let caption = tr(Msg::ResultsTitle);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(rows),
    })
    .into_any()
}

/// The numeric columns of a result set as a line chart over its first time
/// column, or the row order; `None` when it has nothing to draw.
fn chart(page: &ResultPage) -> Option<AnyView> {
    let model = crate::chart_model::derive(&page.columns, &page.rows)?;
    let axis = model.axis(0)?;
    let series = model
        .series
        .iter()
        .enumerate()
        .map(|(index, spec)| Series {
            name: spec.name.clone(),
            points: axis
                .rows
                .iter()
                .filter_map(|row| row.values.get(index).map(|value| (row.x, *value)))
                .collect(),
            palette: u8::try_from(index % usize::from(ferrohealth_ui::view::chart::PALETTE))
                .unwrap_or(0),
        })
        .collect();
    let span = axis.span();
    let ticks = match (axis.rows.first(), axis.rows.last()) {
        (Some(first), Some(last)) => {
            let label = |x: f64| match axis.scale {
                AxisScale::Time => crate::chart_model::time_tick_label(x, span),
                AxisScale::RowOrder => format!("{x:.0}"),
            };
            vec![(first.x, label(first.x)), (last.x, label(last.x))]
        }
        _ => Vec::new(),
    };
    let names: Vec<String> = model.series.iter().map(|spec| spec.name.clone()).collect();
    let chart = LineChart {
        id: "results-chart",
        label: tr(Msg::ResultsChartLabel),
        summary: tr_with(
            Msg::ResultsChartSummary,
            &[
                ("series", names.join(", ").into()),
                ("axis", axis.label.clone().into()),
            ],
        ),
        series,
        x_ticks: ticks,
        y_range: None,
    };
    Some(view! {
        <div class="fui-panel-body">
            {line_chart(&chart, ChartWords::of_page())}
            <p class="fui-small fui-muted">{tr_with(Msg::ResultsAxis, &[("axis", axis.label.clone().into())])}</p>
        </div>
    }
    .into_any())
}

/// An export of the query `aql` with `parameters`: a form that posts it
/// again to `/export/aql` for a download in `format`.
fn export(aql: &str, parameters: &str, format: &'static str, words: String) -> AnyView {
    view! {
        <form method="post" action="/export/aql" class="fui-inline-form">
            <input type="hidden" name="q" value=aql.to_owned() />
            <input type="hidden" name="parameters_json" value=parameters.to_owned() />
            <input type="hidden" name="format" value=format />
            <button type="submit" class="fui-button fui-button-secondary fui-button-small">
                {ferrohealth_ui::icon::icon(Glyph::DOWNLOAD, "14")}
                {words}
            </button>
        </form>
    }
    .into_any()
}

/// What a run answered, as the Results section.
#[derive(Debug)]
pub struct Answered<'a> {
    /// The page of rows.
    pub page: &'a ResultPage,
    /// The AQL that ran, for the exports.
    pub aql: &'a str,
    /// Its parameters as a JSON object text, for the exports.
    pub parameters: &'a str,
    /// Whether the reader asked for the chart.
    pub chart: bool,
}

/// The Results section of a run.
#[must_use]
pub fn results(answered: &Answered<'_>) -> AnyView {
    let page = answered.page;
    let total = page.rows.len();
    let window = if crate::aql_text::carries_own_window(answered.aql) {
        tr_with(Msg::ResultsWindowOwn, &[("count", count(total))])
    } else {
        tr_with(Msg::ResultsWindow, &[("count", count(total))])
    };
    let body = if total == 0 {
        quiet(tr(Msg::ResultsNone))
    } else if answered.chart {
        chart(page).unwrap_or_else(|| {
            view! {
                <p class="fui-small fui-muted">{tr(Msg::ResultsNoChart)}</p>
                {table(page)}
            }
            .into_any()
        })
    } else {
        table(page)
    };
    let actions = (total > 0).then(|| {
        view! {
            <div class="fui-cluster">
                {export(answered.aql, answered.parameters, "csv", tr(Msg::ResultsCsv))}
                {export(answered.aql, answered.parameters, "json", tr(Msg::ResultsJson))}
            </div>
        }
        .into_any()
    });
    section(
        Some(tr(Msg::ResultsTitle)),
        u64::try_from(total).ok(),
        actions,
        view! {
            <p class="fui-results-window">{window}</p>
            {body}
        },
    )
}

/// The Results section before a run.
#[must_use]
pub fn not_run(line: String) -> AnyView {
    section(Some(tr(Msg::ResultsTitle)), None, None, quiet(line))
}
