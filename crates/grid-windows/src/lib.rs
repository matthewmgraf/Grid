//! Windows platform layer: watches window drags, draws the grid overlay and moves windows.
//!
//! No code is injected into other processes. Drags are detected with an out-of-context
//! WinEvent hook, and low-level mouse/keyboard hooks are installed only while a drag is in
//! progress. Everything runs on the main thread's message loop.
//!
//! This file sets up the engine (`run`), holds its state (`App`, reached through `with_app`) and
//! routes its window messages. The work itself is in:
//! - `drag`: a window drag, grid mode and placing the window;
//! - `admin`: windows of administrator apps and restarting Grid as administrator;
//! - `actions`: what the tray menu and hotkeys do;
//! - `ui`: helpers for Grid's own windows (message boxes, icon, minimum size).

mod actions;
mod admin;
mod bitmap;
mod capture;
mod drag;
mod elevation;
mod hotkeys;
mod input;
pub mod install;
pub mod log;
mod monitors;
mod os_settings;
mod overlay;
mod positions;
mod splash;
mod tray;
mod ui;
pub mod update;
mod watchdog;
mod window;

use std::cell::RefCell;
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use grid_core::history::History;
use grid_core::settings::Settings;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Accessibility::SetWinEventHook;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, SendInput};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, EVENT_SYSTEM_MOVESIZEEND,
    EVENT_SYSTEM_MOVESIZESTART, GetMessageW, HICON, KillTimer, MSG, MSGFLT_ALLOW, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND,
    PostMessageW, RegisterClassW, RegisterWindowMessageW, SetTimer, TranslateMessage, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_APP, WM_CONTEXTMENU, WM_ENDSESSION, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_POWERBROADCAST,
    WM_QUERYENDSESSION, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

pub use actions::notify_settings_changed;
pub use admin::{engine_elevated, restart_engine_elevated};
pub use capture::{PARENT_ENV as CAPTURE_PARENT_ENV, capture_hotkey, capture_input};
pub use elevation::RESTARTED;
pub use monitors::Monitor;
pub use os_settings::{drag_full_windows, set_drag_full_windows};
pub use ui::{ask_yes_no, enforce_min_size, monitors, show_message, use_app_icon};

/// Title of the settings window; used to find and focus it instead of opening a second one.
pub const SETTINGS_TITLE: &str = "Grid Settings";
/// Title of the setup (install) window.
pub const SETUP_TITLE: &str = "Grid Setup";
/// Title of the window asking before uninstalling.
pub const UNINSTALL_TITLE: &str = "Uninstall Grid";
/// Title of the window that offers to restart Grid as administrator.
pub const ADMIN_PROMPT_TITLE: &str = "Grid - Administrator rights";
/// Class of the engine's hidden window, which the settings window tells about saved changes.
const MESSAGE_CLASS: &str = "Grid.Message";
/// Registered message posted to the engine after the settings file is saved.
const SETTINGS_CHANGED: &str = "Grid.SettingsChanged";

const WM_APP_REFRESH: u32 = WM_APP + 1;
const WM_APP_TRAY: u32 = WM_APP + 2;
/// Grid mode started: end the OS move loop so Grid can place the window live.
const WM_APP_TAKEOVER: u32 = WM_APP + 3;
/// Left button released after a takeover: finish the drag.
const WM_APP_DROP: u32 = WM_APP + 4;
/// Cancel after a takeover: put the window back.
const WM_APP_CANCEL: u32 = WM_APP + 5;
/// The restart-as-administrator question was answered (see `admin::ask_to_elevate`).
const WM_APP_ELEVATE: u32 = WM_APP + 6;
/// Cancel (bound to something other than Esc) before a takeover: send Esc to the OS move loop.
const WM_APP_ESCAPE: u32 = WM_APP + 7;
/// A restore of window positions finished on its own thread (wparam restored, lparam saved).
const WM_APP_RESTORED: u32 = WM_APP + 8;
/// Sent by Explorer when the taskbar is recreated (e.g. after Explorer restarts).
static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static WM_SETTINGS_CHANGED: AtomicU32 = AtomicU32::new(0);
/// Asks the engine to exit (from the installer or an update).
static WM_EXIT: AtomicU32 = AtomicU32::new(0);
/// From the settings window: restart as administrator.
static WM_ELEVATE: AtomicU32 = AtomicU32::new(0);

struct App {
    settings: Settings,
    settings_path: PathBuf,
    settings_modified: Option<SystemTime>,
    msg_hwnd: HWND,
    icon: HICON,
    drag: Option<drag::Drag>,
    refresh_posted: bool,
    pending: Option<drag::Pending>,
    /// Where snapped windows were before, for Undo.
    history: History<usize>,
    positions_path: PathBuf,
    /// A window of an app running as administrator is being dragged; Grid can't move it.
    blocked: Option<HWND>,
    /// "Don't ask again until Grid restarts" was ticked in the restart-as-administrator question.
    admin_prompt_muted: bool,
    /// The restart-as-administrator question is on screen (asked again after every blocked drag,
    /// but never twice at once).
    admin_prompt_open: bool,
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl App {
    /// Picks up changes saved by the settings window.
    /// Returns true if they changed.
    fn reload_if_changed(&mut self) -> bool {
        let modified = modified(&self.settings_path);
        if modified == self.settings_modified {
            return false;
        }
        self.settings = Settings::load(&self.settings_path).0;
        self.settings_modified = modified;
        true
    }

    fn request_refresh(&mut self) {
        if !self.refresh_posted {
            self.refresh_posted = true;
            unsafe { PostMessageW(self.msg_hwnd, WM_APP_REFRESH, 0, 0) };
        }
    }
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app state unless it is already borrowed further up the stack. Hooks can be
/// re-entered while Grid waits on a call into another process, so this must never panic.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|app| app.try_borrow_mut().ok().and_then(|mut app| app.as_mut().map(f)))
}

/// Runs Grid until Exit is chosen from the tray menu. Settings are read from `settings_path`
/// and reloaded whenever the file changes.
pub fn run(settings_path: PathBuf, restarted: bool) -> Result<(), String> {
    let settings = Settings::load(&settings_path).0;
    let positions_path = settings_path.with_file_name("positions.json");
    let settings_modified = modified(&settings_path);
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // The handle stays open for the life of the process.
        // After a restart as administrator, the old engine may still be closing.
        let mut tries = if restarted { 50 } else { 1 };
        loop {
            let mutex = CreateMutexW(null(), 0, wide("Local\\Grid.SingleInstance").as_ptr());
            if !mutex.is_null() && GetLastError() != ERROR_ALREADY_EXISTS {
                break;
            }
            tries -= 1;
            if tries == 0 {
                return Err("Grid is already running.".into());
            }
            if !mutex.is_null() {
                CloseHandle(mutex);
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let instance = GetModuleHandleW(null());
        let class = wide(MESSAGE_CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(message_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        // A hidden top-level window rather than a message-only one: the tray icon needs it for
        // its menu and to hear when Explorer restarts.
        let msg_hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("Grid").as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if msg_hwnd.is_null() {
            return Err("Could not create the message window.".into());
        }
        overlay::register_class(instance);

        WM_TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);
        WM_SETTINGS_CHANGED.store(RegisterWindowMessageW(wide(SETTINGS_CHANGED).as_ptr()), Ordering::Relaxed);
        WM_EXIT.store(RegisterWindowMessageW(wide(install::EXIT_MESSAGE).as_ptr()), Ordering::Relaxed);
        WM_ELEVATE.store(RegisterWindowMessageW(wide(admin::ELEVATE_MESSAGE).as_ptr()), Ordering::Relaxed);
        if elevation::is_self_elevated() {
            // Windows blocks messages from normal programs (Explorer, the settings window, an
            // installer) to an administrator one unless they are let through.
            let registered = [&WM_TASKBAR_CREATED, &WM_SETTINGS_CHANGED, &WM_EXIT].map(|m| m.load(Ordering::Relaxed));
            for msg in registered.into_iter().chain([WM_APP_TRAY]) {
                ChangeWindowMessageFilterEx(msg_hwnd, msg, MSGFLT_ALLOW, null_mut());
            }
        }
        let icon = tray::create_icon();
        tray::add(msg_hwnd, icon);
        if settings.splash {
            splash::show(settings.appearance);
        }
        let hotkey_errors = hotkeys::register(msg_hwnd, &settings.hotkeys);
        // Not right after an update or a restart as administrator: that Grid just started.
        if settings.updates.check_at_startup && !restarted {
            let (silent, engine) = (settings.updates.install_silently, msg_hwnd as usize);
            std::thread::spawn(move || update::check(false, silent, engine));
        }

        APP.with(|app| {
            *app.borrow_mut() = Some(App {
                settings,
                settings_path,
                settings_modified,
                msg_hwnd,
                icon,
                drag: None,
                refresh_posted: false,
                pending: None,
                history: History::default(),
                positions_path,
                blocked: None,
                admin_prompt_open: false,
                admin_prompt_muted: false,
            });
        });

        let hook = SetWinEventHook(
            EVENT_SYSTEM_MOVESIZESTART,
            EVENT_SYSTEM_MOVESIZEEND,
            null_mut(),
            Some(drag::win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if hook.is_null() {
            return Err("Could not watch for window drags.".into());
        }
        log::write(&format!(
            "Engine started: version {}, {}{}",
            env!("CARGO_PKG_VERSION"),
            if elevation::is_self_elevated() { "administrator" } else { "normal user" },
            if restarted { ", restarted" } else { "" }
        ));
        watchdog::start(msg_hwnd, describe_busy);
        if !hotkey_errors.is_empty() {
            show_message("Grid", &hotkey_errors.join("\n"), true);
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    log::write("Engine stopped");
    Ok(())
}

unsafe extern "system" fn message_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let _busy = watchdog::Busy::new(msg, wparam);
    unsafe { handle_message(hwnd, msg, wparam, lparam) }
}

/// For the watchdog's log: a readable name for what the main thread was handling.
fn describe_busy(what: u32, wparam: usize) -> String {
    let registered = |m: &AtomicU32| what != 0 && what == m.load(Ordering::Relaxed);
    let name = match what {
        0 => "nothing (waiting for messages)",
        watchdog::HOOK_DRAG_EVENT => "a window drag starting or ending",
        watchdog::HOOK_MOUSE => "the mouse hook",
        watchdog::HOOK_KEYBOARD => "the keyboard hook",
        WM_APP_REFRESH => "updating the grid during a drag",
        WM_APP_TRAY => "the tray icon (menu or click)",
        WM_APP_TAKEOVER => "starting grid mode",
        WM_APP_DROP => "placing the window at the end of a drag",
        WM_APP_CANCEL => "cancelling a drag",
        WM_APP_ELEVATE => "restarting as administrator",
        WM_APP_ESCAPE => "sending Esc",
        WM_APP_RESTORED => "reporting restored window positions",
        WM_HOTKEY => "a hotkey",
        WM_TIMER => return format!("timer {wparam}"),
        WM_POWERBROADCAST => "a sleep or resume notice",
        WM_ENDSESSION | WM_QUERYENDSESSION => "Windows ending the session",
        _ if registered(&WM_SETTINGS_CHANGED) => "reloading saved settings",
        _ if registered(&WM_EXIT) => "exiting",
        _ if registered(&WM_ELEVATE) => "restarting as administrator",
        _ if registered(&WM_TASKBAR_CREATED) => "re-adding the tray icon",
        _ => return format!("window message 0x{what:04X}"),
    };
    name.to_string()
}

unsafe fn handle_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_POWERBROADCAST => {
            match wparam as u32 {
                PBT_APMSUSPEND => log::write("PC going to sleep"),
                PBT_APMRESUMEAUTOMATIC => log::write("PC woke up"),
                _ => {}
            }
            1
        }
        WM_ENDSESSION => {
            if wparam != 0 {
                log::write("Windows is signing out or shutting down");
            }
            0
        }
        WM_APP_REFRESH => {
            drag::refresh();
            0
        }
        WM_APP_TAKEOVER => {
            drag::take_over();
            unsafe { SetTimer(hwnd, drag::LOOP_END_TIMER, drag::LOOP_END_TIMEOUT_MS, None) };
            0
        }
        WM_TIMER if wparam == drag::LOOP_END_TIMER => {
            unsafe { KillTimer(hwnd, drag::LOOP_END_TIMER) };
            drag::loop_end_timeout();
            0
        }
        WM_APP_DROP => {
            drag::drop_window(false);
            0
        }
        WM_APP_CANCEL => {
            drag::drop_window(true);
            0
        }
        WM_APP_ESCAPE => {
            drag::send_escape();
            0
        }
        WM_APP_ELEVATE => {
            admin::answered(hwnd, wparam == 1, lparam == 1);
            0
        }
        WM_TIMER if wparam == drag::SETTLE_TIMER => {
            unsafe { KillTimer(hwnd, drag::SETTLE_TIMER) };
            drag::refresh();
            0
        }
        WM_APP_RESTORED => {
            tray::notify(hwnd, &format!("Restored {wparam} of {lparam} saved windows."));
            0
        }
        WM_TIMER if wparam == drag::VERIFY_TIMER => {
            unsafe { KillTimer(hwnd, drag::VERIFY_TIMER) };
            drag::verify();
            0
        }
        WM_HOTKEY => {
            match hotkeys::Action::from_id(wparam) {
                Some(hotkeys::Action::Undo) => actions::undo(),
                Some(hotkeys::Action::SavePositions) => actions::save_positions(),
                Some(hotkeys::Action::RestorePositions) => actions::restore_positions(),
                None => {}
            }
            0
        }
        WM_APP_TRAY => {
            match lparam as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => match tray::show_menu(hwnd, actions::menu_state()) {
                    Some(tray::MenuChoice::Undo) => actions::undo(),
                    Some(tray::MenuChoice::SavePositions) => actions::save_positions(),
                    Some(tray::MenuChoice::RestorePositions) => actions::restore_positions(),
                    Some(tray::MenuChoice::Settings) => actions::open_settings(),
                    Some(tray::MenuChoice::CheckForUpdates) => {
                        let engine = hwnd as usize;
                        std::thread::spawn(move || update::check(true, false, engine));
                    }
                    Some(tray::MenuChoice::Exit) => actions::exit(hwnd, "Exit chosen from the tray menu"),
                    None => {}
                },
                WM_LBUTTONDBLCLK => actions::open_settings(),
                _ => {}
            }
            0
        }
        _ if msg != 0 && msg == WM_EXIT.load(Ordering::Relaxed) => {
            actions::exit(hwnd, "asked to exit by another Grid (installing, uninstalling or updating)");
            0
        }
        _ if msg != 0 && msg == WM_ELEVATE.load(Ordering::Relaxed) => {
            admin::restart_elevated(hwnd);
            0
        }
        _ if msg != 0 && msg == WM_SETTINGS_CHANGED.load(Ordering::Relaxed) => {
            actions::settings_changed();
            0
        }
        _ if msg != 0 && msg == WM_TASKBAR_CREATED.load(Ordering::Relaxed) => {
            if let Some(icon) = with_app(|app| app.icon) {
                tray::add(hwnd, icon);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Whether a key combination is free to use as a hotkey. Called by the settings window.
pub fn hotkey_available(hotkey: grid_core::settings::Hotkey) -> bool {
    hotkeys::available(hotkey)
}

/// Sends input from another thread: it passes through Grid's own hooks, which run on this
/// thread, and calling `SendInput` here could stall until Windows times the hooks out.
pub(crate) fn send_input(inputs: Vec<INPUT>) {
    struct Inputs(Vec<INPUT>);
    // Plain data with no pointers, so it can be moved to another thread.
    unsafe impl Send for Inputs {}
    let inputs = Inputs(inputs);
    std::thread::spawn(move || {
        let inputs = inputs;
        unsafe { SendInput(inputs.0.len() as u32, inputs.0.as_ptr(), size_of::<INPUT>() as i32) };
    });
}

/// Null-terminated UTF-16 string for Win32 calls.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
