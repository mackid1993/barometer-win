// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The stack flyout, ported from CombinedDropdownView.swift.
//
// A stack draws readings from several modules, and someone opening it
// wants the same detail they would get from those modules individually,
// not a summary of it. So this content owns no picture of its own: it
// holds one content per module - the very same types the module panels are
// made of - and shows the one the chosen reading belongs to, complete,
// under a row of chips that switches between the readings. The Swift hosts
// each module's whole dropdown view inside its tabs for the same reason,
// and it is what keeps a stack's panel from ever being a second copy of a
// module's, drifting from it a little each release.
//
// One divergence, on purpose. The Swift's picker offers the stack's source
// *modules*; this one offers its *readings*, by the captions the strip
// shows, because that is what the user sees in the column they clicked and
// a reading is the thing they can name. Two readings from one module open
// the same panel, which costs nothing. The other is that the Swift scrolls
// its picker sideways once there are too many for the width; a hidden
// sideways scroll inside a panel that scrolls up and down is the kind of
// thing nobody finds, so the chips wrap to a second line instead, as the
// CPU panel's share chips already do.

use std::sync::{Arc, Mutex};

use barometer_core::sensors::Sensor;
use barometer_core::stack::{StackEntry, StripItem};
use barometer_core::ModuleId;

use super::cpu::{chip_left, latest, CpuContent, CpuSnapshot, NEUTRAL};
use super::disks::{DisksContent, DisksSnapshot};
use super::gpu::{GpuContent, GpuSnapshot};
use super::memory::{MemoryContent, MemorySnapshot};
use super::network::{NetworkContent, NetworkSnapshot};
use super::sensors::{SensorsContent, SensorsSnapshot};
use super::ui::{Accent, Builder, Element, Id, CARD_GAP, PANEL_PAD};
use super::weather::{self, WeatherContent};
use super::{Action, Content, Context, Page, Response, Wake};
use crate::settings_ui::model::STACK_GLYPH;
use crate::settings_ui::theme::STACK_COLOR;

/// The switcher's chips answer to this plus the reading's index. Far above
/// anything a module content numbers its own controls with, so a press on
/// a chip can never be mistaken for a press on a row of the panel below.
const SWITCH_ID: u32 = 1 << 24;

/// A chip's height, which chip_at fixes, and the Swift's 2 between the
/// chips of a CapsulePicker.
const CHIP_H: f32 = 20.0;
const SWITCH_GAP: f32 = 2.0;
/// Between two lines of chips, when they wrap.
const LINE_GAP: f32 = 6.0;

/// The stack mark as the icon font spells it, for the empty state's glyph.
/// STACK_GLYPH is a char because the tiles want one; `unavailable` wants a
/// string. A test holds the two together.
const STACK_MARK: &str = "\u{E8A9}";

fn switch_id(index: usize) -> Id {
    Id::Custom(SWITCH_ID + index as u32)
}

fn switch_index(id: Id) -> Option<usize> {
    match id {
        Id::Custom(n) if n >= SWITCH_ID => Some((n - SWITCH_ID) as usize),
        _ => None,
    }
}

/// What the strip knows about a stack, published on every tick.
///
/// The readings and the sensors together, because a sensor reading's
/// caption is the sensor's name and only the source knows it. The module
/// readings themselves do not travel this way: each module's content reads
/// its own feed, the one the module's own panel reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackSnapshot {
    /// The stack's instance number, `StackSettings::id`. Informational: the
    /// content is keyed by the id it was built with, and the strip publishes
    /// to the feed that belongs to that stack.
    pub id: u32,
    /// `StackSettings::display_name()`.
    pub name: String,
    /// `StackSettings::metrics`, in the strip's order.
    pub entries: Vec<StackEntry>,
    /// What the sensors module reports, for captioning a sensor reading.
    pub sensors: Vec<Sensor>,
}

impl StackSnapshot {
    /// The name to show: the fallback is `StackSettings::display_name`'s,
    /// so a stack is called the same thing here and in the settings.
    fn title(&self) -> &str {
        if self.name.is_empty() {
            "Untitled"
        } else {
            &self.name
        }
    }
}

/// The module panels' ends of their feeds, as `Flyout::feed(..).slot()`
/// hands them out. The same slots the module contents were built from, so
/// the stack's copy of a module panel reads exactly what the module's does.
#[derive(Clone)]
pub struct ModuleSlots {
    pub cpu: Arc<Mutex<CpuSnapshot>>,
    pub gpu: Arc<Mutex<GpuSnapshot>>,
    pub memory: Arc<Mutex<MemorySnapshot>>,
    pub disks: Arc<Mutex<DisksSnapshot>>,
    pub network: Arc<Mutex<NetworkSnapshot>>,
    pub sensors: Arc<Mutex<SensorsSnapshot>>,
    pub weather: Arc<Mutex<weather::Snapshot>>,
}

/// A stack's content: a switcher over its readings, and the module panels
/// beneath it.
pub struct StackContent {
    id: u32,
    feed: Arc<Mutex<StackSnapshot>>,
    snapshot: StackSnapshot,
    /// One content per module, every one of them, since a reading of any
    /// module can be added to the stack while the panel is open.
    panels: Vec<Box<dyn Content>>,
    /// The reading the user chose, or none for the first. Remembered as
    /// the entry rather than its index so that a reading added in front of
    /// it does not silently change what the panel shows.
    selected: Option<StackEntry>,
    /// Whether the panel is open on this content.
    open: bool,
    /// The module content that has been told it is showing, and has not
    /// yet been told it stopped, so that switching readings gives each
    /// module content the same opened and closed a module panel gets.
    showing: Option<ModuleId>,
}

impl StackContent {
    /// Content for the stack with `id`, reading the stack's own feed and
    /// the module panels' feeds.
    pub fn new(id: u32, feed: Arc<Mutex<StackSnapshot>>, slots: ModuleSlots) -> StackContent {
        let panels: Vec<Box<dyn Content>> = vec![
            Box::new(CpuContent::new(slots.cpu)),
            Box::new(GpuContent::new(slots.gpu)),
            Box::new(MemoryContent::new(slots.memory)),
            Box::new(DisksContent::new(slots.disks)),
            Box::new(NetworkContent::new(slots.network)),
            Box::new(SensorsContent::new(slots.sensors)),
            Box::new(WeatherContent::new(slots.weather)),
        ];
        StackContent { id, feed, snapshot: StackSnapshot::default(), panels, selected: None, open: false, showing: None }
    }

    /// Takes the feed's latest; whether it differs from what is shown.
    fn sync(&mut self) -> bool {
        match latest(&self.feed) {
            Some(snapshot) if snapshot != self.snapshot => {
                self.snapshot = snapshot;
                true
            }
            _ => false,
        }
    }

    /// Which reading is shown: the chosen one while it is still in the
    /// stack, else the first, as the Swift falls back to the first member.
    fn selected_index(&self) -> usize {
        self.selected
            .as_ref()
            .and_then(|chosen| self.snapshot.entries.iter().position(|entry| entry == chosen))
            .unwrap_or(0)
    }

    fn selected_module(&self) -> Option<ModuleId> {
        self.snapshot.entries.get(self.selected_index()).map(|entry| entry.metric.module())
    }

    fn panel(&mut self, module: ModuleId) -> &mut dyn Content {
        self.panels
            .iter_mut()
            .find(|panel| panel.module() == module)
            .map(|panel| panel.as_mut())
            .expect("a content for every module")
    }

    fn selected_panel(&mut self) -> Option<&mut dyn Content> {
        let module = self.selected_module()?;
        Some(self.panel(module))
    }

    /// Makes `module`'s content the one showing, telling the one before
    /// that it closed and this one that it opened - which is what a module
    /// panel would have been told, and what the weather's fetch waits for.
    fn show(&mut self, module: Option<ModuleId>) {
        if !self.open || self.showing == module {
            return;
        }
        if let Some(previous) = self.showing.take() {
            self.panel(previous).closed();
        }
        if let Some(next) = module {
            self.panel(next).opened();
        }
        self.showing = module;
    }
}

impl Content for StackContent {
    /// The chosen reading's module, so the gear and the accent follow it.
    /// With no readings there is nothing to follow; the processor's is as
    /// good as any, and its settings pane is the Strip, where stacks are
    /// edited.
    fn module(&self) -> ModuleId {
        self.selected_module().unwrap_or(ModuleId::Cpu)
    }

    fn item(&self) -> StripItem {
        StripItem::Stack(self.id)
    }

    fn accent(&self) -> Accent {
        match self.selected_module() {
            Some(module) => Accent::signature(module),
            None => Accent { primary: STACK_COLOR, secondary: STACK_COLOR },
        }
    }

    fn attach(&mut self, wake: Wake) {
        for panel in self.panels.iter_mut() {
            panel.attach(wake.clone());
        }
    }

    /// Opens on the first reading. The Swift's selection is view state
    /// that starts over with each opening, and a panel that reopened on a
    /// reading chosen an hour ago would look like it had forgotten which
    /// column was clicked.
    fn opened(&mut self) {
        self.sync();
        self.selected = None;
        self.open = true;
        self.show(self.selected_module());
    }

    fn closed(&mut self) {
        self.show(None);
        self.open = false;
    }

    fn tick(&mut self) -> bool {
        let changed = self.sync();
        if changed {
            // A reading removed in the settings may have taken the shown
            // module with it.
            self.show(self.selected_module());
        }
        let inner = self.selected_panel().is_some_and(|panel| panel.tick());
        changed || inner
    }

    fn actions(&self) -> Vec<Action> {
        let Some(module) = self.selected_module() else { return Vec::new() };
        self.panels.iter().find(|panel| panel.module() == module).map(|panel| panel.actions()).unwrap_or_default()
    }

    fn activate(&mut self, id: Id) -> Response {
        if let Some(index) = switch_index(id) {
            let Some(entry) = self.snapshot.entries.get(index).cloned() else { return Response::None };
            self.selected = Some(entry);
            self.show(self.selected_module());
            return Response::Relayout;
        }
        self.selected_panel().map(|panel| panel.activate(id)).unwrap_or(Response::None)
    }

    /// The pointer on a chip is, to the panel below, the pointer on none of
    /// its controls; the weather would otherwise read the chip's id as an
    /// hour of the day.
    fn hover(&mut self, id: Option<Id>) -> bool {
        let id = id.filter(|id| switch_index(*id).is_none());
        self.selected_panel().is_some_and(|panel| panel.hover(id))
    }

    fn key(&mut self, key: u16) -> Response {
        self.selected_panel().map(|panel| panel.key(key)).unwrap_or(Response::None)
    }

    fn build(&mut self, cx: &Context) -> Page {
        let mut b = cx.builder();
        if self.snapshot.entries.is_empty() {
            empty(&mut b, &self.snapshot);
            return Page::finish(b);
        }
        let index = self.selected_index();
        let module = self.snapshot.entries[index].metric.module();
        // One reading needs no switcher, as one member needs no tabs on
        // the Mac: the panel is simply that module's.
        if self.snapshot.entries.len() > 1 {
            let captions: Vec<String> =
                self.snapshot.entries.iter().map(|entry| entry.caption(&self.snapshot.sensors)).collect();
            switcher(&mut b, &captions, index, Accent::signature(module));
            b.gap(CARD_GAP);
        }
        // The module's page was laid out from the top of the panel, as it
        // is for its own panel; it moves down under the switcher whole.
        // A sensor reading opens the sensors panel on that sensor - its
        // hardware's readings, its row marked - so the CPU tab and the GPU
        // tab of a temperature stack are two different panels.
        let focus = match &self.snapshot.entries[index].metric {
            barometer_core::stack::StackMetric::Sensor(id) => Some(id.clone()),
            _ => None,
        };
        let dy = b.y() - PANEL_PAD;
        let panel = self.panel(module);
        panel.focus_sensor(focus.as_deref());
        let mut page = panel.build(cx);
        lower(&mut page.elements, dy);
        let mut elements = b.elements;
        elements.extend(page.elements);
        Page { elements, height: page.height + dy }
    }
}

/// The readings as chips, the chosen one in its module's accent and the
/// rest neutral, wrapping to a new line where they would run past the
/// panel, each answering to its reading's id.
///
/// The Swift's CapsulePicker fills the chosen capsule with the accent and
/// paints its label white; the CPU panel's range picker already settled
/// for the vocabulary's quieter chip, and the same choice is made here so
/// the two pickers look like one control.
fn switcher(b: &mut Builder<'_>, captions: &[String], selected: usize, accent: Accent) {
    let (left, right) = (b.x(), b.right());
    let mut x = left;
    let mut y = b.y();
    for (index, caption) in captions.iter().enumerate() {
        let color = if index == selected { accent.primary } else { NEUTRAL };
        let mut placed = chip_left(b, x, y, caption, color, None);
        let chip = b.elements.last_mut().expect("chip_left pushes one element");
        // Only the placed chip knows its width, so a chip that ran off the
        // edge is moved down a line after the fact.
        if index > 0 && placed.right() > right {
            x = left;
            y += CHIP_H + LINE_GAP;
            chip.rect.x = x;
            chip.rect.y = y;
            placed = chip.rect;
        }
        chip.id = switch_id(index);
        chip.interactive = true;
        x = placed.right() + SWITCH_GAP;
    }
    b.advance(y - b.y() + CHIP_H);
}

/// The Swift's empty stack: its tile and name over "No readings", and the
/// sentence that says where to add some.
fn empty(b: &mut Builder<'_>, snapshot: &StackSnapshot) {
    let accent = Accent { primary: STACK_COLOR, secondary: STACK_COLOR };
    b.hero_header((STACK_COLOR, STACK_GLYPH), snapshot.title(), Some("No readings"), None, accent);
    let card = b.card_begin(None);
    b.unavailable(STACK_MARK, "No readings", "Add readings in Stacks settings.");
    b.card_end(card);
}

/// Moves a page's elements down by `dy`, zones and all. A zone's rectangle
/// is in the page's vertical coordinates whatever the element's kind, so
/// it moves with the element it belongs to.
fn lower(elements: &mut [Element], dy: f32) {
    for element in elements.iter_mut() {
        element.rect.y += dy;
        for (zone, _) in element.zones.iter_mut() {
            zone.y += dy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui::{Kind, Palette, Style, PANEL_W};
    use super::*;
    use crate::settings_ui::theme::{Theme, DEFAULT_ACCENT};
    use barometer_core::sensors::SensorKind;
    use barometer_core::stack::StackMetric;

    /// Seven DIPs a character at body size, scaled by the style, as the
    /// vocabulary's own tests measure.
    fn measure(text: &str, style: Style, width: f32) -> (f32, f32) {
        let w = text.chars().count() as f32 * 7.0 * style.size() / 14.0;
        if width > 0.0 && w > width {
            (width, (w / width).ceil() * style.line())
        } else {
            (w, style.line())
        }
    }

    fn slots() -> ModuleSlots {
        ModuleSlots {
            cpu: Arc::new(Mutex::new(CpuSnapshot::default())),
            gpu: Arc::new(Mutex::new(GpuSnapshot::default())),
            memory: Arc::new(Mutex::new(MemorySnapshot::default())),
            disks: Arc::new(Mutex::new(DisksSnapshot::default())),
            network: Arc::new(Mutex::new(NetworkSnapshot::default())),
            sensors: Arc::new(Mutex::new(SensorsSnapshot::default())),
            weather: Arc::new(Mutex::new(weather::Snapshot::default())),
        }
    }

    /// A stack content that has been opened on a published snapshot.
    fn stack(entries: Vec<StackEntry>, sensors: Vec<Sensor>) -> (StackContent, Arc<Mutex<StackSnapshot>>) {
        let feed = Arc::new(Mutex::new(StackSnapshot { id: 3, name: "Desk".into(), entries, sensors }));
        let mut content = StackContent::new(3, Arc::clone(&feed), slots());
        content.opened();
        (content, feed)
    }

    fn entry(metric: StackMetric) -> StackEntry {
        StackEntry::new(metric)
    }

    fn palette() -> Palette {
        Palette::resolve(Theme::resolve(false, DEFAULT_ACCENT))
    }

    fn context<'a>(palette: &'a Palette, content: &StackContent) -> Context<'a> {
        Context { width: PANEL_W, palette, accent: content.accent(), measure: &measure, hover: None, now_unix: 0 }
    }

    /// The switcher's chips, by their ids: a module's panel has chips of
    /// its own - the processor's range picker, for one - and those are
    /// not the switcher's.
    fn chips(elements: &[Element]) -> Vec<(&Element, &str, crate::settings_ui::theme::Color)> {
        elements
            .iter()
            .filter(|e| switch_index(e.id).is_some())
            .filter_map(|e| match &e.kind {
                Kind::Chip { text, color, .. } => Some((e, text.as_str(), *color)),
                _ => None,
            })
            .collect()
    }

    fn has_text(elements: &[Element], wanted: &str) -> bool {
        elements.iter().any(|e| matches!(&e.kind, Kind::Text { text, .. } if text == wanted))
    }

    #[test]
    fn a_stack_content_is_keyed_by_its_stack_and_not_by_a_module() {
        let (content, _) = stack(vec![entry(StackMetric::CpuTotal)], Vec::new());
        assert_eq!(content.item(), StripItem::Stack(3));
        assert_eq!(content.module(), ModuleId::Cpu);
        // A module's content, for contrast, is keyed by its module.
        let cpu = CpuContent::new(Arc::new(Mutex::new(CpuSnapshot::default())));
        assert_eq!(cpu.item(), StripItem::Module(ModuleId::Cpu));
    }

    #[test]
    fn a_stack_with_no_readings_says_so_instead_of_showing_a_module() {
        let (mut content, _) = stack(Vec::new(), Vec::new());
        let palette = palette();
        let page = content.build(&context(&palette, &content));
        assert!(has_text(&page.elements, "Desk"));
        assert!(has_text(&page.elements, "No readings"));
        assert!(chips(&page.elements).is_empty());
        assert_eq!(content.accent().primary, STACK_COLOR);
        assert!(content.actions().is_empty());
        assert_eq!(STACK_MARK.chars().next(), Some(STACK_GLYPH));
    }

    #[test]
    fn the_switcher_lists_every_reading_by_its_caption_and_marks_the_chosen_one() {
        let mut labeled = entry(StackMetric::NetworkDownload);
        labeled.label = Some("Home".into());
        let (mut content, _) =
            stack(vec![entry(StackMetric::CpuTotal), entry(StackMetric::MemoryUsedPercent), labeled], Vec::new());
        let palette = palette();
        let page = content.build(&context(&palette, &content));
        let chips = chips(&page.elements);
        let texts: Vec<&str> = chips.iter().map(|(_, text, _)| *text).collect();
        assert_eq!(texts, vec!["CPU", "MEM", "Home"]);
        assert_eq!(chips[0].2, Accent::signature(ModuleId::Cpu).primary);
        assert_eq!(chips[1].2, NEUTRAL);
        for (index, (chip, _, _)) in chips.iter().enumerate() {
            assert!(chip.interactive);
            assert_eq!(chip.id, switch_id(index));
        }
        // The chips sit on one line inside the panel's padding.
        assert_eq!(chips[0].0.rect.x, PANEL_PAD);
        assert_eq!(chips[0].0.rect.y, chips[2].0.rect.y);
        assert!(chips[2].0.rect.right() <= PANEL_W - PANEL_PAD);
    }

    #[test]
    fn choosing_a_reading_shows_that_modules_panel_under_the_switcher() {
        let (mut content, _) =
            stack(vec![entry(StackMetric::CpuTotal), entry(StackMetric::MemoryUsedPercent)], Vec::new());
        let palette = palette();
        assert_eq!(content.activate(switch_id(1)), Response::Relayout);
        assert_eq!(content.module(), ModuleId::Memory);
        assert_eq!(content.accent(), Accent::signature(ModuleId::Memory));
        let page = content.build(&context(&palette, &content));
        assert!(has_text(&page.elements, "Memory"));
        let chips = chips(&page.elements);
        assert_eq!(chips[1].2, Accent::signature(ModuleId::Memory).primary);
        // Everything that is not a chip starts below the chips, a card gap
        // under them.
        let chips_bottom = chips.iter().map(|(chip, _, _)| chip.rect.bottom()).fold(0.0, f32::max);
        let first = page
            .elements
            .iter()
            .filter(|e| !matches!(e.kind, Kind::Chip { .. }))
            .map(|e| e.rect.y)
            .fold(f32::INFINITY, f32::min);
        assert!((first - (chips_bottom + CARD_GAP)).abs() < 1e-3, "{first} vs {chips_bottom}");
        // The page is the module's page plus the switcher.
        let mut memory = MemoryContent::new(Arc::new(Mutex::new(MemorySnapshot::default())));
        let alone = memory.build(&context(&palette, &content));
        assert!((page.height - alone.height - (chips_bottom + CARD_GAP - PANEL_PAD)).abs() < 1e-3);
    }

    #[test]
    fn a_single_reading_gets_its_modules_panel_without_a_switcher() {
        let (mut content, _) = stack(vec![entry(StackMetric::GpuUtilization)], Vec::new());
        let palette = palette();
        let page = content.build(&context(&palette, &content));
        assert!(chips(&page.elements).is_empty());
        assert!(has_text(&page.elements, "GPU"));
        let mut gpu = GpuContent::new(Arc::new(Mutex::new(GpuSnapshot::default())));
        let alone = gpu.build(&context(&palette, &content));
        assert_eq!(page.height, alone.height);
        assert_eq!(page.elements.len(), alone.elements.len());
    }

    #[test]
    fn a_sensor_reading_is_captioned_by_the_sensors_name_and_opens_the_sensors_panel() {
        let sensor = Sensor {
            id: "/amdcpu/0/temperature/2".into(),
            name: "Core #3".into(),
            hardware: "AMD Ryzen 9 7950X".into(),
            kind: SensorKind::Temperature,
            value: Some(61.0),
        };
        let (mut content, _) = stack(
            vec![entry(StackMetric::CpuTotal), entry(StackMetric::Sensor(sensor.id.clone()))],
            vec![sensor],
        );
        let palette = palette();
        content.activate(switch_id(1));
        assert_eq!(content.module(), ModuleId::Sensors);
        let page = content.build(&context(&palette, &content));
        let texts: Vec<&str> = chips(&page.elements).iter().map(|(_, text, _)| *text).collect();
        assert_eq!(texts, vec!["CPU", "Core #3"]);
        assert!(has_text(&page.elements, "Sensors"));
    }

    #[test]
    fn a_reading_that_leaves_the_stack_hands_the_panel_to_the_first_one() {
        let (mut content, feed) =
            stack(vec![entry(StackMetric::CpuTotal), entry(StackMetric::DiskRead)], Vec::new());
        content.activate(switch_id(1));
        assert_eq!(content.module(), ModuleId::Disks);
        // Nothing published: nothing to lay out again for the stack's sake.
        assert!(!content.tick());
        feed.lock().unwrap().entries = vec![entry(StackMetric::CpuTotal)];
        assert!(content.tick());
        assert_eq!(content.module(), ModuleId::Cpu);
        // A reading added in front of the chosen one does not steal the
        // panel from it.
        feed.lock().unwrap().entries = vec![entry(StackMetric::MemoryUsedPercent), entry(StackMetric::CpuTotal)];
        assert!(content.tick());
        content.activate(switch_id(1));
        feed.lock().unwrap().entries =
            vec![entry(StackMetric::GpuUtilization), entry(StackMetric::MemoryUsedPercent), entry(StackMetric::CpuTotal)];
        assert!(content.tick());
        assert_eq!(content.module(), ModuleId::Cpu);
        assert_eq!(content.selected_index(), 2);
    }

    #[test]
    fn each_opening_starts_on_the_first_reading() {
        let (mut content, _) =
            stack(vec![entry(StackMetric::CpuTotal), entry(StackMetric::NetworkUpload)], Vec::new());
        content.activate(switch_id(1));
        assert_eq!(content.module(), ModuleId::Network);
        content.closed();
        content.opened();
        assert_eq!(content.module(), ModuleId::Cpu);
    }

    #[test]
    fn the_footer_and_the_presses_belong_to_the_chosen_module() {
        let (mut content, _) =
            stack(vec![entry(StackMetric::CpuTotal), entry(StackMetric::WeatherTemperature)], Vec::new());
        assert!(content.actions().is_empty(), "the processor's panel has no footer buttons");
        content.activate(switch_id(1));
        let actions = content.actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].text, "Refresh");
        // A press the switcher does not own goes to the panel showing: the
        // weather's, which turns to a day for a day row's id and ignores an
        // id it never laid out.
        assert_eq!(content.activate(Id::Custom(100)), Response::Relayout);
        assert_eq!(content.activate(Id::Custom(50)), Response::None);
        assert_eq!(content.activate(switch_id(7)), Response::None, "a chip that is not there");
        assert!(!content.hover(Some(Id::Custom(1))));
    }

    #[test]
    fn lowering_a_page_moves_its_zones_with_it() {
        let mut elements = vec![Element {
            id: Id::Custom(1),
            rect: crate::settings_ui::geometry::Rect::new(12.0, 12.0, 100.0, 50.0),
            kind: Kind::Plate,
            interactive: false,
            zones: vec![(crate::settings_ui::geometry::Rect::new(12.0, 12.0, 50.0, 50.0), Id::Custom(2))],
        }];
        lower(&mut elements, 30.0);
        assert_eq!(elements[0].rect.y, 42.0);
        assert_eq!(elements[0].rect.x, 12.0);
        assert_eq!(elements[0].zones[0].0.y, 42.0);
        assert_eq!(elements[0].zones[0].0.x, 12.0);
    }
}
