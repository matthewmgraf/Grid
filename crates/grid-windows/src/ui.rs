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

/// Whether Windows apps are set to dark mode (Settings → Personalization → Colors).
pub fn system_dark() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut light: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let ok = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, null_mut(), (&mut light as *mut u32).cast(), &mut size)
    } == 0;
    ok && light == 0
}

/// Gives window `hwnd` (one of this process's, as an integer) a dark or light title bar. On
/// Windows 11 this also sets whether its Acrylic backdrop is dark or light.
pub fn set_dark_window(hwnd: isize, dark: bool) {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    let value: i32 = dark.into();
    unsafe {
        DwmSetWindowAttribute(hwnd as HWND, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, (&value as *const i32).cast(), size_of::<i32>() as u32);
    }
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

/// Centers the top-level window titled `title` (in this process) on the monitor under the mouse
/// pointer, within its work area (the screen minus the taskbar).
pub fn center_window(title: &str) {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };
    unsafe {
        let hwnd = FindWindowW(null(), wide(title).as_ptr());
        if hwnd.is_null() {
            return;
        }
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST), &mut info) == 0 {
            return;
        }
        let work = info.rcWork;
        // Twice: moving to a monitor with different scaling resizes the window, so center it again
        // at its new size.
        for _ in 0..2 {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut r);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let x = work.left + ((work.right - work.left) - w) / 2;
            let y = work.top + ((work.bottom - work.top) - h) / 2;
            SetWindowPos(hwnd, null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
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

/// Opens a web page in the user's default browser. Only `https://` addresses (without quotes), so
/// this can't be used to start programs. When Grid runs as administrator, the page is handed to
/// Windows Explorer, which opens it in the user's normal browser: started straight from here,
/// the browser would get administrator rights too.
pub fn open_url(url: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if !url.starts_with("https://") || url.contains('"') {
        return;
    }
    if crate::elevation::is_self_elevated() {
        open_unelevated(url);
    } else {
        unsafe { ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(url).as_ptr(), null(), null(), SW_SHOWNORMAL) };
    }
}

/// Opens `target` (a web page, or a program to start) through Windows Explorer, which hands it
/// to the Explorer already running for the user: so it opens without administrator rights even
/// when Grid has them. (Not with arguments: Explorer doesn't pass any.) Returns false if it
/// couldn't be handed over.
pub(crate) fn open_unelevated(target: &str) -> bool {
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if target.contains('"') {
        return false;
    }
    // Found in the Windows folder, not by name.
    let mut dir = [0u16; 260];
    let len = unsafe { GetWindowsDirectoryW(dir.as_mut_ptr(), dir.len() as u32) } as usize;
    if len == 0 || len >= dir.len() {
        return false;
    }
    let explorer = format!("{}\\explorer.exe", String::from_utf16_lossy(&dir[..len]));
    let args = format!("\"{target}\"");
    // ShellExecute reports success with a value above 32.
    let result = unsafe {
        ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(&explorer).as_ptr(), wide(&args).as_ptr(), null(), SW_SHOWNORMAL)
    };
    result as usize > 32
}

/// Connected monitors, with bounds in physical pixels.
pub fn monitors() -> Vec<Monitor> {
    // So bounds are physical pixels even when called before any window exists.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    crate::monitors::list()
}

/// Makes DLLs this process loads from now on come from System32 only, never from the exe's own
/// folder or the current folder: a downloaded grid.exe runs from e.g. Downloads (also as
/// administrator, when installing), where a planted DLL with a system DLL's name would otherwise
/// be loaded. Call first thing. (The DLLs grid.exe imports directly are covered by a linker
/// setting, see grid-app's build.rs.)
pub fn load_dlls_from_system32() {
    use windows_sys::Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories};
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
}
