//! Helpers for Grid's own windows and dialogs: message boxes, the app icon, a minimum size the
//! window keeps even when snapped, and the monitor list for the settings window.

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW,
};

use crate::{Monitor, log, tray, wide};

/// Shows a simple message box. Errors are also written to the log.
pub fn show_message(title: &str, text: &str, error: bool) {
    if error {
        log::write(&format!("Error shown: {text}"));
    }
    let style = MB_OK | if error { MB_ICONERROR } else { MB_ICONINFORMATION };
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), style) };
}

/// Asks a Yes/No question in a standard Windows message box. Returns true for Yes.
pub fn ask_yes_no(title: &str, text: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONQUESTION, MB_YESNO};
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

/// Gives the top-level window titled `title` (in this process) Grid's icon from the exe's
/// resources, for its title bar and taskbar button. Called by the settings window once shown.
pub fn use_app_icon(title: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON,
        SendMessageW, WM_SETICON,
    };
    unsafe {
        let hwnd = FindWindowW(null(), wide(title).as_ptr());
        if hwnd.is_null() {
            return;
        }
        // Taskbar and Alt+Tab: the full logo, resource ID 1, embedded by grid-app's build script.
        let size = GetSystemMetrics(SM_CXICON);
        let big = LoadImageW(GetModuleHandleW(null()), 1 as _, IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
        // Title bar: the one-cell logo, same as the tray.
        let small = tray::create_icon();
        for (which, icon) in [(ICON_BIG, big), (ICON_SMALL, small)] {
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, which as usize, icon as isize);
            }
        }
    }
}

/// Minimum client size, in pixels at 100% scaling, for the window given to `enforce_min_size`.
static MIN_SIZE: AtomicU32 = AtomicU32::new(0);

/// Stops the top-level window titled `title` (in this process) from being made smaller than
/// `min_w` × `min_h` (client area, at 100% scaling) by anyone, including Grid snapping it.
/// Windows only applies a window's minimum size when the user drags its edge; programs that call
/// `SetWindowPos` get around it unless the window refuses the size itself, as this makes it do.
pub fn enforce_min_size(title: &str, min_w: u16, min_h: u16) {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    MIN_SIZE.store((min_w as u32) << 16 | min_h as u32, Ordering::Relaxed);
    unsafe {
        let hwnd = FindWindowW(null(), wide(title).as_ptr());
        if !hwnd.is_null() {
            SetWindowSubclass(hwnd, Some(min_size_proc), 1, 0);
        }
    }
}

unsafe extern "system" fn min_size_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, SWP_NOSIZE, WINDOWPOS, WM_WINDOWPOSCHANGING,
    };
    if msg == WM_WINDOWPOSCHANGING {
        let pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
        if pos.flags & SWP_NOSIZE == 0 {
            let packed = MIN_SIZE.load(Ordering::Relaxed);
            let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
            let (mut outer, mut client): (RECT, RECT) = unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
            unsafe {
                GetWindowRect(hwnd, &mut outer);
                GetClientRect(hwnd, &mut client);
            }
            // Frame and title bar, so the minimum applies to the client area.
            let frame_w = (outer.right - outer.left) - client.right;
            let frame_h = (outer.bottom - outer.top) - client.bottom;
            let min_w = ((packed >> 16) as f32 * scale).ceil() as i32 + frame_w;
            let min_h = ((packed & 0xFFFF) as f32 * scale).ceil() as i32 + frame_h;
            pos.cx = pos.cx.max(min_w);
            pos.cy = pos.cy.max(min_h);
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Connected monitors, with bounds in physical pixels.
pub fn monitors() -> Vec<Monitor> {
    // So bounds are physical pixels even when called before any window exists.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    crate::monitors::list()
}
