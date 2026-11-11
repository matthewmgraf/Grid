//! User settings, stored as JSON.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::geometry::Grid;
use crate::session::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Grid used on monitors without their own entry in `monitors`.
    pub grid: Grid,
    pub monitors: Vec<MonitorGrid>,
    /// Swap columns and rows on monitors taller than they are wide.
    pub transpose_portrait: bool,
    pub overlay: Overlay,
    pub bindings: Vec<Binding>,
    /// Light/dark mode for Grid's own windows.
    pub appearance: Appearance,
    pub gaps: Gaps,
    pub hotkeys: Hotkeys,
    /// Show the splash screen when Grid starts.
    pub splash: bool,
    pub updates: Updates,
    /// Offer to restart as administrator after the user drags a window of an app that runs as
    /// administrator (Grid can't move those otherwise). Off = "Never prompt" in Settings.
    pub ask_to_run_as_admin: bool,
}

/// Checking GitHub for a newer Grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Updates {
    /// Look for a newer version each time Grid starts.
    pub check_at_startup: bool,
    /// Install a newer version found at startup without asking first.
    pub install_silently: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self { check_at_startup: true, install_silently: false }
    }
}

/// Space left around snapped windows, in pixels at 100% display scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Gaps {
    /// Between neighbouring windows.
    pub between: u32,
    /// Between a window and the edge of the screen.
    pub edges: u32,
}

/// Global hotkeys. `None` means not set; none are set by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub undo: Option<Hotkey>,
    pub save_positions: Option<Hotkey>,
    pub restore_positions: Option<Hotkey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Follow the OS setting.
    #[default]
    System,
    Light,
    Dark,
}

/// Per-monitor grid. `monitor` is the platform's stable ID for that monitor (on Windows its device
/// path).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorGrid {
    pub monitor: String,
    pub grid: Grid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    /// Strength of the tint over the screen: 0.0 (none) to 1.0 (opaque).
    pub opacity: f32,
    /// Blur the screen behind the overlay (frosted glass).
    pub blur: bool,
    pub tint_color: Color,
    pub line_color: Color,
    pub start_color: Color,
    pub target_color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub input: Input,
    pub command: Command,
}

impl Default for Settings {
    fn default() -> Self {
        let bind = |input, command| Binding { input, command };
        Self {
            grid: Grid { cols: 8, rows: 8 },
            monitors: Vec::new(),
            transpose_portrait: true,
            overlay: Overlay::default(),
            bindings: vec![
                bind(Input::Key(Key::Space), Command::Resize),
                bind(Input::Key(Key::LeftCtrl), Command::Move),
                bind(Input::Mouse(MouseButton::Right), Command::Resize),
                bind(Input::Mouse(MouseButton::Middle), Command::Move),
                bind(Input::Key(Key::Escape), Command::Cancel),
            ],
            appearance: Appearance::System,
            gaps: Gaps::default(),
            hotkeys: Hotkeys::default(),
            splash: true,
            updates: Updates::default(),
            ask_to_run_as_admin: true,
        }
    }
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            opacity: 0.1,
            blur: false,
            tint_color: Color::rgb(0x0F, 0x17, 0x2A),
            line_color: Color::rgb(0x60, 0xCD, 0xFF),
            start_color: Color::rgb(0x00, 0x78, 0xD4),
            target_color: Color::rgb(0x60, 0xCD, 0xFF),
        }
    }
}

impl Settings {
    /// Loads settings from `path`. A missing file is created with defaults. A file that can't be
    /// read or parsed is left untouched and defaults are used; the error is returned as a warning.
    pub fn load(path: &Path) -> (Settings, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let settings = Settings::default();
                let warning = settings
                    .save(path)
                    .err()
                    .map(|e| format!("Could not create {}: {e}", path.display()));
                return (settings, warning);
            }
            Err(e) => return (Settings::default(), Some(format!("Could not read {}: {e}", path.display()))),
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(settings) => (settings.sanitized(), None),
            Err(e) => (
                Settings::default(),
                Some(format!("Could not parse {}: {e}. Using defaults.", path.display())),
            ),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Grid for the monitor with this stable `id`.
    pub fn grid_for(&self, id: &str) -> Grid {
        self.monitor_grid(id).map_or(self.grid, |m| m.grid)
    }

    /// The monitor's own grid, if it has one.
    pub fn monitor_grid(&self, id: &str) -> Option<&MonitorGrid> {
        self.monitors.iter().find(|m| m.monitor.eq_ignore_ascii_case(id))
    }

    pub fn command_for(&self, input: Input) -> Option<Command> {
        self.bindings.iter().find(|b| b.input == input).map(|b| b.command)
    }

    fn sanitized(mut self) -> Self {
        let clamp = |g: &mut Grid| {
            g.cols = g.cols.clamp(1, 64);
            g.rows = g.rows.clamp(1, 64);
        };
        clamp(&mut self.grid);
        self.monitors.iter_mut().for_each(|m| clamp(&mut m.grid));
        self.overlay.opacity = self.overlay.opacity.clamp(0.0, 1.0);
        self.gaps.between = self.gaps.between.min(MAX_GAP);
        self.gaps.edges = self.gaps.edges.min(MAX_GAP);
        // There must always be a way to cancel. This also gives settings files from before
        // Cancel could be rebound their Esc back.
        let esc = Input::Key(Key::Escape);
        if !self.bindings.iter().any(|b| b.command == Command::Cancel) && self.command_for(esc).is_none() {
            self.bindings.push(Binding { input: esc, command: Command::Cancel });
        }
        self
    }
}

/// Largest gap the settings allow, in pixels.
pub const MAX_GAP: u32 = 100;

/// An sRGB color, written as `"#RRGGBB"` in the settings file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl TryFrom<String> for Color {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        let hex = s.strip_prefix('#').unwrap_or(&s);
        let v = u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 6);
        let v = v.ok_or_else(|| format!("invalid color \"{s}\", expected \"#RRGGBB\""))?;
        Ok(Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }
}

impl From<Color> for String {
    fn from(c: Color) -> String {
        format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// The primary button (the one that drags windows), so it can't be bound. Platforms name
    /// buttons after the left-handed swap: `Right` is the secondary (context menu) button.
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Escape,
    Space,
    Tab,
    LeftCtrl,
    RightCtrl,
    LeftShift,
    RightShift,
    LeftAlt,
    RightAlt,
    Enter,
    Backspace,
    CapsLock,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    /// `A`–`Z` or `0`–`9`.
    Char(char),
    /// `F1`–`F24`.
    F(u8),
    /// Number-pad digit `0`–`9`.
    Numpad(u8),
}

/// A key or mouse button that triggers a command while dragging a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Input {
    Key(Key),
    Mouse(MouseButton),
}

const NAMED_INPUTS: &[(&str, Input)] = &[
    ("Esc", Input::Key(Key::Escape)),
    ("Space", Input::Key(Key::Space)),
    ("Tab", Input::Key(Key::Tab)),
    ("LeftCtrl", Input::Key(Key::LeftCtrl)),
    ("RightCtrl", Input::Key(Key::RightCtrl)),
    ("LeftShift", Input::Key(Key::LeftShift)),
    ("RightShift", Input::Key(Key::RightShift)),
    ("LeftAlt", Input::Key(Key::LeftAlt)),
    ("RightAlt", Input::Key(Key::RightAlt)),
    ("Enter", Input::Key(Key::Enter)),
    ("Backspace", Input::Key(Key::Backspace)),
    ("CapsLock", Input::Key(Key::CapsLock)),
    ("Insert", Input::Key(Key::Insert)),
    ("Delete", Input::Key(Key::Delete)),
    ("Home", Input::Key(Key::Home)),
    ("End", Input::Key(Key::End)),
    ("PageUp", Input::Key(Key::PageUp)),
    ("PageDown", Input::Key(Key::PageDown)),
    ("Up", Input::Key(Key::Up)),
    ("Down", Input::Key(Key::Down)),
    ("Left", Input::Key(Key::Left)),
    ("Right", Input::Key(Key::Right)),
    // Named for completeness; the primary button can't be bound.
    ("LeftMouse", Input::Mouse(MouseButton::Left)),
    ("RightMouse", Input::Mouse(MouseButton::Right)),
    ("MiddleMouse", Input::Mouse(MouseButton::Middle)),
    ("Mouse4", Input::Mouse(MouseButton::X1)),
    ("Mouse5", Input::Mouse(MouseButton::X2)),
];

impl FromStr for Input {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        if let Some((_, input)) = NAMED_INPUTS.iter().find(|(name, _)| name.eq_ignore_ascii_case(s)) {
            return Ok(*input);
        }
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && c.is_ascii_alphanumeric() {
                return Ok(Input::Key(Key::Char(c.to_ascii_uppercase())));
            }
        if let Some(n) = s.strip_prefix(['F', 'f']).and_then(|n| n.parse::<u8>().ok())
            && (1..=24).contains(&n) {
                return Ok(Input::Key(Key::F(n)));
            }
        let numpad = s.get(..6).filter(|p| p.eq_ignore_ascii_case("numpad")).and(s.get(6..));
        if let Some(n) = numpad.and_then(|n| n.parse::<u8>().ok())
            && n <= 9 {
                return Ok(Input::Key(Key::Numpad(n)));
            }
        Err(format!("unknown input \"{s}\""))
    }
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((name, _)) = NAMED_INPUTS.iter().find(|(_, input)| input == self) {
            return f.write_str(name);
        }
        match self {
            Input::Key(Key::Char(c)) => write!(f, "{c}"),
            Input::Key(Key::F(n)) => write!(f, "F{n}"),
            Input::Key(Key::Numpad(n)) => write!(f, "Numpad{n}"),
            _ => unreachable!("every other input is in NAMED_INPUTS"),
        }
    }
}

impl TryFrom<String> for Input {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        s.parse()
    }
}

impl From<Input> for String {
    fn from(input: Input) -> String {
        input.to_string()
    }
}

/// A key with modifiers, such as `Ctrl+Alt+Z`, that works anywhere (not only while dragging).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

impl Key {
    /// Ctrl, Shift or Alt on either side. These can't be a hotkey's main key.
    pub fn is_modifier(self) -> bool {
        use Key::*;
        matches!(self, LeftCtrl | RightCtrl | LeftShift | RightShift | LeftAlt | RightAlt)
    }
}

impl FromStr for Hotkey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let mut hotkey = Hotkey { ctrl: false, alt: false, shift: false, win: false, key: Key::Space };
        let mut key = None;
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" => hotkey.ctrl = true,
                "alt" => hotkey.alt = true,
                "shift" => hotkey.shift = true,
                "win" => hotkey.win = true,
                _ => match part.parse::<Input>() {
                    Ok(Input::Key(k)) if !k.is_modifier() && key.is_none() => key = Some(k),
                    _ => return Err(format!("invalid hotkey \"{s}\"")),
                },
            }
        }
        hotkey.key = key.ok_or_else(|| format!("hotkey \"{s}\" has no key"))?;
        Ok(hotkey)
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mods = [(self.ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+"), (self.win, "Win+")];
        for (_, name) in mods.iter().filter(|(on, _)| *on) {
            f.write_str(name)?;
        }
        write!(f, "{}", Input::Key(self.key))
    }
}

impl TryFrom<String> for Hotkey {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        s.parse()
    }
}

impl From<Hotkey> for String {
    fn from(h: Hotkey) -> String {
        h.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_round_trip_through_names() {
        let h: Hotkey = "ctrl+alt+z".parse().unwrap();
        assert_eq!(h, Hotkey { ctrl: true, alt: true, shift: false, win: false, key: Key::Char('Z') });
        assert_eq!(h.to_string(), "Ctrl+Alt+Z");
        assert_eq!("Shift+F5".parse::<Hotkey>().unwrap().to_string(), "Shift+F5");
        assert!("Ctrl+Alt".parse::<Hotkey>().is_err());
        assert!("Ctrl+LeftShift".parse::<Hotkey>().is_err());
    }

    #[test]
    fn cleared_hotkey_stays_cleared() {
        let s: Settings = serde_json::from_str(r#"{ "hotkeys": { "undo": "Ctrl+Alt+Z", "save_positions": null } }"#).unwrap();
        assert!(s.hotkeys.undo.is_some());
        assert_eq!(s.hotkeys.save_positions, None);
        assert_eq!(Settings::default().hotkeys, Hotkeys::default());
        assert_eq!(Hotkeys::default().undo, None);
    }

    #[test]
    fn defaults_round_trip_through_json() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), Settings::default());
    }

    #[test]
    fn partial_file_fills_in_defaults() {
        let s: Settings = serde_json::from_str(r#"{ "grid": { "cols": 6, "rows": 4 } }"#).unwrap();
        assert_eq!(s.grid, Grid { cols: 6, rows: 4 });
        assert_eq!(s.bindings, Settings::default().bindings);
    }

    #[test]
    fn parses_inputs() {
        assert_eq!("space".parse(), Ok(Input::Key(Key::Space)));
        assert_eq!("q".parse(), Ok(Input::Key(Key::Char('Q'))));
        assert_eq!("F12".parse(), Ok(Input::Key(Key::F(12))));
        assert!("F25".parse::<Input>().is_err());
        assert_eq!(Input::Key(Key::F(3)).to_string(), "F3");
    }

    #[test]
    fn per_monitor_grid_overrides_default() {
        let mut s = Settings::default();
        s.monitors.push(MonitorGrid { monitor: "monitor-b".into(), grid: Grid { cols: 3, rows: 2 } });
        assert_eq!(s.grid_for("MONITOR-B"), Grid { cols: 3, rows: 2 });
        assert_eq!(s.grid_for("monitor-a"), s.grid);
    }

    #[test]
    fn all_inputs_round_trip_through_names() {
        let named = NAMED_INPUTS.iter().map(|(_, input)| *input);
        let chars = ('A'..='Z').chain('0'..='9').map(|c| Input::Key(Key::Char(c)));
        let f_keys = (1..=24).map(|n| Input::Key(Key::F(n)));
        let numpad = (0..=9).map(|n| Input::Key(Key::Numpad(n)));
        for input in named.chain(chars).chain(f_keys).chain(numpad) {
            assert_eq!(input.to_string().parse(), Ok(input));
        }
    }

    #[test]
    fn appearance_is_lowercase_in_json() {
        assert_eq!(serde_json::to_string(&Appearance::Dark).unwrap(), "\"dark\"");
    }

    #[test]
    fn missing_cancel_gets_esc() {
        let old: Settings = serde_json::from_str(r#"{ "bindings": [ { "input": "Space", "command": "Resize" } ] }"#).unwrap();
        assert_eq!(old.sanitized().command_for(Input::Key(Key::Escape)), Some(Command::Cancel));
        // Esc bound to something else, but another Cancel input set: left alone.
        let custom: Settings = serde_json::from_str(
            r#"{ "bindings": [ { "input": "Esc", "command": "Move" }, { "input": "Q", "command": "Cancel" } ] }"#,
        )
        .unwrap();
        assert_eq!(custom.clone().sanitized(), custom);
    }

    #[test]
    fn zero_grid_is_clamped() {
        let s: Settings = serde_json::from_str(r#"{ "grid": { "cols": 0, "rows": 500 } }"#).unwrap();
        assert_eq!(s.sanitized().grid, Grid { cols: 1, rows: 64 });
    }
}