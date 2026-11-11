//! Installing and uninstalling Grid. The same `grid.exe` runs portably (settings next to it) or
//! installed:
//! - for all users: `Program Files\Grid` (needs administrator rights), or
//! - for the current user: `%LOCALAPPDATA%\Programs\Grid` (no administrator rights needed).
//!
//! Installing copies the exe, adds a Start menu shortcut and an "Apps & features" entry (so it
//! can be uninstalled from Windows Settings), and optionally starts Grid with Windows. Installed
//! copies keep their settings in `%APPDATA%\Grid`.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows_sys::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
    RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegOpenKeyExW,
    RegSetValueExW,
};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW, ShellLink};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, RegisterWindowMessageW, SW_SHOWNORMAL};
use windows_sys::core::GUID;

use crate::{MESSAGE_CLASS, wide};

/// Who Grid is installed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `Program Files`, for everyone on the PC. Needs administrator rights.
    AllUsers,
    /// The current user's own programs folder. No administrator rights needed.
    CurrentUser,
}

impl Scope {
    /// The command-line word for this scope (`--install all`).
    pub fn arg(self) -> &'static str {
        match self {
            Scope::AllUsers => "all",
            Scope::CurrentUser => "user",
        }
    }

    pub fn from_arg(s: &str) -> Option<Scope> {
        match s {
            "all" => Some(Scope::AllUsers),
            "user" => Some(Scope::CurrentUser),
            _ => None,
        }
    }

    fn root(self) -> HKEY {
        match self {
            Scope::AllUsers => HKEY_LOCAL_MACHINE,
            Scope::CurrentUser => HKEY_CURRENT_USER,
        }
    }
}

const EXE: &str = "grid.exe";
/// A previous exe, renamed so a new one can take its place while it is still running.
const OLD_EXE: &str = "grid.exe.old";
/// A downloaded update, staged next to the exe before it takes its place (see `update.rs`).
pub(crate) const NEW_EXE: &str = "grid.exe.new";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Grid";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Grid";
/// Registered message that asks a running engine to exit (to replace or remove its exe).
pub(crate) const EXIT_MESSAGE: &str = "Grid.Exit";

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Folder Grid is installed to for `scope`.
pub fn install_dir(scope: Scope) -> Option<PathBuf> {
    match scope {
        Scope::AllUsers => env_dir("ProgramFiles").map(|d| d.join("Grid")),
        Scope::CurrentUser => env_dir("LOCALAPPDATA").map(|d| d.join("Programs").join("Grid")),
    }
}

/// Where an installed Grid keeps its settings: `%APPDATA%\Grid`.
pub fn data_dir() -> Option<PathBuf> {
    env_dir("APPDATA").map(|d| d.join("Grid"))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().trim_end_matches('\\').eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches('\\'))
}

/// If this exe is an installed copy, who it is installed for.
pub fn current_scope() -> Option<Scope> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [Scope::AllUsers, Scope::CurrentUser]
        .into_iter()
        .find(|&s| install_dir(s).is_some_and(|d| same_dir(&d, dir)))
}

/// An installed copy of Grid other than this exe, if there is one.
pub fn existing_install() -> Option<Scope> {
    [Scope::AllUsers, Scope::CurrentUser]
        .into_iter()
        .find(|&s| current_scope() != Some(s) && install_dir(s).is_some_and(|d| d.join(EXE).is_file()))
}

/// The settings file: in `%APPDATA%\Grid` for an installed copy, otherwise next to the exe.
pub fn settings_path() -> PathBuf {
    let next_to_exe = || {
        std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.join("grid.json")))
    };
    if current_scope().is_some()
        && let Some(dir) = data_dir()
    {
        let _ = std::fs::create_dir_all(&dir);
        return dir.join("grid.json");
    }
    next_to_exe().unwrap_or_else(|| PathBuf::from("grid.json"))
}

/// True the first time a downloaded (not installed) Grid runs: there are no portable settings
/// next to it yet. That's when Grid offers to install itself.
pub fn is_first_run() -> bool {
    current_scope().is_none()
        && std::env::current_exe().ok().and_then(|e| e.parent().map(|d| !d.join("grid.json").exists())).unwrap_or(false)
}

/// Deletes an exe left over from replacing a running one (install over it, or an update).
pub fn remove_old_exe() {
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let _ = std::fs::remove_file(dir.join(OLD_EXE));
    }
}

/// Puts `new_exe` in place of `target` (an exe that may be running: Windows lets a running exe
/// be renamed, just not overwritten).
pub fn replace_exe(new_exe: &Path, target: &Path) -> Result<(), String> {
    let old = target.with_file_name(OLD_EXE);
    let _ = std::fs::remove_file(&old);
    if target.exists() {
        std::fs::rename(target, &old).map_err(|e| format!("Could not replace {}: {e}", target.display()))?;
    }
    if let Err(e) = std::fs::copy(new_exe, target) {
        // Put the previous exe back so Grid still starts.
        let _ = std::fs::rename(&old, target);
        return Err(format!("Could not copy Grid to {}: {e}", target.display()));
    }
    Ok(())
}

/// Asks any running Grid engine to exit and waits (up to a few seconds) until it has.
pub fn close_running_engine() {
    unsafe {
        let class = wide(MESSAGE_CLASS);
        let engine = FindWindowW(class.as_ptr(), null());
        if engine.is_null() {
            return;
        }
        PostMessageW(engine, RegisterWindowMessageW(wide(EXIT_MESSAGE).as_ptr()), 0, 0);
        let start = Instant::now();
        while !FindWindowW(class.as_ptr(), null()).is_null() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(100));
        }
        // Its window closes just before the process ends.
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Installs the files and system entries for `scope` (all users needs administrator rights):
/// the exe, the Start menu shortcut and the Apps & features entry. Returns the installed exe.
pub fn install_files(scope: Scope) -> Result<PathBuf, String> {
    let dir = install_dir(scope).ok_or("Could not find the install folder.")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let source = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = dir.join(EXE);
    if !same_dir(&source, &target) {
        close_running_engine();
        replace_exe(&source, &target)?;
    }
    if let Some(link) = shortcut_path(scope) {
        // Not fatal: Grid works without it.
        let _ = create_shortcut(&link, &target);
    }
    register_uninstall(scope, &dir, &target)?;
    Ok(target)
}

/// The per-user parts of installing, done without administrator rights: start with Windows,
/// and settings carried over from the portable copy that did the installing.
pub fn finish_install(installed_exe: &Path, autostart: bool) {
    let _ = set_autostart_for(installed_exe, autostart);
    let portable = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    if let (Some(from), Some(to)) = (portable, data_dir()) {
        let _ = std::fs::create_dir_all(&to);
        for name in ["grid.json", "positions.json"] {
            if from.join(name).is_file() && !to.join(name).exists() {
                let _ = std::fs::copy(from.join(name), to.join(name));
            }
        }
    }
}

/// Starts `exe` with `args` (not elevated, when this process isn't).
pub fn launch(exe: &Path, args: &[&str]) -> Result<(), String> {
    std::process::Command::new(exe)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start {}: {e}", exe.display()))
}

/// Runs this exe again as administrator with `args` (Windows asks the user first), waits for it
/// and returns whether it succeeded. `Ok(false)` if the user declined.
pub fn run_elevated_and_wait(args: &str) -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (verb, file, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(args));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: params.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { ShellExecuteExW(&mut info) } == 0 || info.hProcess.is_null() {
        return Ok(false);
    }
    let mut code = 1u32;
    unsafe {
        if WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0 {
            GetExitCodeProcess(info.hProcess, &mut code);
        }
        CloseHandle(info.hProcess);
    }
    if code == 0 { Ok(true) } else { Err("the step that needed administrator rights didn't finish.".into()) }
}

/// Removes an installed Grid: the Start menu shortcut, the Apps & features entry, starting with
/// Windows, and the program folder. Settings in `%APPDATA%\Grid` are kept (see `delete_settings`).
pub fn uninstall(scope: Scope) -> Result<(), String> {
    close_running_engine();
    let _ = set_autostart_for(Path::new(""), false);
    if let Some(link) = shortcut_path(scope) {
        let _ = std::fs::remove_file(link);
    }
    let key = wide(UNINSTALL_KEY);
    unsafe { RegDeleteTreeW(scope.root(), key.as_ptr()) };
    let dir = install_dir(scope).ok_or("Could not find the install folder.")?;
    // This exe (and the copy that asked, which shows a message afterwards) runs from that folder,
    // and a running exe can't be deleted. So a helper keeps trying, once a second for up to five
    // minutes, until it is gone; then it removes the leftover old exe and the folder if empty.
    // Only Grid's own files are deleted.
    let exe = format!("{}\\{EXE}", dir.display());
    let script = format!(
        "for /L %i in (1,1,300) do @(if exist \"{exe}\" (ping -n 2 127.0.0.1 >nul & del /f /q \"{exe}\" 2>nul)) \
         & del /f /q \"{0}\\{OLD_EXE}\" \"{0}\\{NEW_EXE}\" 2>nul & rmdir \"{0}\"",
        dir.display()
    );
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("cmd.exe")
        .raw_arg(format!("/c {script}"))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Could not remove {}: {e}", dir.display()))?;
    Ok(())
}

/// Deletes an installed Grid's settings and saved window positions from `%APPDATA%\Grid`, then
/// that folder if nothing else is in it.
pub fn delete_settings() {
    if let Some(dir) = data_dir() {
        for name in ["grid.json", "positions.json"] {
            let _ = std::fs::remove_file(dir.join(name));
        }
        let _ = std::fs::remove_dir(dir);
    }
}

/// Whether Grid starts with Windows (for this user). Always false for a portable copy, which
/// can't (see `set_autostart`).
pub fn autostart() -> bool {
    current_scope().is_some() && read_string(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE).is_some()
}

/// Turns starting with Windows on or off, for this exe. Only for installed copies: a portable
/// copy can't start with Windows (users are pointed to installing instead).
pub fn set_autostart(on: bool) -> Result<(), String> {
    if on && current_scope().is_none() {
        return Err("Install Grid to start it with Windows.".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    set_autostart_for(&exe, on)
}

/// Removes a start-with-Windows entry that points at this exe when it isn't installed (left
/// from older versions, which allowed it for portable copies). Returns true if one was removed.
pub fn remove_portable_autostart() -> bool {
    if current_scope().is_some() {
        return false;
    }
    let (Ok(exe), Some(entry)) = (std::env::current_exe(), read_string(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE)) else {
        return false;
    };
    if !same_dir(Path::new(entry.trim_matches('"')), &exe) {
        return false;
    }
    set_autostart_for(&exe, false).is_ok()
}

fn set_autostart_for(exe: &Path, on: bool) -> Result<(), String> {
    if on {
        write_values(HKEY_CURRENT_USER, RUN_KEY, &[(RUN_VALUE, Value::Str(format!("\"{}\"", exe.display())))])
    } else {
        let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        Ok(())
    }
}

/// The Apps & features entry.
fn register_uninstall(scope: Scope, dir: &Path, exe: &Path) -> Result<(), String> {
    let exe_text = exe.display().to_string();
    let size_kb = std::fs::metadata(exe).map_or(0, |m| (m.len() / 1024) as u32);
    write_values(
        scope.root(),
        UNINSTALL_KEY,
        &[
            ("DisplayName", Value::Str("Grid".into())),
            ("DisplayVersion", Value::Str(env!("CARGO_PKG_VERSION").into())),
            ("Publisher", Value::Str("Grid".into())),
            ("DisplayIcon", Value::Str(format!("{exe_text},0"))),
            ("InstallLocation", Value::Str(dir.display().to_string())),
            ("UninstallString", Value::Str(format!("\"{exe_text}\" --uninstall"))),
            ("EstimatedSize", Value::Dword(size_kb)),
            ("NoModify", Value::Dword(1)),
            ("NoRepair", Value::Dword(1)),
        ],
    )
    .map_err(|e| format!("Could not add Grid to Apps & features: {e}"))
}

fn shortcut_path(scope: Scope) -> Option<PathBuf> {
    let base = match scope {
        Scope::AllUsers => env_dir("ProgramData")?,
        Scope::CurrentUser => env_dir("APPDATA")?,
    };
    Some(base.join(r"Microsoft\Windows\Start Menu\Programs\Grid.lnk"))
}

enum Value {
    Str(String),
    Dword(u32),
}

fn write_values(root: HKEY, path: &str, values: &[(&str, Value)]) -> Result<(), String> {
    let mut key: HKEY = null_mut();
    let path = wide(path);
    let status = unsafe {
        RegCreateKeyExW(root, path.as_ptr(), 0, null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, null(), &mut key, null_mut())
    };
    if status != 0 {
        return Err(format!("registry error {status}"));
    }
    let mut result = Ok(());
    for (name, value) in values {
        let name = wide(name);
        let status = unsafe {
            match value {
                Value::Str(s) => {
                    let data = wide(s);
                    RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
                }
                Value::Dword(d) => RegSetValueExW(key, name.as_ptr(), 0, REG_DWORD, (d as *const u32).cast(), 4),
            }
        };
        if status != 0 {
            result = Err(format!("registry error {status}"));
        }
    }
    unsafe { RegCloseKey(key) };
    result
}

fn read_string(root: HKEY, path: &str, name: &str) -> Option<String> {
    let (path, name) = (wide(path), wide(name));
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let status =
        unsafe { RegGetValueW(root, path.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr().cast(), &mut len) };
    (status == 0).then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
}

/// Whether this process can write the all-users install (used to decide whether to elevate).
pub fn can_write_machine() -> bool {
    let mut key: HKEY = null_mut();
    let path = wide(r"Software\Microsoft\Windows\CurrentVersion\Uninstall");
    let ok = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_SET_VALUE, &mut key) } == 0;
    if ok {
        unsafe { RegCloseKey(key) };
    }
    ok
}

/// Creates a `.lnk` shortcut to `target` with the Shell's `IShellLinkW`. `windows-sys` has no
/// COM wrappers, so the two interfaces' methods are called through their vtables directly.
fn create_shortcut(link: &Path, target: &Path) -> Result<(), String> {
    #[repr(C)]
    struct ShellLinkVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        // GetPath, GetIDList, SetIDList, GetDescription.
        _unused1: [usize; 4],
        set_description: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        _get_working_directory: usize,
        set_working_directory: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        // GetArguments … Resolve.
        _unused2: [usize; 10],
        set_path: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
    }
    #[repr(C)]
    struct PersistFileVtbl {
        _query_interface: usize,
        _add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        _get_class_id: usize,
        _is_dirty: usize,
        _load: usize,
        save: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32,
    }
    const IID_ISHELLLINKW: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const IID_IPERSISTFILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);

    unsafe {
        CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        let mut link_obj: *mut c_void = null_mut();
        if CoCreateInstance(&ShellLink, null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELLLINKW, &mut link_obj) != 0 {
            return Err("Could not create the Start menu shortcut.".into());
        }
        let vtbl = &**(link_obj as *const *const ShellLinkVtbl);
        let dir = target.parent().unwrap_or(Path::new(""));
        (vtbl.set_path)(link_obj, wide(&target.to_string_lossy()).as_ptr());
        (vtbl.set_working_directory)(link_obj, wide(&dir.to_string_lossy()).as_ptr());
        (vtbl.set_description)(link_obj, wide("Grid window manager").as_ptr());
        let mut file: *mut c_void = null_mut();
        let mut hr = (vtbl.query_interface)(link_obj, &IID_IPERSISTFILE, &mut file);
        if hr == 0 {
            let file_vtbl = &**(file as *const *const PersistFileVtbl);
            hr = (file_vtbl.save)(file, wide(&link.to_string_lossy()).as_ptr(), 1);
            (file_vtbl.release)(file);
        }
        (vtbl.release)(link_obj);
        if hr == 0 { Ok(()) } else { Err("Could not save the Start menu shortcut.".into()) }
    }
}
