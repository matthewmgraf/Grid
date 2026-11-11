//! Notices when the engine's main thread stops responding and logs what it was busy with, so a
//! freeze can be traced afterwards (see `log`).

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{SMTO_NORMAL, SendMessageTimeoutW, WM_NULL};

use crate::log;

/// What the main thread is handling right now: a window message, or one of the `HOOK_*` codes.
/// 0 when it is idle, waiting for the next message.
static BUSY: AtomicU32 = AtomicU32::new(0);
/// The `wparam` of that message (which timer, for `WM_TIMER`).
static BUSY_WPARAM: AtomicUsize = AtomicUsize::new(0);

pub const HOOK_DRAG_EVENT: u32 = 0xF000_0001;
pub const HOOK_MOUSE: u32 = 0xF000_0002;
pub const HOOK_KEYBOARD: u32 = 0xF000_0003;

/// How long the main thread may take to answer before it counts as frozen.
const TIMEOUT: Duration = Duration::from_secs(5);
const CHECK_EVERY: Duration = Duration::from_secs(10);

/// Marks the main thread as busy with `what` until dropped (restoring what it was busy with
/// before, since handlers can run inside other handlers).
pub struct Busy {
    previous: (u32, usize),
}

impl Busy {
    pub fn new(what: u32, wparam: usize) -> Busy {
        let previous = (BUSY.swap(what, Ordering::Relaxed), BUSY_WPARAM.swap(wparam, Ordering::Relaxed));
        Busy { previous }
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.store(self.previous.0, Ordering::Relaxed);
        BUSY_WPARAM.store(self.previous.1, Ordering::Relaxed);
    }
}

/// Starts a thread that checks every few seconds that the engine's window `hwnd` still answers,
/// and logs when it stops and starts again. `describe` names a message for the log.
pub fn start(hwnd: HWND, describe: fn(u32, usize) -> String) {
    let hwnd = hwnd as usize;
    std::thread::spawn(move || {
        let mut frozen_since: Option<Instant> = None;
        loop {
            std::thread::sleep(if frozen_since.is_some() { Duration::from_secs(1) } else { CHECK_EVERY });
            let mut result = 0;
            let answered = unsafe {
                SendMessageTimeoutW(
                    hwnd as HWND,
                    WM_NULL,
                    0,
                    0,
                    SMTO_NORMAL,
                    TIMEOUT.as_millis() as u32,
                    &mut result,
                )
            } != 0;
            match (answered, frozen_since) {
                (false, None) => {
                    let what = describe(BUSY.load(Ordering::Relaxed), BUSY_WPARAM.load(Ordering::Relaxed));
                    log::write(&format!("Not responding for {} s, busy with: {what}", TIMEOUT.as_secs()));
                    frozen_since = Some(Instant::now() - TIMEOUT);
                }
                (true, Some(since)) => {
                    log::write(&format!("Responding again after about {} s", since.elapsed().as_secs()));
                    frozen_since = None;
                }
                _ => {}
            }
        }
    });
}
