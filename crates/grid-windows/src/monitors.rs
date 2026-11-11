use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use grid_core::geometry::Rect;
use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

use crate::window::to_rect;

pub struct Monitor {
    /// Device name, e.g. `\\.\DISPLAY1`, used to look up `id` and `label`. Windows may hand these
    /// out differently after monitors are reconnected, so settings are keyed by `id` instead.
    pub name: String,
    /// Identifies this physical monitor (on this port) across reconnects and restarts: its device
    /// path, which includes the monitor's model code. Falls back to `name` if unavailable.
    pub id: String,
    /// The monitor's own name, such as "ASUS VG27A", or "Built-in display". May be empty.
    pub label: String,
    pub bounds: Rect,
    /// Bounds minus the taskbar and other app bars.
    pub work: Rect,
    /// Display scaling: 1.0 at 100%, 2.0 at 200%.
    pub scale: f32,
}

pub fn list() -> Vec<Monitor> {
    let mut monitors: Vec<Monitor> = Vec::new();
    unsafe { EnumDisplayMonitors(null_mut(), null(), Some(collect), &mut monitors as *mut Vec<Monitor> as LPARAM) };
    let names = device_names();
    for m in &mut monitors {
        if let Some((_, id, label)) = names.iter().find(|(gdi, ..)| gdi.eq_ignore_ascii_case(&m.name)) {
            m.id = id.clone();
            m.label = label.clone();
        }
    }
    monitors
}

unsafe extern "system" fn collect(monitor: HMONITOR, _dc: HDC, _clip: *mut RECT, data: LPARAM) -> BOOL {
    let monitors = unsafe { &mut *(data as *mut Vec<Monitor>) };
    let mut info: MONITORINFOEXW = unsafe { zeroed() };
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) } != 0 {
        let (mut dpi_x, mut dpi_y) = (96, 96);
        unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
        let name = from_wide(&info.szDevice);
        monitors.push(Monitor {
            id: name.clone(),
            name,
            label: String::new(),
            bounds: to_rect(info.monitorInfo.rcMonitor),
            work: to_rect(info.monitorInfo.rcWork),
            scale: dpi_x as f32 / 96.0,
        });
    }
    1
}

/// (device name, device path, friendly name) for each active display, from the display
/// configuration API.
fn device_names() -> Vec<(String, String, String)> {
    let (mut path_count, mut mode_count) = (0u32, 0u32);
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count) } != 0 {
        return Vec::new();
    }
    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![unsafe { zeroed() }; path_count as usize];
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![unsafe { zeroed() }; mode_count as usize];
    let result = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            null_mut(),
        )
    };
    if result != 0 {
        return Vec::new();
    }
    paths.truncate(path_count as usize);
    paths
        .iter()
        .filter_map(|path| {
            let mut source: DISPLAYCONFIG_SOURCE_DEVICE_NAME = unsafe { zeroed() };
            source.header = header(DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>());
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            let mut target: DISPLAYCONFIG_TARGET_DEVICE_NAME = unsafe { zeroed() };
            target.header = header(DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>());
            target.header.adapterId = path.targetInfo.adapterId;
            target.header.id = path.targetInfo.id;
            unsafe {
                if DisplayConfigGetDeviceInfo(&mut source.header) != 0 || DisplayConfigGetDeviceInfo(&mut target.header) != 0 {
                    return None;
                }
            }
            let device_path = from_wide(&target.monitorDevicePath);
            // Laptop screens report a panel part number (e.g. "ATNA33AA08-0"), which means
            // nothing to most people.
            let label = if target.outputTechnology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL {
                "Built-in display".to_string()
            } else {
                from_wide(&target.monitorFriendlyDeviceName).trim().to_string()
            };
            (!device_path.is_empty()).then(|| (from_wide(&source.viewGdiDeviceName), device_path, label))
        })
        .collect()
}

fn header(kind: i32, size: usize) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: kind, size: size as u32, ..unsafe { zeroed() } }
}

/// A null-terminated UTF-16 buffer as a string.
fn from_wide(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}
