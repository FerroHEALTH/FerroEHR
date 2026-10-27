// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The results of a query run.
//!
//! As Query and a stored query's Run tab draw them (FerroUI
//! `design/consoles/ferroehr/`, the results pane E7): the window line, the rows as a table or, when the result set has numbers to
//! draw, as a line chart, and the Export menu.
//!
//! A run is a form post, and its results exist only in the page that answers
//! it: the viewer keeps no store between requests. So FerroUI's pane rides
//! the page's own form: Table and Chart are submits of it that run the query
//! again, and each item of Export is a submit of it sent to
//! `/export/aql/<format>` by `formaction` ([`crate::export`]), since a form
//! cannot hold another. No openEHR spec
//! governs the viewer's pages: our own design.

use ferrohealth_ui::table::view::{Body, Column, DataTable, Sorting, cell, data_table};
use ferrohealth_ui::view::chart::{LineChart, Series};
use ferrohealth_ui::view::results::{Export, ResultsPane, Shown, Switch, results_pane};
use ferrohealth_ui::widget::chart::{ChartWords, line_chart};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::bff::ehrs::{ResultPage, cell_text};
#[cfg(not(target_arch = "wasm32"))]
use crate::chart_model::AxisScale;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{Refused, count, quiet, section};

/// The name the view the results show rides as, so the next Run keeps it;
/// the switch's own `shown` wins.
const SHOWN_DEFAULT: &str = "default-shown";

#[cfg(not(target_arch = "wasm32"))]
/// Whether the reader asked for the chart.
fn chart_asked() -> bool {
    use ferrohealth_ui::view::results::SHOWN;
    crate::posted::posted().is_some_and(|posted| {
        posted
            .get(SHOWN)
            .or_else(|| posted.get(SHOWN_DEFAULT))
            .is_some_and(|shown| shown.trim() == Shown::Chart.key())
    })
}

#[cfg(not(target_arch = "wasm32"))]
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

/// One numeric column of a result set as a line of the chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Line {
    /// The column's name.
    name: String,
    /// Its points over the axis.
    points: Vec<(f64, f64)>,
}

/// A result set as a line chart over its first time column, or the row
/// order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Plot {
    /// The lines.
    lines: Vec<Line>,
    /// The first and the last tick with their labels.
    ticks: Vec<(f64, String)>,
    /// Each point's time, on a time axis.
    times: Vec<(f64, String)>,
    /// What the axis is.
    axis: String,
}

#[cfg(not(target_arch = "wasm32"))]
/// The numeric columns of a result set as a line chart over its first time
/// column, or the row order; `None` when it has nothing to draw.
fn plot(page: &ResultPage) -> Option<Plot> {
    let model = crate::chart_model::derive(&page.columns, &page.rows)?;
    let axis = model.axis(0)?;
    let lines = model
        .series
        .iter()
        .enumerate()
        .map(|(index, spec)| Line {
            name: spec.name.clone(),
            points: axis
                .rows
                .iter()
                .filter_map(|row| row.values.get(index).map(|value| (row.x, *value)))
                .collect(),
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
    let times = match axis.scale {
        AxisScale::Time => axis
            .rows
            .iter()
            .filter_map(|row| {
                crate::chart_model::time_point_label(row.x).map(|label| (row.x, label))
            })
            .collect(),
        AxisScale::RowOrder => Vec::new(),
    };
    Some(Plot {
        lines,
        ticks,
        times,
        axis: axis.label.clone(),
    })
}

/// The rows of a result set as a table.
fn table(columns: &[String], rows: &[Vec<String>]) -> AnyView {
    let columns: Vec<Column> = columns
        .iter()
        .map(|name| Column::owned(name.clone(), name.clone()))
        .collect();
    let rows = rows
        .iter()
        .map(|row| {
            let cells = columns
                .iter()
                .zip(row.iter())
                .enumerate()
                .map(|(index, (column, words))| {
                    if index == 0 {
                        view! {
                            <th scope="row" data-role="title">
                                <code>{words.clone()}</code>
                            </th>
                        }
                        .into_any()
                    } else {
                        cell(column, words.clone())
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

/// A plot, drawn.
fn chart(plot: Plot) -> AnyView {
    let names: Vec<String> = plot.lines.iter().map(|line| line.name.clone()).collect();
    let series = plot
        .lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| Series {
            name: line.name,
            points: line.points,
            palette: u8::try_from(index % usize::from(ferrohealth_ui::view::chart::PALETTE))
                .unwrap_or(0),
        })
        .collect();
    let x_labels = plot
        .times
        .iter()
        .map(|(x, label)| (*x, ferrohealth_ui::view::time::reading(label)))
        .collect();
    let chart = LineChart {
        id: "results-chart",
        label: tr(Msg::ResultsChartLabel),
        summary: tr_with(
            Msg::ResultsChartSummary,
            &[
                ("series", names.join(", ").into()),
                ("axis", plot.axis.clone().into()),
            ],
        ),
        series,
        x_ticks: plot.ticks,
        x_labels,
        y_range: None,
    };
    view! {
        {line_chart(&chart, ChartWords::of_page())}
        <p class="fui-small fui-muted">
            {tr_with(Msg::ResultsAxis, &[("axis", plot.axis.into())])}
        </p>
    }
    .into_any()
}

/// What a run answered: the rows as words, the chart they draw, and what the
/// exports run again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ran {
    /// The column names.
    columns: Vec<String>,
    /// Each row's cells, as words.
    rows: Vec<Vec<String>>,
    /// The chart, when the rows have numbers to draw.
    plot: Option<Plot>,
    /// Whether the AQL carries its own row window.
    own_window: bool,
    /// The AQL that ran, for the exports.
    aql: String,
    /// Its parameters as a JSON object text, for the exports.
    parameters: String,
    /// Whether the reader asked for the chart.
    chart: bool,
    /// The route the exports post to.
    export: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl Ran {
    /// What `page` answered to `aql` with `parameters`.
    fn of(page: &ResultPage, aql: &str, parameters: &str) -> Self {
        Self {
            columns: page.columns.clone(),
            rows: page
                .rows
                .iter()
                .map(|row| row.iter().map(cell_words).collect())
                .collect(),
            plot: if page.rows.is_empty() {
                None
            } else {
                plot(page)
            },
            own_window: crate::aql_text::carries_own_window(aql),
            aql: aql.to_owned(),
            parameters: parameters.to_owned(),
            chart: chart_asked(),
            export: crate::export::EXPORT_ROUTE.to_owned(),
        }
    }
}

/// The Results section of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Answer {
    /// Nothing ran, and the line saying why.
    NotRun(String),
    /// What the run answered.
    Ran(Box<Ran>),
    /// The CDR's refusal of the run.
    Refused(Refused),
}

impl Answer {
    #[cfg(not(target_arch = "wasm32"))]
    /// The answer of a run of `aql` with `parameters`.
    #[must_use]
    pub fn of(outcome: Result<ResultPage, ViewerError>, aql: &str, parameters: &str) -> Self {
        match outcome {
            Ok(page) => Self::Ran(Box::new(Ran::of(&page, aql, parameters))),
            Err(error) => Self::Refused(Refused::new(tr(Msg::QueryRunRefused), error)),
        }
    }

    /// The Results section `answer` draws, or the one before a run.
    #[must_use]
    pub fn view(answer: Option<Self>) -> AnyView {
        match answer {
            None => not_run(tr(Msg::ResultsNotRun)),
            Some(Self::NotRun(line)) => not_run(line),
            Some(Self::Ran(ran)) => results(*ran),
            Some(Self::Refused(refusal)) => {
                section(Some(tr(Msg::ResultsTitle)), None, None, refusal.view())
            }
        }
    }
}

/// An item of the Export menu for the run in `format`: a submit of the
/// page's own form, which the Results section always sits in, sent to
/// `/export/aql/<format>` by `formaction`.
fn export(route: &str, format: &str) -> Export {
    Export::Submit {
        action: format!("{route}/{format}"),
        form: None,
    }
}

/// The Results section of a run (E7).
fn results(ran: Ran) -> AnyView {
    let total = ran.rows.len();
    let window = if ran.own_window {
        tr_with(Msg::ResultsWindowOwn, &[("count", count(total))])
    } else {
        tr_with(Msg::ResultsWindow, &[("count", count(total))])
    };
    let asked = if ran.chart {
        Shown::Chart
    } else {
        Shown::Table
    };
    let numeric = ran.plot.is_some();
    let (shown, body) = match ran.plot.filter(|_| ran.chart) {
        Some(plot) => (Shown::Chart, chart(plot)),
        None if total == 0 => (Shown::Table, ().into_any()),
        None => (Shown::Table, table(&ran.columns, &ran.rows)),
    };
    let body = view! {
        <input type="hidden" name=SHOWN_DEFAULT value=asked.key() />
        <input type="hidden" name="q" value=ran.aql />
        <input type="hidden" name="parameters_json" value=ran.parameters />
        {body}
    }
    .into_any();
    results_pane(ResultsPane {
        title: tr(Msg::ResultsTitle),
        count: u64::try_from(total).unwrap_or(u64::MAX),
        numeric,
        shown,
        switch: Switch::Form { form: None },
        csv: export(&ran.export, "csv"),
        json: export(&ran.export, "json"),
        window,
        body,
    })
}

/// The Results section before a run.
fn not_run(line: String) -> AnyView {
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
