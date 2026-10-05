// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared activity chart: ONE pure-Rust SVG line chart every
//! events-per-day timeline in the viewer draws with — the dashboard's
//! commit-activity trend and an EHR's contribution timeline.
//!
//! Charts are `leptos-chartistry` (pure Rust + SVG): the no-JavaScript mandate
//! bans every charting-library binding. The chart draws client-side once the
//! container is measured and renders a placeholder on the server pass, so the
//! surrounding view structure is identical on both passes; its points
//! come from [`crate::activity::bucket_by_day`], a pure function of the wire
//! data.

use leptos::prelude::*;
use leptos_chartistry::{
    AspectRatio, AxisMarker, Chart, IntoInner, Line, Series, TickLabels, YGridLine,
};

use crate::activity::ActivityPoint;
use crate::components::empty_state::EmptyState;

/// Render events-per-day `points` as a minimal line chart, or the shared
/// [`EmptyState`] when there is nothing to plot.
///
/// The X axis is the day index (the points are already ascending by day);
/// `series_name` labels the line and is also the chart's `data-activity-chart`
/// hook, so a journey can name the chart it is asserting on.
#[must_use]
pub fn activity_chart(
    points: &[ActivityPoint],
    series_name: &'static str,
    empty_message: &'static str,
    empty_hint: &'static str,
) -> AnyView {
    if points.is_empty() {
        return view! { <EmptyState icon=icondata_lu::LuChartLine message=empty_message hint=empty_hint /> }
        .into_any();
    }
    let data: Vec<(f64, f64)> = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let x = f64::from(u32::try_from(index).unwrap_or(u32::MAX));
            (x, f64::from(point.count))
        })
        .collect();
    let data = RwSignal::new(data);
    view! {
        <div class="overflow-x-auto" data-activity-chart=series_name>
            <Chart
                aspect_ratio=AspectRatio::from_outer_ratio(640.0, 240.0)
                left=TickLabels::aligned_floats()
                bottom=TickLabels::aligned_floats()
                inner=[
                    AxisMarker::left_edge().into_inner(),
                    AxisMarker::bottom_edge().into_inner(),
                    YGridLine::default().into_inner(),
                ]
                series=Series::new(|(x, _): &(f64, f64)| *x)
                    .line(Line::new(|(_, y): &(f64, f64)| *y).with_name(series_name))
                data=data
            />
        </div>
    }
    .into_any()
}
