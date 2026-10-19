// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The results pane's chart view: a grouped multi-series line chart over one
//! page of an AQL `RESULT_SET`, shared by the query builder and the raw AQL
//! editor.
//!
//! FerroUI's line chart (`ferrohealth_ui::widget::chart`), SVG drawn in Rust:
//! one line per mostly-numeric result column, a legend that shows and hides
//! series, and an X-axis picker offering every ISO 8601 column as a real time
//! scale with the row order as the fallback. Every one of those decisions is
//! derived by [`crate::chart_model`], a pure function of the fetched rows, so
//! the server pass and the browser hydration build the same structure; each
//! line pins its palette colour by its own index, so hiding one recolours
//! none.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use ferrohealth_ui::view::chart::{LineChart, Series};
use ferrohealth_ui::widget::chart::line_chart;
use leptos::prelude::*;
use serde_json::Value;

use crate::chart_model::{AxisScale, AxisSpec, ChartModel, time_tick_label};
use crate::components::empty_state::EmptyState;
use crate::components::field::SELECT;
use crate::locale::{Msg, tr, tr_with};

/// How many ticks the X axis carries.
const X_TICKS: u32 = 5;

/// The chart half of a results pane: the multi-series chart for a chartable
/// page, or a designed explanation of why this result set has no chart.
///
/// Takes the result set's columns and rows (never a screen type), so both
/// results panes and the unit tests share one entry point.
#[expect(clippy::must_use_candidate, reason = "consumed by the caller's view!")]
pub fn results_chart(columns: &[String], rows: &[Vec<Value>]) -> AnyView {
    match crate::chart_model::derive(columns, rows) {
        Some(model) => chart_panel(model),
        None => chart_absent(rows.len()),
    }
}

/// The ticks of the X axis over `axis`: evenly spaced across its rows, each
/// labelled as an instant on a time axis and as a row number otherwise.
fn x_ticks(axis: &AxisSpec) -> Vec<(f64, String)> {
    let (low, high) = axis
        .rows
        .iter()
        .map(|row| row.x)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), x| {
            (low.min(x), high.max(x))
        });
    if !low.is_finite() || !high.is_finite() {
        return Vec::new();
    }
    let span = axis.span();
    (0..X_TICKS)
        .map(|step| {
            let at = low + (high - low) * f64::from(step) / f64::from(X_TICKS - 1);
            let label = match axis.scale {
                AxisScale::Time => time_tick_label(at, span),
                AxisScale::RowOrder => format!("{}", at.round()),
            };
            (at, label)
        })
        .collect()
}

/// The FerroUI chart of `model` over the axis `axis`.
fn line_chart_of(model: &ChartModel, axis: usize) -> LineChart {
    let spec = model.axis(axis);
    let series = model
        .series
        .iter()
        .enumerate()
        .map(|(index, series)| Series {
            name: series.name.clone(),
            points: spec
                .map(|spec| {
                    spec.rows
                        .iter()
                        .map(|row| (row.x, row.values.get(index).copied().unwrap_or(f64::NAN)))
                        .collect()
                })
                .unwrap_or_default(),
            palette: u8::try_from(index % 256).unwrap_or(0),
        })
        .collect();
    let axis_label = spec.map(|spec| spec.label.clone()).unwrap_or_default();
    LineChart {
        id: "results-chart",
        label: tr(Msg::ResultSetChart),
        summary: tr_with(
            Msg::SeriesOver,
            &[
                ("model", (model.series.len()).to_string().into()),
                ("axis_label", axis_label.clone().into()),
            ],
        ),
        series,
        x_ticks: spec.map(x_ticks).unwrap_or_default(),
        y_range: None,
    }
}

/// The assembled panel: the axis picker above, the chart and its legend below.
fn chart_panel(model: ChartModel) -> AnyView {
    let axis_labels: Vec<String> = model.axes.iter().map(|axis| axis.label.clone()).collect();
    // The series count doubles as the E2E hook's value.
    let series_hook = model.series.len().to_string();
    let axis = RwSignal::new(0_usize);
    let shown = RwSignal::new(vec![true; model.series.len()]);
    let model = StoredValue::new(model);
    let picker = if axis_labels.len() > 1 {
        axis_picker(&axis_labels, axis)
    } else {
        ().into_any()
    };
    let chart =
        move || model.with_value(|model| line_chart(line_chart_of(model, axis.get()), shown));
    view! {
        <div data-results-chart=series_hook>
            <div class="mb-2 flex flex-wrap items-center justify-between gap-2">{picker}</div>
            {chart}
        </div>
    }
    .into_any()
}

/// The X-axis picker: every temporal column the result set offers, plus the row
/// order. Rendered only when there is a real choice.
fn axis_picker(labels: &[String], axis: RwSignal<usize>) -> AnyView {
    let options = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            // Two bindings: the view! macro moves child text before evaluating
            // attribute clones, so one String cannot serve both positions.
            let value = index.to_string();
            let text = label.clone();
            view! { <option value=value>{text}</option> }
        })
        .collect::<Vec<_>>();
    view! {
        <label class="flex items-center gap-2">
            <span class="fui-label">{tr(Msg::XAxis)}</span>
            <select
                class=format!("{SELECT} w-auto")
                data-chart-axis="picker"
                prop:value=move || axis.get().to_string()
                on:change:target=move |ev| {
                    if let Ok(index) = ev.target().value().parse::<usize>() {
                        axis.set(index);
                    }
                }
            >
                {options}
            </select>
        </label>
    }
    .into_any()
}

/// The designed state for a result set with no chart: why there is none, and
/// what would produce one.
fn chart_absent(row_count: usize) -> AnyView {
    if row_count <= 1 {
        return view! {
            <div data-chart-empty=tr(Msg::SingleRow)>
                <EmptyState
                    icon=ferrohealth_ui::icon::Glyph::CHART_LINE
                    message=tr(Msg::OneRowNotTrend)
                    hint=tr(Msg::ChartNeedsLeastTwoRowsWiden)
                />
            </div>
        }
        .into_any();
    }
    view! {
        <div data-chart-empty=tr(Msg::NoNumericColumn)>
            <EmptyState
                icon=ferrohealth_ui::icon::Glyph::CHART_LINE
                message=tr(Msg::NothingNumericToChart)
                hint=tr(Msg::ChartingNeedsColumnHoldingLeastTwo)
            />
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::line_chart_of;

    /// Each series keeps its palette index whatever the axis, so hiding one
    /// recolours none, and every series carries one point per row.
    #[test]
    fn series_keep_their_palette_index_and_their_points() {
        let columns = vec!["observed".to_owned(), "a".to_owned(), "b".to_owned()];
        let rows = vec![
            vec![json!("2026-08-20T10:00:00Z"), json!(1), json!(2)],
            vec![json!("2026-08-21T10:00:00Z"), json!(3), json!(4)],
        ];
        let model = crate::chart_model::derive(&columns, &rows).expect("a chartable page");
        for axis in 0..model.axes.len() {
            let chart = line_chart_of(&model, axis);
            assert_eq!(chart.series.len(), 2);
            for (index, series) in chart.series.iter().enumerate() {
                assert_eq!(usize::from(series.palette), index);
                assert_eq!(series.points.len(), 2);
            }
            assert_eq!(chart.x_ticks.len(), 5);
        }
    }
}
