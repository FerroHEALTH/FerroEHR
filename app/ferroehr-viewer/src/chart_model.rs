// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Grouped multi-series chart derivation for an AQL result set — the
//! component-free half of the results pane's chart view (the view itself is
//! [`crate::components::results_chart`]).
//!
//! One series per mostly-numeric result column (the column header is the
//! series name), and one X axis per column whose cells read as ISO 8601
//! date/times, plus the row order as the always-available fallback. No openEHR
//! spec governs the viewer chart — our own design / product extension; the
//! cell texts it reads ARE spec-bound, because a projected `DV_DATE_TIME` /
//! `DV_DATE` value carries an ISO 8601 string
//! (`docs/specs/openehr/RM/docs/data_types/master06-quantity_package.adoc`
//! §Date/Time types), which is what [`iso_epoch_seconds`] reads to place a
//! point on a real time axis.
//!
//! Everything here is a pure, deterministic function of the fetched rows — no
//! clock, no locale, no randomness, no I/O — which is what keeps the chart
//! hydration-safe: the server pass and the browser hydration derive
//! byte-identical series, axes, and tick labels.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use serde_json::Value;

/// How many usable points a column needs before it is worth drawing: one point
/// is a dot, not a series (and a one-row result set is not a chart).
const MIN_POINTS: usize = 2;

/// Seconds in a day — the granularity ladder [`time_tick_label`] switches on.
const DAY: f64 = 86_400.0;

/// One result column promoted to a chart series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesSpec {
    /// The result-set column header (its AQL alias, or the projected path),
    /// used as the series name in the legend.
    pub name: String,
    /// The zero-based result-set column this series reads. Unique per series,
    /// so it is also the legend's stable, data-derived key. Fixed-size on
    /// purpose (WASM is 32-bit).
    pub column: u16,
}

/// One plotted row: the X position every series shares, plus one Y value per
/// series in [`ChartModel::series`] order.
///
/// `f64::NAN` marks a cell that was absent, null, or not a number —
/// `leptos-chartistry` draws those as a gap in the line rather than a point at
/// zero, which is also how a legend-hidden series is drawn (see
/// [`crate::components::results_chart`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ChartRow {
    /// The X position: a row ordinal, or seconds since the Unix epoch.
    pub x: f64,
    /// One Y value per derived series, in series order.
    pub values: Vec<f64>,
}

/// What an axis's X positions mean — which decides its tick labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisScale {
    /// X is the row's ordinal in the fetched page (the fallback axis).
    RowOrder,
    /// X is seconds since the Unix epoch: a real time scale.
    Time,
}

/// One X axis the chart offers.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisSpec {
    /// The axis name shown in the picker ("Row order", or the column header).
    pub label: String,
    /// What the X positions mean.
    pub scale: AxisScale,
    /// The plotted rows, ascending by `x` (chartistry requires sorted data).
    pub rows: Vec<ChartRow>,
}

impl AxisSpec {
    /// The width of the axis in X units (`0.0` for fewer than two rows) — the
    /// span a time axis picks its label granularity from.
    #[must_use]
    pub fn span(&self) -> f64 {
        match (self.rows.first(), self.rows.last()) {
            (Some(first), Some(last)) => last.x - first.x,
            _ => 0.0,
        }
    }
}

/// A chartable result set: the derived series and the axes they can be drawn
/// over.
///
/// [`Self::axes`] is never empty, and its first entry is the default — a
/// temporal axis when the result set offers one, the row order otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartModel {
    /// One series per mostly-numeric column, in result-set column order.
    pub series: Vec<SeriesSpec>,
    /// The offered X axes: every temporal column (leftmost first), then the
    /// row order.
    pub axes: Vec<AxisSpec>,
}

impl ChartModel {
    /// The axis at `index`, falling back to the default (first) axis when the
    /// index is out of range.
    #[must_use]
    pub fn axis(&self, index: usize) -> Option<&AxisSpec> {
        self.axes.get(index).or_else(|| self.axes.first())
    }
}

/// Derive the chart model for one page of an AQL `RESULT_SET`, or `None` when
/// nothing in it is worth drawing (no column carries two or more numbers).
///
/// A column becomes a series when at least half of its non-null cells are
/// finite numbers and at least `MIN_POINTS` of them are — the rule the
/// single-column predecessor used, now applied to EVERY column instead of
/// stopping at the first match.
#[must_use]
pub fn derive(columns: &[String], rows: &[Vec<Value>]) -> Option<ChartModel> {
    let series = numeric_columns(columns, rows);
    if series.is_empty() {
        return None;
    }
    let mut axes: Vec<AxisSpec> = temporal_columns(columns, rows, &series)
        .into_iter()
        .filter_map(|(column, label)| time_axis(&series, label, column, rows))
        .collect();
    axes.push(row_order_axis(&series, rows));
    Some(ChartModel { series, axes })
}

/// The numeric value of one cell: a JSON number, or a string that parses as a
/// finite number (result sets carry both — a `DV_QUANTITY` magnitude arrives as
/// a number, some projected leaves as text). Never `NaN`/infinity: those are
/// the model's "missing" marker.
fn numeric_cell(cell: &Value) -> Option<f64> {
    cell.as_f64()
        .or_else(|| cell.as_str().and_then(|text| text.parse::<f64>().ok()))
        .filter(|value| value.is_finite())
}

/// The Y value of `column` in `row`, or `f64::NAN` when the cell is absent,
/// null, or not a number.
fn cell_value(row: &[Value], column: u16) -> f64 {
    row.get(usize::from(column))
        .and_then(numeric_cell)
        .unwrap_or(f64::NAN)
}

/// The display name of a column: its header, or a positional fallback when the
/// result set gave the column no name.
fn column_label(header: &str, column: u16) -> String {
    if header.trim().is_empty() {
        format!("column {}", u32::from(column).saturating_add(1))
    } else {
        header.to_owned()
    }
}

/// Every mostly-numeric column, in result-set order.
fn numeric_columns(columns: &[String], rows: &[Vec<Value>]) -> Vec<SeriesSpec> {
    let mut series = Vec::new();
    for (index, header) in columns.iter().enumerate() {
        let Ok(column) = u16::try_from(index) else {
            break;
        };
        let mut non_null = 0usize;
        let mut numeric = 0usize;
        for row in rows {
            match row.get(index) {
                None => {}
                Some(cell) if cell.is_null() => {}
                Some(cell) => {
                    non_null = non_null.saturating_add(1);
                    if numeric_cell(cell).is_some() {
                        numeric = numeric.saturating_add(1);
                    }
                }
            }
        }
        if non_null > 0 && numeric.saturating_mul(2) >= non_null && numeric >= MIN_POINTS {
            series.push(SeriesSpec {
                name: column_label(header, column),
                column,
            });
        }
    }
    series
}

/// Every column that reads as ISO 8601 date/time — the temporal X-axis
/// candidates, in result-set order. A column already charted as a series is
/// never an axis candidate.
fn temporal_columns(
    columns: &[String],
    rows: &[Vec<Value>],
    series: &[SeriesSpec],
) -> Vec<(u16, String)> {
    let mut candidates = Vec::new();
    for (index, header) in columns.iter().enumerate() {
        let Ok(column) = u16::try_from(index) else {
            break;
        };
        if series.iter().any(|spec| spec.column == column) {
            continue;
        }
        let mut non_null = 0usize;
        let mut temporal = 0usize;
        for row in rows {
            match row.get(index) {
                None => {}
                Some(cell) if cell.is_null() => {}
                Some(cell) => {
                    non_null = non_null.saturating_add(1);
                    if cell.as_str().and_then(iso_epoch_seconds).is_some() {
                        temporal = temporal.saturating_add(1);
                    }
                }
            }
        }
        if non_null > 0 && temporal.saturating_mul(2) >= non_null && temporal >= MIN_POINTS {
            candidates.push((column, column_label(header, column)));
        }
    }
    candidates
}

/// The row-order axis: X is the row's position in the fetched page, so an
/// `ORDER BY` in the query is the order on screen.
fn row_order_axis(series: &[SeriesSpec], rows: &[Vec<Value>]) -> AxisSpec {
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "row ordinals of one page are tiny"
    )]
    let points = rows
        .iter()
        .enumerate()
        .map(|(index, row)| ChartRow {
            x: index as f64,
            values: series
                .iter()
                .map(|spec| cell_value(row, spec.column))
                .collect(),
        })
        .collect();
    AxisSpec {
        label: "Row order".to_owned(),
        scale: AxisScale::RowOrder,
        rows: points,
    }
}

/// The time axis over `column`: the rows whose cell parses as ISO 8601, sorted
/// by instant. `None` when too few rows survive, or when no series keeps two
/// points on it (a lone dot is not a line).
fn time_axis(
    series: &[SeriesSpec],
    label: String,
    column: u16,
    rows: &[Vec<Value>],
) -> Option<AxisSpec> {
    let mut points: Vec<ChartRow> = rows
        .iter()
        .filter_map(|row| {
            let x = row
                .get(usize::from(column))
                .and_then(Value::as_str)
                .and_then(iso_epoch_seconds)?;
            Some(ChartRow {
                x,
                values: series
                    .iter()
                    .map(|spec| cell_value(row, spec.column))
                    .collect(),
            })
        })
        .collect();
    // `total_cmp` is a total order and the sort is stable (equal instants keep
    // result-set order), so the axis is deterministic on both targets.
    points.sort_by(|left, right| left.x.total_cmp(&right.x));
    if points.len() < MIN_POINTS || !any_series_plottable(series.len(), &points) {
        return None;
    }
    Some(AxisSpec {
        label,
        scale: AxisScale::Time,
        rows: points,
    })
}

/// Whether at least one series still has [`MIN_POINTS`] real values once the
/// rows without an X position have been dropped.
fn any_series_plottable(series_count: usize, points: &[ChartRow]) -> bool {
    (0..series_count).any(|index| {
        points
            .iter()
            .filter(|row| row.values.get(index).copied().is_some_and(f64::is_finite))
            .count()
            >= MIN_POINTS
    })
}

// ---------------------------------------------------------------------------
// ISO 8601 <-> epoch seconds (jiff — the workspace's ISO 8601 crate; only
// pure parsing/formatting is used, so no clock and no tzdb, which is what
// keeps it wasm-clean)
// ---------------------------------------------------------------------------

/// Seconds since the Unix epoch for an ISO 8601 date or date/time, or `None`
/// for anything else (a number, a partial date, junk).
///
/// Accepts `YYYY-MM-DD`, optionally followed by `T` (or a space) and
/// `hh:mm[:ss[.fraction]]`, optionally followed by `Z` or a `±hh[:mm]` offset —
/// the shapes an openEHR `DV_DATE`/`DV_DATE_TIME` value takes on the wire.
/// Fractional seconds are accepted and dropped: this places points on an axis,
/// it does not re-serialize instants. A value with no offset is read as UTC,
/// which shifts a whole column uniformly and so never reorders it.
#[must_use]
pub fn iso_epoch_seconds(text: &str) -> Option<f64> {
    let text = text.trim();
    // Reject bare years / year-month early: jiff's `Date` parser demands a full
    // date too, but a plain number ("37") must not fall through to any parser
    // that could read it as something else.
    let seconds = if let Ok(timestamp) = text.parse::<jiff::Timestamp>() {
        // A full instant (offset present, `Z` / `±hh:mm` / `±hhmm`).
        timestamp.as_second()
    } else if let Ok(datetime) = text.parse::<jiff::civil::DateTime>() {
        // Offset-less date/time — read as UTC (uniform shift, order-preserving).
        jiff::tz::TimeZone::UTC
            .to_timestamp(datetime)
            .ok()?
            .as_second()
    } else if let Ok(date) = text.parse::<jiff::civil::Date>() {
        // Date only — midnight UTC.
        jiff::tz::TimeZone::UTC
            .to_timestamp(date.at(0, 0, 0, 0))
            .ok()?
            .as_second()
    } else {
        return None;
    };
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "instants are inside f64's exact integer range"
    )]
    Some(seconds as f64)
}

/// Label one tick of a time axis whose visible span is `span_seconds` wide.
///
/// The span picks the granularity, so a single day of readings reads as clock
/// times, a few months as `MM-DD hh:mm`, and a multi-year series as dates — a
/// label never repeats what the axis already makes obvious. A tick that is not
/// a finite instant (or is outside jiff's representable range, ±9999 years)
/// renders as `-`, the same placeholder chartistry's own float formatter uses.
#[must_use]
pub fn time_tick_label(seconds: f64, span_seconds: f64) -> String {
    if !seconds.is_finite() || seconds.abs() > 1e12 {
        return "-".to_owned();
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "range-guarded immediately above"
    )]
    let Ok(timestamp) = jiff::Timestamp::from_second(seconds as i64) else {
        return "-".to_owned();
    };
    let civil = jiff::tz::TimeZone::UTC.to_datetime(timestamp);
    if span_seconds <= DAY {
        format!("{:02}:{:02}", civil.hour(), civil.minute())
    } else if span_seconds <= DAY * 366.0 {
        format!(
            "{:02}-{:02} {:02}:{:02}",
            civil.month(),
            civil.day(),
            civil.hour(),
            civil.minute()
        )
    } else {
        format!(
            "{:04}-{:02}-{:02}",
            civil.year(),
            civil.month(),
            civil.day()
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AxisScale, ChartRow, derive, iso_epoch_seconds, time_tick_label};

    /// Every mostly-numeric column becomes its own series, named by the result
    /// set's own header — the multi-series re-specification of the former
    /// "chart the FIRST mostly-numeric column" rule, with that rule's cell
    /// cases (numeric text, nulls, a text column) intact.
    #[test]
    fn derives_one_series_per_mostly_numeric_column() {
        let columns = vec![
            "name".to_owned(),
            "magnitude".to_owned(),
            "systolic".to_owned(),
        ];
        let rows = vec![
            vec![json!("a"), json!(37.2), json!(120)],
            vec![json!("b"), json!("38.1"), json!(122)],
            vec![json!("c"), serde_json::Value::Null, json!(118)],
        ];
        let model = derive(&columns, &rows).expect("chartable");
        let names: Vec<&str> = model.series.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["magnitude", "systolic"]);
        assert_eq!(
            model.series.iter().map(|s| s.column).collect::<Vec<_>>(),
            vec![1, 2]
        );
        // The text column is not a series; the only axis is the row order,
        // whose rows carry one Y per series (NaN where the cell was null).
        let axis = model.axis(0).expect("an axis");
        assert_eq!(axis.scale, AxisScale::RowOrder);
        assert_eq!(axis.label, "Row order");
        assert_eq!(
            axis.rows.first(),
            Some(&ChartRow {
                x: 0.0,
                values: vec![37.2, 120.0],
            })
        );
        assert_eq!(
            axis.rows.get(1),
            Some(&ChartRow {
                x: 1.0,
                values: vec![38.1, 122.0],
            })
        );
        let last = axis.rows.get(2).expect("the third row");
        assert!(last.values.first().is_some_and(|v| v.is_nan()));
        assert_eq!(last.values.get(1), Some(&118.0));
    }

    /// One numeric column is the degenerate case: exactly one series over the
    /// row order — what the single-column predecessor produced.
    #[test]
    fn a_single_numeric_column_derives_one_series() {
        let columns = vec!["name".to_owned(), "magnitude".to_owned()];
        let rows = vec![
            vec![json!("a"), json!(37.2)],
            vec![json!("b"), json!("38.1")],
            vec![json!("c"), serde_json::Value::Null],
        ];
        let model = derive(&columns, &rows).expect("chartable");
        assert_eq!(model.series.len(), 1);
        assert_eq!(model.axes.len(), 1);
        let axis = model.axis(0).expect("an axis");
        assert_eq!(
            axis.rows.iter().map(|row| row.x).collect::<Vec<_>>(),
            vec![0.0, 1.0, 2.0]
        );
        assert_eq!(
            axis.rows.first().map(|row| row.values.clone()),
            Some(vec![37.2])
        );
    }

    /// Text-only, empty, and single-point result sets are not charts — the
    /// rejection cases the predecessor pinned, re-asserted on the model.
    #[test]
    fn rejects_text_only_empty_and_single_point_result_sets() {
        let text_only = vec![vec![json!("a")], vec![json!("b")]];
        assert!(derive(&["name".to_owned()], &text_only).is_none());
        assert!(derive(&["v".to_owned()], &[]).is_none());
        assert!(derive(&[], &[]).is_none());
        let single = vec![vec![json!(1.0)]];
        assert!(derive(&["v".to_owned()], &single).is_none());
        // Numeric in only two of five non-null cells: not "mostly numeric".
        let mostly_text = vec![
            vec![json!("x")],
            vec![json!("y")],
            vec![json!("z")],
            vec![json!(1)],
            vec![json!(2)],
        ];
        assert!(derive(&["v".to_owned()], &mostly_text).is_none());
    }

    /// An ISO 8601 column is offered as a real time axis ahead of the row
    /// order, with its rows sorted by instant whatever the row order was.
    #[test]
    fn offers_a_temporal_axis_sorted_by_instant() {
        let columns = vec!["observed".to_owned(), "magnitude".to_owned()];
        let rows = vec![
            vec![json!("2026-07-16T08:00:00Z"), json!(39.1)],
            vec![json!("2026-07-14T08:00:00Z"), json!(36.5)],
            vec![json!("2026-07-15T08:00:00Z"), json!(37.8)],
        ];
        let model = derive(&columns, &rows).expect("chartable");
        assert_eq!(model.series.len(), 1, "the timestamp column is the axis");
        assert_eq!(model.axes.len(), 2, "the temporal axis plus the row order");
        let time = model.axis(0).expect("the default axis");
        assert_eq!(time.scale, AxisScale::Time);
        assert_eq!(time.label, "observed");
        assert_eq!(
            time.rows
                .iter()
                .map(|row| row.values.clone())
                .collect::<Vec<_>>(),
            vec![vec![36.5], vec![37.8], vec![39.1]]
        );
        assert!((time.span() - 2.0 * 86_400.0).abs() < f64::EPSILON);
        assert_eq!(
            model.axis(1).map(|axis| axis.scale),
            Some(AxisScale::RowOrder)
        );
    }

    /// A timestamp column with too few parseable cells is not an axis, and the
    /// chart falls back to the row order.
    #[test]
    fn falls_back_to_row_order_without_a_temporal_column() {
        let columns = vec!["observed".to_owned(), "magnitude".to_owned()];
        let rows = vec![
            vec![json!("not a date"), json!(36.5)],
            vec![json!("2026-07-15T08:00:00Z"), json!(37.8)],
            vec![json!("also not a date"), json!(39.1)],
        ];
        let model = derive(&columns, &rows).expect("chartable");
        assert_eq!(model.axes.len(), 1);
        assert_eq!(
            model.axis(0).map(|axis| axis.scale),
            Some(AxisScale::RowOrder)
        );
    }

    #[test]
    fn parses_the_iso_8601_shapes_a_result_set_carries() {
        assert_eq!(iso_epoch_seconds("1970-01-01"), Some(0.0));
        assert_eq!(iso_epoch_seconds("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(iso_epoch_seconds("1970-01-02"), Some(86_400.0));
        assert_eq!(iso_epoch_seconds("1969-12-31T23:59:59Z"), Some(-1.0));
        // 2026-07-14T08:00:00Z, the shape the seeded compositions carry.
        assert_eq!(
            iso_epoch_seconds("2026-07-14T08:00:00Z"),
            Some(1_784_016_000.0)
        );
        // A space separator, an absent seconds field, and a fraction all pass.
        assert_eq!(iso_epoch_seconds("2026-07-14 08:00"), Some(1_784_016_000.0));
        assert_eq!(
            iso_epoch_seconds("2026-07-14T08:00:00.123Z"),
            Some(1_784_016_000.0)
        );
        // An offset shifts the instant (east of UTC is earlier in UTC).
        assert_eq!(
            iso_epoch_seconds("2026-07-14T10:00:00+02:00"),
            Some(1_784_016_000.0)
        );
        assert_eq!(
            iso_epoch_seconds("2026-07-14T06:00:00-0200"),
            Some(1_784_016_000.0)
        );
        // A bare date is midnight UTC.
        assert_eq!(
            iso_epoch_seconds("2026-07-14"),
            Some(1_784_016_000.0 - 28_800.0)
        );
    }

    #[test]
    fn rejects_everything_that_is_not_a_date() {
        for text in [
            "",
            "37.2",
            "2026",
            "2026-07",
            "26-07-14",
            "2026-13-01",
            "2026-07-32",
            "2026-07-14T25:00:00Z",
            "2026-07-14T08:61:00Z",
            "2026-07-14T08:00:00+99:00",
            "yesterday",
            "PT1H",
        ] {
            assert!(
                iso_epoch_seconds(text).is_none(),
                "`{text}` must not parse as an instant"
            );
        }
    }

    #[test]
    fn tick_labels_follow_the_axis_span() {
        let instant = 1_784_016_000.0; // 2026-07-14T08:00:00Z
        assert_eq!(time_tick_label(instant, 3_600.0), "08:00");
        assert_eq!(time_tick_label(instant, 2.0 * 86_400.0), "07-14 08:00");
        assert_eq!(time_tick_label(instant, 800.0 * 86_400.0), "2026-07-14");
        assert_eq!(time_tick_label(f64::NAN, 3_600.0), "-");
        assert_eq!(time_tick_label(1e15, 3_600.0), "-");
    }
}
