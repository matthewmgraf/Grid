//! Windows settings that Grid recommends: window contents shown while dragging. Changed only when
//! the user ticks or unticks it in Grid's settings. (Aero Shake was dropped: it is off by default
//! in Windows 11 and toggling it had no visible effect.)

use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETDRAGFULLWINDOWS, SPI_SETDRAGFULLWINDOWS, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SystemParametersInfoW,
};

/// Whether windows show their contents while being dragged (rather than an outline).
pub fn drag_full_windows() -> Option<bool> {
    let mut value: i32 = 0;
    let ok = unsafe { SystemParametersInfoW(SPI_GETDRAGFULLWINDOWS, 0, &mut value as *mut i32 as *mut c_void, 0) };
    (ok != 0).then_some(value != 0)
}

pub fn set_drag_full_windows(enabled: bool) -> Result<(), String> {
    let ok = unsafe {
        SystemParametersInfoW(SPI_SETDRAGFULLWINDOWS, enabled as u32, null_mut(), SPIF_UPDATEINIFILE | SPIF_SENDCHANGE)
    };
    if ok != 0 { Ok(()) } else { Err("Could not change \"Show window contents while dragging\".".into()) }
}
