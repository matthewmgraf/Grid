//! Writing Grid's own files (settings, saved positions). The platform layer can replace how files
//! are written, e.g. with extra checks while Grid runs with raised rights (see grid-windows'
//! `safe_file`); by default it's a plain `std::fs::write`.

use std::path::Path;
use std::sync::OnceLock;

/// Writes `data` to `path`, replacing what was there.
pub type Writer = fn(&Path, &[u8]) -> std::io::Result<()>;

static WRITER: OnceLock<Writer> = OnceLock::new();

/// Sets how `write` writes files. Only the first call counts; call it at startup.
pub fn set_writer(writer: Writer) {
    let _ = WRITER.set(writer);
}

/// Writes `data` to `path`, replacing what was there, the way the platform layer set (or
/// `std::fs::write`).
pub fn write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    match WRITER.get() {
        Some(writer) => writer(path, data),
        None => std::fs::write(path, data),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_with_std_by_default() {
        let path = std::env::temp_dir().join(format!("grid-store-test-{}.txt", std::process::id()));
        super::write(&path, b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        let _ = std::fs::remove_file(&path);
    }
}
