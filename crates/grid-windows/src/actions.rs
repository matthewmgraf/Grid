//! What the tray menu and hotkeys do: undo, save and restore window positions, open settings,
//! pick up saved settings, and exit.

use std::ptr::null;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use grid_core::positions::Snapshot;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, IsWindow, PostMessageW, PostQuitMessage, RegisterWindowMessageW, SW_RESTORE,
    SetForegroundWindow, ShowWindowAsync, WM_CLOSE,
};

use crate::{
    MESSAGE_CLASS, SETTINGS_CHANGED, SETTINGS_TITLE, WM_APP_RESTORED, drag, hotkeys, log, positions, show_message, tray,
    wide, window, with_app,
};

/// After restoring window positions, waits this long for minimized windows to come back before
/// stacking them in their saved order.
const STACK_DELAY_MS: u64 = 300;
/// A restore of window positions is running on its own thread.
static RESTORING: AtomicBool = AtomicBool::new(false);

/// Re-reads the settings file after the settings window saved it, and re-registers hotkeys.
pub(crate) fn settings_changed() {
    let changed = with_app(|app| app.reload_if_changed().then_some((app.msg_hwnd, app.settings.hotkeys))).flatten();
    let Some((hwnd, hotkeys)) = changed else { return };
    let errors = hotkeys::register(hwnd, &hotkeys);
    if !errors.is_empty() {
        show_message("Grid", &errors.join("\n"), true);
    }
}

pub(crate) fn menu_state() -> tray::MenuState {
    let (can_undo, path) = with_app(|app| (!app.history.is_empty(), app.positions_path.clone())).unwrap_or_default();
    tray::MenuState { can_undo, can_restore: path.exists() }
}

/// Puts the most recently snapped window (that is still open) back where it was.
pub(crate) fn undo() {
    let entry = with_app(|app| {
        if app.drag.is_some() {
            return None;
        }
        app.history.pop(|w| unsafe { IsWindow(w as HWND) } != 0)
    })
    .flatten();
    if let Some((hwnd, before)) = entry {
        window::move_to(hwnd as HWND, before);
    }
}

pub(crate) fn save_positions() {
    let Some((hwnd, path)) = with_app(|app| (app.msg_hwnd, app.positions_path.clone())) else { return };
    let snapshot = positions::snapshot();
    match snapshot.save(&path) {
        Ok(()) => tray::notify(hwnd, &format!("Saved the positions of {} windows.", snapshot.windows.len())),
        Err(e) => show_message("Grid", &format!("Could not save window positions to {}: {e}", path.display()), true),
    }
}

pub(crate) fn restore_positions() {
    let Some((hwnd, path)) = with_app(|app| (app.msg_hwnd, app.positions_path.clone())) else { return };
    match Snapshot::load(&path) {
        Ok(snapshot) => {
            // On another thread: placing and stacking windows waits for each app to answer, and
            // a busy app would otherwise freeze Grid (and its hooks) until it does.
            if RESTORING.swap(true, Ordering::AcqRel) {
                return; // the previous restore is still waiting on an app
            }
            let engine = hwnd as usize;
            std::thread::spawn(move || {
                let (count, front_to_back) = positions::restore(&snapshot);
                // Gives minimized windows time to come back before they are stacked.
                std::thread::sleep(Duration::from_millis(STACK_DELAY_MS));
                positions::stack(&front_to_back);
                RESTORING.store(false, Ordering::Release);
                unsafe { PostMessageW(engine as HWND, WM_APP_RESTORED, count, snapshot.windows.len() as isize) };
            });
        }
        Err(_) if !path.exists() => tray::notify(hwnd, "No window positions have been saved yet."),
        Err(e) => show_message("Grid", &format!("Could not read {}: {e}", path.display()), true),
    }
}

/// Tells a running Grid engine that the settings file was saved. Called by the settings window.
pub fn notify_settings_changed() {
    unsafe {
        let engine = FindWindowW(wide(MESSAGE_CLASS).as_ptr(), null());
        if !engine.is_null() {
            PostMessageW(engine, RegisterWindowMessageW(wide(SETTINGS_CHANGED).as_ptr()), 0, 0);
        }
    }
}

/// Focuses the settings window if it is open, otherwise starts it as a separate process.
pub(crate) fn open_settings() {
    let existing = unsafe { FindWindowW(null(), wide(SETTINGS_TITLE).as_ptr()) };
    if !existing.is_null() {
        unsafe {
            if IsIconic(existing) != 0 {
                // Async: a busy settings window mustn't freeze the engine.
                ShowWindowAsync(existing, SW_RESTORE);
            }
            SetForegroundWindow(existing);
        }
        return;
    }
    let result = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--settings").spawn());
    if let Err(e) = result {
        show_message("Grid", &format!("Could not open settings: {e}"), true);
    }
}

/// Quits Grid: the settings window, the tray icon and the engine. `reason` goes in the log.
pub(crate) fn exit(hwnd: HWND, reason: &str) {
    log::write(&format!("Exiting: {reason}"));
    drag::finish_drag();
    close_settings();
    tray::remove(hwnd);
    unsafe { PostQuitMessage(0) };
}

/// Asks an open settings window (a separate process) to close, so Exit quits all of Grid.
fn close_settings() {
    let existing = unsafe { FindWindowW(null(), wide(SETTINGS_TITLE).as_ptr()) };
    if !existing.is_null() {
        unsafe { PostMessageW(existing, WM_CLOSE, 0, 0) };
    }
}
