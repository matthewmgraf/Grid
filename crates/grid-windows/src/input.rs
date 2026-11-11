use grid_core::settings::{Input, Key, MouseButton};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_BACK, VK_CAPITAL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F24, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU,
    VK_LSHIFT, VK_NEXT, VK_NUMPAD0, VK_NUMPAD9, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT,
    VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};

/// Maps a virtual-key code from a low-level keyboard hook to a bindable input.
pub fn from_vk(vk: u32) -> Option<Input> {
    let vk = u16::try_from(vk).ok()?;
    let key = match vk {
        VK_ESCAPE => Key::Escape,
        VK_SPACE => Key::Space,
        VK_TAB => Key::Tab,
        VK_LCONTROL => Key::LeftCtrl,
        VK_RCONTROL => Key::RightCtrl,
        VK_LSHIFT => Key::LeftShift,
        VK_RSHIFT => Key::RightShift,
        VK_LMENU => Key::LeftAlt,
        VK_RMENU => Key::RightAlt,
        VK_RETURN => Key::Enter,
        VK_BACK => Key::Backspace,
        VK_CAPITAL => Key::CapsLock,
        VK_INSERT => Key::Insert,
        VK_DELETE => Key::Delete,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        // Virtual-key codes for 0-9 and A-Z are their ASCII codes.
        0x30..=0x39 | 0x41..=0x5A => Key::Char(vk as u8 as char),
        VK_F1..=VK_F24 => Key::F((vk - VK_F1 + 1) as u8),
        VK_NUMPAD0..=VK_NUMPAD9 => Key::Numpad((vk - VK_NUMPAD0) as u8),
        _ => return None,
    };
    Some(Input::Key(key))
}

/// Virtual-key code for `key` (the reverse of `from_vk`).
pub fn to_vk(key: Key) -> u32 {
    let vk = match key {
        Key::Escape => VK_ESCAPE,
        Key::Space => VK_SPACE,
        Key::Tab => VK_TAB,
        Key::LeftCtrl => VK_LCONTROL,
        Key::RightCtrl => VK_RCONTROL,
        Key::LeftShift => VK_LSHIFT,
        Key::RightShift => VK_RSHIFT,
        Key::LeftAlt => VK_LMENU,
        Key::RightAlt => VK_RMENU,
        Key::Enter => VK_RETURN,
        Key::Backspace => VK_BACK,
        Key::CapsLock => VK_CAPITAL,
        Key::Insert => VK_INSERT,
        Key::Delete => VK_DELETE,
        Key::Home => VK_HOME,
        Key::End => VK_END,
        Key::PageUp => VK_PRIOR,
        Key::PageDown => VK_NEXT,
        Key::Up => VK_UP,
        Key::Down => VK_DOWN,
        Key::Left => VK_LEFT,
        Key::Right => VK_RIGHT,
        Key::Char(c) => c.to_ascii_uppercase() as u16,
        Key::F(n) => VK_F1 + n as u16 - 1,
        Key::Numpad(n) => VK_NUMPAD0 + n as u16,
    };
    vk as u32
}

/// The button that clicks and drags windows, as low-level hooks report it. Windows applies the
/// left-handed swap ("primary mouse button: Right") before hooks see a click, so the primary
/// button always arrives as a left-button message. Bound inputs are named the same way:
/// "RightMouse" means the secondary (context menu) button, whichever side it is on.
/// (Seen on the owner's machine: with the swap on, the physical right button captured as
/// LeftMouse.)
pub const PRIMARY: MouseButton = MouseButton::Left;

/// Whether the mouse buttons are swapped for left-handed use. `SendInput` takes physical
/// buttons, so releasing the primary button means sending a right-button release then.
pub fn buttons_swapped() -> bool {
    unsafe { GetSystemMetrics(SM_SWAPBUTTON) != 0 }
}
