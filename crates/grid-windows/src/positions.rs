//! Save and restore the positions of all open windows.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};

use grid_core::positions::{SavedWindow, ShowState, Snapshot, WindowId, match_windows};
use windows_sys::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW, GetWindow, GetWindowLongPtrW,
    GetWindowPlacement, GetWindowTextW, GetWindowThreadProcessId, IsHungAppWindow, IsIconic, IsWindowVisible, IsZoomed,
    HWND_TOP, IsWindow, SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOOWNERZORDER, SWP_NOSIZE, SetForegroundWindow, SetWindowPlacement, SetWindowPos, ShowWindowAsync,
    WINDOWPLACEMENT, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use crate::window::{self, to_rect};

/// Desktop and taskbar windows, which are never saved.
const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// The positions of all open app windows.
pub fn snapshot() -> Snapshot {
    let windows = app_windows()
        .into_iter()
        .filter_map(|(hwnd, id)| {
            let mut placement: WINDOWPLACEMENT = unsafe { zeroed() };
            placement.length = size_of::<WINDOWPLACEMENT>() as u32;
            if unsafe { GetWindowPlacement(hwnd, &mut placement) } == 0 {
                return None;
            }
            let state = if unsafe { IsIconic(hwnd) } != 0 {
                ShowState::Minimized
            } else if unsafe { IsZoomed(hwnd) } != 0 {
                ShowState::Maximized
            } else {
                ShowState::Normal
            };
            let frame = window::frame_rect(hwnd)?;
            Some(SavedWindow { id, frame, normal: to_rect(placement.rcNormalPosition), state })
        })
        .collect();
    Snapshot { windows }
}

/// Puts open windows back where `snapshot` has them. Returns how many were matched, and the
/// matched windows that aren't minimized, front to back as they were saved, for `stack` once
/// they are showing again.
pub fn restore(snapshot: &Snapshot) -> (usize, Vec<HWND>) {
    let open = app_windows();
    let saved_ids: Vec<WindowId> = snapshot.windows.iter().map(|w| w.id.clone()).collect();
    let open_ids: Vec<WindowId> = open.iter().map(|(_, id)| id.clone()).collect();
    let mut pairs = match_windows(&saved_ids, &open_ids);
    // Snapshots list windows front to back (`EnumWindows` order).
    pairs.sort_by_key(|&(s, _)| s);
    let mut front_to_back = Vec::new();
    for &(s, o) in &pairs {
        let (hwnd, saved) = (open[o].0, &snapshot.windows[s]);
        // Placement calls wait for the app; a hung one would freeze Grid.
        if unsafe { IsHungAppWindow(hwnd) } != 0 {
            continue;
        }
        match saved.state {
            ShowState::Normal => {
                if unsafe { IsIconic(hwnd) } != 0 {
                    // Not activated: the order comes from `stack`, not from which came back last.
                    unsafe { ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE) };
                }
                window::move_to(hwnd, saved.frame);
                front_to_back.push(hwnd);
            }
            ShowState::Maximized | ShowState::Minimized => {
                let mut placement: WINDOWPLACEMENT = unsafe { zeroed() };
                placement.length = size_of::<WINDOWPLACEMENT>() as u32;
                unsafe { GetWindowPlacement(hwnd, &mut placement) };
                let n = saved.normal;
                placement.rcNormalPosition = RECT { left: n.x, top: n.y, right: n.right(), bottom: n.bottom() };
                placement.showCmd =
                    if saved.state == ShowState::Maximized { SW_SHOWMAXIMIZED } else { SW_SHOWMINNOACTIVE } as u32;
                placement.flags = 0;
                unsafe { SetWindowPlacement(hwnd, &placement) };
                if saved.state == ShowState::Maximized {
                    front_to_back.push(hwnd);
                }
            }
        }
    }
    (pairs.len(), front_to_back)
}

/// Stacks `windows` in the given order (front to back), in front of all other windows, and
/// brings the front one to the foreground.
pub fn stack(windows: &[HWND]) {
    let mut behind = HWND_TOP;
    for &hwnd in windows {
        // Z-order calls also wait for the app (see `restore`).
        if unsafe { IsWindow(hwnd) == 0 || IsHungAppWindow(hwnd) != 0 } {
            continue;
        }
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
        unsafe { SetWindowPos(hwnd, behind, 0, 0, 0, 0, flags) };
        behind = hwnd;
    }
    if let Some(&front) = windows.first() {
        unsafe { SetForegroundWindow(front) };
    }
}

/// Visible top-level windows that show on the taskbar, other than Grid's own.
fn app_windows() -> Vec<(HWND, WindowId)> {
    let mut handles: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(collect), &mut handles as *mut Vec<HWND> as LPARAM) };
    let own = unsafe { GetCurrentProcessId() };
    handles
        .into_iter()
        .filter(|&h| is_app_window(h))
        .filter_map(|h| Some((h, window_id(h)?)))
        .filter(|(_, id)| id.pid != own)
        .collect()
}

unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
    unsafe { (*(data as *mut Vec<HWND>)).push(hwnd) };
    1
}

fn is_app_window(hwnd: HWND) -> bool {
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return false;
        }
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let owned = !GetWindow(hwnd, GW_OWNER).is_null();
        if ex & WS_EX_TOOLWINDOW != 0 || (owned && ex & WS_EX_APPWINDOW == 0) {
            return false;
        }
        // Store apps that are suspended or on another virtual desktop are "cloaked".
        let mut cloaked: u32 = 0;
        let hr = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as _, &mut cloaked as *mut u32 as *mut c_void, 4);
        hr != 0 || cloaked == 0
    }
}

fn window_id(hwnd: HWND) -> Option<WindowId> {
    let class = text(|buf| unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) });
    let title = text(|buf| unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) });
    if title.is_empty() || SHELL_CLASSES.contains(&class.as_str()) {
        return None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    Some(WindowId { handle: hwnd as u64, pid, app: app_id(hwnd, pid), class, title })
}

/// The app's ID if it is packaged (Store apps, which share `ApplicationFrameHost.exe`, and
/// others whose install folder changes with each update), otherwise its executable path.
fn app_id(hwnd: HWND, pid: u32) -> String {
    let (exe, aumid) = process_info(pid);
    if exe.to_ascii_lowercase().ends_with("\\applicationframehost.exe") {
        // The app itself runs in a child window owned by a different process.
        if let Some(child_pid) = child_process(hwnd, pid)
            && let (_, Some(id)) = process_info(child_pid)
        {
            return id;
        }
    }
    aumid.unwrap_or(exe)
}

/// (Executable path, packaged app ID) for a process.
fn process_info(pid: u32) -> (String, Option<String>) {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return (String::new(), None);
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let exe = if QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0 {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            String::new()
        };
        let mut len = buf.len() as u32;
        let aumid = (GetApplicationUserModelId(process, &mut len, buf.as_mut_ptr()) == 0)
            .then(|| String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]));
        CloseHandle(process);
        (exe, aumid)
    }
}

/// A process other than `pid` that owns a child window of `hwnd`.
fn child_process(hwnd: HWND, pid: u32) -> Option<u32> {
    let mut found: (u32, u32) = (pid, 0);
    unsafe extern "system" fn check(child: HWND, data: LPARAM) -> BOOL {
        let found = unsafe { &mut *(data as *mut (u32, u32)) };
        let mut child_pid = 0;
        unsafe { GetWindowThreadProcessId(child, &mut child_pid) };
        if child_pid != found.0 {
            found.1 = child_pid;
            return 0;
        }
        1
    }
    unsafe { EnumChildWindows(hwnd, Some(check), &mut found as *mut (u32, u32) as LPARAM) };
    (found.1 != 0).then_some(found.1)
}

fn text(read: impl FnOnce(&mut [u16]) -> i32) -> String {
    let mut buf = [0u16; 512];
    let len = read(&mut buf).max(0) as usize;
    String::from_utf16_lossy(&buf[..len.min(buf.len())])
}
