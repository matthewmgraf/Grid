//! A window drag: noticing it start and end, grid mode (Resize/Move/Cancel) while it lasts, the
//! input hooks, and placing the window live and at the end.

use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use grid_core::geometry::{Point, Rect};
use grid_core::session::{Command, Session};
use grid_core::settings::{Input, Key, MouseButton};
use grid_core::zones::{anchor_in, apply_gaps, area_for, grid_zones, reach};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTUP,
    MOUSEINPUT, VK_ESCAPE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, GetCursorPos, HC_ACTION, HHOOK, IsWindow,
    KBDLLHOOKSTRUCT, KillTimer, LLMHF_INJECTED, MSLLHOOKSTRUCT, OBJID_WINDOW, PostMessageW, SetTimer, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
    WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::{
    App, WM_APP_CANCEL, WM_APP_DROP, WM_APP_ESCAPE, WM_APP_TAKEOVER, admin, input, monitors, overlay, send_input,
    watchdog, window, with_app,
};

/// Marks input Grid sends itself (`dwExtraInfo`), so its own hooks let it through.
const GRID_INPUT: usize = 0x4752_4944; // "GRID"
pub(crate) const VERIFY_TIMER: usize = 1;
/// Re-checks a live placement after the window has had time to respond.
pub(crate) const SETTLE_TIMER: usize = 2;
/// How long a window gets to respond to a live placement before Grid corrects it.
const SETTLE_MS: u64 = 60;
/// Places the window anyway if Windows doesn't report the end of its move loop after a takeover.
pub(crate) const LOOP_END_TIMER: usize = 3;
pub(crate) const LOOP_END_TIMEOUT_MS: u32 = 250;
/// Delay before checking that a snapped window stayed put (DPI changes, Aero Snap).
const VERIFY_DELAY_MS: u32 = 80;

pub(crate) struct Drag {
    hwnd: HWND,
    session: Session,
    /// Bound inputs currently held down. Used to ignore key auto-repeat and to only swallow
    /// releases whose press Grid also swallowed.
    held: Vec<Input>,
    cancelled: bool,
    hooks: [HHOOK; 2],
    /// Window frame when the drag started, restored on Cancel.
    original: Rect,
    /// Grid has ended the OS move loop and now places the window itself, live.
    taken_over: bool,
    /// Windows' own move loop has ended after the takeover. Until then Grid doesn't place the
    /// window, because the ending loop would put it back to its old size for a moment.
    loop_ended: bool,
    /// When the takeover started, for the `LOOP_END_TIMEOUT_MS` fallback.
    taken_at: Option<Instant>,
    /// Smallest width and height the window has refused to go below during this drag.
    min_size: (i32, i32),
    /// Last rectangle sent to the window while taken over, and when.
    applied: Option<Rect>,
    applied_at: Option<Instant>,
    /// How many more times the current target is re-sent when the window ends up the wrong size.
    /// Moving onto a monitor with different scaling makes apps resize themselves, so the first
    /// placement often doesn't stick.
    live_retries: u8,
    /// Monitor work areas, to keep windows that won't shrink on screen.
    work_areas: Vec<Rect>,
    /// Display scaling of each monitor in `work_areas`, for gaps.
    scales: Vec<f32>,
    /// Gaps from the settings, at 100% scaling.
    gaps: (u32, u32),
    /// The zones as drawn (where windows go, i.e. with the gaps).
    cells: Vec<Rect>,
}

impl Drag {
    /// Where the window goes for grid rectangle `target`: inset by the gaps, scaled for the
    /// monitor it is mostly on.
    fn placed(&self, target: Rect) -> Rect {
        let area = area_for(&self.work_areas, target);
        let scale = self.work_areas.iter().position(|a| *a == area).map_or(1.0, |i| self.scales[i]);
        let px = |v: u32| (v as f32 * scale).round() as i32;
        apply_gaps(target, &self.work_areas, px(self.gaps.0), px(self.gaps.1))
    }

    /// `placed` grown to the window's minimum size once the app has refused a smaller one,
    /// anchored like `live_placement` does, so the window is sent a size it accepts first time.
    fn fitted(&self, target: Rect) -> Rect {
        let placed = self.placed(target);
        let (w, h) = (placed.w.max(self.min_size.0), placed.h.max(self.min_size.1));
        if (w, h) == (placed.w, placed.h) {
            return placed;
        }
        let bounds = reach(&self.work_areas, placed).unwrap_or_else(|| area_for(&self.work_areas, placed));
        anchor_in(placed, w, h, bounds)
    }
}

/// A finished snap, checked again shortly afterwards (see `verify`).
#[derive(Clone, Copy)]
pub(crate) struct Pending {
    hwnd: HWND,
    target: Rect,
    retries: u8,
    /// Work area of the target's monitor.
    work: Rect,
}

pub(crate) unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != OBJID_WINDOW || hwnd.is_null() {
        return;
    }
    let _busy = watchdog::Busy::new(watchdog::HOOK_DRAG_EVENT, 0);
    match event {
        EVENT_SYSTEM_MOVESIZESTART => drag_start(hwnd),
        EVENT_SYSTEM_MOVESIZEEND => drag_end(hwnd),
        _ => {}
    }
}

fn drag_start(hwnd: HWND) {
    finish_drag();
    if admin::drag_started(hwnd) {
        return;
    }
    let monitors = monitors::list();
    let Some((zones, style, gaps)) = with_app(|app| {
        app.pending = None;
        app.reload_if_changed();
        let s = &app.settings;
        let zones: Vec<Rect> = monitors
            .iter()
            .flat_map(|m| grid_zones(m.work, s.grid_for(&m.id), s.transpose_portrait))
            .collect();
        (zones, s.overlay.clone(), (s.gaps.between, s.gaps.edges))
    }) else {
        return;
    };
    if zones.is_empty() {
        return;
    }

    let original = window::frame_rect(hwnd).unwrap_or_default();
    let work_areas: Vec<Rect> = monitors.iter().map(|m| m.work).collect();
    let scales: Vec<f32> = monitors.iter().map(|m| m.scale).collect();
    let session = Session::new(zones, work_areas.clone(), cursor_pos(), original);
    let mut drag = Drag {
        hwnd,
        session,
        held: Vec::new(),
        cancelled: false,
        hooks: [null_mut(); 2],
        original,
        taken_over: false,
        loop_ended: false,
        taken_at: None,
        min_size: (0, 0),
        applied: None,
        applied_at: None,
        live_retries: 0,
        work_areas,
        scales,
        gaps,
        cells: Vec::new(),
    };
    drag.cells = drag.session.zones().iter().map(|z| drag.placed(*z)).collect();
    overlay::create(&monitors, style.blur, hwnd);
    // The grid itself is drawn now, while the user is still dragging, so that pressing a key only
    // has to add the highlight. This happens before the input hooks exist, so the mouse can't
    // stutter while it draws.
    overlay::prepare(&overlay::View {
        zones: drag.cells.clone(),
        start: None,
        target: None,
        gapless: drag.gaps == (0, 0),
        style,
    });
    drag.hooks = unsafe {
        let module = GetModuleHandleW(null());
        [
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0),
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0),
        ]
    };
    with_app(|app| app.drag = Some(drag));
}

/// The OS move loop ended. After a takeover that was Grid's doing, so the drag continues and
/// Grid can now place the window.
fn drag_end(hwnd: HWND) {
    if admin::drag_ended(hwnd) {
        return;
    }
    let taken_over = with_app(|app| {
        let drag = app.drag.as_mut().filter(|d| d.taken_over)?;
        drag.loop_ended = true;
        unsafe { KillTimer(app.msg_hwnd, LOOP_END_TIMER) };
        app.request_refresh();
        Some(())
    })
    .flatten()
    .is_some();
    if taken_over {
        return;
    }
    let Some(mut drag) = finish_drag() else { return };
    if drag.hwnd != hwnd || drag.cancelled {
        return;
    }
    drag.session.set_window(window::frame_rect(hwnd).unwrap_or_default());
    drag.session.update(cursor_pos());
    if let Some(target) = drag.session.target() {
        finish_snap(&drag, target);
    }
}

/// Places the dragged window for grid rectangle `target` and remembers where it was, for Undo.
fn finish_snap(drag: &Drag, target: Rect) {
    let placed = drag.fitted(target);
    with_app(|app| app.history.push(drag.hwnd as usize, drag.original));
    snap(Pending { hwnd: drag.hwnd, target: placed, retries: 2, work: area_for(&drag.work_areas, target) });
}

/// Ends the OS move loop by sending a synthetic primary-button release, so Grid can place the
/// window itself. The real release is swallowed later (see `mouse_proc`) and ends the drag.
pub(crate) fn take_over() {
    let release = if input::buttons_swapped() { MOUSEEVENTF_RIGHTUP } else { MOUSEEVENTF_LEFTUP };
    send_input(vec![INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: release, time: 0, dwExtraInfo: GRID_INPUT },
        },
    }]);
}

/// Presses and releases Esc, which makes the OS move loop put the window back and end.
pub(crate) fn send_escape() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VK_ESCAPE, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: GRID_INPUT },
        },
    };
    send_input(vec![key(0), key(KEYEVENTF_KEYUP)]);
}

/// Finishes a taken-over drag: the window gets its final place, or goes back on Cancel.
pub(crate) fn drop_window(cancelled: bool) {
    let Some(drag) = finish_drag() else { return };
    if cancelled || drag.cancelled {
        window::move_to(drag.hwnd, drag.original);
    } else if let Some(target) = drag.session.target() {
        finish_snap(&drag, target);
    }
}

/// Ends the current drag (if any): removes the hooks and the overlay.
pub(crate) fn finish_drag() -> Option<Drag> {
    let drag = with_app(|app| app.drag.take()).flatten()?;
    for hook in drag.hooks {
        if !hook.is_null() {
            unsafe { UnhookWindowsHookEx(hook) };
        }
    }
    overlay::destroy();
    Some(drag)
}

fn snap(pending: Pending) {
    window::move_to(pending.hwnd, pending.target);
    with_app(|app| {
        app.pending = Some(pending);
        unsafe { SetTimer(app.msg_hwnd, VERIFY_TIMER, VERIFY_DELAY_MS, None) };
    });
}

/// Re-applies the last snap if the window didn't end up where it was sent, e.g. because the
/// app rescaled itself for a monitor with different DPI. If the app still won't take the size
/// (it has a minimum size), the window is anchored to the target's corner instead.
pub(crate) fn verify() {
    let Some(p) = with_app(|app| app.pending.take()).flatten() else { return };
    if unsafe { IsWindow(p.hwnd) } == 0 {
        return;
    }
    let Some(frame) = window::frame_rect(p.hwnd) else { return };
    if frame == p.target {
        return;
    }
    if p.retries > 0 {
        snap(Pending { retries: p.retries - 1, ..p });
    } else if (frame.w, frame.h) != (p.target.w, p.target.h) {
        let anchored = anchor_in(p.target, frame.w, frame.h, p.work);
        window::move_frame_to(p.hwnd, anchored.x, anchored.y);
    }
}

enum Placement {
    /// Move and resize the window to this rectangle.
    Snap(Rect),
    /// Move the window's top-left here without resizing.
    Shift(i32, i32),
}

/// What to do to a taken-over window so it matches `target`, given where it actually is.
fn live_placement(drag: &mut Drag, target: Rect, window: Rect) -> Option<Placement> {
    if drag.applied != Some(target) {
        drag.applied = Some(target);
        drag.live_retries = 2;
        return Some(Placement::Snap(target));
    }
    let settled = drag.applied_at.is_none_or(|at| at.elapsed() >= Duration::from_millis(SETTLE_MS));
    if window == target || !settled {
        return None;
    }
    if (window.w, window.h) == (target.w, target.h) {
        // Right size, wrong place: something (the ending OS move loop) moved it. Send it again.
        return Some(Placement::Snap(target));
    }
    // Took one side exactly and is only bigger on the other: a minimum size, not a DPI rescale
    // (which changes both), so trying again won't help.
    let min_size = (window.w == target.w && window.h > target.h) || (window.h == target.h && window.w > target.w);
    if drag.live_retries > 0 && !min_size {
        // Possibly the app rescaling itself for another monitor's DPI; it usually takes the size
        // the second time.
        drag.live_retries -= 1;
        return Some(Placement::Snap(target));
    }
    // The app won't take this size: keep its own size, anchored to the target and on screen.
    // Remember it, so later cells are sent a size the app accepts.
    if window.w > target.w {
        drag.min_size.0 = drag.min_size.0.max(window.w);
    }
    if window.h > target.h {
        drag.min_size.1 = drag.min_size.1.max(window.h);
    }
    let bounds = reach(&drag.work_areas, target).unwrap_or_else(|| area_for(&drag.work_areas, target));
    let anchored = anchor_in(target, window.w, window.h, bounds);
    // This is what `fitted` now gives for the same cell, so it isn't sent again.
    drag.applied = Some(anchored);
    ((anchored.x, anchored.y) != (window.x, window.y)).then_some(Placement::Shift(anchored.x, anchored.y))
}

/// Feeds the current cursor and window position to the session and redraws the overlay.
pub(crate) fn refresh() {
    let Some(hwnd) = with_app(|app| {
        app.refresh_posted = false;
        app.drag.as_ref().map(|d| d.hwnd)
    })
    .flatten() else {
        return;
    };
    let (cursor, window) = (cursor_pos(), window::frame_rect(hwnd).unwrap_or_default());
    let mut placement = None;
    let view = with_app(|app| {
        let drag = app.drag.as_mut()?;
        if !drag.taken_over {
            drag.session.set_window(window);
        }
        drag.session.update(cursor);
        let target = drag.session.target();
        // Timers wait while other messages are queued, so the fallback is also checked here.
        if drag.taken_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(LOOP_END_TIMEOUT_MS.into())) {
            drag.loop_ended = true;
        }
        if drag.loop_ended
            && let Some(t) = target
        {
            let placed = drag.fitted(t);
            placement = live_placement(drag, placed, window);
            if placement.is_some() {
                drag.applied_at = Some(Instant::now());
                // Check again once the window has had time to respond, even if the mouse stops.
                unsafe { SetTimer(app.msg_hwnd, SETTLE_TIMER, SETTLE_MS as u32 + 10, None) };
            }
        }
        let session = &drag.session;
        // Drawn where windows actually go, i.e. with the gaps.
        Some(session.is_active().then(|| overlay::View {
            zones: drag.cells.clone(),
            start: session.start_zone().map(|z| drag.placed(z)),
            target: target.map(|t| drag.placed(t)),
            gapless: drag.gaps == (0, 0),
            style: app.settings.overlay.clone(),
        }))
    })
    .flatten();
    // Outside `with_app`: moving another process's window can re-enter the hooks.
    match placement {
        Some(Placement::Snap(target)) => window::move_to(hwnd, target),
        Some(Placement::Shift(x, y)) => window::move_frame_to(hwnd, x, y),
        None => {}
    }
    match view {
        Some(Some(view)) => overlay::show(view),
        Some(None) => overlay::hide(),
        None => {}
    }
}

/// The OS move loop's end wasn't reported in time after a takeover: place the window anyway.
pub(crate) fn loop_end_timeout() {
    with_app(|app| {
        if let Some(drag) = app.drag.as_mut() {
            drag.loop_ended = drag.taken_over;
        }
    });
    refresh();
}

/// Handles a press or release of a possibly bound input. Returns true to swallow it.
fn handle_input(input: Input, down: bool) -> bool {
    with_app(|app| {
        let command = app.settings.command_for(input)?;
        app.drag.as_ref()?;
        app.request_refresh();
        let drag = app.drag.as_mut()?;
        let was_held = drag.held.contains(&input);
        if !down {
            drag.held.retain(|i| *i != input);
            drag.session.release(command);
            return Some(was_held);
        }
        if was_held {
            return Some(true); // auto-repeat
        }
        if command == Command::Cancel {
            drag.cancelled = true;
            drag.session.cancel();
            let message = if drag.taken_over {
                WM_APP_CANCEL
            } else if input == Input::Key(Key::Escape) {
                // Let Esc through: Windows cancels its own move and puts the window back.
                return Some(false);
            } else {
                WM_APP_ESCAPE
            };
            drag.held.push(input);
            unsafe { PostMessageW(app.msg_hwnd, message, 0, 0) };
            return Some(true);
        }
        drag.held.push(input);
        drag.cancelled = false;
        drag.session.press(command);
        if !drag.taken_over {
            drag.taken_over = true;
            drag.taken_at = Some(Instant::now());
            unsafe { PostMessageW(app.msg_hwnd, WM_APP_TAKEOVER, 0, 0) };
        }
        Some(true)
    })
    .flatten()
    .unwrap_or(false)
}

/// The user released the primary (dragging) button. Swallowed after a takeover, where it ends
/// the drag.
fn primary_released() -> bool {
    with_app(|app| {
        let taken_over = app.drag.as_ref()?.taken_over;
        if taken_over {
            unsafe { PostMessageW(app.msg_hwnd, WM_APP_DROP, 0, 0) };
        }
        Some(taken_over)
    })
    .flatten()
    .unwrap_or(false)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let _busy = watchdog::Busy::new(watchdog::HOOK_MOUSE, 0);
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        let x_button = || if (info.mouseData >> 16) as u16 == 1 { MouseButton::X1 } else { MouseButton::X2 };
        let event = match wparam as u32 {
            WM_MOUSEMOVE => {
                with_app(App::request_refresh);
                None
            }
            WM_LBUTTONDOWN => Some((MouseButton::Left, true)),
            WM_LBUTTONUP => Some((MouseButton::Left, false)),
            WM_RBUTTONDOWN => Some((MouseButton::Right, true)),
            WM_RBUTTONUP => Some((MouseButton::Right, false)),
            WM_MBUTTONDOWN => Some((MouseButton::Middle, true)),
            WM_MBUTTONUP => Some((MouseButton::Middle, false)),
            WM_XBUTTONDOWN => Some((x_button(), true)),
            WM_XBUTTONUP => Some((x_button(), false)),
            _ => None,
        };
        if let Some((button, down)) = event {
            if button == input::PRIMARY {
                // Grid's own synthetic release (from `take_over`) passes through to end the OS move.
                let injected = info.flags & LLMHF_INJECTED != 0;
                if !down && !injected && primary_released() {
                    return 1;
                }
            } else if handle_input(Input::Mouse(button), down) {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let _busy = watchdog::Busy::new(watchdog::HOOK_KEYBOARD, 0);
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        // Grid's own Esc (from `send_escape`) goes straight to the OS move loop.
        if info.dwExtraInfo != GRID_INPUT
            && let Some(input) = input::from_vk(info.vkCode)
            && handle_input(input, down)
        {
            return 1;
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

fn cursor_pos() -> Point {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    Point::new(p.x, p.y)
}
