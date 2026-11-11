//! "Press the input you want" for the settings window: waits for one bindable key or mouse
//! button anywhere, and swallows it (press and release) so it doesn't also reach an app.
//!
//! Run this in a small helper process (`grid.exe --capture`), not in the settings window's
//! process. Inside that process the keyboard hook was never called (mouse buttons were), on any
//! thread; the same code in a separate process works, even with the settings window in front.
//! The exact cause is unknown.

use std::cell::Cell;
use std::mem::zeroed;
use std::ptr::{null, null_mut};

use grid_core::settings::{Hotkey, Input, Key, MouseButton};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU,
    VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, PostQuitMessage,
    SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_TIMER, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::input::{PRIMARY, from_vk};

/// How long to keep swallowing after the press, waiting for its release.
const RELEASE_TIMEOUT_MS: u32 = 2000;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Waiting,
    /// Result known; swallowing the release of this key (virtual-key code).
    ReleasingKey(u32),
    /// Result known; swallowing this mouse "up" message.
    ReleasingMouse(u32),
    Finished,
}

thread_local! {
    static STATE: Cell<State> = const { Cell::new(State::Waiting) };
    static RESULT: Cell<Option<Input>> = const { Cell::new(None) };
    /// Waiting for a hotkey (a key with modifiers) rather than a single input.
    static HOTKEY_MODE: Cell<bool> = const { Cell::new(false) };
    static HOTKEY: Cell<Option<Hotkey>> = const { Cell::new(None) };
}

/// Blocks until the user presses a key, possibly with Ctrl, Alt, Shift and Win held, and returns it
/// as a hotkey. Returns `None` on Esc or a click of the primary mouse button.
pub fn capture_hotkey() -> Option<Hotkey> {
    HOTKEY_MODE.set(true);
    capture_input();
    HOTKEY.get()
}

/// Blocks until the user presses a bindable key (Esc included) or mouse button, and returns it.
/// Returns `None` if they click the primary mouse button (it drags windows, so it can't be
/// bound; the click still goes through).
pub fn capture_input() -> Option<Input> {
    unsafe {
        let module = GetModuleHandleW(null());
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
        // Stops waiting if the settings window that asked closes meanwhile; otherwise this helper
        // would stay behind and swallow the user's next key press.
        let parent = parent_process();
        let parent_check = if parent.is_null() { 0 } else { SetTimer(null_mut(), 0, PARENT_CHECK_MS, None) };
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == WM_TIMER {
                if parent_check != 0 && msg.wParam == parent_check {
                    if WaitForSingleObject(parent, 0) == WAIT_OBJECT_0 {
                        RESULT.set(None);
                        HOTKEY.set(None);
                        break;
                    }
                    continue;
                }
                break; // the release never came
            }
            DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(keyboard);
        UnhookWindowsHookEx(mouse);
        if !parent.is_null() {
            CloseHandle(parent);
        }
    }
    RESULT.get()
}

/// Environment variable through which the settings window passes its process ID to the helper.
pub const PARENT_ENV: &str = "GRID_CAPTURE_PARENT";
const PARENT_CHECK_MS: u32 = 500;

/// The process that started this helper (see `PARENT_ENV`), opened to wait on; null if unknown.
fn parent_process() -> HANDLE {
    let Some(pid) = std::env::var(PARENT_ENV).ok().and_then(|v| v.parse::<u32>().ok()) else { return null_mut() };
    unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) }
}

/// Records the result, then keeps swallowing until `next`'s release (or a timeout).
fn captured(input: Option<Input>, next: State) {
    RESULT.set(input);
    STATE.set(next);
    if next == State::Finished {
        unsafe { PostQuitMessage(0) };
    } else {
        unsafe { SetTimer(null_mut(), 0, RELEASE_TIMEOUT_MS, None) };
    }
}

fn hotkey_with_modifiers(key: Key) -> Hotkey {
    let held = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } < 0;
    let win = held(VK_LWIN) || held(VK_RWIN);
    if win {
        // The key itself is swallowed, so Windows would see Win pressed and released on its own
        // and open the Start menu. An unassigned key in between stops that.
        let key = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: UNASSIGNED_VK, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        crate::send_input(vec![key(0), key(KEYEVENTF_KEYUP)]);
    }
    Hotkey { ctrl: held(VK_CONTROL), alt: held(VK_MENU), shift: held(VK_SHIFT), win, key }
}

/// A virtual-key code Windows doesn't assign to anything.
const UNASSIGNED_VK: u16 = 0xE8;

fn finished() {
    STATE.set(State::Finished);
    unsafe { PostQuitMessage(0) };
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let vk = unsafe { (*(lparam as *const KBDLLHOOKSTRUCT)).vkCode };
        let msg = wparam as u32;
        match STATE.get() {
            State::Waiting if matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN) => {
                let hotkey_mode = HOTKEY_MODE.get();
                // Esc can be bound (to Cancel), so it only cancels when waiting for a hotkey.
                if hotkey_mode && vk == VK_ESCAPE as u32 {
                    captured(None, State::ReleasingKey(vk));
                    return 1;
                }
                match from_vk(vk) {
                    // Modifiers are part of a hotkey, not the key itself.
                    Some(Input::Key(key)) if hotkey_mode && !key.is_modifier() => {
                        HOTKEY.set(Some(hotkey_with_modifiers(key)));
                        captured(None, State::ReleasingKey(vk));
                        return 1;
                    }
                    Some(input) if !hotkey_mode => {
                        captured(Some(input), State::ReleasingKey(vk));
                        return 1;
                    }
                    _ => {}
                }
            }
            State::ReleasingKey(held) if held == vk => {
                if matches!(msg, WM_KEYUP | WM_SYSKEYUP) {
                    finished();
                }
                return 1; // the release, or auto-repeat while held
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        let msg = wparam as u32;
        match STATE.get() {
            State::Waiting => {
                let x_button = if (info.mouseData >> 16) as u16 == 1 { MouseButton::X1 } else { MouseButton::X2 };
                let pressed = match msg {
                    WM_LBUTTONDOWN => Some((MouseButton::Left, WM_LBUTTONUP)),
                    WM_RBUTTONDOWN => Some((MouseButton::Right, WM_RBUTTONUP)),
                    WM_MBUTTONDOWN => Some((MouseButton::Middle, WM_MBUTTONUP)),
                    WM_XBUTTONDOWN => Some((x_button, WM_XBUTTONUP)),
                    _ => None,
                };
                match pressed {
                    // Cancel, and let the click through.
                    Some((button, _)) if button == PRIMARY => captured(None, State::Finished),
                    // Hotkeys are keys only; other buttons go through.
                    Some(_) if HOTKEY_MODE.get() => {}
                    Some((button, up)) => {
                        captured(Some(Input::Mouse(button)), State::ReleasingMouse(up));
                        return 1;
                    }
                    None => {}
                }
            }
            State::ReleasingMouse(up) if up == msg => {
                finished();
                return 1;
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}
