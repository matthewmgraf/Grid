//! A small log of what Grid does: starting, exiting, errors, and freezes found by the watchdog.
//! Never input: no keys, buttons or window titles.
//!
//! Written to `grid.log` next to `grid.json` (so portable copies keep it in their folder). When
//! it passes `MAX_BYTES` it is renamed to `grid.log.old`, replacing the previous one.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

const MAX_BYTES: u64 = 256 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();

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
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
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
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}
