//! Writing Grid's own files (log, settings, saved positions) safely while Grid runs as
//! administrator.
//!
//! These files live in a folder the user owns (`%APPDATA%\Grid`, or next to a portable exe), so
//! any program running as that user could swap the folder or a file for a junction or link and
//! redirect a write made with Grid's administrator rights onto a protected file. So, while
//! elevated, Grid opens each file without following a link at its end, checks that the handle
//! is the file it asked for (its real path is the one it asked for, so no junction on the way;
//! it isn't itself a link; it has a single hard link), and only then writes, through that same
//! handle. If a check fails, nothing is written (and a file it created in the wrong place is
//! deleted again). Without administrator rights none of this is needed (the user's own rights
//! apply), and files are written the usual way.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::sync::OnceLock;

use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_NAME_NORMALIZED, FileDispositionInfo,
    GetFileInformationByHandle, GetFinalPathNameByHandleW, GetLongPathNameW, SetFileInformationByHandle, VOLUME_NAME_DOS,
};

/// Whether this process runs as administrator (it doesn't change while it runs).
fn elevated() -> bool {
    static ELEVATED: OnceLock<bool> = OnceLock::new();
    *ELEVATED.get_or_init(crate::elevation::is_self_elevated)
}

/// Replaces `path`'s contents with `data`. Used for grid.json and positions.json (through
/// `grid_core::store`).
pub fn write(path: &Path, data: &[u8]) -> io::Result<()> {
    if !elevated() {
        return std::fs::write(path, data);
    }
    let mut file = open_checked(path)?;
    file.set_len(0)?;
    file.write_all(data)
}

/// Adds `data` to the end of `path`, creating it if needed (the log).
pub fn append(path: &Path, data: &[u8]) -> io::Result<()> {
    if !elevated() {
        return OpenOptions::new().create(true).append(true).open(path)?.write_all(data);
    }
    let mut file = open_checked(path)?;
    file.seek(SeekFrom::End(0))?;
    file.write_all(data)
}

/// Moves `path`'s contents to `to`, replacing it, and leaves `path` empty (the log's rotation).
/// While elevated it copies through checked handles instead of renaming, which would act on
/// whatever the names lead to.
pub fn rotate(path: &Path, to: &Path) -> io::Result<()> {
    if !elevated() {
        return std::fs::rename(path, to);
    }
    let mut from = open_checked(path)?;
    let mut data = Vec::new();
    from.read_to_end(&mut data)?;
    write(to, &data)?;
    from.set_len(0)
}

/// Opens (or creates) `path` for reading and writing, checked as described at the top.
pub(crate) fn open_checked(path: &Path) -> io::Result<File> {
    let dir = path.parent().ok_or_else(|| refused(path, "it has no folder"))?;
    check_dir(dir)?;
    let mut options = OpenOptions::new();
    // `write(true)`: std only creates files with it set; `access_mode` then sets the actual rights.
    options
        .write(true)
        .access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    // Created here or already there: only a file created here may be deleted if it's refused.
    let (file, created) = match options.clone().create_new(true).open(path) {
        Ok(file) => (file, true),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (options.open(path)?, false),
        Err(e) => return Err(e),
    };
    if let Err(e) = check_file(&file, path) {
        if created {
            delete(&file);
        }
        return Err(e);
    }
    Ok(file)
}

/// Checks that folder `dir` is where it says (no junction on the way) and isn't a link.
fn check_dir(dir: &Path) -> io::Result<()> {
    let handle = OpenOptions::new()
        .access_mode(FILE_GENERIC_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(dir)?;
    let info = file_info(&handle)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(refused(dir, "the folder is a link"));
    }
    check_path(&handle, &long_path(dir))
}

/// Checks that open file `file` is a plain file (not a link, one hard link) at `path`.
fn check_file(file: &File, path: &Path) -> io::Result<()> {
    let info = file_info(file)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(refused(path, "it is a link"));
    }
    if info.nNumberOfLinks != 1 {
        return Err(refused(path, "it has more than one hard link"));
    }
    // Compared with the folder's long form, as `real_path` gives it (see `long_path`).
    let expected = match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => long_path(dir).join(name),
        _ => path.to_path_buf(),
    };
    check_path(file, &expected)
}

/// Checks that the real path of `file` (after any junctions) is `expected`.
fn check_path(file: &File, expected: &Path) -> io::Result<()> {
    let real = real_path(file)?;
    if same_path(&real, &expected.to_string_lossy()) { Ok(()) } else { Err(refused(expected, "it leads elsewhere")) }
}

fn file_info(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

/// The path Windows has for open `file`, all junctions and links followed, as `C:\...` or
/// `\\server\share\...`.
fn real_path(file: &File) -> io::Result<String> {
    let flags = FILE_NAME_NORMALIZED | VOLUME_NAME_DOS;
    let path = utf16_call(|buf, len| unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), buf, len, flags) })?;
    Ok(match path.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => path.strip_prefix(r"\\?\").unwrap_or(&path).to_string(),
    })
}

/// `dir` with any short "8.3" names (e.g. `PROGRA~1`) written out in full, as `real_path` gives
/// them. (Only names change: links aren't followed here; `check_path` looks at those.) As given if
/// it can't be looked up.
fn long_path(dir: &Path) -> std::path::PathBuf {
    let name = crate::wide(&dir.to_string_lossy());
    utf16_call(|buf, len| unsafe { GetLongPathNameW(name.as_ptr(), buf, len) }).map_or_else(|_| dir.to_path_buf(), Into::into)
}

/// Calls a Windows function that writes a string into a buffer and returns its length, or the
/// length it needs when the buffer is too small (0 on failure); retries with a big enough buffer.
fn utf16_call(call: impl Fn(*mut u16, u32) -> u32) -> io::Result<String> {
    let mut buf = vec![0u16; 512];
    loop {
        let len = call(buf.as_mut_ptr(), buf.len() as u32) as usize;
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        if len < buf.len() {
            return Ok(String::from_utf16_lossy(&buf[..len]));
        }
        buf.resize(len + 1, 0);
    }
}

/// Whether two Windows paths name the same place, ignoring case, `/` vs `\`, a trailing `\`
/// and a `\\?\` prefix.
fn same_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let s = s.strip_prefix(r"\\?\").unwrap_or(s).replace('/', "\\");
        s.trim_end_matches('\\').to_lowercase()
    };
    norm(a) == norm(b)
}

/// Deletes open `file` itself (not whatever its name leads to now).
fn delete(file: &File) {
    let info = FILE_DISPOSITION_INFO { DeleteFile: 1 };
    unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&info as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
}

fn refused(path: &Path, why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("Not writing {}: {why}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts that opening `path` was refused by the checks (not some other error).
    fn assert_refused(path: &Path) {
        let e = open_checked(path).expect_err("should be refused");
        assert!(e.to_string().starts_with("Not writing"), "{e}");
    }

    /// A new, empty folder for one test.
    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("grid-safe-file-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn plain_files_are_written() {
        let dir = temp_dir("plain");
        let path = dir.join("grid.json");
        let mut file = open_checked(&path).unwrap();
        file.write_all(b"{}").unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        // And opened again once it exists.
        assert!(open_checked(&path).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hard_links_are_refused() {
        let dir = temp_dir("hardlink");
        let target = dir.join("protected.txt");
        std::fs::write(&target, b"keep").unwrap();
        let link = dir.join("grid.log");
        std::fs::hard_link(&target, &link).unwrap();
        assert_refused(&link);
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn junctions_are_refused_and_nothing_is_left_behind() {
        let dir = temp_dir("junction");
        let real = dir.join("real");
        std::fs::create_dir(&real).unwrap();
        let junction = dir.join("Grid");
        // Junctions need no special rights, unlike symbolic links.
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&real)
            .output()
            .unwrap();
        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
        // Through the junction: refused, and the file it created on the other side is gone.
        assert_refused(&junction.join("grid.json"));
        assert!(!real.join("grid.json").exists());
        // An existing file on the other side is left as it was.
        std::fs::write(real.join("grid.log"), b"keep").unwrap();
        assert_refused(&junction.join("grid.log"));
        assert_eq!(std::fs::read(real.join("grid.log")).unwrap(), b"keep");
        let _ = std::fs::remove_dir(&junction);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn short_names_are_accepted() {
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        let dir = temp_dir("short-name-folder");
        let short = utf16_call(|buf, len| unsafe { GetShortPathNameW(crate::wide(&dir.to_string_lossy()).as_ptr(), buf, len) })
            .unwrap();
        // Only meaningful where the drive keeps 8.3 names.
        if short.to_lowercase() != dir.to_string_lossy().to_lowercase() {
            let path = std::path::Path::new(&short).join("grid.json");
            assert!(open_checked(&path).is_ok(), "{short}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paths_compare_like_windows() {
        assert!(same_path(r"C:\Users\A\AppData\Roaming\Grid", r"c:\users\a\appdata\roaming\grid\"));
        assert!(same_path(r"\\?\C:\x\y", "C:/x/y"));
        assert!(!same_path(r"C:\x\y", r"C:\x\z"));
    }
}
