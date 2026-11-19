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
    // Not as administrator from a folder the user can change (see `elevated_outside_program_files`):
    // Grid restarts without administrator rights, and that Grid installs the update.
    if elevated_outside_program_files() {
        let restart = crate::ask_yes_no(
            "Grid update",
            &format!(
                "Grid {} is available (you have {VERSION}). To install it, Grid restarts without \
                 administrator rights first. Restart and update now?\n\nWhat's new: {}",
                release.version, release.page_url
            ),
        );
        if restart {
            restart_unelevated_to_update(engine);
        }
        return;
    }
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

/// Left next to the settings by `restart_unelevated_to_update`: the next Grid to start installs the
/// latest update straight away (the user already agreed). Only ever leads to installing a signed
/// release, so it doesn't matter who can create it.
pub const UPDATE_PENDING: &str = "grid-update-pending";

/// Restarts Grid without administrator rights to install an update: leaves `UPDATE_PENDING`,
/// starts grid.exe through Explorer (so as the user, see `open_unelevated`) and exits. The new
/// Grid waits for this one to close, then installs the update.
fn restart_unelevated_to_update(engine: usize) {
    let marker = install::settings_path().with_file_name(UPDATE_PENDING);
    let started = crate::safe_file::write(&marker, b"1").is_ok()
        && std::env::current_exe().is_ok_and(|exe| crate::ui::open_unelevated(&exe.to_string_lossy()));
    if started {
        unsafe { PostMessageW(engine as _, RegisterWindowMessageW(wide(install::EXIT_MESSAGE).as_ptr()), 0, 0) };
    } else {
        let _ = std::fs::remove_file(&marker);
        crate::show_message("Grid", "Grid couldn't restart itself. Exit Grid, start it again normally, and check for updates.", true);
    }
}

/// Whether Grid runs as administrator from a folder the user can change (portable, or installed
/// just for them). Updating then renames and deletes files there with administrator rights, which
/// a program running as the user could redirect onto other files (e.g. to delete any file). So
/// Grid doesn't update itself that way; installs for all users (Program Files, which only
/// administrators can change) update as usual.
fn elevated_outside_program_files() -> bool {
    crate::elevation::is_self_elevated() && install::current_scope() != Some(install::Scope::AllUsers)
}

/// The latest release on GitHub, if it is newer than this build.
fn newer_release() -> Result<Option<Release>, String> {
    #[cfg(debug_assertions)]
    if let Some(release) = test_release() {
        return Ok(Some(release));
    }
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
    if elevated_outside_program_files() {
        return Err("Grid is running as administrator from a folder you can change; start it normally to update.".into());
    }
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
    let bytes = download(release)?;
    // A complete Windows program: the size GitHub listed, starting with the "MZ" exe header.
    if bytes.len() as u64 != release.size || !bytes.starts_with(b"MZ") {
        return Err("the download was incomplete or not a program.".into());
    }
    // Signed with Grid's release key, together with the version (see `signature` and
    // `signed_data`): checked before anything is swapped in.
    let url = release.signature_url.as_deref().ok_or("this release isn't signed, so Grid won't install it.")?;
    let signature = fetch(url, "application/octet-stream")?;
    let signed = grid_core::update::signed_data(&release.version, &bytes);
    if !crate::signature::verify(&crate::signature::RELEASE_KEY, &signed, &signature) {
        return Err("the download's signature doesn't match Grid's release key, so it wasn't installed.".into());
    }
    let staged = target.with_file_name(install::NEW_EXE);
    std::fs::write(&staged, &bytes).map_err(|e| format!("couldn't save the download: {e}"))?;
    let result = install::replace_exe(&staged, target);
    let _ = std::fs::remove_file(&staged);
    result
}

/// The release's `grid.exe`.
fn download(release: &Release) -> Result<Vec<u8>, String> {
    fetch(&release.download_url, "application/octet-stream")
}

/// Downloads `url` (in debug builds, also a local test file, see `test_release`).
fn fetch(url: &str, accept: &str) -> Result<Vec<u8>, String> {
    #[cfg(debug_assertions)]
    if let Some(path) = url.strip_prefix(TEST_PREFIX) {
        return std::fs::read(path).map_err(|e| format!("couldn't read {path}: {e}"));
    }
    let (status, bytes) = get(url, accept)?;
    if status != 200 {
        return Err(format!("the download failed (status {status})."));
    }
    Ok(bytes)
}

/// Debug builds only: with `GRID_TEST_UPDATE` set to the path of a `grid.exe`, "Check for
/// updates" offers that file as a new version, so updating can be tried without a release. Its
/// signature is read from the same path plus `.sig` (`tools/sign-release.ps1` makes one).
#[cfg(debug_assertions)]
const TEST_PREFIX: &str = "test-file:";

#[cfg(debug_assertions)]
fn test_release() -> Option<Release> {
    let path = std::env::var("GRID_TEST_UPDATE").ok()?;
    let size = std::fs::metadata(&path).ok()?.len();
    Some(Release {
        version: "test".into(),
        download_url: format!("{TEST_PREFIX}{path}"),
        size,
        signature_url: Some(format!("{TEST_PREFIX}{path}.sig")),
        page_url: path,
    })
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
