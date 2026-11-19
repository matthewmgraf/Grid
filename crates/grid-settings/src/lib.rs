//! The settings window (Slint). Runs in its own process so the always-on part of Grid stays small.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use grid_core::geometry::{Grid, Rect};
use grid_core::session::Command;
use grid_core::settings::{Appearance, Binding, Color, Gaps, Hotkey, Input, MAX_GAP, MonitorGrid, Overlay, Settings};
use grid_core::startup::Startup;
use slint::{Model, SharedString, VecModel};

slint::include_modules!();

/// A connected monitor, as reported by the platform layer.
#[derive(Clone, PartialEq)]
pub struct Monitor {
    /// Stable ID, used as the key in the settings file.
    pub id: String,
    /// The monitor's own name (e.g. "ASUS VG27A"); may be empty.
    pub label: String,
    pub bounds: Rect,
}

/// Starts waiting for the user to press a key or mouse button; the callback gets it (`None` if
/// canceled), possibly on another thread. Provided by the platform layer.
pub type CaptureFn = fn(Box<dyn FnOnce(Option<Input>) + Send>);

/// Like `CaptureFn`, for a key combination such as Ctrl+Alt+Z.
pub type CaptureHotkeyFn = fn(Box<dyn FnOnce(Option<Hotkey>) + Send>);

/// Reads and changes OS settings that Grid recommends.
pub struct OsSettings {
    pub drag_full_windows: fn() -> Option<bool>,
    pub set_drag_full_windows: fn(bool) -> Result<(), String>,
}

/// Installing, uninstalling and starting with the OS, for the General tab.
pub struct Installation {
    /// Where and how this copy runs, e.g. "Installed for all users in C:\Program Files\Grid".
    pub status: fn() -> String,
    pub installed: fn() -> bool,
    /// Starts the setup (install) flow in another process.
    pub install: fn(),
    /// Starts uninstalling in another process (it asks first).
    pub uninstall: fn(),
    /// How Grid starts with the OS, as things are.
    pub startup: fn() -> Startup,
    /// Changes it, asking the OS for administrator rights first where needed, and returns how it
    /// is now.
    pub set_startup: fn(Startup) -> Result<Startup, String>,
    /// Whether the "for all users" and administrator options are offered (e.g. only when
    /// installed for all users).
    pub machine_startup: fn() -> bool,
    /// Whether the signed-in user's account is an administrator (the administrator options are
    /// hidden for others).
    pub user_is_admin: fn() -> bool,
}

/// Running the engine as administrator, for the General tab.
pub struct Admin {
    /// Whether the running engine is an administrator one; `None` if no engine is running.
    pub engine_elevated: fn() -> Option<bool>,
    /// Asks the running engine to restart as administrator (the OS asks the user first).
    pub restart_engine_elevated: fn(),
}

/// What the user chose in the setup window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupChoice {
    /// `startup`: how to start with the OS. Only `me` applies when not installing for all users.
    Install { all_users: bool, startup: Startup },
    /// Run from this folder. Portable copies can't start with Windows.
    Portable,
}

/// What the platform layer provides. Anything left `None` is hidden or reported as unsupported.
#[derive(Default)]
pub struct Platform {
    pub installation: Option<Installation>,
    pub admin: Option<Admin>,
    pub capture: Option<CaptureFn>,
    pub capture_hotkey: Option<CaptureHotkeyFn>,
    /// Whether a key combination is free to use as a hotkey (not taken by the OS or another app).
    pub hotkey_available: Option<fn(Hotkey) -> bool>,
    /// Called after the settings file is saved, so the running engine can pick up the changes.
    pub settings_saved: Option<fn()>,
    pub os: Option<OsSettings>,
    /// Called once the window is on screen (e.g. to give it the app's icon).
    pub window_shown: Option<fn()>,
    /// Lists the connected monitors. Checked again every few seconds, so plugging in a monitor
    /// shows it without reopening the window.
    pub monitors: Option<fn() -> Vec<Monitor>>,
    /// Opens a web address in the user's browser (the About tab's links).
    pub open_url: Option<fn(&str)>,
    pub look: Look,
}

/// What the platform layer provides for the glass look, shared by all of Grid's windows.
#[derive(Clone, Copy, Default)]
pub struct Look {
    /// Whether the system is in dark mode (used when Appearance is "System").
    pub system_dark: Option<fn() -> bool>,
    /// The window (its native handle: an HWND on Windows) turned dark (true) or light: e.g. to
    /// match its title bar and backdrop.
    pub set_dark: Option<fn(isize, bool)>,
}

/// The parts of a window (with `theme.slint`'s `GlassBackdrop`) that `use_look` needs.
trait Themed: slint::ComponentHandle + 'static {
    fn set_system_dark(&self, dark: bool);
    fn on_dark_changed(&self, f: impl FnMut(bool) + 'static);
    fn dark(&self) -> bool;
    fn set_glass(&self, glass: bool);
}

macro_rules! themed {
    ($($window:ty),*) => {$(
        impl Themed for $window {
            fn set_system_dark(&self, dark: bool) { <$window>::set_system_dark(self, dark) }
            fn on_dark_changed(&self, f: impl FnMut(bool) + 'static) { <$window>::on_dark_changed(self, f) }
            fn dark(&self) -> bool { self.get_dark() }
            fn set_glass(&self, glass: bool) { <$window>::set_glass(self, glass) }
        }
    )*};
}
themed!(SettingsWindow, AdminPrompt, SetupWindow, UninstallWindow);

/// Gives `ui` the glass look: light or dark like the system (unless the Appearance setting says
/// otherwise), with a matching title bar, and the blurred desktop behind it once it's shown.
fn use_look(ui: &impl Themed, look: Look) {
    ui.set_system_dark(look.system_dark.is_some_and(|dark| dark()));
    let weak = ui.as_weak();
    ui.on_dark_changed(move |dark| {
        // Before the window is shown there is no native window yet; the timer below covers that.
        if let (Some(set_dark), Some(handle)) = (look.set_dark, weak.upgrade().and_then(|ui| native_handle(ui.window()))) {
            set_dark(handle, dark);
        }
    });
    // The native window only exists once the event loop is running and the window is shown.
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(GLASS_DELAY_MS), move || {
        if let Some(ui) = weak.upgrade() {
            if let (Some(set_dark), Some(handle)) = (look.set_dark, native_handle(ui.window())) {
                set_dark(handle, ui.dark());
            }
            ui.set_glass(apply_glass(ui.window()));
        }
    });
}

/// The window's native handle (HWND on Windows, NSView on macOS), once it exists.
fn native_handle(window: &slint::Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        RawWindowHandle::AppKit(h) => Some(h.ns_view.as_ptr() as isize),
        _ => None,
    }
}

/// How long after starting a window its glass is applied (see `use_look`).
const GLASS_DELAY_MS: u64 = 300;

/// Grid's home page, shown on the About tab.
pub const REPO_URL: &str = "https://github.com/matthewmgraf/Grid";
/// Where people can support Grid with a donation (GitHub Sponsors).
pub const SPONSOR_URL: &str = "https://github.com/sponsors/matthewmgraf";

/// Logo sizes in logical pixels (same as in the .slint files): the settings window's sidebar,
/// the admin and uninstall windows, and the setup window.
const SIDEBAR_LOGO: u32 = 28;
const DIALOG_LOGO: u32 = 32;
const SETUP_LOGO: u32 = 48;

/// Gives `ui` the logo at `size` logical pixels, drawn at the screen's exact pixel size once the
/// window is shown (a shrunken large logo looks smudged).
fn sharp_logo<T: slint::ComponentHandle + 'static>(ui: &T, size: u32, set: fn(&T, slint::Image)) {
    set(ui, logo_image(size));
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(GLASS_DELAY_MS), move || {
        if let Some(ui) = weak.upgrade() {
            let px = (size as f32 * ui.window().scale_factor()).round() as u32;
            set(&ui, logo_image(px));
        }
    });
}

/// How often the monitor list is checked for changes.
const MONITOR_POLL: std::time::Duration = std::time::Duration::from_millis(1500);

/// Shows the settings window until it is closed.
pub fn run(path: &Path, platform: Platform) -> Result<(), String> {
    let Platform {
        installation,
        admin,
        capture,
        capture_hotkey,
        hotkey_available,
        settings_saved,
        os,
        window_shown,
        monitors: list_monitors,
        open_url,
        look,
    } = platform;
    let monitors = list_monitors.map(|list| list()).unwrap_or_default();
    let (settings, warning) = Settings::load(path);
    let ui = SettingsWindow::new().map_err(|e| e.to_string())?;

    use_look(&ui, look);
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    ui.set_logo(logo_image(256));
    sharp_logo(&ui, SIDEBAR_LOGO, SettingsWindow::set_sidebar_logo);
    ui.set_repo_url(REPO_URL.into());
    ui.set_sponsor_url(SPONSOR_URL.into());
    ui.set_links_available(open_url.is_some());
    ui.on_open_url(move |url| {
        if let Some(open) = open_url {
            open(&url);
        }
    });
    ui.set_status(warning.unwrap_or_default().into());
    ui.set_appearance(appearance_name(settings.appearance).into());
    ui.set_cols(settings.grid.cols as i32);
    ui.set_rows(settings.grid.rows as i32);
    ui.set_transpose_portrait(settings.transpose_portrait);

    show_overlay(&ui, &settings.overlay);
    ui.set_gap_between(settings.gaps.between as i32);
    ui.set_gap_edges(settings.gaps.edges as i32);
    let hotkey_text = |h: Option<Hotkey>| SharedString::from(h.map(|h| h.to_string()).unwrap_or_default());
    ui.set_hotkey_undo(hotkey_text(settings.hotkeys.undo));
    ui.set_hotkey_save(hotkey_text(settings.hotkeys.save_positions));
    ui.set_hotkey_restore(hotkey_text(settings.hotkeys.restore_positions));
    ui.set_splash(settings.splash);
    ui.set_never_ask_admin(!settings.ask_to_run_as_admin);
    ui.set_update_at_startup(settings.updates.check_at_startup);
    ui.set_update_silently(settings.updates.install_silently);
    if let Some(os) = &os {
        ui.set_os_available(true);
        ui.set_drag_full_windows((os.drag_full_windows)().unwrap_or(true));
    }
    if let Some(inst) = &installation {
        ui.set_install_available(true);
        ui.set_install_status((inst.status)().into());
        ui.set_installed((inst.installed)());
        let (install, uninstall, startup, set_startup) = (inst.install, inst.uninstall, inst.startup, inst.set_startup);
        let machine = (inst.machine_startup)();
        ui.set_all_users_startup_available(machine);
        ui.set_admin_startup_available(machine && (inst.user_is_admin)());
        // Shown once read, so the window doesn't wait for it.
        startup_in_background(&ui.as_weak(), move || (startup(), String::new()));
        ui.on_install_clicked(install);
        ui.on_uninstall_clicked(uninstall);
        let weak = ui.as_weak();
        ui.on_startup_changed(move || {
            let Some(ui) = weak.upgrade() else { return };
            let to = Startup {
                me: ui.get_autostart(),
                all_users: ui.get_autostart_all_users(),
                admin: ui.get_autostart_admin(),
                all_admins: ui.get_autostart_all_admins(),
            };
            let ok = startup_message(SHOWN_STARTUP.get(), to);
            ui.set_status("Working…".into());
            startup_in_background(&weak, move || match set_startup(to) {
                Ok(now) => (now, ok.to_string()),
                // Refused or failed: show how things really are.
                Err(e) => (startup(), e),
            });
        });
    }

    // Kept up to date, since the engine can be restarted (or exited) while this window is open.
    let admin_timer = slint::Timer::default();
    if let Some(Admin { engine_elevated, restart_engine_elevated }) = admin {
        let state = move || match engine_elevated() {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        };
        ui.set_admin_available(true);
        ui.set_admin_state(state());
        let weak = ui.as_weak();
        admin_timer.start(slint::TimerMode::Repeated, MONITOR_POLL, move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_admin_state(state());
            }
        });
        let weak = ui.as_weak();
        ui.on_restart_as_admin(move || {
            restart_engine_elevated();
            if let Some(ui) = weak.upgrade() {
                // Whether it happens is up to Windows' prompt; the group above shows the result.
                ui.set_status("Windows will ask to confirm. If you say No, Grid keeps running as it is.".into());
            }
        });
    }

    let connected = Rc::new(RefCell::new(monitors));
    let saved = Rc::new(RefCell::new(settings.clone()));
    // Re-checks whether the form differs from what's saved, after every edit.
    let check_dirty: Rc<dyn Fn()> = Rc::new({
        let (ui, saved, connected) = (ui.as_weak(), saved.clone(), connected.clone());
        move || {
            if let Some(ui) = ui.upgrade() {
                update_dirty(&ui, &saved.borrow(), &connected.borrow());
            }
        }
    });
    ui.on_edited({
        let check_dirty = check_dirty.clone();
        move || check_dirty()
    });

    let monitor_rows = Rc::new(VecModel::from(rows_for(&connected.borrow(), &[], &[], &settings)));
    ui.set_monitors(monitor_rows.clone().into());
    // Picks up monitors being plugged in or removed while the window is open. Rows for monitors
    // that stay keep any unsaved edits.
    let monitor_timer = slint::Timer::default();
    if let Some(list) = list_monitors {
        let (rows, connected, saved, check_dirty) =
            (monitor_rows.clone(), connected.clone(), saved.clone(), check_dirty.clone());
        monitor_timer.start(slint::TimerMode::Repeated, MONITOR_POLL, move || {
            let now = list();
            if now == *connected.borrow() {
                return;
            }
            let old_rows: Vec<MonitorRow> = rows.iter().collect();
            rows.set_vec(rows_for(&now, &connected.borrow(), &old_rows, &saved.borrow()));
            *connected.borrow_mut() = now;
            check_dirty();
        });
    }
    ui.on_monitor_edited({
        let (rows, check_dirty) = (monitor_rows.clone(), check_dirty.clone());
        move |i, custom, cols, row_count| {
            if let Some(mut row) = rows.row_data(i as usize) {
                (row.custom, row.cols, row.rows) = (custom, cols, row_count);
                rows.set_row_data(i as usize, row);
            }
            check_dirty();
        }
    });

    // One list of input names per command, shown as separate groups.
    let inputs_for = |command: Command| -> Rc<VecModel<SharedString>> {
        Rc::new(VecModel::from(
            settings
                .bindings
                .iter()
                .filter(|b| b.command == command)
                .map(|b| SharedString::from(b.input.to_string()))
                .collect::<Vec<_>>(),
        ))
    };
    ui.set_resize_inputs(inputs_for(Command::Resize).into());
    ui.set_move_inputs(inputs_for(Command::Move).into());
    ui.set_cancel_inputs(inputs_for(Command::Cancel).into());
    let group = {
        let ui = ui.as_weak();
        move |command: &str| ui.upgrade().and_then(|ui| groups(&ui).into_iter().find(|g| g.name == command))
    };

    ui.on_capture({
        let ui = ui.as_weak();
        move |command, i| {
            if let Some(ui) = ui.upgrade() {
                start_capture(&ui, capture, command, i);
            }
        }
    });
    ui.on_resolve_conflict({
        let ui = ui.as_weak();
        move |move_it| {
            if let Some(ui) = ui.upgrade() {
                place_input(&ui, move_it.then(|| ui.get_conflict_input()));
            }
        }
    });
    ui.on_capture_hotkey({
        let (ui, saved) = (ui.as_weak(), saved.clone());
        move |which| {
            let Some(ui) = ui.upgrade() else { return };
            let Some(capture_hotkey) = capture_hotkey else {
                ui.set_status("Changing hotkeys isn't supported on this platform yet.".into());
                return;
            };
            if ui.get_capturing_index() >= 0 || !ui.get_capturing_hotkey().is_empty() {
                return;
            }
            ui.set_capturing_hotkey(which);
            ui.set_status("Hold Ctrl, Alt or Win (Shift too, if you like) and press another key. Esc cancels.".into());
            // The saved hotkeys are the ones the running Grid has registered.
            let registered: Vec<String> = {
                let h = saved.borrow().hotkeys;
                [h.undo, h.save_positions, h.restore_positions].into_iter().flatten().map(|h| h.to_string()).collect()
            };
            let ui = ui.as_weak();
            capture_hotkey(Box::new(move |result| {
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui.upgrade() else { return };
                    let which = ui.get_capturing_hotkey();
                    ui.set_capturing_hotkey(SharedString::new());
                    let Some(hotkey) = result else {
                        ui.set_status(SharedString::new());
                        return;
                    };
                    // Grid's own hotkeys, saved (registered by the running Grid) or still only in
                    // the form, would look taken; they aren't.
                    let name = hotkey.to_string();
                    let in_form = [ui.get_hotkey_undo(), ui.get_hotkey_save(), ui.get_hotkey_restore()];
                    let ours = registered.contains(&name) || in_form.iter().any(|h| h.as_str() == name);
                    let status = if let Some(problem) = hotkey_problem(hotkey) {
                        problem
                    } else if !ours && hotkey_available.is_some_and(|free| !free(hotkey)) {
                        format!("{hotkey} is already used by Windows or another app. Try another combination.")
                    } else {
                        set_hotkey(&ui, &which, hotkey.to_string().into());
                        format!("{hotkey} set. Click Save to start using it.")
                    };
                    ui.set_status(status.into());
                });
            }));
        }
    });
    ui.on_clear_hotkey({
        let ui = ui.as_weak();
        move |which| {
            if let Some(ui) = ui.upgrade() {
                set_hotkey(&ui, &which, SharedString::new());
            }
        }
    });
    ui.on_reset_overlay({
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                show_overlay(&ui, &Overlay::default());
                ui.invoke_edited();
            }
        }
    });
    ui.on_os_setting_changed({
        let ui = ui.as_weak();
        move |_which, on| {
            let (Some(ui), Some(os)) = (ui.upgrade(), &os) else { return };
            let state = if on { "on" } else { "off" };
            let status = (os.set_drag_full_windows)(on)
                .err()
                .unwrap_or_else(|| format!("Show window contents while dragging turned {state} in Windows (no need to Save)."));
            ui.set_status(status.into());
            // Show what Windows actually has now.
            ui.set_drag_full_windows((os.drag_full_windows)().unwrap_or(on));
        }
    });
    ui.on_add_binding({
        let (ui, group) = (ui.as_weak(), group.clone());
        move |command| {
            let Some(ui) = ui.upgrade() else { return };
            if capture.is_none() || ui.get_capturing_index() >= 0 {
                return;
            }
            // An empty row that fills in once the user presses something.
            let Some(group) = group(&command) else { return };
            let Some(inputs) = group.vec() else { return };
            inputs.push(SharedString::new());
            start_capture(&ui, capture, command, inputs.row_count() as i32 - 1);
        }
    });
    ui.on_remove_binding({
        let (group, check_dirty) = (group.clone(), check_dirty.clone());
        move |command, i| {
            if let Some(inputs) = group(&command).as_ref().and_then(Group::vec)
                && (i as usize) < inputs.row_count()
            {
                inputs.remove(i as usize);
            }
            check_dirty();
        }
    });

    ui.on_save({
        let ui = ui.as_weak();
        let path: PathBuf = path.to_owned();
        move || {
            let Some(ui) = ui.upgrade() else { return };
            let result = read_form(&ui, &saved.borrow(), &connected.borrow());
            let status = match result {
                Ok(new) => match new.save(&path) {
                    Ok(()) => {
                        *saved.borrow_mut() = new;
                        ui.set_dirty(false);
                        if let Some(notify) = settings_saved {
                            notify();
                        }
                        "Saved.".to_string()
                    }
                    Err(e) => format!("Could not save {}: {e}", path.display()),
                },
                Err(e) => e,
            };
            ui.set_status(status.into());
        }
    });
    ui.on_close_requested(|| {
        let _ = slint::quit_event_loop();
    });

    if let Some(shown) = window_shown {
        slint::Timer::single_shot(std::time::Duration::from_millis(50), shown);
    }
    ui.run().map_err(|e| e.to_string())
}

/// What the status line says after changing how Grid starts with the OS from `from` to `to`.
fn startup_message(from: Startup, to: Startup) -> &'static str {
    if to.admin && !from.admin {
        "Grid will start as administrator from your next sign-in (no need to Save). To use it now, click \"Restart Grid as administrator\"."
    } else if !to.me {
        "Grid won't start with Windows for you (no need to Save)."
    } else if from.admin && !to.admin {
        "Grid will start without administrator rights from your next sign-in (no need to Save)."
    } else {
        "Grid will start with Windows as chosen (no need to Save)."
    }
}

thread_local! {
    /// The start-with-the-OS options as last read or set (for the status message after a change).
    static SHOWN_STARTUP: std::cell::Cell<Startup> = const {
        std::cell::Cell::new(Startup { me: false, all_users: false, admin: false, all_admins: false })
    };
}

/// Runs `work` on another thread, since it asks the OS (and maybe the user, for administrator
/// rights), which would freeze the window meanwhile. Then shows the start options it returns,
/// and its status message if not empty. The checkboxes are disabled until then.
fn startup_in_background(weak: &slint::Weak<SettingsWindow>, work: impl FnOnce() -> (Startup, String) + Send + 'static) {
    if let Some(ui) = weak.upgrade() {
        ui.set_install_busy(true);
    }
    let weak = weak.clone();
    std::thread::spawn(move || {
        let (startup, status) = work();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            SHOWN_STARTUP.set(startup);
            ui.set_autostart(startup.me);
            ui.set_autostart_all_users(startup.all_users);
            ui.set_autostart_admin(startup.admin);
            ui.set_autostart_all_admins(startup.all_admins);
            if !status.is_empty() {
                ui.set_status(status.into());
            }
            ui.set_install_busy(false);
        });
    });
}

/// Puts a frosted-glass backdrop (blurred desktop) behind `window`: Acrylic on Windows 11,
/// vibrancy on macOS. Returns false where the system can't, and the window stays opaque.
fn apply_glass(window: &slint::Window) -> bool {
    let handle = window.window_handle();
    #[cfg(target_os = "windows")]
    let result = window_vibrancy::apply_acrylic(&handle, None);
    #[cfg(target_os = "macos")]
    let result = window_vibrancy::apply_vibrancy(&handle, window_vibrancy::NSVisualEffectMaterial::HudWindow, None, None);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result: Result<(), ()> = {
        let _ = handle;
        Err(())
    };
    result.is_ok()
}

/// Starts waiting for a key press for row `index` of `command`'s inputs. The result arrives
/// later through `finish_capture`.
fn start_capture(ui: &SettingsWindow, capture: Option<CaptureFn>, command: SharedString, index: i32) {
    let Some(capture) = capture else {
        ui.set_status("Changing inputs isn't supported on this platform yet.".into());
        return;
    };
    if ui.get_capturing_index() >= 0 || !ui.get_capturing_hotkey().is_empty() {
        return;
    }
    ui.set_capturing_command(command);
    ui.set_capturing_index(index);
    let ui = ui.as_weak();
    capture(Box::new(move |result| {
        // May run on the capture thread: hand the result to the UI thread.
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                finish_capture(&ui, result);
            }
        });
    }));
}

/// Handles a captured input for the row that was waiting for it. If the other action already
/// uses it, asks first (the dialog's answer goes to `place_input`).
fn finish_capture(ui: &SettingsWindow, result: Option<Input>) {
    let Some(input) = result else {
        place_input(ui, None);
        return;
    };
    let name = SharedString::from(input.to_string());
    let command = ui.get_capturing_command();
    let all = groups(ui);
    let title = |n: &str| all.iter().find(|g| g.name == n).map_or("", |g| g.title);
    let other = all.iter().find(|g| g.name != command.as_str() && g.inputs.iter().any(|n| n == name));
    if let Some(other) = other {
        let what = if matches!(input, Input::Mouse(_)) { "button" } else { "key" };
        ui.set_conflict_input(name);
        ui.set_conflict_message(
            format!(
                "This {what} is already bound to \"{}\", would you like to move it to \"{}\"?",
                other.title,
                title(&command)
            )
            .into(),
        );
        return;
    }
    place_input(ui, Some(name));
}

/// The inputs bound to one command, as shown on the Input tab.
struct Group {
    /// Name used between the UI and this code.
    name: &'static str,
    /// Name shown to the user.
    title: &'static str,
    command: Command,
    inputs: slint::ModelRc<SharedString>,
}

impl Group {
    fn vec(&self) -> Option<&VecModel<SharedString>> {
        self.inputs.as_any().downcast_ref()
    }
}

/// Every command's inputs, in the order the Input tab lists them.
fn groups(ui: &SettingsWindow) -> [Group; 3] {
    [
        Group { name: "Resize", title: "Resize", command: Command::Resize, inputs: ui.get_resize_inputs() },
        Group { name: "Move", title: "Move", command: Command::Move, inputs: ui.get_move_inputs() },
        Group {
            name: "Cancel",
            title: "Cancel",
            command: Command::Cancel,
            inputs: ui.get_cancel_inputs(),
        },
    ]
}

/// Ends the capture. With `Some`, puts the input in the capturing row and removes it from
/// wherever else it was bound. With `None`, leaves the bindings as they were (a newly added,
/// still empty row is dropped).
fn place_input(ui: &SettingsWindow, name: Option<SharedString>) {
    let (command, mut index) = (ui.get_capturing_command(), ui.get_capturing_index() as usize);
    ui.set_capturing_index(-1);
    ui.set_conflict_message(SharedString::new());
    ui.set_conflict_input(SharedString::new());
    let all = groups(ui);
    let Some(inputs) = all.iter().find(|g| g.name == command.as_str()).and_then(Group::vec) else { return };
    let Some(current) = inputs.row_data(index) else { return };

    match name {
        None if current.is_empty() => {
            inputs.remove(index);
        }
        None => {}
        Some(name) => {
            for group in &all {
                let Some(model) = group.vec() else { continue };
                let same_group = group.name == command.as_str();
                let found = model.iter().position(|n| n == name);
                if let Some(j) = found.filter(|&j| !(same_group && j == index)) {
                    model.remove(j);
                    if same_group && j < index {
                        index -= 1;
                    }
                }
            }
            inputs.set_row_data(index, name);
        }
    }
    ui.invoke_edited();
}

fn show_overlay(ui: &SettingsWindow, overlay: &Overlay) {
    ui.set_overlay_opacity(overlay.opacity);
    ui.set_blur(overlay.blur);
    ui.set_tint_color(String::from(overlay.tint_color).into());
    ui.set_line_color(String::from(overlay.line_color).into());
    ui.set_start_color(String::from(overlay.start_color).into());
    ui.set_target_color(String::from(overlay.target_color).into());
}

/// Why `hotkey` can't be used, if it can't: it needs Ctrl, Alt or Win, or it would get in the way
/// of typing (only F-keys are allowed with Shift alone or no modifier).
fn hotkey_problem(hotkey: Hotkey) -> Option<String> {
    let typing = !matches!(hotkey.key, grid_core::settings::Key::F(_));
    (typing && !hotkey.ctrl && !hotkey.alt && !hotkey.win).then(|| {
        format!(
            "{hotkey} wasn't set: it would block typing. Add Ctrl, Alt or Win (e.g. Ctrl+Alt+{}).",
            Input::Key(hotkey.key)
        )
    })
}

/// Sets hotkey `which` ("undo", "save" or "restore") to `text` ("" clears it). A combination
/// already used by another hotkey is moved here.
fn set_hotkey(ui: &SettingsWindow, which: &str, text: SharedString) {
    if !text.is_empty() {
        if ui.get_hotkey_undo() == text {
            ui.set_hotkey_undo(SharedString::new());
        }
        if ui.get_hotkey_save() == text {
            ui.set_hotkey_save(SharedString::new());
        }
        if ui.get_hotkey_restore() == text {
            ui.set_hotkey_restore(SharedString::new());
        }
    }
    match which {
        "undo" => ui.set_hotkey_undo(text),
        "save" => ui.set_hotkey_save(text),
        _ => ui.set_hotkey_restore(text),
    }
    ui.invoke_edited();
}

/// Enables Save only when the form differs from the saved settings. A form that can't be read
/// (e.g. a mistyped color) counts as changed, so Save can report the problem.
fn update_dirty(ui: &SettingsWindow, saved: &Settings, monitors: &[Monitor]) {
    let dirty = read_form(ui, saved, monitors).map_or(true, |form| !same_settings(&form, saved));
    ui.set_dirty(dirty);
    if dirty && ui.get_status() == "Saved." {
        ui.set_status(SharedString::new());
    }
}

/// Equality that ignores the order of per-monitor grids, and of bindings across commands (the
/// form lists them grouped by command).
fn same_settings(a: &Settings, b: &Settings) -> bool {
    let normalized = |s: &Settings| {
        let mut s = s.clone();
        s.monitors.sort_by_key(|m| m.monitor.to_ascii_lowercase());
        s.bindings.sort_by_key(|b| b.command as u8);
        s
    };
    normalized(a) == normalized(b)
}

/// Builds settings from the form, keeping anything the form doesn't show (such as grids for
/// monitors that aren't connected right now).
fn read_form(ui: &SettingsWindow, original: &Settings, monitors: &[Monitor]) -> Result<Settings, String> {
    let mut s = original.clone();
    s.appearance = match ui.get_appearance().as_str() {
        "Light" => Appearance::Light,
        "Dark" => Appearance::Dark,
        _ => Appearance::System,
    };
    let count = |v: i32| v.clamp(1, 64) as u32;
    s.grid = Grid { cols: count(ui.get_cols()), rows: count(ui.get_rows()) };
    s.transpose_portrait = ui.get_transpose_portrait();

    // Entries for connected monitors are rewritten from the form.
    s.monitors.retain(|g| !monitors.iter().any(|m| m.id.eq_ignore_ascii_case(&g.monitor)));
    for (m, row) in monitors.iter().zip(ui.get_monitors().iter()) {
        if row.custom {
            let grid = Grid { cols: count(row.cols), rows: count(row.rows) };
            s.monitors.push(MonitorGrid { monitor: m.id.clone(), grid });
        }
    }

    let gap = |v: i32| (v.max(0) as u32).min(MAX_GAP);
    s.gaps = Gaps { between: gap(ui.get_gap_between()), edges: gap(ui.get_gap_edges()) };
    s.splash = ui.get_splash();
    s.ask_to_run_as_admin = !ui.get_never_ask_admin();
    s.updates.check_at_startup = ui.get_update_at_startup();
    s.updates.install_silently = ui.get_update_silently();
    let hotkey = |label: &str, text: SharedString| -> Result<Option<Hotkey>, String> {
        if text.is_empty() { Ok(None) } else { text.parse().map(Some).map_err(|e| format!("{label}: {e}")) }
    };
    s.hotkeys.undo = hotkey("Undo last snap", ui.get_hotkey_undo())?;
    s.hotkeys.save_positions = hotkey("Save window positions", ui.get_hotkey_save())?;
    s.hotkeys.restore_positions = hotkey("Restore window positions", ui.get_hotkey_restore())?;

    s.overlay.opacity = ui.get_overlay_opacity();
    s.overlay.blur = ui.get_blur();
    let color = |label: &str, value: SharedString| {
        Color::try_from(value.trim().to_string()).map_err(|e| format!("{label}: {e}"))
    };
    s.overlay.tint_color = color("Tint", ui.get_tint_color())?;
    s.overlay.line_color = color("Grid lines", ui.get_line_color())?;
    s.overlay.start_color = color("Start cell", ui.get_start_color())?;
    s.overlay.target_color = color("Target area", ui.get_target_color())?;

    s.bindings.clear();
    for group in groups(ui) {
        // Empty names are rows still waiting for a key press.
        for name in group.inputs.iter().filter(|n| !n.is_empty()) {
            let input = name.parse::<Input>()?;
            if s.bindings.iter().any(|b| b.input == input) {
                return Err(format!("{name} is used more than once. Each input can start only one command."));
            }
            s.bindings.push(Binding { input, command: group.command });
        }
    }
    if !s.bindings.iter().any(|b| b.command == Command::Cancel) {
        return Err("Cancel needs at least one input.".into());
    }
    Ok(s)
}

/// Asks whether to restart Grid as administrator, after it failed to move an elevated app's
/// window. Returns (Yes, "Don't ask again until Grid restarts"); closing the window counts as No.
/// `admin_autostart` says whether Grid can start as administrator with the OS (`None`: not this
/// copy, e.g. not installed for all users) and whether it already does, for a tip about it.
pub fn ask_admin_restart(
    path: &Path,
    window_shown: Option<fn()>,
    look: Look,
    admin_autostart: Option<bool>,
) -> Result<(bool, bool), String> {
    let (settings, _) = Settings::load(path);
    let ui = AdminPrompt::new().map_err(|e| e.to_string())?;
    ui.set_appearance(appearance_name(settings.appearance).into());
    use_look(&ui, look);
    sharp_logo(&ui, DIALOG_LOGO, AdminPrompt::set_logo);
    let off = "To turn this question off for good, see Settings → General.";
    let hint = match admin_autostart {
        Some(false) => format!("Tip: tick \"Start as administrator\" in Settings → General, and Grid will start with these rights every time you sign in. {off}"),
        None => format!("Tip: install Grid for all users, and it can start with these rights every time you sign in. {off}"),
        Some(true) => off.to_string(),
    };
    ui.set_hint(hint.into());
    let answer = Rc::new(std::cell::Cell::new(false));
    ui.on_answered({
        let answer = answer.clone();
        move |yes| {
            answer.set(yes);
            let _ = slint::quit_event_loop();
        }
    });
    if let Some(shown) = window_shown {
        slint::Timer::single_shot(std::time::Duration::from_millis(50), shown);
    }
    ui.run().map_err(|e| e.to_string())?;
    Ok((answer.get(), ui.get_dont_ask()))
}

/// Shows the setup window: install for all users, for this user, or run portably. `existing` is
/// shown above the choices (e.g. that Grid is already installed). `user_is_admin`: whether to
/// offer the administrator start options. `startup`: the installed Grid's start options, to
/// tick to begin with (`None`: not installed, so the defaults). `None` if canceled.
pub fn ask_setup(
    existing: Option<String>,
    window_shown: Option<fn()>,
    look: Look,
    user_is_admin: bool,
    startup: Option<Startup>,
) -> Result<Option<SetupChoice>, String> {
    let ui = SetupWindow::new().map_err(|e| e.to_string())?;
    use_look(&ui, look);
    sharp_logo(&ui, SETUP_LOGO, SetupWindow::set_logo);
    ui.set_existing(existing.unwrap_or_default().into());
    ui.set_user_is_admin(user_is_admin);
    if let Some(s) = startup {
        ui.set_autostart(s.me);
        ui.set_autostart_all_users(s.all_users);
        ui.set_autostart_admin(s.admin);
        ui.set_autostart_all_admins(s.all_admins);
    }
    let answer = Rc::new(std::cell::Cell::new(None));
    ui.on_chosen({
        let (answer, weak) = (answer.clone(), ui.as_weak());
        move |which| {
            let Some(ui) = weak.upgrade() else { return };
            let all = Startup {
                me: ui.get_autostart(),
                all_users: ui.get_autostart_all_users(),
                // Kept even where hidden (a standard account), so reinstalling doesn't turn off
                // what an administrator set up.
                admin: ui.get_autostart_admin(),
                all_admins: ui.get_autostart_all_admins(),
            };
            // The other options need an all-users install.
            let mine = Startup { me: all.me, ..Startup::default() };
            answer.set(match which.as_str() {
                "all" => Some(SetupChoice::Install { all_users: true, startup: all }),
                "user" => Some(SetupChoice::Install { all_users: false, startup: mine }),
                _ => Some(SetupChoice::Portable),
            });
            let _ = slint::quit_event_loop();
        }
    });
    ui.on_canceled(|| {
        let _ = slint::quit_event_loop();
    });
    if let Some(shown) = window_shown {
        slint::Timer::single_shot(std::time::Duration::from_millis(50), shown);
    }
    ui.run().map_err(|e| e.to_string())?;
    Ok(answer.get())
}

/// Asks before uninstalling. `Some(delete_settings)` to go ahead, `None` if canceled.
pub fn ask_uninstall(path: &Path, window_shown: Option<fn()>, look: Look) -> Result<Option<bool>, String> {
    let (settings, _) = Settings::load(path);
    let ui = UninstallWindow::new().map_err(|e| e.to_string())?;
    ui.set_appearance(appearance_name(settings.appearance).into());
    use_look(&ui, look);
    sharp_logo(&ui, DIALOG_LOGO, UninstallWindow::set_logo);
    let answer = Rc::new(std::cell::Cell::new(None));
    ui.on_answered({
        let (answer, weak) = (answer.clone(), ui.as_weak());
        move |yes| {
            answer.set(yes.then(|| weak.upgrade().is_some_and(|ui| ui.get_delete_settings())));
            let _ = slint::quit_event_loop();
        }
    });
    if let Some(shown) = window_shown {
        slint::Timer::single_shot(std::time::Duration::from_millis(50), shown);
    }
    ui.run().map_err(|e| e.to_string())?;
    Ok(answer.get())
}

/// The logo as a Slint image, `size` pixels square (drawn larger than shown, for high-DPI screens).
fn logo_image(size: u32) -> slint::Image {
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(size, size);
    for (dst, p) in buffer.make_mut_slice().iter_mut().zip(grid_core::logo::draw(size as i32)) {
        *dst = slint::Rgba8Pixel { r: (p >> 16) as u8, g: (p >> 8) as u8, b: p as u8, a: (p >> 24) as u8 };
    }
    slint::Image::from_rgba8_premultiplied(buffer)
}

fn monitor_label(index: usize, m: &Monitor) -> String {
    let size = format!("{} × {}", m.bounds.w, m.bounds.h);
    if m.label.is_empty() {
        format!("Display {} ({size})", index + 1)
    } else {
        format!("Display {}: {} ({size})", index + 1, m.label)
    }
}

/// One row per monitor in `now`. A monitor that was already listed (in `before`, shown as
/// `before_rows`) keeps its row, with any unsaved edits; others start from the saved settings.
fn rows_for(now: &[Monitor], before: &[Monitor], before_rows: &[MonitorRow], saved: &Settings) -> Vec<MonitorRow> {
    now.iter()
        .enumerate()
        .map(|(i, m)| {
            let label = monitor_label(i, m).into();
            if let Some(row) = before.iter().position(|b| b.id == m.id).and_then(|j| before_rows.get(j)) {
                return MonitorRow { label, ..row.clone() };
            }
            let own = saved.monitor_grid(&m.id);
            let grid = own.map_or(saved.grid, |g| g.grid);
            MonitorRow { label, custom: own.is_some(), cols: grid.cols as i32, rows: grid.rows as i32 }
        })
        .collect()
}

fn appearance_name(a: Appearance) -> &'static str {
    match a {
        Appearance::System => "System",
        Appearance::Light => "Light",
        Appearance::Dark => "Dark",
    }
}

#[cfg(test)]
mod tests {
    use super::hotkey_problem;

    #[test]
    fn hotkeys_must_not_block_typing() {
        let ok = |s: &str| hotkey_problem(s.parse().unwrap()).is_none();
        assert!(ok("Ctrl+Alt+Z"));
        assert!(ok("Ctrl+Win+X"));
        assert!(ok("Win+F5"));
        assert!(ok("F8"));
        assert!(ok("Shift+F5"));
        assert!(!ok("Z"));
        assert!(!ok("Shift+Z"));
        assert!(!ok("Space"));
    }
}
