# Grid

A lightweight window manager for Windows. Drag a window, press a key, and snap it to a grid of
cells on your screen: one cell, or any rectangle of cells, even across monitors.

<!-- Screenshot or short GIF of a window being snapped goes here, e.g. docs/demo.gif -->

## Features

- **Resize**: while dragging a window, hold Space (or the right mouse button) to pick the first
  cell, release, then move to the last cell. The window covers everything in between.
- **Move**: press Left Ctrl (or the middle mouse button) and the window snaps to whole cells and
  moves a cell at a time, keeping its size.
- **Cancel** with Esc: the window goes back to where it was.
- Any grid size, a different grid per monitor, and grids that turn with portrait monitors.
- Multiple monitors, including monitors with different display scaling.
- Gaps between snapped windows and at the screen edges.
- Undo the last snaps, and save and restore all your window positions (with optional hotkeys).
- All inputs can be changed, including extra mouse buttons.
- A frosted-glass overlay with your own colours and opacity, and light and dark mode.
- Small and light: one ~9 MB `grid.exe` with nothing else to install, using about 10 MB of
  memory while running (as Task Manager shows it).
- Install it (for everyone or just you) or run it portably from any folder.
- Checks GitHub for new versions and updates itself.

## Download and install

1. Download `grid.exe` from the [latest release](https://github.com/matthewmgraf/Grid/releases/latest).
2. Run it. Choose **Install for all users** (recommended), **Install just for me**, or **Run
   without installing** (portable: everything stays in its folder, but it can't start with
   Windows).
3. Grid lives in the notification area (tray). Right-click its icon for Settings, updates and Exit.

Grid isn't code-signed yet, so Windows SmartScreen may warn about it the first time. Choose
"More info", then "Run anyway".

To uninstall, use Windows Settings → Apps → Installed apps → Grid, or Grid's Settings → General.

## Build from source

Requires Rust (stable, MSVC toolchain) and the Visual Studio C++ Build Tools.

```
cargo build --release
```

The program is written to `target/release/grid.exe`. `cargo test` runs the tests.

## Licence

Grid is free software under the [GNU General Public License v3.0 or later](LICENSE).
Donations are welcome.

Grid is a new program written from scratch, inspired by WindowGrid by Joshua Wilding. It isn't
affiliated with WindowGrid and uses none of its code or artwork.
