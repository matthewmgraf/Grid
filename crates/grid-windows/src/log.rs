//! A small log of what Grid does: starting, exiting, errors, and freezes found by the watchdog.
//! Never input: no keys, buttons or window titles.
//!
//! Written to `grid.log` next to `grid.json` (so portable copies keep it in their folder). When
//! it passes `MAX_BYTES` its contents move to `grid.log.old`, replacing the previous one. Written
//! through `safe_file`, which checks the files first while Grid runs as administrator.

use std::path::PathBuf;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

const MAX_BYTES: u64 = 256 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();
/// Set when moving the log to grid.log.old was refused (see `safe_file`), so it isn't tried again
/// (reading the whole log) for every line this process writes.
static ROTATION_REFUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Where the log goes. Call once at startup; until then nothing is written.
pub fn init(settings_path: &std::path::Path) {
    let _ = PATH.set(settings_path.with_file_name("grid.log"));
    std::panic::set_hook(Box::new(|info| {
        // A panic inside a Windows callback ends the process at once, so record it first.
        write(&format!("Crashed: {info}"));
    }));
}

/// Adds a line with the time, process ID and what this process is (`role`, e.g. "engine").
pub fn write(message: &str) {
    let Some(path) = PATH.get() else { return };
    use std::sync::atomic::Ordering::Relaxed;
    if !ROTATION_REFUSED.load(Relaxed)
        && std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES)
        && crate::safe_file::rotate(path, &path.with_extension("log.old")).is_err()
    {
        ROTATION_REFUSED.store(true, Relaxed);
    }
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} [{}] {}\n",
        t.wYear,
        t.wMonth,
        t.wDay,
        t.wHour,
        t.wMinute,
        t.wSecond,
        t.wMilliseconds,
        std::process::id(),
        message.replace('\n', " | ")
    );
    // Opened per line: lines are rare, and several Grid processes may write at once.
    let _ = crate::safe_file::append(path, line.as_bytes());
}
