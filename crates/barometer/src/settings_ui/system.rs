// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// What the settings window asks Windows about: which appearance the shell is
// in, what the accent color is, which fonts are installed, and how to dress
// a window as a Windows 11 citizen.

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
};
use windows_sys::Win32::Graphics::Gdi::{
    EnumFontFamiliesExW, GetDC, GetMonitorInfoW, MonitorFromPoint, ReleaseDC, DEFAULT_CHARSET,
    LOGFONTW, MONITORINFO, TEXTMETRICW,
};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::gdi::wide_nul;
use super::theme::{Color, Theme};

/// Whether the shell is drawing itself light.
///
/// `SystemUsesLightTheme` rather than `AppsUseLightTheme`: the two are
/// separate switches, and this window follows the taskbar's because the strip
/// it previews lives there. `window.rs` reads the same value for the strip
/// and keeps it private, so this is the second reading of one key; the two
/// should share it once the strip exposes it.
pub fn taskbar_is_light() -> bool {
    dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "SystemUsesLightTheme")
        .is_some_and(|value| value != 0)
}

/// The user's accent color.
///
/// DWM records it as 0xAABBGGRR, which is neither a COLORREF nor the design's
/// 0xRRGGBB; the bytes are picked out by hand so the conversion is visible.
pub fn accent_color() -> Option<Color> {
    let value = dword(r"Software\Microsoft\Windows\DWM", "AccentColor")?;
    Some(Color::rgb(value as u8, (value >> 8) as u8, (value >> 16) as u8))
}

fn dword(path: &str, name: &str) -> Option<u32> {
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the buffer and its size agree, and both strings are terminated.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide_nul(path).as_ptr(),
            wide_nul(name).as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut value as *mut u32 as *mut _,
            &mut size,
        )
    };
    (status == 0).then_some(value)
}

/// The theme for the current appearance and accent.
pub fn current_theme() -> Theme {
    Theme::resolve(taskbar_is_light(), accent_color().unwrap_or(super::theme::DEFAULT_ACCENT))
}

/// Every font family GDI will draw with, sorted, instances and all.
///
/// This is the list the font cache resolves against; the picker shows
/// `families_for_picker` of it.
pub fn installed_families() -> Vec<String> {
    let mut found: Vec<(String, u32)> = Vec::new();
    // SAFETY: a screen DC, released; the callback only reads what it is given
    // and writes into the Vec whose pointer is passed through lparam.
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return Vec::new();
        }
        let mut wanted: LOGFONTW = std::mem::zeroed();
        wanted.lfCharSet = DEFAULT_CHARSET;
        EnumFontFamiliesExW(
            dc,
            &wanted,
            Some(collect_family),
            &mut found as *mut Vec<(String, u32)> as LPARAM,
            0,
        );
        ReleaseDC(std::ptr::null_mut(), dc);
    }
    family_names(found)
}

unsafe extern "system" fn collect_family(
    logical: *const LOGFONTW,
    _metrics: *const TEXTMETRICW,
    kind: u32,
    lparam: LPARAM,
) -> i32 {
    if logical.is_null() || lparam == 0 {
        return 1;
    }
    let found = &mut *(lparam as *mut Vec<(String, u32)>);
    let name = &(*logical).lfFaceName;
    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    found.push((String::from_utf16_lossy(&name[..end]), kind));
    1
}

/// Every family the machine has.
///
/// Nothing is filtered. There used to be a list here - icon fonts, bitmap
/// fonts, the `@` families Windows uses for vertical writing - dropped on the
/// grounds that nobody means Wingdings when they ask for a font. Some of that
/// was even true, and none of it was ours to decide: a blocklist written
/// against the fonts on one machine is wrong about every other machine, and
/// somebody who picks an icon font gets a row of little pictures, learns
/// something, and picks again.
///
/// GDI reports a family once per style, so repeats are collapsed. Sorted
/// without regard to case so "Segoe UI" and "segoe script" sit together.
pub fn family_names(found: Vec<(String, u32)>) -> Vec<String> {
    let mut families: Vec<String> = found
        .into_iter()
        .filter(|(name, kind)| keep_family(name, *kind))
        .map(|(name, _)| name)
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    families
}

/// The families the picker offers: every family the machine has.
///
/// These used to be folded into their parents - "Segoe UI Semibold" shown as
/// "Segoe UI", with the Weight control meant to put the Semibold back. The
/// reasoning was that offering both asks the same question twice in two
/// vocabularies. It is a fair argument and it was the wrong call: a named
/// instance is a real family with faces somebody drew, and folding it away
/// meant the one face a person actually wanted could not be chosen by name.
/// "Segoe UI Semibold" is what a Windows font dialog lists, it is what
/// TrafficMonitor writes into its INI, and it is a different thing from
/// Segoe UI asked to be bolder.
///
/// So the list is what is installed, and the Weight control stays for the
/// families that carry their weights as styles rather than as separate
/// names. Choosing an instance family and leaving the weight alone asks for
/// exactly the face - see `instance_family` and the FW_NORMAL it implies.
pub fn families_for_picker(names: &[String]) -> Vec<String> {
    let mut families: Vec<String> = names.to_vec();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    families
}

/// The family an instance name belongs to, or the name itself.
///
/// A trailing style word is dropped only while what remains is itself a
/// listed family, so "Segoe UI Variable Small" keeps its "Small" - that is
/// an optical size, and there is no "Segoe UI Variable" it is a weight of
/// in the sense that matters - while "Bahnschrift SemiBold Condensed" folds
/// twice, to "Bahnschrift". The words are matched by prefix because a face
/// name is cut at 31 characters: "Semibol" and "Semib" are both "Semibold".
pub fn fold_family(name: &str, families: &[String]) -> String {
    let mut current = name.trim().to_string();
    loop {
        let Some((parent, last)) = current.rsplit_once(' ') else { return current };
        let is_style = last.len() >= 4
            && STYLE_WORDS.iter().any(|style| style.to_lowercase().starts_with(&last.to_lowercase()));
        let parent_listed = families.iter().any(|f| f.eq_ignore_ascii_case(parent));
        if !is_style || !parent_listed {
            return current;
        }
        current = parent.to_string();
    }
}

/// What a weight or width is called when it is a family of its own.
const STYLE_WORDS: [&str; 16] = [
    "Thin",
    "Extralight",
    "Ultralight",
    "Light",
    "Semilight",
    "Regular",
    "Medium",
    "Semibold",
    "Demibold",
    "Bold",
    "Extrabold",
    "Ultrabold",
    "Black",
    "Heavy",
    "Condensed",
    "Semicondensed",
];

fn keep_family(name: &str, _kind: u32) -> bool {
    !name.is_empty()
}

/// Dresses the window frame to match the theme.
///
/// The caption is painted the same color as the navigation pane beneath it,
/// which is how the Settings app's title bar merges with its sidebar. DWM
/// draws the caption buttons in whichever ink suits the dark-mode flag, so
/// that is set too. Neither call is checked: on a Windows 10 without these
/// attributes the frame stays default, which is correct there.
/// Asks DWM for Mica behind the leftmost `glass_px` of the window.
///
/// Returns whether the backdrop was accepted. A build of Windows that does
/// not know the attribute refuses it, and the frame must not then be extended
/// - a client area declared as frame with no backdrop behind it is a hole.
///
/// Only a strip, not the whole client, and the painter fills that strip with
/// black: inside an extended frame DWM renders pure black as glass. So the
/// navigation pane becomes the desktop's own tint and the content panel
/// beside it stays a solid color, which is the division Windows 11's own
/// Settings makes.
///
/// **Only ever the dark appearance.** A frame region composites what is
/// painted over the backdrop *additively* - measured, by painting the strip
/// red and reading `#FF2020` back off the screen where `#FF0000` was drawn
/// over a `#202020` backdrop. Light ink on a dark backdrop is exactly what
/// that suits. Dark ink on a light one is what it cannot do: `#1A1A1A` over
/// a `#F3F3F3` Mica saturates to white and the navigation pane's labels
/// disappear. Writing the alpha channel instead would avoid this, but GDI's
/// `BitBlt` does not carry alpha into a window's surface - also measured -
/// and an alpha-capable presenter is a rewrite of the painter, not a
/// setting. So the light appearance keeps its solid pane, and the caller is
/// the one that knows which appearance is on.
pub fn apply_backdrop(hwnd: HWND, glass_px: i32) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
        DWM_SYSTEMBACKDROP_TYPE,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;

    let backdrop: DWM_SYSTEMBACKDROP_TYPE = DWMSBT_MAINWINDOW;
    // SAFETY: a live window and a value of the size the attribute wants.
    let accepted = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE as u32,
            &backdrop as *const _ as *const _,
            std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    } == 0;
    if !accepted {
        return false;
    }
    // Only the left strip is declared frame, not the whole client. Inside an
    // extended frame DWM renders pure black as glass, so painting the strip
    // black is what puts the backdrop behind the navigation pane - and
    // leaving the rest of the client alone is what keeps the content panel a
    // solid color rather than the backdrop showing through everything.
    //
    // Writing the alpha channel of the painter's own bitmap instead does not
    // work: GDI's BitBlt does not carry alpha into the window's surface,
    // which was measured rather than assumed.
    let strip = MARGINS {
        cxLeftWidth: glass_px.max(0),
        cxRightWidth: 0,
        cyTopHeight: 0,
        cyBottomHeight: 0,
    };
    // SAFETY: a live window and a local of the right shape.
    // SAFETY: a live window and a local of the right shape.
    unsafe { DwmExtendFrameIntoClientArea(hwnd, &strip) };
    true
}

pub fn apply_chrome(hwnd: HWND, theme: &Theme) {
    let dark: i32 = if theme.light { 0 } else { 1 };
    let caption = theme.surface_base.colorref();
    let text = theme.text_primary.colorref();
    // SAFETY: each pointer is to a local of the size stated.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &dark as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR as u32,
            &caption as *const u32 as *const _,
            std::mem::size_of::<u32>() as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR as u32,
            &text as *const u32 as *const _,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

/// Gives the window the executable's own icon for its caption and Alt+Tab.
///
/// The icon is a resource the build script compiles into `barometer.exe`
/// under id 1, which is where `winresource` puts the first icon. A window
/// class with no icon shows Windows' generic one, which is the tell of a
/// program that forgot. Found through the window's own instance handle, so
/// nothing here needs to know which module it lives in; a binary without
/// the resource - a test harness - keeps the default.
pub fn adopt_executable_icon(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, LoadIconW, SendMessageW, GWLP_HINSTANCE, ICON_BIG, ICON_SMALL,
        WM_SETICON,
    };
    const FIRST_ICON: u16 = 1;
    // SAFETY: a live window; the icon handle is a shared resource that is
    // never freed by the caller.
    unsafe {
        let instance = GetWindowLongPtrW(hwnd, GWLP_HINSTANCE) as *mut core::ffi::c_void;
        let icon = LoadIconW(instance, FIRST_ICON as usize as *const u16);
        if icon.is_null() {
            return;
        }
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
    }
}

/// Opens a page in the user's browser.
///
/// Takes a `'static` string on purpose. Everything this window opens is a
/// compile-time constant - a project page, a license - and the type makes it
/// impossible to hand the shell something that arrived over the network,
/// which is AGENTS.md's rule for anything that executes.
pub fn open_url(url: &'static str) {
    // SAFETY: terminated constants; the returned pseudo-handle is not a
    // resource.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide_nul("open").as_ptr(),
            wide_nul(url).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// The primary monitor's work area, which is where the window first opens.
pub fn primary_work_area() -> Option<RECT> {
    // Deliberately the primary monitor and not the taskbar's: the two are
    // the same on every machine that has a taskbar to show a strip on.
    const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
    // SAFETY: the info struct's size is set as the call requires.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        if monitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        Some(info.rcWork)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_keeps_every_font_the_machine_has() {
        // Nothing is dropped. A blocklist written against the fonts on one
        // machine is wrong about every other machine, and which of these is
        // a sensible readout face is the user's judgement, not this list's.
        // Only the repeats GDI reports - one per style - are collapsed, and
        // only case distinguishes them.
        let found = vec![
            ("Segoe UI".to_string(), 4),
            ("@Segoe UI".to_string(), 4),
            ("Terminal".to_string(), 1),
            ("Segoe Fluent Icons".to_string(), 4),
            ("Wingdings 2".to_string(), 4),
            ("Cascadia Mono".to_string(), 4),
            ("segoe ui".to_string(), 4),
            ("Arial".to_string(), 4),
        ];
        assert_eq!(
            family_names(found),
            vec![
                "@Segoe UI",
                "Arial",
                "Cascadia Mono",
                "Segoe Fluent Icons",
                "Segoe UI",
                "Terminal",
                "Wingdings 2",
            ]
        );
    }

    /// A named instance is a real family with faces somebody drew, and the
    /// picker offers it by name. It used to fold them into their parents -
    /// "Segoe UI Semibold" shown as "Segoe UI" with a Weight control meant
    /// to put the Semibold back - which meant the one face a person wanted
    /// could not be asked for by name.
    #[test]
    fn the_picker_offers_the_instance_families_by_their_own_names() {
        let names: Vec<String> = [
            "Arial",
            "Arial Black",
            "Bahnschrift",
            "Bahnschrift SemiBold",
            "Segoe UI",
            "Segoe UI Semibold",
            "Segoe UI Variable Display",
            "Segoe UI Variable Display Semib",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let offered = families_for_picker(&names);
        // Every one of them, the parents among them.
        assert_eq!(offered.len(), names.len());
        for wanted in ["Segoe UI", "Segoe UI Semibold", "Arial Black", "Bahnschrift SemiBold"] {
            assert!(offered.iter().any(|f| f == wanted), "{wanted} is missing from {offered:?}");
        }
        // Sorted without regard to case, and the truncated instance name
        // Windows actually registers is kept as it is.
        assert!(offered.iter().any(|f| f == "Segoe UI Variable Display Semib"));
        assert!(offered.windows(2).all(|p| p[0].to_lowercase() <= p[1].to_lowercase()));
    }

    #[test]
    fn the_accent_bytes_come_out_in_the_designs_order() {
        // DWM stores 0xAABBGGRR; the default blue #0067C0 is 0xFFC06700.
        let value: u32 = 0xFFC0_6700;
        let color = Color::rgb(value as u8, (value >> 8) as u8, (value >> 16) as u8);
        assert_eq!(color, Color(0x0067C0));
    }
}
