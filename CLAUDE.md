# Grid

Grid is a lightweight window manager for Windows that splits each monitor into a grid
(or custom zones) and lets the user move and resize windows to fit it. It is written from
scratch in Rust.

Notes for AI assistants and contributors. Maintainers may keep extra private notes in
`CLAUDE.local.md` (gitignored).

## Goals

- Drag-and-snap window management with a modern UI (light and dark mode).
- Super lightweight: a single small native exe with no runtime to install.
- Portable OR installed. One exe does both.
- Structured so a macOS port only needs a new platform layer.
- Tray icon menu: Settings, Check for updates, Exit.

## Decisions

| Topic | Decision |
|---|---|
| Name | "Grid". Logo: one rounded 16:9 frosted cell on a dark rounded square, drawn in `logo.rs` (`logo::draw`). The exe icon, tray and title-bar icon, splash and About tab all use it. |
| Language | Rust (stable, MSVC toolchain) |
| Hosting / updates | GitHub repo + GitHub Releases; the app checks the latest release |
| License | GPLv3 (`GPL-3.0-or-later`), see `LICENSE`. Keep the `AboutSlint` widget on the About tab. |
| Single exe | One `grid.exe` runs as the engine, `--settings` (settings window), `--capture` (press-to-bind helper) and the other modes below. Two `grid.exe` processes while Settings is open is expected. |
| Settings UI | Slint 1.18, software renderer, Fluent style, in its own process (`grid.exe --settings`). Measured: ~34 MB working set vs 129–311 MB for GPU renderers/egui. |
| Code signing | Not yet. |
| Install model | Same exe can install itself (Program Files or per-user, Start menu, optional autostart) or run portably. Only installed copies can start with Windows, to steer users to installing; a portable copy removes an old Run entry pointing at itself. The installer recommends an administrator install ("for the best compatibility") but also offers a per-user install that needs no admin rights. |
| Admin windows | Windows (UIPI) stops a normal Grid moving windows of apps run as administrator (e.g. Task Manager). When such a drag ends, Grid asks whether to restart with administrator rights (No / Yes, one popup at a time). "Don't ask again until Grid restarts" mutes it for the session; Settings → General → "Never prompt to start Grid as administrator" turns it off (`ask_to_run_as_admin`). It can't ask only when a snap is attempted: a normal Grid doesn't see input to admin windows (tested: `GetAsyncKeyState` and Raw Input with `RIDEV_INPUTSINK` both see nothing). Settings → General also has a "Restart Grid as administrator" button. Yes relaunches via UAC (`runas`, `--restarted`). |
| Settings storage | JSON file. Portable: next to the exe. Installed: `%APPDATA%\Grid`. |
| Logging | Errors and lifecycle only. Never log keystrokes (or other input, or window titles). `grid.log` next to `grid.json`, renamed to `grid.log.old` past 256 KB (`log.rs`). Records starts/exits with reasons, errors shown, panics, sleep/resume, sign-out, and freezes: a watchdog thread pings the engine every 10 s and logs what the main thread was handling if it takes over 5 s (`watchdog.rs`). |
| Mixed DPI | Allow snapping across monitors with different scaling. After `SetWindowPos`, re-check the rect because DPI-aware apps resize themselves on `WM_DPICHANGED`; correct it if needed. |

## Features

- Drag a window (left button held), press a bound input to enter grid mode:
  - **Resize**: press → selects start cell, release → drag to end cell; window covers the span.
  - **Move**: on press the window snaps at once to the nearest whole cells (minimum one cell)
    and keeps that size. It then follows the cursor a cell at a time, always covering the cell
    under the cursor at the same spot. It stays on screen but may straddle neighbouring monitors.
  - Both update the window live. When grid mode starts, Grid ends the OS move loop with a
    synthetic left-button-up and places the window itself until the real button-up (swallowed).
  - **Cancel** (default Esc, rebindable on the Input tab) puts the window back.
- Default bindings: Space → Resize, Left Ctrl → Move, Right mouse → Resize, Middle mouse → Move,
  Esc → Cancel. All rebindable.
- Defaults: 8×8 grid, no gaps, 10% opacity, no blur, no hotkeys set, splash on.
- N×M grid, per-monitor grids, grid transposes on portrait monitors.
- Overlay with blur, opacity, target-area highlight and colours.
- Multi-monitor, including spanning a window across monitors.
- Gaps between snapped windows (only the placed window is inset; the overlay is drawn as normal).
- Save and restore window positions; undo last snap (up to 10).
- Splash screen toggle; update check at startup; silent update.
- "Recommended OS settings": show window contents while dragging.
- `TESTING.md` is the hands-on test checklist; keep it in step with features.

### Next version

- Keyboard-only snapping: a hotkey shows the grid for the focused window.
- Ignored apps list.
- Custom zones defined to the pixel (e.g. zone 1 = left half; zones 2 and 3 = right half split
  top/bottom). When a selection spans monitors/zones of different heights, expand to cover
  the zones that match the start zone's extent. `select_span` already handles this.
- Saved layouts (switchable).

### Ideas for later

- Alt to subdivide the grid while dragging
- Halves/thirds/quarters hotkeys that cycle on repeat
- Move window to next monitor hotkey
- Resize side-by-side windows together

## Save/restore window matching

Match saved entries to open windows from strongest to weakest identifier. Each open window
can fill only one saved slot, and saved entries with no match are skipped.

1. Same window handle (HWND), if still valid and still owned by the same process
2. Same process ID + window class + title
3. Same executable path + window class + title (e.g. a new Acrobat window with the same document title)
4. Same executable path + window class (title differs)
5. Same executable path only

Notes:
- HWNDs and PIDs do not survive app restarts or reboots, so levels 3–5 handle those cases.
- UWP/Store apps all run under `ApplicationFrameHost.exe`; identify them by AppUserModelID instead of exe path.
- A non-elevated Grid cannot move windows of elevated (admin) apps (UIPI). This applies to all snapping.

## Architecture

Cargo workspace:

- `crates/grid-core`: platform-agnostic logic with no OS calls. Everything here is unit-tested.
  - `geometry`: `Point`, `Rect`, `Grid` (physical pixels).
  - `zones`: grid cells, cross-monitor span selection (`select_span` grows the bounding box to
    take in zones it covers by at least half), move snapping, `apply_gaps`.
  - `session`: the Resize/Move/Cancel state machine for one drag. Platform layers feed it input.
  - `settings`: JSON settings model with defaults; inputs written as names like `"Space"`, `"RightMouse"`.
  - `history`: undo stack (last 10 snaps). `positions`: saved window snapshot + `match_windows`.
  - `update`: reading GitHub's latest release and comparing versions. `logo`: the logo.
- `crates/grid-windows`: Win32 layer (`windows-sys`). `lib.rs` sets up the engine and routes its
  messages; `drag.rs` (drag detection, hooks, grid mode, placement), `admin.rs` (administrator
  windows and restarting elevated), `actions.rs` (tray and hotkey actions), `ui.rs` (message boxes,
  icon, minimum size), plus the overlay, tray (`tray.rs`), install (`install.rs`), updates
  (`update.rs`), `log.rs` and `watchdog.rs`.
  All state lives on the main thread; use `with_app` (never panics on re-entry).
  The engine reloads `grid.json` at the start of each drag if the file changed.
- `crates/grid-settings`: the Slint windows (`ui/settings.slint`, `ui/setup.slint`,
  `ui/admin.slint`). Platform-agnostic; the app passes in platform functions (`Platform`).
  Monitors are polled every 1.5 s so plugged-in monitors appear. Per-monitor grids are keyed by
  the monitor's device path (stable across reconnects; from the DisplayConfig API in `monitors.rs`).
- `crates/grid-app`: the `grid` executable. `build.rs` turns `grid_core::logo` into an .ico and
  embeds it (resource ID 1) plus version info with `embed-resource`.
  - Modes: `grid.exe` (engine), `--settings`, `--capture`, `--capture-hotkey`, `--admin-prompt`,
    `--setup` (install window; also shown on the first run of a downloaded exe with no
    `grid.json` next to it), `--install all|user` (the administrator half of installing),
    `--uninstall` (Apps & features), `--apply-update <version>` (administrator half of an all-users update: downloads and checks the file itself).
  - Settings path: `grid.json` next to the exe when portable, `%APPDATA%\Grid\grid.json` when
    installed (`install::settings_path`).
  - Installing: Program Files or `%LOCALAPPDATA%\Programs\Grid`, Start menu shortcut via
    `IShellLinkW`, Uninstall registry entry, HKCU Run key for autostart. Running engines are
    asked to exit with the registered `Grid.Exit` message; a running exe is replaced by renaming
    it to `grid.exe.old`.
  - Updates: WinHTTP, GitHub latest release, asset `grid.exe`; see `RELEASING.md`.
  - Release builds have no console.
- Later: `crates/grid-macos` with its own `run`, reusing `grid-core` and `grid-settings`.

Implementation notes:
- **Do not inject DLLs** into other processes (antivirus flags, broken browser tab tearing). Use
  out-of-context `SetWinEventHook` (`EVENT_SYSTEM_MOVESIZESTART/END`) plus low-level
  `WH_MOUSE_LL` / `WH_KEYBOARD_LL` hooks from the Grid process.
- Input Grid sends itself (`SendInput`) is sent from another thread, because it passes through
  Grid's own low-level hooks on the main thread.
- The overlay is "frosted glass": per-pixel-alpha layered windows (`UpdateLayeredWindowIndirect`
  with dirty rects), blur via the undocumented `SetWindowCompositionAttribute` (on/off only).
  With blur off, cells are outlines only. It is kept directly below the dragged window so that
  window stays sharp. `cargo test -p grid-windows preview -- --ignored` with
  `GRID_OVERLAY_PREVIEW=<file.bmp>` renders a preview without dragging.
- Hotkeys use `RegisterHotKey`; the settings window posts `Grid.SettingsChanged` to the engine
  after saving. Window positions are saved to `positions.json` next to `grid.json`.
- Measured: engine ~22 MB working set / 6.5 MB private at start, ~34 MB / 9 MB after a day of use (Task Manager shows ~10 MB); settings window ~34 MB / 10 MB;
  release exe ~9.3 MB.

## Conventions

- Release builds are size-optimized (see `[profile.release]` in the root `Cargo.toml`).
- The C runtime is statically linked (`.cargo/config.toml`), so the exe needs only Windows system
  DLLs. Check with `dumpbin /dependents` that no `VCRUNTIME140.dll` / `api-ms-win-crt-*` reappears.
- Keep dependencies minimal; justify each new crate.
- Run `cargo test` and `cargo clippy` before committing.
