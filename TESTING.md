# Grid hands-on test checklist

Tick each box when the result matches. Note anything odd with the app you used.

**Before you start**
- Exit WindowGrid (both react to the same keys).
- Run `target\release\grid.exe`. Default bindings: Space and right mouse = Resize, Left Ctrl
  and middle mouse = Move.
- "Drag" below means: hold the left button on a window's title bar and move it.

## 1. Start and tray

- [ ] Starting Grid shows the splash (logo, "Grid", version), which fades out after about 1.5 s.
- [ ] Turn the splash off (Settings → General → untick, Save), exit and restart: no splash.
      Turn it back on afterwards.
- [ ] Starting a second `grid.exe` says Grid is already running.
- [ ] The tray icon, the splash, the About tab and grid.exe's icon in Explorer all show the
      single-cell logo. Hover over the tray icon shows "Grid".
- [ ] Right-click the tray icon: Undo last snap, Save / Restore window positions, Settings,
      Check for updates, Exit.
- [ ] Double-click the tray icon opens Settings. Doing it again brings the open window to the
      front (no second window).
- [ ] Check for updates shows the "not available yet" message.
- [ ] Exit closes Grid and an open Settings window, and removes the tray icon.

## 2. Resize

- [ ] Drag a window, press and hold Space: the grid appears behind the window, the cell under
      the cursor is highlighted and the window fills it.
- [ ] Still holding Space, move: the start cell follows the cursor.
- [ ] Release Space and move: the window stretches from the start cell to the cell under the
      cursor, live.
- [ ] Release the mouse button: the window stays there and the grid disappears.
- [ ] Same with the right mouse button instead of Space (no context menu appears).
- [ ] Press Esc during grid mode: the window goes back to where it was before the drag.
- [ ] Press Esc during a normal drag (before Space): Windows puts the window back as usual.
- [ ] Bind Cancel to another key (e.g. Q, see Input tab): Q puts the window back both during
      grid mode and during a normal drag. Esc no longer does anything in grid mode.
- [ ] Grid mode starts without a noticeable delay. Try a few different apps.
- [ ] When grid mode starts, the window goes straight to the cell: no brief flash at its old
      (full) size.

## 3. Move

- [ ] Drag a window, press Left Ctrl: the window snaps to the whole cells it mostly covers.
- [ ] Move the cursor: the window jumps a cell at a time, keeping its size, with the cell under
      the cursor in the same spot of the window.
- [ ] Near a screen edge the window stays fully on screen.
- [ ] Same with the middle mouse button.
- [ ] Start with Resize, then press Move in the same drag: Move keeps the resized size.
- [ ] Esc puts the window back.

## 4. Apps that behave differently

- [ ] A maximized window: dragging it down from the top and snapping works.
- [ ] An app with a minimum size (e.g. Settings, or a small cell on a big app): the window keeps
      its own size, anchored to the target, and stays on screen.
- [ ] Same app, moving across several small cells: the window steps from cell to cell smoothly,
      without flickering as it tries a smaller size each time.
- [ ] Browser tab tearing (drag a Chrome/Edge tab out) still works normally.
- [ ] Windows 11 Snap Layouts (drag to the top of the screen without pressing a key) still work.

## 5. Administrator windows

- [ ] With Grid not elevated, drag Task Manager and press Space: nothing moves. When you let go,
      a popup asks to restart Grid as administrator. Its text, "Don't ask again until Grid
      restarts" and No/Yes fit.
- [ ] No closes it. Dragging Task Manager again asks again (never two popups at once).
- [ ] Tick "Don't ask again until Grid restarts", No: further drags don't ask. Exit Grid and
      start it again: it asks again.
- [ ] Settings → General → Administrator rights → tick "Never prompt to start Grid as
      administrator", Save: dragging Task Manager doesn't ask, even after restarting Grid.
      Untick and Save: it asks again.
- [ ] Settings → General → Administrator rights: "Restart Grid as administrator" restarts it
      (after Windows' prompt) while Settings stays open; within a couple of seconds the group
      says Grid is running as administrator and the button is greyed out.
- [ ] Restart Grid, drag Task Manager, choose Yes and accept Windows' prompt: the splash shows,
      the tray icon comes back, and Task Manager now snaps.
- [ ] While elevated: Settings opens from the tray, and Save still updates the running Grid.

## 6. Settings window

- [ ] Opens in your light/dark mode. Appearance Light / Dark / System switches it right away.
- [ ] Save is greyed out until you change something, and greyed again if you change it back.
- [ ] Shrinking the window stops at its minimum size; every tab scrolls so Close and Save stay
      visible. Snapping the Settings window into a small cell also can't shrink it below that.
- [ ] Close with unsaved changes discards them (reopen to check).
- [ ] About tab: logo, version, GitHub link and Slint badge, centered, not stretching.

### Grid tab
- [ ] Change columns/rows (e.g. 4 × 3), Save, drag: the new grid shows. No restart needed.
- [ ] Gaps: set Between windows 20 and At screen edges 10, Save. Snapped windows leave a 20 px
      gap between neighbours and 10 px at screen edges (scaled with display scaling). The grid
      cells line up with where windows land.
- [ ] Portrait monitor (if you have one): with "Swap columns and rows" on, a 4 × 2 grid shows as
      2 × 4 there.

### Overlay tab
- [ ] Opacity slider changes how dark the tint is (Save, then drag).
- [ ] Blur on: the screen behind the grid is blurred and cells look frosted. Blur off: plain
      tint with cell outlines.
- [ ] Each colour (Tint, Grid lines, Start cell, Target area) changes the overlay. A bad value
      such as `#12` makes Save show an error instead of saving.
- [ ] Reset to defaults puts the colours and opacity back.

### Input tab
- [ ] Click an input, press a key (e.g. Q): it becomes Q. A left click elsewhere cancels and
      keeps the old one.
- [ ] "Cancel" is listed under Move, with Esc. Esc can be bound by pressing
      it. Removing every Cancel input makes Save show an error.
- [ ] Hotkeys: Esc still cancels waiting for a key combination.
- [ ] Clicking the left (primary) mouse button while it waits cancels, and the click still works.
- [ ] Add input, then press Mouse 4 or 5 (if your mouse has them): it is added and works in a drag.
- [ ] Bind a key already used by the other command: it asks to move it. Move moves it; Cancel
      keeps things as they were.
- [ ] Remove an input, Save: that input no longer starts grid mode.

### General tab
- [ ] "Show window contents while dragging" changes Windows right away (no Save). Untick it and
      drag: Windows shows an outline. Tick it again.

## 7. Hotkeys (Input tab → Hotkeys)

- [ ] Set Undo to e.g. Ctrl+Alt+Z, Save. Snap a window, press the hotkey: it goes back to where
      it was before the snap. Repeated presses step back through earlier snaps (up to 10).
- [ ] Tray → Undo last snap does the same, and is greyed out when there is nothing to undo.
- [ ] Set Save positions and Restore positions hotkeys, Save, and try them (see section 8).
- [ ] Giving a second action the same combination moves it there (the first is cleared).
- [ ] While waiting for a combination, the bottom-left message explains what to press. After
      pressing, it says the combination was set (or why not). At the smallest window size the
      whole message still shows.
- [ ] Close Settings while it waits for a key: your next key press works normally (it isn't
      swallowed).
- [ ] Ctrl+Win+a key (e.g. Ctrl+Win+X) is set as "Ctrl+Win+X", and the Start menu doesn't open.
- [ ] A key with no Ctrl/Alt/Win (e.g. Z or Shift+Z) isn't set and the message says why. F-keys
      alone (e.g. F8) are allowed.
- [ ] A combination Windows or another app already uses (e.g. Win+E) isn't set, with a message
      saying so.
- [ ] Clear removes a hotkey; after Save it does nothing.

## 8. Save and restore window positions

- [ ] Arrange several windows (one maximized, one minimized), tray → Save window positions:
      a notification says how many were saved.
- [ ] Move them all around, tray → Restore: each goes back, including maximized and minimized.
- [ ] Overlapping windows come back stacked in the order they were in when saved (the front one
      in front and active). Also after Win+D (show desktop), then Restore.
- [ ] Close an app and reopen it (same document if possible), Restore: it goes back too.
- [ ] With nothing saved yet, tray → Restore is greyed out and the Restore hotkey says nothing
      has been saved. (Rename `positions.json` next to `grid.exe` to try, then rename it back.)

## 9. Multiple monitors (still on the "test later" list)

- [ ] Resize from a cell on one monitor to a cell on the other: the window spans both.
- [ ] Move a window across: it can sit half on each monitor, and the live preview matches where
      it lands. Try monitors with different scaling.
- [ ] Settings → Grid → Monitors lists each monitor by name. Plugging one in or unplugging it
      while Settings is open updates the list within a couple of seconds.
- [ ] Give one monitor its own grid, Save: only that monitor uses it.
- [ ] Unplug and replug that monitor (or restart): it keeps its own grid.

## 10. Left-handed mouse (still on the "test later" list)

Windows Settings → Bluetooth & devices → Mouse → Primary mouse button: Right.
- [ ] Dragging with the right button and pressing Space resizes; releasing places the window.
- [ ] The left button now acts as "RightMouse" (Resize) during a drag.
- [ ] Set the primary button back to Left afterwards.

## 11. Log

- [ ] `grid.log` next to `grid.json` (portable: next to `grid.exe`; installed: `%APPDATA%\Grid`)
      has a line when Grid starts, and one saying why when it exits (tray Exit, restart as
      administrator). It contains no keys, buttons or window titles.

## 11b. Light footprint

- [ ] Task Manager → Details: `grid.exe` (engine) uses roughly 20–25 MB memory while idle, and
      about 0% CPU when you aren't dragging.
- [ ] With Settings open there is a second `grid.exe` (~35 MB) that goes away when you close it.

## 12. Install, uninstall and start with Windows

Exit the running Grid first. Copy `target\release\grid.exe` into an empty folder (e.g. a new
folder in Downloads) and run it from there.
- [ ] The first run shows "Welcome to Grid" with three choices and "Start Grid when Windows
      starts" ticked. Cancel closes it and nothing is created in that folder.
- [ ] **Run without installing**: Grid starts (splash, tray icon) and `grid.json` appears next
      to the exe. Running it again goes straight to Grid, no setup window.
- [ ] The portable copy doesn't start with Windows, even with "Start Grid when Windows starts"
      ticked on that screen (the screen says it's only for installs). In Settings → General the
      checkbox is greyed out with a note to install. An old start-with-Windows entry for a
      portable copy is removed when it starts (and `grid.log` says so).
- [ ] Delete that `grid.json`, run again, choose **Install just for me**: no administrator
      prompt. A message says Grid is installed, and Grid is running from
      `%LOCALAPPDATA%\Programs\Grid`. Settings → General → Installation says so.
- [ ] Start menu: "Grid" is there and starts it. Windows Settings → Apps → Installed apps lists
      Grid with its icon and version.
- [ ] Settings carried over: anything changed in the portable copy is still set.
- [ ] Sign out and in (or restart): Grid starts by itself. Untick "Start Grid when Windows starts"
      in Settings → General: it doesn't any more.
- [ ] Task Manager → Startup apps lists "Grid" (reopen Task Manager after changing the setting).
- [ ] **Uninstall** from Windows Settings → Apps: the "Uninstall Grid" window asks first, then
      Grid exits, the Start menu entry and the `Programs\Grid` folder are gone, and it's no longer
      listed. Without "Also delete my settings", `%APPDATA%\Grid` is still there; with it, the
      folder is gone.
- [ ] Repeat with **Install for all users**: Windows asks for administrator rights; Grid ends
      up in `C:\Program Files\Grid` and runs as a normal (non-administrator) process.
      Uninstall from Settings → General → "Uninstall Grid…" also works (with Windows' prompt).
- [ ] Installing while an installed Grid is running (run the downloaded copy again after
      deleting its `grid.json`): the setup window says Grid is already installed; installing
      replaces it and Grid restarts.

## 13. Updates (needs a published GitHub Release; see RELEASING.md)

- [ ] Tray → Check for updates with no release published: "Couldn't check for updates: no
      published release was found on GitHub." (Also what a private repository gives.)
- [ ] With a newer release published: it offers the new version with a link to its notes.
      Yes downloads it, replaces grid.exe and restarts Grid; About shows the new version.
- [ ] With the latest version already running: "You have the latest version of Grid".
- [ ] Settings → General → Updates: with "Check for updates when Grid starts" on, a newer
      release is offered right after Grid starts; with "Install updates without asking" on, it
      installs and restarts by itself (an all-users install still shows Windows' administrator
      prompt).
