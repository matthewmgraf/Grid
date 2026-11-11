//! Global hotkeys (undo, save and restore window positions), registered with the OS.

use std::ptr::null_mut;

use grid_core::settings::{Hotkey, Hotkeys};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};

use crate::input::to_vk;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Undo,
    SavePositions,
    RestorePositions,
}

const ACTIONS: [Action; 3] = [Action::Undo, Action::SavePositions, Action::RestorePositions];

impl Action {
    fn id(self) -> i32 {
        ACTIONS.iter().position(|a| *a == self).unwrap_or(0) as i32 + 1
    }

    /// The action for a `WM_HOTKEY` id.
    pub fn from_id(id: usize) -> Option<Action> {
        ACTIONS.get(id.checked_sub(1)?).copied()
    }

    fn name(self) -> &'static str {
        match self {
            Action::Undo => "Undo last snap",
            Action::SavePositions => "Save window positions",
            Action::RestorePositions => "Restore window positions",
        }
    }

    fn hotkey(self, hotkeys: &Hotkeys) -> Option<Hotkey> {
        match self {
            Action::Undo => hotkeys.undo,
            Action::SavePositions => hotkeys.save_positions,
            Action::RestorePositions => hotkeys.restore_positions,
        }
    }
}

/// Registers `hotkeys` for `hwnd`, replacing any registered before. Returns a message for each
/// one that couldn't be registered (usually because another app already uses it).
pub fn register(hwnd: HWND, hotkeys: &Hotkeys) -> Vec<String> {
    let mut failed = Vec::new();
    for action in ACTIONS {
        unsafe { UnregisterHotKey(hwnd, action.id()) };
        let Some(hotkey) = action.hotkey(hotkeys) else { continue };
        if unsafe { RegisterHotKey(hwnd, action.id(), modifiers(hotkey), to_vk(hotkey.key)) } == 0 {
            failed.push(format!("{hotkey} ({}) is already used by Windows or another app.", action.name()));
        }
    }
    failed
}

/// Whether `hotkey` could be registered right now, i.e. Windows or another app doesn't already
/// use it. Tried for a moment and released again.
pub fn available(hotkey: Hotkey) -> bool {
    const TRIAL_ID: i32 = 0xBFFF;
    let ok = unsafe { RegisterHotKey(null_mut(), TRIAL_ID, modifiers(hotkey), to_vk(hotkey.key)) } != 0;
    if ok {
        unsafe { UnregisterHotKey(null_mut(), TRIAL_ID) };
    }
    ok
}

fn modifiers(hotkey: Hotkey) -> HOT_KEY_MODIFIERS {
    [(hotkey.ctrl, MOD_CONTROL), (hotkey.alt, MOD_ALT), (hotkey.shift, MOD_SHIFT), (hotkey.win, MOD_WIN)]
        .iter()
        .filter(|(on, _)| *on)
        .fold(MOD_NOREPEAT, |acc, (_, m)| acc | m)
}
