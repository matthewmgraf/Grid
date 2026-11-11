//! Windows of apps that run as administrator. A normal Grid can't move them (Windows' UIPI), so
//! after such a drag Grid offers to restart itself as administrator, and the settings window has
//! a button for the same.
//!
//! Grid can't see keys pressed in those windows either (tested: `GetAsyncKeyState` and Raw Input
//! both see nothing), so it asks after any drag of one, not only after an attempt to snap.

use std::ptr::null;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, PostQuitMessage, RegisterWindowMessageW};

use crate::{MESSAGE_CLASS, WM_APP_ELEVATE, drag, elevation, log, tray, wide, with_app};

/// Registered message from the settings window: restart the engine as administrator.
pub(crate) const ELEVATE_MESSAGE: &str = "Grid.Elevate";

/// A drag started. If Grid can't move this window (an administrator app's, while Grid isn't
/// one), remembers it and returns true: the drag is left to Windows.
pub(crate) fn drag_started(hwnd: HWND) -> bool {
    let blocked = !elevation::is_self_elevated() && elevation::is_window_elevated(hwnd) == Some(true);
    with_app(|app| {
        if blocked {
            // Picks up "Never prompt" saved in the settings window.
            app.reload_if_changed();
        }
        app.blocked = blocked.then_some(hwnd);
    });
    blocked
}

/// A drag ended. Returns true if it was a blocked one (nothing more to do for it), asking
/// whether to restart as administrator unless that is turned off, muted or already on screen.
pub(crate) fn drag_ended(hwnd: HWND) -> bool {
    let Some((blocked, ask, msg_hwnd)) = with_app(|app| {
        let blocked = app.blocked.take() == Some(hwnd);
        let ask = blocked && app.settings.ask_to_run_as_admin && !app.admin_prompt_muted && !app.admin_prompt_open;
        app.admin_prompt_open |= ask;
        (blocked, ask, app.msg_hwnd)
    }) else {
        return false;
    };
    if ask {
        ask_to_elevate(msg_hwnd);
    }
    blocked
}

/// Asks (in a helper process, so the engine keeps running) whether to restart Grid as
/// administrator. The answer is posted back as WM_APP_ELEVATE: wparam 1 for Yes, and lparam 1
/// when "Don't ask again until Grid restarts" was ticked.
fn ask_to_elevate(msg_hwnd: HWND) {
    let hwnd = msg_hwnd as usize;
    std::thread::spawn(move || {
        let output = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--admin-prompt").output());
        let answer = output.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        let yes = answer.lines().any(|l| l.trim() == "yes");
        let mute = answer.lines().any(|l| l.trim() == "mute");
        unsafe { PostMessageW(hwnd as HWND, WM_APP_ELEVATE, yes as usize, mute as isize) };
    });
}

/// The answer to `ask_to_elevate` arrived.
pub(crate) fn answered(hwnd: HWND, yes: bool, mute: bool) {
    with_app(|app| {
        app.admin_prompt_open = false;
        app.admin_prompt_muted |= mute;
    });
    if yes {
        restart_elevated(hwnd);
    }
}

/// Replaces this engine with one running as administrator. If Windows' confirmation is declined,
/// this Grid simply keeps running. An open settings window stays open.
pub(crate) fn restart_elevated(hwnd: HWND) {
    if elevation::is_self_elevated() {
        return;
    }
    if elevation::restart_elevated() {
        log::write("Exiting: restarting as administrator");
        drag::finish_drag();
        tray::remove(hwnd);
        unsafe { PostQuitMessage(0) };
    }
}

/// Whether the running engine is an administrator one; `None` if Grid isn't running. For the
/// settings window.
pub fn engine_elevated() -> Option<bool> {
    let engine = unsafe { FindWindowW(wide(MESSAGE_CLASS).as_ptr(), null()) };
    if engine.is_null() {
        return None;
    }
    Some(elevation::is_window_elevated(engine).unwrap_or(false))
}

/// Asks the running engine to restart as administrator. Called by the settings window.
pub fn restart_engine_elevated() {
    unsafe {
        let engine = FindWindowW(wide(MESSAGE_CLASS).as_ptr(), null());
        if !engine.is_null() {
            PostMessageW(engine, RegisterWindowMessageW(wide(ELEVATE_MESSAGE).as_ptr()), 0, 0);
        }
    }
}
