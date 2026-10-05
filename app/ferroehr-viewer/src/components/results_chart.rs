// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The results pane's chart view: a grouped multi-series line chart over one
//! page of an AQL `RESULT_SET`, shared by the query builder and the raw AQL
//! editor.
//!
//! Pure Rust + SVG through `leptos-chartistry` (the no-JS mandate): one line
//! per mostly-numeric result column, an interactive legend that shows and
//! hides series, and an X-axis picker offering every ISO 8601 column as a real
//! time scale with the row order as the fallback. Every one of those decisions
//! is derived by [`crate::chart_model`], a pure function of the fetched rows,
//! so the server pass and the browser hydration build the same structure.
//!
//! Hiding a series feeds `f64::NAN` for its Y values instead of rebuilding the
//! chart: chartistry treats a `NaN` as missing data (a gap in the line) and
//! recomputes the Y range from what is left, so a toggle re-renders only the
//! series it touched — no remount, no re-measure, no colour reshuffle (each
//! line pins its palette colour by its own index).

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::prelude::*;
use leptos_chartistry::{
    AspectRatio, AxisMarker, Chart, Colour, Interpolation, IntoInner, Line, SERIES_COLOUR_SCHEME,
    Series, TickLabels, YGridLine,
};
use serde_json::Value;

use crate::chart_model::{AxisScale, ChartModel, ChartRow, SeriesSpec, time_tick_label};
use crate::components::empty_state::EmptyState;
use crate::components::field::SELECT;

/// A legend chip for a series that is currently drawn.
const CHIP_ON: &str = "inline-flex items-center gap-1.5 rounded-control border border-edge-strong bg-raised px-2 py-1 text-xs font-medium text-ink hover:bg-sunken focus:outline-none focus:ring-2 focus:ring-accent disabled:cursor-default disabled:opacity-70";

/// A legend chip for a series the user has hidden.
const CHIP_OFF: &str = "inline-flex items-center gap-1.5 rounded-control border border-dashed border-edge px-2 py-1 text-xs font-medium text-ink-faint line-through hover:bg-sunken focus:outline-none focus:ring-2 focus:ring-accent";

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

/// The assembled panel: axis picker + legend above, chart below.
fn chart_panel(model: ChartModel) -> AnyView {
    let specs = model.series.clone();
    let axis_labels: Vec<String> = model.axes.iter().map(|axis| axis.label.clone()).collect();
    // The series count doubles as the E2E hook's value.
    let series_hook = specs.len().to_string();
    let axis = RwSignal::new(0_usize);
    let visible = RwSignal::new(vec![true; specs.len()]);
    let model = StoredValue::new(model);

    // A single series is the degenerate case: one line, no legend noise.
    let legend = if specs.len() > 1 {
        legend_row(&specs, visible)
    } else {
        ().into_any()
    };
    let picker = if axis_labels.len() > 1 {
        axis_picker(&axis_labels, axis)
    } else {
        ().into_any()
    };
    let chart = chart_svg(&specs, model, axis, visible);
    view! {
        <div data-results-chart=series_hook>
            <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
                {picker} {legend}
            </div>
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
        <label class="flex items-center gap-2 text-xs text-ink-muted">
            "X axis"
            <select
                class=SELECT
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

/// The interactive legend: one toggle chip per series, coloured like its line.
fn legend_row(specs: &[SeriesSpec], visible: RwSignal<Vec<bool>>) -> AnyView {
    let shown = Memo::new(move |_| visible.with(|flags| flags.iter().filter(|on| **on).count()));
    // A static list built once (the series of one fetched page never change),
    // so there is no <For> key to get wrong.
    let chips = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| legend_chip(index, &spec.name, visible, shown))
        .collect::<Vec<_>>();
    view! {
        <div class="flex flex-wrap items-center gap-1.5" data-chart-legend="series">
            {chips}
        </div>
    }
    .into_any()
}

/// One legend chip. Clicking it hides or shows its series; the last visible
/// series cannot be hidden, so the chart never empties itself.
fn legend_chip(
    index: usize,
    name: &str,
    visible: RwSignal<Vec<bool>>,
    shown: Memo<usize>,
) -> AnyView {
    let is_on = Memo::new(move |_| visible.with(|flags| flags.get(index).copied().unwrap_or(true)));
    let swatch = format!("background-color: {}", series_colour(index));
    let hook = name.to_owned();
    let label = name.to_owned();
    view! {
        <button
            type="button"
            class=move || if is_on.get() { CHIP_ON } else { CHIP_OFF }
            data-chart-series=hook
            data-visible=move || if is_on.get() { "true" } else { "false" }
            aria-pressed=move || if is_on.get() { "true" } else { "false" }
            disabled=move || is_on.get() && shown.get() <= 1
            on:click=move |_| {
                visible
                    .update(|flags| {
                        if let Some(flag) = flags.get_mut(index) {
                            *flag = !*flag;
                        }
                    });
            }
        >
            <span class="h-2 w-2 shrink-0 rounded-full" style=swatch></span>
            {label}
        </button>
    }
    .into_any()
}

/// The chart itself: one line per series over the selected axis.
///
/// Built once. The axis choice drives the data signal and the bottom tick
/// formatter (both read `axis`), and the legend drives each line's Y getter, so
/// every interaction is a fine-grained update rather than a new `<Chart>`.
fn chart_svg(
    specs: &[SeriesSpec],
    model: StoredValue<ChartModel>,
    axis: RwSignal<usize>,
    visible: RwSignal<Vec<bool>>,
) -> AnyView {
    let data = Signal::derive(move || {
        model.with_value(|chart| {
            chart
                .axis(axis.get())
                .map(|spec| spec.rows.clone())
                .unwrap_or_default()
        })
    });
    // A temporal axis labels its ticks as instants (the tick position IS epoch
    // seconds); the row-order axis keeps chartistry's own float labels.
    let bottom = TickLabels::aligned_floats().with_format(move |tick, state| {
        model.with_value(|chart| match chart.axis(axis.get()) {
            Some(spec) if spec.scale == AxisScale::Time => time_tick_label(*tick, spec.span()),
            _ => state.format(tick),
        })
    });
    // Annotated: with no lines added yet the Y type would be ambiguous.
    let mut series: Series<ChartRow, f64, f64> = Series::new(|row: &ChartRow| row.x);
    for (index, spec) in specs.iter().enumerate() {
        // NOTE: explicit Linear (leptos-chartistry 0.2.3 src/series/line/
        // interpolation.rs — the default Monotone's tangent divides by
        // `x_next - x`, so duplicate-x rows emit `S x,NaN`, invalid SVG).
        series = series.line(
            Line::new(move |row: &ChartRow| series_y(row, index, visible))
                .with_name(spec.name.clone())
                .with_colour(series_colour(index))
                .with_interpolation(Interpolation::Linear),
        );
    }
    view! {
        <div class="overflow-x-auto">
            <Chart
                aspect_ratio=AspectRatio::from_outer_ratio(640.0, 240.0)
                left=TickLabels::aligned_floats()
                bottom=bottom
                inner=[
                    AxisMarker::left_edge().into_inner(),
                    AxisMarker::bottom_edge().into_inner(),
                    YGridLine::default().into_inner(),
                ]
                series=series
                data=data
            />
        </div>
    }
    .into_any()
}

/// The Y value one line reads: the series' cell, or `f64::NAN` when the cell
/// held no number — or when the legend has hidden the series (chartistry draws
/// `NaN` as missing data, so the line disappears and the Y range rescales).
fn series_y(row: &ChartRow, index: usize, visible: RwSignal<Vec<bool>>) -> f64 {
    if visible.with(|flags| flags.get(index).copied().unwrap_or(true)) {
        row.values.get(index).copied().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// The palette colour of series `index`, pinned by index (never by draw order)
/// so hiding a series never recolours the others. `SERIES_COLOUR_SCHEME` is
/// chartistry's own line palette, so the legend swatch matches the line.
fn series_colour(index: usize) -> Colour {
    let palette = SERIES_COLOUR_SCHEME;
    palette
        .get(index % palette.len())
        .copied()
        .unwrap_or(Colour::from_rgb(0x12, 0xA5, 0xED))
}

/// The designed state for a result set with no chart: why there is none, and
/// what would produce one.
fn chart_absent(row_count: usize) -> AnyView {
    if row_count <= 1 {
        return view! {
            <div data-chart-empty="single-row">
                <EmptyState
                    icon=icondata_lu::LuChartLine
                    message="One row is not a trend"
                    hint="A chart needs at least two rows — widen the query's window, or relax a condition, and run it again."
                />
            </div>
        }
        .into_any();
    }
    view! {
        <div data-chart-empty="no-numeric-column">
            <EmptyState
                icon=icondata_lu::LuChartLine
                message="Nothing numeric to chart"
                hint="Charting needs a column holding at least two numbers — project a quantity magnitude, a count, or an ordinal value. An ISO date/time column then becomes the X axis."
            />
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::series_colour;

    /// Series colours are pinned per index and wrap around the palette, so a
    /// hidden series never shifts another series' colour.
    #[test]
    fn series_colours_are_pinned_per_index() {
        assert_ne!(series_colour(0), series_colour(1));
        assert_ne!(series_colour(1), series_colour(2));
        // Ten palette entries, then it wraps — never an out-of-range panic.
        assert_eq!(series_colour(0), series_colour(10));
        assert_eq!(series_colour(3), series_colour(13));
    }
}
