// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// When a flyout should go away, ported from PopoverDismissalMonitor.swift.
//
// The Mac monitor watches the pointer every fifty milliseconds and closes the
// panel a second after the pointer has left it, because a menu bar dropdown
// on macOS is a thing you hover. Windows' own tray flyouts do not work that
// way: they stay until the user clicks somewhere else or the window loses
// activation, and docs/ui-design.md section 10 asks for exactly that. So
// the hover-exit grace and the menu-tracking suspension are deliberately not
// ported, and what remains is the part the Mac tests pin that still holds
// here: a press inside the panel keeps it, a press on the strip column that
// opened it keeps it (the column's own click is what toggles), a press
// anywhere else closes it, and a stopped monitor never closes anything.
//
// Losing activation is the other half, and it needs no geometry: the panel
// is a foreground window while it is open, and WA_INACTIVE arriving means
// the user went somewhere else - another window, Start, the lock screen.

use crate::settings_ui::geometry::PxRect;

fn contains(rect: PxRect, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

/// Decides, for a press or a loss of activation, whether the panel closes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Monitor {
    /// Where a press does not close the panel: the panel itself and the
    /// column it belongs to, in screen pixels.
    regions: Vec<PxRect>,
    active: bool,
}

impl Monitor {
    /// Starts watching, with the panel's frame and the anchor it opened
    /// from as the places a press is allowed.
    pub fn start(regions: Vec<PxRect>) -> Monitor {
        Monitor { regions, active: true }
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.regions.clear();
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Whether a point is somewhere a press is allowed.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.regions.iter().any(|region| contains(*region, x, y))
    }

    /// Whether a button going down at a point closes the panel.
    pub fn press(&self, x: i32, y: i32) -> bool {
        self.active && !self.contains(x, y)
    }

    /// Whether the panel losing activation closes it.
    pub fn deactivated(&self) -> bool {
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: PxRect = PxRect { left: 100, top: 100, right: 480, bottom: 700 };
    const WIDGET: PxRect = PxRect { left: 260, top: 1040, right: 320, bottom: 1072 };

    /// The Mac's outsideClickDismissal: an outside click dismisses at once,
    /// an inside click keeps the popover open.
    #[test]
    fn an_outside_press_dismisses_and_an_inside_press_keeps_the_panel() {
        let monitor = Monitor::start(vec![PANEL]);
        assert!(!monitor.press(200, 200));
        assert!(monitor.press(0, 0));
        assert!(monitor.press(480, 200), "the right edge is outside");
        assert!(!monitor.press(479, 699), "the last pixel inside is inside");
    }

    /// The Mac's widgetHoverKeepsDropdownOpen, for a press: the strip column
    /// counts as inside, because its own click is what toggles the panel.
    #[test]
    fn a_press_on_the_column_that_opened_the_panel_does_not_dismiss_it() {
        let monitor = Monitor::start(vec![PANEL, WIDGET]);
        assert!(!monitor.press(290, 1050));
        assert!(monitor.press(290, 1000), "the taskbar beside the column is outside");
    }

    #[test]
    fn a_stopped_monitor_never_dismisses_anything() {
        let mut monitor = Monitor::start(vec![PANEL]);
        monitor.stop();
        assert!(!monitor.press(0, 0));
        assert!(!monitor.deactivated());
        assert!(!monitor.is_active());
    }

    #[test]
    fn losing_activation_dismisses_while_the_panel_is_open() {
        let monitor = Monitor::start(vec![PANEL]);
        assert!(monitor.deactivated());
        assert!(!Monitor::default().deactivated());
    }
}
