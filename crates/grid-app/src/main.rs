// Release builds have no console window; Grid lives in the tray.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

fn main() {
    let path = settings_path();
    // grid.log next to grid.json: starts, exits, errors and freezes only (never input).
    #[cfg(windows)]
    grid_windows::log::init(&path);
    let args: Vec<String> = std::env::args().collect();
    let has_arg = |flag: &str| args.iter().any(|a| a == flag);
    let arg_after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let result = if has_arg("--capture-hotkey") {
        capture_hotkey()
    } else if has_arg("--capture") {
        capture()
    } else if has_arg("--settings") {
        settings(&path)
    } else if has_arg("--admin-prompt") {
        admin_prompt(&path)
    } else if has_arg("--setup") {
        setup::run(false)
    } else if let Some(scope) = arg_after("--install") {
        setup::install_elevated(&scope)
    } else if has_arg("--uninstall") {
        setup::uninstall(has_arg("--confirmed"))
    } else if let Some(version) = arg_after("--apply-update") {
        setup::apply_update(&version)
    } else {
        engine(&path)
    };
    if let Err(e) = result {
        report_error(&e);
        std::process::exit(1);
    }
}

/// Next to the exe when portable; in the user's app data folder when installed.
#[cfg(windows)]
fn settings_path() -> PathBuf {
    grid_windows::install::settings_path()
}

#[cfg(not(windows))]
fn settings_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("grid.json")))
        .unwrap_or_else(|| PathBuf::from("grid.json"))
}

/// Installing, uninstalling and the first-run setup window.
#[cfg(windows)]
mod setup {
    use grid_settings::SetupChoice;
    use grid_windows::install::{self, Scope};

    fn scope_text(scope: Scope) -> &'static str {
        match scope {
            Scope::AllUsers => "for all users",
            Scope::CurrentUser => "for you only",
        }
    }

    /// Where and how this copy runs, for the settings window.
    pub fn status() -> String {
        match install::current_scope() {
            Some(scope) => {
                let dir = install::install_dir(scope).map(|d| d.display().to_string()).unwrap_or_default();
                format!("Installed {} in {dir}. Settings are kept in your app data folder.", scope_text(scope))
            }
            None => "Running portably: Grid and its settings are in the folder grid.exe is in.".into(),
        }
    }

    /// Shows the setup window and does what the user picks. `first_run` means no engine is
    /// running yet and a portable choice should start one.
    pub fn run(first_run: bool) -> Result<(), String> {
        let existing = install::existing_install().map(|scope| {
            format!("Grid is already installed {} on this PC. Installing again replaces it with this copy.", scope_text(scope))
        });
        let shown: fn() = || grid_windows::use_app_icon(grid_windows::SETUP_TITLE);
        match grid_settings::ask_setup(existing, Some(shown))? {
            None => Ok(()),
            Some(SetupChoice::Portable) => {
                if first_run {
                    // Creates grid.json next to the exe, so the next start is a normal one.
                    let _ = grid_core::settings::Settings::load(&super::settings_path());
                    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                    install::launch(&exe, &[])?;
                }
                Ok(())
            }
            Some(SetupChoice::Install { all_users, autostart }) => {
                let scope = if all_users { Scope::AllUsers } else { Scope::CurrentUser };
                let exe = if scope == Scope::AllUsers && !install::can_write_machine() {
                    if !install::run_elevated_and_wait(&format!("--install {}", scope.arg()))? {
                        return Ok(()); // the user said no to Windows' prompt
                    }
                    install::install_dir(scope).ok_or("Could not find the install folder.")?.join("grid.exe")
                } else {
                    install::install_files(scope)?
                };
                install::finish_install(&exe, autostart);
                // A portable Grid may still be running (Install from its settings window).
                install::close_running_engine();
                install::launch(&exe, &[grid_windows::RESTARTED])?;
                grid_windows::show_message(
                    "Grid",
                    &format!(
                        "Grid is installed {} and running. You can find it in the Start menu, and remove \
                         it from Windows Settings → Apps. This downloaded copy is no longer needed.",
                        scope_text(scope)
                    ),
                    false,
                );
                Ok(())
            }
        }
    }

    /// `--install all|user`: the part of installing that needs administrator rights. Run by
    /// `run` through Windows' administrator prompt; the exit code reports success.
    pub fn install_elevated(scope: &str) -> Result<(), String> {
        let scope = Scope::from_arg(scope).ok_or_else(|| format!("Unknown install type \"{scope}\"."))?;
        install::install_files(scope).map(|_| ())
    }

    /// `--uninstall`: from Windows Settings → Apps, or Settings → General. Asks first unless
    /// `confirmed` (the administrator copy started after the user already said yes).
    pub fn uninstall(confirmed: bool) -> Result<(), String> {
        let Some(scope) = install::current_scope() else {
            grid_windows::show_message("Grid", "This copy of Grid isn't installed, so there is nothing to uninstall.", false);
            return Ok(());
        };
        if confirmed {
            // The administrator half, started below: settings are handled by the normal half.
            return install::uninstall(scope);
        }
        let shown: fn() = || grid_windows::use_app_icon(grid_windows::UNINSTALL_TITLE);
        let Some(delete_settings) = grid_settings::ask_uninstall(&super::settings_path(), Some(shown))? else {
            return Ok(());
        };
        if scope == Scope::AllUsers && !install::can_write_machine() {
            if !install::run_elevated_and_wait("--uninstall --confirmed")? {
                return Ok(()); // the user said no to Windows' prompt
            }
        } else {
            install::uninstall(scope)?;
        }
        // Also here, in the user's own process: the administrator copy above may run as another
        // account (an administrator's password at Windows' prompt) and so clear that one's entry.
        let _ = install::set_autostart(false);
        if delete_settings {
            install::delete_settings();
        }
        let kept = if delete_settings { "" } else { " Your settings are kept, in case you install it again." };
        grid_windows::show_message("Grid", &format!("Grid has been uninstalled.{kept}"), false);
        Ok(())
    }

    /// `--apply-update <version>`: run as administrator by an update of an all-users install, to
    /// download that version and swap it in.
    pub fn apply_update(version: &str) -> Result<(), String> {
        grid_windows::update::apply_update(version)
    }

    /// For the settings window's Install button: the setup flow, in its own process.
    pub fn start_install() {
        if let Ok(exe) = std::env::current_exe() {
            let _ = install::launch(&exe, &["--setup"]);
        }
    }

    pub fn start_uninstall() {
        if let Ok(exe) = std::env::current_exe() {
            let _ = install::launch(&exe, &["--uninstall"]);
        }
    }
}

#[cfg(not(windows))]
mod setup {
    pub fn run(_first_run: bool) -> Result<(), String> {
        Err("Installing isn't supported on this platform yet.".into())
    }
    pub fn install_elevated(_scope: &str) -> Result<(), String> {
        run(false)
    }
    pub fn uninstall(_confirmed: bool) -> Result<(), String> {
        run(false)
    }
    pub fn apply_update(_version: &str) -> Result<(), String> {
        run(false)
    }
}

fn settings(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let platform = grid_settings::Platform {
        installation: Some(grid_settings::Installation {
            status: setup::status,
            installed: || grid_windows::install::current_scope().is_some(),
            install: setup::start_install,
            uninstall: setup::start_uninstall,
            autostart: grid_windows::install::autostart,
            set_autostart: grid_windows::install::set_autostart,
        }),
        admin: Some(grid_settings::Admin {
            engine_elevated: grid_windows::engine_elevated,
            restart_engine_elevated: grid_windows::restart_engine_elevated,
        }),
        capture: Some(capture_in_helper),
        capture_hotkey: Some(capture_hotkey_in_helper),
        hotkey_available: Some(grid_windows::hotkey_available),
        settings_saved: Some(grid_windows::notify_settings_changed),
        os: Some(grid_settings::OsSettings {
            drag_full_windows: grid_windows::drag_full_windows,
            set_drag_full_windows: grid_windows::set_drag_full_windows,
        }),
        window_shown: Some(|| {
            grid_windows::use_app_icon(grid_windows::SETTINGS_TITLE);
            // Same as min-width / min-height in settings.slint.
            grid_windows::enforce_min_size(grid_windows::SETTINGS_TITLE, 520, 440);
        }),
        monitors: Some(|| {
            grid_windows::monitors()
                .into_iter()
                .map(|m| grid_settings::Monitor { id: m.id, label: m.label, bounds: m.bounds })
                .collect()
        }),
    };
    #[cfg(not(windows))]
    let platform = grid_settings::Platform::default();
    grid_settings::run(path, platform)
}

/// `--capture` mode: waits for one key or mouse button and prints its name (nothing if
/// cancelled). The settings window runs this as a helper process; see `grid_windows::capture_input`.
#[cfg(windows)]
fn capture() -> Result<(), String> {
    if let Some(input) = grid_windows::capture_input() {
        println!("{input}");
    }
    Ok(())
}

#[cfg(not(windows))]
fn capture() -> Result<(), String> {
    Err("Capturing inputs isn't supported on this platform yet.".into())
}

/// `--capture-hotkey` mode: like `--capture`, for a key combination such as `Ctrl+Alt+Z`.
#[cfg(windows)]
fn capture_hotkey() -> Result<(), String> {
    if let Some(hotkey) = grid_windows::capture_hotkey() {
        println!("{hotkey}");
    }
    Ok(())
}

#[cfg(not(windows))]
fn capture_hotkey() -> Result<(), String> {
    Err("Capturing hotkeys isn't supported on this platform yet.".into())
}

/// Runs `grid.exe --capture` without a window and passes its answer to `on_captured`.
#[cfg(windows)]
fn capture_in_helper(on_captured: Box<dyn FnOnce(Option<grid_core::settings::Input>) + Send>) {
    run_helper("--capture", on_captured);
}

#[cfg(windows)]
fn capture_hotkey_in_helper(on_captured: Box<dyn FnOnce(Option<grid_core::settings::Hotkey>) + Send>) {
    run_helper("--capture-hotkey", on_captured);
}

/// Runs `grid.exe <flag>` without a window on another thread, and passes what it prints
/// (parsed) to `on_done`.
#[cfg(windows)]
fn run_helper<T: std::str::FromStr + 'static>(flag: &'static str, on_done: Box<dyn FnOnce(Option<T>) + Send>) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::thread::spawn(move || {
        let output = std::env::current_exe()
            .and_then(|exe| {
                std::process::Command::new(exe)
                    .arg(flag)
                    // So the helper stops waiting if this window closes first.
                    .env(grid_windows::CAPTURE_PARENT_ENV, std::process::id().to_string())
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
            });
        let value = output.ok().and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse().ok());
        on_done(value);
    });
}

/// `--admin-prompt` mode: asks whether to restart Grid as administrator and prints `yes` if so.
/// The engine runs this after the user drags a window Grid isn't allowed to move.
#[cfg(windows)]
fn admin_prompt(path: &Path) -> Result<(), String> {
    let shown: fn() = || grid_windows::use_app_icon(grid_windows::ADMIN_PROMPT_TITLE);
    let (yes, mute) = grid_settings::ask_admin_restart(path, Some(shown))?;
    if yes {
        println!("yes");
    }
    if mute {
        println!("mute");
    }
    Ok(())
}

#[cfg(not(windows))]
fn admin_prompt(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn engine(path: &Path) -> Result<(), String> {
    let restarted = std::env::args().any(|a| a == grid_windows::RESTARTED);
    grid_windows::install::remove_old_exe();
    if grid_windows::install::remove_portable_autostart() {
        grid_windows::log::write("Removed start with Windows for this portable copy (only installed copies can)");
    }
    if grid_windows::install::is_first_run() && !restarted {
        return setup::run(true);
    }
    println!("Grid {} is running. Settings: {}", env!("CARGO_PKG_VERSION"), path.display());
    grid_windows::run(path.to_owned(), restarted)
}

#[cfg(not(windows))]
fn engine(_path: &Path) -> Result<(), String> {
    Err("Grid currently runs on Windows only.".into())
}

fn report_error(message: &str) {
    eprintln!("{message}");
    #[cfg(windows)]
    grid_windows::show_message("Grid", message, true);
}
