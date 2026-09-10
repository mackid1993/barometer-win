// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Paints a panel into a bitmap so a person can look at it.
//
// Not a test of anything. The tests that use it are ignored and run by
// hand - `cargo test -p barometer-app render_ -- --ignored` - with
// `BAROMETER_RENDER_DIR` pointing at a folder, or the temp directory when
// it is not set. A panel is a popup over a live taskbar, which no test
// harness has and no screenshot tool can reach when the app runs elevated,
// and every polish pass on the panels has started by looking at one of
// these: the same painter, the same fonts, the same fixture the tests lay
// out, at the scale of the display the owner looks at.

use std::cell::RefCell;
use std::path::PathBuf;

use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};

use super::paint::{self, Surface};
use super::ui::{self, Palette, Style};
use super::{Content, Context};
use crate::settings_ui::gdi::{Canvas, FontCache};
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::theme::{Theme, DEFAULT_ACCENT};

/// The scale the pictures are drawn at: 150%, the display they are looked
/// at on. Text hinting differs by scale, so a picture at 100% would not be
/// the panel the eye sees.
const SCALE: f32 = 1.5;

/// Lays the content out with the real fonts, paints its whole page, and
/// writes it as `<name>-dark.bmp` or `<name>-light.bmp`.
pub fn to_bitmap(content: &mut dyn Content, name: &str, light: bool, now_unix: i64) -> PathBuf {
    let dir = std::env::var("BAROMETER_RENDER_DIR").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    let palette = Palette::resolve(Theme::resolve(light, DEFAULT_ACCENT));
    let accent = content.accent();
    let families = vec!["Segoe UI Variable Text".to_string()];
    let mut fonts = FontCache::new(&families);

    // SAFETY: a memory DC and a DIB this function owns and releases; the
    // canvas borrows the DC only inside the blocks below.
    let file = unsafe {
        let measure_dc = CreateCompatibleDC(std::ptr::null_mut());
        let page = {
            let mut canvas = Canvas::new(measure_dc, SCALE, &mut fonts);
            let surface = RefCell::new(Surface { canvas: &mut canvas, palette: &palette, accent });
            let measure = |text: &str, style: Style, wrap: f32| -> (f32, f32) { surface.borrow_mut().measure(text, style, wrap) };
            let cx = Context { width: ui::PANEL_W, palette: &palette, accent, measure: &measure, hover: None, now_unix };
            content.build(&cx)
        };
        DeleteDC(measure_dc);

        let width = (ui::PANEL_W * SCALE).round() as i32;
        let height = (page.height * SCALE).round().max(1.0) as i32;
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB as u32;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        assert!(!bitmap.is_null(), "the bitmap could not be made");
        let previous = SelectObject(dc, bitmap as _);
        {
            let mut canvas = Canvas::new(dc, SCALE, &mut fonts);
            let mut surface = Surface { canvas: &mut canvas, palette: &palette, accent };
            let whole = Rect::new(0.0, 0.0, ui::PANEL_W, page.height);
            surface.canvas.fill_rect(whole, palette.ground);
            paint::draw(&mut surface, &page.elements, None, None, 0.0, 0.0, whole);
        }
        SelectObject(dc, previous);
        let pixels = std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);
        let file = bmp(width, height, pixels);
        DeleteObject(bitmap as _);
        DeleteDC(dc);
        file
    };
    let path = dir.join(format!("{name}-{}.bmp", if light { "light" } else { "dark" }));
    std::fs::write(&path, &file).expect("the bitmap could not be written");
    path
}

/// A 32-bit top-down BMP around the pixels, the same shape `--marks` writes.
fn bmp(width: i32, height: i32, pixels: &[u8]) -> Vec<u8> {
    let mut file: Vec<u8> = Vec::with_capacity(pixels.len() + 54);
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&(54u32 + pixels.len() as u32).to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&54u32.to_le_bytes());
    file.extend_from_slice(&40u32.to_le_bytes());
    file.extend_from_slice(&width.to_le_bytes());
    file.extend_from_slice(&(-height).to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&32u16.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
    file.extend_from_slice(&[0u8; 16]);
    file.extend_from_slice(pixels);
    file
}
