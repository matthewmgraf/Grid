//! Notification-area (tray) icon and its menu.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_NONE, NIIF_NOSOUND, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON};

use grid_core::logo;

use crate::menu::{self, Entry};
use crate::{WM_APP_TRAY, bitmap};

pub enum MenuChoice {
    Undo,
    SavePositions,
    RestorePositions,
    Settings,
    CheckForUpdates,
    Exit,
}

/// Which menu items can be used right now.
pub struct MenuState {
    pub can_undo: bool,
    pub can_restore: bool,
    /// Show the menu in dark mode.
    pub dark: bool,
}

const ID_SETTINGS: usize = 1;
const ID_UPDATES: usize = 2;
const ID_EXIT: usize = 3;
const ID_UNDO: usize = 4;
const ID_SAVE: usize = 5;
const ID_RESTORE: usize = 6;

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data
}

/// Copies `text` into a fixed-size, null-terminated UTF-16 field, cutting it short if needed.
fn copy_text(dst: &mut [u16], text: &str) {
    let max = dst.len() - 1;
    for (d, s) in dst.iter_mut().zip(text.encode_utf16().take(max).chain(Some(0))) {
        *d = s;
    }
}

pub fn add(hwnd: HWND, icon: HICON) -> bool {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_APP_TRAY;
    data.hIcon = icon;
    copy_text(&mut data.szTip, "Grid");
    unsafe { Shell_NotifyIconW(NIM_ADD, &data) != 0 }
}

pub fn remove(hwnd: HWND) {
    let data = icon_data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}

/// Shows a short notification from the tray icon.
pub fn notify(hwnd: HWND, text: &str) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_NONE | NIIF_NOSOUND;
    copy_text(&mut data.szInfoTitle, "Grid");
    copy_text(&mut data.szInfo, text);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

/// Shows the tray menu at the cursor and returns the chosen item.
pub fn show_menu(hwnd: HWND, state: MenuState) -> Option<MenuChoice> {
    let item = |id, label, enabled| Entry::Item { id, label, enabled };
    let entries = vec![
        item(ID_UNDO, "Undo last snap", state.can_undo),
        Entry::Separator,
        item(ID_SAVE, "Save window positions", true),
        item(ID_RESTORE, "Restore window positions", state.can_restore),
        Entry::Separator,
        item(ID_SETTINGS, "Settings", true),
        item(ID_UPDATES, "Check for updates", true),
        Entry::Separator,
        item(ID_EXIT, "Exit", true),
    ];
    match menu::show(hwnd, entries, state.dark)? {
        ID_UNDO => Some(MenuChoice::Undo),
        ID_SAVE => Some(MenuChoice::SavePositions),
        ID_RESTORE => Some(MenuChoice::RestorePositions),
        ID_SETTINGS => Some(MenuChoice::Settings),
        ID_UPDATES => Some(MenuChoice::CheckForUpdates),
        ID_EXIT => Some(MenuChoice::Exit),
        _ => None,
    }
}

/// The tray icon, made from the logo. Also used for title bars.
pub fn create_icon() -> HICON {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let Some((color, bits)) = bitmap::create(size, size) else { return null_mut() };
    unsafe {
        let pixels = std::slice::from_raw_parts_mut(bits, (size * size) as usize);
        pixels.copy_from_slice(&logo::draw(size));
        let mask = CreateBitmap(size, size, 1, 1, null());
        let icon_info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&icon_info);
        DeleteObject(mask);
        DeleteObject(color);
        icon
    }
}
