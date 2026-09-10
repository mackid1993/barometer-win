// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The preview strip: the taskbar readout, drawn inside the settings window.
//
// Composing a stack blind is guesswork, and the whole point of a live preview
// is that a reading added to a stack appears in the column as it is added.
// So the preview is rebuilt from the model on every change and drawn with
// the same rules the strip uses - the same gap the user set, the
// same font, the same right-aligned two-row columns, the same weather mark
// beside its temperature - on a ground the color of the taskbar.
//
// It is a second painter, not the strip's own, because the strip's lives in
// `window.rs` and draws from a `StripModel` of already-laid-out columns. What
// is shared is the arithmetic that shapes the columns, which is in this file
// with no window near it, so it is tested; what is copied is the drawing
// order, which should move into one place once the strip reads its layout
// from the store.

use barometer_core::module::{ModuleId, Readout};
use barometer_core::stack::{format_sensor, StackLayout, StackMetric, UnitPrefs};
use barometer_core::taskbar::Density;
use barometer_core::weather::badge::Condition;
use barometer_core::weather::glyph;
use barometer_core::weather::models::TemperatureUnit;

use windows_sys::Win32::Foundation::SIZE;
use windows_sys::Win32::Graphics::Gdi::HFONT;

use super::gdi::{wide, Canvas};
use super::geometry::{to_px, Rect};
use super::model::{Model, Snapshot, StripItem};
use super::theme::{Color, Theme};

/// One column of the readout: a label over a value, or two values, or the
/// weather.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Column {
    pub top: String,
    pub bottom: String,
    /// The widest text this column may hold, so the preview holds still as
    /// the live numbers change, exactly as the strip does.
    pub reserved: String,
    pub badge: Option<Condition>,
    /// The top line is a label: drawn at 82% of the ink, so the eye lands
    /// on the values.
    pub label_top: bool,
    /// The reading could not be taken; drawn at 70% of the ink.
    pub dim: bool,
    /// A stack reading's caption, drawn before the value in the heading
    /// face with a colon: `CPU: 45%`. Empty for a module's own column,
    /// whose label is a line of its own.
    pub top_label: String,
    pub bottom_label: String,
}

/// One item on the strip, as one or more columns.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub item: StripItem,
    pub columns: Vec<Column>,
    /// Stacks draw a hairline between their columns, as the macOS Combined
    /// item does, whatever the gap setting says.
    pub separators: bool,
}

/// The strip's items in order, as cells, from the model and the live readings.
///
/// Hidden modules and empty stacks are left out, the way the strip leaves
/// them out. A stack in `Columns` layout pairs its readings two to a column,
/// one over the other, the way every column on the strip is two rows.
pub fn cells(model: &Model, snapshot: &Snapshot) -> Vec<Cell> {
    // Hardware temperatures, not the weather's: a stack's sensor readings are
    // drawn with `settings.sensors.temperature` a few lines below, and
    // reserving from the other setting gave the column the wrong width for
    // every user who had the two set differently.
    let units = UnitPrefs {
        hardware_fahrenheit: model.settings.sensors.temperature == TemperatureUnit::Fahrenheit,
        rate: model.settings.network_unit,
    };
    let upload_first = model.settings.network_upload_first;
    model
        .order
        .iter()
        .filter(|item| model.is_shown(**item))
        .map(|item| match item {
            StripItem::Module(id) => {
                let readout = snapshot.readout(*id).cloned().unwrap_or_else(|| sample_readout_in(*id, upload_first));
                Cell { item: *item, columns: vec![module_column(*id, &readout)], separators: false }
            }
            StripItem::Stack(id) => {
                let stack = model.stack(*id).expect("shown stacks exist");
                let temperature = model.settings.sensors.temperature;
                let readings: Vec<(String, String, String)> = stack
                    .metrics
                    .iter()
                    .map(|entry| {
                        let value = metric_value(&entry.metric, snapshot, upload_first, temperature);
                        (
                            entry.caption(&snapshot.sensors),
                            value,
                            entry.metric.reserved_value_in(&snapshot.sensors, units).to_string(),
                        )
                    })
                    .collect();
                // Label and value are kept apart: the renderer draws the
                // label in the heading face with a colon after it, which is
                // the difference between a readout and a jumble of words.
                let columns = match stack.layout {
                    StackLayout::Columns => readings
                        .chunks(2)
                        .map(|pair| {
                            let (top, top_label, bottom, bottom_label) = match pair {
                                [a, b] => (a.1.clone(), a.0.clone(), b.1.clone(), b.0.clone()),
                                [a] => (a.1.clone(), a.0.clone(), String::new(), String::new()),
                                _ => (String::new(), String::new(), String::new(), String::new()),
                            };
                            // The wider of the pair's reserved *values*; the
                            // first when they tie, so the choice is stable.
                            //
                            // The caption is deliberately not in here. It is
                            // drawn in the heading face, and this string is
                            // measured in the value face, so a caption folded
                            // into it is measured in the wrong font - and only
                            // one of the two rows' captions would be counted.
                            // The renderers add the wider caption themselves,
                            // where they know which face draws it.
                            let reserved = pair.iter().map(|r| r.2.clone()).fold(
                                String::new(),
                                |best, s| if s.chars().count() > best.chars().count() { s } else { best },
                            );
                            Column {
                                top,
                                bottom,
                                reserved,
                                badge: None,
                                label_top: false,
                                dim: false,
                                top_label,
                                bottom_label,
                            }
                        })
                        .collect(),
                    _ => readings
                        .iter()
                        .map(|(label, value, reserved)| Column {
                            top: value.clone(),
                            bottom: String::new(),
                            // The value alone; the caption is `top_label` and
                            // the renderers add its width in the heading face.
                            reserved: reserved.clone(),
                            badge: None,
                            label_top: false,
                            dim: false,
                            top_label: label.clone(),
                            bottom_label: String::new(),
                        })
                        .collect(),
                };
                Cell { item: *item, columns, separators: true }
            }
        })
        .collect()
}

/// A stack reading's label as drawn: with one colon, whatever the user typed.
///
/// Labels were typed with a colon by people who wanted one before the
/// renderer drew it; stripping and re-adding it means "CPU" and "CPU:" both
/// come out as `CPU:`.
pub fn label_text(label: &str) -> String {
    format!("{}:", label.trim_end().trim_end_matches(':').trim_end())
}

/// A module's own column, shaped the way `main.rs` shapes it for the strip.
fn module_column(id: ModuleId, readout: &Readout) -> Column {
    let reserved = readout.reserved.clone().unwrap_or_default();
    match (&readout.secondary, readout.badge) {
        (_, Some(condition)) => Column {
            top: String::new(),
            bottom: readout.primary.clone(),
            reserved,
            badge: Some(condition),
            label_top: false,
            dim: readout.unavailable,
            top_label: String::new(),
            bottom_label: String::new(),
        },
        (Some(second), None) => Column {
            top: readout.primary.clone(),
            bottom: second.clone(),
            reserved,
            badge: None,
            label_top: false,
            dim: readout.unavailable,
            top_label: String::new(),
            bottom_label: String::new(),
        },
        (None, None) => Column {
            top: id.label().to_string(),
            bottom: readout.primary.clone(),
            reserved,
            badge: None,
            label_top: true,
            dim: readout.unavailable,
            top_label: String::new(),
            bottom_label: String::new(),
        },
    }
}

/// The live value for a stack reading, or the placeholder the strip shows
/// while there is none.
///
/// The engine publishes one readout per module, not one per metric, so only
/// each module's primary reading has a live number; the rest show "--" the
/// way an unavailable reading does. This is honest about what the engine
/// produces today, and the column is sized from the reserved string either
/// way, so it will not move when the numbers arrive.
fn metric_value(
    metric: &StackMetric,
    snapshot: &Snapshot,
    upload_first: bool,
    temperature: TemperatureUnit,
) -> String {
    // A sensor reading is live for every sensor, not only a module's
    // primary: the source reports them all and the snapshot carries them.
    if let Some(sensor) = metric.sensor(&snapshot.sensors) {
        return format_sensor(sensor, temperature);
    }
    // A figure the owning module produced for exactly this reading.
    if let Some((_, value)) = snapshot.metric_values.iter().find(|(m, _)| m == metric) {
        return value.clone();
    }
    let module = metric.module();
    let readout = snapshot.readout(module).cloned().unwrap_or_else(|| sample_readout_in(module, upload_first));
    if readout.unavailable {
        return "--".to_string();
    }
    // The network module's two lines follow the upload-first switch, so the
    // download reading is the second line while that is on.
    let swapped = module == ModuleId::Network && upload_first;
    let (first, second) = if swapped {
        (readout.secondary.clone().unwrap_or_else(|| "--".to_string()), Some(readout.primary.clone()))
    } else {
        (readout.primary.clone(), readout.secondary.clone())
    };
    let primary = StackMetric::primary(module).as_ref() == Some(metric);
    match metric {
        _ if primary => first,
        StackMetric::NetworkUpload | StackMetric::DiskWrite => second.unwrap_or_else(|| "--".to_string()),
        _ => "--".to_string(),
    }
}

/// A plausible reading for a module, for the preview before the app has
/// published any and in the tests.
///
/// The figures are the design document's own example strip, so the preview
/// matches what the layouts were drawn against.
pub fn sample_readout(id: ModuleId) -> Readout {
    sample_readout_in(id, false)
}

/// `sample_readout`, with the network module's lines in the order the
/// settings ask for, as the module itself would publish them.
fn sample_readout_in(id: ModuleId, upload_first: bool) -> Readout {
    // The reserved strings here are the modules' own, not approximations of
    // them. The strip falls back to this readout for any module that has not
    // reported yet, so a sample that reserves less than the module does is a
    // column that widens on its first reading - and a preview that draws a
    // narrower strip than the one it is previewing.
    let rate = barometer_core::format::RateUnit::Bytes.widest();
    match id {
        ModuleId::Cpu => Readout::one("24%").reserving("100%"),
        ModuleId::Gpu => Readout::one("18%").reserving("100%"),
        ModuleId::Memory => Readout::one("61%").reserving("100%"),
        ModuleId::Disks => Readout::two("2.10 MB/s", "512 KB/s").reserving(rate),
        ModuleId::Network if upload_first => Readout::two("\u{2191} 1.20 KB/s", "\u{2193} 12.3 KB/s")
            .reserving(format!("\u{2193} {rate}")),
        ModuleId::Network => Readout::two("\u{2193} 12.3 KB/s", "\u{2191} 1.20 KB/s")
            .reserving(format!("\u{2193} {rate}")),
        ModuleId::Sensors => Readout::one("51\u{b0}C").reserving("100\u{b0}C"),
        // The wider of the two units, since the sample does not know which
        // one is set - see `reserved_air_temperature`.
        ModuleId::Weather => Readout::one("72\u{b0}F")
            .reserving(barometer_core::weather::models::reserved_air_temperature(
                TemperatureUnit::Fahrenheit,
            ))
            .with_badge(Condition::PartlyCloudy),
    }
}

/// The type size and row count the strip would choose for this taskbar.
pub fn density(model: &Model, snapshot: &Snapshot) -> Density {
    Density::choose(snapshot.taskbar_height_dip, model.settings.font.size_dip)
}

/// The header line over the composer: "3 items · 9 pt text".
pub fn summary(model: &Model, snapshot: &Snapshot) -> String {
    let count = model.shown_count();
    let density = density(model, snapshot);
    let items = if count == 1 { "1 item".to_string() } else { format!("{count} items") };
    format!("{items} · {} pt text", density.text_dip.round())
}

/// Gap between a weather mark and its temperature, in DIPs. `window.rs`'s
/// own figure.
const MARK_GAP_DIP: f32 = 3.0;

/// Gap between the columns of a stack, and the hairline between them, as the
/// macOS Combined item draws its members. Fixed, because inside one item
/// the spacing is the item's own and not the user's gap.
const STACK_GAP_DIP: f32 = 6.0;

/// The mark's size for a line height. `window.rs`'s own rule.
fn icon_size(line_height: i32) -> i32 {
    ((line_height as f32) * 1.05).round().max(8.0) as i32
}

/// Paints the strip into a rectangle, centered as the strip centers itself
/// in the space the tray reservation gives it.
///
/// Returns where each item landed, in the canvas's DIPs, so the pointer can
/// be matched to an item from the same numbers the picture was drawn with.
pub fn paint(
    canvas: &mut Canvas,
    theme: &Theme,
    area: Rect,
    model: &Model,
    snapshot: &Snapshot,
    light: bool,
    hover: Option<StripItem>,
) -> Vec<(StripItem, f32, f32)> {
    let ground = Theme::strip_ground(light);
    let ink = Theme::strip_ink(light);
    canvas.fill_round(area, super::ui::RADIUS_SURFACE, ground);
    canvas.stroke_round(area, super::ui::RADIUS_SURFACE, theme.stroke_card);
    let mut spans = Vec::new();

    let density = density(model, snapshot);
    let cells = cells(model, snapshot);
    let font = &model.settings.font;
    let scale = canvas.scale;
    let text_px = to_px(density.text_dip, scale);
    let text_font = canvas.fonts.get(&font.family, text_px, font.weight.dwrite_weight() as i32);
    // The heading row of a label-over-value column has a weight of its own;
    // everything else on the strip, a stack's "CPU 24%" included, is a value.
    // The heading face the user chose, or the value face when they chose
    // none - the same rule `window::ensure_heading_font` draws by. The
    // preview promises to show every change on the pane, and for a while it
    // built the headings from the value family and quietly broke that.
    let heading_family = font.heading_family.as_deref().unwrap_or(font.family.as_str());
    let heading_font = canvas.fonts.get(heading_family, text_px, font.heading_weight.dwrite_weight() as i32);

    // The band the strip occupies: the taskbar's height, centered in the
    // area, so a 32 DIP small-taskbar preview reads as short rather than as
    // a tall preview with small text in it.
    let band_h = snapshot.taskbar_height_dip.min(area.h);
    let band = Rect::new(area.x, area.y + (area.h - band_h) / 2.0, area.w, band_h);
    let band_px = band.px(scale);
    let height = band_px.height();

    if cells.is_empty() {
        let hint = "Nothing in the strip. Turn on an item to start.";
        canvas.text(band, hint, super::gdi::TextStyle::Caption, ink.over(ground, 0.82), super::gdi::Align::Center);
        return spans;
    }

    let gap = to_px(model.settings.column_gap_dip, scale);
    let stack_gap = to_px(STACK_GAP_DIP, scale);
    let mark_gap = to_px(MARK_GAP_DIP, scale);

    // Measure every column first, so the block can be centered.
    struct Measured {
        width: i32,
        top: (i32, i32),
        bottom: (i32, i32),
        mark: i32,
        /// How far into the line the value starts, after a label and its
        /// colon; zero when the line has no label.
        top_value_x: i32,
        bottom_value_x: i32,
    }
    let label_gap = canvas.font_measure(&wide(" "), text_font).cx;
    // A labeled line: the label in the heading face with its colon, a
    // space, then the value - measured as one extent so the column is laid
    // out at the width the two pieces actually take.
    let measure_line = |label: &str, text: &str, text_font: HFONT| -> (i32, i32, i32) {
        let value = canvas.font_measure(&wide(text), text_font);
        if label.is_empty() {
            return (value.cx, value.cy, 0);
        }
        let head = canvas.font_measure(&wide(&label_text(label)), heading_font);
        (head.cx + label_gap + value.cx, value.cy.max(head.cy), head.cx + label_gap)
    };
    let mut measured: Vec<Vec<Measured>> = Vec::with_capacity(cells.len());
    for cell in &cells {
        let mut columns = Vec::with_capacity(cell.columns.len());
        for column in &cell.columns {
            let top_font = if column.label_top { heading_font } else { text_font };
            let (top_cx, top_cy, top_value_x) = measure_line(&column.top_label, &column.top, top_font);
            let top = SIZE { cx: top_cx, cy: top_cy };
            let (bottom_cx, bottom_cy, bottom_value_x) =
                measure_line(&column.bottom_label, &column.bottom, text_font);
            let bottom = SIZE { cx: bottom_cx, cy: bottom_cy };
            // `reserved` is the value alone, so the caption's own width comes
            // from the measured line - `window.rs` does the same, and the two
            // have to agree or the preview is not a preview. It may also hold
            // several candidates; the column is as wide as the widest.
            let caption = top_value_x.max(bottom_value_x);
            let held = if column.reserved.is_empty() {
                0
            } else {
                caption
                    + column
                        .reserved
                        .split(barometer_core::module::RESERVED_SEPARATOR)
                        .map(|candidate| canvas.font_measure(&wide(candidate), text_font).cx)
                        .max()
                        .unwrap_or(0)
            };
            let (width, mark) = if column.badge.is_some() {
                let mark = icon_size(bottom.cy);
                (bottom.cx.max(held) + mark + mark_gap, mark)
            } else {
                (top.cx.max(bottom.cx).max(held), 0)
            };
            columns.push(Measured {
                width,
                top: (top.cx, top.cy),
                bottom: (bottom.cx, bottom.cy),
                mark,
                top_value_x,
                bottom_value_x,
            });
        }
        measured.push(columns);
    }

    let mut content = 0;
    for (index, columns) in measured.iter().enumerate() {
        if index > 0 {
            content += gap;
        }
        let inner: i32 = columns.iter().map(|c| c.width).sum();
        content += inner + stack_gap * (columns.len().saturating_sub(1) as i32);
    }

    let width = band_px.width();
    let mut x = band_px.left
        + ((width - content) / 2).max(0);

    let label_ink = ink.over(ground, 0.82);
    let dim_ink = ink.over(ground, 0.70);
    let divider_ink = ink.over(ground, 0.24);

    let clip = canvas.clip(band);
    for (cell, columns) in cells.iter().zip(&measured) {
        let cell_width: i32 =
            columns.iter().map(|c| c.width).sum::<i32>() + stack_gap * (columns.len().saturating_sub(1) as i32);
        spans.push((cell.item, (x - gap / 2) as f32 / scale, (x + cell_width + gap / 2) as f32 / scale));

        // The hover backplate the taskbar gives its own buttons, so the item
        // boundaries teach themselves as the pointer crosses the preview.
        if hover == Some(cell.item) {
            let wash = Rect::new(
                (x - to_px(4.0, scale)) as f32 / scale,
                band.y + 4.0,
                (cell_width + to_px(8.0, scale)) as f32 / scale,
                band.h - 8.0,
            );
            let wash_ink = if light { Color(0x000000) } else { Color(0xFFFFFF) };
            canvas.fill_round_alpha(wash, 4.0, wash_ink, if light { 15 } else { 20 });
        }

        for (index, (column, size)) in cell.columns.iter().zip(columns).enumerate() {
            if index > 0 {
                x += stack_gap;
                if cell.separators {
                    let sep_x = (x - stack_gap / 2) as f32 / scale;
                    canvas.vline(sep_x, band.y + 6.0, band.bottom() - 6.0, divider_ink);
                }
            }
            let value_ink = if column.dim { dim_ink } else { ink };
            if let Some(condition) = column.badge {
                let mark = glyph::composed(condition);
                let mark_px = size.mark;
                let icon_font = canvas.fonts.get(glyph::FAMILY, mark_px, 400);
                let mark_y = band_px.top + (height - mark_px) / 2;
                canvas.font_text(x, mark_y, &[mark.base as u16], icon_font, value_ink);
                if let Some(accent) = mark.accent {
                    let accent_px = ((mark_px as f32) * accent.scale).round().max(1.0) as i32;
                    let accent_font = canvas.fonts.get(glyph::FAMILY, accent_px, 400);
                    canvas.font_text(
                        x + ((mark_px as f32) * accent.dx).round() as i32,
                        mark_y + ((mark_px as f32) * accent.dy).round() as i32,
                        &[accent.glyph as u16],
                        accent_font,
                        value_ink,
                    );
                }
                let text_y = band_px.top + (height - size.bottom.1) / 2;
                canvas.font_text(x + mark_px + mark_gap, text_y, &wide(&column.bottom), text_font, value_ink);
            } else if !column.bottom.is_empty() {
                let line = size.top.1.max(size.bottom.1);
                let top_y = band_px.top + (height - line * 2) / 2;
                let (top_font, top_ink) = if column.label_top { (heading_font, label_ink) } else { (text_font, value_ink) };
                let top_x = x + size.width - size.top.0;
                if !column.top_label.is_empty() {
                    canvas.font_text(top_x, top_y, &wide(&label_text(&column.top_label)), heading_font, label_ink);
                }
                canvas.font_text(top_x + size.top_value_x, top_y, &wide(&column.top), top_font, top_ink);
                let bottom_x = x + size.width - size.bottom.0;
                if !column.bottom_label.is_empty() {
                    canvas.font_text(
                        bottom_x,
                        top_y + line,
                        &wide(&label_text(&column.bottom_label)),
                        heading_font,
                        label_ink,
                    );
                }
                canvas.font_text(
                    bottom_x + size.bottom_value_x,
                    top_y + line,
                    &wide(&column.bottom),
                    text_font,
                    value_ink,
                );
            } else {
                let (text, label, extent, value_x) = if column.bottom.is_empty() {
                    (&column.top, &column.top_label, size.top, size.top_value_x)
                } else {
                    (&column.bottom, &column.bottom_label, size.bottom, size.bottom_value_x)
                };
                let line_x = x + size.width - extent.0;
                let line_y = band_px.top + (height - extent.1) / 2;
                if !label.is_empty() {
                    canvas.font_text(line_x, line_y, &wide(&label_text(label)), heading_font, label_ink);
                }
                canvas.font_text(line_x + value_x, line_y, &wide(text), text_font, value_ink);
            }
            x += size.width;
        }
        x += gap;
    }
    canvas.unclip(clip);
    spans
}

/// What the Text size slider says under itself: whether the taskbar took
/// the size as asked, which is the one consequence of the number that is
/// not obvious from the number.
pub fn size_caption(model: &Model, snapshot: &Snapshot) -> String {
    let asked = model.settings.font.size_dip;
    let density = density(model, snapshot);
    if density.held_to_the_bar(asked) {
        format!(
            "Held to {} pt, the largest two rows fit in your {} DIP taskbar. The strip is always two rows.",
            density.text_dip.round(),
            snapshot.taskbar_height_dip.round()
        )
    } else {
        "Two rows, label over value, at this size. The strip grows sideways to fit.".to_string()
    }
}

/// The item under a point in the preview, for the hover backplate.
///
/// Recomputed from the same widths the painter used, which the caller keeps
/// from the last paint, so the pointer and the picture agree.
pub fn item_at(spans: &[(StripItem, f32, f32)], x: f32) -> Option<StripItem> {
    spans.iter().find(|(_, left, right)| x >= *left && x < *right).map(|(item, _, _)| *item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use barometer_core::store::Settings;

    fn model() -> Model {
        let mut settings = Settings::default();
        for entry in settings.modules.iter_mut() {
            entry.enabled = true;
        }
        Model::from_settings(settings)
    }

    #[test]
    fn a_module_with_one_reading_is_a_label_over_a_value() {
        let cells = cells(&model(), &Snapshot::default());
        let cpu = cells.iter().find(|c| c.item == StripItem::Module(ModuleId::Cpu)).unwrap();
        assert_eq!(cpu.columns.len(), 1);
        assert_eq!(cpu.columns[0].top, "CPU");
        assert_eq!(cpu.columns[0].bottom, "24%");
        assert!(cpu.columns[0].label_top);
        assert_eq!(cpu.columns[0].reserved, "100%");
    }

    #[test]
    fn a_stack_row_shows_the_figure_its_module_produced_for_that_reading() {
        let snapshot = Snapshot {
            metric_values: vec![(StackMetric::CpuUser, "12%".to_string())],
            ..Default::default()
        };
        let value = metric_value(&StackMetric::CpuUser, &snapshot, true, TemperatureUnit::Celsius);
        assert_eq!(value, "12%");
    }

    #[test]
    fn two_readings_and_the_weather_keep_their_shapes() {
        let cells = cells(&model(), &Snapshot::default());
        let net = cells.iter().find(|c| c.item == StripItem::Module(ModuleId::Network)).unwrap();
        assert!(!net.columns[0].label_top);
        assert!(net.columns[0].top.starts_with('\u{2193}'));
        // Upload on top swaps the sample the way the module swaps its lines.
        let mut swapped = model();
        swapped.settings.network_upload_first = true;
        let cells = super::cells(&swapped, &Snapshot::default());
        let net = cells.iter().find(|c| c.item == StripItem::Module(ModuleId::Network)).unwrap();
        assert!(net.columns[0].top.starts_with('\u{2191}'));
        assert!(net.columns[0].bottom.starts_with('\u{2193}'));
        let wx = cells.iter().find(|c| c.item == StripItem::Module(ModuleId::Weather)).unwrap();
        assert_eq!(wx.columns[0].badge, Some(Condition::PartlyCloudy));
        assert_eq!(wx.columns[0].bottom, "72\u{b0}F");
    }

    #[test]
    fn live_readings_replace_the_samples_and_unavailable_ones_dim() {
        let snapshot = Snapshot {
            readouts: vec![
                (ModuleId::Cpu, Readout::one("87%")),
                (ModuleId::Memory, Readout::unavailable()),
            ],
            ..Snapshot::default()
        };
        let cells = cells(&model(), &snapshot);
        let cpu = cells.iter().find(|c| c.item == StripItem::Module(ModuleId::Cpu)).unwrap();
        assert_eq!(cpu.columns[0].bottom, "87%");
        let mem = cells.iter().find(|c| c.item == StripItem::Module(ModuleId::Memory)).unwrap();
        assert!(mem.columns[0].dim);
    }

    #[test]
    fn a_stack_in_columns_layout_pairs_its_readings_two_to_a_column() {
        let mut m = model();
        let id = m.add_stack(Some(StackMetric::CpuTotal));
        m.add_metric(id, StackMetric::GpuUtilization);
        m.add_metric(id, StackMetric::MemoryUsedPercent);
        let cells = cells(&m, &Snapshot::default());
        let stack = cells.iter().find(|c| c.item == StripItem::Stack(id)).unwrap();
        assert!(stack.separators);
        assert_eq!(stack.columns.len(), 2);
        assert_eq!(stack.columns[0].top, "24%");
        assert_eq!(stack.columns[0].top_label, "CPU");
        assert_eq!(stack.columns[0].bottom, "18%");
        assert_eq!(stack.columns[0].bottom_label, "GPU");
        assert_eq!(stack.columns[1].top, "61%");
        assert_eq!(stack.columns[1].top_label, "MEM");
        assert_eq!(stack.columns[1].bottom, "");
        // Sized from the widest thing a reading can show, not from today's -
        // and from the *value* alone. The caption goes with `top_label`, so
        // the renderers can measure it in the heading face it is drawn in
        // rather than in the value face this string is measured in.
        assert_eq!(stack.columns[0].reserved, "100%");
    }

    #[test]
    fn a_single_row_stack_runs_along_one_line_whatever_the_strip_does() {
        let mut m = model();
        let id = m.add_stack(Some(StackMetric::CpuTotal));
        m.add_metric(id, StackMetric::GpuUtilization);
        m.set_stack_layout(id, StackLayout::SingleRow);
        let cells = cells(&m, &Snapshot::default());
        let stack = cells.iter().find(|c| c.item == StripItem::Stack(id)).unwrap();
        assert_eq!(stack.columns.len(), 2);
        assert!(stack.columns.iter().all(|c| c.bottom.is_empty()));
    }

    #[test]
    fn hiding_sources_takes_the_module_column_out_of_the_preview() {
        let mut m = model();
        let id = m.add_stack(Some(StackMetric::CpuTotal));
        m.set_stack_hides_sources(id, true);
        let cells = cells(&m, &Snapshot::default());
        assert!(!cells.iter().any(|c| c.item == StripItem::Module(ModuleId::Cpu)));
        assert!(cells.iter().any(|c| c.item == StripItem::Stack(id)));
    }

    #[test]
    fn readings_without_a_live_figure_show_a_placeholder_not_a_number() {
        let mut m = model();
        let id = m.add_stack(Some(StackMetric::CpuUser));
        m.add_metric(id, StackMetric::NetworkUpload);
        let cells = cells(&m, &Snapshot::default());
        let stack = cells.iter().find(|c| c.item == StripItem::Stack(id)).unwrap();
        // Two readings pair into one column, one over the other.
        assert_eq!(stack.columns.len(), 1);
        assert_eq!(stack.columns[0].top, "--");
        assert_eq!(stack.columns[0].top_label, "USR");
        // Upload is the network module's second line, which does exist.
        assert_eq!(stack.columns[0].bottom_label, "UP");
        assert!(stack.columns[0].bottom.starts_with('\u{2191}'));
    }

    #[test]
    fn the_summary_counts_what_is_shown() {
        let mut m = model();
        let snapshot = Snapshot::default();
        assert!(summary(&m, &snapshot).starts_with("7 items"));
        for entry in m.settings.modules.iter_mut() {
            entry.enabled = false;
        }
        assert!(summary(&m, &snapshot).starts_with("0 items"));
    }

    #[test]
    fn the_item_under_the_pointer_comes_from_the_painted_spans() {
        let spans = vec![
            (StripItem::Module(ModuleId::Cpu), 10.0, 40.0),
            (StripItem::Stack(1), 52.0, 120.0),
        ];
        assert_eq!(item_at(&spans, 12.0), Some(StripItem::Module(ModuleId::Cpu)));
        assert_eq!(item_at(&spans, 45.0), None);
        assert_eq!(item_at(&spans, 100.0), Some(StripItem::Stack(1)));
    }

    #[test]
    fn every_module_has_a_sample_reading_shaped_like_its_real_one() {
        for id in ModuleId::ALL {
            let sample = sample_readout(id);
            assert!(!sample.primary.is_empty(), "{id}");
            assert!(sample.reserved.is_some(), "{id}");
        }
        assert!(sample_readout(ModuleId::Network).secondary.is_some());
        assert!(sample_readout(ModuleId::Weather).badge.is_some());
    }
}
