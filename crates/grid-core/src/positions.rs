//! Saved window positions, and matching them to the windows that are open now.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;

/// What identifies a window, from strongest to weakest: its handle, its process, its app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowId {
    /// OS window handle. Only meaningful until the window closes.
    pub handle: u64,
    /// Process ID. Only meaningful until the process exits.
    pub pid: u32,
    /// Executable path, or the app's ID for apps that share one host process (Store apps).
    pub app: String,
    pub class: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShowState {
    Normal,
    Maximized,
    Minimized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub id: WindowId,
    /// Visible frame, used to put a normal window back.
    pub frame: Rect,
    /// The OS's restored position, used for maximized and minimized windows. On Windows this is
    /// `WINDOWPLACEMENT::rcNormalPosition`, stored as is.
    pub normal: Rect,
    pub state: ShowState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub windows: Vec<SavedWindow>,
}

impl Snapshot {
    pub fn load(path: &Path) -> Result<Snapshot, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

type Level = fn(&WindowId, &WindowId) -> bool;

/// Pairs saved windows with open ones, as `(saved index, open index)`. Each level is tried for
/// every saved window before the next, weaker one, so a strong match is never taken by a weaker
/// one first. Each window is used at most once; saved windows with no match are left out.
///
/// Levels: same handle and process; process + class + title; app + class + title;
/// app + class; app.
pub fn match_windows(saved: &[WindowId], open: &[WindowId]) -> Vec<(usize, usize)> {
    fn same_app(a: &WindowId, b: &WindowId) -> bool {
        !a.app.is_empty() && a.app.eq_ignore_ascii_case(&b.app)
    }
    let levels: [Level; 5] = [
        |a, b| a.handle == b.handle && a.pid == b.pid,
        |a, b| a.pid == b.pid && a.class == b.class && a.title == b.title,
        |a, b| same_app(a, b) && a.class == b.class && a.title == b.title,
        |a, b| same_app(a, b) && a.class == b.class,
        |a, b| same_app(a, b),
    ];
    let (mut saved_used, mut open_used) = (vec![false; saved.len()], vec![false; open.len()]);
    let mut pairs = Vec::new();
    for matches in levels {
        for (i, s) in saved.iter().enumerate() {
            if saved_used[i] {
                continue;
            }
            if let Some(j) = (0..open.len()).find(|&j| !open_used[j] && matches(s, &open[j])) {
                saved_used[i] = true;
                open_used[j] = true;
                pairs.push((i, j));
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(handle: u64, pid: u32, app: &str, class: &str, title: &str) -> WindowId {
        WindowId { handle, pid, app: app.into(), class: class.into(), title: title.into() }
    }

    #[test]
    fn same_window_matches_first() {
        let saved = [id(1, 10, "a.exe", "C", "One"), id(2, 10, "a.exe", "C", "Two")];
        let open = [id(2, 10, "a.exe", "C", "Two"), id(1, 10, "a.exe", "C", "One")];
        assert_eq!(match_windows(&saved, &open), vec![(0, 1), (1, 0)]);
    }

    #[test]
    fn reopened_document_matches_by_app_and_title() {
        // Acrobat restarted: new handles and process, same document title.
        let saved = [id(1, 10, r"C:\Acrobat.exe", "AcroFrame", "Report.pdf")];
        let open = [
            id(7, 20, r"c:\acrobat.exe", "AcroFrame", "Other.pdf"),
            id(8, 20, r"C:\Acrobat.exe", "AcroFrame", "Report.pdf"),
        ];
        assert_eq!(match_windows(&saved, &open), vec![(0, 1)]);
    }

    #[test]
    fn stronger_match_is_not_taken_by_a_weaker_one() {
        // Saved 0 would match open 0 by app only, but saved 1 matches it by title.
        let saved = [id(1, 10, "a.exe", "X", "Old"), id(2, 10, "a.exe", "C", "Doc")];
        let open = [id(9, 30, "a.exe", "C", "Doc")];
        assert_eq!(match_windows(&saved, &open), vec![(1, 0)]);
    }

    #[test]
    fn closed_apps_are_skipped() {
        let saved = [id(1, 10, "gone.exe", "C", "T"), id(2, 11, "b.exe", "C", "T")];
        let open = [id(5, 12, "b.exe", "D", "U")];
        assert_eq!(match_windows(&saved, &open), vec![(1, 0)]);
    }
}
