// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The hourly chart's geometry, ported from HourlyForecastChart,
// WeatherChartPlot and WeatherTemperatureGeometry in WeatherDropdownView.swift.
//
// A temperature curve with the area under it washed in, rain bars along the
// bottom, and a column of labels every third hour. Everything here is a
// coordinate in DIPs relative to the chart's own left edge, and nothing here
// draws; paint.rs turns it into pixels, and the tests can hold it to the
// Swift's proportions without a window.
//
// The chart is wider than the plate it sits on and scrolls sideways under
// it, as the Swift's does inside its horizontal ScrollView: forty-eight hours
// at 28 points each, never narrower than 720 on the overview or 336 on a
// day's page. A first port fitted the next twenty-four hours to the plate's
// width instead, to keep one wheel to one direction; that was a second
// chart, not this one, and it was changed back. The panel scrolls up and
// down; the chart scrolls left and right on a horizontal wheel, on the wheel
// with Shift held, and by dragging, and says so with a chip beside its
// label.

use barometer_core::weather::badge::Condition;
use barometer_core::weather::detail::{HourlyPoint, LocalTime};

use super::{clock, format};
use crate::settings_ui::geometry::Rect;

/// How many hours the overview plots, from the Swift's prefix(48).
pub const PLOTTED_HOURS: usize = 48;

/// A label under every third hour, as in the Swift.
pub const LABEL_EVERY: usize = 3;

/// The plate's height, from the Swift.
pub const HEIGHT: f32 = 150.0;

/// One hour's width, from the Swift's 28 points.
pub const HOUR_W: f32 = 28.0;

/// The narrowest the chart is drawn on the overview and on a day's page,
/// the Swift's `max(720, ...)` and `max(336, ...)`.
pub const OVERVIEW_MIN_W: f32 = 720.0;
pub const DAY_MIN_W: f32 = 336.0;

/// How wide the chart is for `count` hours: an hour's width each, and never
/// under the minimum, so a short series still spreads across the plate.
pub fn content_width(count: usize, minimum: f32) -> f32 {
    (count as f32 * HOUR_W).max(minimum)
}

/// A horizontal scroll offset held to what the content can actually show:
/// never past its left edge, and never so far that the plate goes empty.
pub fn clamp_offset(offset: f32, content_w: f32, viewport_w: f32) -> f32 {
    offset.clamp(0.0, (content_w - viewport_w).max(0.0))
}

/// The offset that brings a column into the middle of the plate, as near
/// as the ends of the content allow.
pub fn offset_showing(column: usize, step: f32, content_w: f32, viewport_w: f32) -> f32 {
    let center = (column as f32 + 0.5) * step;
    clamp_offset(center - viewport_w / 2.0, content_w, viewport_w)
}

/// The label column's rows, from the plate's top: the hour, the mark, the
/// temperature. The chance of rain sits under the bars, above the thumb.
pub const HOUR_ROW: (f32, f32) = (6.0, 14.0);
pub const MARK_ROW: (f32, f32) = (21.0, 20.0);
pub const TEMPERATURE_ROW: (f32, f32) = (42.0, 16.0);
pub const PROBABILITY_ROW_H: f32 = 12.0;
/// Where the chance of rain's row ends, from the plate's bottom: clear of
/// the scroll thumb the plate draws along its edge.
pub const PROBABILITY_ROW_BOTTOM: f32 = 12.0;

/// The hours still ahead, up to `count` of them.
///
/// Strictly the ones at or after `now`, which is the Swift's filter: the
/// hour in progress is excluded, so the first point plotted is the next
/// whole hour.
pub fn hours_to_plot(hourly: &[HourlyPoint], now: LocalTime, count: usize) -> Vec<&HourlyPoint> {
    hourly.iter().filter(|point| point.time >= now).take(count).collect()
}

/// One label column: the block of hours it names, and what it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// The block's left edge and width. The label is centered on the block
    /// of hours that starts here rather than on its own hour, as the Swift
    /// lays it out, so the first label is never cut by the plate's edge.
    pub x: f32,
    pub width: f32,
    pub hour: String,
    pub condition: Option<Condition>,
    pub temperature: String,
    /// The chance of rain, printed only when there is one.
    pub probability: Option<String>,
}

/// One plotted hour's column.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// The column's center.
    pub x: f32,
    /// Where the temperature landed on the curve, when there was one.
    pub dot: Option<(f32, f32)>,
}

/// The whole chart, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub width: f32,
    pub height: f32,
    /// One hour's share of the width.
    pub step: f32,
    pub columns: Vec<Column>,
    /// The temperature curve, through every hour that has a temperature.
    pub line: Vec<(f32, f32)>,
    /// The wash under it, closed along the baseline.
    pub area: Vec<(f32, f32)>,
    /// Where the area's gradient starts and ends, top and bottom.
    pub area_top: f32,
    pub area_bottom: f32,
    pub bars: Vec<Rect>,
    pub labels: Vec<Label>,
}

impl Geometry {
    /// Lays the chart out for a plate of `width` by `height`.
    pub fn new(points: &[&HourlyPoint], width: f32, height: f32) -> Geometry {
        let count = points.len();
        let step = if count > 0 { width / count as f32 } else { width };
        let temperatures: Vec<f64> = points.iter().filter_map(|point| point.temperature).collect();
        let minimum = temperatures.iter().copied().fold(f64::INFINITY, f64::min);
        let minimum = if minimum.is_finite() { minimum } else { 0.0 };
        let maximum = temperatures.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let maximum = if maximum.is_finite() { maximum } else { minimum + 1.0 };
        // A flat day still needs a curve to sit somewhere, so the spread is
        // never allowed to reach zero.
        let spread = (maximum - minimum).max(1.0);

        // The Swift's bands, measured from the plate's bottom, each lifted
        // eight so the chance of rain has a row of its own above the plate's
        // scroll thumb: printed at the Swift's height it sat on the thumb.
        let top = height - 76.0;
        let bottom = height - 34.0;
        let area_bottom = height - 30.0;
        let bar_base = height - 26.0;

        let mut columns = Vec::with_capacity(count);
        let mut line = Vec::new();
        let mut bars = Vec::new();
        for (index, point) in points.iter().enumerate() {
            let x = (index as f32 + 0.5) * step;
            let dot = point.temperature.map(|temperature| {
                let normalized = ((temperature - minimum) / spread) as f32;
                (x, bottom - normalized * (bottom - top))
            });
            if let Some(dot) = dot {
                line.push(dot);
            }
            columns.push(Column { x, dot });

            let probability = (point.precipitation_probability.unwrap_or(0.0) / 100.0) as f32;
            // Any chance at all gets a bar at least two DIPs tall, so a 3%
            // day is a sliver rather than nothing.
            let bar_h = if probability > 0.0 { (probability * 22.0).max(2.0) } else { 0.0 };
            if bar_h > 0.0 {
                let bar_w = (step * 0.56).max(2.0);
                bars.push(Rect::new(x - bar_w / 2.0, bar_base - bar_h, bar_w, bar_h));
            }
        }

        // The wash under the curve drops to the baseline at each end.
        let area = match (line.first(), line.last()) {
            (Some(first), Some(last)) if line.len() > 1 => {
                let mut area = Vec::with_capacity(line.len() + 2);
                area.push((first.0, area_bottom));
                area.extend(line.iter().copied());
                area.push((last.0, area_bottom));
                area
            }
            _ => Vec::new(),
        };

        let labels = points
            .iter()
            .enumerate()
            .filter(|(index, _)| index % LABEL_EVERY == 0)
            .map(|(index, point)| Label {
                x: index as f32 * step,
                width: step * LABEL_EVERY.min(count - index) as f32,
                hour: clock::hour_label(point.time),
                condition: point
                    .weather_code
                    .map(|code| Condition::for_code(code, point.is_day.unwrap_or(true))),
                temperature: format::degree(point.temperature),
                probability: point
                    .precipitation_probability
                    .filter(|chance| *chance > 0.0)
                    .map(|chance| format::percent(Some(chance))),
            })
            .collect();

        Geometry { width, height, step, columns, line, area, area_top: top, area_bottom, bars, labels }
    }

    /// The hour under a horizontal position on the plate.
    pub fn column_at(&self, x: f32) -> Option<usize> {
        if self.columns.is_empty() || x < 0.0 || x >= self.width || self.step <= 0.0 {
            return None;
        }
        Some(((x / self.step) as usize).min(self.columns.len() - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use barometer_core::weather::detail::parse_detail;

    /// Sixty hours: two days and the morning after, with a null temperature
    /// in the first afternoon's run-up and rain in the first afternoon.
    fn forecast() -> Vec<HourlyPoint> {
        let mut times = Vec::new();
        let mut temperatures = Vec::new();
        let mut chances = Vec::new();
        let mut codes = Vec::new();
        for hour in 0..60u32 {
            let date = ["2026-09-08", "2026-09-09", "2026-09-10"][(hour / 24) as usize];
            let clock = hour % 24;
            times.push(format!("\"{date}T{clock:02}:00\""));
            temperatures.push(if hour == 5 { "null".to_string() } else { format!("{}", 60 + hour % 24) });
            chances.push(if (14..18).contains(&hour) { "40" } else { "0" }.to_string());
            codes.push(if (14..18).contains(&hour) { "61" } else { "1" }.to_string());
        }
        let body = format!(
            r#"{{"hourly":{{"time":[{}],"temperature_2m":[{}],"precipitation_probability":[{}],"weather_code":[{}],"is_day":[{}]}}}}"#,
            times.join(","),
            temperatures.join(","),
            chances.join(","),
            codes.join(","),
            (0..60).map(|_| "1").collect::<Vec<_>>().join(",")
        );
        parse_detail(&body).unwrap().hourly
    }

    fn at(clock: &str) -> LocalTime {
        LocalTime::parse(clock).unwrap()
    }

    #[test]
    fn only_the_hours_still_ahead_are_plotted_and_no_more_than_asked_for() {
        let hourly = forecast();
        let plotted = hours_to_plot(&hourly, at("2026-09-08T14:30"), PLOTTED_HOURS);
        // 14:00 is in progress and excluded, as in the Swift; 15:00 is first.
        assert_eq!(plotted[0].time, at("2026-09-08T15:00"));
        // Nine hours remain today, twenty-four tomorrow and twelve the morning
        // after: forty-five, under the cap.
        assert_eq!(plotted.len(), 45);
        let capped = hours_to_plot(&hourly, at("2026-09-08T00:00"), PLOTTED_HOURS);
        assert_eq!(capped.len(), PLOTTED_HOURS);
        assert_eq!(PLOTTED_HOURS, 48, "the Swift's prefix(48)");
        assert!(hours_to_plot(&hourly, at("2026-09-11T00:00"), PLOTTED_HOURS).is_empty());
    }

    #[test]
    fn forty_eight_hours_lay_out_an_hour_wide_each_in_a_chart_wider_than_its_plate() {
        let hourly = forecast();
        let points = hours_to_plot(&hourly, at("2026-09-08T00:00"), PLOTTED_HOURS);
        let width = content_width(points.len(), OVERVIEW_MIN_W);
        assert_eq!(width, 48.0 * HOUR_W);
        assert!(width > 332.0, "the chart has to scroll under a 332 plate");
        let chart = Geometry::new(&points, width, HEIGHT);
        assert_eq!(chart.columns.len(), 48);
        assert!((chart.step - HOUR_W).abs() < 1e-4);
        // Sixteen labeled blocks of three hours, each 84 wide as the Swift's.
        assert_eq!(chart.labels.len(), 16);
        assert!((chart.labels[15].width - 3.0 * HOUR_W).abs() < 1e-4);
        assert!((chart.labels[15].x + chart.labels[15].width - width).abs() < 1e-3, "the last block ends at the chart's edge");
        // A short series is spread over the minimum rather than bunched left.
        assert_eq!(content_width(10, OVERVIEW_MIN_W), OVERVIEW_MIN_W);
        assert_eq!(content_width(24, DAY_MIN_W), 24.0 * HOUR_W);
    }

    #[test]
    fn the_label_columns_rows_do_not_overlap_and_the_mark_fits_its_row() {
        // The defect this pins from the layout's side: the mark row is
        // drawn at 18 DIPs inside a 20 DIP row, and the fitted mark stays in
        // those 18, so nothing of it can reach the hour above or the
        // temperature below.
        let (hour_top, hour_h) = HOUR_ROW;
        let (mark_top, mark_h) = MARK_ROW;
        let (temperature_top, _) = TEMPERATURE_ROW;
        assert!(hour_top + hour_h <= mark_top);
        assert!(mark_top + mark_h <= temperature_top);
        assert!(mark_h >= 18.0, "the mark is drawn at 18");
    }

    #[test]
    fn a_scroll_offset_is_held_between_the_charts_two_ends() {
        let content = 48.0 * HOUR_W;
        assert_eq!(clamp_offset(-20.0, content, 332.0), 0.0);
        assert_eq!(clamp_offset(500.0, content, 332.0), 500.0);
        assert_eq!(clamp_offset(5000.0, content, 332.0), content - 332.0);
        // Content narrower than the plate never scrolls.
        assert_eq!(clamp_offset(100.0, 300.0, 332.0), 0.0);
    }

    #[test]
    fn showing_a_column_centers_it_unless_an_end_of_the_chart_is_nearer() {
        let content = 24.0 * HOUR_W;
        // The two o'clock hour: its center at 14.5 columns, less half a plate.
        let offset = offset_showing(14, HOUR_W, content, 332.0);
        assert!((offset - (14.5 * HOUR_W - 166.0)).abs() < 1e-4);
        assert_eq!(offset_showing(0, HOUR_W, content, 332.0), 0.0);
        assert_eq!(offset_showing(23, HOUR_W, content, 332.0), content - 332.0);
    }

    #[test]
    fn the_curve_spans_the_band_from_its_coldest_hour_to_its_warmest() {
        let hourly = forecast();
        let points: Vec<&HourlyPoint> = hourly.iter().take(24).collect();
        let chart = Geometry::new(&points, 332.0, HEIGHT);
        let top = HEIGHT - 76.0;
        let bottom = HEIGHT - 34.0;
        // Midnight is the coldest hour and sits on the band's floor; 11pm the
        // warmest, on its ceiling.
        assert!((chart.columns[0].dot.unwrap().1 - bottom).abs() < 1e-3);
        assert!((chart.columns[23].dot.unwrap().1 - top).abs() < 1e-3);
        // The null hour has no dot and the curve simply skips it.
        assert_eq!(chart.columns[5].dot, None);
        assert_eq!(chart.line.len(), 23);
        assert_eq!(chart.columns.len(), 24);
        assert!((chart.step - 332.0 / 24.0).abs() < 1e-4);
    }

    #[test]
    fn the_wash_drops_to_the_baseline_at_both_ends() {
        let hourly = forecast();
        let points: Vec<&HourlyPoint> = hourly.iter().take(24).collect();
        let chart = Geometry::new(&points, 332.0, HEIGHT);
        let first = chart.area.first().unwrap();
        let last = chart.area.last().unwrap();
        assert_eq!(first.1, chart.area_bottom);
        assert_eq!(last.1, chart.area_bottom);
        assert_eq!(first.0, chart.line[0].0);
        assert_eq!(chart.area.len(), chart.line.len() + 2);
    }

    #[test]
    fn rain_bars_stand_only_where_there_is_a_chance_and_never_vanish() {
        let hourly = forecast();
        let points: Vec<&HourlyPoint> = hourly.iter().take(24).collect();
        let chart = Geometry::new(&points, 332.0, HEIGHT);
        // Four wet hours, four bars, each 40% of the 22 DIP maximum.
        assert_eq!(chart.bars.len(), 4);
        for bar in &chart.bars {
            assert!((bar.h - 8.8).abs() < 1e-3);
            assert!((bar.bottom() - (HEIGHT - 26.0)).abs() < 1e-3);
        }
        // A 1% chance is still a two-DIP sliver.
        let mut faint = hourly[0].clone();
        faint.precipitation_probability = Some(1.0);
        let chart = Geometry::new(&[&faint, &hourly[1]], 100.0, HEIGHT);
        assert_eq!(chart.bars.len(), 1);
        assert_eq!(chart.bars[0].h, 2.0);
    }

    #[test]
    fn labels_come_every_third_hour_and_sit_on_their_block() {
        let hourly = forecast();
        let points: Vec<&HourlyPoint> = hourly.iter().take(24).collect();
        let chart = Geometry::new(&points, 332.0, HEIGHT);
        assert_eq!(chart.labels.len(), 8);
        assert_eq!(chart.labels[0].hour, "12am");
        assert_eq!(chart.labels[1].hour, "3am");
        assert_eq!(chart.labels[5].hour, "3pm");
        assert_eq!(chart.labels[0].x, 0.0);
        assert!((chart.labels[1].x - 3.0 * chart.step).abs() < 1e-4);
        assert!((chart.labels[0].width - 3.0 * chart.step).abs() < 1e-4);
        // The wet block names its chance; the dry ones say nothing.
        assert_eq!(chart.labels[5].probability.as_deref(), Some("40%"));
        assert_eq!(chart.labels[0].probability, None);
        assert_eq!(chart.labels[5].condition, Some(Condition::Rain));
        assert_eq!(chart.labels[0].temperature, "60\u{00B0}");
    }

    #[test]
    fn a_short_series_gives_its_last_label_only_the_hours_that_remain() {
        let hourly = forecast();
        let points: Vec<&HourlyPoint> = hourly.iter().take(7).collect();
        let chart = Geometry::new(&points, 140.0, HEIGHT);
        assert_eq!(chart.labels.len(), 3);
        assert!((chart.labels[2].width - chart.step).abs() < 1e-4);
        // And the block never runs past the chart's edge, where the plate
        // would cut its words in half.
        assert!(chart.labels[2].x + chart.labels[2].width <= 140.0 + 1e-3);
    }

    #[test]
    fn a_position_on_the_plate_names_the_hour_under_it() {
        let hourly = forecast();
        let points: Vec<&HourlyPoint> = hourly.iter().take(24).collect();
        let chart = Geometry::new(&points, 240.0, HEIGHT);
        assert_eq!(chart.column_at(0.0), Some(0));
        assert_eq!(chart.column_at(9.9), Some(0));
        assert_eq!(chart.column_at(10.0), Some(1));
        assert_eq!(chart.column_at(239.9), Some(23));
        assert_eq!(chart.column_at(240.0), None);
        assert_eq!(chart.column_at(-1.0), None);
        assert_eq!(Geometry::new(&[], 240.0, HEIGHT).column_at(10.0), None);
    }

    #[test]
    fn an_empty_series_lays_out_nothing_rather_than_dividing_by_zero() {
        let chart = Geometry::new(&[], 332.0, HEIGHT);
        assert!(chart.columns.is_empty() && chart.line.is_empty() && chart.area.is_empty());
        assert!(chart.labels.is_empty() && chart.bars.is_empty());
        assert!(chart.step.is_finite());
    }
}
