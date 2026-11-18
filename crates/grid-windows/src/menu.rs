//! The tray menu, drawn by Grid in the settings window's glass look (Acrylic on Windows 11)
//! instead of the classic Windows menu. Used like `TrackPopupMenu`: `show` returns the chosen
//! item once the menu closes. It closes when an item is chosen, on Esc, or when the user clicks
//! anywhere else.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, DeleteDC, DeleteObject, EndPaint,
    GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT,
    SRCCOPY, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOW, SetForegroundWindow, ShowWindow, TranslateMessage,
    WA_INACTIVE, WM_ACTIVATE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_NULL, WM_RBUTTONUP, WM_SYSKEYDOWN, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::theme::{self, Align};
use crate::{bitmap, wide};

/// One line of the menu.
pub enum Entry {
    Item { id: usize, label: &'static str, enabled: bool },
    Separator,
}

const CLASS: &str = "Grid.Menu";
// Sizes at 100% scaling.
/// The menu is as wide as its longest item, but at least this.
const MIN_WIDTH: f32 = 160.0;
const PAD: f32 = 4.0;
const ITEM_HEIGHT: f32 = 28.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
/// Space left and right of an item's text.
const TEXT_SIDE: f32 = 12.0;
/// Same as the settings window: Slint's default text size, at the sidebar's medium weight.
const FONT_SIZE: f32 = 12.0;
const FONT_WEIGHT: f32 = 500.0;
const RADIUS: f32 = 6.0;

struct Menu {
    entries: Vec<Entry>,
    /// Top and bottom of each entry, in pixels.
    rows: Vec<(i32, i32)>,
    hovered: Option<usize>,
    chosen: Option<usize>,
    done: bool,
    dark: bool,
    /// The blurred desktop shows through (Windows 11); otherwise the menu is opaque.
    glass: bool,
    scale: f32,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

/// Shows the menu at the mouse pointer, owned by `owner`, and returns the chosen item's id once
/// it closes (`None` if closed without choosing).
pub fn show(owner: HWND, entries: Vec<Entry>, dark: bool) -> Option<usize> {
    // Already open (e.g. the tray icon was clicked again while it was).
    if MENU.with_borrow(|m| m.is_some()) {
        return None;
    }
    unsafe {
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        let (mut dpi, mut dpi_y) = (96, 96);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
        let scale = dpi as f32 / 96.0;
        let px = |v: f32| (v * scale).round() as i32;

        let mut rows = Vec::with_capacity(entries.len());
        let mut y = px(PAD);
        for entry in &entries {
            let h = px(if matches!(entry, Entry::Separator) { SEPARATOR_HEIGHT } else { ITEM_HEIGHT });
            rows.push((y, y + h));
            y += h;
        }
        let widest = entries
            .iter()
            .map(|e| match e {
                Entry::Item { label, .. } => theme::text_width(label, FONT_SIZE * scale, FONT_WEIGHT),
                Entry::Separator => 0.0,
            })
            .fold(0.0, f32::max);
        let w = (widest.ceil() as i32 + px(2.0 * (PAD + TEXT_SIDE))).max(px(MIN_WIDTH));
        let h = y + px(PAD);

        // Like a Windows menu: right of and below the pointer, flipped where it would leave the
        // work area (e.g. above the taskbar).
        let work = info.rcWork;
        let x = if cursor.x + w > work.right { cursor.x - w } else { cursor.x }.max(work.left);
        let y = if cursor.y + h > work.bottom { cursor.y.min(work.bottom) - h } else { cursor.y }.max(work.top);

        let instance = GetModuleHandleW(null());
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..zeroed()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            wide("Grid menu").as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            owner,
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return None;
        }
        let glass = use_backdrop(hwnd, dark);
        MENU.set(Some(Menu { entries, rows, hovered: None, chosen: None, done: false, dark, glass, scale }));
        ShowWindow(hwnd, SW_SHOW);
        // So the menu gets the keyboard, and closes when the user clicks anywhere else.
        SetForegroundWindow(hwnd);

        let mut msg: MSG = zeroed();
        while !MENU.with_borrow(|m| m.as_ref().is_none_or(|m| m.done)) {
            match GetMessageW(&mut msg, null_mut(), 0, 0) {
                0 => {
                    // WM_QUIT: pass it on to the main loop.
                    PostQuitMessage(msg.wParam as i32);
                    break;
                }
                -1 => break,
                _ => {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        DestroyWindow(hwnd);
        let menu = MENU.take()?;
        let index = menu.chosen?;
        match menu.entries.get(index) {
            Some(Entry::Item { id, .. }) => Some(*id),
            _ => None,
        }
    }
}

/// Puts the system's blurred backdrop (Acrylic, Windows 11 22H2 and later) behind the window,
/// with rounded corners. Returns false where it isn't available.
fn use_backdrop(hwnd: HWND, dark: bool) -> bool {
    const DWMWCP_ROUND: i32 = 2;
    const DWMSBT_TRANSIENTWINDOW: i32 = 3;
    let set = |attribute: i32, value: i32| unsafe {
        DwmSetWindowAttribute(hwnd, attribute as u32, (&value as *const i32).cast(), size_of::<i32>() as u32) == 0
    };
    set(DWMWA_USE_IMMERSIVE_DARK_MODE, dark.into());
    set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND);
    // The whole window is "frame", so the backdrop shows wherever the menu is see-through.
    let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
    unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) == 0 && set(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW) }
}

/// Draws the menu into premultiplied pixels (`w`×`h`).
fn draw(menu: &Menu, pixels: &mut [u32], w: i32) {
    let colors = theme::colors(menu.dark);
    let px = |v: f32| (v * menu.scale).round() as i32;
    pixels.fill(theme::premultiply(colors.tint, if menu.glass { colors.tint_alpha } else { 1.0 }));
    for (i, (entry, &(top, bottom))) in menu.entries.iter().zip(&menu.rows).enumerate() {
        match entry {
            Entry::Separator => {
                let y = (top + bottom) / 2;
                let line = RECT { left: px(PAD + 8.0), top: y, right: w - px(PAD + 8.0), bottom: y + px(1.0).max(1) };
                theme::fill_rounded(pixels, w, line, 0.0, colors.divider, colors.divider_alpha);
            }
            Entry::Item { label, enabled, .. } => {
                if menu.hovered == Some(i) {
                    let r = RECT { left: px(PAD), top, right: w - px(PAD), bottom };
                    theme::fill_rounded(pixels, w, r, RADIUS * menu.scale, colors.selected, colors.selected_alpha);
                }
                let (x, center) = ((PAD + TEXT_SIDE) * menu.scale, (top + bottom) as f32 / 2.0);
                let alpha = if *enabled { 1.0 } else { 0.4 };
                theme::draw_text(pixels, w, label, x, center, Align::Left, FONT_SIZE * menu.scale, FONT_WEIGHT, colors.text, alpha);
            }
        }
    }
}

fn paint(hwnd: HWND) {
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut r: RECT = zeroed();
        GetClientRect(hwnd, &mut r);
        let (w, h) = (r.right, r.bottom);
        if let Some((bitmap, bits)) = bitmap::create(w, h) {
            let pixels = std::slice::from_raw_parts_mut(bits, (w * h) as usize);
            MENU.with_borrow(|m| {
                if let Some(m) = m {
                    draw(m, pixels, w);
                }
            });
            let mem = CreateCompatibleDC(dc);
            let old = SelectObject(mem, bitmap);
            BitBlt(dc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old);
            DeleteDC(mem);
            DeleteObject(bitmap);
        }
        EndPaint(hwnd, &ps);
    }
}

/// The item under client point `y`, if it can be chosen.
fn item_at(menu: &Menu, y: i32) -> Option<usize> {
    menu.rows.iter().zip(&menu.entries).position(|(&(top, bottom), e)| {
        y >= top && y < bottom && matches!(e, Entry::Item { enabled: true, .. })
    })
}

/// The next item that can be chosen after (`step` 1) or before (`step` -1) the hovered one.
fn next_item(menu: &Menu, step: isize) -> Option<usize> {
    let n = menu.entries.len() as isize;
    let start = menu.hovered.map_or(if step > 0 { -1 } else { n }, |i| i as isize);
    (1..=n).map(|k| (start + k * step).rem_euclid(n) as usize).find(|&i| matches!(menu.entries[i], Entry::Item { enabled: true, .. }))
}

/// Changes the menu and redraws it if anything visible changed.
fn update(hwnd: HWND, f: impl FnOnce(&mut Menu)) {
    let (changed, done) = MENU.with_borrow_mut(|m| {
        let Some(m) = m else { return (false, false) };
        let before = m.hovered;
        f(m);
        (m.hovered != before, m.done)
    });
    if changed {
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
    if done {
        // Wakes `show`'s loop, which may be waiting for a message (e.g. after WM_ACTIVATE, which
        // is sent rather than posted).
        unsafe { PostMessageW(hwnd, WM_NULL, 0, 0) };
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
    match msg {
        WM_PAINT => {
            paint(hwnd);
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            unsafe { TrackMouseEvent(&mut track) };
            update(hwnd, |m| m.hovered = item_at(m, y));
            return 0;
        }
        WM_MOUSELEAVE => {
            update(hwnd, |m| m.hovered = None);
            return 0;
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            update(hwnd, |m| {
                if let Some(i) = item_at(m, y) {
                    m.chosen = Some(i);
                    m.done = true;
                }
            });
            return 0;
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            update(hwnd, |m| match key {
                VK_UP => m.hovered = next_item(m, -1),
                VK_DOWN => m.hovered = next_item(m, 1),
                VK_RETURN | VK_SPACE => {
                    m.chosen = m.hovered;
                    m.done = m.hovered.is_some();
                }
                VK_ESCAPE => m.done = true,
                _ => {}
            });
            return 0;
        }
        // Alt or F10, as in Windows menus.
        WM_SYSKEYDOWN => {
            update(hwnd, |m| m.done = true);
            return 0;
        }
        WM_ACTIVATE if (wparam & 0xFFFF) as u32 == WA_INACTIVE => {
            update(hwnd, |m| m.done = true);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
