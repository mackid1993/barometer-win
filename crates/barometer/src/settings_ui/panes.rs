// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The panes: which controls each one shows, in what order, saying what.
//
// Everything here is a pure function from the model and the facts beside it
// to a list of elements, plus the pure inverse - what choosing item three of
// a dropdown, or dragging a slider to 11, does to the model. Keeping the two
// halves in one file is what stops a dropdown's list and its handler drifting
// apart, and keeping both free of any window handle is what lets the tests
// exercise every pane without one.
//
// The composer is where a module and a stack have to be told apart, and the
// telling is done in words next to the switch rather than by a diagram:
// a module's row says "Hidden while Main is on"; its inspector's switch is
// captioned with what that stack is doing to it; and the stack's inspector
// names the modules it hides. The preview above them all shows the result.

use barometer_core::format::RateUnit;
use barometer_core::settings::FontWeight;
use barometer_core::stack::{StackLayout, StackMetric};
use barometer_core::weather::models::{
    Location, PrecipitationUnit, PressureUnit, TemperatureUnit, WindSpeedUnit,
    REFRESH_INTERVAL_MINUTES,
};
use barometer_core::ModuleId;

use super::geometry::Rect;
use super::model::{
    self, clamp_gap, clamp_padding, clamp_poll, clamp_size, GpuChoice, Model, SensorSource,
    Snapshot, StripItem,
};
use super::preview;
use super::system;
use super::theme::{self, STACK_COLOR};
use super::ui::{
    Builder, ButtonSpec, DropdownSpec, Element, Id, Ink, Kind, Mark, Measure, Pane, DROPDOWN_W,
    NAV_HEADING_H, NAV_ITEM_H, NAV_W, PANE_PAD, POPUP_ITEM_H,
};
use crate::{lhm_install, pawnio};

/// How the LibreHardwareMonitor download is going, as the pane shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum LhmState {
    Idle,
    Installing(lhm_install::Progress),
    /// The files are being deleted, on a thread of its own.
    ///
    /// A state rather than nothing, because removal takes seconds - the
    /// sensor helper is asked to let go and given three seconds to do it -
    /// and without something on screen the pane would look untouched until
    /// the library suddenly vanished from it.
    Removing,
    Installed(String),
    Failed(String),
}

/// The location search, as the pane shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchView {
    pub query: String,
    pub results: Vec<Location>,
    pub pending: bool,
    pub error: Option<String>,
    /// A search has returned for the current query, so "no results" means
    /// nothing matched rather than nothing has been asked yet.
    pub answered: bool,
}

/// The update check, as the pane shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateView {
    pub checking: bool,
    pub result: Option<String>,
}

/// Everything a pane reads besides the model.
pub struct View<'a> {
    pub model: &'a Model,
    pub snapshot: &'a Snapshot,
    pub selection: Option<StripItem>,
    pub families: &'a [String],
    /// Every (family, weight) GDI reports, so the weight controls can offer
    /// only the weights a family actually has.
    pub faces: &'a [(String, i32)],
    /// The unfolded family list, for asking whether a weight has a real face.
    pub instances: &'a [String],
    pub pawnio: pawnio::Status,
    pub lhm: &'a LhmState,
    pub search: &'a SearchView,
    pub update: &'a UpdateView,
    /// The field that currently hosts the EDIT child.
    pub editing: Option<Id>,
    /// A composer row being dragged, and the index it would drop at.
    pub drag: Option<(StripItem, usize)>,
    /// Which appearance the preview strip is showing.
    pub preview_light: bool,
    pub version: &'a str,
    /// Whether Barometer is in the per-user Run key.
    ///
    /// Read from the registry rather than kept in the settings file: the user
    /// can take it out from Task Manager's Startup tab, and a switch drawn
    /// from a copy in a file of ours would then be showing something that is
    /// not true.
    pub starts_at_login: bool,
}

/// The project's pages, for the links that open a browser.
///
/// This program's own repository. The macOS app has its own, and the two are
/// separate implementations rather than one program built twice - different
/// languages, different features, different release numbers.
pub const REPOSITORY_URL: &str = "https://github.com/mackid1993/barometer-win";
/// The macOS app, for the About pane to point at.
pub const MACOS_REPOSITORY_URL: &str = "https://github.com/mackid1993/Barometer";
pub const LHM_URL: &str = "https://github.com/LibreHardwareMonitor/LibreHardwareMonitor";
pub const OPEN_METEO_URL: &str = "https://open-meteo.com/";
/// PawnIO's own site. `pawnio.rs` holds the same address privately for its
/// first-run prompt; this is the second copy, and the report asks for the
/// first to be made public so there is one.
pub const PAWNIO_URL: &str = "https://pawnio.eu/";
pub const PAWNIO_SOURCE_URL: &str = "https://github.com/namazso/PawnIO";
pub const GPL_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

/// The navigation pane, in window coordinates.
///
/// About is pinned to the bottom, as the Settings app pins its own last
/// item, so it stays where a person expects it however many panes there are.
pub fn nav(height: f32, selected: Pane, measure: Measure) -> Vec<Element> {
    let mut b = Builder::new(8.0, 8.0, NAV_W - 16.0, measure);
    for pane in Pane::ALL {
        // The modules are a group, under a word that says so, as the Swift's
        // sidebar puts them under "Modules".
        if pane == Pane::MODULES[0] {
            let rect = Rect::new(8.0, b.y() + 6.0, NAV_W - 16.0, NAV_HEADING_H);
            b.push(Element {
                id: Id::None,
                rect,
                kind: Kind::NavHeading { text: "Modules".to_string() },
                interactive: false,
                focusable: false,
            });
            b.advance(NAV_HEADING_H + 10.0);
        }
        let y = if pane == Pane::About { height - 8.0 - NAV_ITEM_H } else { b.y() };
        let rect = Rect::new(8.0, y, NAV_W - 16.0, NAV_ITEM_H);
        b.push(Element {
            id: Id::Nav(pane),
            rect,
            kind: Kind::NavItem {
                text: pane.title().to_string(),
                selected: pane == selected,
                mark: pane.mark(),
            },
            interactive: true,
            focusable: true,
        });
        if pane != Pane::About {
            b.advance(NAV_ITEM_H + 2.0);
        }
        // A gap after the last module, so the group reads as one.
        if pane == Pane::MODULES[Pane::MODULES.len() - 1] {
            b.advance(6.0);
        }
    }
    b.elements
}

/// A pane's content, in content coordinates, and how tall it is.
pub fn content(pane: Pane, view: &View, width: f32, measure: Measure) -> (Vec<Element>, f32) {
    let inner = width - 2.0 * PANE_PAD;
    let mut b = Builder::new(PANE_PAD, PANE_PAD, inner, measure);
    match pane {
        Pane::Strip => strip_pane(&mut b, view, measure),
        Pane::Module(id) => module_page(&mut b, view, id),
        Pane::Stacks => stacks_page(&mut b, view, measure),
        Pane::Appearance => appearance_pane(&mut b, view),
        Pane::General => general_pane(&mut b, view),
        Pane::About => about_pane(&mut b, view),
    }
    // Room under the last card, so it does not sit on the window's edge.
    let height = b.y() + 32.0;
    (b.elements, height)
}

/// The preview strip with its flip button, full width of the pane.
fn preview_block(b: &mut Builder, view: &View) {
    let height = view.snapshot.taskbar_height_dip.max(40.0) + 16.0;
    let rect = Rect::new(b.x(), b.y(), b.width(), height);
    b.push(Element { id: Id::Preview, rect, kind: Kind::Preview, interactive: true, focusable: false });
    let flip = Rect::new(rect.right() - 8.0 - 32.0, rect.y + (height - 32.0) / 2.0, 32.0, 32.0);
    // The button shows the appearance it would flip *to*, the way a theme
    // switch does: a sun on the dark strip, a moon on the light one.
    let glyph = if view.preview_light { theme::glyph::MOON } else { theme::glyph::SUN };
    b.push(Element {
        id: Id::PreviewFlip,
        rect: flip,
        kind: Kind::IconButton { glyph, enabled: true },
        interactive: true,
        focusable: true,
    });
    b.advance(height);
}

/// The strip itself: what is on it, in what order, and how much room it
/// takes.
///
/// The per-item options that used to sit in a column beside this list have
/// their own pages now; a row here opens its page, so the list is the map
/// and each page is the territory. What stays is what is genuinely about
/// the strip as a whole rather than about one item on it.
fn strip_pane(b: &mut Builder, view: &View, measure: Measure) {
    b.subtitle("Strip");
    b.caption(&preview::summary(view.model, view.snapshot));
    b.advance(16.0);
    preview_block(b, view);
    b.advance(24.0);

    let list_w = 340.0;
    let gap = 24.0;
    let side_x = b.x() + list_w + gap;
    let side_w = b.width() - list_w - gap;
    let top = b.y();

    let mut list = Builder::new(b.x(), top, list_w, measure);
    list.column_header("Items");
    composer_list(&mut list, view);
    let list_bottom = list.y();
    let list_elements = std::mem::take(&mut list.elements);

    let mut side = Builder::new(side_x, top, side_w, measure);
    side.column_header("Order");
    side.caption(
        "Drag an item to move it. The switch takes it off the strip without \
         forgetting how it was set up.",
    );
    let side_bottom = side.y();
    let side_elements = std::mem::take(&mut side.elements);

    for element in list_elements.into_iter().chain(side_elements) {
        b.push(element);
    }
    let bottom = list_bottom.max(side_bottom);
    b.advance(bottom - b.y());
}

/// One module's page: everything that belongs to that module and nothing
/// else.
fn module_page(b: &mut Builder, view: &View, id: ModuleId) {
    module_inspector(b, view, id);
    // The two modules that own something beyond their column: where the
    // temperatures come from, and where the weather is read for. Both had a
    // pane of their own before every module had one.
    match id {
        ModuleId::Sensors => sensors_sections(b, view),
        ModuleId::Weather => weather_sections(b, view),
        _ => {}
    }
}

/// The stacks, and the one being edited.
fn stacks_page(b: &mut Builder, view: &View, measure: Measure) {
    b.subtitle("Stacks");
    b.caption(
        "A stack shows several readings in one column, and can hide the modules it shows.",
    );
    b.advance(16.0);

    let list_w = 256.0;
    let gap = 24.0;
    let editor_x = b.x() + list_w + gap;
    let editor_w = b.width() - list_w - gap;
    let top = b.y();

    let mut list = Builder::new(b.x(), top, list_w, measure);
    list.column_header("Stacks");
    let mut any = false;
    for item in list_order(view) {
        let StripItem::Stack(id) = item else { continue };
        let Some(stack) = view.model.stack(id) else { continue };
        any = true;
        list.list_row(
            item,
            STACK_COLOR,
            Mark::Stack,
            &model::stack_title(stack),
            &model::stack_caption(stack),
            stack.is_enabled,
            view.selection == Some(item),
            view.drag.is_some_and(|(dragged, _)| dragged == item),
        );
    }
    if !any {
        list.caption("None yet.");
    }
    list.advance(12.0);
    list.button(Id::AddStack, "New stack", false);
    let list_bottom = list.y();
    let list_elements = std::mem::take(&mut list.elements);

    let mut editor = Builder::new(editor_x, top, editor_w, measure);
    match view.selection {
        Some(StripItem::Stack(id)) if view.model.stack(id).is_some() => {
            stack_inspector(&mut editor, view, id)
        }
        _ => {
            editor.column_header("Nothing selected");
            editor.caption("Choose a stack on the left, or make one.");
        }
    }
    let editor_bottom = editor.y();
    let editor_elements = std::mem::take(&mut editor.elements);

    for element in list_elements.into_iter().chain(editor_elements) {
        b.push(element);
    }
    let bottom = list_bottom.max(editor_bottom);
    b.advance(bottom - b.y());
}

/// A module's mark, drawn rather than set from a font. See `icon`.
fn module_mark(id: ModuleId) -> Mark {
    Mark::Module(id)
}

/// The order the list shows: the real one, or the one a drag would produce.
fn list_order(view: &View) -> Vec<StripItem> {
    let mut order = view.model.order.clone();
    if let Some((item, target)) = view.drag {
        if let Some(from) = order.iter().position(|i| *i == item) {
            model::move_in(&mut order, from, target);
        }
    }
    order
}

fn composer_list(b: &mut Builder, view: &View) {
    for item in list_order(view) {
        let lifted = view.drag.is_some_and(|(dragged, _)| dragged == item);
        let selected = view.selection == Some(item);
        match item {
            StripItem::Module(id) => {
                let fact = view.snapshot.fact(id, view.model);
                let caption = model::module_caption(view.model, id, fact);
                b.list_row(
                    item,
                    theme::module_color(id),
                    module_mark(id),
                    model::module_name(id),
                    &caption,
                    view.model.is_module_enabled(id),
                    selected,
                    lifted,
                );
            }
            StripItem::Stack(id) => {
                let Some(stack) = view.model.stack(id) else { continue };
                b.list_row(
                    item,
                    STACK_COLOR,
                    Mark::Stack,
                    &model::stack_title(stack),
                    &model::stack_caption(stack),
                    stack.is_enabled,
                    selected,
                    lifted,
                );
            }
        }
    }
    b.advance(12.0);
    b.button(Id::AddStack, "New stack", false);
    b.advance(8.0);
    b.caption("A stack shows several readings in one column, and can hide the modules it shows.");
}

/// What the switch on a module's own column means right now.
fn show_caption(view: &View, id: ModuleId) -> String {
    let name = model::module_name(id);
    if let Some(stack) = view.model.hidden_by(id) {
        return format!(
            "Off the strip while {} is on: that stack hides the modules it shows. \
             Turn that off in the stack to show both.",
            model::stack_title(stack)
        );
    }
    match view.model.stacks_using(id).first() {
        Some(stack) => format!(
            "Its own column, separate from the {} stack. The stack shows the reading either way.",
            model::stack_title(stack)
        ),
        None => format!("{name} as a column of its own on the strip."),
    }
}

fn module_inspector(b: &mut Builder, view: &View, id: ModuleId) {
    b.header(theme::module_color(id), module_mark(id), model::module_name(id), model::module_blurb(id));

    b.section("On the strip");
    b.card_begin();
    b.row_toggle(
        Id::Show,
        "Show its own column",
        Some(&show_caption(view, id)),
        view.model.is_module_enabled(id),
        true,
    );
    if id == ModuleId::Disks {
        let (disks, disk) = dropdown_items(view, Id::DiskDevice);
        b.row_dropdown(
            Id::DiskDevice,
            "Rates shown for",
            Some("Every disk added together, or one of them on its own. A PC usually has several, and the one that is busy is not always the one Windows is on."),
            &disks[disk.min(disks.len() - 1)],
            260.0,
            disks.len() > 1,
        );
        let (volumes, chosen) = dropdown_items(view, Id::DiskVolume);
        b.row_dropdown(
            Id::DiskVolume,
            "Space shown for",
            Some("Which drive the free and used readings are about. The rates above are every disk together either way."),
            &volumes[chosen.min(volumes.len() - 1)],
            260.0,
            volumes.len() > 1,
        );
    }
    if id == ModuleId::Network {
        b.row_toggle(
            Id::UploadFirst,
            "Upload on top",
            Some("The upload rate on the top line and the download rate under it. Off is the usual order, download over upload."),
            view.model.settings.network_upload_first,
            true,
        );
        let (rates, rate) = dropdown_items(view, Id::RateUnit);
        b.row_dropdown(
            Id::RateUnit,
            "Rate in",
            Some("Bytes if you are watching a download finish, bits if you are checking it against what you pay for. Neither converts into the other in your head."),
            &rates[rate.min(rates.len() - 1)],
            220.0,
            true,
        );
        let (items, selected) = dropdown_items(view, Id::NetworkInterface);
        b.row_dropdown(
            Id::NetworkInterface,
            "Interface",
            Some("Every interface added up, or one of them on its own. Adding them up leaves out VPN tunnels, whose bytes also cross the adapter underneath them and would otherwise be counted twice; following one adapter reads zero while the machine is on another."),
            &items[selected.min(items.len() - 1)],
            240.0,
            true,
        );
        b.row_toggle(
            Id::ShowPublicIp,
            "Public address",
            Some("Shows the address the internet sees this machine from, looked up through ipify.org while the panel is open. Off means nothing is asked of anyone."),
            view.model.settings.network_shows_public_ip,
            true,
        );
    }
    if id == ModuleId::Gpu {
        let (items, selected) = dropdown_items(view, Id::GpuAdapter);
        b.row_dropdown(
            Id::GpuAdapter,
            "Adapter",
            Some("Automatic follows the busiest adapter, which is what Task Manager reports."),
            &items[selected.min(items.len() - 1)],
            240.0,
            true,
        );
    }
    // Sensors is configured here, with every other module, and not on the pane
    // named after it. That pane is about the *source* - which library, where it
    // is, install and remove - and somebody who wants to change which
    // temperature is on the strip has no reason to look there for it. Two
    // places called Sensors, one of which could not switch the module on, was
    // the confusion.
    if id == ModuleId::Sensors {
        let (items, selected) = dropdown_items(view, Id::PinnedSensor);
        if items.len() > 1 {
            b.row_dropdown(
                Id::PinnedSensor,
                "Reading shown",
                Some("Automatic is the hottest processor temperature."),
                &items[selected.min(items.len() - 1)],
                280.0,
                true,
            );
        } else {
            b.row_info(
                "Reading shown",
                Some("Automatic: the hottest processor temperature. Other sensors can be chosen once a source is running."),
            );
        }
        let (units, unit) = dropdown_items(view, Id::SensorUnit);
        b.row_dropdown(
            Id::SensorUnit,
            "Temperature unit",
            Some("Separate from the weather's. Wanting the forecast in Fahrenheit and a processor in Celsius is an ordinary pair."),
            &units[unit.min(units.len() - 1)],
            160.0,
            true,
        );
        let (places, place) = dropdown_items(view, Id::SensorDecimals);
        b.row_dropdown(
            Id::SensorDecimals,
            "Decimals",
            Some("How finely a temperature is written in the panel. A die temperature that moves half a degree between reads sits still at none."),
            &places[place.min(places.len() - 1)],
            180.0,
            true,
        );
        let poll = view.model.settings.sensors.poll_seconds;
        b.row_slider(
            Id::PollSeconds,
            "Read every",
            Some("A read walks every device the library knows and takes a moment; slower is cheaper."),
            poll as f32,
            2.0,
            30.0,
            1.0,
            &format!("{poll} s"),
        );
    }
    b.card_end();

    b.section("In a stack");
    b.card_begin();
    for stack in view.model.stacks_using(id) {
        let labels: Vec<String> = stack
            .metrics
            .iter()
            .filter(|e| e.metric.module() == id)
            .map(|e| e.metric.display_name_in(&view.snapshot.sensors))
            .collect();
        let caption = if stack.hides_source_items && stack.is_enabled {
            format!("Shows {} and hides this module's own column", labels.join(", "))
        } else {
            format!("Shows {}", labels.join(", "))
        };
        b.row_with_link(Id::EditStack(stack.id), &model::stack_title(stack), Some(&caption), "Edit");
    }
    let (items, _) = dropdown_items(view, Id::AddToStack);
    let hint = if view.model.stacks.stacks.is_empty() {
        format!(
            "Put {} beside other readings in one column - CPU and GPU together, say. \
             The module keeps its own switch; the stack decides whether its own column stays.",
            model::module_name(id)
        )
    } else {
        format!("Add {}'s reading to a stack, or start a new one with it.", model::module_name(id))
    };
    b.row_dropdown(Id::AddToStack, "Put in a stack", Some(&hint), "Choose", 240.0, !items.is_empty());
    b.card_end();

    // Deliberately nothing module-specific below the stack row. The sensor
    // source and the weather's locations, units and refresh each have a pane
    // of their own in the sidebar: both are about the machine or the world
    // rather than about this module's column, and buried here they were
    // reachable only by selecting a module the user may have switched off.
    if id == ModuleId::Weather {
        b.advance(8.0);
        b.row_info("Locations, units and how often it refreshes are in the Weather pane.", None);
    }
}

/// Everything about the weather: where it is read for, and how it is shown.
#[allow(dead_code)]
fn weather_pane(b: &mut Builder, view: &View) {
    b.subtitle("Weather");
    b.caption("Where the forecast is for, the units it is shown in, and how often it is fetched.");
    b.advance(16.0);
    weather_sections(b, view);
}

/// Everything about where temperatures come from.
#[allow(dead_code)]
fn sensors_pane(b: &mut Builder, view: &View) {
    b.subtitle("Sensors Configuration");
    b.caption("Where temperatures, fans and voltages are read from.");
    b.advance(16.0);
    sensors_sections(b, view);
}

/// Bytes or bits, with an example of each rather than the bare word.
///
/// `network_unit` has been in the settings file, the module, the flyout and
/// the strip's preview since the module was written, and nothing anywhere
/// could change it - the one setting that decides whether the network column
/// agrees with Task Manager or with the number on the bill.
const RATE_UNITS: [(RateUnit, &str); 2] =
        [(RateUnit::Bytes, "Bytes (1.21 MB/s)"), (RateUnit::Bits, "Bits (9.68 Mb/s)")];

/// How finely a temperature can be written, as the Mac offers it.
///
/// Named rather than numbered in the list, because "1" beside "Decimals"
/// reads as a count of something rather than as how the number will look.
const DECIMALS: [(u32, &str); 3] =
    [(0, "None (48\u{00B0}C)"), (1, "One (48.3\u{00B0}C)"), (2, "Two (48.25\u{00B0}C)")];

fn sensors_sections(b: &mut Builder, view: &View) {
    b.section("Source");
    b.card_begin();
    match view.lhm {
        LhmState::Installing(progress) => {
            let (fraction, words) = match progress {
                lhm_install::Progress::Downloading { received, total } => (
                    if *total > 0 { *received as f32 / *total as f32 } else { 0.0 },
                    "Downloading LibreHardwareMonitor",
                ),
                lhm_install::Progress::Verifying => (0.9, "Checking the download against its digest"),
                lhm_install::Progress::Extracting => (0.95, "Unpacking"),
            };
            b.row_status(Ink::Tertiary, words, Some("From its GitHub releases, into your own AppData."));
            let row = b.row(32.0);
            b.push(Element {
                id: Id::None,
                rect: Rect::new(row.x + 16.0, row.y + 6.0, row.w - 32.0, 20.0),
                kind: Kind::Progress { fraction },
                interactive: false,
                focusable: false,
            });
        }
        LhmState::Removing => {
            b.row_status(
                Ink::Tertiary,
                "Removing LibreHardwareMonitor",
                Some("Closing the sensor helper first, so nothing is holding the files."),
            );
        }
        LhmState::Installed(path) => {
            b.row_status(
                Ink::Success,
                &format!("LibreHardwareMonitor {} is installed", lhm_install::TAG),
                Some(&format!("In {path}. Readings appear right away.")),
            );
        }
        LhmState::Failed(why) => {
            b.row_status(Ink::Critical, "LibreHardwareMonitor could not be installed", Some(why));
            b.row_buttons(
                None,
                &[ButtonSpec { id: Id::InstallLhm, text: "Try again", accent: false, external: false, enabled: true }],
            );
        }
        LhmState::Idle => match &view.snapshot.sensor_source {
            SensorSource::Providing { name, readings } => {
                b.row_status(
                    Ink::Success,
                    &format!("{name} is providing {readings} readings"),
                    Some(&format!(
                        "Read through the sensor helper every {} s.",
                        view.model.settings.sensors.poll_seconds
                    )),
                );
            }
            SensorSource::NotInstalled => {
                b.row_status(Ink::Tertiary, "No sensor source yet", None);
                b.row_paragraph(
                    "Windows gives no temperatures, fans or voltages to apps, so Barometer reads \
                     them through LibreHardwareMonitor. It is not included: Barometer can download \
                     it for you - about 9 MB from its GitHub releases, into your own AppData - and \
                     only ever reads from it.",
                );
            }
            SensorSource::NotRunning => {
                b.row_status(
                    Ink::Tertiary,
                    "LibreHardwareMonitor is installed but the sensor helper did not answer",
                    Some("Readings appear when it does. Reinstalling replaces the library and starts it again."),
                );
            }
            SensorSource::Failed(why) => {
                b.row_status(Ink::Tertiary, "The sensor helper reported a problem", Some(why));
            }
            SensorSource::Unknown => {
                b.row_status(Ink::Tertiary, "Waiting for the sensor helper", None);
            }
        },
    }

    // The buttons are outside the match on purpose.
    //
    // They used to live inside two of its arms, which meant that on a machine
    // reporting anything else - waiting for the helper, or a helper that had
    // reported a problem - there was no way to install LibreHardwareMonitor at
    // all. That is the one thing somebody opens this pane to do, and it was
    // reachable only from the states that happened to be anticipated. A button
    // whose availability depends on a state machine is a button that is missing
    // exactly when it is needed.
    let installed = lhm_install::is_installed();
    let mut buttons = vec![ButtonSpec {
        id: Id::InstallLhm,
        text: if installed {
            "Reinstall LibreHardwareMonitor"
        } else {
            "Download LibreHardwareMonitor"
        },
        accent: !installed,
        external: false,
        enabled: !matches!(view.lhm, LhmState::Installing(_) | LhmState::Removing),
    }];
    if installed {
        // Removing it has to be offered here, because nothing else will: it is
        // unpacked into AppData and has no entry in Add or Remove Programs, so
        // this pane is the only place it can be got rid of.
        buttons.push(ButtonSpec {
            id: Id::RemoveLhm,
            text: "Remove",
            accent: false,
            external: false,
            enabled: !matches!(view.lhm, LhmState::Installing(_) | LhmState::Removing),
        });
    }
    b.row_buttons(
        Some(
            "About 9 MB from LibreHardwareMonitor's own GitHub releases, into your AppData. Free software under the MPL 2.0, checked against its published digest, and only ever read from - Barometer ships none of it.",
        ),
        &buttons,
    );
    match view.pawnio {
        pawnio::Status::Ready => {
            b.row_status(
                Ink::Success,
                "PawnIO is installed",
                Some("Processor, motherboard and memory sensors are available."),
            );
        }
        pawnio::Status::NeedsElevation => {
            b.row_status(
                Ink::Caution,
                "PawnIO is installed but closed to this process",
                Some("Run Barometer as administrator for the processor, motherboard and memory sensors. Graphics and drive temperatures arrive either way."),
            );
        }
        pawnio::Status::NotInstalled => {
            b.row_status(
                Ink::Tertiary,
                "PawnIO is not installed",
                Some("Optional. Graphics and drive temperatures arrive without it; the processor's temperatures and the motherboard's fans, voltages and board temperatures need this signed driver, which you install yourself."),
            );
            b.row_buttons(
                None,
                &[ButtonSpec { id: Id::GetPawnIo, text: "Get PawnIO", accent: false, external: true, enabled: true }],
            );
        }
        pawnio::Status::Unknown => {}
    }
    b.card_end();

    // Which reading is shown, in what unit and how often, all live in the
    // Strip pane's Sensors inspector now - with every other module's settings,
    // which is where somebody looks for them. What is left here is the source.

    b.section("Library");
    b.card_begin();
    // Not "leave it empty": Barometer writes the path here itself when it
    // fetches a copy, so the field is only ever typed into by somebody
    // pointing at an installation of their own.
    b.row_field(
        Id::LibraryDir,
        Some("Location"),
        Some(
            "The folder holding LibreHardwareMonitorLib.dll. Barometer fills this in when it downloads a copy for you; set it yourself only to use an installation of your own, such as a portable copy or one from LibreHardwareMonitor's installer.",
        ),
        view.model.settings.library_directory.as_deref().unwrap_or(""),
        "Not set",
        false,
        view.editing == Some(Id::LibraryDir),
    );
    b.card_end();
}

fn weather_sections(b: &mut Builder, view: &View) {
    let weather = &view.model.settings.weather;
    b.section("Locations");
    b.card_begin();
    let primary = weather.primary_location().map(|l| l.id.clone());
    for (index, location) in weather.locations.iter().enumerate() {
        let region = region_of(location);
        b.row_location(index, &location.name, &region, primary.as_deref() == Some(location.id.as_str()));
    }
    if weather.locations.is_empty() && !weather.uses_current_location {
        b.row_info("Add a city below to start Weather.", None);
    }
    let located = match (&view.snapshot.located, &view.snapshot.weather_error) {
        _ if !weather.uses_current_location => "Guesses where you are from your internet address.".to_string(),
        (Some(place), _) => format!("Near {}", place.display_name()),
        (None, Some(why)) => format!("Could not locate you: {why}"),
        (None, None) => "Finding your location\u{2026}".to_string(),
    };
    b.row_toggle(
        Id::CurrentLocation,
        "Use current location",
        Some(&located),
        weather.uses_current_location,
        true,
    );
    b.card_end();

    b.section("Add a location");
    b.card_begin();
    let status = if view.search.pending {
        Some("Searching\u{2026}".to_string())
    } else if let Some(why) = &view.search.error {
        Some(format!("Couldn't reach Open-Meteo: {why}"))
    } else if view.search.answered && view.search.results.is_empty() && view.search.query.trim().len() >= 2 {
        Some("No matching places".to_string())
    } else {
        None
    };
    b.row_field(
        Id::Search,
        None,
        status.as_deref(),
        &view.search.query,
        "Search cities",
        true,
        view.editing == Some(Id::Search),
    );
    for (index, place) in view.search.results.iter().take(8).enumerate() {
        b.row_result(index, &place.name, &region_of(place));
    }
    b.card_end();

    b.section("Units");
    b.card_begin();
    for (id, label) in [
        (Id::TempUnit, "Temperature"),
        (Id::WindUnit, "Wind"),
        (Id::PressureUnit, "Pressure"),
        (Id::PrecipUnit, "Precipitation"),
    ] {
        let (items, selected) = dropdown_items(view, id);
        b.row_dropdown(id, label, None, &items[selected.min(items.len() - 1)], DROPDOWN_W, true);
    }
    b.card_end();

    b.section("Forecast");
    b.card_begin();
    let minutes = weather.clamped_refresh_minutes();
    b.row_slider(
        Id::RefreshMinutes,
        "Refresh every",
        None,
        minutes as f32,
        *REFRESH_INTERVAL_MINUTES.start() as f32,
        *REFRESH_INTERVAL_MINUTES.end() as f32,
        5.0,
        &format!("{minutes} min"),
    );
    b.card_end();
    b.advance(16.0);
    b.link(Id::Link(OPEN_METEO_URL), "Weather data by Open-Meteo.com");
}

/// "Texas, United States" under a place name.
fn region_of(location: &Location) -> String {
    match &location.admin {
        Some(admin) if !admin.is_empty() && !location.country.is_empty() => {
            format!("{admin}, {}", location.country)
        }
        Some(admin) if !admin.is_empty() => admin.clone(),
        _ => location.country.clone(),
    }
}

fn stack_inspector(b: &mut Builder, view: &View, id: u32) {
    let Some(stack) = view.model.stack(id) else {
        b.caption("This stack no longer exists.");
        return;
    };
    b.header(
        STACK_COLOR,
        Mark::Stack,
        &model::stack_title(stack),
        "Several readings in one column. Each reading comes from its module, on or off.",
    );

    b.section("On the strip");
    b.card_begin();
    b.row_toggle(Id::Show, "Show this stack", None, stack.is_enabled, true);
    b.row_field(
        Id::StackName,
        Some("Name"),
        None,
        &stack.name,
        "Name this stack",
        false,
        view.editing == Some(Id::StackName),
    );
    let (layouts, selected) = dropdown_items(view, Id::Layout);
    b.row_dropdown(
        Id::Layout,
        "Layout",
        Some("Two rows pairs the readings, which is what makes a stack cheaper than separate columns."),
        &layouts[selected],
        240.0,
        true,
    );
    b.card_end();

    b.section("Readings");
    b.card_begin();
    let count = stack.metrics.len();
    for (index, entry) in stack.metrics.iter().enumerate() {
        let module = entry.metric.module();
        let name = entry.metric.display_name_in(&view.snapshot.sensors);
        // The usual label as the placeholder, so an empty field visibly
        // means "the usual" and the user sees what they would be replacing.
        let placeholder = entry.default_caption(&view.snapshot.sensors);
        let label = super::ui::MetricLabel {
            text: entry.label.as_deref().unwrap_or(""),
            placeholder: &placeholder,
            active: view.editing == Some(Id::MetricLabel(index)),
        };
        b.row_metric(
            index,
            theme::module_color(module),
            module_mark(module),
            &name,
            model::module_name(module),
            Some(label),
            index == 0,
            index + 1 == count,
        );
    }
    let (items, _) = dropdown_items(view, Id::AddMetric);
    b.row_dropdown(
        Id::AddMetric,
        "Add a reading",
        if count == 0 { Some("The preview above shows the column as readings are added.") } else { None },
        "Choose a reading",
        240.0,
        !items.is_empty(),
    );
    b.card_end();

    b.section("Modules");
    b.card_begin();
    let sources = model::source_names(stack);
    let caption = if sources.is_empty() {
        "Once the stack has readings, their modules can stop drawing their own columns.".to_string()
    } else {
        format!(
            "{sources} keep their own switches, but stop drawing their own columns while this stack is on."
        )
    };
    b.row_toggle(
        Id::HideSources,
        "Hide these modules' own columns",
        Some(&caption),
        stack.hides_source_items,
        !stack.metrics.is_empty(),
    );
    b.card_end();

    b.advance(16.0);
    b.link(Id::RemoveStack, "Remove this stack");
}

fn appearance_pane(b: &mut Builder, view: &View) {
    b.subtitle("Appearance");
    b.caption("Type and spacing for the strip. Every change shows in the preview and on the taskbar.");
    b.advance(16.0);
    preview_block(b, view);

    let font = &view.model.settings.font;
    b.section("Text");
    b.card_begin();
    let (families, selected) = dropdown_items(view, Id::Family);
    b.row_dropdown(
        Id::Family,
        "Font",
        None,
        &families[selected.min(families.len() - 1)],
        280.0,
        true,
    );
    let (heads, head) = dropdown_items(view, Id::HeadingFamily);
    b.row_dropdown(
        Id::HeadingFamily,
        "Heading font",
        Some("A face of its own for the names on the strip. On Windows a weight is often a family - Segoe UI Semibold is its own - so a heading can be a different face rather than only a heavier one."),
        &heads[head.min(heads.len() - 1)],
        280.0,
        true,
    );
    // One row, two weights: the strip is two kinds of text, a heading naming
    // the column and the number under it, and they are chosen side by side
    // so the relationship is on the screen rather than in a caption.
    let (head_weights, heading) = dropdown_items(view, Id::HeadingWeight);
    let (value_weights, value) = dropdown_items(view, Id::Weight);
    b.row_dropdown_pair(
        "Weight",
        Some("Headings are the names on the strip, CPU and MEM; values are the numbers under them. Only the weights each family has faces for are offered."),
        [
            DropdownSpec {
                id: Id::HeadingWeight,
                name: "Headings",
                value: &head_weights[heading.min(head_weights.len() - 1)],
            },
            DropdownSpec {
                id: Id::Weight,
                name: "Values",
                value: &value_weights[value.min(value_weights.len() - 1)],
            },
        ],
    );
    let size = font.max_size_dip;
    b.row_slider(
        Id::Size,
        "Text size",
        Some(&preview::size_caption(font, view.snapshot, view.model.shown_count())),
        size,
        barometer_core::settings::StripFont::MIN_SIZE_DIP,
        barometer_core::settings::StripFont::MAX_SIZE_DIP,
        1.0,
        &format!("{} pt", size.round()),
    );
    b.card_end();

    b.section("Spacing");
    b.card_begin();
    let gap = view.model.settings.column_gap_dip;
    b.row_slider(
        Id::Gap,
        "Between columns",
        Some("Wide enough that two numbers read as two, and no wider: room here comes out of the task buttons."),
        gap,
        0.0,
        24.0,
        1.0,
        &format!("{} px", gap.round()),
    );
    let padding = view.model.settings.padding_dip;
    b.row_slider(
        Id::Padding,
        "At the ends",
        Some("Inside the space the tray gives the strip, which already leaves a little slack."),
        padding,
        0.0,
        24.0,
        1.0,
        &format!("{} px", padding.round()),
    );
    b.card_end();
}

fn general_pane(b: &mut Builder, view: &View) {
    b.subtitle("General");
    b.caption("When Barometer starts, and how it keeps itself current.");
    b.advance(16.0);

    b.section("Startup");
    b.card_begin();
    b.row_toggle(
        Id::StartAtLogin,
        "Start Barometer when I sign in",
        Some("As a logon task, because Barometer runs with administrator rights and Windows will not start those from the ordinary startup list."),
        view.starts_at_login,
        true,
    );
    b.card_end();

    b.section("Updates");
    b.card_begin();
    let status = if view.update.checking {
        "Checking\u{2026}".to_string()
    } else {
        view.update.result.clone().unwrap_or_else(|| "From Barometer's GitHub releases.".to_string())
    };
    b.row_button(
        &format!("Barometer {}", view.version),
        Some(&status),
        ButtonSpec {
            id: Id::CheckNow,
            text: "Check now",
            accent: false,
            external: false,
            enabled: !view.update.checking,
        },
    );
    b.row_toggle(
        Id::CheckUpdates,
        "Check for updates automatically",
        Some("Once a week. Nothing is installed without you."),
        view.model.settings.check_for_updates,
        true,
    );
    if let Some(skipped) = &view.model.settings.skipped_update {
        b.row_with_link(
            Id::ClearSkipped,
            &format!("Skipping {skipped}"),
            Some("The automatic check stays quiet about this release."),
            "Stop skipping",
        );
    }
    b.card_end();
}

fn about_pane(b: &mut Builder, view: &View) {
    b.masthead(
        "Barometer",
        &format!("Version {} \u{00B7} Free software under the GNU GPL 3.0", view.version),
        "A system monitor for the Windows taskbar, ported from Barometer for macOS by the same author.",
    );
    b.advance(24.0);

    b.card_begin();
    b.row_buttons(
        None,
        &[
            ButtonSpec { id: Id::Link(REPOSITORY_URL), text: "Source code", accent: false, external: true, enabled: true },
            ButtonSpec { id: Id::Link(GPL_URL), text: "License", accent: false, external: true, enabled: true },
        ],
    );
    b.card_end();

    b.section("Credits");
    b.card_begin();
    b.row_with_link(
        Id::Link(LHM_URL),
        "LibreHardwareMonitor",
        Some("Temperatures, fans and voltages. MPL 2.0. Not included; downloaded on request."),
        "Project page",
    );
    b.row_with_link(
        Id::Link(PAWNIO_SOURCE_URL),
        "PawnIO",
        Some("The signed kernel driver LibreHardwareMonitor reads processor registers through. GPL 2.0, with LGPL 2.1 modules. Installed by you, never by Barometer."),
        "Project page",
    );
    b.row_with_link(
        Id::Link(OPEN_METEO_URL),
        "Open-Meteo",
        Some("Weather data. CC BY 4.0."),
        "open-meteo.com",
    );
    b.card_end();
}

/// The choices a dropdown offers, and which is current.
///
/// A selected index past the end means nothing is current, which is how the
/// "add" dropdowns - readings to add, stacks to join - present themselves.
pub fn dropdown_items(view: &View, id: Id) -> (Vec<String>, usize) {
    let model = view.model;
    match id {
        Id::Family => {
            // A settings file may name an instance - "Segoe UI Variable Small
            // Semibol" from before the picker offered families - and that
            // still has to find its family in the list.
            // The saved name as it stands: the list carries the real
            // families now, instance names among them, so there is nothing
            // to fold it into.
            let current = model.settings.font.family.clone();
            let mut items: Vec<String> = view.families.to_vec();
            let selected = match items.iter().position(|f| *f == current) {
                Some(index) => index,
                None => {
                    // A face chosen on another machine, or removed since.
                    items.insert(0, format!("{current} (not installed)"));
                    0
                }
            };
            (items, selected)
        }
        Id::HeadingFamily => {
            // The first entry is not a family: it is the absence of one.
            let mut items = vec![SAME_FAMILY.to_string()];
            let mut selected = 0;
            for family in view.families {
                if model.settings.font.heading_family.as_deref() == Some(family.as_str()) {
                    selected = items.len();
                }
                items.push(family.clone());
            }
            (items, selected)
        }
        Id::Weight | Id::HeadingWeight => {
            // Only the weights this family has faces for. GDI never refuses a
            // weight - asked for one a family cannot draw it smears the
            // nearest face - so offering all five was offering four lies on a
            // single-weight family.
            let family = weight_family(model, id);
            let offered = weights_offered(&family, view.faces);
            let items = offered.iter().map(|(_, name)| name.to_string()).collect();
            let current = if id == Id::Weight {
                model.settings.font.weight
            } else {
                model.settings.font.heading_weight
            };
            let selected = offered.iter().position(|(w, _)| *w == current).unwrap_or(0);
            (items, selected)
        }
        Id::NetworkInterface => {
            // "All interfaces" is every one of them that is not a tunnel,
            // and the caption under the row says why; a tunnel is named as
            // one, as the Swift's picker names a VPN.
            let mut items = vec!["All interfaces".to_string()];
            let mut selected = 0;
            items.extend(
                view.snapshot
                    .interfaces
                    .iter()
                    .map(|(name, tunnel)| if *tunnel { format!("{name} (VPN)") } else { name.clone() }),
            );
            if let Some(chosen) = &model.settings.network_interface {
                match view.snapshot.interfaces.iter().position(|(name, _)| name == chosen) {
                    Some(index) => selected = index + 1,
                    // An interface that is not here now - a dock left behind -
                    // is still what was chosen, and saying so is better than
                    // quietly reading as "all".
                    None => {
                        items.push(format!("{chosen} (not found)"));
                        selected = items.len() - 1;
                    }
                }
            }
            (items, selected)
        }
        Id::GpuAdapter => {
            let mut items = vec!["Automatic (busiest adapter)".to_string()];
            let mut selected = 0;
            for adapter in &view.snapshot.gpus {
                items.push(adapter.name.clone());
            }
            if let GpuChoice::Adapter { key, name } = &model.gpu {
                match view.snapshot.gpus.iter().position(|a| &a.key == key) {
                    Some(index) => selected = index + 1,
                    None => {
                        items.push(format!("{name} (not found)"));
                        selected = items.len() - 1;
                    }
                }
            }
            (items, selected)
        }
        Id::SensorUnit => {
            let items =
                vec!["Celsius (\u{00B0}C)".to_string(), "Fahrenheit (\u{00B0}F)".to_string()];
            let selected = match model.settings.sensors.temperature {
                TemperatureUnit::Celsius => 0,
                TemperatureUnit::Fahrenheit => 1,
            };
            (items, selected)
        }
        Id::DiskDevice => {
            let mut items = vec!["Every disk together".to_string()];
            let mut selected = 0;
            for device in &view.snapshot.disks {
                if model.settings.disk_device.as_deref() == Some(device.id.as_str()) {
                    selected = items.len();
                }
                // The counter's name is "1 D: E:"; the model is what is
                // written on the drive. Both, because two identical drives
                // are told apart only by the number.
                items.push(match &device.model {
                    Some(model) => format!("{} \u{00B7} {model}", device.name),
                    None => device.name.clone(),
                });
            }
            (items, selected)
        }
        Id::DiskVolume => {
            let mut items = vec!["The drive Windows is on".to_string()];
            let mut selected = 0;
            for volume in &view.snapshot.volumes {
                if model.settings.disk_volume.as_deref() == Some(volume.mount.as_str()) {
                    selected = items.len();
                }
                items.push(match volume.name.is_empty() {
                    true => volume.mount.clone(),
                    false => format!("{} ({})", volume.mount, volume.name),
                });
            }
            (items, selected)
        }
        Id::RateUnit => {
            let items = RATE_UNITS.iter().map(|(_, label)| label.to_string()).collect();
            let selected = RATE_UNITS
                .iter()
                .position(|(unit, _)| *unit == model.settings.network_unit)
                .unwrap_or(0);
            (items, selected)
        }
        Id::SensorDecimals => {
            let items = DECIMALS.iter().map(|(_, label)| label.to_string()).collect();
            let places = model.settings.sensors.decimal_places;
            let selected = DECIMALS.iter().position(|(n, _)| *n == places).unwrap_or(1);
            (items, selected)
        }
        Id::PinnedSensor => {
            let mut items = vec!["Automatic (hottest processor)".to_string()];
            let mut selected = 0;
            for sensor in temperature_sensors(view.snapshot) {
                if Some(sensor.id.as_str()) == model.settings.sensors.pinned_sensor_id.as_deref() {
                    selected = items.len();
                }
                let value = sensor
                    .value
                    .map(|v| format!(" \u{00B7} {v:.0}\u{00B0}C"))
                    .unwrap_or_default();
                items.push(format!("{}{value} \u{2014} {}", sensor.name, sensor.hardware));
            }
            (items, selected)
        }
        Id::Layout => {
            let items = LAYOUTS.iter().map(|(_, name)| name.to_string()).collect();
            let layout = view
                .selection
                .and_then(|s| match s {
                    StripItem::Stack(id) => model.stack(id).map(|st| st.layout),
                    _ => None,
                })
                .unwrap_or_default();
            (items, LAYOUTS.iter().position(|(l, _)| *l == layout).unwrap_or(0))
        }
        Id::AddMetric => {
            let Some(StripItem::Stack(stack)) = view.selection else { return (Vec::new(), usize::MAX) };
            let items = model
                .metric_candidates(stack, &view.snapshot.sensors)
                .into_iter()
                .map(|(_, name)| name)
                .collect();
            (items, usize::MAX)
        }
        Id::AddToStack => {
            let Some(StripItem::Module(module)) = view.selection else { return (Vec::new(), usize::MAX) };
            let Some(primary) = StackMetric::primary(module) else { return (Vec::new(), usize::MAX) };
            let mut items: Vec<String> = model
                .stacks
                .stacks
                .iter()
                .filter(|s| !s.has(&primary))
                .map(|s| format!("Add to {}", model::stack_title(s)))
                .collect();
            items.push(format!("New stack with {}", model::module_name(module)));
            (items, usize::MAX)
        }
        Id::TempUnit => {
            let items = TEMPERATURES.iter().map(|(_, n)| n.to_string()).collect();
            let current = model.settings.weather.units.temperature;
            (items, TEMPERATURES.iter().position(|(u, _)| *u == current).unwrap_or(0))
        }
        Id::WindUnit => {
            let items = WINDS.iter().map(|(_, n)| n.to_string()).collect();
            let current = model.settings.weather.units.wind_speed;
            (items, WINDS.iter().position(|(u, _)| *u == current).unwrap_or(0))
        }
        Id::PressureUnit => {
            let items = PRESSURES.iter().map(|(_, n)| n.to_string()).collect();
            let current = model.settings.weather.units.pressure;
            (items, PRESSURES.iter().position(|(u, _)| *u == current).unwrap_or(0))
        }
        Id::PrecipUnit => {
            let items = PRECIPITATIONS.iter().map(|(_, n)| n.to_string()).collect();
            let current = model.settings.weather.units.precipitation;
            (items, PRECIPITATIONS.iter().position(|(u, _)| *u == current).unwrap_or(0))
        }
        _ => (Vec::new(), usize::MAX),
    }
}

/// What the heading-font control calls "no family of its own".
pub const SAME_FAMILY: &str = "Same as the values";

/// Which family a weight control belongs to.
fn weight_family(model: &Model, id: Id) -> String {
    let font = &model.settings.font;
    match id {
        Id::HeadingWeight => font.heading_family.clone().unwrap_or_else(|| font.family.clone()),
        _ => font.family.clone(),
    }
}

/// The weights `family` has faces for, named.
///
/// GDI's weights are numbers and the picker offers names, so each face is
/// put in the nearest named bucket and the buckets nobody landed in are not
/// offered. A family whose faces are all between two names collapses onto
/// whichever is closer, which is the honest answer: that is the face the
/// user would get.
fn weights_offered(family: &str, faces: &[(String, i32)]) -> Vec<(FontWeight, &'static str)> {
    let have = system::weights_for(family, faces);
    let mut offered: Vec<(FontWeight, &'static str)> = Vec::new();
    for weight in have {
        let nearest = WEIGHTS
            .iter()
            .min_by_key(|(w, _)| (w.dwrite_weight() as i32 - weight).abs())
            .copied();
        if let Some(entry) = nearest {
            if !offered.iter().any(|(w, _)| *w == entry.0) {
                offered.push(entry);
            }
        }
    }
    offered.sort_by_key(|(w, _)| w.dwrite_weight());
    if offered.is_empty() {
        offered.push((FontWeight::Regular, "Regular"));
    }
    offered
}

const WEIGHTS: [(FontWeight, &str); 5] = [
    (FontWeight::Light, "Light"),
    (FontWeight::Regular, "Regular"),
    (FontWeight::Medium, "Medium"),
    (FontWeight::Semibold, "Semibold"),
    (FontWeight::Bold, "Bold"),
];

const LAYOUTS: [(StackLayout, &str); 2] = [
    (StackLayout::Columns, "Two rows, readings paired"),
    (StackLayout::SingleRow, "One row"),
];

const TEMPERATURES: [(TemperatureUnit, &str); 2] = [
    (TemperatureUnit::Celsius, "Celsius (\u{00B0}C)"),
    (TemperatureUnit::Fahrenheit, "Fahrenheit (\u{00B0}F)"),
];

// Wind and pressure go by their symbols. The units card sits in the
// inspector, where a 160 dropdown is what keeps a unit on one line beside
// its label, and "Kilometers per hour (km/h)" does not fit one; the symbol
// is also what the flyout will print beside the number.
const WINDS: [(WindSpeedUnit, &str); 4] = [
    (WindSpeedUnit::KilometersPerHour, "km/h"),
    (WindSpeedUnit::MilesPerHour, "mph"),
    (WindSpeedUnit::MetersPerSecond, "m/s"),
    (WindSpeedUnit::Knots, "Knots"),
];

const PRESSURES: [(PressureUnit, &str); 3] = [
    (PressureUnit::Hectopascals, "hPa"),
    (PressureUnit::InchesOfMercury, "inHg"),
    (PressureUnit::MillimetersOfMercury, "mmHg"),
];

const PRECIPITATIONS: [(PrecipitationUnit, &str); 2] = [
    (PrecipitationUnit::Millimeters, "Millimeters (mm)"),
    (PrecipitationUnit::Inches, "Inches (in)"),
];

/// The sensors the pin picker offers: temperatures, in the source's order.
fn temperature_sensors(snapshot: &Snapshot) -> Vec<&barometer_core::sensors::Sensor> {
    snapshot
        .sensors
        .iter()
        .filter(|s| s.kind == barometer_core::sensors::SensorKind::Temperature)
        .collect()
}

/// What choosing a dropdown item does. The selection may need to move: a
/// stack made from a module's inspector opens ready to be filled.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Select(StripItem),
}

/// Applies the `index`th choice of dropdown `id` to the model.
pub fn choose(model: &mut Model, view: &ChoiceContext, id: Id, index: usize) -> Effect {
    match id {
        Id::Family => {
            // The "(not installed)" entry is the current family itself.
            let installed = view.families.get(index.saturating_sub(usize::from(view.family_missing)));
            if let Some(family) = installed {
                if !(view.family_missing && index == 0) {
                    model.settings.font.family = family.clone();
                }
            }
        }
        Id::HeadingFamily => {
            model.settings.font.heading_family = match index {
                0 => None,
                n => view.families.get(n - 1).cloned(),
            };
        }
        Id::Weight => {
            let family = weight_family(model, Id::Weight);
            if let Some((weight, _)) = weights_offered(&family, &view.faces).get(index) {
                model.settings.font.weight = *weight;
            }
        }
        Id::HeadingWeight => {
            let family = weight_family(model, Id::HeadingWeight);
            if let Some((weight, _)) = weights_offered(&family, &view.faces).get(index) {
                model.settings.font.heading_weight = *weight;
            }
        }
        Id::NetworkInterface => {
            model.settings.network_interface = match index {
                0 => None,
                // The "(not found)" entry: keep what was saved.
                n => view
                    .interfaces
                    .get(n - 1)
                    .map(|(name, _)| name.clone())
                    .or_else(|| model.settings.network_interface.clone()),
            };
        }
        Id::GpuAdapter => {
            model.gpu = match index {
                0 => GpuChoice::Automatic,
                n => match view.gpus.get(n - 1) {
                    Some(adapter) => GpuChoice::Adapter { key: adapter.key.clone(), name: adapter.name.clone() },
                    // The "(not found)" entry: keep what was saved.
                    None => model.gpu.clone(),
                },
            };
        }
        Id::PinnedSensor => {
            model.settings.sensors.pinned_sensor_id = match index {
                0 => None,
                n => view.sensor_ids.get(n - 1).cloned(),
            };
        }
        Id::SensorUnit => {
            model.settings.sensors.temperature = match index {
                1 => TemperatureUnit::Fahrenheit,
                _ => TemperatureUnit::Celsius,
            };
        }
        Id::DiskDevice => {
            model.settings.disk_device = match index {
                0 => None,
                n => view.disks.get(n - 1).cloned(),
            };
        }
        Id::DiskVolume => {
            model.settings.disk_volume = match index {
                0 => None,
                n => view.volumes.get(n - 1).cloned(),
            };
        }
        Id::RateUnit => {
            if let Some((unit, _)) = RATE_UNITS.get(index) {
                model.settings.network_unit = *unit;
            }
        }
        Id::SensorDecimals => {
            if let Some((places, _)) = DECIMALS.get(index) {
                model.settings.sensors.decimal_places = *places;
            }
        }
        Id::Layout => {
            if let (Some(StripItem::Stack(stack)), Some((layout, _))) = (view.selection, LAYOUTS.get(index)) {
                model.set_stack_layout(stack, *layout);
            }
        }
        Id::AddMetric => {
            if let Some(StripItem::Stack(stack)) = view.selection {
                let candidates = model.metric_candidates(stack, &view.sensors);
                if let Some((metric, _)) = candidates.get(index) {
                    model.add_metric(stack, metric.clone());
                }
            }
        }
        Id::AddToStack => {
            if let Some(StripItem::Module(module)) = view.selection {
                let Some(primary) = StackMetric::primary(module) else { return Effect::None };
                let candidates: Vec<u32> = model
                    .stacks
                    .stacks
                    .iter()
                    .filter(|s| !s.has(&primary))
                    .map(|s| s.id)
                    .collect();
                return match candidates.get(index) {
                    Some(stack) => {
                        model.add_metric(*stack, primary);
                        Effect::Select(StripItem::Stack(*stack))
                    }
                    None => {
                        let stack = model.add_stack(Some(primary));
                        Effect::Select(StripItem::Stack(stack))
                    }
                };
            }
        }
        Id::TempUnit => {
            if let Some((unit, _)) = TEMPERATURES.get(index) {
                model.settings.weather.units.temperature = *unit;
            }
        }
        Id::WindUnit => {
            if let Some((unit, _)) = WINDS.get(index) {
                model.settings.weather.units.wind_speed = *unit;
            }
        }
        Id::PressureUnit => {
            if let Some((unit, _)) = PRESSURES.get(index) {
                model.settings.weather.units.pressure = *unit;
            }
        }
        Id::PrecipUnit => {
            if let Some((unit, _)) = PRECIPITATIONS.get(index) {
                model.settings.weather.units.precipitation = *unit;
            }
        }
        _ => {}
    }
    Effect::None
}

/// What `choose` needs to know about the lists it is choosing from, captured
/// when the dropdown opened so the index still means what the person saw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChoiceContext {
    pub selection: Option<StripItem>,
    pub families: Vec<String>,
    pub family_missing: bool,
    pub gpus: Vec<model::GpuAdapter>,
    pub interfaces: Vec<(String, bool)>,
    pub sensor_ids: Vec<String>,
    /// Every (family, weight) the machine has, for the weight controls.
    pub faces: Vec<(String, i32)>,
    /// The mounts of the volumes on offer, in the order they are listed.
    pub volumes: Vec<String>,
    /// The counter instances of the disks on offer, in the same order.
    pub disks: Vec<String>,
    /// Every sensor, for the stack picker's sensor entries.
    pub sensors: Vec<barometer_core::sensors::Sensor>,
}

impl ChoiceContext {
    pub fn capture(view: &View) -> ChoiceContext {
        let current = view.model.settings.font.family.clone();
        ChoiceContext {
            selection: view.selection,
            families: view.families.to_vec(),
            faces: view.faces.to_vec(),
            family_missing: !view.families.iter().any(|f| *f == current),
            gpus: view.snapshot.gpus.clone(),
            interfaces: view.snapshot.interfaces.clone(),
            sensor_ids: temperature_sensors(view.snapshot).into_iter().map(|s| s.id.clone()).collect(),
            volumes: view.snapshot.volumes.iter().map(|v| v.mount.clone()).collect(),
            disks: view.snapshot.disks.iter().map(|d| d.id.clone()).collect(),
            sensors: view.snapshot.sensors.clone(),
        }
    }
}

/// Applies a slider's new value.
pub fn slide(model: &mut Model, id: Id, value: f32) {
    match id {
        Id::Size => model.settings.font.max_size_dip = clamp_size(value.round()),
        Id::Gap => model.settings.column_gap_dip = clamp_gap(value.round()),
        Id::Padding => model.settings.padding_dip = clamp_padding(value.round()),
        Id::PollSeconds => model.settings.sensors.poll_seconds = clamp_poll(value.round() as u32),
        Id::RefreshMinutes => {
            model.settings.weather.refresh_interval_minutes = (value.round() as u32)
                .clamp(*REFRESH_INTERVAL_MINUTES.start(), *REFRESH_INTERVAL_MINUTES.end())
        }
        _ => {}
    }
}

/// Flips a switch.
/// The strip item an open page is about.
///
/// A module page is about its module, whatever the Strip list was last left
/// on: every control on that page is drawn from the pane, so reading the
/// subject from anywhere else put one module's name over another module's
/// switch. Both ways of reaching a module page without touching the list -
/// the sidebar, and a panel's gear - did exactly that, and "Show its own
/// column" turned off a module the user had not named.
pub fn subject(pane: Pane, selection: Option<StripItem>) -> Option<StripItem> {
    match pane {
        Pane::Module(id) => Some(StripItem::Module(id)),
        _ => selection,
    }
}

pub fn toggle(model: &mut Model, selection: Option<StripItem>, id: Id) {
    match id {
        Id::RowToggle(item) => {
            let on = model.is_item_enabled(item);
            model.set_item_enabled(item, !on);
        }
        Id::Show => {
            if let Some(item) = selection {
                let on = model.is_item_enabled(item);
                model.set_item_enabled(item, !on);
            }
        }
        Id::HideSources => {
            if let Some(StripItem::Stack(stack)) = selection {
                let hides = model.stack(stack).is_some_and(|s| s.hides_source_items);
                model.set_stack_hides_sources(stack, !hides);
            }
        }
        Id::CurrentLocation => {
            model.settings.weather.uses_current_location = !model.settings.weather.uses_current_location;
        }
        Id::UploadFirst => model.settings.network_upload_first = !model.settings.network_upload_first,
        Id::ShowPublicIp => model.settings.network_shows_public_ip = !model.settings.network_shows_public_ip,
        Id::CheckUpdates => model.settings.check_for_updates = !model.settings.check_for_updates,
        _ => {}
    }
}

/// Adds a searched-for place to the saved locations. The first one saved
/// becomes the primary; a place already saved is left alone.
pub fn add_location(model: &mut Model, place: &Location) -> bool {
    let weather = &mut model.settings.weather;
    if weather.locations.iter().any(|l| l.id == place.id) {
        return false;
    }
    weather.locations.push(place.clone());
    if weather.primary_location_id.is_none() {
        weather.primary_location_id = Some(place.id.clone());
    }
    true
}

pub fn remove_location(model: &mut Model, index: usize) {
    let weather = &mut model.settings.weather;
    if index >= weather.locations.len() {
        return;
    }
    let removed = weather.locations.remove(index);
    if weather.primary_location_id.as_deref() == Some(removed.id.as_str()) {
        weather.primary_location_id = weather.locations.first().map(|l| l.id.clone());
    }
}

pub fn set_primary_location(model: &mut Model, index: usize) {
    let weather = &mut model.settings.weather;
    if let Some(place) = weather.locations.get(index) {
        weather.primary_location_id = Some(place.id.clone());
    }
}

/// The rows of an open dropdown, in window coordinates, over everything.
pub fn dropdown_overlay(popup: Rect, items: &[String], selected: usize, scroll: f32) -> Vec<Element> {
    let mut elements = vec![Element {
        id: Id::None,
        rect: popup,
        kind: Kind::Popup,
        interactive: true,
        focusable: false,
    }];
    let mut y = popup.y + 4.0 - scroll;
    for (index, item) in items.iter().enumerate() {
        let rect = Rect::new(popup.x + 4.0, y, popup.w - 8.0, POPUP_ITEM_H);
        // Whole rows only. The popup is not clipped when it is drawn, and a
        // row half outside it would hang off the card; the scroll arithmetic
        // keeps the row being moved to inside, so nothing reachable is lost.
        if rect.y >= popup.y && rect.bottom() <= popup.bottom() {
            elements.push(Element {
                id: Id::DropdownItem(index),
                rect,
                kind: Kind::ListItem { text: item.clone(), selected: index == selected },
                interactive: true,
                focusable: false,
            });
        }
        y += POPUP_ITEM_H;
    }
    elements
}

/// The rows of the composer, in list order, with their rects, for the drop
/// target arithmetic.
pub fn composer_rows(elements: &[Element]) -> Vec<(StripItem, Rect)> {
    elements
        .iter()
        .filter_map(|e| match (e.id, &e.kind) {
            (Id::Row(item), Kind::Row { .. }) => Some((item, e.rect)),
            _ => None,
        })
        .collect()
}

/// Which slot a dragged row would drop into for a pointer at `y`.
pub fn drop_index(rows: &[(StripItem, Rect)], y: f32) -> usize {
    if rows.is_empty() {
        return 0;
    }
    rows.iter()
        .position(|(_, rect)| y < rect.center_y())
        .unwrap_or(rows.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_ui::gdi::TextStyle;
    use barometer_core::store::Settings;

    fn measure(text: &str, style: TextStyle, width: f32) -> (f32, f32) {
        let w = text.chars().count() as f32 * 7.0;
        if width > 0.0 && w > width {
            (width, (w / width).ceil() * style.line_dip())
        } else {
            (w, style.line_dip())
        }
    }

    struct Fixture {
        model: Model,
        snapshot: Snapshot,
        families: Vec<String>,
        /// The unfolded list, as GDI enumerates it: the picker's families
        /// plus the instance families that carry the real weights.
        instances: Vec<String>,
        lhm: LhmState,
        faces: Vec<(String, i32)>,
        search: SearchView,
        update: UpdateView,
    }

    impl Fixture {
        fn new() -> Fixture {
            let mut settings = Settings::default();
            for entry in settings.modules.iter_mut() {
                entry.enabled = true;
            }
            Fixture {
                model: Model::from_settings(settings),
                snapshot: Snapshot::default(),
                families: vec!["Segoe UI".into(), "Segoe UI Variable Text".into()],
                instances: vec![
                    "Segoe UI".into(),
                    "Segoe UI Light".into(),
                    "Segoe UI Semibold".into(),
                    "Segoe UI Variable Text".into(),
                    "Segoe UI Variable Text Light".into(),
                    // Truncated at 31 characters, the way GDI enumerates it.
                    "Segoe UI Variable Text Semibold".into(),
                ],
                lhm: LhmState::Idle,
                faces: vec![
                    // As GDI enumerates them: a family reports the weights it
                    // has faces for, and Segoe UI's Light and Semibold are
                    // families of their own rather than weights of this one.
                    ("Segoe UI".to_string(), 400),
                    ("Segoe UI".to_string(), 700),
                    ("Segoe UI Semibold".to_string(), 600),
                    ("Segoe UI Variable Text".to_string(), 300),
                    ("Segoe UI Variable Text".to_string(), 400),
                    ("Segoe UI Variable Text".to_string(), 600),
                ],
                search: SearchView::default(),
                update: UpdateView::default(),
            }
        }

        fn view(&self, selection: Option<StripItem>) -> View<'_> {
            View {
                model: &self.model,
                snapshot: &self.snapshot,
                selection,
                families: &self.families,
                faces: &self.faces,
                instances: &self.instances,
                pawnio: pawnio::Status::Unknown,
                lhm: &self.lhm,
                search: &self.search,
                update: &self.update,
                editing: None,
                drag: None,
                preview_light: false,
                version: "0.1.0",
                starts_at_login: false,
            }
        }
    }

    fn texts(elements: &[Element]) -> Vec<String> {
        elements
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Text { text, .. } => Some(text.clone()),
                Kind::Dropdown { text, .. } => Some(text.clone()),
                Kind::Button { text, .. } => Some(text.clone()),
                Kind::Link { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_nav_pins_about_to_the_bottom() {
        let items = nav(640.0, Pane::Strip, &measure);
        // Counted from the enum rather than written down, so adding a pane is
        // not also a test edit. One element beyond the panes: the heading
        // that groups the modules.
        let rows: Vec<&Element> = items.iter().filter(|e| matches!(e.kind, Kind::NavItem { .. })).collect();
        assert_eq!(rows.len(), Pane::ALL.len());
        assert_eq!(items.len(), Pane::ALL.len() + 1);
        let heading = items.iter().find(|e| matches!(e.kind, Kind::NavHeading { .. })).expect("heading");
        let first_module = items.iter().find(|e| e.id == Id::Nav(Pane::MODULES[0])).expect("first module");
        assert!(heading.rect.bottom() <= first_module.rect.y);
        // Every module's row carries its own mark, which is what makes the
        // sidebar read as the modules rather than as a list of words.
        for pane in Pane::MODULES {
            let row = items.iter().find(|e| e.id == Id::Nav(pane)).expect("module row");
            assert!(matches!(&row.kind, Kind::NavItem { mark: Some(_), .. }), "{pane:?}");
        }
        let about = items.iter().find(|e| e.id == Id::Nav(Pane::About)).unwrap();
        assert_eq!(about.rect.bottom(), 640.0 - 8.0);
        let strip = items.iter().find(|e| e.id == Id::Nav(Pane::Strip)).unwrap();
        assert_eq!(strip.rect.y, 8.0);
        assert!(matches!(strip.kind, Kind::NavItem { selected: true, .. }));
    }

    #[test]
    fn every_pane_builds_and_ends_below_its_content() {
        let f = Fixture::new();
        for pane in Pane::ALL {
            let (elements, height) = content(pane, &f.view(Some(StripItem::Module(ModuleId::Cpu))), 704.0, &measure);
            assert!(!elements.is_empty(), "{pane:?}");
            let lowest = elements.iter().map(|e| e.rect.bottom()).fold(0.0, f32::max);
            assert!(height >= lowest, "{pane:?}: {height} < {lowest}");
        }
    }

    #[test]
    fn the_composer_lists_every_item_once_with_the_selected_row_marked() {
        let mut f = Fixture::new();
        let stack = f.model.add_stack(Some(StackMetric::CpuTotal));
        let (elements, _) = content(Pane::Strip, &f.view(Some(StripItem::Stack(stack))), 704.0, &measure);
        let rows = composer_rows(&elements);
        assert_eq!(rows.len(), ModuleId::ALL.len() + 1);
        assert_eq!(rows.last().unwrap().0, StripItem::Stack(stack));
        let selected: Vec<_> = elements
            .iter()
            .filter(|e| matches!(e.kind, Kind::Row { selected: true, .. }))
            .map(|e| e.id)
            .collect();
        assert_eq!(selected, vec![Id::Row(StripItem::Stack(stack))]);
    }

    #[test]
    fn every_setting_the_modules_carry_has_a_control_on_the_page_it_belongs_to() {
        // Each of these was already read by the module, saved to the file and
        // honored on the strip, with nothing anywhere that could change it.
        let f = Fixture::new();
        let view = f.view(None);
        let has = |pane: Pane, id: Id| {
            let (elements, _) = content(pane, &view, 704.0, &measure);
            elements.iter().any(|e| e.id == id)
        };
        // Bytes or bits: the one setting that decides whether the network
        // column agrees with Task Manager or with the number on the bill.
        assert!(has(Pane::Module(ModuleId::Network), Id::RateUnit));
        // Which drive the space readings are about, rather than always the
        // one Windows booted from, and which disk the rates are about rather
        // than always every disk added together. A Mac has one built-in SSD
        // and a PC routinely has several.
        assert!(has(Pane::Module(ModuleId::Disks), Id::DiskVolume));
        assert!(has(Pane::Module(ModuleId::Disks), Id::DiskDevice));
        // How finely a temperature is written, which was pinned at none.
        assert!(has(Pane::Module(ModuleId::Sensors), Id::SensorDecimals));
        // And starting with Windows, which had no control at all.
        assert!(has(Pane::General, Id::StartAtLogin));
    }

    #[test]
    fn the_switch_on_a_module_page_belongs_to_the_module_the_page_is_showing() {
        // Reached by the sidebar or by a panel's gear, nothing selects a row
        // in the Strip list first, so the page opened on one module while the
        // selection still named another - and the switch drawn for the first
        // toggled the second. Whatever the list was left on, the page decides.
        let mut f = Fixture::new();
        let stale = Some(StripItem::Module(ModuleId::Cpu));
        assert_eq!(
            subject(Pane::Module(ModuleId::Memory), stale),
            Some(StripItem::Module(ModuleId::Memory))
        );
        toggle(&mut f.model, subject(Pane::Module(ModuleId::Memory), stale), Id::Show);
        assert!(!f.model.is_module_enabled(ModuleId::Memory));
        assert!(f.model.is_module_enabled(ModuleId::Cpu));
        // Off a module page there is no module in the pane, so the selection
        // is still what a control acts on.
        let stack = f.model.add_stack(None);
        assert_eq!(
            subject(Pane::Stacks, Some(StripItem::Stack(stack))),
            Some(StripItem::Stack(stack))
        );
    }

    #[test]
    fn a_module_hidden_by_a_stack_says_so_in_both_places() {
        let mut f = Fixture::new();
        let stack = f.model.add_stack(Some(StackMetric::GpuUtilization));
        f.model.set_stack_name(stack, "Main");
        f.model.set_stack_hides_sources(stack, true);
        let view = f.view(Some(StripItem::Module(ModuleId::Gpu)));
        // The list says it on the Strip page; the module's own page says why.
        let (listed, _) = content(Pane::Strip, &view, 704.0, &measure);
        assert!(texts(&listed).iter().any(|t| t == "Hidden while Main is on"), "{:?}", texts(&listed));
        let (elements, _) = content(Pane::Module(ModuleId::Gpu), &view, 704.0, &measure);
        let words = texts(&elements);
        assert!(words.iter().any(|t| t.starts_with("Off the strip while Main is on")), "{words:?}");
        assert!(words.iter().any(|t| t == "Main"));
        assert!(elements.iter().any(|e| e.id == Id::EditStack(stack)));
    }

    #[test]
    fn a_stack_inspector_lists_its_readings_with_the_ends_unable_to_move_past_them() {
        let mut f = Fixture::new();
        let stack = f.model.add_stack(Some(StackMetric::CpuTotal));
        f.model.add_metric(stack, StackMetric::GpuUtilization);
        let (elements, _) = content(Pane::Stacks, &f.view(Some(StripItem::Stack(stack))), 704.0, &measure);
        let enabled = |id: Id| elements.iter().find(|e| e.id == id).map(|e| e.interactive);
        assert_eq!(enabled(Id::MetricUp(0)), Some(false));
        assert_eq!(enabled(Id::MetricDown(0)), Some(true));
        assert_eq!(enabled(Id::MetricUp(1)), Some(true));
        assert_eq!(enabled(Id::MetricDown(1)), Some(false));
        assert_eq!(enabled(Id::MetricRemove(1)), Some(true));
        let words = texts(&elements);
        assert!(words.iter().any(|t| t == "CPU and GPU keep their own switches, but stop drawing their own columns while this stack is on."), "{words:?}");
    }

    #[test]
    fn putting_a_module_in_a_new_stack_seeds_and_selects_it() {
        let mut f = Fixture::new();
        let view = f.view(Some(StripItem::Module(ModuleId::Gpu)));
        let (items, _) = dropdown_items(&view, Id::AddToStack);
        assert_eq!(items, vec!["New stack with GPU"]);
        let context = ChoiceContext::capture(&view);
        let effect = choose(&mut f.model, &context, Id::AddToStack, 0);
        let Effect::Select(StripItem::Stack(stack)) = effect else { panic!("{effect:?}") };
        assert_eq!(f.model.stack(stack).unwrap().metrics.iter().map(|e| e.metric.clone()).collect::<Vec<_>>(), vec![StackMetric::GpuUtilization]);

        // A second module joins the existing stack rather than making another.
        let view = f.view(Some(StripItem::Module(ModuleId::Cpu)));
        let (items, _) = dropdown_items(&view, Id::AddToStack);
        assert_eq!(items, vec!["Add to GPU", "New stack with CPU"]);
        let context = ChoiceContext::capture(&view);
        assert_eq!(choose(&mut f.model, &context, Id::AddToStack, 0), Effect::Select(StripItem::Stack(stack)));
        assert_eq!(f.model.stack(stack).unwrap().metrics.iter().map(|e| e.metric.clone()).collect::<Vec<_>>(), vec![StackMetric::GpuUtilization, StackMetric::CpuTotal]);
        // And the stack is no longer offered to a module already in it.
        let view = f.view(Some(StripItem::Module(ModuleId::Cpu)));
        assert_eq!(dropdown_items(&view, Id::AddToStack).0, vec!["New stack with CPU"]);
    }

    #[test]
    fn add_reading_offers_only_what_the_stack_lacks_and_adds_by_index() {
        let mut f = Fixture::new();
        let stack = f.model.add_stack(Some(StackMetric::CpuTotal));
        let view = f.view(Some(StripItem::Stack(stack)));
        let (items, selected) = dropdown_items(&view, Id::AddMetric);
        assert_eq!(items.len(), StackMetric::ALL.len() - 1);
        assert_eq!(selected, usize::MAX);
        assert_eq!(items[0], "CPU \u{00B7} CPU user");
        let context = ChoiceContext::capture(&view);
        choose(&mut f.model, &context, Id::AddMetric, 0);
        assert_eq!(f.model.stack(stack).unwrap().metrics.iter().map(|e| e.metric.clone()).collect::<Vec<_>>(), vec![StackMetric::CpuTotal, StackMetric::CpuUser]);
    }

    #[test]
    fn the_font_picker_names_a_missing_family_and_keeps_it_when_rechosen() {
        let mut f = Fixture::new();
        f.model.settings.font.family = "Nonexistent".into();
        let view = f.view(None);
        let (items, selected) = dropdown_items(&view, Id::Family);
        assert_eq!(items[0], "Nonexistent (not installed)");
        assert_eq!(selected, 0);
        let context = ChoiceContext::capture(&view);
        choose(&mut f.model, &context, Id::Family, 0);
        assert_eq!(f.model.settings.font.family, "Nonexistent");
        choose(&mut f.model, &context, Id::Family, 2);
        assert_eq!(f.model.settings.font.family, "Segoe UI Variable Text");
    }

    #[test]
    fn the_gpu_picker_hands_back_the_adapters_opaque_key() {
        let mut f = Fixture::new();
        f.snapshot.gpus = vec![
            model::GpuAdapter { key: "nv:0".into(), name: "NVIDIA GeForce RTX 4090".into() },
            model::GpuAdapter { key: "intel:0".into(), name: "Intel(R) UHD Graphics 770".into() },
        ];
        let view = f.view(Some(StripItem::Module(ModuleId::Gpu)));
        let (items, selected) = dropdown_items(&view, Id::GpuAdapter);
        assert_eq!(items.len(), 3);
        assert_eq!(selected, 0);
        let context = ChoiceContext::capture(&view);
        choose(&mut f.model, &context, Id::GpuAdapter, 2);
        assert_eq!(
            f.model.gpu,
            GpuChoice::Adapter { key: "intel:0".into(), name: "Intel(R) UHD Graphics 770".into() }
        );
        // An adapter that has since gone is still named, not blanked.
        f.snapshot.gpus.clear();
        let view = f.view(Some(StripItem::Module(ModuleId::Gpu)));
        let (items, selected) = dropdown_items(&view, Id::GpuAdapter);
        assert_eq!(items[selected], "Intel(R) UHD Graphics 770 (not found)");
        let context = ChoiceContext::capture(&view);
        choose(&mut f.model, &context, Id::GpuAdapter, 0);
        assert_eq!(f.model.gpu, GpuChoice::Automatic);
    }

    #[test]
    fn sliders_clamp_and_round_into_their_settings() {
        let mut f = Fixture::new();
        slide(&mut f.model, Id::Size, 11.4);
        assert_eq!(f.model.settings.font.max_size_dip, 11.0);
        slide(&mut f.model, Id::Gap, 90.0);
        assert_eq!(f.model.settings.column_gap_dip, barometer_core::store::MAX_SPACING_DIP);
        slide(&mut f.model, Id::PollSeconds, 0.0);
        assert_eq!(f.model.settings.sensors.poll_seconds, barometer_core::store::MIN_POLL_SECONDS);
        slide(&mut f.model, Id::RefreshMinutes, 999.0);
        assert_eq!(f.model.settings.weather.refresh_interval_minutes, 60);
    }

    #[test]
    fn toggles_flip_the_thing_they_name() {
        let mut f = Fixture::new();
        let stack = f.model.add_stack(Some(StackMetric::CpuTotal));
        toggle(&mut f.model, None, Id::RowToggle(StripItem::Module(ModuleId::Cpu)));
        assert!(!f.model.is_module_enabled(ModuleId::Cpu));
        toggle(&mut f.model, Some(StripItem::Module(ModuleId::Cpu)), Id::Show);
        assert!(f.model.is_module_enabled(ModuleId::Cpu));
        toggle(&mut f.model, Some(StripItem::Stack(stack)), Id::HideSources);
        assert!(f.model.stack(stack).unwrap().hides_source_items);
        toggle(&mut f.model, None, Id::CheckUpdates);
        assert!(!f.model.settings.check_for_updates);
        toggle(&mut f.model, None, Id::UploadFirst);
        assert!(f.model.settings.network_upload_first);
    }

    #[test]
    fn the_interface_picker_leads_with_all_of_them_and_keeps_one_that_has_gone() {
        let mut f = Fixture::new();
        f.snapshot.interfaces = vec![("Ethernet".to_string(), false), ("Wi-Fi".to_string(), true)];
        let context = {
            let view = f.view(Some(StripItem::Module(ModuleId::Network)));
            let (items, selected) = dropdown_items(&view, Id::NetworkInterface);
            assert_eq!(items, ["All interfaces", "Ethernet", "Wi-Fi (VPN)"]);
            assert_eq!(selected, 0);
            ChoiceContext::capture(&view)
        };
        choose(&mut f.model, &context, Id::NetworkInterface, 2);
        assert_eq!(f.model.settings.network_interface.as_deref(), Some("Wi-Fi"));

        // The dock is left behind: the choice is kept and said to be missing.
        f.snapshot.interfaces = vec![("Wi-Fi 2".to_string(), false)];
        let view = f.view(Some(StripItem::Module(ModuleId::Network)));
        let (items, selected) = dropdown_items(&view, Id::NetworkInterface);
        assert_eq!(items, ["All interfaces", "Wi-Fi 2", "Wi-Fi (not found)"]);
        assert_eq!(selected, 2);

        // And back to all of them.
        let context = ChoiceContext::capture(&view);
        choose(&mut f.model, &context, Id::NetworkInterface, 0);
        assert_eq!(f.model.settings.network_interface, None);
    }

    /// The switch the strip already honors has to be reachable from the one
    /// place a person would look for it: the network module's own options.
    #[test]
    fn the_network_inspector_offers_upload_on_top_and_nothing_else_does() {
        let f = Fixture::new();
        let (elements, _) = content(Pane::Module(ModuleId::Network), &f.view(Some(StripItem::Module(ModuleId::Network))), 704.0, &measure);
        let switch = elements.iter().find(|e| e.id == Id::UploadFirst && matches!(e.kind, Kind::Toggle { .. }));
        assert!(matches!(switch.map(|e| &e.kind), Some(Kind::Toggle { on: false, enabled: true })));
        // And the public address, off until asked for.
        let public = elements.iter().find(|e| e.id == Id::ShowPublicIp && matches!(e.kind, Kind::Toggle { .. }));
        assert!(matches!(public.map(|e| &e.kind), Some(Kind::Toggle { on: false, enabled: true })));
        let (elements, _) = content(Pane::Module(ModuleId::Cpu), &f.view(Some(StripItem::Module(ModuleId::Cpu))), 704.0, &measure);
        assert!(!elements.iter().any(|e| e.id == Id::UploadFirst || e.id == Id::ShowPublicIp));
    }

    /// The heading and value weights are two settings offered as one row,
    /// each chosen without touching the other.
    #[test]
    fn the_two_weights_are_offered_together_and_chosen_apart() {
        let mut f = Fixture::new();
        let context = {
            let view = f.view(None);
            let (items, heading) = dropdown_items(&view, Id::HeadingWeight);
            let (_, value) = dropdown_items(&view, Id::Weight);
            // Only the weights this family has faces for. Windows knows
            // which those are - it hands the weight of every face to the
            // enumeration - and offering the rest invited GDI to smear the
            // nearest face into a weight nobody drew.
            assert_eq!(items[heading], "Semibold");
            assert_eq!(items[value], "Regular");
            let names: Vec<&str> = items.iter().map(String::as_str).collect();
            assert_eq!(names, vec!["Light", "Regular", "Semibold"]);
            assert!(!names.contains(&"Medium"), "{names:?}");
            assert!(!names.contains(&"Bold"), "{names:?}");
            let (elements, _) = content(Pane::Appearance, &view, 704.0, &measure);
            let headings = elements.iter().find(|e| e.id == Id::HeadingWeight).expect("headings").rect;
            let values = elements.iter().find(|e| e.id == Id::Weight).expect("values").rect;
            assert_eq!(headings.y, values.y);
            ChoiceContext::capture(&view)
        };
        // By weight rather than by position, and against the weights this
        // family actually offers rather than the five names that exist:
        // hardcoded indices chose the neighbour the moment the list changed,
        // and the list is now as long as the family has faces.
        let offered = weights_offered("Segoe UI Variable Text", &context.faces);
        let at = |wanted| offered.iter().position(|(w, _)| *w == wanted).expect("offered");
        choose(&mut f.model, &context, Id::HeadingWeight, at(FontWeight::Light));
        assert_eq!(f.model.settings.font.heading_weight, FontWeight::Light);
        assert_eq!(f.model.settings.font.weight, FontWeight::Regular);
        choose(&mut f.model, &context, Id::Weight, at(FontWeight::Semibold));
        assert_eq!(f.model.settings.font.weight, FontWeight::Semibold);
        assert_eq!(f.model.settings.font.heading_weight, FontWeight::Light);
    }

    /// About leads with the mark and the version, which is what the pane is
    /// for; the license is named there too, since it is the one fact about
    /// the program that is not a setting.
    #[test]
    fn the_about_pane_leads_with_the_mark_the_version_and_the_license() {
        let f = Fixture::new();
        let (elements, _) = content(Pane::About, &f.view(None), 704.0, &measure);
        let icon = elements.iter().find(|e| matches!(e.kind, Kind::AppIcon)).expect("the icon").rect;
        assert!(icon.w >= 48.0 && icon.w == icon.h);
        let words = texts(&elements);
        assert!(words.iter().any(|t| t == "Barometer"));
        assert!(words.iter().any(|t| t.starts_with("Version 0.1.0") && t.contains("GPL")), "{words:?}");
        // Everything else starts under the masthead.
        let lowest_card = elements.iter().filter(|e| matches!(e.kind, Kind::Card)).map(|e| e.rect.y).fold(f32::MAX, f32::min);
        assert!(lowest_card >= icon.bottom());
    }

    /// The flip button names the appearance it flips to, not the one shown.
    #[test]
    fn the_preview_flip_shows_the_other_appearance() {
        let f = Fixture::new();
        let mut view = f.view(None);
        let glyph_of = |elements: &[Element]| {
            elements.iter().find_map(|e| match (e.id, &e.kind) {
                (Id::PreviewFlip, Kind::IconButton { glyph, .. }) => Some(*glyph),
                _ => None,
            })
        };
        let (dark, _) = content(Pane::Appearance, &view, 704.0, &measure);
        assert_eq!(glyph_of(&dark), Some(theme::glyph::SUN));
        view.preview_light = true;
        let (light, _) = content(Pane::Appearance, &view, 704.0, &measure);
        assert_eq!(glyph_of(&light), Some(theme::glyph::MOON));
    }

    fn austin(id: &str) -> Location {
        Location {
            id: id.into(),
            name: "Austin".into(),
            admin: Some("Texas".into()),
            country: "United States".into(),
            latitude: "30.2672".into(),
            longitude: "-97.7431".into(),
            time_zone: "America/Chicago".into(),
        }
    }

    #[test]
    fn the_first_saved_location_becomes_primary_and_a_repeat_is_refused() {
        let mut f = Fixture::new();
        assert!(add_location(&mut f.model, &austin("a")));
        assert!(!add_location(&mut f.model, &austin("a")));
        assert!(add_location(&mut f.model, &austin("b")));
        assert_eq!(f.model.settings.weather.primary_location_id.as_deref(), Some("a"));
        set_primary_location(&mut f.model, 1);
        assert_eq!(f.model.settings.weather.primary_location_id.as_deref(), Some("b"));
        remove_location(&mut f.model, 1);
        // Removing the primary falls back to the first left, never to nothing.
        assert_eq!(f.model.settings.weather.primary_location_id.as_deref(), Some("a"));
        assert_eq!(region_of(&austin("a")), "Texas, United States");
    }

    #[test]
    fn the_drop_index_is_the_slot_the_pointer_is_over() {
        let rows = vec![
            (StripItem::Module(ModuleId::Cpu), Rect::new(0.0, 0.0, 100.0, 52.0)),
            (StripItem::Module(ModuleId::Gpu), Rect::new(0.0, 52.0, 100.0, 52.0)),
            (StripItem::Module(ModuleId::Memory), Rect::new(0.0, 104.0, 100.0, 52.0)),
        ];
        assert_eq!(drop_index(&rows, 10.0), 0);
        assert_eq!(drop_index(&rows, 60.0), 1);
        assert_eq!(drop_index(&rows, 500.0), 2);
        assert_eq!(drop_index(&[], 10.0), 0);
    }

    #[test]
    fn a_drag_previews_the_order_it_would_produce_with_the_row_lifted() {
        let f = Fixture::new();
        let mut view = f.view(None);
        view.drag = Some((StripItem::Module(ModuleId::Weather), 0));
        let (elements, _) = content(Pane::Strip, &view, 704.0, &measure);
        let rows = composer_rows(&elements);
        assert_eq!(rows[0].0, StripItem::Module(ModuleId::Weather));
        let lifted = elements
            .iter()
            .find(|e| matches!(e.kind, Kind::Row { lifted: true, .. }))
            .map(|e| e.id);
        assert_eq!(lifted, Some(Id::Row(StripItem::Module(ModuleId::Weather))));
    }

    #[test]
    fn the_dropdown_overlay_shows_only_the_rows_inside_the_popup() {
        let popup = Rect::new(0.0, 0.0, 200.0, 4.0 + 2.0 * POPUP_ITEM_H + 4.0);
        let items: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let overlay = dropdown_overlay(popup, &items, 1, 0.0);
        let rows = overlay.iter().filter(|e| matches!(e.kind, Kind::ListItem { .. })).count();
        assert_eq!(rows, 2);
        let scrolled = dropdown_overlay(popup, &items, 1, 8.0 * POPUP_ITEM_H);
        assert!(scrolled.iter().any(|e| e.id == Id::DropdownItem(9)));
        assert!(!scrolled.iter().any(|e| e.id == Id::DropdownItem(0)));
    }

    /// The download button has to be there in *every* state, which is the bug
    /// this guards: it used to live inside two arms of the source match, so on
    /// a machine reporting anything else there was no way to install the
    /// library at all - and that is the one thing this pane exists for.
    #[test]
    fn the_download_button_is_offered_whatever_the_source_is_doing() {
        let mut f = Fixture::new();
        for source in [
            SensorSource::NotInstalled,
            SensorSource::NotRunning,
            SensorSource::Unknown,
            SensorSource::Failed("something".into()),
        ] {
            f.snapshot.sensor_source = source.clone();
            let (elements, _) = content(Pane::Module(ModuleId::Sensors), &f.view(None), 704.0, &measure);
            assert!(
                elements.iter().any(|e| e.id == Id::InstallLhm),
                "no install button for {source:?}"
            );
        }
    }

    #[test]
    fn the_sensors_pane_stays_calm_about_a_fresh_install() {
        let mut f = Fixture::new();
        f.snapshot.sensor_source = SensorSource::NotInstalled;
        let (elements, _) = content(Pane::Module(ModuleId::Sensors), &f.view(None), 704.0, &measure);
        let words = texts(&elements);
        assert!(words.iter().any(|t| t == "No sensor source yet"));
        assert!(words
            .iter()
            .any(|t| t.starts_with("Download LibreHardwareMonitor")
                || t.starts_with("Reinstall LibreHardwareMonitor")));
        // Nothing here is colored as an error: the only dots are neutral.
        let critical = elements.iter().any(|e| matches!(e.kind, Kind::Dot { ink: Ink::Critical }));
        assert!(!critical);
    }

    #[test]
    fn the_install_seam_reports_progress_and_the_result() {
        let mut f = Fixture::new();
        f.lhm = LhmState::Installing(lhm_install::Progress::Downloading { received: 50, total: 100 });
        let (elements, _) = content(Pane::Module(ModuleId::Sensors), &f.view(None), 704.0, &measure);
        let bar = elements.iter().find_map(|e| match e.kind {
            Kind::Progress { fraction } => Some(fraction),
            _ => None,
        });
        assert_eq!(bar, Some(0.5));
        f.lhm = LhmState::Failed("no network".into());
        let (elements, _) = content(Pane::Module(ModuleId::Sensors), &f.view(None), 704.0, &measure);
        assert!(elements.iter().any(|e| e.id == Id::InstallLhm));
    }
}
