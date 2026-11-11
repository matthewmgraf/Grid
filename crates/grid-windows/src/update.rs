//! Checks GitHub Releases for a newer Grid, downloads it and swaps it in (WinHTTP, which is part
//! of Windows, so no extra libraries). Runs on a background thread so the engine keeps going.

use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use grid_core::update::{LATEST_RELEASE_URL, Release, is_newer, parse_latest};
use windows_sys::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW};

use crate::{RESTARTED, install, wide};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Checks for a newer version and, if there is one, installs it (asking first unless `silent`)
/// and restarts Grid. `interactive` (from the tray menu) also reports "up to date" and errors;
/// the startup check stays quiet about those. `engine` is the engine's message window.
pub fn check(interactive: bool, silent: bool, engine: usize) {
    let release = match newer_release() {
        Ok(Some(r)) => r,
        Ok(None) => {
            if interactive {
                crate::show_message("Grid", &format!("You have the latest version of Grid ({VERSION})."), false);
            }
            return;
        }
        Err(e) => {
            if interactive {
                crate::show_message("Grid", &format!("Couldn't check for updates: {e}"), true);
            }
            return;
        }
    };
    let go = (silent && !interactive)
        || crate::ask_yes_no(
            "Grid update",
            &format!(
                "Grid {} is available (you have {VERSION}). Update now? Grid restarts afterwards.\n\nWhat's new: {}",
                release.version, release.page_url
            ),
        );
    if !go {
        return;
    }
    match install_release(&release) {
        Ok(exe) => {
            if install::launch(&exe, &[RESTARTED]).is_ok() {
                // The new Grid waits for this one to exit.
                unsafe { PostMessageW(engine as _, RegisterWindowMessageW(wide(install::EXIT_MESSAGE).as_ptr()), 0, 0) };
            }
        }
        Err(e) => crate::show_message("Grid", &format!("Couldn't install the update: {e}"), true),
    }
}

/// The latest release on GitHub, if it is newer than this build.
fn newer_release() -> Result<Option<Release>, String> {
    let release = latest_release()?;
    Ok(is_newer(&release.version, VERSION).then_some(release))
}

/// The latest published release on GitHub.
fn latest_release() -> Result<Release, String> {
    let (status, body) = get(LATEST_RELEASE_URL, "application/vnd.github+json")?;
    match status {
        200 => {}
        // GitHub answers 404 when nothing is published yet (or the repository is private).
        404 => return Err("no published release was found on GitHub.".into()),
        _ => return Err(format!("GitHub answered with status {status}.")),
    }
    parse_latest(&String::from_utf8_lossy(&body))
}

/// Puts `release` in place of this exe, asking Windows for administrator rights when the folder
/// needs them (an install for all users, in Program Files). Returns the exe to start.
fn install_release(release: &Release) -> Result<PathBuf, String> {
    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    if install::current_scope() == Some(install::Scope::AllUsers) && !crate::elevation::is_self_elevated() {
        // The administrator copy downloads and checks the file itself, rather than taking one
        // this (normal) process saved somewhere other programs could change it first.
        return match install::run_elevated_and_wait(&format!("--apply-update {}", release.version)) {
            Ok(true) => Ok(target),
            Ok(false) => Err("administrator rights are needed to update Grid for all users.".into()),
            Err(e) => Err(e),
        };
    }
    download_and_replace(release, &target)?;
    Ok(target)
}

/// Downloads `release`, checks it, stages it next to `target` (in Grid's own folder, not a
/// shared temporary one) and swaps it in.
fn download_and_replace(release: &Release, target: &Path) -> Result<(), String> {
    let (status, bytes) = get(&release.download_url, "application/octet-stream")?;
    if status != 200 {
        return Err(format!("the download failed (status {status})."));
    }
    // A complete Windows program: the size GitHub listed, starting with the "MZ" exe header.
    if bytes.len() as u64 != release.size || !bytes.starts_with(b"MZ") {
        return Err("the download was incomplete or not a program.".into());
    }
    let staged = target.with_file_name(install::NEW_EXE);
    std::fs::write(&staged, &bytes).map_err(|e| format!("couldn't save the download: {e}"))?;
    let result = install::replace_exe(&staged, target);
    let _ = std::fs::remove_file(&staged);
    result
}

/// `--apply-update <version>`, run as administrator for an all-users install: downloads the
/// latest release, if it is still `version` (the one the user agreed to), and swaps it in.
pub fn apply_update(version: &str) -> Result<(), String> {
    let release = latest_release()?;
    if release.version != version {
        return Err(format!("GitHub now has version {} instead of {version}. Try again.", release.version));
    }
    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    download_and_replace(&release, &target)
}

/// An HTTPS GET. Follows redirects (GitHub sends downloads elsewhere). Returns the status and body.
fn get(url: &str, accept: &str) -> Result<(u32, Vec<u8>), String> {
    let rest = url.strip_prefix("https://").ok_or("only https addresses are supported.")?;
    let (host, path) = rest.split_once('/').map_or((rest, "/".to_string()), |(h, p)| (h, format!("/{p}")));
    let failed = |what: &str| format!("couldn't connect to {host} ({what}).");
    struct Handle(*mut std::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }
    unsafe {
        let agent = wide(&format!("Grid/{VERSION}"));
        let session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            return Err(failed("open"));
        }
        let connection = Handle(WinHttpConnect(session.0, wide(host).as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if connection.0.is_null() {
            return Err(failed("connect"));
        }
        let request = Handle(WinHttpOpenRequest(
            connection.0,
            wide("GET").as_ptr(),
            wide(&path).as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(failed("request"));
        }
        let headers = wide(&format!("Accept: {accept}"));
        // -1: the headers string is null-terminated.
        if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, null_mut()) == 0
        {
            return Err(failed("no answer"));
        }
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut len,
            null_mut(),
        );
        let mut body = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read) == 0 {
                return Err(failed("download interrupted"));
            }
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
        }
        Ok((status, body))
    }
}

#[cfg(test)]
mod tests {
    /// Talks to GitHub for real. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn fetches_from_github() {
        let (status, body) = super::get(super::LATEST_RELEASE_URL, "application/vnd.github+json").unwrap();
        println!("status {status}, {} bytes", body.len());
        assert!(status == 200 || status == 404);
    }
}
