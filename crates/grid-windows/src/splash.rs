//! Splash screen: a small card with the logo and version, shown briefly when Grid starts.

use std::cell::Cell;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use grid_core::logo;
use grid_core::settings::Appearance;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, CreateCompatibleDC, CreateFontW, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
    DeleteDC, DeleteObject, DrawTextW, FW_NORMAL, FW_SEMIBOLD, GetCurrentObject, GetMonitorInfoW, HDC,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, OBJ_BITMAP, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowRect, KillTimer, RegisterClassW, SW_SHOWNOACTIVATE, SetTimer, ShowWindow,
    ULW_ALPHA, UpdateLayeredWindow, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::{bitmap, wide};

const CLASS: &str = "Grid.Splash";
const SHOW_MS: u32 = 1400;
const FADE_STEP_MS: u32 = 16;
const FADE_STEPS: u32 = 12;
const TIMER: usize = 1;

thread_local! {
    /// Fade-out progress: 0 while fully shown.
    static FADE: Cell<u32> = const { Cell::new(0) };
    static DC: Cell<HDC> = const { Cell::new(null_mut()) };
}

/// Shows the splash screen centred on the primary monitor. It fades out by itself.
pub fn show(appearance: Appearance) {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide(CLASS);
        let wc = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance, lpszClassName: class.as_ptr(), ..zeroed() };
        RegisterClassW(&wc);

        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        let (mut dpi, mut dpi_y) = (96, 96);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
        let scale = dpi as f32 / 96.0;
        let px = |v: f32| (v * scale).round() as i32;

        let (w, h) = (px(320.0), px(180.0));
        let work = info.rcWork;
        let x = work.left + (work.right - work.left - w) / 2;
        let y = work.top + (work.bottom - work.top - h) / 2;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            wide("Grid").as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        let Some(dc) = draw(w, h, scale, is_dark(appearance)) else {
            DestroyWindow(hwnd);
            return;
        };
        DC.set(dc);
        FADE.set(0);
        update(hwnd, 255);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, TIMER, SHOW_MS, None);
    }
}

/// Draws the card into a new memory DC (with its bitmap selected).
fn draw(w: i32, h: i32, scale: f32, dark: bool) -> Option<HDC> {
    let (bitmap, bits) = bitmap::create(w, h)?;
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        SelectObject(dc, bitmap);
        let pixels = std::slice::from_raw_parts_mut(bits, (w * h) as usize);

        // Card: opaque inside a rounded rectangle, anti-aliased at the corners.
        let (bg, fg, sub) = if dark { (0x20_20_20, 0xFF_FF_FF, 0xA0_A0_A0) } else { (0xF3_F3_F3, 0x1A_1A_1A, 0x60_60_60) };
        let radius = 8.0 * scale;
        for y in 0..h {
            for x in 0..w {
                let cover = corner_cover(x, y, w, h, radius);
                let a = (cover * 255.0 + 0.5) as u32;
                let ch = |shift: u32| (((bg >> shift) & 0xFF) * a / 255) << shift;
                pixels[(y * w + x) as usize] = a << 24 | ch(16) | ch(8) | ch(0);
            }
        }

        // Logo, centred near the top.
        let size = (56.0 * scale) as i32;
        let (lx, ly) = ((w - size) / 2, (28.0 * scale) as i32);
        for (i, &p) in logo::draw(size).iter().enumerate() {
            let (x, y) = (lx + i as i32 % size, ly + i as i32 / size);
            if p >> 24 != 0 {
                let d = &mut pixels[(y * w + x) as usize];
                let inv = 255 - (p >> 24);
                let mix = |s: u32| (((p >> s) & 0xFF) + ((*d >> s) & 0xFF) * inv / 255).min(255) << s;
                *d = mix(24) | mix(16) | mix(8) | mix(0);
            }
        }

        // Name and version. GDI clears the alpha of the pixels it writes; fixed up below.
        SetBkMode(dc, TRANSPARENT as i32);
        let text = |s: &str, size: f32, weight: u32, color: u32, top: f32, bottom: f32| {
            let font = CreateFontW(
                -(size * scale) as i32,
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                0,
                0,
                0,
                5, // CLEARTYPE_QUALITY
                0,
                wide("Segoe UI").as_ptr(),
            );
            let old = SelectObject(dc, font);
            // COLORREF is 0x00BBGGRR.
            SetTextColor(dc, (color >> 16 & 0xFF) | (color & 0xFF00) | (color & 0xFF) << 16);
            let mut r = RECT { left: 0, top: (top * scale) as i32, right: w, bottom: (bottom * scale) as i32 };
            let s = wide(s);
            DrawTextW(dc, s.as_ptr(), -1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            SelectObject(dc, old);
            DeleteObject(font);
        };
        text("Grid", 22.0, FW_SEMIBOLD, fg, 92.0, 126.0);
        text(&format!("Version {}", env!("CARGO_PKG_VERSION")), 12.0, FW_NORMAL, sub, 126.0, 150.0);
        for (i, p) in pixels.iter_mut().enumerate() {
            let (x, y) = (i as i32 % w, i as i32 / w);
            if *p >> 24 == 0 && corner_cover(x, y, w, h, radius) >= 1.0 {
                *p |= 0xFF00_0000;
            }
        }
        Some(dc)
    }
}

/// How much of pixel (`x`, `y`) is inside a `w`×`h` rectangle with rounded corners.
fn corner_cover(x: i32, y: i32, w: i32, h: i32, radius: f32) -> f32 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let cx = px.clamp(radius, w as f32 - radius);
    let cy = py.clamp(radius, h as f32 - radius);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - radius;
    (0.5 - d).clamp(0.0, 1.0)
}

fn update(hwnd: HWND, alpha: u8) {
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let mut r: RECT = unsafe { zeroed() };
    unsafe { GetWindowRect(hwnd, &mut r) };
    let size = SIZE { cx: r.right - r.left, cy: r.bottom - r.top };
    let src = POINT { x: 0, y: 0 };
    unsafe { UpdateLayeredWindow(hwnd, null_mut(), null(), &size, DC.get(), &src, 0, &blend, ULW_ALPHA) };
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TIMER && wparam == TIMER {
        let step = FADE.get() + 1;
        FADE.set(step);
        if step == 1 {
            unsafe { SetTimer(hwnd, TIMER, FADE_STEP_MS, None) };
        }
        if step >= FADE_STEPS {
            unsafe {
                KillTimer(hwnd, TIMER);
                DestroyWindow(hwnd);
                let dc = DC.replace(null_mut());
                let bitmap = GetCurrentObject(dc, OBJ_BITMAP as u32);
                DeleteDC(dc);
                DeleteObject(bitmap);
            }
        } else {
            update(hwnd, (255 * (FADE_STEPS - step) / FADE_STEPS) as u8);
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Whether Grid's windows should be dark, following Windows' app mode for `System`.
pub fn is_dark(appearance: Appearance) -> bool {
    match appearance {
        Appearance::Dark => true,
        Appearance::Light => false,
        Appearance::System => {
            let mut value: u32 = 1;
            let mut size = size_of::<u32>() as u32;
            let ok = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize").as_ptr(),
                    wide("AppsUseLightTheme").as_ptr(),
                    RRF_RT_REG_DWORD,
                    null_mut(),
                    &mut value as *mut u32 as *mut c_void,
                    &mut size,
                )
            } == 0;
            ok && value == 0
        }
    }
}
