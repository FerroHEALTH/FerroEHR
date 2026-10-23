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
//! the page's own form, applied by the next run, and each export is a submit
//! of that same form sent to `/export/aql` ([`crate::export`]), since a form
//! cannot hold another. No openEHR spec governs the viewer's pages: our own
//! design.

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

/// The words of one result cell: a data value read as a person reads it (a
/// quantity with its unit, a text, a code or a time by its value), anything
/// else as the CDR sent it.
#[expect(
    clippy::disallowed_types,
    reason = "a RESULT_SET cell is the CDR's untyped JSON"
)]
fn cell_words(value: &serde_json::Value) -> String {
    use serde_json::Value;
    let Some(kind) = value.get("_type").and_then(Value::as_str) else {
        return cell_text(value);
    };
    let text = |key: &str| value.get(key).map(cell_text);
    match kind {
        "DV_QUANTITY" => match (text("magnitude"), text("units")) {
            (Some(magnitude), Some(units)) => format!("{magnitude} {units}"),
            (Some(magnitude), None) => magnitude,
            _ => cell_text(value),
        },
        "DV_COUNT" | "DV_ORDINAL" | "DV_PROPORTION" => text("magnitude")
            .or_else(|| text("value"))
            .or_else(|| text("numerator"))
            .unwrap_or_else(|| cell_text(value)),
        _ if kind.starts_with("DV_") => text("value").unwrap_or_else(|| cell_text(value)),
        _ => cell_text(value),
    }
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
                                <code>{cell_words(value)}</code>
                            </th>
                        }
                        .into_any()
                    } else {
                        cell(column, cell_words(value))
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

// TODO(#3942): E7 draws this section with FerroHEALTH/FerroUI#355.
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
        // TODO(#3942): each point carries its label with FerroHEALTH/FerroUI#358.
        x_ticks: ticks,
        y_range: None,
    };
    Some(
        view! {
            <div class="fui-panel-body">
                {line_chart(&chart, ChartWords::of_page())}
                <p class="fui-small fui-muted">
                    {tr_with(Msg::ResultsAxis, &[("axis", axis.label.clone().into())])}
                </p>
            </div>
        }
        .into_any(),
    )
}

/// An export of the run in `format`: a submit of the page's own form, which
/// the Results section always sits in, sent to `/export/aql` instead.
fn export(format: &'static str, words: String) -> AnyView {
    view! {
        <button
            type="submit"
            name="format"
            value=format
            formaction="/export/aql"
            formmethod="post"
            formnovalidate
            class="fui-button fui-button-secondary fui-button-small"
        >
            {ferrohealth_ui::icon::icon(Glyph::DOWNLOAD, "14")}
            {words}
        </button>
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
                <input type="hidden" name="q" value=answered.aql.to_owned() />
                <input type="hidden" name="parameters_json" value=answered.parameters.to_owned() />
                {export("csv", tr(Msg::ResultsCsv))}
                {export("json", tr(Msg::ResultsJson))}
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

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "a RESULT_SET cell is the CDR's untyped JSON"
)]
mod tests {
    use super::cell_words;

    #[test]
    fn a_data_value_reads_as_a_person_reads_it() {
        let quantity =
            serde_json::json!({"_type": "DV_QUANTITY", "magnitude": 38.2, "units": "kg"});
        assert_eq!(cell_words(&quantity), "38.2 kg");
        let time = serde_json::json!({"_type": "DV_DATE_TIME", "value": "2026-10-20T08:00:00Z"});
        assert_eq!(cell_words(&time), "2026-10-20T08:00:00Z");
        assert_eq!(cell_words(&serde_json::json!("text")), "text");
        assert_eq!(cell_words(&serde_json::json!({"a": 1})), r#"{"a":1}"#);
    }
}
