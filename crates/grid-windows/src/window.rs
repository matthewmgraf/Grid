use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;

use grid_core::geometry::Rect;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsZoomed, SW_RESTORE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE,
    SWP_NOZORDER,
    SetWindowPos, ShowWindowAsync,
};

pub fn to_rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

/// Window rectangle including the invisible resize borders Windows 10+ adds.
fn outer_rect(hwnd: HWND) -> Option<Rect> {
    let mut r: RECT = unsafe { zeroed() };
    (unsafe { GetWindowRect(hwnd, &mut r) } != 0).then(|| to_rect(r))
}

/// Visible frame of the window, without the invisible borders.
pub fn frame_rect(hwnd: HWND) -> Option<Rect> {
    let mut r: RECT = unsafe { zeroed() };
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as _,
            &mut r as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
    };
    if hr == 0 { Some(to_rect(r)) } else { outer_rect(hwnd) }
}

/// Moves and resizes the window so its visible frame covers `target`. Asynchronous, so a hung
/// app can't freeze Grid.
pub fn move_to(hwnd: HWND, target: Rect) {
    if unsafe { IsZoomed(hwnd) } != 0 {
        unsafe { ShowWindowAsync(hwnd, SW_RESTORE) };
    }
    let (Some(outer), Some(frame)) = (outer_rect(hwnd), frame_rect(hwnd)) else { return };
    let (left, top) = (frame.x - outer.x, frame.y - outer.y);
    let (right, bottom) = (outer.right() - frame.right(), outer.bottom() - frame.bottom());
    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            target.x - left,
            target.y - top,
            target.w + left + right,
            target.h + top + bottom,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
        )
    };
}

/// Moves the window without resizing it, so its visible frame's top-left is at (`x`, `y`).
pub fn move_frame_to(hwnd: HWND, x: i32, y: i32) {
    let (Some(outer), Some(frame)) = (outer_rect(hwnd), frame_rect(hwnd)) else { return };
    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            outer.x + x - frame.x,
            outer.y + y - frame.y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
        )
    };
}
