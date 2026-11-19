// Release builds have no console window; Grid lives in the tray.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

fn main() {
    // Before anything else can load a DLL (see the function).
    #[cfg(windows)]
    grid_windows::load_dlls_from_system32();
    let path = settings_path();
    // grid.log next to grid.json: starts, exits, errors and freezes only (never input).
    #[cfg(windows)]
    grid_windows::log::init(&path);
    // grid.json and positions.json, written with checks while running as administrator.
    #[cfg(windows)]
    grid_core::store::set_writer(grid_windows::write_file);
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
    } else if let Some(at) = args.iter().position(|a| a == "--install") {
        let scope = args.get(at + 1).cloned().unwrap_or_default();
        setup::install_elevated(&scope, args.get(at + 2..).unwrap_or_default())
    } else if let Some(at) = args.iter().position(|a| a == "--startup") {
        setup::machine_startup(args.get(at + 1..).unwrap_or_default())
    } else if has_arg("--uninstall") {
        setup::uninstall(has_arg("--confirmed"), arg_after("--user"))
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

/// The glass look of Grid's windows: light or dark like Windows, with a matching title bar.
#[cfg(windows)]
fn look() -> grid_settings::Look {
    grid_settings::Look { system_dark: Some(grid_windows::system_dark), set_dark: Some(grid_windows::set_dark_window) }
}

#[cfg(not(windows))]
fn look() -> grid_settings::Look {
    grid_settings::Look::default()
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
        match grid_settings::ask_setup(existing, Some(shown), super::look(), grid_windows::user_is_admin(), install::installed_startup())? {
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
            Some(SetupChoice::Install { all_users, startup }) => {
                let scope = if all_users { Scope::AllUsers } else { Scope::CurrentUser };
                let (entries, user) = (startup.entries(grid_windows::user_is_admin()), install::current_user());
                // A running Grid is closed by `install_files`, just before its exe is replaced (not
                // earlier: if the user declines Windows' prompt, it keeps running).
                // With an administrator start chosen, the administrator step below starts Grid,
                // so it runs as administrator straight away (not only from the next sign-in).
                let mut started = false;
                let exe = if scope == Scope::AllUsers && !install::can_write_machine() {
                    // The start options that need administrator rights go with the install, so
                    // Windows asks only once.
                    let bit = |on: bool| if on { "1" } else { "0" };
                    let args = format!(
                        "--install {} {} {} {} \"{user}\"",
                        scope.arg(),
                        bit(entries.all_users),
                        bit(entries.user_task),
                        bit(entries.admins_task)
                    );
                    if !install::run_elevated_and_wait(&args)? {
                        return Ok(()); // the user said no to Windows' prompt
                    }
                    started = startup.admin;
                    install::install_dir(scope).ok_or("Could not find the install folder.")?.join("grid.exe")
                } else {
                    let exe = install::install_files(scope)?;
                    if scope == Scope::AllUsers {
                        install::set_machine_startup_for(&exe, entries, &user)?;
                    }
                    exe
                };
                install::finish_install(&exe, entries);
                // If the administrator step couldn't start Grid, start it normally.
                if !(started && install::wait_for_engine(std::time::Duration::from_secs(5))) {
                    install::launch(&exe, &[grid_windows::RESTARTED])?;
                }
                let admin = if startup.admin { " It runs as administrator, now and at every sign-in." } else { "" };
                grid_windows::show_message(
                    "Grid",
                    &format!(
                        "Grid is installed {} and running.{admin} You can find it in the Start menu, and \
                         remove it from Windows Settings → Apps. This downloaded copy is no longer needed.",
                        scope_text(scope)
                    ),
                    false,
                );
                Ok(())
            }
        }
    }

    /// `--install all|user [<all users> <user task> <administrators task> <user>]`: the part of
    /// installing that needs administrator rights, and for all users, the start options that do
    /// too (each 1 or 0; see `machine_startup`). Run by `run` through Windows' administrator
    /// prompt; the exit code reports success.
    pub fn install_elevated(scope: &str, startup: &[String]) -> Result<(), String> {
        let scope = Scope::from_arg(scope).ok_or_else(|| format!("Unknown install type \"{scope}\"."))?;
        let exe = install::install_files(scope)?;
        if scope == Scope::AllUsers && startup.len() == 4 {
            let (entries, user) = startup_args(startup)?;
            install::set_machine_startup_for(&exe, entries, &user)?;
            // Started from here, it runs as administrator (see `run`). Not an error if it can't:
            // Grid is installed, and `run` then starts it normally.
            if (entries.user_task || entries.admins_task)
                && let Err(e) = install::launch(&exe, &[grid_windows::RESTARTED])
            {
                grid_windows::log::write(&format!("Could not start Grid as administrator after installing: {e}"));
            }
        }
        Ok(())
    }

    /// Reads `<all users> <user task> <administrators task> <user>` (each 1 or 0).
    fn startup_args(args: &[String]) -> Result<(grid_core::startup::Entries, String), String> {
        let bit = |i: usize| args.get(i).is_some_and(|a| a == "1");
        let user = args.get(3).ok_or("Missing the user for the start options.")?;
        let entries = grid_core::startup::Entries { all_users: bit(0), user_task: bit(1), admins_task: bit(2), ..Default::default() };
        Ok((entries, user.clone()))
    }

    /// `--uninstall`: from Windows Settings → Apps, or Settings → General. Asks first unless
    /// `confirmed` (the administrator copy started after the user already said yes).
    pub fn uninstall(confirmed: bool, user: Option<String>) -> Result<(), String> {
        let Some(scope) = install::current_scope() else {
            grid_windows::show_message("Grid", "This copy of Grid isn't installed, so there is nothing to uninstall.", false);
            return Ok(());
        };
        if confirmed {
            // The administrator half, started below: settings are handled by the normal half.
            return install::uninstall(scope, &user.unwrap_or_else(install::current_user));
        }
        let shown: fn() = || grid_windows::use_app_icon(grid_windows::UNINSTALL_TITLE);
        let Some(delete_settings) = grid_settings::ask_uninstall(&super::settings_path(), Some(shown), super::look())? else {
            return Ok(());
        };
        if scope == Scope::AllUsers && !install::can_write_machine() {
            let args = format!("--uninstall --confirmed --user \"{}\"", install::current_user());
            if !install::run_elevated_and_wait(&args)? {
                return Ok(()); // the user said no to Windows' prompt
            }
        } else {
            install::uninstall(scope, &install::current_user())?;
        }
        // Also here, in the user's own process: the administrator copy above may run as another
        // account (an administrator's password at Windows' prompt) and so clear that one's entry.
        install::remove_user_startup();
        if delete_settings {
            install::delete_settings();
        }
        let kept = if delete_settings { "" } else { " Your settings are kept, in case you install it again." };
        grid_windows::show_message("Grid", &format!("Grid has been uninstalled.{kept}"), false);
        Ok(())
    }

    /// `--startup <all users> <user task> <administrators task> <user>` (each 1 or 0): the
    /// administrator half of Settings → General → start with Windows (see `install::set_startup`).
    pub fn machine_startup(args: &[String]) -> Result<(), String> {
        let (entries, user) = startup_args(args)?;
        install::set_machine_startup(entries, &user)
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
    pub fn install_elevated(_scope: &str, _startup: &[String]) -> Result<(), String> {
        run(false)
    }
    pub fn machine_startup(_args: &[String]) -> Result<(), String> {
        run(false)
    }
    pub fn uninstall(_confirmed: bool, _user: Option<String>) -> Result<(), String> {
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
            startup: grid_windows::install::startup,
            set_startup: grid_windows::install::set_startup,
            machine_startup: grid_windows::install::machine_startup_available,
            user_is_admin: grid_windows::user_is_admin,
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
            grid_windows::enforce_min_size(grid_windows::SETTINGS_TITLE, 680, 460);
        }),
        monitors: Some(|| {
            grid_windows::monitors()
                .into_iter()
                .map(|m| grid_settings::Monitor { id: m.id, label: m.label, bounds: m.bounds })
                .collect()
        }),
        open_url: Some(grid_windows::open_url),
        look: look(),
    };
    #[cfg(not(windows))]
    let platform = grid_settings::Platform::default();
    grid_settings::run(path, platform)
}

/// `--capture` mode: waits for one key or mouse button and prints its name (nothing if
/// canceled). The settings window runs this as a helper process; see `grid_windows::capture_input`.
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
    let shown: fn() = || {
        grid_windows::use_app_icon(grid_windows::ADMIN_PROMPT_TITLE);
        grid_windows::center_window(grid_windows::ADMIN_PROMPT_TITLE);
    };
    // The tip about starting as administrator only for accounts that can (Settings hides it for
    // others).
    let offered = grid_windows::install::machine_startup_available() && grid_windows::user_is_admin();
    let admin_autostart = offered.then(|| grid_windows::install::startup().admin);
    let (yes, mute) = grid_settings::ask_admin_restart(path, Some(shown), look(), admin_autostart)?;
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
    // Not straight after an update or install (`--restarted`): security software can read a
    // program deleting the exe that started it as malware hiding its tracks. The next normal start
    // tidies it up instead.
    if !restarted {
        grid_windows::install::remove_old_exe();
    }
    if grid_windows::install::remove_portable_autostart() {
        grid_windows::log::write("Removed start with Windows for this portable copy (only installed copies can)");
    }
    if grid_windows::install::is_first_run() && !restarted {
        return setup::run(true);
    }
    // Started at sign-in: this user may have opted out of the all-users start, or an administrator
    // task may start Grid for them too (e.g. with "For all users" on as well).
    let autostart = std::env::args().any(|a| a == grid_windows::install::AUTOSTART);
    if let Some(why) = autostart.then(grid_windows::install::quit_at_sign_in).flatten() {
        grid_windows::log::write(&format!("Not started at sign-in: {why}"));
        return Ok(());
    }
    println!("Grid {} is running. Settings: {}", env!("CARGO_PKG_VERSION"), path.display());
    match grid_windows::run(path.to_owned(), restarted) {
        // Two ways of starting at sign-in both did: no need to say so.
        Err(e) if autostart && e == grid_windows::ALREADY_RUNNING => {
            grid_windows::log::write("Not started at sign-in: Grid is already running");
            Ok(())
        }
        result => result,
    }
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
