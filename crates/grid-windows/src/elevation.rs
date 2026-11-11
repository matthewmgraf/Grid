//! Administrator rights. Windows won't let a normal program move the windows of an app that runs
//! as administrator (Task Manager, for example), so Grid offers to restart itself with those rights.

use std::mem::size_of;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, SW_SHOWNORMAL};

use crate::wide;

/// Command-line flag for the restarted engine: it waits for the old one to exit.
pub const RESTARTED: &str = "--restarted";

/// Whether this process (Grid) runs as administrator.
pub fn is_self_elevated() -> bool {
    token_elevated(unsafe { GetCurrentProcess() }) == Some(true)
}

/// Whether the app that owns `hwnd` runs as administrator. `None` if that can't be checked.
pub fn is_window_elevated(hwnd: HWND) -> Option<bool> {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let elevated = token_elevated(process);
    unsafe { CloseHandle(process) };
    elevated
}

fn token_elevated(process: HANDLE) -> Option<bool> {
    let mut token: HANDLE = null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut len = 0;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    unsafe { CloseHandle(token) };
    (ok != 0).then_some(elevation.TokenIsElevated != 0)
}

/// Starts a new Grid engine as administrator (Windows asks the user to confirm). Returns false if
/// it didn't start, e.g. because the user said no.
pub fn restart_elevated() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let (verb, file, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(RESTARTED));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: params.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..unsafe { std::mem::zeroed() }
    };
    unsafe { ShellExecuteExW(&mut info) != 0 }
}
