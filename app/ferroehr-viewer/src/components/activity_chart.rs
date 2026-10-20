// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared activity chart: ONE chart every events-per-day timeline in the
//! viewer draws with — the dashboard's commit-activity trend and an EHR's
//! contribution timeline.
//!
//! It is FerroUI's activity chart (`ferrohealth_ui::view::chart`), SVG drawn
//! on the server, so the server pass and hydration draw the same; its points
//! come from [`crate::activity::bucket_by_day`], a pure function of the wire
//! data.

use ferrohealth_ui::view::chart::ActivityChart;
use leptos::prelude::*;

use crate::activity::ActivityPoint;
use crate::components::empty_state::EmptyState;
use crate::locale::{Msg, tr_with};

/// The sentence that summarises `points`: how many `noun` over how many days,
/// and the busiest day.
fn summary(points: &[ActivityPoint], noun: &str) -> String {
    let total: u64 = points.iter().map(|point| u64::from(point.count)).sum();
    let busiest = points
        .iter()
        .max_by_key(|point| point.count)
        .map(|point| point.day.as_str())
        .unwrap_or_default();
    tr_with(
        Msg::OverDaysMostOn,
        &[
            ("total", (total).to_string().into()),
            ("noun", (noun).to_string().into()),
            ("points", (points.len()).to_string().into()),
            ("busiest", (busiest).to_string().into()),
        ],
    )
}

/// Render events-per-day `points` as FerroUI's activity chart, or the shared
/// [`EmptyState`] when there is nothing to plot.
///
/// The X axis is the day index (the points are already ascending by day);
/// `series_name` labels the line and is also the chart's `data-activity-chart`
/// hook, so a journey can name the chart it is asserting on.
#[must_use]
pub fn activity_chart(
    points: &[ActivityPoint],
    series_name: &'static str,
    empty_message: String,
    empty_hint: String,
) -> AnyView {
    if points.is_empty() {
        return view! {
            <EmptyState
                icon=ferrohealth_ui::icon::Glyph::CHART_LINE
                message=empty_message
                hint=empty_hint
            />
        }
        .into_any();
    }
    let ends = (
        points
            .first()
            .map(|point| point.day.clone())
            .unwrap_or_default(),
        points
            .last()
            .map(|point| point.day.clone())
            .unwrap_or_default(),
    );
    let chart = ActivityChart {
        name: series_name,
        label: tr_with(
            Msg::PerDay,
            &[("series_name", (series_name).to_string().into())],
        ),
        summary: summary(points, &series_name.to_lowercase()),
        points: (0_u32..)
            .zip(points)
            .map(|(day, point)| (day, point.count))
            .collect(),
        ends,
    };
    ferrohealth_ui::view::chart::activity_chart(&chart).into_any()
}

#[cfg(test)]
mod tests {
    use super::summary;
    use crate::activity::ActivityPoint;

    #[test]
    fn the_summary_counts_the_events_and_names_the_busiest_day() {
        let points = [
            ActivityPoint {
                day: "2026-08-20".to_owned(),
                count: 3,
            },
            ActivityPoint {
                day: "2026-08-21".to_owned(),
                count: 7,
            },
        ];
        assert_eq!(
            summary(&points, "commits"),
            "10 commits over 2 days, most on 2026-08-21."
        );
    }
}
