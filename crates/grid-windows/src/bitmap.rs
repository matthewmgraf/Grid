use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;

use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP,
};

/// A `w`×`h` 32-bit bitmap, rows top first, and a pointer to its `w * h` pixels (`0xAARRGGBB`),
/// which Grid draws into directly. The pixels live until the bitmap is deleted. Used for the
/// overlay, the splash screen and the tray icon.
pub fn create(w: i32, h: i32) -> Option<(HBITMAP, *mut u32)> {
    unsafe {
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = w;
        info.bmiHeader.biHeight = -h; // negative: top-down
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB as _;
        let mut bits: *mut c_void = null_mut();
        let bitmap = CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            return None;
        }
        Some((bitmap, bits as *mut u32))
    }
}
